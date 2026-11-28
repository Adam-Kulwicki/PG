// src/ingest/football/clubelo_archive.rs
//! Archiwalne ratingi ClubElo (`EloRatings.csv`) do backfillu historycznego Top 5 lig.
//!
//! Wykorzystuje snapshoty ze zbioru `xgabora/Club-Football-Match-Data` (z twardym odcięciem
//! `<= 2025-06-01`, aby wykluczyć estymacje *provisional*) oraz automatyczną kalibrację offsetu
//! na bazie mediany różnic w oknie zakładkowym `[2023-01-01, 2025-06-01]`.

use std::{collections::HashMap, path::Path, time::Duration};

use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, NaiveTime, Utc};
use sqlx::PgPool;

use super::{
    clubelo::{lookup_pre_match_elo_point, ClubEloPoint},
    teams::{normalize_football_team_name, LeagueSpec, TOP5_LEAGUES},
};
use crate::{
    error::{AppError, AppResult},
    ingest::{fetch_text_with_timeout, tennis::split_csv_line},
};

/// Podstawowy adres pliku `EloRatings.csv` (GitHub raw).
pub const CLUBELO_ARCHIVE_PRIMARY_URL: &str =
    "https://raw.githubusercontent.com/xgabora/Club-Football-Match-Data/main/data/EloRatings.csv";

/// Zapasowy adres pliku `EloRatings.csv` (HuggingFace).
pub const CLUBELO_ARCHIVE_MIRROR_URL: &str =
    "https://huggingface.co/datasets/xgabora/club-football-match-data/resolve/main/EloRatings.csv";

/// Ścieżka lokalnego cache archiwum w wolumenie `data/`.
pub const CLUBELO_ARCHIVE_CACHE_PATH: &str = "data/clubelo_elo_ratings.csv";

/// Dolna granica dat wczytywanych z archiwum (wyklucza starsze sezony spoza zakresu ETL).
pub const CLUBELO_ARCHIVE_MIN_DATE_YMD: (i32, u32, u32) = (2015, 1, 1);

/// Twarde odcięcie archiwum — od `2025-06-15` plik zawiera tymczasowe estymacje *provisional*.
pub const CLUBELO_ARCHIVE_MAX_DATE_YMD: (i32, u32, u32) = (2025, 6, 1);

/// Początek okna zakładkowego używanego do kalibracji offsetu ligi między live a archiwum.
pub const CALIBRATION_WINDOW_START_YMD: (i32, u32, u32) = (2023, 1, 1);

/// Koniec okna zakładkowego używanego do kalibracji offsetu ligi.
pub const CALIBRATION_WINDOW_END_YMD: (i32, u32, u32) = (2025, 6, 1);

/// Maksymalna odległość w dniach przy parowaniu snapshotu archiwalnego z punktem live Vega.
pub const CALIBRATION_MAX_DATE_DIFF_DAYS: i64 = 3;

/// Minimalna liczba próbek wymagana do przyjęcia wyliczonej mediany zamiast bezpiecznego `0.0`.
pub const CALIBRATION_MIN_SAMPLES: usize = 20;

const CLUBELO_ARCHIVE_TIMEOUT: Duration = Duration::from_secs(60);
const CLUBELO_ARCHIVE_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_secs(2), Duration::from_secs(5)];

/// Mapa archiwalnych punktów Elo indeksowana parą `(country_code, canonical_team_name)`,
/// z punktami posortowanymi rosnąco po dacie.
pub type ClubEloArchiveMap = HashMap<(String, String), Vec<ClubEloPoint>>;

#[derive(Debug, sqlx::FromRow)]
struct SqlCalibrationMatchRow {
    match_date: DateTime<Utc>,
    home_name: String,
    away_name: String,
    home_elo: Option<f64>,
    away_elo: Option<f64>,
}

fn ymd_to_naive_date((year, month, day): (i32, u32, u32)) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day)
        .expect("Stała konfiguracji daty ClubElo musi reprezentować poprawny dzień")
}

fn resolve_top5_clubelo_country(raw_country: &str) -> Option<&'static str> {
    TOP5_LEAGUES
        .iter()
        .find(|spec| spec.clubelo_country.eq_ignore_ascii_case(raw_country))
        .map(|spec| spec.clubelo_country)
}

