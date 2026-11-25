import time
import pandas as pd
import numpy as np
from nba_api.stats.endpoints import teamgamelogs, leaguedashplayerstats

BASE_SIM_COLS = [
    "OFF_RATING",
    "DEF_RATING",
    "PACE",
    "EFG_PCT",
    "TS_PCT",
    "TM_TOV_PCT",
    "OREB_PCT",
    "PIE",
]
STATS_COLS = BASE_SIM_COLS + ["NET_RATING"]
adv_cols = ["GAME_ID", "TEAM_ID"] + STATS_COLS


# Funkcja odpowiedzialna za pobranie wszystkich meczów z NBA z lat 2018 do 2026
def fetch_nba_info(
    start_year: int = 2018,
    end_year: int = 2026,
    season_types: tuple = ("Regular Season", "Playoffs"),
) -> pd.DataFrame:
    all_info = []

    for year in range(start_year, end_year + 1):
        season_str = f"{year}-{str(year + 1)[-2:]}"  # to aktualny sezon który pobieramy
        # Pobieramy statystyki zarówno z playoffów jak i sezonu regularnego
        for s_type in season_types:
            try:
                # Pobranie statystyk podstawowych tj. PTS, FGA, FTA, OREB, TOV, WL, MATCHUP
                base_df = teamgamelogs.TeamGameLogs(
                    season_nullable=season_str,
                    season_type_nullable=s_type,
                    measure_type_player_game_logs_nullable="Base",
                    timeout=60,
                ).get_data_frames()[0]
                time.sleep(0.7)  # Krótka pauza dla bezpieczeństwa IP

                # Pobranie bardziej zaawansowanych statystyk (dokładne ORtg, DRtg, Pace, eFG%, TS%, PIE)
                adv_df = teamgamelogs.TeamGameLogs(
                    season_nullable=season_str,
                    season_type_nullable=s_type,
                    measure_type_player_game_logs_nullable="Advanced",
                    timeout=60,
                ).get_data_frames()[0]
                time.sleep(0.7)  # Znowu czekamy chwilę dla bezpieczeństwa IP

                if base_df.empty or adv_df.empty:
                    continue

                # Łączymy wszystkie cechy do jednej tabeli wybierając tylko
                # pożądane (z adv_cols)
                merged = pd.merge(
                    base_df, adv_df[adv_cols], on=["GAME_ID", "TEAM_ID"], how="inner"
                )
                all_info.append(merged)

            except Exception as e:
                print(f"Błąd podczas pobierania {season_str} ({s_type}): {e}")
                time.sleep(3)  # W razie przycięcia serwerów odczekujemy chwilę

    # Jeżeli nasze informacje sa puste gdzieś popełniliśmy błąd
    if not all_info:
        raise RuntimeError("Nie udało się pobrać żadnych danych")

    return pd.concat(all_info, ignore_index=True)


def fetch_player_ratings(season: str = "2026-27") -> pd.DataFrame:
    # Pobieranie statystyk aktywnych graczy w sezonie i liczy ich rating (wpływ na grę)
    df = leaguedashplayerstats.LeagueDashPlayerStats(
        season=season,
        measure_type_detailed_defense="Advanced",
        per_mode_detailed="PerGame",
        timeout=60,
    ).get_data_frames()[0]
    time.sleep(0.7)

    cols = [
        "PLAYER_ID",
        "PLAYER_NAME",
        "TEAM_ABBREVIATION",
        "GP",
        "MIN",
        "OFF_RATING",
        "DEF_RATING",
        "NET_RATING",
        "PIE",
        "USG_PCT",
    ]
    df = df[cols].copy()

    # Pomijamy osoby, które rozegrały osoby "3 świetne minuty" aby nie zanieczyścić danych
    # Aby osoba liczyła się do danych musi mięc rozegrane 3 gry i co najmniej 10 minut
    small_sample = (df["MIN"] < 10.0) | (df["GP"] < 3)
    df.loc[small_sample, "PIE"] = df.loc[small_sample, "PIE"].clip(upper=0.10)

    df["MIN_SHARE"] = (df["MIN"] / 240.0).clip(upper=0.20)
    df["PLAYER_RATING"] = (df["PIE"] * 100 * 0.6 + (df["NET_RATING"] + 15) * 0.4) * (
        df["MIN"] / 36.0
    )
    return df


