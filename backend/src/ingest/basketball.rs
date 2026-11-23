// src/ingest/basketball.rs
//! Koszykówka: statystyki NBA (`stats.nba.com/stats/teamgamelogs`), terminarz i kwarty
//! z BALLDONTLIE oraz opcjonalnie Euroliga z TheSportsDB.

use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    fetch_text, fetch_text_with_timeout, full_season_label, normalize_season_label,
    season_start_year, short_season_label, store_matches, winner_from_scores, IngestedMatch,
    IngestedParticipant,
};
use crate::{
    config::NbaIngestSource,
    error::{AppError, AppResult},
    models::match_model::{BasketballTeamStats, MatchDetails},
    state::AppState,
};

/// Przerwa między zapytaniami do BALLDONTLIE – darmowy klucz pozwala na 5 zapytań na minutę.
const BALLDONTLIE_REQUEST_DELAY: Duration = Duration::from_secs(12);

/// Liczba kwart w regulaminowym czasie gry; `period` powyżej tej wartości oznacza dogrywkę.
const REGULATION_PERIODS: i32 = 4;

/// Kolejne czasy oczekiwania przed ponowieniem nieudanego zapytania do `stats.nba.com`.
const NBA_STATS_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(3),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

/// Typy rozgrywek NBA pobierane z `teamgamelogs` dla każdego sezonu.
const NBA_SEASON_TYPES: [&str; 2] = ["Regular Season", "Playoffs"];

/// Czas gry drużyny (w minutach) powyżej 50 oznacza co najmniej jedną dogrywkę
/// (drobne ułamki 48.03..48.4 zdarzają się w zwykłych meczach z zegara hali).
const OVERTIME_MIN_THRESHOLD: f64 = 50.0;

/// Nagłówki HTTP wymagane przez `stats.nba.com` (zgodne z `nba_api`; bez `Host`, `Origin` i `x-nba-stats-*`).
const NBA_STATS_HEADERS: [(&str, &str); 10] = [
    (
        "User-Agent",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36",
    ),
    ("Accept", "application/json, text/plain, */*"),
    ("Accept-Encoding", "gzip, deflate, br"),
    ("Accept-Language", "en-US,en;q=0.5"),
    ("Referer", "https://www.nba.com/"),
    ("Pragma", "no-cache"),
    ("Cache-Control", "no-cache"),
    (
        "Sec-Ch-Ua",
        r#""Not:A-Brand";v="99", "Google Chrome";v="145", "Chromium";v="145""#,
    ),
    ("Sec-Ch-Ua-Mobile", "?0"),
    ("Sec-Fetch-Dest", "empty"),
];

// ---------------------------------------------------------------------------
// NBA Stats (`stats.nba.com/stats/teamgamelogs`)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct NbaStatsResponse {
    #[serde(rename = "resultSets", default)]
    result_sets: Vec<NbaResultSet>,
}

#[derive(Debug, Deserialize)]
struct NbaResultSet {
    headers: Vec<String>,
    #[serde(rename = "rowSet")]
    row_set: Vec<Vec<Value>>,
}

/// 9 zaawansowanych metryk z `MeasureType=Advanced` łączonych z wierszami `Base`.
#[derive(Debug, Clone, Copy, Default)]
struct AdvMetrics {
    off_rating: Option<f64>,
    def_rating: Option<f64>,
    net_rating: Option<f64>,
    pace: Option<f64>,
    efg_pct: Option<f64>,
    ts_pct: Option<f64>,
    tm_tov_pct: Option<f64>,
    oreb_pct: Option<f64>,
    pie: Option<f64>,
}

/// Sparsowany wiersz jednej drużyny z jednego meczu NBA przed sparowaniem gospodarz/gość.
#[derive(Debug)]
struct NbaTeamGameRow {
    season_year: String,
    team_id: i64,
    team_abbr: Option<String>,
    team_name: String,
    match_date: DateTime<Utc>,
    matchup: String,
    pts: i32,
    stats: BasketballTeamStats,
}