fn get_trimmed_field<'a>(
    fields: &'a [String],
    header_index: &HashMap<String, usize>,
    column: &str,
) -> Option<&'a str> {
    header_index
        .get(column)
        .and_then(|&index| fields.get(index))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn find_nearest_point_within_days(
    points: &[ClubEloPoint],
    target_date: NaiveDate,
    max_days: i64,
) -> Option<f64> {
    points
        .iter()
        .filter_map(|point| {
            let day_diff = (point.date - target_date).num_days().abs();
            (day_diff <= max_days && point.elo.is_finite()).then_some((day_diff, point.elo))
        })
        .min_by_key(|(day_diff, _)| *day_diff)
        .map(|(_, elo)| elo)
}

/// Parsuje zawartość `EloRatings.csv` (`"date","club","country","elo"`), filtrując rekordy do lig Top 5
/// i zakresu `[2015-01-01, 2025-06-01]`, a nazwy klubów normalizuje do postaci kanonicznej.
pub fn parse_clubelo_archive_csv(csv_content: &str) -> AppResult<ClubEloArchiveMap> {
    let trimmed_content = csv_content.trim_start_matches('\u{feff}');
    let mut lines = trimmed_content
        .lines()
        .filter(|line| !line.trim().is_empty());

    let header_line = lines
        .next()
        .ok_or_else(|| AppError::ExternalData("ClubElo archiwum: pusty plik CSV".to_string()))?;
    let header_index: HashMap<String, usize> = split_csv_line(header_line)
        .into_iter()
        .enumerate()
        .map(|(index, column)| (column.trim().to_ascii_lowercase(), index))
        .collect();

    for required in ["date", "club", "country", "elo"] {
        if !header_index.contains_key(required) {
            return Err(AppError::ExternalData(format!(
                "ClubElo archiwum: brak wymaganej kolumny '{required}'"
            )));
        }
    }

    let min_date = ymd_to_naive_date(CLUBELO_ARCHIVE_MIN_DATE_YMD);
    let max_date = ymd_to_naive_date(CLUBELO_ARCHIVE_MAX_DATE_YMD);
    let mut archive: ClubEloArchiveMap = HashMap::new();

    for line in lines {
        let fields = split_csv_line(line);
        let (Some(raw_date), Some(raw_club), Some(raw_country), Some(raw_elo)) = (
            get_trimmed_field(&fields, &header_index, "date"),
            get_trimmed_field(&fields, &header_index, "club"),
            get_trimmed_field(&fields, &header_index, "country"),
            get_trimmed_field(&fields, &header_index, "elo"),
        ) else {
            continue;
        };

        let Some(country_code) = resolve_top5_clubelo_country(raw_country) else {
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(raw_date, "%Y-%m-%d") else {
            continue;
        };
        if date < min_date || date > max_date {
            continue;
        }
        let Ok(elo) = raw_elo.parse::<f64>() else {
            continue;
        };
        if !elo.is_finite() {
            continue;
        }

        let canonical_club = normalize_football_team_name(raw_club).trim();
        if canonical_club.is_empty() {
            continue;
        }

        archive
            .entry((country_code.to_string(), canonical_club.to_string()))
            .or_default()
            .push(ClubEloPoint { date, elo });
    }

    for points in archive.values_mut() {
        points.sort_by_key(|point| point.date);
    }

    Ok(archive)
}

async fn write_cache_atomically(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        if let Err(err) = tokio::fs::create_dir_all(parent).await {
            tracing::warn!(
                path = %parent.display(),
                "ClubElo archiwum: nie udało się utworzyć katalogu cache: {err}"
            );
            return;
        }
    }

    let temp_path = path.with_extension(format!("csv.tmp.{}", std::process::id()));
    if let Err(err) = tokio::fs::write(&temp_path, content).await {
        tracing::warn!(
            path = %temp_path.display(),
            "ClubElo archiwum: nie udało się zapisać pliku tymczasowego cache: {err}"
        );
        return;
    }

    if let Err(err) = tokio::fs::rename(&temp_path, path).await {
        tracing::warn!(
            path = %path.display(),
            "ClubElo archiwum: nie udało się atomowo podmienić pliku cache: {err}"
        );
        let _ = tokio::fs::remove_file(&temp_path).await;
    }
}

