import time
import pandas as pd
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
    df["MIN_SHARE"] = (df["MIN"] / 240.0).clip(upper=0.20)
    df["PLAYER_RATING"] = (df["PIE"] * 100 * 0.6 + (df["NET_RATING"] + 15) * 0.4) * (
        df["MIN"] / 36.0
    )
    return df


# Póki co sprawdzam na jednym sezonie czy wszystko śmiga
if __name__ == "__main__":
    df_test = fetch_nba_info(
        start_year=2024, end_year=2024, season_types=("Regular Season",)
    )
    print("Wymiary tabeli:", df_test.shape)
    print(df_test[["GAME_DATE", "MATCHUP", "PTS", "OFF_RATING", "PACE"]].head())

    df2_test = fetch_player_ratings(season="2026-27")
    print("Wymiary tabeli: ", df2_test.shape)
    print(df2_test[["PLAYER_ID", "PLAYER_NAME", "PIE"]].head())