/// Ujednolica nazwy drużyn NBA między `stats.nba.com` a `BALLDONTLIE` (w tym historyczne).
pub fn normalize_nba_team_name(name: &str) -> &str {
    match name.trim() {
        "LA Clippers" => "Los Angeles Clippers",
        "Charlotte Bobcats" => "Charlotte Hornets",
        "New Orleans Hornets" => "New Orleans Pelicans",
        "New Jersey Nets" => "Brooklyn Nets",
        other => other,
    }
}

/// Buduje dokładny adres `stats.nba.com/stats/teamgamelogs` z zestawem 21 parametrów query.
pub fn nba_team_gamelogs_url(season_short: &str, season_type: &str, measure_type: &str) -> String {
    let encoded_type = season_type.replace(' ', "+");
    format!(
        "https://stats.nba.com/stats/teamgamelogs?DateFrom=&DateTo=&GameSegment=&LastNGames=&LeagueID=00&Location=&MeasureType={measure_type}&Month=&OpponentTeamID=&Outcome=&PORound=&PerMode=&Period=&PlayerID=&Season={season_short}&SeasonSegment=&SeasonType={encoded_type}&ShotClockRange=&TeamID=&VsConference=&VsDivision="
    )
}

/// Buduje indeks nazw kolumn w `resultSets[0].headers` na wypadek zmiany ich kolejności przez API.
fn build_header_index(headers: &[String]) -> HashMap<&str, usize> {
    headers
        .iter()
        .enumerate()
        .map(|(idx, name)| (name.as_str(), idx))
        .collect()
}

/// Sprawdza, czy tabela wynikowa zawiera minimalny zestaw wymaganych kolumn.
fn require_headers(index: &HashMap<&str, usize>, required: &[&str], label: &str) -> AppResult<()> {
    for col in required {
        if !index.contains_key(col) {
            return Err(AppError::ExternalData(format!(
                "NBA Stats ({}): brak wymaganej kolumny {}",
                label, col
            )));
        }
    }
    Ok(())
}

fn row_value<'a>(row: &'a [Value], index: &HashMap<&str, usize>, col: &str) -> Option<&'a Value> {
    index.get(col).and_then(|&i| row.get(i))
}

fn row_str<'a>(row: &'a [Value], index: &HashMap<&str, usize>, col: &str) -> Option<&'a str> {
    row_value(row, index, col)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn row_i64(row: &[Value], index: &HashMap<&str, usize>, col: &str) -> Option<i64> {
    row_value(row, index, col)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)))
}

fn row_f64(row: &[Value], index: &HashMap<&str, usize>, col: &str) -> Option<f64> {
    row_value(row, index, col).and_then(Value::as_f64)
}

/// Parsuje `GAME_DATE` z `TeamGameLogs` (`"2025-04-13T00:00:00"` bez strefy lub `"2025-04-13"`).
fn parse_nba_game_date(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc())
        .or_else(|| {
            raw.get(..10)
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .map(|d| d.and_time(NaiveTime::MIN).and_utc())
        })
}

