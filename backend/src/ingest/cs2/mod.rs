// src/ingest/cs2/mod.rs
//! Counter-Strike 2 (hybryda):
//! - **Kaggle CSV** (`ektarr/counter-strike-pro-matches`) — historia meczów, dokładne rundy map,
//!   składy meczowe i statystyki graczy (automatycznie aktualizowane po wykryciu nowej wersji),
//! - **PandaScore Free** (`/csgo/*`) — terminarz, mecze `LIVE`, statusy `CANCELLED` oraz bieżące wyniki.

pub mod kaggle_csv;
pub mod pandascore;
pub mod teams;

pub use kaggle_csv::{
    extract_csv_bytes, infer_tier_from_filename, parse_kaggle_cs2_csv,
    parse_kaggle_dataset_version, sync_kaggle_csv, CS2_CACHE_CSV_PATH, CS2_CACHE_VERSION_PATH,
    KAGGLE_CS2_DATASET, KAGGLE_CS2_FILE,
};
pub use pandascore::{
    build_pandascore_past_range, parse_pandascore_matches, parse_pandascore_tournament_rosters,
    sync_pandascore,
};
pub use teams::normalize_cs2_team_name;

use crate::{error::AppResult, state::AppState};

/// Synchronizuje mecze CS2 z obu źródeł niezależnie (błąd jednego nie blokuje drugiego).
pub async fn sync(state: &AppState, include_history: bool) -> AppResult<usize> {
    let mut total_saved = 0;

    // 1. Najpierw PandaScore (terminarz i bieżące wyniki), następnie Kaggle CSV może je wzbogacić.
    match &state.config.pandascore_api_key {
        Some(api_key) => match sync_pandascore(state, api_key).await {
            Ok(count) => total_saved += count,
            Err(err) => tracing::warn!("PandaScore CS2: pominięto synchronizację: {err}"),
        },
        None => tracing::info!("Brak PANDASCORE_API_KEY – pomijam terminarz CS2 z PandaScore"),
    }

    match sync_kaggle_csv(state, include_history).await {
        Ok(count) => total_saved += count,
        Err(err) => tracing::warn!("Kaggle CS2 CSV: pominięto synchronizację: {err}"),
    }

    Ok(total_saved)
}
