"""Odtwarza lokalną historię meczów i pokazuje prognozy Elo. Wstepnie na podstawie loklanego pliku CSV"""
"""Planowane jest rozszerzenie o pobieranie historii meczów z API HLTV, ale obecnie nie jest to zaimplementowane."""

import argparse
import csv
from bisect import insort
from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime, timedelta
from math import isfinite
from pathlib import Path
from typing import Optional

from .elo import EloParameters, decay_rating, form_rating_bonus, update_ratings, win_probability


MATCH_COLUMNS = {
    "match_id", "starts_at", "result_available_at",
    "team_a_id", "team_b_id", "winner_team_id",
}
MAP_SCORE_COLUMNS = ("maps_won_a", "maps_won_b")


@dataclass(frozen=True)
class Prediction:
    """Prognoza z odtworzenia meczu wraz z wynikiem potrzebnym do jej oceny."""

    match_id: str
    predicted_at: datetime
    team_a_id: str
    team_b_id: str
    winner_team_id: str
    probability_a: float


def read_events(path: Path) -> list[tuple[datetime, bool, dict[str, str]]]:
    """Wczytuje osobne zdarzenia rozpoczęcia meczu i dostępności wyniku."""
    events = [] 
    seen_match_ids = set() 
    with path.open(encoding="utf-8-sig", newline="") as stream:
        reader = csv.DictReader(stream, strict=True) 
        missing = MATCH_COLUMNS.difference(reader.fieldnames or [])
        if missing:
            raise ValueError(f"Brak kolumn w {path} ({', '.join(sorted(missing))})")

        for row in reader:
            try:
                match = {column: (row[column] or "").strip() for column in MATCH_COLUMNS}
                if not all(match.values()):
                    raise ValueError("Brakuje wartości w jednej z wymaganych kolumn")
                if match["match_id"] in seen_match_ids:
                    raise ValueError(f"Powtórzony identyfikator meczu {match['match_id']}")
                teams = {match["team_a_id"], match["team_b_id"]}
                if len(teams) != 2 or match["winner_team_id"] not in teams:
                    raise ValueError("Mecz wymaga dwóch drużyn i zwycięzcy z tej pary")

                starts_at = datetime.fromisoformat(match["starts_at"].replace("Z", "+00:00"))
                result_at = datetime.fromisoformat(match["result_available_at"].replace("Z", "+00:00"))
                if starts_at.tzinfo is None or result_at.tzinfo is None:
                    raise ValueError("Czasy muszą zawierać strefę czasową")
                if result_at <= starts_at:
                    raise ValueError("Wynik musi być dostępny po starcie meczu")
            except ValueError as error:
                raise ValueError(f"{path}, wiersz {reader.line_num}. {error}") from error

            seen_match_ids.add(match["match_id"])
            for column in MAP_SCORE_COLUMNS:
                match[column] = (row.get(column) or "").strip()
            events.append((starts_at, False, match))
            events.append((result_at, True, match))

    # Przy tym samym czasie start (False) jest przed wynikiem (True).
    # Prognoza korzysta więc tylko z wyników znanych przed rozpoczęciem meczu.
    return sorted(events, key=lambda event: (event[0], event[1], event[2]["match_id"]))


def _rating_at(
    saved_rating: tuple[float, datetime],
    at: datetime,
    half_life_days: Optional[float],
    initial_rating: float,
) -> float:
    """Oblicza rating na podaną chwilę na podstawie ostatniej zapisanej aktualizacji."""
    rating, last_result_at = saved_rating
    if half_life_days is None:
        return rating
    idle_days = (at - last_result_at) / timedelta(days=1)
    return decay_rating(rating, idle_days, half_life_days, initial_rating=initial_rating)


def _map_score(match: dict[str, str]) -> Optional[tuple[int, int]]:
    """Zwraca wynik map, jeśli historia zawiera tę parę wartości."""
    maps_a, maps_b = (match.get(column, "") for column in MAP_SCORE_COLUMNS)
    if not maps_a and not maps_b:
        return None
    try:
        return int(maps_a), int(maps_b)
    except ValueError as error:
        raise ValueError("Wynik map wymaga dwóch liczb całkowitych") from error