/// Parsuje i łączy odpowiedzi `MeasureType=Base` oraz `MeasureType=Advanced` dla jednego typu sezonu.
///
/// Łączenie po `(GAME_ID, TEAM_ID)` działa jak `inner join` (zgodnie z prototypem `main.py`).
/// Mecze na terenie neutralnym (gdzie oba wiersze mają `" @ "`) porządkowane są deterministycznie
/// po `TEAM_ID`, dzięki czemu żaden mecz nie zostaje pominięty.
pub fn parse_nba_team_gamelogs(
    base_json: &str,
    adv_json: &str,
    season_type: &str,
) -> AppResult<Vec<IngestedMatch>> {
    let base_resp: NbaStatsResponse = serde_json::from_str(base_json)
        .map_err(|e| AppError::ExternalData(format!("NBA Stats Base: {}", e)))?;
    let adv_resp: NbaStatsResponse = serde_json::from_str(adv_json)
        .map_err(|e| AppError::ExternalData(format!("NBA Stats Advanced: {}", e)))?;

    let base_set = base_resp
        .result_sets
        .first()
        .ok_or_else(|| AppError::ExternalData("NBA Stats Base: brak resultSets".to_string()))?;
    let adv_set = adv_resp
        .result_sets
        .first()
        .ok_or_else(|| AppError::ExternalData("NBA Stats Advanced: brak resultSets".to_string()))?;

    // Na początku sezonu sekcja Playoffs ma puste `rowSet` – to poprawny stan, nie błąd.
    if base_set.row_set.is_empty() || adv_set.row_set.is_empty() {
        return Ok(Vec::new());
    }

    let base_idx = build_header_index(&base_set.headers);
    let adv_idx = build_header_index(&adv_set.headers);
    require_headers(
        &base_idx,
        &[
            "SEASON_YEAR",
            "TEAM_ID",
            "TEAM_NAME",
            "GAME_ID",
            "GAME_DATE",
            "MATCHUP",
            "PTS",
        ],
        "Base",
    )?;
    require_headers(&adv_idx, &["GAME_ID", "TEAM_ID"], "Advanced")?;

    let mut adv_map: HashMap<(String, i64), AdvMetrics> =
        HashMap::with_capacity(adv_set.row_set.len());
    for row in &adv_set.row_set {
        let (Some(game_id), Some(team_id)) = (
            row_str(row, &adv_idx, "GAME_ID"),
            row_i64(row, &adv_idx, "TEAM_ID"),
        ) else {
            continue;
        };
        adv_map.insert(
            (game_id.to_string(), team_id),
            AdvMetrics {
                off_rating: row_f64(row, &adv_idx, "OFF_RATING"),
                def_rating: row_f64(row, &adv_idx, "DEF_RATING"),
                net_rating: row_f64(row, &adv_idx, "NET_RATING"),
                pace: row_f64(row, &adv_idx, "PACE"),
                efg_pct: row_f64(row, &adv_idx, "EFG_PCT"),
                ts_pct: row_f64(row, &adv_idx, "TS_PCT"),
                tm_tov_pct: row_f64(row, &adv_idx, "TM_TOV_PCT"),
                oreb_pct: row_f64(row, &adv_idx, "OREB_PCT"),
                pie: row_f64(row, &adv_idx, "PIE"),
            },
        );
    }

    // `BTreeMap` porządkuje mecze stabilnie po `GAME_ID` przed końcowym sortowaniem po dacie.
    let mut games: BTreeMap<String, Vec<NbaTeamGameRow>> = BTreeMap::new();
    for row in &base_set.row_set {
        let (
            Some(season_year),
            Some(team_id),
            Some(team_name),
            Some(game_id),
            Some(match_date),
            Some(matchup),
            Some(pts),
        ) = (
            row_str(row, &base_idx, "SEASON_YEAR"),
            row_i64(row, &base_idx, "TEAM_ID"),
            row_str(row, &base_idx, "TEAM_NAME"),
            row_str(row, &base_idx, "GAME_ID"),
            row_str(row, &base_idx, "GAME_DATE").and_then(parse_nba_game_date),
            row_str(row, &base_idx, "MATCHUP"),
            row_i64(row, &base_idx, "PTS").map(|p| p as i32),
        )
        else {
            continue;
        };

        let Some(&adv) = adv_map.get(&(game_id.to_string(), team_id)) else {
            continue;
        };

        let stats = BasketballTeamStats {
            min: row_f64(row, &base_idx, "MIN"),
            fgm: row_f64(row, &base_idx, "FGM"),
            fga: row_f64(row, &base_idx, "FGA"),
            fg_pct: row_f64(row, &base_idx, "FG_PCT"),
            fg3m: row_f64(row, &base_idx, "FG3M"),
            fg3a: row_f64(row, &base_idx, "FG3A"),
            fg3_pct: row_f64(row, &base_idx, "FG3_PCT"),
            ftm: row_f64(row, &base_idx, "FTM"),
            fta: row_f64(row, &base_idx, "FTA"),
            ft_pct: row_f64(row, &base_idx, "FT_PCT"),
            oreb: row_f64(row, &base_idx, "OREB"),
            dreb: row_f64(row, &base_idx, "DREB"),
            reb: row_f64(row, &base_idx, "REB"),
            ast: row_f64(row, &base_idx, "AST"),
            tov: row_f64(row, &base_idx, "TOV"),
            stl: row_f64(row, &base_idx, "STL"),
            blk: row_f64(row, &base_idx, "BLK"),
            blka: row_f64(row, &base_idx, "BLKA"),
            pf: row_f64(row, &base_idx, "PF"),
            pfd: row_f64(row, &base_idx, "PFD"),
            plus_minus: row_f64(row, &base_idx, "PLUS_MINUS"),
            off_rating: adv.off_rating,
            def_rating: adv.def_rating,
            net_rating: adv.net_rating,
            pace: adv.pace,
            efg_pct: adv.efg_pct,
            ts_pct: adv.ts_pct,
            tm_tov_pct: adv.tm_tov_pct,
            oreb_pct: adv.oreb_pct,
            pie: adv.pie,
        };

        games
            .entry(game_id.to_string())
            .or_default()
            .push(NbaTeamGameRow {
                season_year: season_year.to_string(),
                team_id,
                team_abbr: row_str(row, &base_idx, "TEAM_ABBREVIATION").map(str::to_string),
                team_name: team_name.to_string(),
                match_date,
                matchup: matchup.to_string(),
                pts,
                stats,
            });
    }

    let mut result = Vec::with_capacity(games.len());
    for (game_id, mut rows) in games {
        if rows.len() != 2 {
            tracing::warn!(
                game_id = %game_id,
                rows = rows.len(),
                "Pominięto mecz NBA Stats z niekompletną parą drużyn"
            );
            continue;
        }
        let second = rows.pop().expect("zweryfikowano dokładnie 2 wiersze");
        let first = rows.pop().expect("zweryfikowano dokładnie 2 wiersze");

        let first_home = first.matchup.contains(" vs. ");
        let second_home = second.matchup.contains(" vs. ");
        let (home, away) = match (first_home, second_home) {
            (true, false) => (first, second),
            (false, true) => (second, first),
            // Mecze na terenie neutralnym mają oba wiersze z `" @ "` – wybieramy stronę po TEAM_ID.
            _ if first.team_id <= second.team_id => (first, second),
            _ => (second, first),
        };

        let mut home_participant =
            IngestedParticipant::new(normalize_nba_team_name(&home.team_name), home.team_abbr);
        home_participant.metadata = json!({ "nba_team_id": home.team_id });

        let mut away_participant =
            IngestedParticipant::new(normalize_nba_team_name(&away.team_name), away.team_abbr);
        away_participant.metadata = json!({ "nba_team_id": away.team_id });

        let overtime = home.stats.min.is_some_and(|m| m > OVERTIME_MIN_THRESHOLD)
            || away.stats.min.is_some_and(|m| m > OVERTIME_MIN_THRESHOLD);

        result.push(IngestedMatch {
            sport: "BASKETBALL",
            competition_code: "NBA".to_string(),
            competition_name: "NBA".to_string(),
            competition_country: Some("USA".to_string()),
            season: normalize_season_label(&home.season_year),
            match_date: home.match_date,
            status: "FINISHED".to_string(),
            home: home_participant,
            away: away_participant,
            home_score: Some(home.pts),
            away_score: Some(away.pts),
            winner: winner_from_scores(home.pts, away.pts, false).map(str::to_string),
            details: MatchDetails::Basketball {
                quarters: vec![],
                overtime,
                season_type: Some(season_type.to_string()),
                home_stats: Some(Box::new(home.stats)),
                away_stats: Some(Box::new(away.stats)),
            },
            external_ids: json!({ "nba_game_id": game_id }),
        });
    }

    result.sort_by(|a, b| {
        a.match_date.cmp(&b.match_date).then_with(|| {
            a.external_ids["nba_game_id"]
                .as_str()
                .cmp(&b.external_ids["nba_game_id"].as_str())
        })
    });
    Ok(result)
}