def calculate_missing_penalty(
    players_df: pd.DataFrame | None,
    team_abbr: str,
    missing_players: list[str] | None,
) -> tuple[float, float]:
    # Wylicza korektę (off_penalty, def_penalty) za brakujących zawodników.
    if players_df is None or not missing_players:
        return 0.0, 0.0

    team_roster = players_df[players_df["TEAM_ABBREVIATION"] == team_abbr.upper()]
    out_df = team_roster[team_roster["PLAYER_NAME"].isin(missing_players)]
    if out_df.empty:
        return 0.0, 0.0
    # Brak zawodnika jest oparty na PIE czyli Player Impact Estimate
    # Gdy gwiazda odpada (ok 0.18 PIE) wproawdzamy za niego srednia wartosc
    # ligowa czyli 0.10. Jeżeli wchodzi ktos ta wartość jest poniżej czyli wypadł
    # rezerwowy albo słaby zawodnik jego brak nie osłabia drużyny (kara = 0)
    # Całość skalujemy do przez 240 bo 5 * 48 (12 minut jedna kwarta).
    # Całość skalujemy potem do realny punktów na 100 posiadań
    # Dodatkowo ograniczamy PIE do maksymalnego 0.4 które jest i tak nie możliwym wynikiem
    impact_loss = (
        (out_df["PIE"].clip(upper=0.4) - 0.10).clip(lower=0)
        * out_df["MIN_SHARE"]
        * 250.0
    ).sum()

    # A następnie rozbijamy na znaczenie w ofensywie i defensywie
    off_penalty = -float(impact_loss * 0.6)
    def_penalty = float(impact_loss * 0.4)
    return off_penalty, def_penalty


def add_elo_ratings(
    games: pd.DataFrame,
    k_factor: float = 20.0,  # Ile dostanie punktów dostanie zwycięska drużyna
    home_adv: float = 70.0,  # Ile "dodatwkoych" punktów daje fakt bycia gospodarzem
) -> tuple[pd.DataFrame, dict]:

    games = games.sort_values("GAME_DATE").reset_index(drop=True)
    elo: dict[str, float] = {}
    last_season = None
    elo_home, elo_away = [], []

    # Przeliczamy po wszystkich meczach aby zrobić rzetelny ranking elo
    for row in games.itertuples():
        # Robimy soft reset dla nowego sezonu z uwagi na zmianę składów i układów
        # oraz ogólny zastój w graniu
        if last_season and row.SEASON_YEAR != last_season:
            for team, rating in elo.items():
                elo[team] = 0.75 * rating + 375.0
        last_season = row.SEASON_YEAR

        h_team = row.TEAM_ABBREVIATION_HOME
        a_team = row.TEAM_ABBREVIATION_AWAY

        # pobieramy rankingi meczowe przed meczem, jeżeli nie mają bierzemy 1500
        h_val = elo.get(h_team, 1500.0)
        a_val = elo.get(a_team, 1500.0)

        elo_home.append(h_val)
        elo_away.append(a_val)

        # Obliczamy szansę drużyny na wygranie i na tej podstawie wyliczamy
        # możliwe punkty elo do przyznania
        prob_home = 1.0 / (1.0 + 10.0 ** ((a_val - (h_val + home_adv)) / 400.0))
        shift = k_factor * (row.TARGET_HOME_WIN - prob_home)

        # Aktualizujemy punkty elo drużyny
        elo[h_team] = h_val + shift
        elo[a_team] = a_val - shift

    # Aktualizujemy punkty elo
    games["ELO_HOME"] = elo_home
    games["ELO_AWAY"] = elo_away
    games["DIFF_ELO"] = games["ELO_HOME"] - games["ELO_AWAY"]
    # Zwracamy tabelę do treningu
    return games, elo


