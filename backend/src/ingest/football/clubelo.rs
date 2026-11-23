// src/ingest/football/clubelo.rs
//! Klient i parser rankingów siły drużyn z `https://clubelo.com`.
//!
//! Dla ochrony przed *data leakage* historyczny rating każdego meczu pochodzi zawsze z dnia
//! poprzedzającego spotkanie (`Date <= match_date.date_naive() - 1 dzień`), a ze strony
//! `/Fixtures` pobierane są wyłącznie przedmeczowe prognozy dla spotkań `SCHEDULED`.

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use serde::Deserialize;
use sqlx::types::Json;

use super::teams::{clubelo_slug, normalize_football_team_name};
use crate::{
    error::{AppError, AppResult},
    ingest::{fetch_text_with_timeout, merge_match_details},
    models::match_model::{FootballPreMatchElo, MatchDetails},
    state::AppState,
};

const CLUBELO_TIMEOUT: Duration = Duration::from_secs(20);
const CLUBELO_REQUEST_DELAY: Duration = Duration::from_millis(300);
const CLUBELO_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];

const CLUBELO_HEADERS: [(&str, &str); 2] = [
    (
        "User-Agent",
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36",
    ),
    ("Accept", "text/html,application/xhtml+xml"),
];

/// Punkt na osi czasu Elo z wykresu na stronie `https://clubelo.com/{slug}`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClubEloPoint {
    pub date: NaiveDate,
    pub elo: f64,
}

/// Nadchodzący mecz z prawdopodobieństwami wyliczonymi na `https://clubelo.com/Fixtures`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClubEloFixture {
    pub date: NaiveDate,
    pub home: String,
    pub away: String,
    pub home_rank: Option<i32>,
    pub away_rank: Option<i32>,
    pub prob_home: f64,
    pub prob_draw: f64,
    pub prob_away: f64,
}

#[derive(Debug, Deserialize)]
struct VegaEloRow {
    #[serde(rename = "Date")]
    date: String,
    #[serde(rename = "Elo")]
    elo: f64,
}

#[derive(Debug, sqlx::FromRow)]
struct FootballMatchEloRow {
    id: i32,
    match_date: DateTime<Utc>,
    status: String,
    match_details: Json<MatchDetails>,
    home_name: String,
    away_name: String,
}

/// Wyciąga historię ratingów Elo z osadzonego JSON-a Vega-Lite (`"datasets": { ...: [...] }`)
/// na stronie `https://clubelo.com/{ClubSlug}`.
pub fn parse_clubelo_team_page_history(html: &str) -> AppResult<Vec<ClubEloPoint>> {
    let datasets_pos = html.find("\"datasets\"").ok_or_else(|| {
        AppError::ExternalData("ClubElo: brak sekcji datasets na stronie klubu".to_string())
    })?;
    let after_datasets = &html[datasets_pos..];

    // Pierwsza tablica `[...]` w `datasets` zawiera punkty `{Date, Elo, Golo, segment_id}`.
    let array_start = after_datasets
        .find('[')
        .ok_or_else(|| AppError::ExternalData("ClubElo: brak tablicy punktów Elo".to_string()))?;
    let slice = &after_datasets[array_start..];
    let array_end = slice
        .find(']')
        .ok_or_else(|| AppError::ExternalData("ClubElo: niedomknięta tablica Elo".to_string()))?;

    let rows: Vec<VegaEloRow> = serde_json::from_str(&slice[..=array_end])
        .map_err(|e| AppError::ExternalData(format!("ClubElo JSON: {e}")))?;

    let mut points: Vec<ClubEloPoint> = rows
        .into_iter()
        .filter(|r| r.elo.is_finite())
        .filter_map(|r| {
            let date = r
                .date
                .get(..10)
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())?;
            Some(ClubEloPoint { date, elo: r.elo })
        })
        .collect();

    points.sort_by_key(|p| p.date);
    Ok(points)
}

/// Wyszukuje przedmeczowy rating Elo z dnia `<= match_date.date_naive() - 1 dzień` (`D - 1`).
pub fn lookup_pre_match_elo_point(
    points: &[ClubEloPoint],
    match_date: DateTime<Utc>,
) -> Option<f64> {
    let target_day = match_date.date_naive() - ChronoDuration::days(1);
    points
        .iter()
        .filter(|p| p.date <= target_day)
        .max_by_key(|p| p.date)
        .map(|p| p.elo)
}

/// Usuwa znaczniki HTML z krótkiego fragmentu i zwraca czysty tekst.
fn strip_html_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Wyciąga pierwszy napis pomiędzy `prefix` i `suffix`.
fn extract_between<'a>(haystack: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let start = haystack.find(prefix)? + prefix.len();
    let rest = &haystack[start..];
    let end = rest.find(suffix)?;
    Some(&rest[..end])
}

