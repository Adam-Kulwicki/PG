// src/ingest/cs2/pandascore.rs
//! Klient PandaScore Free (`/csgo/*`) dla terminarza, wyników na żywo i bieżących rezultatów CS2.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Datelike, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use super::teams::normalize_cs2_team_name;
use crate::{
    error::{AppError, AppResult},
    ingest::{fetch_text, store_matches, winner_from_scores, IngestedMatch, IngestedParticipant},
    models::match_model::{Cs2MapResult, Cs2Player, MatchDetails},
    state::AppState,
};

/// Bazowy adres REST API PandaScore dla wspólnych zasobów turniejowych (bez prefiksu gry).
const PANDASCORE_ROOT_API_URL: &str = "https://api.pandascore.co";

/// Bazowy adres REST API PandaScore dla Counter-Strike 2 (historyczny prefiks `/csgo`).
const PANDASCORE_API_BASE_URL: &str = "https://api.pandascore.co/csgo";

/// Zakres wsteczny w dniach dla endpointu `/matches/past` (ogranicza paginację w cyklach crona).
const PANDASCORE_PAST_WINDOW_DAYS: i64 = 14;

/// Maksymalna liczba rekordów na stronę w PandaScore.
const PANDASCORE_PAGE_SIZE: usize = 100;

/// Twarde ograniczenie liczby stron na jedno wywołanie endpointu (ochrona limitu 1000 req/h).
const PANDASCORE_MAX_PAGES: usize = 10;

/// Maksymalna liczba zapytań o składy turniejowe w jednym cyklu synchronizacji.
const PANDASCORE_MAX_TOURNAMENT_ROSTER_FETCHES: usize = 20;

#[derive(Debug, Deserialize)]
struct PandaScoreMatch {
    id: i64,
    status: String,
    #[serde(default)]
    forfeit: bool,
    #[serde(default)]
    draw: bool,
    number_of_games: Option<u8>,
    begin_at: Option<String>,
    scheduled_at: Option<String>,
    winner_id: Option<i64>,
    #[serde(default)]
    opponents: Vec<PandaScoreOpponentWrapper>,
    #[serde(default)]
    results: Vec<PandaScoreResult>,
    #[serde(default)]
    games: Vec<PandaScoreGame>,
    tournament: Option<PandaScoreTournament>,
    league: Option<PandaScoreNamedEntity>,
    serie: Option<PandaScoreSerie>,
}

#[derive(Debug, Deserialize)]
struct PandaScoreOpponentWrapper {
    opponent: PandaScoreTeam,
}