def update_elo_single_game(
    elo: dict[str, float],
    home_team: str,
    away_team: str,
    home_won: bool,
    k_factor: float = 20.0,
    home_adv: float = 70.0,
) -> dict[str, float]:
    # Robimy to samo co w funkcji wyżej tylko w momencie aktualizacji wyników nie Przeliczamy
    # całej historii od nowa tylko robimy aktualizację elo o roozegrany mecz
    h_team = home_team.upper()
    a_team = away_team.upper()

    h_val = elo.get(h_team, 1500.0)
    a_val = elo.get(a_team, 1500.0)

    prob_home = 1.0 / (1.0 + 10.0 ** ((a_val - (h_val + home_adv)) / 400.0))
    shift = k_factor * (float(home_won) - prob_home)

    elo[h_team] = h_val + shift
    elo[a_team] = a_val - shift
    return elo


def prepare_training_data(df: pd.DataFrame) -> tuple[pd.DataFrame, list[str], dict]:
    # Tworzymy gotowy zbiór treningowy do wytrenowania modelu z kopi DataFrame
    df = df.copy()

    # Konwertujemy GAME_DATE na datę i sotujemy do średnich kroczących
    df["GAME_DATE"] = pd.to_datetime(df["GAME_DATE"])
    df = df.sort_values(["TEAM_ID", "GAME_DATE"]).reset_index(drop=True)

    # Stwierdzamy:
    # - czy drużyna grała u siebie czy na wyjeździe
    # - ile miała dni odpoczynku (max 7 dni bo potem nie ma sensu trzymać większej ilości dni)
    # - czy gra dzień po dniu
    df["IS_HOME"] = df["MATCHUP"].str.contains("vs.").astype(int)
    df["DAYS_REST"] = (
        df.groupby("TEAM_ID")["GAME_DATE"].diff().dt.days.fillna(7).clip(upper=7)
    )
    df["IS_B2B"] = (df["DAYS_REST"] == 1).astype(int)

    # Tworzymy średnie kroczące z 5 i 15 meczów mówiące o dyspozycji danego zespołu
    for col in STATS_COLS:
        shifted = df.groupby("TEAM_ID")[col].shift(1)
        df[f"{col}_EWM5"] = (
            shifted.groupby(df["TEAM_ID"])
            .ewm(span=5, min_periods=3)
            .mean()
            .reset_index(level=0, drop=True)
        )
        df[f"{col}_EWM15"] = (
            shifted.groupby(df["TEAM_ID"])
            .ewm(span=15, min_periods=5)
            .mean()
            .reset_index(level=0, drop=True)
        )

    feature_base = [c for c in df.columns if "_EWM" in c] + ["DAYS_REST", "IS_B2B"]
    keep_cols = [
        "GAME_ID",
        "GAME_DATE",
        "SEASON_YEAR",
        "TEAM_ABBREVIATION",
        "PTS",
    ] + feature_base

    # Rodzielamy tabelę na statystyki gości i gospodarzy a następnie łączymy po id
    # dzięki temu jeden wiersz to całe info o meczu
    home_df = df[df["IS_HOME"] == 1][keep_cols]
    away_df = df[df["IS_HOME"] == 0][keep_cols]

    games = pd.merge(
        home_df,
        away_df,
        on=["GAME_ID", "GAME_DATE", "SEASON_YEAR"],
        suffixes=("_HOME", "_AWAY"),
        how="inner",
    )

    # Ponieważ drzewa XGBoost działają programi kolumn uzywamy różnic do celniejszego oceniania
    # przewagi
    games["DIFF_NET_RTG_15"] = (
        games["NET_RATING_EWM15_HOME"] - games["NET_RATING_EWM15_AWAY"]
    )
    games["DIFF_NET_RTG_5"] = (
        games["NET_RATING_EWM5_HOME"] - games["NET_RATING_EWM5_AWAY"]
    )
    games["DIFF_REST"] = games["DAYS_REST_HOME"] - games["DAYS_REST_AWAY"]
    games["EXPECTED_PACE"] = (games["PACE_EWM15_HOME"] + games["PACE_EWM15_AWAY"]) / 2

    # Ustalnenie któ©a drużyna wygrała
    games["TARGET_HOME_WIN"] = (games["PTS_HOME"] > games["PTS_AWAY"]).astype(int)

    # Liczymy ranking elo dla drużyn
    games, current_elo = add_elo_ratings(games)

    # Usuwamy mecze które mają wartość NaN
    games = games.dropna().sort_values("GAME_DATE").reset_index(drop=True)

    # Filtrujemy identyfikatory, nazwy żeby nie był← cechą predykcyjną
    exclude = {
        "GAME_ID",
        "GAME_DATE",
        "SEASON_YEAR",
        "TEAM_ABBREVIATION_HOME",
        "TEAM_ABBREVIATION_AWAY",
        "PTS_HOME",
        "PTS_AWAY",
        "TARGET_HOME_WIN",
    }
    feature_cols = [c for c in games.columns if c not in exclude]

    # Zwracamy gotową tabelę listę kolumn wejściowych i aktualne punkty elo
    return games, feature_cols, current_elo