/// Wczytuje archiwum `EloRatings.csv` z lokalnego cache albo pobiera je z sieci (primary + mirror),
/// walidując format CSV przed atomowym zapisem na dysk.
pub async fn load_or_fetch_clubelo_archive(
    client: &reqwest::Client,
) -> AppResult<ClubEloArchiveMap> {
    let cache_path = Path::new(CLUBELO_ARCHIVE_CACHE_PATH);
    if cache_path.exists() {
        match tokio::fs::read_to_string(cache_path).await {
            Ok(cached_csv) => match parse_clubelo_archive_csv(&cached_csv) {
                Ok(archive) => {
                    tracing::debug!(
                        path = %cache_path.display(),
                        clubs = archive.len(),
                        "ClubElo archiwum: wczytano z lokalnego cache"
                    );
                    return Ok(archive);
                }
                Err(err) => {
                    tracing::warn!(
                        path = %cache_path.display(),
                        "ClubElo archiwum: uszkodzony lokalny cache, pobieram ponownie: {err}"
                    );
                }
            },
            Err(err) => {
                tracing::warn!(
                    path = %cache_path.display(),
                    "ClubElo archiwum: nie udało się odczytać cache, pobieram z sieci: {err}"
                );
            }
        }
    }

    let csv_content = match fetch_text_with_timeout(
        client,
        CLUBELO_ARCHIVE_PRIMARY_URL,
        &[],
        CLUBELO_ARCHIVE_TIMEOUT,
        &CLUBELO_ARCHIVE_RETRY_DELAYS,
    )
    .await
    {
        Ok(body) => body,
        Err(primary_err) => {
            tracing::warn!(
                url = CLUBELO_ARCHIVE_PRIMARY_URL,
                "ClubElo archiwum: pobieranie z primary nieudane ({primary_err}), próbuję mirror"
            );
            fetch_text_with_timeout(
                client,
                CLUBELO_ARCHIVE_MIRROR_URL,
                &[],
                CLUBELO_ARCHIVE_TIMEOUT,
                &CLUBELO_ARCHIVE_RETRY_DELAYS,
            )
            .await?
        }
    };

    // Walidujemy CSV przed zapisem na dysk, żeby nie utrwalić odpowiedzi błędu lub strony HTML.
    let archive = parse_clubelo_archive_csv(&csv_content)?;
    write_cache_atomically(cache_path, &csv_content).await;
    tracing::info!(
        clubs = archive.len(),
        "ClubElo archiwum: pobrano i sparsowano historyczne ratingi"
    );
    Ok(archive)
}

/// Zwraca medianę skończonych wartości w `values` lub `None`, gdy brak poprawnych elementów.
pub fn median(values: &[f64]) -> Option<f64> {
    let mut finite_values: Vec<f64> = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    if finite_values.is_empty() {
        return None;
    }

    finite_values.sort_by(f64::total_cmp);
    let mid = finite_values.len() / 2;
    if finite_values.len() % 2 == 1 {
        Some(finite_values[mid])
    } else {
        Some((finite_values[mid - 1] + finite_values[mid]) / 2.0)
    }
}

/// Zbiera różnice `elo_live - elo_arch` per `country_code` w oknie `[2023-01-01, 2025-06-01]`
/// dla par punktów oddalonych o co najwyżej `CALIBRATION_MAX_DATE_DIFF_DAYS` dni.
pub fn collect_live_calibration_samples(
    club_histories: &HashMap<String, Vec<ClubEloPoint>>,
    archive: &ClubEloArchiveMap,
) -> HashMap<String, Vec<f64>> {
    let window_start = ymd_to_naive_date(CALIBRATION_WINDOW_START_YMD);
    let window_end = ymd_to_naive_date(CALIBRATION_WINDOW_END_YMD);
    let mut samples_by_country: HashMap<String, Vec<f64>> = HashMap::new();

    for ((country_code, canonical_club), archive_points) in archive {
        let Some(live_points) = club_histories
            .get(canonical_club)
            .filter(|points| !points.is_empty())
        else {
            continue;
        };

        for archive_point in archive_points {
            if archive_point.date < window_start || archive_point.date > window_end {
                continue;
            }
            let Some(live_elo) = find_nearest_point_within_days(
                live_points,
                archive_point.date,
                CALIBRATION_MAX_DATE_DIFF_DAYS,
            ) else {
                continue;
            };
            let diff = live_elo - archive_point.elo;
            if diff.is_finite() {
                samples_by_country
                    .entry(country_code.clone())
                    .or_default()
                    .push(diff);
            }
        }
    }

    samples_by_country
}

