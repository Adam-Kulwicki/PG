import optuna
import pandas as pd
from sklearn.metrics import accuracy_score, brier_score_loss, log_loss
from xgboost import XGBClassifier
from sklearn.model_selection import TimeSeriesSplit

# Mute logów optuny bo bardzo długie (poza warningami i błędami)
optuna.logging.set_verbosity(optuna.logging.WARNING)


# Funkcja do optymalizacji hiperparametrów za pomocą Optuny
def tune_hyperparameters(
    X_train: pd.DataFrame,
    y_train: pd.Series,
    n_trials: int = 50,
) -> dict:
    tscv = TimeSeriesSplit(n_splits=5)

    def objective(trial: optuna.Trial) -> float:
        params = {
            "n_estimators": trial.suggest_int("n_estimators", 100, 600),
            "max_depth": trial.suggest_int("max_depth", 2, 5),
            "learning_rate": trial.suggest_float("learning_rate", 0.01, 0.1, log=True),
            "subsample": trial.suggest_float("subsample", 0.6, 1.0),
            "colsample_bytree": trial.suggest_float("colsample_bytree", 0.5, 1.0),
            "min_child_weight": trial.suggest_int("min_child_weight", 1, 20),
            "reg_alpha": trial.suggest_float("reg_alpha", 1e-3, 10.0, log=True),
            "reg_lambda": trial.suggest_float("reg_lambda", 1e-3, 10.0, log=True),
            "eval_metric": "logloss",
            "random_state": 42,
        }

        scores = []
        for tr_idx, val_idx in tscv.split(X_train):
            model = XGBClassifier(**params)
            model.fit(
                X_train.iloc[tr_idx],
                y_train.iloc[tr_idx],
                verbose=False,
            )
            probs = model.predict_proba(X_train.iloc[val_idx])[:, 1]
            scores.append(log_loss(y_train.iloc[val_idx], probs))

        return float(sum(scores) / len(scores))

    study = optuna.create_study(direction="minimize")
    study.optimize(objective, n_trials=n_trials, show_progress_bar=True)
    return study.best_params


def train_model(
    games: pd.DataFrame,
    feature_cols: list[str],
    current_elo: dict[str, float] | None = None,
    model_dir: str = "models",
    tune: bool = False,
) -> tuple[XGBClassifier, dict]:
    # Podział chronologiczny bez losowania, żeby uniknąć data leakage
    split_idx = int(len(games) * 0.8)
    train_df, test_df = games.iloc[:split_idx], games.iloc[split_idx:]

    X_train, y_train = train_df[feature_cols], train_df["TARGET_HOME_WIN"]
    X_test, y_test = test_df[feature_cols], test_df["TARGET_HOME_WIN"]

    # Jeżeli chcemy optymalizacje to ustawiamy tune na true
    # w przeciwnym przypadku działamy na "baseline" czyli w miarę domyślnych
    # parametrach
    if tune:
        best_params = tune_hyperparameters(X_train, y_train, n_trials=50)
        model = XGBClassifier(**best_params, eval_metric="logloss", random_state=1)
    else:
        model = XGBClassifier(
            n_estimators=200,
            max_depth=3,
            learning_rate=0.03,
            subsample=0.8,
            colsample_bytree=0.8,
            eval_metric="logloss",
            random_state=1,
        )
    # Finalny trening na zbiorze
    model.fit(X_train, y_train, verbose=False)

    # Model baseline wygrywa gospodarz z prawdopodobieństwem wziętym ze zbioru
    # treningowego
    home_win_rate = float(y_train.mean())
    dummy_probs = [home_win_rate] * len(y_test)
    dummy_preds = [1] * len(y_test)

    # Ewaluacja na najnowszych 20% meczów - tych których jeszcze nie widział
    probs = model.predict_proba(X_test)[:, 1]
    preds = (probs >= 0.5).astype(int)

    # Metryki:
    # - accuracy: jaki % zwycięzców model wytypował poprawnie
    # - log_loss: unikanie sytuacji w której drużyna ma 90% na wygranie a przegrywa (mocne kary)
    # - brier_score: średni błąd kwadratowy
    # - dummy_accuracy: skuteczność baseline'u - to trzeba pobić
    metrics = {
        "train_games": len(train_df),
        "test_games": len(test_df),
        "dummy_accuracy": round(float(accuracy_score(y_test, dummy_preds)), 4),
        "dummy_log_loss": round(float(log_loss(y_test, dummy_probs)), 4),
        "accuracy": round(float(accuracy_score(y_test, preds)), 4),
        "log_loss": round(float(log_loss(y_test, probs)), 4),
        "brier_score": round(float(brier_score_loss(y_test, probs)), 4),
    }
    # TODO: Zapisać wytrenowany model, obecne elo i cechy
    return model, metrics


# TODO: dodać load_model() oraz dorzucić wrzucenie do cacha. Zrboić restrukturyzację pliku prepare_data.py i lepiej to podzielić