def build_matchup_features(
    df: pd.DataFrame,
    current_elo: dict,
    home_team: str,
    away_team: str,
    players_df: pd.DataFrame | None = None,
    missing_home: list[str] | None = None,
    missing_away: list[str] | None = None,
    match_date: str | pd.Timestamp | None = None,
) -> tuple[pd.Series, dict[str, np.ndarray]]:
    # Zmeiniamy na typ danych na datę i normalizujemy
    target_date = (
        pd.to_datetime(match_date) if match_date else pd.Timestamp.today().normalize()
    )
    teams = {home_team.upper(), away_team.upper()}

    # Filtrujemy od razu 2 drużyny zamiast kopiować i parsować całą tabelę
    sub_df = df[df["TEAM_ABBREVIATION"].isin(teams)].copy()
    sub_df["GAME_DATE"] = pd.to_datetime(sub_df["GAME_DATE"])

    # Bierzemy mecze przed datą meczu aby uniknąć wycieku danych
    sub_df = sub_df[sub_df["GAME_DATE"] < target_date].sort_values("GAME_DATE")

    features = {}
    cov_matrices = {}

    # Wyliczamy cechy dla gości i gospodarzy
    for team, suffix, missing in [
        (home_team.upper(), "HOME", missing_home),
        (away_team.upper(), "AWAY", missing_away),
    ]:
        team_df = sub_df[sub_df["TEAM_ABBREVIATION"] == team]
        # Żeby mieć średnie korczące z 15 meczy to tyle meczy musi się odbyć
        if len(team_df) < 15:
            raise ValueError(f"Za mało danych historycznych dla drużyny: {team}")

        # Liczymy dni przerwy do meczu
        days_rest = int(
            np.clip((target_date - team_df["GAME_DATE"].iloc[-1]).days, 1, 7)
        )
        features[f"DAYS_REST_{suffix}"] = days_rest
        # Sprawdzamy czy mecz jest back to back
        features[f"IS_B2B_{suffix}"] = int(days_rest == 1)
        # Wyciągamy elo drużyny
        features[f"ELO_{suffix}"] = current_elo.get(team, 1500.0)

        # Liczymy kary ofensywne i defensywne na podstawie brakujących zawodników
        off_pen, def_pen = calculate_missing_penalty(players_df, team, missing)

        # Liczymy EWM naraz dla wszystkich kolumn z BASE_SIM_COLS
        for span, min_p in ((5, 3), (15, 5)):
            ewm = (
                team_df[BASE_SIM_COLS].ewm(span=span, min_periods=min_p).mean().iloc[-1]
            )
            ewm["OFF_RATING"] += off_pen
            ewm["DEF_RATING"] += def_pen
            ewm["NET_RATING"] = ewm["OFF_RATING"] - ewm["DEF_RATING"]
            features.update(
                {f"{col}_EWM{span}_{suffix}": float(val) for col, val in ewm.items()}
            )
        # Uzupełniamy macierze kowariancji potrzebne do metody Monte Carlo (TO-DO)
        # Mnożenie 1e-6 jest po to aby macierz była określona
        cov = np.nan_to_num(team_df[BASE_SIM_COLS].tail(15).cov().values, nan=0.0)
        cov_matrices[suffix] = cov + np.eye(len(BASE_SIM_COLS)) * 1e-6

    # Updatejumy nasze features
    features.update(
        {
            "DIFF_ELO": features["ELO_HOME"] - features["ELO_AWAY"],
            "DIFF_NET_RTG_15": features["NET_RATING_EWM15_HOME"]
            - features["NET_RATING_EWM15_AWAY"],
            "DIFF_NET_RTG_5": features["NET_RATING_EWM5_HOME"]
            - features["NET_RATING_EWM5_AWAY"],
            "DIFF_REST": features["DAYS_REST_HOME"] - features["DAYS_REST_AWAY"],
            "EXPECTED_PACE": (features["PACE_EWM15_HOME"] + features["PACE_EWM15_AWAY"])
            / 2,
        }
    )
    # Zwracamy cechy do predykcji
    return pd.Series(features), cov_matrices


