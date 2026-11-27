"""Dobiera parametry Elo na walidacji i ocenia wybrany wariant na późniejszych meczach."""

import argparse
import csv
import json
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from itertools import product
from math import isfinite
from pathlib import Path
from tempfile import NamedTemporaryFile
from time import perf_counter
from typing import Callable, Optional, TextIO

from .elo import EloParameters
from .replay import Prediction, read_events, replay_events
from .report import Metrics, ScoreAccumulator


DATA_DIR = Path(__file__).resolve().parent / "data"
PREVIOUS_ANALYSIS_FROM = datetime(2026, 1, 1, tzinfo=timezone.utc)
PREVIOUS_ANALYSIS_THROUGH = datetime(2026, 11, 27, 11, 41, 8, tzinfo=timezone.utc)
DEFAULT_GRID = {
    "k_factors": "16,32,48,64",
    "half_life_days": "none,60,90,180",
    "score_margin_weights": "0,0.25,0.5",
    "form_windows": "0,3,5",
    "form_elo_weights": "25,50",
}


@dataclass(frozen=True)
class Configuration:
    """Parametry jednego niezależnego przebiegu replayu."""

    k_factor: float
    half_life_days: Optional[float]
    score_margin_weight: float
    form_window: int
    form_elo_weight: float

    def elo_parameters(self) -> EloParameters:
        return EloParameters(
            k_factor=self.k_factor,
            score_margin_weight=self.score_margin_weight,
            form_elo_weight=self.form_elo_weight,
        )

    def export(self) -> dict:
        return {
            **asdict(self.elo_parameters()),
            "half_life_days": self.half_life_days,
            "form_window": self.form_window,
        }


CLASSIC_ELO = Configuration(32.0, None, 0.0, 0, 0.0)


