// tests/ingest_football.rs
use backend::{
    ingest::{
        football::{normalize_season_label, parse_football_data, parse_openfootball},
        short_season_label, winner_from_scores,
    },
    models::match_model::MatchDetails,
};

/// Etykiety sezonów z różnych źródeł sprowadzamy do formatu `RRRR-RRRR`.
#[test]
fn normalizes_season_labels() {
    assert_eq!(normalize_season_label("2023-24"), "2023-2024");
    assert_eq!(normalize_season_label("2023/24"), "2023-2024");
    assert_eq!(normalize_season_label("2023-2024"), "2023-2024");
    assert_eq!(normalize_season_label("2026"), "2026");
    assert_eq!(short_season_label(2024), "2024-25");
}

/// Remis jest dozwolony tylko w piłce nożnej.
#[test]
fn picks_winner_from_scores() {
    assert_eq!(winner_from_scores(2, 1, true), Some("HOME"));
    assert_eq!(winner_from_scores(0, 3, false), Some("AWAY"));
    assert_eq!(winner_from_scores(1, 1, true), Some("DRAW"));
    assert_eq!(winner_from_scores(90, 90, false), None);
}

/// Rozegrany mecz dostaje wynik i status FINISHED, a nierozegrany pozostaje SCHEDULED.
#[test]
fn parses_openfootball_matches() {
    let json = r#"{
        "name": "English Premier League 2023/24",
        "matches": [
            {"round": "Matchday 1", "date": "2023-08-11", "time": "20:00",
             "team1": "Burnley FC", "team2": "Manchester City FC",
             "score": {"ht": [0, 3], "ft": [0, 3]}},
            {"round": "Matchday 2", "date": "2023-08-19",
             "team1": {"name": "Arsenal FC", "code": "ARS"}, "team2": "Chelsea FC"},
            {"round": "Matchday 3", "date": "not-a-date", "team1": "A", "team2": "B"}
        ]
    }"#;

    let matches = parse_openfootball(json, "2023-24").unwrap();
    // Mecz z nieczytelną datą jest pomijany.
    assert_eq!(matches.len(), 2);

    let first = &matches[0];
    assert_eq!(first.season, "2023-2024");
    assert_eq!(first.status, "FINISHED");
    assert_eq!(first.home.name, "Burnley FC");
    assert_eq!((first.home_score, first.away_score), (Some(0), Some(3)));
    assert_eq!(first.winner.as_deref(), Some("AWAY"));
    assert_eq!(first.match_date.to_rfc3339(), "2023-08-11T20:00:00+00:00");

    let second = &matches[1];
    assert_eq!(second.status, "SCHEDULED");
    assert_eq!(second.home.short_code.as_deref(), Some("ARS"));
    assert_eq!(second.winner, None);
    // Brak godziny w źródle oznacza domyślne 15:00 UTC.
    assert_eq!(second.match_date.to_rfc3339(), "2023-08-19T15:00:00+00:00");
}

/// Niepoprawny JSON kończy się błędem, a nie paniką.
#[test]
fn rejects_malformed_openfootball_json() {
    assert!(parse_openfootball("not json", "2023-24").is_err());
}