def replay_events(
    events: list[tuple[datetime, bool, dict[str, str]]],
    half_life_days: Optional[float] = None,
    *,
    score_margin: bool = False,
    form_window: int = 0,
    parameters: EloParameters = EloParameters(),
    prediction_from: Optional[datetime] = None,
    on_prediction: Optional[Callable[[Prediction], None]] = None,
) -> tuple[list[Prediction], dict[str, float]]:
    """Odtwarza zdarzenia i zwraca ranking końcowy.

    Prognozy od prediction_from trafiają do listy lub do on_prediction.
    Wcześniejsze zdarzenia służą do zbudowania ratingów i formy.
    """
    if half_life_days is not None and (not isfinite(half_life_days) or half_life_days <= 0):
        raise ValueError("Okres półtrwania musi być dodatnią, skończoną liczbą dni")
    if form_window < 0:
        raise ValueError("Okno formy musi być nieujemną liczbą meczów")
    if prediction_from is not None and prediction_from.tzinfo is None:
        raise ValueError("Początek prognoz musi zawierać strefę czasową")

    ratings: dict[str, tuple[float, datetime]] = {}
    predictions: list[Prediction] = []
    publish = predictions.append if on_prediction is None else on_prediction
    initial_rating = parameters.initial_rating
    # Oczekiwanie z chwili startu czeka na wynik tej samej serii.
    pre_match_expectations: dict[str, tuple[datetime, float]] = {}
    recent_form: dict[str, list[tuple[datetime, str, float]]] = {}
    for at, result_available, match in events:
        should_publish = prediction_from is None or at >= prediction_from
        if not result_available and not should_publish and not form_window:
            continue
        match_id = match["match_id"]
        team_a, team_b = match["team_a_id"], match["team_b_id"]
        rating_a = _rating_at(ratings.get(team_a, (initial_rating, at)), at, half_life_days, initial_rating)
        rating_b = _rating_at(ratings.get(team_b, (initial_rating, at)), at, half_life_days, initial_rating)
        winner = match["winner_team_id"]

        if result_available:
            # Dopiero dostępny wynik zapisuje nowe ratingi i rozpoczyna kolejny okres przerwy.
            team_a_won = winner == team_a
            try:
                map_score = _map_score(match) if score_margin else None
                updated_a, updated_b = update_ratings(
                    rating_a,
                    rating_b,
                    team_a_won,
                    map_score=map_score,
                    k_factor=parameters.k_factor,
                    elo_scale=parameters.elo_scale,
                    score_margin_weight=parameters.score_margin_weight,
                )
            except ValueError as error:
                raise ValueError(f"Mecz {match_id}. {error}") from error
            ratings[team_a] = (updated_a, at)
            ratings[team_b] = (updated_b, at)

            if form_window:
                starts_at, expected = pre_match_expectations.pop(match_id)
                residual = float(team_a_won) - expected
                for team, adjustment in ((team_a, residual), (team_b, -residual)):
                    history = recent_form.setdefault(team, [])
                    # Opóźniony wynik trafia do okna według chwili rozpoczęcia meczu.
                    insort(history, (starts_at, match_id, adjustment))
                    if len(history) > form_window:
                        history.pop(0)
        else:
            probability_a = win_probability(rating_a, rating_b, elo_scale=parameters.elo_scale)
            if form_window:
                pre_match_expectations[match_id] = (at, probability_a)
            if not should_publish:
                continue
            if form_window:
                form_a = form_rating_bonus(
                    (residual for _, _, residual in recent_form.get(team_a, [])),
                    form_window,
                    form_elo_weight=parameters.form_elo_weight,
                )
                form_b = form_rating_bonus(
                    (residual for _, _, residual in recent_form.get(team_b, [])),
                    form_window,
                    form_elo_weight=parameters.form_elo_weight,
                )
                probability_a = win_probability(
                    rating_a + form_a, rating_b + form_b, elo_scale=parameters.elo_scale
                )

            publish(
                Prediction(
                    match_id=match_id,
                    predicted_at=at,
                    team_a_id=team_a,
                    team_b_id=team_b,
                    winner_team_id=winner,
                    probability_a=probability_a,
                )
            )

    if not events:
        return predictions, {}
    # Porównujemy wszystkie drużyny na tę samą chwilę w symulacji.
    last_event_at = events[-1][0]
    final_ratings = {
        team: _rating_at(saved, last_event_at, half_life_days, initial_rating)
        for team, saved in ratings.items()
    }
    return predictions, final_ratings