def _timestamp(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("Granice okresów wymagają strefy czasowej")
    return parsed.astimezone(timezone.utc)


def _grid_values(raw: str, convert: Callable, accepts: Callable, option: str) -> list:
    try:
        values = [convert(item.strip()) for item in raw.split(",")]
    except ValueError as error:
        raise ValueError(f"Niepoprawna lista w {option}") from error
    if not all(accepts(value) for value in values):
        raise ValueError(f"Niepoprawna wartość w {option}")
    return list(dict.fromkeys(values))


def _parse_grid(args: argparse.Namespace) -> dict:
    return {
        "k_factors": _grid_values(
            args.k_factors, float, lambda value: isfinite(value) and value >= 0, "--k-factors"
        ),
        "half_life_days": _grid_values(
            args.half_life_days,
            lambda value: None if value.lower() == "none" else float(value),
            lambda value: value is None or (isfinite(value) and value > 0),
            "--half-life-days",
        ),
        "score_margin_weights": _grid_values(
            args.score_margin_weights, float,
            lambda value: isfinite(value) and value >= 0, "--score-margin-weights",
        ),
        "form_windows": _grid_values(
            args.form_windows, int, lambda value: value >= 0, "--form-windows"
        ),
        "form_elo_weights": _grid_values(
            args.form_elo_weights, float,
            lambda value: isfinite(value) and value >= 0, "--form-elo-weights",
        ),
    }


def _configurations(grid: dict) -> list[Configuration]:
    # Każdy wariant z wyłączoną formą odpowiada tej samej parze okna i wagi.
    form_settings = dict.fromkeys(
        (window, weight) if window and weight else (0, 0.0)
        for window, weight in product(grid["form_windows"], grid["form_elo_weights"])
    )
    return [
        Configuration(k_factor, half_life_days, score_margin_weight, window, weight)
        for k_factor, half_life_days, score_margin_weight, (window, weight) in product(
            grid["k_factors"], grid["half_life_days"], grid["score_margin_weights"], form_settings
        )
    ]


def _periods(events: list, validation_from: datetime, test_from: datetime) -> tuple[dict, int]:
    """Liczy mecze według chwili startu oraz znajduje początek zdarzeń testowych."""
    periods = {
        name: {"matches": 0, "first_match_at": None, "last_match_at": None}
        for name in ("training", "validation", "test")
    }
    test_start = len(events)
    for index, (at, is_result, _) in enumerate(events):
        if at >= test_from and test_start == len(events):
            test_start = index
        if is_result:
            continue
        period = periods["training" if at < validation_from else "validation" if at < test_from else "test"]
        start = at.astimezone(timezone.utc).isoformat()
        if period["matches"] == 0:
            period["first_match_at"] = start
        period["matches"] += 1
        period["last_match_at"] = start
    for name, period in periods.items():
        if period["matches"] == 0:
            raise ValueError(f"Okres {name} musi zawierać przynajmniej jeden mecz")
    return periods, test_start


def _evaluate(
    events: list,
    configuration: Configuration,
    prediction_from: datetime,
    *,
    eligible_match_ids: Optional[set[str]] = None,
    uniform_scores: Optional[ScoreAccumulator] = None,
) -> Metrics:
    scores = ScoreAccumulator()

    def score(prediction: Prediction) -> None:
        if eligible_match_ids is not None and prediction.match_id not in eligible_match_ids:
            return
        won = prediction.winner_team_id == prediction.team_a_id
        scores.add(prediction.probability_a, won)
        if uniform_scores is not None:
            uniform_scores.add(0.5, won)

    replay_events(
        events,
        configuration.half_life_days,
        score_margin=configuration.score_margin_weight > 0,
        form_window=configuration.form_window,
        parameters=configuration.elo_parameters(),
        prediction_from=prediction_from,
        on_prediction=score,
    )
    return scores.metrics()


def _write_atomic(path: Path, write: Callable[[TextIO], None]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary_path = None
    try:
        with NamedTemporaryFile(
            mode="w", encoding="utf-8", newline="", dir=path.parent, delete=False
        ) as stream:
            temporary_path = Path(stream.name)
            write(stream)
        temporary_path.replace(path)
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)


def _write_results(stream: TextIO, ranked: list[tuple[Configuration, Metrics]]) -> None:
    first_configuration, first_scores = ranked[0]
    columns = ["rank", *first_configuration.export(), *asdict(first_scores)]
    writer = csv.DictWriter(stream, fieldnames=columns)
    writer.writeheader()
    for rank, (configuration, scores) in enumerate(ranked, start=1):
        writer.writerow({"rank": rank, **configuration.export(), **asdict(scores)})


def _metrics_line(metrics: Metrics) -> str:
    return (
        f"Brier {metrics.brier_score:.6f} | log loss {metrics.log_loss:.6f} | "
        f"accuracy {metrics.accuracy:.2%} | Brier skill {metrics.brier_skill_score:+.2%} | "
        f"mecze {metrics.matches}"
    )


def optimize(
    history: Path,
    validation_from: datetime,
    test_from: datetime,
    *,
    grid: dict,
    metric: str,
    output: Path,
    best_output: Path,
) -> dict:
    """Optymalizuje na walidacji i uruchamia test po wyborze parametrów."""
    if validation_from.tzinfo is None or test_from.tzinfo is None:
        raise ValueError("Granice okresów wymagają strefy czasowej")
    if validation_from >= test_from:
        raise ValueError("--validation-from musi poprzedzać --test-from")
    history_path, csv_path, json_path = (path.resolve() for path in (history, output, best_output))
    if len({history_path, csv_path, json_path}) != 3:
        raise ValueError("Historia, wyniki CSV i najlepsza konfiguracja wymagają osobnych plików")
    if (
        output.is_dir() or best_output.is_dir()
        or csv_path in json_path.parents or json_path in csv_path.parents
    ):
        raise ValueError("Ścieżki wyników muszą wskazywać pliki")
    metric = "brier" if metric == "brier-score" else metric
    score_field = {"log-loss": "log_loss", "brier": "brier_score"}[metric]

    started_at = perf_counter()
    events = read_events(history)
    periods, test_start = _periods(events, validation_from, test_from)
    validation_events = events[:test_start]
    # Etykieta walidacyjna musi już być znana przed wyborem konfiguracji na początek testu.
    available_match_ids = {match["match_id"] for _, is_result, match in validation_events if is_result}
    eligible_match_ids = {
        match["match_id"] for at, is_result, match in validation_events
        if not is_result and at >= validation_from and match["match_id"] in available_match_ids
    }
    if not eligible_match_ids:
        raise ValueError("Żaden wynik walidacyjny nie był dostępny przed --test-from")
    eligible_count = len(eligible_match_ids)
    excluded_count = periods["validation"]["matches"] - eligible_count
    periods["validation"]["scored_matches"] = eligible_count
    periods["validation"]["excluded_results_at_or_after_test_from"] = excluded_count

    configurations = _configurations(grid)
    total = len(configurations)
    if total == 0:
        raise ValueError("Siatka musi zawierać co najmniej jedną konfigurację")
    estimated_visits = len(validation_events) * total
    print(
        f"Training {periods['training']['matches']} | "
        f"validation {eligible_count}/{periods['validation']['matches']} ocenionych | "
        f"test {periods['test']['matches']}",
        flush=True,
    )
    if excluded_count:
        print(f"Wyniki {excluded_count} meczów walidacyjnych były dostępne od granicy testu.", flush=True)
    print(
        f"Grid search obejmie {total} konfiguracji i około {estimated_visits:,} "
        f"odtworzeń zdarzeń przed fazą testową. Kryterium {metric}.",
        flush=True,
    )
    results: list[tuple[Configuration, Metrics]] = []
    best_score = float("inf")
    progress_interval = max(1, total // 20)
    for index, configuration in enumerate(configurations, start=1):
        scores = _evaluate(
            validation_events, configuration, validation_from,
            eligible_match_ids=eligible_match_ids,
        )
        results.append((configuration, scores))
        best_score = min(best_score, getattr(scores, score_field))
        if index == 1 or index % progress_interval == 0 or index == total:
            print(f"Sprawdzono {index}/{total} | najlepsza walidacja {metric} {best_score:.6f}", flush=True)
    ranked = sorted(results, key=lambda result: getattr(result[1], score_field))
    best_configuration, validation_scores = ranked[0]

    uniform = ScoreAccumulator()
    classic_scores = _evaluate(events, CLASSIC_ELO, test_from, uniform_scores=uniform)
    if best_configuration == CLASSIC_ELO:
        selected_scores = classic_scores
    else:
        selected_scores = _evaluate(events, best_configuration, test_from)
    uniform_scores = uniform.metrics()

    previously_analyzed = sum(
        not is_result and max(test_from, PREVIOUS_ANALYSIS_FROM) <= at <= PREVIOUS_ANALYSIS_THROUGH
        for at, is_result, _ in events
    )
    note = (
        f"Test obejmuje {previously_analyzed} meczów z okresu analizowanego wcześniej "
        "(1 stycznia – 27 listopada 2026 r.). Ocena tego okresu jest retrospektywna."
        if previously_analyzed else
        "Ten przebieg używa testu wyłącznie do oceny po wyborze parametrów."
    )
    elapsed_seconds = perf_counter() - started_at
    best = {
        "parameters": best_configuration.export(),
        "validation": asdict(validation_scores),
        "test": {
            "selected": asdict(selected_scores),
            "classic_elo": asdict(classic_scores),
            "uniform_50_50": asdict(uniform_scores),
            "previously_analyzed_matches": previously_analyzed,
            "previous_analysis_from": PREVIOUS_ANALYSIS_FROM.isoformat(),
            "previous_analysis_through": PREVIOUS_ANALYSIS_THROUGH.isoformat(),
            "note": note,
        },
        "periods": periods,
        "experiment": {
            "history": str(history),
            "metric": metric,
            "validation_from": validation_from.isoformat(),
            "test_from": test_from.isoformat(),
            "grid": grid,
            "configurations": total,
            "accuracy_tie_credit": 0.5,
            "elapsed_seconds": elapsed_seconds,
        },
    }
    _write_atomic(output, lambda stream: _write_results(stream, ranked))
    _write_atomic(best_output, lambda stream: json.dump(best, stream, ensure_ascii=False, indent=2))

    print("\nNajlepsze konfiguracje na walidacji")
    for rank, (configuration, scores) in enumerate(ranked[:10], start=1):
        print(f"{rank:>2}. {configuration} | {_metrics_line(scores)}")
    print(f"Najlepsze parametry {best_configuration.export()}")
    print(f"Walidacja | {_metrics_line(validation_scores)}")
    print(f"Test wybranego wariantu | {_metrics_line(selected_scores)}")
    print(f"Test klasycznego Elo K=32 | {_metrics_line(classic_scores)}")
    print(f"Test prognozy 50/50 | {_metrics_line(uniform_scores)}")
    print(note)
    print(f"Czas optymalizacji i oceny wyniósł {elapsed_seconds:.2f} s")
    print(f"Wyniki zapisano w {output} oraz {best_output}")
    return best


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("history_csv", type=Path, help="CSV z historią meczów")
    parser.add_argument("--validation-from", required=True, help="Początek walidacji w ISO 8601")
    parser.add_argument("--test-from", required=True, help="Początek testu w ISO 8601")
    parser.add_argument("--metric", choices=("log-loss", "brier", "brier-score"), default="log-loss")
    parser.add_argument("--output", type=Path, default=DATA_DIR / "optimization_results.csv")
    parser.add_argument("--best-output", type=Path, default=DATA_DIR / "best_params.json")
    for name, default in DEFAULT_GRID.items():
        parser.add_argument(
            "--" + name.replace("_", "-"), default=default, help=f"Lista wartości, domyślnie {default}"
        )
    args = parser.parse_args()
    try:
        optimize(
            args.history_csv,
            _timestamp(args.validation_from),
            _timestamp(args.test_from),
            grid=_parse_grid(args),
            metric=args.metric,
            output=args.output,
            best_output=args.best_output,
        )
    except (OSError, ValueError, csv.Error, OverflowError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