/// OpenFootball bywa zapisywany z `"score": [0, 0]` bez `ft`/`ht`, a uszkodzony pojedynczy
/// rekord nie powinien przerywać parsowania pozostałych meczów w sezonie.
#[test]
fn parses_openfootball_direct_score_and_skips_malformed_entry() {
    let json = r#"{
        "name": "English Premier League 2025/26",
        "matches": [
            {"round": "Matchday 1", "date": "2025-08-16", "time": "12:30",
             "team1": "Aston Villa FC", "team2": "Newcastle United FC",
             "score": [0, 0]},
            {"round": "Matchday 1", "date": "2025-08-16", "time": "15:00",
             "team1": "Broken FC", "team2": "Invalid FC",
             "score": 0},
            {"round": "Matchday 1", "date": "2025-08-16", "time": "15:00",
             "team1": "Brighton & Hove Albion FC", "team2": "Fulham FC",
             "score": {"ft": [1, 1], "ht": [0, 0]}}
        ]
    }"#;

    let matches = parse_openfootball(json, "2025-26").unwrap();
    assert_eq!(matches.len(), 2);

    let direct = &matches[0];
    assert_eq!(direct.season, "2025-2026");
    assert_eq!(direct.status, "FINISHED");
    assert_eq!(direct.home.name, "Aston Villa FC");
    assert_eq!(direct.away.name, "Newcastle United FC");
    assert_eq!((direct.home_score, direct.away_score), (Some(0), Some(0)));
    assert_eq!(direct.winner.as_deref(), Some("DRAW"));
    match &direct.details {
        MatchDetails::Football {
            ht_home,
            ht_away,
            round,
        } => {
            assert_eq!((*ht_home, *ht_away), (None, None));
            assert_eq!(round, "Matchday 1");
        }
        other => panic!("Nieoczekiwany wariant MatchDetails: {:?}", other),
    }

    let valid = &matches[1];
    assert_eq!(valid.home.name, "Brighton & Hove Albion FC");
    assert_eq!((valid.home_score, valid.away_score), (Some(1), Some(1)));
}

/// Football-Data: wynik zapisujemy tylko dla zakończonych meczów, a mecze bez nazw drużyn pomijamy.
#[test]
fn parses_football_data_matches() {
    let json = r#"{
        "matches": [
            {"id": 1, "utcDate": "2025-08-15T19:00:00Z", "status": "FINISHED", "matchday": 1,
             "season": {"startDate": "2025-08-15", "endDate": "2026-05-24"},
             "homeTeam": {"name": "Liverpool FC", "tla": "LIV"},
             "awayTeam": {"name": "AFC Bournemouth", "tla": "BOU"},
             "score": {"winner": "HOME_TEAM",
                       "fullTime": {"home": 4, "away": 2}, "halfTime": {"home": 1, "away": 0}}},
            {"id": 2, "utcDate": "2025-08-16T14:00:00Z", "status": "IN_PLAY", "matchday": 1,
             "season": {"startDate": "2025-08-15", "endDate": "2026-05-24"},
             "homeTeam": {"name": "Arsenal FC", "tla": "ARS"},
             "awayTeam": {"name": "Chelsea FC", "tla": "CHE"},
             "score": {"winner": null,
                       "fullTime": {"home": 1, "away": 0}, "halfTime": {"home": 1, "away": 0}}},
            {"id": 3, "utcDate": "2025-08-17T14:00:00Z", "status": "TIMED", "matchday": 2,
             "season": {"startDate": "2025-08-15", "endDate": "2026-05-24"},
             "homeTeam": {"name": null, "tla": null},
             "awayTeam": {"name": "Chelsea FC", "tla": "CHE"},
             "score": {"winner": null, "fullTime": {"home": null, "away": null},
                       "halfTime": {"home": null, "away": null}}}
        ]
    }"#;

    let matches = parse_football_data(json).unwrap();
    assert_eq!(matches.len(), 2);

    let finished = &matches[0];
    assert_eq!(finished.season, "2025-2026");
    assert_eq!(finished.status, "FINISHED");
    assert_eq!(finished.winner.as_deref(), Some("HOME"));
    assert_eq!(
        (finished.home_score, finished.away_score),
        (Some(4), Some(2))
    );
    assert_eq!(finished.home.short_code.as_deref(), Some("LIV"));
    assert_eq!(finished.external_ids["football_data"], 1);

    // Trwający mecz nie może wyglądać na rozstrzygnięty.
    let live = &matches[1];
    assert_eq!(live.status, "LIVE");
    assert_eq!((live.home_score, live.away_score), (None, None));
    assert_eq!(live.winner, None);
}