/// Normalizuje nazwę klubu na podstawie sluga URL (`href="/Slug"`) lub wyświetlanej nazwy.
fn resolve_club_name(slug: Option<&str>, display_name: &str) -> String {
    let cleaned_name = strip_html_tags(display_name);
    if !cleaned_name.is_empty() {
        let by_name = normalize_football_team_name(&cleaned_name);
        if by_name != cleaned_name.trim() {
            return by_name.to_string();
        }
    }
    if let Some(s) = slug.map(str::trim).filter(|s| !s.is_empty()) {
        let by_slug = normalize_football_team_name(s);
        if by_slug != s {
            return by_slug.to_string();
        }
    }
    if !cleaned_name.is_empty() {
        normalize_football_team_name(&cleaned_name).to_string()
    } else {
        slug.unwrap_or_default().trim().to_string()
    }
}

/// Parsuje bieżący ranking wszystkich klubów (`const eloData = [...]`) ze strony głównej `https://clubelo.com/`.
pub fn parse_clubelo_home_rankings(html: &str) -> AppResult<HashMap<String, (f64, Option<i32>)>> {
    let marker = "const eloData = [";
    let start = html
        .find(marker)
        .map(|i| i + marker.len())
        .ok_or_else(|| AppError::ExternalData("ClubElo: brak tablicy eloData".to_string()))?;
    let rest = &html[start..];
    let end = rest.find("];").ok_or_else(|| {
        AppError::ExternalData("ClubElo: niedomknięta tablica eloData".to_string())
    })?;

    let mut rankings = HashMap::new();
    let mut cursor = &rest[..end];

    // Każdy wpis ma format: `['<td ...><small> 1 </small><a href="/Arsenal">Arsenal...</a></td>', '2045', ...]`
    while let Some(item_start) = cursor.find("['") {
        let after_start = &cursor[item_start + 2..];
        let Some(item_end) = after_start.find("']") else {
            break;
        };
        let row_slice = &after_start[..item_end];
        cursor = &after_start[item_end + 2..];

        let cols: Vec<&str> = row_slice.split("', '").collect();
        if cols.len() < 2 {
            continue;
        }
        let cell = cols[0];
        let Ok(elo) = cols[1].trim().parse::<f64>() else {
            continue;
        };
        if !elo.is_finite() {
            continue;
        }

        let rank =
            extract_between(cell, "<small>", "</small>").and_then(|r| r.trim().parse::<i32>().ok());
        let club_cell = cell
            .rfind("href=\"/")
            .map(|pos| &cell[pos..])
            .unwrap_or(cell);
        let slug = extract_between(club_cell, "href=\"/", "\"");
        let anchor_body = club_cell
            .find('>')
            .map(|gt| &club_cell[gt + 1..])
            .and_then(|s| s.split("</a>").next())
            .unwrap_or(club_cell);
        let canonical = resolve_club_name(slug, anchor_body);
        if !canonical.is_empty() {
            rankings.insert(canonical, (elo, rank));
        }
    }

    Ok(rankings)
}

/// Wyciąga nazwę klubu, ranking oraz pierwszą niepustą wartość procentową z wiersza tabeli `/Fixtures`.
fn parse_fixture_side(row_html: &str) -> Option<(String, Option<i32>, f64)> {
    let rank =
        extract_between(row_html, "<small>", "</small>").and_then(|r| r.trim().parse::<i32>().ok());

    let anchor_pos = row_html.find("href=\"/")?;
    let after_href = &row_html[anchor_pos + 7..];
    // Pomijamy pierwszy link flagi kraju (`/ENG`, `/FRA`), szukając linku samego klubu po nim.
    let club_href_start = after_href
        .find("href=\"/")
        .map(|offset| anchor_pos + 7 + offset)
        .unwrap_or(anchor_pos);
    let club_slice = &row_html[club_href_start..];
    let slug = extract_between(club_slice, "href=\"/", "\"");
    let display_name = extract_between(club_slice, "<span class=\"min641\">", "</span>")
        .or_else(|| extract_between(club_slice, ">", "</a>"))
        .unwrap_or_default();
    let club = resolve_club_name(slug, display_name);
    if club.is_empty() {
        return None;
    }

    let after_club_link = club_slice
        .find("</a>")
        .map(|i| &club_slice[i + 4..])
        .unwrap_or(club_slice);
    let prob = first_non_empty_percent(after_club_link)?;
    Some((club, rank, prob))
}

fn first_non_empty_percent(mut slice: &str) -> Option<f64> {
    let marker = "<span class=\"min1081\">";
    while let Some(pos) = slice.find(marker) {
        let rest = &slice[pos + marker.len()..];
        let Some(end) = rest.find("</span>") else {
            break;
        };
        let raw = rest[..end].trim().trim_end_matches('%');
        if let Ok(pct) = raw.parse::<f64>() {
            if pct.is_finite() {
                return Some(pct / 100.0);
            }
        }
        slice = &rest[end + 7..];
    }
    None
}

