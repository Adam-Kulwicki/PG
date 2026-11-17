import time
import pandas as pd
from nba_api.stats.endpoints import teamgamelogs


# Funkcja odpowiedzialna za pobranie wszystkich meczów z NBA z lat 2018 do 2026
def fetch_nba_info(
    start_year: int = 2018,
    end_year: int = 2025,
    season_types: tuple = ("Regular Season", "Playoffs"),
) -> pd.DataFrame:
    all_info = []

    for year in range(start_year, end_year + 1):
        season_str = f"{year}-{str(year + 1)[-2:]}"  # to aktualny sezon który pobieramy
        # Pobieramy statystyki zarówno z playoffów jak i sezonu regularnego
        for s_type in season_types:
            try:
                ## Pobranie statystyk podstawowych tj. PTS, FGA, FTA, OREB, TOV, WL, MATCHUP
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
                time.sleep(0.7)  # Znwou czekamy chwilę dla bezpieczeństwa IP

                if base_df.empty or adv_df.empty:
                    continue

                # Wybieramy tylko unikalne kolumny z tabeli Advanced
                adv_cols = [
                    "GAME_ID",
                    "TEAM_ID",
                    "OFF_RATING",
                    "DEF_RATING",
                    "NET_RATING",
                    "PACE",
                    "EFG_PCT",
                    "TS_PCT",
                    "TM_TOV_PCT",
                    "OREB_PCT",
                    "PIE",
                ]
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


# Póki co sprawdzam na jednym sezonie czy wszystko śmiga
if __name__ == "__main__":
    df_test = fetch_nba_info(
        start_year=2024, end_year=2024, season_types=("Regular Season",)
    )
    print("Wymiary tabeli:", df_test.shape)
    print(df_test[["GAME_DATE", "MATCHUP", "PTS", "OFF_RATING", "PACE"]].head())
