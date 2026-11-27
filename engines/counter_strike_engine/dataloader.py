"""Eksportuje zakończone mecze CS2 z backendu PG do CSV dla silnika Elo.

Plik wynikowy można przekazać wprost do replay lub report. Gdy API nie podaje
czasu dostępności wyniku, opcja --result-delay-hours określa przybliżenie
liczone od startu meczu.
"""

import argparse
import csv
import json
from collections.abc import Iterator
from datetime import datetime, timedelta, timezone
from math import isfinite
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import Optional
from urllib.parse import urlencode, urlsplit
from urllib.request import urlopen


PAGE_SIZE = 500
# replay czyta mecz, drużyny, czasy i wynik map. result_time_source opisuje pochodzenie czasu wyniku.
CSV_COLUMNS = (
    "match_id", "season", "game_version", "starts_at", "result_available_at",
    "team_a_id", "team_b_id", "winner_team_id", "maps_won_a", "maps_won_b",
    "result_time_source",
)


def _fetch_matches(api_url: str) -> Iterator[dict]:
    """Pobiera zakończone mecze CS2 stronami w kolejności zwracanej przez backend."""
    parts = urlsplit(api_url)
    if parts.scheme not in ("http", "https") or not parts.netloc or parts.query or parts.fragment:
        raise ValueError("Podaj bazowy adres HTTP lub HTTPS backendu PG")

    seen_ids = set()
    offset = 0
    while True:
        query = urlencode({"sport": "CS2", "status": "FINISHED", "limit": PAGE_SIZE, "offset": offset})
        url = f"{api_url.rstrip('/')}/api/v1/matches?{query}"
        with urlopen(url, timeout=30) as response:
            page = json.load(response)
        if not isinstance(page, list):
            raise ValueError("Backend PG powinien zwrócić listę meczów")

        for match in page:
            if not isinstance(match, dict) or type(match.get("id")) is not int:
                raise ValueError("Backend PG zwrócił mecz bez poprawnego ID")
            match_id = match["id"]
            # Powtórzone ID może oznaczać zmianę listy w bazie podczas pobierania kolejnych stron.
            if match_id in seen_ids:
                raise ValueError(f"API zwróciło ponownie mecz {match_id}. Ponów eksport")
            seen_ids.add(match_id)
            yield match

        # Krótsza strona oznacza koniec listy. offset liczymy po rekordach zwróconych przez API.
        if len(page) < PAGE_SIZE:
            return
        offset += len(page)


def _timestamp(value: str) -> datetime:
    """Zachowuje tę samą chwilę po przeliczeniu znacznika na UTC."""
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("Czas musi zawierać strefę czasową")
    return parsed.astimezone(timezone.utc)


def _history_row(match: dict, result_delay: Optional[timedelta]) -> Optional[tuple]:
    """Mapuje jeden rekord API na wiersz wejściowy silnika."""
    winner = match.get("winner")
    if match.get("sport") != "CS2" or match.get("status") != "FINISHED":
        return None
    # Dla Elo potrzebujemy jednego zwycięzcy, którego stronę zamienimy na ID drużyny.
    if winner not in ("HOME", "AWAY"):
        return None

    try:
        team_a = match["home_participant_id"]
        team_b = match["away_participant_id"]
        if any(type(team) is not int or team <= 0 for team in (team_a, team_b)) or team_a == team_b:
            raise ValueError("Mecz wymaga dwóch różnych identyfikatorów drużyn")
        starts_at = _timestamp(match["match_date"])

        # replay aktualizuje rating dopiero o result_at. Czas z API ma pierwszeństwo przed szacunkiem.
        reported_time = match.get("result_available_at")
        if reported_time is not None:
            result_at = _timestamp(reported_time)
            time_source = "reported"
        elif result_delay is not None:
            result_at = starts_at + result_delay
            time_source = "estimated"
        else:
            raise ValueError("Brak czasu dostępności wyniku. Podaj --result-delay-hours")
        if result_at <= starts_at:
            raise ValueError("Wynik musi być dostępny po starcie meczu")

        # Wynik główny CS2 to wygrane mapy. Wyniki rund znajdują się w match_details.maps.
        maps_a, maps_b = match.get("home_score"), match.get("away_score")
        # Puste pola map pozwalają użyć samego zwycięzcy do podstawowej aktualizacji Elo.
        if maps_a is None or maps_b is None or (maps_a, maps_b) == (0, 0):
            maps_a = maps_b = ""
        elif (
            any(type(score) is not int or score < 0 for score in (maps_a, maps_b))
            or maps_a == maps_b
            or (maps_a > maps_b) != (winner == "HOME")
        ):
            raise ValueError("Wynik map musi być zgodny ze zwycięzcą")

        return (
            match["id"], match.get("season") or "", "CS2", starts_at.isoformat(),
            result_at.isoformat(), team_a, team_b, team_a if winner == "HOME" else team_b,
            maps_a, maps_b, time_source,
        )
    except (AttributeError, KeyError, TypeError, ValueError, OverflowError) as error:
        raise ValueError(f"Mecz {match['id']}. {error}") from error


def export_history(api_url: str, output: Path, result_delay_hours: Optional[float] = None) -> int:
    """Zapisuje historię po pobraniu i sprawdzeniu wszystkich rekordów."""
    result_delay = None
    if result_delay_hours is not None:
        if not isfinite(result_delay_hours) or result_delay_hours <= 0:
            raise ValueError("Opóźnienie wyniku musi być dodatnią, skończoną liczbą godzin")
        result_delay = timedelta(hours=result_delay_hours)

    rows = []
    # Pobieramy i sprawdzamy całą historię przed otwarciem pliku wynikowego.
    for match in _fetch_matches(api_url):
        row = _history_row(match, result_delay)
        if row is not None:
            rows.append(row)
    if not rows:
        raise ValueError("Backend PG nie zwrócił zakończonych meczów CS2 ze zwycięzcą")

    output.parent.mkdir(parents=True, exist_ok=True)
    temporary_path = None
    try:
        # Zapis w tym samym katalogu pozwala podmienić eksport dopiero po zapisaniu wszystkich wierszy.
        with NamedTemporaryFile(
            mode="w", encoding="utf-8", newline="", dir=output.parent, delete=False
        ) as stream:
            temporary_path = Path(stream.name)
            writer = csv.writer(stream)
            writer.writerow(CSV_COLUMNS)
            writer.writerows(rows)
        temporary_path.replace(output)
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)
    return len(rows)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Pobierz historię CS2 z backendu PG")
    parser.add_argument("--api-url", default="http://localhost:3000", help="Bazowy adres backendu")
    parser.add_argument("--output", type=Path, required=True, help="Ścieżka do wynikowego CSV")
    parser.add_argument(
        "--result-delay-hours",
        type=float,
        help="Szacowany odstęp od startu do dostępności wyniku, w godzinach",
    )
    args = parser.parse_args()
    try:
        count = export_history(args.api_url, args.output, args.result_delay_hours)
    except (OSError, ValueError, OverflowError) as error:
        parser.error(str(error))
    print(f"Zapisano {count} meczów CS2 w {args.output}")