# Póki co sprawdzam na jednym sezonie czy wszystko śmiga
if __name__ == "__main__":
    df_test = fetch_nba_info(
        start_year=2025, end_year=2025, season_types=("Regular Season",)
    )
    print("Wymiary tabeli:", df_test.shape)
    print(df_test[["GAME_DATE", "MATCHUP", "PTS", "OFF_RATING", "PACE"]].head())

    df2_test = fetch_player_ratings(season="2026-27")
    print("Wymiary tabeli: ", df2_test.shape)
    print(df2_test[["PLAYER_ID", "PLAYER_NAME", "PIE"]].head())

    games_test, feature_cols, current_elo = prepare_training_data(df_test)
    print("Wymiary tabeli treningowej:", games_test.shape)
    print("Liczba cech wejściowych (feature_cols):", len(feature_cols))
    print("Liczba braków (NaN) w zbiorze:", games_test.isna().sum().sum())
    print("\nTop 5 drużyn wg ELO po sezonie:")
    print(pd.Series(current_elo).sort_values(ascending=False).head(5).round(1))

    star = df2_test.sort_values("PIE", ascending=False).iloc[0]
    off_pen, def_pen = calculate_missing_penalty(
        players_df=df2_test,
        team_abbr=star["TEAM_ABBREVIATION"],
        missing_players=[star["PLAYER_NAME"]],
    )
    print(
        f"Brak gracza: {star['PLAYER_NAME']} ({star['TEAM_ABBREVIATION']}, "
        f"PIE = {star['PIE']})"
    )

    feats, covs = build_matchup_features(
        df=df_test,
        current_elo=current_elo,
        home_team="NYK",
        away_team="SAS",
        players_df=df2_test,
        match_date="2026-06-03",
    )

    assert set(feats.index) == set(feature_cols), "Niezgodność kolumn!"
    print("\n(NYK vs SAS)")
    print("Liczba cech:", len(feats), "| Braki (NaN):", int(feats.isna().sum()))
    print("Macierz kowariancji HOME:", covs["HOME"].shape)
    print(feats[["ELO_HOME", "ELO_AWAY", "DIFF_ELO", "DIFF_NET_RTG_15"]].round(2))