#[derive(Debug, Deserialize)]
struct PandaScoreTeam {
    id: i64,
    name: String,
    acronym: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PandaScoreResult {
    team_id: Option<i64>,
    score: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct PandaScoreGame {
    position: Option<u8>,
    map: Option<PandaScoreMap>,
    winner: Option<PandaScoreWinnerRef>,
}

#[derive(Debug, Deserialize)]
struct PandaScoreMap {
    name: Option<String>,
    slug: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PandaScoreWinnerRef {
    id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct PandaScoreTournament {
    name: Option<String>,
    tier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PandaScoreNamedEntity {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PandaScoreSerie {
    full_name: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PandaScoreRosterTeam {
    id: i64,
    #[serde(default)]
    players: Vec<PandaScorePlayer>,
}

#[derive(Debug, Deserialize)]
struct PandaScorePlayer {
    id: Option<i64>,
    name: String,
}

fn parse_pandascore_date(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

fn map_pandascore_status(status: &str, is_forfeit: bool, has_winner: bool) -> &'static str {
    match status.trim().to_ascii_lowercase().as_str() {
        "canceled" | "cancelled" => "CANCELLED",
        "postponed" => "POSTPONED",
        "running" => "LIVE",
        // Walkower bez wskazanego zwycięzcy rozliczamy jako anulowany (VOID w settlement).
        "finished" if is_forfeit && !has_winner => "CANCELLED",
        "finished" => "FINISHED",
        _ => "SCHEDULED",
    }
}

fn format_tournament_name(m: &PandaScoreMatch) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(name) = m
        .league
        .as_ref()
        .and_then(|l| l.name.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        parts.push(name.to_string());
    }
    if let Some(serie) = m.serie.as_ref() {
        let serie_name = serie
            .full_name
            .as_deref()
            .or(serie.name.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(s) = serie_name {
            if !parts.iter().any(|existing| existing == s) {
                parts.push(s.to_string());
            }
        }
    }
    if let Some(tournament) = m
        .tournament
        .as_ref()
        .and_then(|t| t.name.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !parts.iter().any(|existing| existing == tournament) {
            parts.push(tournament.to_string());
        }
    }

    (!parts.is_empty()).then(|| parts.join(" — "))
}

/// Parsuje odpowiedź `/tournaments/{id}/rosters` na mapę `team_id -> Vec<Cs2Player>`.
pub fn parse_pandascore_tournament_rosters(
    json_body: &str,
) -> AppResult<HashMap<i64, Vec<Cs2Player>>> {
    let root: Value = serde_json::from_str(json_body)
        .map_err(|err| AppError::ExternalData(format!("PandaScore rosters: {err}")))?;

    let rosters_value = if root.is_array() {
        root
    } else if let Some(inner) = root.get("rosters") {
        inner.clone()
    } else {
        return Ok(HashMap::new());
    };

    let teams: Vec<PandaScoreRosterTeam> = serde_json::from_value(rosters_value)
        .map_err(|err| AppError::ExternalData(format!("PandaScore rosters schema: {err}")))?;

    let mut result = HashMap::with_capacity(teams.len());
    for team in teams {
        let players = team
            .players
            .into_iter()
            .map(|p| Cs2Player {
                external_id: p.id,
                name: p.name.trim().to_string(),
            })
            .filter(|p| !p.name.is_empty())
            .collect();
        result.insert(team.id, players);
    }
    Ok(result)
}

/// Parsuje listę meczów z endpointów `/csgo/matches/{upcoming,running,past}`.
pub fn parse_pandascore_matches(
    json_body: &str,
    rosters_by_team: &HashMap<i64, Vec<Cs2Player>>,
) -> AppResult<Vec<IngestedMatch>> {
    let matches: Vec<PandaScoreMatch> = serde_json::from_str(json_body)
        .map_err(|err| AppError::ExternalData(format!("PandaScore matches: {err}")))?;

    let mut ingested = Vec::with_capacity(matches.len());
    for m in matches {
        // Pomijamy nieobsadzone pary drabinkowe (TBD vs TBD).
        if m.opponents.len() < 2 {
            continue;
        }
        let home_team = &m.opponents[0].opponent;
        let away_team = &m.opponents[1].opponent;

        let Some(match_date) = m
            .begin_at
            .as_deref()
            .or(m.scheduled_at.as_deref())
            .and_then(parse_pandascore_date)
        else {
            continue;
        };

        let status = map_pandascore_status(&m.status, m.forfeit, m.winner_id.is_some());
        let is_finished = status == "FINISHED";

        let find_score = |team_id: i64| {
            m.results
                .iter()
                .find(|r| r.team_id == Some(team_id))
                .and_then(|r| r.score)
        };
        let series_scores = if is_finished {
            find_score(home_team.id).zip(find_score(away_team.id))
        } else {
            None
        };

        let winner = if is_finished {
            if m.draw {
                Some("DRAW".to_string())
            } else if m.winner_id == Some(home_team.id) {
                Some("HOME".to_string())
            } else if m.winner_id == Some(away_team.id) {
                Some("AWAY".to_string())
            } else {
                series_scores
                    .and_then(|(h, a)| winner_from_scores(h, a, true))
                    .map(str::to_string)
            }
        } else {
            None
        };

        let tournament_name = format_tournament_name(&m);
        let tournament_tier = m.tournament.as_ref().and_then(|t| t.tier.clone());

        let mut games = m.games;
        games.sort_by_key(|g| g.position.unwrap_or(u8::MAX));
        let maps: Vec<Cs2MapResult> = games
            .into_iter()
            .filter_map(|game| {
                let map_name = game
                    .map
                    .as_ref()
                    .and_then(|map| map.name.as_deref().or(map.slug.as_deref()))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())?
                    .to_string();
                let map_winner = game.winner.and_then(|w| w.id).and_then(|winner_id| {
                    if winner_id == home_team.id {
                        Some("HOME".to_string())
                    } else if winner_id == away_team.id {
                        Some("AWAY".to_string())
                    } else {
                        None
                    }
                });
                Some(Cs2MapResult {
                    map_name,
                    home_score: None,
                    away_score: None,
                    winner: map_winner,
                })
            })
            .collect();

        let mut home_participant = IngestedParticipant::new(
            normalize_cs2_team_name(&home_team.name),
            home_team.acronym.clone(),
        );
        home_participant.metadata = json!({ "pandascore_id": home_team.id });

        let mut away_participant = IngestedParticipant::new(
            normalize_cs2_team_name(&away_team.name),
            away_team.acronym.clone(),
        );
        away_participant.metadata = json!({ "pandascore_id": away_team.id });

        let home_roster = rosters_by_team
            .get(&home_team.id)
            .cloned()
            .unwrap_or_default();
        let away_roster = rosters_by_team
            .get(&away_team.id)
            .cloned()
            .unwrap_or_default();

        ingested.push(IngestedMatch {
            sport: "CS2",
            competition_code: "CS2".to_string(),
            competition_name: "Counter-Strike 2".to_string(),
            competition_country: Some("International".to_string()),
            season: match_date.year().to_string(),
            match_date,
            status: status.to_string(),
            home: home_participant,
            away: away_participant,
            home_score: series_scores.map(|(h, _)| h),
            away_score: series_scores.map(|(_, a)| a),
            winner,
            details: MatchDetails::Cs2 {
                best_of: m.number_of_games,
                tournament_name,
                tournament_tier,
                maps,
                home_roster,
                away_roster,
                player_stats: Vec::new(),
                veto: Vec::new(),
            },
            external_ids: json!({ "pandascore": m.id }),
        });
    }

    Ok(ingested)
}

async fn fetch_paginated_matches(
    client: &reqwest::Client,
    auth_header: &str,
    endpoint: &str,
    extra_query: Option<&str>,
) -> AppResult<Vec<Value>> {
    let mut raw_matches = Vec::new();
    for page in 1..=PANDASCORE_MAX_PAGES {
        let mut url = format!(
            "{PANDASCORE_API_BASE_URL}/{endpoint}?per_page={PANDASCORE_PAGE_SIZE}&page={page}"
        );
        if let Some(query) = extra_query {
            url.push('&');
            url.push_str(query);
        }

        let body = fetch_text(client, &url, &[("Authorization", auth_header)]).await?;
        let page_items: Vec<Value> = serde_json::from_str(&body)
            .map_err(|err| AppError::ExternalData(format!("PandaScore {endpoint}: {err}")))?;
        let page_count = page_items.len();
        raw_matches.extend(page_items);

        if page_count < PANDASCORE_PAGE_SIZE {
            break;
        }
    }
    Ok(raw_matches)
}

/// Buduje parametr `range[begin_at]` w formacie ISO 8601 UTC z sufiksem `Z` (bez `+00:00` i nanosekund,
/// ponieważ niezakodowane `+` w query stringu PandaScore traktuje jako spację i zwraca `400 Bad Request`).
pub fn build_pandascore_past_range(now: DateTime<Utc>) -> String {
    let start = (now - Duration::days(PANDASCORE_PAST_WINDOW_DAYS))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let end = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    format!("range[begin_at]={start},{end}")
}

/// Pobiera terminarz, mecze na żywo oraz zakończone z ostatnich 14 dni z PandaScore Free.
pub async fn sync_pandascore(state: &AppState, api_key: &str) -> AppResult<usize> {
    let auth_header = format!("Bearer {api_key}");
    let past_range = build_pandascore_past_range(Utc::now());

    let mut all_raw = Vec::new();
    for (endpoint, extra_query) in [
        ("matches/upcoming", None),
        ("matches/running", None),
        ("matches/past", Some(past_range.as_str())),
    ] {
        let items =
            fetch_paginated_matches(&state.http_client, &auth_header, endpoint, extra_query)
                .await?;
        all_raw.extend(items);
    }

    if all_raw.is_empty() {
        return Ok(0);
    }

    let tournament_ids: BTreeSet<i64> = all_raw
        .iter()
        .filter_map(|item| item.get("tournament")?.get("id")?.as_i64())
        .take(PANDASCORE_MAX_TOURNAMENT_ROSTER_FETCHES)
        .collect();

    let mut rosters_by_team: HashMap<i64, Vec<Cs2Player>> = HashMap::new();
    for tournament_id in tournament_ids {
        let url = format!("{PANDASCORE_ROOT_API_URL}/tournaments/{tournament_id}/rosters");
        match fetch_text(&state.http_client, &url, &[("Authorization", &auth_header)]).await {
            Ok(body) => match parse_pandascore_tournament_rosters(&body) {
                Ok(parsed) => rosters_by_team.extend(parsed),
                Err(err) => tracing::warn!(
                    tournament_id,
                    "PandaScore: niepoprawny format składów turnieju: {err}"
                ),
            },
            Err(err) => tracing::warn!(
                tournament_id,
                "PandaScore: pominięto składy turnieju: {err}"
            ),
        }
    }

    let combined_json = serde_json::to_string(&all_raw)
        .map_err(|err| AppError::ExternalData(format!("PandaScore serialize: {err}")))?;
    let matches = parse_pandascore_matches(&combined_json, &rosters_by_team)?;
    let saved_count = store_matches(&state.db, &matches).await?;
    tracing::info!(saved_count, "PandaScore CS2: zapisano mecze");
    Ok(saved_count)
}