/// Parsuje prawdopodobieństwa nadchodzących meczów ze strony `https://clubelo.com/Fixtures`.
pub fn parse_clubelo_fixtures_html(html: &str) -> AppResult<Vec<ClubEloFixture>> {
    let mut fixtures = Vec::new();
    let marker = "data-utc-time='";
    let mut rest = html;

    while let Some(pos) = rest.find(marker) {
        let after_marker = &rest[pos + marker.len()..];
        let Some(date_raw) = after_marker.get(..10) else {
            break;
        };
        let Ok(date) = NaiveDate::parse_from_str(date_raw, "%Y-%m-%d") else {
            rest = after_marker;
            continue;
        };

        let Some(home_end) = after_marker.find("</tr>") else {
            break;
        };
        let home_row = &after_marker[..home_end];
        let after_home = &after_marker[home_end + 5..];
        let Some(away_end) = after_home.find("</tr>") else {
            break;
        };
        let away_row = &after_home[..away_end];
        rest = &after_home[away_end + 5..];

        let Some((home, home_rank, prob_home)) = parse_fixture_side(home_row) else {
            continue;
        };
        let Some((away, away_rank, prob_away)) = parse_fixture_side(away_row) else {
            continue;
        };
        let Some(draw_cell_pos) = home_row.find("rowspan=\"2\"") else {
            continue;
        };
        let Some(prob_draw) = first_non_empty_percent(&home_row[draw_cell_pos..]) else {
            continue;
        };

        fixtures.push(ClubEloFixture {
            date,
            home,
            away,
            home_rank,
            away_rank,
            prob_home,
            prob_draw,
            prob_away,
        });
    }

    Ok(fixtures)
}

/// Składa strukturę [`FootballPreMatchElo`] z opcjonalnych ratingów drużyn oraz prognoz `/Fixtures`.
pub fn build_pre_match_elo(
    home: Option<(f64, Option<i32>)>,
    away: Option<(f64, Option<i32>)>,
    fixture: Option<&ClubEloFixture>,
) -> Option<FootballPreMatchElo> {
    if home.is_none() && away.is_none() && fixture.is_none() {
        return None;
    }

    let home_elo = home.map(|(e, _)| e);
    let away_elo = away.map(|(e, _)| e);
    let elo_diff = home_elo.zip(away_elo).map(|(h, a)| h - a);
    let home_rank = home
        .and_then(|(_, r)| r)
        .or_else(|| fixture.and_then(|f| f.home_rank));
    let away_rank = away
        .and_then(|(_, r)| r)
        .or_else(|| fixture.and_then(|f| f.away_rank));

    let probs = fixture.filter(|f| f.prob_home + f.prob_draw + f.prob_away > 0.5);

    Some(FootballPreMatchElo {
        home_elo,
        away_elo,
        elo_diff,
        home_rank,
        away_rank,
        clubelo_prob_home: probs.map(|f| f.prob_home),
        clubelo_prob_draw: probs.map(|f| f.prob_draw),
        clubelo_prob_away: probs.map(|f| f.prob_away),
    })
}

async fn fetch_clubelo_html(client: &reqwest::Client, path: &str) -> AppResult<String> {
    let url = format!("https://clubelo.com/{path}");
    fetch_text_with_timeout(
        client,
        &url,
        &CLUBELO_HEADERS,
        CLUBELO_TIMEOUT,
        &CLUBELO_RETRY_DELAYS,
    )
    .await
}

