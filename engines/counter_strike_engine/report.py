"""Brier score i log loss mierzą jakość prognoz prawdopodobieństwa. 
W raporcie liczymy błąd dla każdego meczu i uśredniamy. W obu przypadkach mniej znaczy lepiej.
- Brier score to kwadrat różnicy między prognozą a wynikiem. 
Jeśli dajesz drużynie A 80% szans, jej wygrana daje błąd (0,8 − 1)² = 0,04, a przegrana (0,8 − 0)² = 0,64.
- Log loss to −ln(p), gdzie p jest szansą, którą model przypisał faktycznemu zwycięzcy. 
Jeśli A wygra, strata wynosi −ln(0,8) ≈ 0,223. Jeśli wygra B, wynosi −ln(0,2) ≈ 1,609. Ta metryka szczególnie mocno karze pewne, błędne prognozy."""


"""Porównuje prognozy Elo i zapisuje je do lokalnego CSV."""

import argparse
import csv
from datetime import datetime
from math import log
from pathlib import Path
from statistics import fmean
from tempfile import NamedTemporaryFile

from .replay import add_feature_arguments, read_events, replay_events


def _scores(probabilities: list[float], outcomes: list[bool]) -> tuple[float, float]:
    """Liczy Brier score i log loss dla prawdopodobieństwa wygranej drużyny A."""
    brier = fmean((probability - won) ** 2 for probability, won in zip(probabilities, outcomes))
    winner_probabilities = (
        probability if won else 1 - probability for probability, won in zip(probabilities, outcomes)
    )
    # Dolna granica pozwala policzyć log loss także przy prognozie równej zeru dla zwycięzcy.
    log_loss = fmean(-log(max(probability, 1e-15)) for probability in winner_probabilities)
    return brier, log_loss


def _write_csv(output: Path, columns: list[str], rows: list[tuple[object, ...]]) -> None:
    """Zastępuje raport po zapisaniu pełnego zestawu prognoz."""
    temporary_path = None
    try:
        with NamedTemporaryFile(
            mode="w", encoding="utf-8", newline="", dir=output.parent, delete=False
        ) as stream:
            temporary_path = Path(stream.name)
            writer = csv.writer(stream)
            writer.writerow(columns)
            writer.writerows(rows)
        temporary_path.replace(output)
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)


def report(
    history: Path,
    output: Path,
    test_from: datetime,
    half_life_days: float,
    *,
    score_margin: bool = False,
    form_window: int = 0,
) -> None:
    """Ocenia warianty na meczach zaczynających się od wyznaczonej chwili."""
    if test_from.tzinfo is None:
        raise ValueError("Początek oceny musi zawierać strefę czasową")
    if history.resolve() == output.resolve():
        raise ValueError("Historia i raport wymagają osobnych plików")

    # Każdy wariant odtwarza pełną historię, żeby użyć ratingów ze starszych meczów.
    events = read_events(history)
    baseline, _ = replay_events(events)
    decayed, _ = replay_events(events, half_life_days)
    decay_by_match = {prediction.match_id: prediction.probability_a for prediction in decayed}
    matches = [prediction for prediction in baseline if prediction.predicted_at >= test_from]
    if not matches:
        raise ValueError("Brak meczów rozpoczynających się w wybranym okresie oceny")

    models = [
        ("Elo", "prob_a_elo", [prediction.probability_a for prediction in matches]),
        (
            f"Elo z zanikiem ({half_life_days:g} dni)",
            "prob_a_decay",
            [decay_by_match[prediction.match_id] for prediction in matches],
        ),
        ("50/50", "prob_a_uniform", [0.5] * len(matches)),
    ]
    settings: dict[str, int] = {}
    if score_margin or form_window:
        enhanced, _ = replay_events(
            events, half_life_days, score_margin=score_margin, form_window=form_window
        )
        enhanced_by_match = {prediction.match_id: prediction.probability_a for prediction in enhanced}
        features = ["zanik"]
        if score_margin:
            features.append("wynik map")
        if form_window:
            features.append(f"forma N={form_window}")
        models.append((
            "Elo + " + " + ".join(features),
            "prob_a_enhanced",
            [enhanced_by_match[prediction.match_id] for prediction in matches],
        ))
        settings = {"score_margin": int(score_margin), "form_window": form_window}

    probability_columns = [column for _, column, _ in models]
    columns = [
        "match_id", "predicted_at", "team_a_id", "team_b_id", "winner_team_id",
        *probability_columns, "half_life_days", *settings,
    ]
    rows = []
    for index, prediction in enumerate(matches):
        probabilities = [values[index] for _, _, values in models]
        rows.append((
            prediction.match_id, prediction.predicted_at.isoformat(),
            prediction.team_a_id, prediction.team_b_id, prediction.winner_team_id,
            *probabilities, half_life_days, *settings.values(),
        ))
    _write_csv(output, columns, rows)

    outcomes = [prediction.winner_team_id == prediction.team_a_id for prediction in matches]
    model_width = max(28, max(len(name) for name, _, _ in models))
    print(f"Mecze od {test_from.isoformat()} włącznie — {len(matches)}")
    print(f"{'Model':<{model_width}} {'Brier score':>12} {'Log loss':>12}")
    for name, _, probabilities in models:
        brier, log_loss = _scores(probabilities, outcomes)
        print(f"{name:<{model_width}} {brier:>12.6f} {log_loss:>12.6f}")
    print(f"\nCSV zapisano w {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Oceń prognozy Elo i zapisz raport CSV")
    parser.add_argument("history_csv", type=Path, help="Ścieżka do CSV z historią meczów")
    parser.add_argument("--test-from", required=True, help="Początek oceny w ISO 8601 ze strefą czasową")
    parser.add_argument("--half-life-days", required=True, type=float, help="Okres półtrwania w dniach")
    parser.add_argument("--output", required=True, type=Path, help="Ścieżka do raportu CSV")
    add_feature_arguments(parser)
    args = parser.parse_args()
    try:
        test_from = datetime.fromisoformat(args.test_from.replace("Z", "+00:00"))
        report(
            args.history_csv,
            args.output,
            test_from,
            args.half_life_days,
            score_margin=args.score_margin,
            form_window=args.form_window,
        )
    except (OSError, ValueError, csv.Error) as error:
        parser.error(str(error))