/// Pobiera sezon NBA z `stats.nba.com` (`Regular Season` oraz `Playoffs` dla `Base` i `Advanced`).
async fn sync_nba_stats_season(state: &AppState, season_start: i32) -> AppResult<usize> {
    let season_short = short_season_label(season_start);
    let timeout = Duration::from_secs(state.config.nba_stats_timeout_secs.max(10));
    let delay = Duration::from_millis(state.config.nba_stats_delay_ms);
    let mut total = 0;

    for season_type in NBA_SEASON_TYPES {
        let base_url = nba_team_gamelogs_url(&season_short, season_type, "Base");
        let base_json = fetch_text_with_timeout(
            &state.nba_stats_client,
            &base_url,
            &NBA_STATS_HEADERS,
            timeout,
            &NBA_STATS_RETRY_DELAYS,
        )
        .await?;
        tokio::time::sleep(delay).await;

        let adv_url = nba_team_gamelogs_url(&season_short, season_type, "Advanced");
        let adv_json = fetch_text_with_timeout(
            &state.nba_stats_client,
            &adv_url,
            &NBA_STATS_HEADERS,
            timeout,
            &NBA_STATS_RETRY_DELAYS,
        )
        .await?;
        tokio::time::sleep(delay).await;

        let matches = parse_nba_team_gamelogs(&base_json, &adv_json, season_type)?;
        if matches.is_empty() {
            continue;
        }
        let count = store_matches(&state.db, &matches).await?;
        tracing::info!(
            season = %season_short,
            season_type,
            count,
            "NBA Stats: zapisano mecze"
        );
        total += count;
    }

    Ok(total)
}

