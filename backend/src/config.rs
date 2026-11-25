// src/config.rs
//! Konfiguracja aplikacji wczytywana ze zmiennych środowiskowych (ustawianych w docker-compose.yml).

use std::env;

use rust_decimal::Decimal;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

use crate::domain::odds::default_bookmaker_margin;

/// Domyślny katalog zapisu dziennych plików logów (`LOG_DIR`).
pub const DEFAULT_LOG_DIR: &str = "logs";

/// Domyślny limit przechowywanych dziennych plików logów (`LOG_MAX_FILES`).
pub const DEFAULT_LOG_MAX_FILES: usize = 14;

/// Prefiks nazw plików logów tworzących wzorzec `backend.log.YYYY-MM-DD`.
pub const LOG_FILE_PREFIX: &str = "backend.log";

/// Ustawienia zapisu logów do dziennych plików (`LOG_TO_FILE`, `LOG_DIR`, `LOG_MAX_FILES`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogConfig {
    /// Czy zapisywać logi również do plików (`LOG_TO_FILE`, domyślnie `false`).
    pub to_file: bool,
    /// Katalog docelowy na pliki logów (`LOG_DIR`, domyślnie `logs`).
    pub dir: String,
    /// Maksymalna liczba przechowywanych plików dziennych (`None` = bez limitu przy `0`).
    pub max_files: Option<usize>,
}

impl LogConfig {
    /// Parsuje opcjonalne wartości tekstowe ze środowiska do [`LogConfig`].
    pub fn parse(to_file: Option<&str>, dir: Option<&str>, max_files: Option<&str>) -> Self {
        let to_file = to_file
            .map(str::trim)
            .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes"));

        let dir = dir
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(DEFAULT_LOG_DIR)
            .to_string();

        let max_files = match max_files.map(str::trim).filter(|v| !v.is_empty()) {
            Some(raw) => match raw.parse::<usize>() {
                Ok(0) => None,
                Ok(n) => Some(n),
                Err(_) => Some(DEFAULT_LOG_MAX_FILES),
            },
            None => Some(DEFAULT_LOG_MAX_FILES),
        };

        Self {
            to_file,
            dir,
            max_files,
        }
    }

    /// Wczytuje konfigurację logowania do pliku ze zmiennych środowiskowych.
    pub fn from_env() -> Self {
        Self::parse(
            env::var("LOG_TO_FILE").ok().as_deref(),
            env::var("LOG_DIR").ok().as_deref(),
            env::var("LOG_MAX_FILES").ok().as_deref(),
        )
    }

    /// Tworzy katalog `dir` i buduje dzienny appender `backend.log.YYYY-MM-DD`.
    pub fn build_file_appender(
        &self,
    ) -> Result<RollingFileAppender, Box<dyn std::error::Error + Send + Sync>> {
        std::fs::create_dir_all(&self.dir)?;
        let mut builder = RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix(LOG_FILE_PREFIX);
        if let Some(limit) = self.max_files {
            builder = builder.max_log_files(limit);
        }
        Ok(builder.build(&self.dir)?)
    }
}

/// Źródło danych piłki nożnej wybierane zmienną `INGEST_FOOTBALL_SOURCE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FootballIngestSource {
    /// Historia i statystyki z `football-data.co.uk`, bieżący sezon z Football-Data.org (gdy jest klucz) lub CSV.
    #[default]
    Auto,
    /// Wyłącznie `football-data.co.uk` (plus ewentualne wzbogacenie ClubElo).
    FootballDataUk,
    /// Wyłącznie `api.football-data.org/v4` (tryb awaryjny, gdy `football-data.co.uk` jest niedostępne).
    FootballData,
}

