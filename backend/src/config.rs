// src/config.rs
//! Konfiguracja aplikacji wczytywana ze zmiennych środowiskowych (ustawianych w docker-compose.yml).

use std::env;

/// Źródło danych NBA wybierane zmienną `INGEST_NBA_SOURCE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NbaIngestSource {
    /// Historia i rozegrane mecze ze `stats.nba.com`, terminarz/kwarty (i fallback) z BALLDONTLIE.
    #[default]
    Auto,
    /// Wyłącznie `stats.nba.com/stats/teamgamelogs`.
    NbaStats,
    /// Wyłącznie `api.balldontlie.io` (tryb awaryjny).
    Balldontlie,
}

impl NbaIngestSource {
    /// Rozpoznaje tryb pracy importu NBA; nieznana wartość zwraca domyślny tryb `Auto`.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "nba_stats" | "nba-stats" | "nbastats" => Self::NbaStats,
            "balldontlie" => Self::Balldontlie,
            _ => Self::Auto,
        }
    }
}

/// Ustawienia serwera, bazy danych i kluczy do zewnętrznych API sportowych.
#[derive(Clone, Debug)]
pub struct Config {
    /// Adres połączenia z PostgreSQL (zmienna `DATABASE_URL`, wymagana).
    pub database_url: String,
    /// Adres, na którym nasłuchuje serwer (zmienna `HOST`, domyślnie `0.0.0.0`).
    pub host: String,
    /// Port serwera (zmienna `PORT`, domyślnie `3000`).
    pub port: u16,
    /// Klucz do Football-Data.org (opcjonalny – bez niego pomijamy bieżący sezon piłki nożnej).
    pub football_data_api_key: Option<String>,
    /// Klucz do BALLDONTLIE (opcjonalny – służy do terminarza, kwart, dokładnego `datetime` UTC i fallbacku historii).
    pub balldontlie_api_key: Option<String>,
    /// Czy uruchamiać cykliczny import danych sportowych (`INGEST_ENABLED`, domyślnie `true`).
    pub ingest_enabled: bool,
    /// Co ile minut powtarzać import bieżących danych (`INGEST_INTERVAL_MINUTES`, domyślnie 60).
    pub ingest_interval_minutes: u64,
    /// Sezony historyczne piłki nożnej z OpenFootball, np. `2023-24,2024-25`
    /// (`INGEST_FOOTBALL_SEASONS`; puste = dwa ostatnie zakończone sezony).
    pub ingest_football_seasons: Vec<String>,
    /// Sezony historyczne NBA podane rokiem startu, np. `2023,2024` (`INGEST_NBA_SEASONS`).
    pub ingest_nba_seasons: Vec<i32>,
    /// Strategia wyboru źródła danych NBA (`INGEST_NBA_SOURCE`, domyślnie `auto`).
    pub ingest_nba_source: NbaIngestSource,
    /// Lata historyczne tenisa z archiwum Sackmanna, np. `2023,2024` (`INGEST_TENNIS_YEARS`).
    pub ingest_tennis_years: Vec<i32>,
    /// Toury tenisowe do importu: `ATP`, `WTA` (`INGEST_TENNIS_TOURS`, domyślnie `ATP`).
    pub ingest_tennis_tours: Vec<String>,
    /// Identyfikator ligi Euroligi w TheSportsDB (`THESPORTSDB_EUROLEAGUE_ID`);
    /// bez niego Euroliga jest pomijana.
    pub thesportsdb_euroleague_id: Option<String>,
}

/// Wczytuje listę wartości rozdzielonych przecinkami; puste i niepoprawne elementy są pomijane.
fn env_list<T: std::str::FromStr>(name: &str) -> Vec<T> {
    env::var(name)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .filter_map(|item| item.parse::<T>().ok())
        .collect()
}

/// Wczytuje zmienną środowiskową jako `Option<String>`, traktując pusty napis jako brak wartości.
fn env_non_empty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl Config {
    /// Wczytuje konfigurację ze zmiennych środowiskowych.
    /// Panikuje z czytelnym komunikatem, jeśli brakuje `DATABASE_URL` lub `PORT` jest niepoprawny.
    pub fn from_env() -> Self {
        // Adres bazy jest jedyną zmienną bez wartości domyślnej
        let database_url = env::var("DATABASE_URL").expect(
            "BŁĄD KRYTYCZNY: Zmienna DATABASE_URL musi być ustawiona w docker-compose.yml!",
        );

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let port = env::var("PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse::<u16>()
            .expect("BŁĄD: Zmienna PORT musi być poprawną liczbą (u16)");

        // Klucze API są opcjonalne – puste zmienne z `.env` nie udają aktywnego klucza
        let football_data_api_key = env_non_empty("FOOTBALL_DATA_API_KEY");
        let balldontlie_api_key = env_non_empty("BALLDONTLIE_API_KEY");

        let ingest_enabled = env::var("INGEST_ENABLED")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0" | "no"))
            .unwrap_or(true);

        let ingest_interval_minutes = env::var("INGEST_INTERVAL_MINUTES")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(60);

        let ingest_nba_source = match env_non_empty("INGEST_NBA_SOURCE") {
            Some(raw) => {
                let parsed = NbaIngestSource::parse(&raw);
                if parsed == NbaIngestSource::Auto && !raw.trim().eq_ignore_ascii_case("auto") {
                    tracing::warn!(
                        value = %raw,
                        "Nieznana wartość INGEST_NBA_SOURCE – używam trybu auto"
                    );
                }
                parsed
            }
            None => NbaIngestSource::Auto,
        };

        let mut ingest_tennis_tours: Vec<String> = env_list::<String>("INGEST_TENNIS_TOURS")
            .into_iter()
            .map(|t| t.to_ascii_uppercase())
            .collect();
        if ingest_tennis_tours.is_empty() {
            ingest_tennis_tours.push("ATP".to_string());
        }

        Self {
            ingest_enabled,
            ingest_interval_minutes,
            ingest_football_seasons: env_list("INGEST_FOOTBALL_SEASONS"),
            ingest_nba_seasons: env_list("INGEST_NBA_SEASONS"),
            ingest_nba_source,
            ingest_tennis_years: env_list("INGEST_TENNIS_YEARS"),
            ingest_tennis_tours,
            thesportsdb_euroleague_id: env_non_empty("THESPORTSDB_EUROLEAGUE_ID"),
            database_url,
            host,
            port,
            football_data_api_key,
            balldontlie_api_key,
        }
    }

    /// Zwraca adres w formacie `host:port`, np. `0.0.0.0:3000`.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