def replay(
    path: Path,
    half_life_days: Optional[float] = None,
    *,
    score_margin: bool = False,
    form_window: int = 0,
    parameters: EloParameters = EloParameters(),
) -> None:
    """Pokazuje prognozy przedmeczowe, rzeczywiste wyniki i końcowe ratingi."""
    predictions, final_ratings = replay_events(
        read_events(path), half_life_days,
        score_margin=score_margin, form_window=form_window, parameters=parameters,
    )
    for prediction in predictions:
        probability_a = prediction.probability_a
        print(
            f"{prediction.match_id} | {prediction.team_a_id} {probability_a:.1%} | "
            f"{prediction.team_b_id} {1 - probability_a:.1%} | wygrana {prediction.winner_team_id}"
        )

    print("\nKońcowe ratingi Elo")
    for team, rating in sorted(final_ratings.items(), key=lambda entry: (-entry[1], entry[0])):
        print(f"{team}  {rating:.1f}")


def add_feature_arguments(parser: argparse.ArgumentParser) -> None:
    """Dodaje opcje wyniku map i formy do poleceń silnika."""
    parser.add_argument("--score-margin", action="store_true", help="Uwzględnij przewagę map w Elo")
    parser.add_argument(
        "--form-window", type=int, default=0, help="Liczba ostatnich meczów w formie (0 wyłącza)"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Odtwórz prognozy Elo z historii meczów")
    parser.add_argument("history_csv", type=Path, help="Ścieżka do lokalnego CSV z meczami")
    parser.add_argument(
        "--half-life-days",
        type=float,
        help="Liczba dni, po których odchylenie Elo od 1500 maleje o połowę",
    )
    add_feature_arguments(parser)
    args = parser.parse_args()
    try:
        replay(
            args.history_csv,
            args.half_life_days,
            score_margin=args.score_margin,
            form_window=args.form_window,
        )
    except (OSError, ValueError, csv.Error) as error:
        parser.error(str(error))


#przykladowe dzialanie na ewentach z pliku lokalnego CSV:
#fake-001 | onyx-falcons 50.0% | copper-wolves 50.0% | wygrana onyx-falcons
#fake-002 | ivory-comets 50.0% | violet-foxes 50.0% | wygrana ivory-comets
#fake-003 | violet-foxes 45.4% | onyx-falcons 54.6% | wygrana onyx-falcons
#fake-004 | copper-wolves 45.4% | ivory-comets 54.6% | wygrana copper-wolves
#fake-005 | onyx-falcons 54.6% | ivory-comets 45.4% | wygrana onyx-falcons
#fake-006 | copper-wolves 54.6% | violet-foxes 45.4% | wygrana violet-foxes
#fake-007 | onyx-falcons 58.3% | violet-foxes 41.7% | wygrana violet-foxes
#fake-008 | ivory-comets 50.0% | copper-wolves 50.0% | wygrana ivory-comets
#fake-009 | copper-wolves 45.4% | silver-lynx 54.6% | wygrana copper-wolves
#fake-010 | copper-wolves 44.1% | onyx-falcons 55.9% | wygrana onyx-falcons
#fake-011 | silver-lynx 46.7% | violet-foxes 53.3% | wygrana silver-lynx
#fake-012 | onyx-falcons 55.9% | silver-lynx 44.1% | wygrana onyx-falcons

#Końcowe ratingi Elo
#onyx-falcons  1554.7
#ivory-comets  1500.0
#violet-foxes  1488.5
#silver-lynx  1485.5
#copper-wolves  1471.3