// ---------------------------------------------------------------------------
// BALLDONTLIE (NBA)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BalldontliePage {
    data: Vec<BalldontlieGame>,
    meta: Option<BalldontlieMeta>,
}

#[derive(Debug, Deserialize)]
struct BalldontlieMeta {
    /// Kursor następnej strony; brak oznacza koniec danych.
    next_cursor: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct BalldontlieGame {
    id: i64,
    date: String,
    datetime: Option<String>,
    season: Option<i32>,
    status: String,
    period: Option<i32>,
    postponed: Option<bool>,
    home_team_score: Option<i32>,
    visitor_team_score: Option<i32>,
    home_q1: Option<i32>,
    home_q2: Option<i32>,
    home_q3: Option<i32>,
    home_q4: Option<i32>,
    home_ot1: Option<i32>,
    home_ot2: Option<i32>,
    home_ot3: Option<i32>,
    visitor_q1: Option<i32>,
    visitor_q2: Option<i32>,
    visitor_q3: Option<i32>,
    visitor_q4: Option<i32>,
    visitor_ot1: Option<i32>,
    visitor_ot2: Option<i32>,
    visitor_ot3: Option<i32>,
    home_team: BalldontlieTeam,
    visitor_team: BalldontlieTeam,
}

#[derive(Debug, Deserialize)]
struct BalldontlieTeam {
    id: Option<i64>,
    full_name: String,
    abbreviation: Option<String>,
    conference: Option<String>,
}

impl BalldontlieTeam {
    fn into_participant(self) -> IngestedParticipant {
        let normalized = normalize_nba_team_name(&self.full_name).to_string();
        let mut participant = IngestedParticipant::new(normalized, self.abbreviation);
        let mut metadata = serde_json::Map::new();
        if let Some(id) = self.id {
            metadata.insert("balldontlie_id".to_string(), json!(id));
        }
        if let Some(conference) = self.conference {
            metadata.insert("conference".to_string(), json!(conference));
        }
        if !metadata.is_empty() {
            participant.metadata = Value::Object(metadata);
        }
        participant
    }
}

/// Mapuje pole `status` BALLDONTLIE: zakończone mecze mają `Final`, niezaczęte `period` równe 0.
fn map_balldontlie_status(
    status: &str,
    period: Option<i32>,
    postponed: Option<bool>,
) -> &'static str {
    if postponed == Some(true) {
        return "POSTPONED";
    }
    let lowered = status.to_ascii_lowercase();
    if lowered == "final" {
        "FINISHED"
    } else if lowered.contains("postponed") {
        "POSTPONED"
    } else if lowered.contains("cancel") {
        "CANCELLED"
    } else if period.unwrap_or(0) == 0 {
        "SCHEDULED"
    } else {
        "LIVE"
    }
}