/// Pobiera z PostgreSQL istniejące historyczne ratingi danej ligi z okna `[2023-01-01, 2025-06-01]`
/// jako zapasowe próbki kalibracyjne `stored_elo - archive_elo`, gdy w pamięci zebrano `< 20` próbek.
pub async fn fetch_sql_calibration_samples(
    db: &PgPool,
    spec: &LeagueSpec,
    archive: &ClubEloArchiveMap,
) -> AppResult<Vec<f64>> {
    let window_start_dt = ymd_to_naive_date(CALIBRATION_WINDOW_START_YMD)
        .and_time(NaiveTime::MIN)
        .and_utc();
    let window_end_exclusive_dt = (ymd_to_naive_date(CALIBRATION_WINDOW_END_YMD)
        + ChronoDuration::days(1))
    .and_time(NaiveTime::MIN)
    .and_utc();

    let rows: Vec<SqlCalibrationMatchRow> = sqlx::query_as(
        r#"
        SELECT
            m.match_date,
            home.name AS home_name,
            away.name AS away_name,
            (m.match_details->'pre_match_elo'->>'home_elo')::DOUBLE PRECISION AS home_elo,
            (m.match_details->'pre_match_elo'->>'away_elo')::DOUBLE PRECISION AS away_elo
        FROM matches m
        JOIN competitions c ON c.id = m.competition_id
        JOIN participants home ON home.id = m.home_participant_id
        JOIN participants away ON away.id = m.away_participant_id
        WHERE c.sport = 'FOOTBALL'
          AND c.external_code = $1
          AND m.status != 'SCHEDULED'
          AND m.match_date >= $2
          AND m.match_date < $3
          AND (
              m.match_details->'pre_match_elo'->>'home_elo' IS NOT NULL
              OR m.match_details->'pre_match_elo'->>'away_elo' IS NOT NULL
          )
        "#,
    )
    .bind(spec.code)
    .bind(window_start_dt)
    .bind(window_end_exclusive_dt)
    .fetch_all(db)
    .await?;

    let mut samples = Vec::new();
    for row in rows {
        for (team_name, stored_elo) in [
            (&row.home_name, row.home_elo),
            (&row.away_name, row.away_elo),
        ] {
            let Some(stored_elo) = stored_elo.filter(|value| value.is_finite()) else {
                continue;
            };
            let canonical = normalize_football_team_name(team_name).to_string();
            let Some(archive_points) = archive.get(&(spec.clubelo_country.to_string(), canonical))
            else {
                continue;
            };
            let Some(archive_elo) = lookup_pre_match_elo_point(archive_points, row.match_date)
            else {
                continue;
            };
            let diff = stored_elo - archive_elo;
            if diff.is_finite() {
                samples.push(diff);
            }
        }
    }

    Ok(samples)
}

/// Wyznacza per kraj (`ENG`, `ESP`, `GER`, `ITA`, `FRA`) offset kalibracyjny między aktualnym
/// modelem `clubelo.com` a archiwum `EloRatings.csv`, opadając do `0.0` przy `< 20` próbkach.
pub fn compute_league_calibration_offsets(
    club_histories: &HashMap<String, Vec<ClubEloPoint>>,
    archive: &ClubEloArchiveMap,
    fallback_samples: &HashMap<String, Vec<f64>>,
) -> HashMap<String, f64> {
    let live_samples_by_country = collect_live_calibration_samples(club_histories, archive);
    let mut offsets = HashMap::with_capacity(TOP5_LEAGUES.len());

    for spec in TOP5_LEAGUES {
        let country_code = spec.clubelo_country;
        let mut samples = live_samples_by_country
            .get(country_code)
            .cloned()
            .unwrap_or_default();

        if samples.len() < CALIBRATION_MIN_SAMPLES {
            if let Some(extra) = fallback_samples.get(country_code) {
                samples.extend(extra.iter().copied().filter(|value| value.is_finite()));
            }
        }

        let offset = if samples.len() >= CALIBRATION_MIN_SAMPLES {
            median(&samples).unwrap_or(0.0)
        } else {
            0.0
        };

        tracing::info!(
            country = country_code,
            sample_count = samples.len(),
            offset,
            "ClubElo archiwum: wyznaczono offset kalibracyjny ligi"
        );
        offsets.insert(country_code.to_string(), offset);
    }

    offsets
}
