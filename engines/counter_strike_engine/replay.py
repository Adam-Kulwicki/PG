"""Odtwarza lokalną historię meczów i pokazuje prognozy Elo. Wstepnie na podstawie loklanego pliku CSV"""
"""Planowane jest rozszerzenie o pobieranie historii meczów z API HLTV, ale obecnie nie jest to zaimplementowane."""

import argparse
import csv
from datetime import datetime
from pathlib import Path

from .elo import update_ratings, win_probability


# Każda drużyna dostaje ten rating przed pierwszym znanym wynikiem.
INITIAL_RATING = 1500.0
MATCH_COLUMNS = {
    "match_id", "starts_at", "result_available_at",
    "team_a_id", "team_b_id", "winner_team_id",
}


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
            events.append((starts_at, False, match))
            events.append((result_at, True, match))

    # Przy tym samym czasie start (False) jest przed wynikiem (True).
    # Prognoza korzysta więc tylko z wyników znanych przed rozpoczęciem meczu.
    return sorted(events, key=lambda event: (event[0], event[1], event[2]["match_id"]))


def replay(path: Path) -> None:
    """Pokazuje prognozy przedmeczowe, rzeczywiste wyniki i końcowe ratingi."""
    ratings: dict[str, float] = {}
    for _, result_available, match in read_events(path):
        team_a, team_b = match["team_a_id"], match["team_b_id"]
        rating_a = ratings.get(team_a, INITIAL_RATING)
        rating_b = ratings.get(team_b, INITIAL_RATING)
        winner = match["winner_team_id"]

        if result_available:
            ratings[team_a], ratings[team_b] = update_ratings(rating_a, rating_b, winner == team_a)
        else:
            probability_a = win_probability(rating_a, rating_b)
            print(
                f"{match['match_id']} | {team_a} {probability_a:.1%} | "
                f"{team_b} {1 - probability_a:.1%} | wygrana {winner}"
            )

    print("\nKońcowe ratingi Elo")
    for team, rating in sorted(ratings.items(), key=lambda entry: (-entry[1], entry[0])):
        print(f"{team}  {rating:.1f}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Odtwórz prognozy Elo z historii meczów")
    parser.add_argument("history_csv", type=Path, help="Ścieżka do lokalnego CSV z meczami")
    args = parser.parse_args()
    try:
        replay(args.history_csv)
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