/// Składa listę punktów w kolejnych kwartach i dogrywkach z pól `home_q*`/`visitor_q*`.
fn extract_balldontlie_quarters(game: &BalldontlieGame) -> Vec<(i32, i32)> {
    [
        (game.home_q1, game.visitor_q1),
        (game.home_q2, game.visitor_q2),
        (game.home_q3, game.visitor_q3),
        (game.home_q4, game.visitor_q4),
        (game.home_ot1, game.visitor_ot1),
        (game.home_ot2, game.visitor_ot2),
        (game.home_ot3, game.visitor_ot3),
    ]
    .into_iter()
    .filter_map(|(h, v)| h.zip(v))
    .collect()
}

/// Parsuje jedną stronę odpowiedzi `/v1/games` i zwraca mecze oraz kursor następnej strony.
pub fn parse_balldontlie_page(json: &str) -> AppResult<(Vec<IngestedMatch>, Option<i64>)> {
    let page: BalldontliePage = serde_json::from_str(json)
        .map_err(|e| AppError::ExternalData(format!("BALLDONTLIE: {}", e)))?;
    let next_cursor = page.meta.and_then(|m| m.next_cursor);

    let mut result = Vec::with_capacity(page.data.len());
    for game in page.data {
        // Dokładny czas startu jest w `datetime`; gdy go brak, zostaje sama data (północ UTC).
        let match_date = game
            .datetime
            .as_deref()
            .and_then(|d| DateTime::parse_from_rfc3339(d).ok())
            .map(|d| d.with_timezone(&Utc))
            .or_else(|| {
                game.date
                    .get(..10)
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    .map(|d| d.and_time(NaiveTime::MIN).and_utc())
            });
        let Some(match_date) = match_date else {
            continue;
        };

        let status = map_balldontlie_status(&game.status, game.period, game.postponed);
        let finished = status == "FINISHED";
        let scores = if finished {
            game.home_team_score.zip(game.visitor_team_score)
        } else {
            None
        };
        let quarters = if finished {
            extract_balldontlie_quarters(&game)
        } else {
            vec![]
        };
        let overtime = finished
            && (game.period.unwrap_or(0) > REGULATION_PERIODS
                || quarters.len() > REGULATION_PERIODS as usize);

        // Pole `season` z API eliminuje błędy w nietypowych sezonach (np. NBA Bubble 2020).
        let season_start = game.season.unwrap_or_else(|| season_start_year(match_date));

        result.push(IngestedMatch {
            sport: "BASKETBALL",
            competition_code: "NBA".to_string(),
            competition_name: "NBA".to_string(),
            competition_country: Some("USA".to_string()),
            season: full_season_label(season_start),
            match_date,
            status: status.to_string(),
            home: game.home_team.into_participant(),
            away: game.visitor_team.into_participant(),
            home_score: scores.map(|(h, _)| h),
            away_score: scores.map(|(_, a)| a),
            winner: scores
                .and_then(|(h, a)| winner_from_scores(h, a, false))
                .map(str::to_string),
            details: MatchDetails::Basketball {
                quarters,
                overtime,
                season_type: None,
                home_stats: None,
                away_stats: None,
            },
            external_ids: json!({ "balldontlie": game.id }),
        });
    }
    Ok((result, next_cursor))
}

