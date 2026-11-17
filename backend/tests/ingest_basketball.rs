// tests/ingest_basketball.rs
use backend::{
    ingest::basketball::{parse_balldontlie_page, parse_thesportsdb_events},
    models::match_model::MatchDetails,
};

/// BALLDONTLIE: status, wynik, dogrywka i kursor następnej strony.
#[test]
fn parses_balldontlie_page() {
    let json = r#"{
        "data": [
            {"id": 10, "date": "2024-10-22", "datetime": "2024-10-23T23:30:00.000Z",
             "season": 2024, "status": "Final", "period": 5,
             "home_team_score": 109, "visitor_team_score": 116,
             "home_team": {"full_name": "Boston Celtics", "abbreviation": "BOS", "conference": "East"},
             "visitor_team": {"full_name": "New York Knicks", "abbreviation": "NYK", "conference": "East"}},
            {"id": 11, "date": "2024-10-24", "datetime": "2024-10-25T01:00:00.000Z",
             "season": 2024, "status": "2024-10-25T01:00:00Z", "period": 0,
             "home_team_score": 0, "visitor_team_score": 0,
             "home_team": {"full_name": "Los Angeles Lakers", "abbreviation": "LAL", "conference": "West"},
             "visitor_team": {"full_name": "Phoenix Suns", "abbreviation": "PHX", "conference": "West"}}
        ],
        "meta": {"next_cursor": 12, "per_page": 100}
    }"#;

    let (matches, cursor) = parse_balldontlie_page(json).unwrap();
    assert_eq!(cursor, Some(12));
    assert_eq!(matches.len(), 2);

    let finished = &matches[0];
    assert_eq!(finished.season, "2024-2025");
    assert_eq!(finished.status, "FINISHED");
    assert_eq!(finished.winner.as_deref(), Some("AWAY"));
    assert_eq!((finished.home_score, finished.away_score), (Some(109), Some(116)));
    assert!(matches!(
        finished.details,
        MatchDetails::Basketball { overtime: true, .. }
    ));
    assert_eq!(finished.home.metadata["conference"], "East");

    // Mecz niezaczęty nie może mieć wyniku, choć API zwraca zera.
    let scheduled = &matches[1];
    assert_eq!(scheduled.status, "SCHEDULED");
    assert_eq!((scheduled.home_score, scheduled.away_score), (None, None));
    assert_eq!(scheduled.winner, None);
}

/// Ostatnia strona nie ma kursora.
#[test]
fn last_page_has_no_cursor() {
    let (matches, cursor) = parse_balldontlie_page(r#"{"data": [], "meta": {"per_page": 100}}"#).unwrap();
    assert!(matches.is_empty());
    assert_eq!(cursor, None);
}

/// TheSportsDB: wyniki bywają napisami, a `events` może być `null`.
#[test]
fn parses_thesportsdb_events() {
    let json = r#"{
        "events": [
            {"idEvent": "1", "strHomeTeam": "Real Madrid", "strAwayTeam": "Olympiacos",
             "intHomeScore": "88", "intAwayScore": "81",
             "strTimestamp": "2024-10-03T19:30:00", "dateEvent": "2024-10-03", "strTime": "19:30:00"},
            {"idEvent": "2", "strHomeTeam": "Fenerbahce", "strAwayTeam": "Zalgiris",
             "intHomeScore": null, "intAwayScore": null,
             "dateEvent": "2024-10-05", "strTime": "18:00:00"}
        ]
    }"#;

    let matches = parse_thesportsdb_events(json, "2024-2025").unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].status, "FINISHED");
    assert_eq!(matches[0].winner.as_deref(), Some("HOME"));
    assert_eq!(matches[0].match_date.to_rfc3339(), "2024-10-03T19:30:00+00:00");
    assert_eq!(matches[1].status, "SCHEDULED");
    // Brak znacznika czasu: data i godzina składane są z osobnych pól.
    assert_eq!(matches[1].match_date.to_rfc3339(), "2024-10-05T18:00:00+00:00");

    assert!(parse_thesportsdb_events(r#"{"events": null}"#, "2024-2025")
        .unwrap()
        .is_empty());
}