impl FootballIngestSource {
    /// Rozpoznaje tryb pracy importu piłki nożnej; nieznana wartość zwraca domyślny tryb `Auto`.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "football_data_uk" | "football-data-uk" | "footballdatauk" | "csv" => {
                Self::FootballDataUk
            }
            "football_data" | "football-data" | "footballdata" => Self::FootballData,
            _ => Self::Auto,
        }
    }
}

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
    /// Klucz do Football-Data.org (opcjonalny – bez niego bieżący sezon piłki obsługuje `football-data.co.uk`).
    pub football_data_api_key: Option<String>,
    /// Klucz do BALLDONTLIE (opcjonalny – służy do terminarza, kwart, dokładnego `datetime` UTC i fallbacku historii).
    pub balldontlie_api_key: Option<String>,
    /// Klucz Bearer do PandaScore Free (`/csgo/*` dla CS2; opcjonalny).
    pub pandascore_api_key: Option<String>,
    /// Token dostępu Kaggle (`KAGGLE_API_TOKEN`, np. `KGAT_...`; opcjonalny).
    pub kaggle_api_token: Option<String>,
    /// Czy uruchamiać cykliczny import danych sportowych (`INGEST_ENABLED`, domyślnie `true`).
    pub ingest_enabled: bool,
    /// Co ile minut powtarzać import bieżących danych (`INGEST_INTERVAL_MINUTES`, domyślnie 60).
    pub ingest_interval_minutes: u64,
    /// Sezony historyczne piłki nożnej, np. `2023-24,2024-25`, `2023-2024` lub `2023`
    /// (`INGEST_FOOTBALL_SEASONS`; puste = dwa ostatnie zakończone sezony).
    pub ingest_football_seasons: Vec<String>,
    /// Strategia wyboru źródła meczów piłki nożnej (`INGEST_FOOTBALL_SOURCE`, domyślnie `auto`).
    pub ingest_football_source: FootballIngestSource,
    /// Czy wzbogacać mecze piłkarskie o przedmeczowe ratingi ClubElo (`INGEST_FOOTBALL_CLUBELO_ENABLED`, domyślnie `true`).
    pub ingest_football_clubelo_enabled: bool,
    /// Kody lig piłkarskich do importu, np. `PL,PD,BL1,SA,FL1` (`INGEST_FOOTBALL_LEAGUES`, domyślnie `PL`).
    pub ingest_football_leagues: Vec<String>,
    /// Sezony historyczne NBA podane rokiem startu, np. `2023,2024` (`INGEST_NBA_SEASONS`).
    pub ingest_nba_seasons: Vec<i32>,
    /// Strategia wyboru źródła danych NBA (`INGEST_NBA_SOURCE`, domyślnie `auto`).
    pub ingest_nba_source: NbaIngestSource,
    /// Opcjonalne proxy HTTP/HTTPS wyłącznie dla `stats.nba.com` (`NBA_API_PROXY`, np. dla serwerów chmurowych).
    pub nba_api_proxy: Option<String>,
    /// Limit czasu pojedynczego zapytania do `stats.nba.com` w sekundach (`NBA_STATS_TIMEOUT_SECS`, domyślnie `90`).
    pub nba_stats_timeout_secs: u64,
    /// Pauza między kolejnymi zapytaniami do `stats.nba.com` w ms (`NBA_STATS_DELAY_MS`, domyślnie `1500`).
    pub nba_stats_delay_ms: u64,
    /// Lata historyczne tenisa z archiwum Sackmanna, np. `2023,2024` (`INGEST_TENNIS_YEARS`).
    pub ingest_tennis_years: Vec<i32>,
    /// Toury tenisowe do importu: `ATP`, `WTA` (`INGEST_TENNIS_TOURS`, domyślnie `ATP`).
    pub ingest_tennis_tours: Vec<String>,
    /// Identyfikator ligi Euroligi w TheSportsDB (`THESPORTSDB_EUROLEAGUE_ID`);
    /// bez niego Euroliga jest pomijana.
    pub thesportsdb_euroleague_id: Option<String>,
    /// Marża bukmacherska stosowana przy wyliczaniu kursów `MARKET_AVG` (`BOOKMAKER_MARGIN`, domyślnie `0.05`).
    pub bookmaker_margin: Decimal,
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
        let pandascore_api_key = env_non_empty("PANDASCORE_API_KEY");
        let kaggle_api_token = env_non_empty("KAGGLE_API_TOKEN");

        let ingest_enabled = env::var("INGEST_ENABLED")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0" | "no"))
            .unwrap_or(true);

        let ingest_interval_minutes = env::var("INGEST_INTERVAL_MINUTES")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(60);

        let ingest_football_source = match env_non_empty("INGEST_FOOTBALL_SOURCE") {
            Some(raw) => {
                let parsed = FootballIngestSource::parse(&raw);
                if parsed == FootballIngestSource::Auto && !raw.trim().eq_ignore_ascii_case("auto")
                {
                    tracing::warn!(
                        value = %raw,
                        "Nieznana wartość INGEST_FOOTBALL_SOURCE – używam trybu auto"
                    );
                }
                parsed
            }
            None => FootballIngestSource::Auto,
        };

        let ingest_football_clubelo_enabled = env::var("INGEST_FOOTBALL_CLUBELO_ENABLED")
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0" | "no"))
            .unwrap_or(true);

        let mut ingest_football_leagues: Vec<String> =
            env_list::<String>("INGEST_FOOTBALL_LEAGUES")
                .into_iter()
                .map(|l| l.to_ascii_uppercase())
                .collect();
        if ingest_football_leagues.is_empty() {
            ingest_football_leagues.push("PL".to_string());
        }

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

        let nba_api_proxy = env_non_empty("NBA_API_PROXY");
        let nba_stats_timeout_secs = env::var("NBA_STATS_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(90);
        let nba_stats_delay_ms = env::var("NBA_STATS_DELAY_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(1500);

        let mut ingest_tennis_tours: Vec<String> = env_list::<String>("INGEST_TENNIS_TOURS")
            .into_iter()
            .map(|t| t.to_ascii_uppercase())
            .collect();
        if ingest_tennis_tours.is_empty() {
            ingest_tennis_tours.push("ATP".to_string());
        }

        let bookmaker_margin = match env_non_empty("BOOKMAKER_MARGIN") {
            Some(raw) => match raw.parse::<Decimal>() {
                Ok(m) if m >= Decimal::ZERO && m < Decimal::ONE => m,
                _ => {
                    tracing::warn!(
                        value = %raw,
                        "Niepoprawna wartość BOOKMAKER_MARGIN (oczekiwano [0.00, 1.00)) – używam 0.05"
                    );
                    default_bookmaker_margin()
                }
            },
            None => default_bookmaker_margin(),
        };

        Self {
            ingest_enabled,
            ingest_interval_minutes,
            ingest_football_seasons: env_list("INGEST_FOOTBALL_SEASONS"),
            ingest_football_source,
            ingest_football_clubelo_enabled,
            ingest_football_leagues,
            ingest_nba_seasons: env_list("INGEST_NBA_SEASONS"),
            ingest_nba_source,
            nba_api_proxy,
            nba_stats_timeout_secs,
            nba_stats_delay_ms,
            ingest_tennis_years: env_list("INGEST_TENNIS_YEARS"),
            ingest_tennis_tours,
            thesportsdb_euroleague_id: env_non_empty("THESPORTSDB_EUROLEAGUE_ID"),
            bookmaker_margin,
            database_url,
            host,
            port,
            football_data_api_key,
            balldontlie_api_key,
            pandascore_api_key,
            kaggle_api_token,
        }
    }

    /// Zwraca adres w formacie `host:port`, np. `0.0.0.0:3000`.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