/// Pobiera i zapisuje wszystkie strony meczów jednego sezonu NBA z BALLDONTLIE (strona po stronie).
async fn sync_nba_season(state: &AppState, key: &str, season_start: i32) -> AppResult<usize> {
    let mut total = 0;
    let mut cursor: Option<i64> = None;

    loop {
        let mut url = format!(
            "https://api.balldontlie.io/v1/games?seasons[]={}&per_page=100",
            season_start
        );
        if let Some(c) = cursor {
            url.push_str(&format!("&cursor={}", c));
        }

        let body = fetch_text(&state.http_client, &url, &[("Authorization", key)]).await?;
        let (matches, next_cursor) = parse_balldontlie_page(&body)?;
        total += store_matches(&state.db, &matches).await?;

        match next_cursor {
            Some(next) => {
                cursor = Some(next);
                tokio::time::sleep(BALLDONTLIE_REQUEST_DELAY).await;
            }
            None => break,
        }
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// TheSportsDB (Euroliga)
// ---------------------------------------------------------------------------

/// Zamienia wartość JSON (liczbę lub napis, bo API bywa niespójne) na `i32`.
fn json_to_i32(value: &Value) -> Option<i32> {
    value
        .as_i64()
        .map(|n| n as i32)
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Parsuje odpowiedź `eventsseason.php`. Pole `events` bywa `null`, gdy sezon nie ma meczów.
pub fn parse_thesportsdb_events(json: &str, season: &str) -> AppResult<Vec<IngestedMatch>> {
    let root: Value = serde_json::from_str(json)
        .map_err(|e| AppError::ExternalData(format!("TheSportsDB: {}", e)))?;

    let mut result = Vec::new();
    for event in root["events"].as_array().into_iter().flatten() {
        let (Some(home), Some(away)) =
            (event["strHomeTeam"].as_str(), event["strAwayTeam"].as_str())
        else {
            continue;
        };

        // Preferujemy pełny znacznik czasu (UTC), a w razie jego braku składamy datę i godzinę.
        let match_date = event["strTimestamp"]
            .as_str()
            .and_then(|t| NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S").ok())
            .map(|t| t.and_utc())
            .or_else(|| {
                let date = event["dateEvent"]
                    .as_str()
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())?;
                let time = event["strTime"]
                    .as_str()
                    .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M:%S").ok())
                    .unwrap_or(NaiveTime::MIN);
                Some(date.and_time(time).and_utc())
            });
        let Some(match_date) = match_date else {
            continue;
        };

        let scores = json_to_i32(&event["intHomeScore"]).zip(json_to_i32(&event["intAwayScore"]));
        let postponed = event["strPostponed"]
            .as_str()
            .is_some_and(|p| p.eq_ignore_ascii_case("yes"));
        let status = if postponed {
            "POSTPONED"
        } else if scores.is_some() {
            "FINISHED"
        } else {
            "SCHEDULED"
        };
        let scores = if status == "FINISHED" { scores } else { None };

        result.push(IngestedMatch {
            sport: "BASKETBALL",
            competition_code: "EUROLEAGUE".to_string(),
            competition_name: "Euroleague".to_string(),
            competition_country: Some("Europe".to_string()),
            season: season.to_string(),
            match_date,
            status: status.to_string(),
            home: IngestedParticipant::new(home, None),
            away: IngestedParticipant::new(away, None),
            home_score: scores.map(|(h, _)| h),
            away_score: scores.map(|(_, a)| a),
            winner: scores
                .and_then(|(h, a)| winner_from_scores(h, a, false))
                .map(str::to_string),
            details: MatchDetails::Basketball {
                quarters: vec![],
                overtime: false,
                season_type: None,
                home_stats: None,
                away_stats: None,
            },
            external_ids: json!({ "thesportsdb": event["idEvent"] }),
        });
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Synchronizacja
// ---------------------------------------------------------------------------

/// Importuje koszykówkę: NBA (wg `INGEST_NBA_SOURCE`: domyślnie `stats.nba.com` dla statystyk
/// i historii oraz `BALLDONTLIE` dla terminarza/kwart) i Euroligę (`THESPORTSDB_EUROLEAGUE_ID`).
pub async fn sync(state: &AppState, include_history: bool) -> AppResult<usize> {
    let current = season_start_year(Utc::now());
    let mut history_seasons: Vec<i32> = Vec::new();
    if include_history {
        let configured = if state.config.ingest_nba_seasons.is_empty() {
            vec![current - 2, current - 1]
        } else {
            state.config.ingest_nba_seasons.clone()
        };
        for season in configured {
            if season != current && !history_seasons.contains(&season) {
                history_seasons.push(season);
            }
        }
    }

    let mut total = 0;

    match state.config.ingest_nba_source {
        NbaIngestSource::Auto => {
            for season in &history_seasons {
                match sync_nba_stats_season(state, *season).await {
                    Ok(count) => total += count,
                    Err(err) => {
                        tracing::warn!(season, "NBA Stats: pominięto sezon historyczny: {}", err);
                        if let Some(key) = &state.config.balldontlie_api_key {
                            match sync_nba_season(state, key, *season).await {
                                Ok(count) => {
                                    tracing::info!(
                                        season,
                                        count,
                                        "BALLDONTLIE (fallback): zapisano mecze"
                                    );
                                    total += count;
                                }
                                Err(bdl_err) => tracing::warn!(
                                    season,
                                    "BALLDONTLIE (fallback): pominięto sezon: {}",
                                    bdl_err
                                ),
                            }
                        }
                    }
                }
            }

            match sync_nba_stats_season(state, current).await {
                Ok(count) => total += count,
                Err(err) => {
                    tracing::warn!(
                        season = current,
                        "NBA Stats: błąd sezonu bieżącego: {}",
                        err
                    );
                }
            }

            match &state.config.balldontlie_api_key {
                Some(key) => match sync_nba_season(state, key, current).await {
                    Ok(count) => {
                        tracing::info!(season = current, count, "BALLDONTLIE: zapisano mecze");
                        total += count;
                    }
                    Err(err) => tracing::warn!(
                        season = current,
                        "BALLDONTLIE: pominięto sezon bieżący: {}",
                        err
                    ),
                },
                None => tracing::info!(
                    "Brak BALLDONTLIE_API_KEY – pomijam terminarz i kwarty NBA (tylko stats.nba.com)"
                ),
            }
        }
        NbaIngestSource::NbaStats => {
            for season in history_seasons
                .iter()
                .copied()
                .chain(std::iter::once(current))
            {
                match sync_nba_stats_season(state, season).await {
                    Ok(count) => total += count,
                    Err(err) => tracing::warn!(season, "NBA Stats: pominięto sezon: {}", err),
                }
            }
        }
        NbaIngestSource::Balldontlie => match &state.config.balldontlie_api_key {
            Some(key) => {
                for season in history_seasons
                    .iter()
                    .copied()
                    .chain(std::iter::once(current))
                {
                    match sync_nba_season(state, key, season).await {
                        Ok(count) => {
                            tracing::info!(season, count, "BALLDONTLIE: zapisano mecze");
                            total += count;
                        }
                        Err(err) => {
                            tracing::warn!(season, "BALLDONTLIE: pominięto sezon: {}", err);
                        }
                    }
                }
            }
            None => tracing::warn!(
                "INGEST_NBA_SOURCE=balldontlie wymaga ustawionego BALLDONTLIE_API_KEY – pomijam NBA"
            ),
        },
    }

    if let Some(league_id) = &state.config.thesportsdb_euroleague_id {
        let mut euroleague_seasons = history_seasons;
        euroleague_seasons.push(current);

        for season in euroleague_seasons {
            let label = full_season_label(season);
            let url = format!(
                "https://www.thesportsdb.com/api/v1/json/123/eventsseason.php?id={}&s={}",
                league_id, label
            );
            let result = async {
                let body = fetch_text(&state.http_client, &url, &[]).await?;
                let matches = parse_thesportsdb_events(&body, &label)?;
                store_matches(&state.db, &matches).await
            }
            .await;

            match result {
                Ok(count) => {
                    tracing::info!(season = %label, count, "TheSportsDB: zapisano mecze");
                    total += count;
                }
                Err(err) => {
                    tracing::warn!(season = %label, "TheSportsDB: pominięto sezon: {}", err);
                }
            }
        }
    }

    Ok(total)
}