/// Wzbogaca istniejące mecze piłkarskie w bazie o przedmeczowe ratingi z `https://clubelo.com` (`D - 1`)
/// oraz prawdopodobieństwa `/Fixtures` dla spotkań `SCHEDULED`. Nigdy nie tworzy nowych meczów.
pub async fn enrich_matches_with_clubelo(
    state: &AppState,
    league_codes: &[&str],
) -> AppResult<usize> {
    if league_codes.is_empty() {
        return Ok(0);
    }

    let codes: Vec<String> = league_codes.iter().map(|c| (*c).to_string()).collect();
    let rows: Vec<FootballMatchEloRow> = sqlx::query_as(
        r#"
        SELECT
            m.id,
            m.match_date,
            m.status,
            m.match_details,
            home.name AS home_name,
            away.name AS away_name
        FROM matches m
        JOIN competitions c ON c.id = m.competition_id
        JOIN participants home ON home.id = m.home_participant_id
        JOIN participants away ON away.id = m.away_participant_id
        WHERE c.sport = 'FOOTBALL'
          AND c.external_code = ANY($1)
          AND (
              m.match_details->'pre_match_elo' IS NULL
              OR m.status = 'SCHEDULED'
          )
        ORDER BY m.match_date ASC
        "#,
    )
    .bind(&codes)
    .fetch_all(&state.db)
    .await?;

    if rows.is_empty() {
        return Ok(0);
    }

    // 1. Aktualny ranking klubów (używany dla `SCHEDULED` oraz jako źródło `rank` w historii).
    let current_rankings: HashMap<String, (f64, Option<i32>)> =
        match fetch_clubelo_html(&state.http_client, "").await {
            Ok(html) => parse_clubelo_home_rankings(&html).unwrap_or_else(|err| {
                tracing::warn!("ClubElo home: błąd parsowania rankingu: {err}");
                HashMap::new()
            }),
            Err(err) => {
                tracing::warn!("ClubElo home: zapytanie nieudane: {err}");
                HashMap::new()
            }
        };

    // 2. Prognozy `/Fixtures` pobieramy tylko wtedy, gdy mamy mecze `SCHEDULED`.
    let has_scheduled = rows.iter().any(|r| r.status == "SCHEDULED");
    let fixtures: Vec<ClubEloFixture> = if has_scheduled {
        match fetch_clubelo_html(&state.http_client, "Fixtures").await {
            Ok(html) => parse_clubelo_fixtures_html(&html).unwrap_or_else(|err| {
                tracing::warn!("ClubElo Fixtures: błąd parsowania: {err}");
                Vec::new()
            }),
            Err(err) => {
                tracing::warn!("ClubElo Fixtures: zapytanie nieudane: {err}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // 3. Dla meczów historycznych pobieramy raz na klub wykres historii z `clubelo.com/{slug}`.
    let mut club_histories: HashMap<String, Vec<ClubEloPoint>> = HashMap::new();
    let mut fetched_any_club = false;
    for row in &rows {
        if row.status == "SCHEDULED" {
            continue;
        }
        for team_name in [&row.home_name, &row.away_name] {
            let canonical = normalize_football_team_name(team_name).to_string();
            if club_histories.contains_key(&canonical) {
                continue;
            }
            let Some(slug) = clubelo_slug(&canonical) else {
                club_histories.insert(canonical, Vec::new());
                continue;
            };
            if fetched_any_club {
                tokio::time::sleep(CLUBELO_REQUEST_DELAY).await;
            }
            fetched_any_club = true;

            let points = match fetch_clubelo_html(&state.http_client, slug).await {
                Ok(html) => parse_clubelo_team_page_history(&html).unwrap_or_else(|err| {
                    tracing::warn!(club = slug, "ClubElo: błąd parsowania strony klubu: {err}");
                    Vec::new()
                }),
                Err(err) => {
                    tracing::warn!(club = slug, "ClubElo: błąd pobierania historii: {err}");
                    Vec::new()
                }
            };
            club_histories.insert(canonical, points);
        }
    }

    // 4. Dopinamy wyliczone Elo do `match_details` w jednej transakcji SQL.
    let mut tx = state.db.begin().await?;
    let mut updated = 0;

    for row in rows {
        let home_canonical = normalize_football_team_name(&row.home_name);
        let away_canonical = normalize_football_team_name(&row.away_name);

        let resolve_team = |canonical: &str| -> Option<(f64, Option<i32>)> {
            if row.status == "SCHEDULED" {
                current_rankings.get(canonical).copied()
            } else {
                let elo = club_histories
                    .get(canonical)
                    .and_then(|points| lookup_pre_match_elo_point(points, row.match_date))?;
                let rank = current_rankings.get(canonical).and_then(|(_, r)| *r);
                Some((elo, rank))
            }
        };

        let home_elo = resolve_team(home_canonical);
        let away_elo = resolve_team(away_canonical);

        let fixture = if row.status == "SCHEDULED" {
            let match_day = row.match_date.date_naive();
            fixtures.iter().find(|f| {
                f.home == home_canonical
                    && f.away == away_canonical
                    && (f.date - match_day).num_days().abs() <= 1
            })
        } else {
            None
        };

        let Some(new_elo) = build_pre_match_elo(home_elo, away_elo, fixture) else {
            continue;
        };

        let incoming_details = MatchDetails::Football {
            ht_home: None,
            ht_away: None,
            round: String::new(),
            home_stats: None,
            away_stats: None,
            pre_match_elo: Some(Box::new(new_elo)),
        };
        let merged_details = merge_match_details(row.match_details.0.clone(), incoming_details);
        if merged_details == row.match_details.0 {
            continue;
        }

        sqlx::query(
            r#"
            UPDATE matches
            SET match_details = $1,
                updated_at = NOW()
            WHERE id = $2
            "#,
        )
        .bind(Json(&merged_details))
        .bind(row.id)
        .execute(&mut *tx)
        .await?;

        updated += 1;
    }

    tx.commit().await?;
    Ok(updated)
}
