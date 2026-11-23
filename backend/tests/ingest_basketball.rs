// tests/ingest_basketball.rs
use backend::{
    api::matches::{normalize_pagination, MAX_MATCHES_LIMIT},
    config::NbaIngestSource,
    ingest::{
        basketball::{
            nba_team_gamelogs_url, normalize_nba_team_name, parse_balldontlie_page,
            parse_nba_team_gamelogs, parse_thesportsdb_events,
        },
        choose_match_date, merge_existing_match, merge_match_details, select_matching_row,
        ExistingMatchRow, IngestedMatch, IngestedParticipant,
    },
    models::match_model::{BasketballTeamStats, MatchDetails},
    state::build_nba_stats_client,
};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::types::Json;

fn dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn sample_stats(min: f64, off_rating: f64) -> BasketballTeamStats {
    BasketballTeamStats {
        min: Some(min),
        off_rating: Some(off_rating),
        ..Default::default()
    }
}

fn sample_nba_ingested(
    date: &str,
    home_name: &str,
    away_name: &str,
    scores: Option<(i32, i32)>,
    external_ids: Value,
    details: MatchDetails,
) -> IngestedMatch {
    let winner = scores.map(|(h, a)| if h > a { "HOME" } else { "AWAY" }.to_string());
    IngestedMatch {
        sport: "BASKETBALL",
        competition_code: "NBA".to_string(),
        competition_name: "NBA".to_string(),
        competition_country: Some("USA".to_string()),
        season: "2024-2025".to_string(),
        match_date: dt(date),
        status: if scores.is_some() {
            "FINISHED"
        } else {
            "SCHEDULED"
        }
        .to_string(),
        home: IngestedParticipant::new(home_name, None),
        away: IngestedParticipant::new(away_name, None),
        home_score: scores.map(|(h, _)| h),
        away_score: scores.map(|(_, a)| a),
        winner,
        details,
        external_ids,
    }
}

/// BALLDONTLIE: status, kwarty, dogrywka, przełożenie, jawny sezon, metadane i kursor.
#[test]
fn parses_balldontlie_page() {
    let json = r#"{
        "data": [
            {"id": 10, "date": "2020-10-11", "datetime": "2020-10-12T00:30:00.000Z",
             "season": 2019, "status": "Final", "period": 5, "postponed": false,
             "home_team_score": 109, "visitor_team_score": 116,
             "home_q1": 25, "home_q2": 28, "home_q3": 24, "home_q4": 24, "home_ot1": 8,
             "visitor_q1": 27, "visitor_q2": 26, "visitor_q3": 25, "visitor_q4": 23, "visitor_ot1": 15,
             "home_team": {"id": 2, "full_name": "Boston Celtics", "abbreviation": "BOS", "conference": "East"},
             "visitor_team": {"id": 20, "full_name": "LA Clippers", "abbreviation": "LAC", "conference": "West"}},
            {"id": 11, "date": "2024-10-24", "datetime": "2024-10-25T01:00:00.000Z",
             "season": 2024, "status": "2024-10-25T01:00:00Z", "period": 0, "postponed": false,
             "home_team_score": 0, "visitor_team_score": 0,
             "home_q1": 0, "visitor_q1": 0,
             "home_team": {"id": 14, "full_name": "Los Angeles Lakers", "abbreviation": "LAL", "conference": "West"},
             "visitor_team": {"id": 24, "full_name": "Phoenix Suns", "abbreviation": "PHX", "conference": "West"}},
            {"id": 12, "date": "2025-01-10", "datetime": "2025-01-11T03:30:00.000Z",
             "season": 2024, "status": "Scheduled", "period": 0, "postponed": true,
             "home_team": {"id": 14, "full_name": "Los Angeles Lakers", "abbreviation": "LAL", "conference": "West"},
             "visitor_team": {"id": 4, "full_name": "Charlotte Hornets", "abbreviation": "CHA", "conference": "East"}}
        ],
        "meta": {"next_cursor": 12, "per_page": 100}
    }"#;

    let (matches, cursor) = parse_balldontlie_page(json).unwrap();
    assert_eq!(cursor, Some(12));
    assert_eq!(matches.len(), 3);

    // Mecz z NBA Bubble w październiku zachowuje jawny `season: 2019` zamiast zgadywania z daty.
    let finished = &matches[0];
    assert_eq!(finished.season, "2019-2020");
    assert_eq!(finished.status, "FINISHED");
    assert_eq!(finished.winner.as_deref(), Some("AWAY"));
    assert_eq!(finished.away.name, "Los Angeles Clippers");
    assert_eq!(
        (finished.home_score, finished.away_score),
        (Some(109), Some(116))
    );
    assert_eq!(finished.home.metadata["conference"], "East");
    assert_eq!(finished.home.metadata["balldontlie_id"], 2);
    assert_eq!(finished.away.metadata["balldontlie_id"], 20);
    assert_eq!(finished.external_ids["balldontlie"], 10);
    match &finished.details {
        MatchDetails::Basketball {
            quarters,
            overtime,
            season_type,
            home_stats,
            away_stats,
        } => {
            assert_eq!(
                quarters,
                &vec![(25, 27), (28, 26), (24, 25), (24, 23), (8, 15)]
            );
            assert!(*overtime);
            assert_eq!(season_type, &None);
            assert_eq!(home_stats, &None);
            assert_eq!(away_stats, &None);
        }
        other => panic!("oczekiwano MatchDetails::Basketball, otrzymano {:?}", other),
    }

    // Mecz niezaczęty nie może mieć wyniku ani kwart, choć API potrafi zwrócić zera.
    let scheduled = &matches[1];
    assert_eq!(scheduled.status, "SCHEDULED");
    assert_eq!((scheduled.home_score, scheduled.away_score), (None, None));
    assert_eq!(scheduled.winner, None);
    assert!(matches!(
        &scheduled.details,
        MatchDetails::Basketball { quarters, overtime: false, .. } if quarters.is_empty()
    ));

    // Flaga `postponed: true` nadpisuje status na POSTPONED.
    assert_eq!(matches[2].status, "POSTPONED");
}

/// Ostatnia strona nie ma kursora.
#[test]
fn last_page_has_no_cursor() {
    let (matches, cursor) =
        parse_balldontlie_page(r#"{"data": [], "meta": {"per_page": 100}}"#).unwrap();
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
    assert_eq!(
        matches[0].match_date.to_rfc3339(),
        "2024-10-03T19:30:00+00:00"
    );
    assert_eq!(matches[1].status, "SCHEDULED");
    // Brak znacznika czasu: data i godzina składane są z osobnych pól.
    assert_eq!(
        matches[1].match_date.to_rfc3339(),
        "2024-10-05T18:00:00+00:00"
    );

    assert!(parse_thesportsdb_events(r#"{"events": null}"#, "2024-2025")
        .unwrap()
        .is_empty());
}

/// Prawdziwe przycięte odpowiedzi `stats.nba.com` (`Base` + `Advanced`):
/// ułamkowy `MIN` (bez dogrywki), 2OT (`58.0`), mecz neutralny (`@` / `@`), `"LA Clippers"`
/// oraz wszystkie 9 metryk `Advanced`.
#[test]
fn parses_nba_team_gamelogs_from_real_fixtures() {
    let base_json = include_str!("fixtures/nba_base_sample.json");
    let adv_json = include_str!("fixtures/nba_advanced_sample.json");

    let matches = parse_nba_team_gamelogs(base_json, adv_json, "Regular Season").unwrap();
    assert_eq!(matches.len(), 4);

    // 1. 2024-12-10: OKC vs. DAL (MIN = 48.033333 -> brak dogrywki).
    let frac = &matches[0];
    assert_eq!(frac.external_ids["nba_game_id"], "0022401203");
    assert_eq!(frac.season, "2024-2025");
    assert_eq!(frac.match_date, dt("2024-12-10T00:00:00Z"));
    assert_eq!(frac.home.name, "Oklahoma City Thunder");
    assert_eq!(frac.away.name, "Dallas Mavericks");
    assert_eq!((frac.home_score, frac.away_score), (Some(118), Some(104)));
    assert_eq!(frac.winner.as_deref(), Some("HOME"));
    match &frac.details {
        MatchDetails::Basketball {
            overtime,
            season_type,
            home_stats,
            away_stats,
            ..
        } => {
            assert!(!*overtime);
            assert_eq!(season_type.as_deref(), Some("Regular Season"));
            let h = home_stats.as_ref().unwrap();
            let a = away_stats.as_ref().unwrap();
            assert_eq!(h.min, Some(48.033333));
            assert_eq!(h.tov, Some(14.0));
            assert_eq!(h.plus_minus, Some(14.0));
            assert_eq!(h.off_rating, Some(118.0));
            assert_eq!(a.off_rating, Some(104.0));
        }
        other => panic!("oczekiwano MatchDetails::Basketball, otrzymano {:?}", other),
    }

    // 2. 2025-01-25: IND @ SAS i SAS @ IND (mecz neutralny w Paryżu; mniejszy TEAM_ID = IND).
    let neutral = &matches[1];
    assert_eq!(neutral.external_ids["nba_game_id"], "0022400633");
    assert_eq!(neutral.home.name, "Indiana Pacers");
    assert_eq!(neutral.away.name, "San Antonio Spurs");
    assert_eq!(neutral.home.metadata["nba_team_id"], 1610612754);
    assert_eq!(neutral.away.metadata["nba_team_id"], 1610612759);
    assert_eq!(
        (neutral.home_score, neutral.away_score),
        (Some(136), Some(98))
    );
    assert_eq!(neutral.winner.as_deref(), Some("HOME"));

    // 3. 2025-04-11: SAC vs. LAC ("LA Clippers" -> "Los Angeles Clippers") + wszystkie metryki.
    let lac = &matches[2];
    assert_eq!(lac.external_ids["nba_game_id"], "0022401184");
    assert_eq!(lac.home.name, "Sacramento Kings");
    assert_eq!(lac.home.short_code.as_deref(), Some("SAC"));
    assert_eq!(lac.away.name, "Los Angeles Clippers");
    assert_eq!(lac.away.short_code.as_deref(), Some("LAC"));
    assert_eq!(lac.away.metadata["nba_team_id"], 1610612746);
    assert_eq!((lac.home_score, lac.away_score), (Some(100), Some(101)));
    assert_eq!(lac.winner.as_deref(), Some("AWAY"));
    match &lac.details {
        MatchDetails::Basketball {
            overtime,
            away_stats,
            ..
        } => {
            assert!(!*overtime);
            let s = away_stats.as_ref().unwrap();
            assert_eq!(s.min, Some(48.0));
            assert_eq!(s.fgm, Some(40.0));
            assert_eq!(s.fga, Some(78.0));
            assert_eq!(s.fg_pct, Some(0.513));
            assert_eq!(s.tov, Some(12.0));
            assert_eq!(s.plus_minus, Some(1.0));
            assert_eq!(s.off_rating, Some(111.0));
            assert_eq!(s.def_rating, Some(107.5));
            assert_eq!(s.net_rating, Some(3.5));
            assert_eq!(s.pace, Some(92.0));
            assert_eq!(s.efg_pct, Some(0.558));
            assert_eq!(s.ts_pct, Some(0.588));
            assert_eq!(s.tm_tov_pct, Some(0.132));
            assert_eq!(s.oreb_pct, Some(0.195));
            assert_eq!(s.pie, Some(0.499));
        }
        other => panic!("oczekiwano MatchDetails::Basketball, otrzymano {:?}", other),
    }

    // 4. 2025-04-13: CLE vs. IND (MIN = 58.0 -> podwójna dogrywka, overtime = true).
    let double_ot = &matches[3];
    assert_eq!(double_ot.external_ids["nba_game_id"], "0022401189");
    assert_eq!(double_ot.home.name, "Cleveland Cavaliers");
    assert_eq!(double_ot.away.name, "Indiana Pacers");
    assert_eq!(
        (double_ot.home_score, double_ot.away_score),
        (Some(118), Some(126))
    );
    assert_eq!(double_ot.winner.as_deref(), Some("AWAY"));
    match &double_ot.details {
        MatchDetails::Basketball {
            overtime,
            away_stats,
            ..
        } => {
            assert!(*overtime);
            let s = away_stats.as_ref().unwrap();
            assert_eq!(s.min, Some(58.0));
            assert_eq!(s.off_rating, Some(99.2));
            assert_eq!(s.def_rating, Some(95.2));
            assert_eq!(s.net_rating, Some(4.1));
            assert_eq!(s.pace, Some(103.86));
            assert_eq!(s.efg_pct, Some(0.445));
            assert_eq!(s.ts_pct, Some(0.49));
            assert_eq!(s.tm_tov_pct, Some(0.102));
            assert_eq!(s.oreb_pct, Some(0.264));
            assert_eq!(s.pie, Some(0.603));
        }
        other => panic!("oczekiwano MatchDetails::Basketball, otrzymano {:?}", other),
    }
}

/// Puste `rowSet` (np. Playoffs przed startem), brak pasującego wiersza w Advanced i błędny JSON.
#[test]
fn handles_empty_and_incomplete_nba_team_gamelogs() {
    let empty = r#"{"resultSets": [{"headers": ["GAME_ID"], "rowSet": []}]}"#;
    assert!(parse_nba_team_gamelogs(empty, empty, "Playoffs")
        .unwrap()
        .is_empty());

    assert!(parse_nba_team_gamelogs("not json", empty, "Regular Season").is_err());
    assert!(parse_nba_team_gamelogs(r#"{"resultSets": []}"#, empty, "Regular Season").is_err());

    let base_one_game = r#"{
        "resultSets": [{
            "headers": ["SEASON_YEAR", "TEAM_ID", "TEAM_ABBREVIATION", "TEAM_NAME", "GAME_ID", "GAME_DATE", "MATCHUP", "MIN", "PTS"],
            "rowSet": [
                ["2024-25", 1, "BOS", "Boston Celtics", "g1", "2024-10-22T00:00:00", "BOS vs. NYK", 48.0, 132],
                ["2024-25", 2, "NYK", "New York Knicks", "g1", "2024-10-22T00:00:00", "NYK @ BOS", 48.0, 109]
            ]
        }]
    }"#;
    let adv_only_one_team = r#"{
        "resultSets": [{
            "headers": ["GAME_ID", "TEAM_ID", "OFF_RATING"],
            "rowSet": [["g1", 1, 125.0]]
        }]
    }"#;
    // Brak drugiego zespołu w Advanced działa jak inner join – niekompletny mecz jest odrzucany.
    assert!(
        parse_nba_team_gamelogs(base_one_game, adv_only_one_team, "Regular Season")
            .unwrap()
            .is_empty()
    );
}

/// Dokładny URL `teamgamelogs` (21 parametrów query) i normalizacja nazw drużyn NBA.
#[test]
fn builds_nba_url_and_normalizes_team_names() {
    let url = nba_team_gamelogs_url("2024-25", "Regular Season", "Base");
    assert_eq!(
        url,
        "https://stats.nba.com/stats/teamgamelogs?DateFrom=&DateTo=&GameSegment=&LastNGames=&LeagueID=00&Location=&MeasureType=Base&Month=&OpponentTeamID=&Outcome=&PORound=&PerMode=&Period=&PlayerID=&Season=2024-25&SeasonSegment=&SeasonType=Regular+Season&ShotClockRange=&TeamID=&VsConference=&VsDivision="
    );
    assert_eq!(
        normalize_nba_team_name("LA Clippers"),
        "Los Angeles Clippers"
    );
    assert_eq!(
        normalize_nba_team_name("Charlotte Bobcats"),
        "Charlotte Hornets"
    );
    assert_eq!(normalize_nba_team_name("Boston Celtics"), "Boston Celtics");
}

/// Zgodność wsteczna JSONB: stare rekordy bez nowych pól deserializują się bez błędu,
/// a puste pola opcjonalne nie zaśmiecają zserializowanego JSON-a.
#[test]
fn match_details_basketball_serde_compat() {
    let legacy = r#"{"sport_type":"basketball","quarters":[[28,30],[25,29]],"overtime":false}"#;
    let parsed: MatchDetails = serde_json::from_str(legacy).unwrap();
    assert_eq!(
        parsed,
        MatchDetails::Basketball {
            quarters: vec![(28, 30), (25, 29)],
            overtime: false,
            season_type: None,
            home_stats: None,
            away_stats: None,
        }
    );

    let serialized = serde_json::to_value(&parsed).unwrap();
    assert!(serialized.get("season_type").is_none());
    assert!(serialized.get("home_stats").is_none());
    assert!(serialized.get("away_stats").is_none());
}

/// Deduplikacja i scalanie danych ze `stats.nba.com` i `BALLDONTLIE` (przesunięcie UTC,
/// ochrona meczów back-to-back co 24h, zamiana stron na terenie neutralnym i blokada cofania statusu).
#[test]
fn deduplicates_and_merges_nba_sources() {
    // 1. Dokładniejsza godzina UTC zawsze wygrywa z północą.
    assert_eq!(
        choose_match_date(dt("2024-11-05T01:45:00Z"), dt("2024-11-04T00:00:00Z")),
        dt("2024-11-05T01:45:00Z")
    );
    assert_eq!(
        choose_match_date(dt("2024-11-04T00:00:00Z"), dt("2024-11-05T01:45:00Z")),
        dt("2024-11-05T01:45:00Z")
    );

    // 2. Scalanie szczegółów działa symetrycznie w obu kolejnościach źródeł.
    let from_bdl = MatchDetails::Basketball {
        quarters: vec![(30, 25), (28, 27), (26, 29), (24, 20)],
        overtime: false,
        season_type: None,
        home_stats: None,
        away_stats: None,
    };
    let from_nba = MatchDetails::Basketball {
        quarters: vec![],
        overtime: true,
        season_type: Some("Regular Season".to_string()),
        home_stats: Some(Box::new(sample_stats(53.0, 115.2))),
        away_stats: Some(Box::new(sample_stats(53.0, 108.4))),
    };

    let bdl_then_nba = merge_match_details(from_bdl.clone(), from_nba.clone());
    let nba_then_bdl = merge_match_details(from_nba.clone(), from_bdl.clone());
    assert_eq!(bdl_then_nba, nba_then_bdl);
    match &bdl_then_nba {
        MatchDetails::Basketball {
            quarters,
            overtime,
            season_type,
            home_stats,
            away_stats,
        } => {
            assert_eq!(quarters.len(), 4);
            assert!(*overtime);
            assert_eq!(season_type.as_deref(), Some("Regular Season"));
            assert_eq!(home_stats.as_ref().unwrap().off_rating, Some(115.2));
            assert_eq!(away_stats.as_ref().unwrap().off_rating, Some(108.4));
        }
        other => panic!("oczekiwano MatchDetails::Basketball, otrzymano {:?}", other),
    }

    // 3. Przesunięcie UTC wieczornego meczu USA (Nov 4 00:00Z == Nov 5 01:45Z) oraz ochrona back-to-back.
    let cand_day1_bdl = ExistingMatchRow {
        id: 1,
        match_date: dt("2024-11-05T01:45:00Z"),
        status: "FINISHED".to_string(),
        home_participant_id: 10,
        away_participant_id: 20,
        home_score: Some(109),
        away_score: Some(97),
        winner: Some("HOME".to_string()),
        match_details: Json(from_bdl.clone()),
        external_ids: Json(json!({ "balldontlie": 1001 })),
    };
    let cand_day2_bdl = ExistingMatchRow {
        id: 2,
        match_date: dt("2024-11-06T01:30:00Z"),
        status: "FINISHED".to_string(),
        home_participant_id: 10,
        away_participant_id: 20,
        home_score: Some(112),
        away_score: Some(115),
        winner: Some("AWAY".to_string()),
        match_details: Json(from_bdl.clone()),
        external_ids: Json(json!({ "balldontlie": 1002 })),
    };

    let inc_day1_nba = sample_nba_ingested(
        "2024-11-04T00:00:00Z",
        "Houston Rockets",
        "New York Knicks",
        Some((109, 97)),
        json!({ "nba_game_id": "0022400155" }),
        from_nba.clone(),
    );
    assert_eq!(
        select_matching_row(
            &[cand_day2_bdl.clone(), cand_day1_bdl.clone()],
            &inc_day1_nba,
            10,
            20
        ),
        Some(1)
    );

    // Same-source back-to-back 24h później (inny `nba_game_id`) NIE może się scalić.
    let cand_day1_nba_only = ExistingMatchRow {
        id: 3,
        match_date: dt("2024-10-29T00:00:00Z"),
        status: "FINISHED".to_string(),
        home_participant_id: 30,
        away_participant_id: 40,
        home_score: Some(124),
        away_score: Some(106),
        winner: Some("HOME".to_string()),
        match_details: Json(from_nba.clone()),
        external_ids: Json(json!({ "nba_game_id": "0022400116" })),
    };
    let inc_day2_nba_only = sample_nba_ingested(
        "2024-10-30T00:00:00Z",
        "Golden State Warriors",
        "New Orleans Pelicans",
        Some((104, 89)),
        json!({ "nba_game_id": "0022400126" }),
        from_nba.clone(),
    );
    assert_eq!(
        select_matching_row(&[cand_day1_nba_only], &inc_day2_nba_only, 30, 40),
        None
    );

    // Mecz na terenie neutralnym ze zamienionymi stronami: zachowujemy kierunek z bazy.
    let existing_neutral = ExistingMatchRow {
        id: 4,
        match_date: dt("2025-01-25T17:00:00Z"),
        status: "FINISHED".to_string(),
        home_participant_id: 59, // SAS jako home w BALLDONTLIE
        away_participant_id: 54, // IND jako away w BALLDONTLIE
        home_score: Some(98),
        away_score: Some(136),
        winner: Some("AWAY".to_string()),
        match_details: Json(MatchDetails::Basketball {
            quarters: vec![(23, 33), (27, 32), (24, 33), (24, 38)],
            overtime: false,
            season_type: None,
            home_stats: None,
            away_stats: None,
        }),
        external_ids: Json(json!({ "balldontlie": 2001 })),
    };
    let inc_neutral_nba = sample_nba_ingested(
        "2025-01-25T00:00:00Z",
        "Indiana Pacers",
        "San Antonio Spurs",
        Some((136, 98)),
        json!({ "nba_game_id": "0022400633" }),
        MatchDetails::Basketball {
            quarters: vec![],
            overtime: false,
            season_type: Some("Regular Season".to_string()),
            home_stats: Some(Box::new(sample_stats(48.0, 133.3))), // IND
            away_stats: Some(Box::new(sample_stats(48.0, 96.1))),  // SAS
        },
    );
    assert_eq!(
        select_matching_row(
            std::slice::from_ref(&existing_neutral),
            &inc_neutral_nba,
            54,
            59
        ),
        Some(0)
    );
    // Różny wynik zakończonego meczu wyklucza scalenie mimo okna czasowego.
    let inc_conflicting_score = sample_nba_ingested(
        "2025-01-25T00:00:00Z",
        "Indiana Pacers",
        "San Antonio Spurs",
        Some((140, 110)),
        json!({ "nba_game_id": "0022400621" }),
        from_nba,
    );
    assert_eq!(
        select_matching_row(
            std::slice::from_ref(&existing_neutral),
            &inc_conflicting_score,
            54,
            59
        ),
        None
    );

    let merged_neutral = merge_existing_match(&existing_neutral, &inc_neutral_nba, 54, 59);
    assert_eq!(merged_neutral.match_date, dt("2025-01-25T17:00:00Z"));
    assert_eq!(merged_neutral.home_score, Some(98));
    assert_eq!(merged_neutral.away_score, Some(136));
    assert_eq!(merged_neutral.winner.as_deref(), Some("AWAY"));
    match merged_neutral.details {
        MatchDetails::Basketball {
            quarters,
            home_stats,
            away_stats,
            ..
        } => {
            assert_eq!(quarters.len(), 4);
            // SAS był gospodarzem w bazie -> dostał 96.1, IND (gość) dostał 133.3.
            assert_eq!(home_stats.unwrap().off_rating, Some(96.1));
            assert_eq!(away_stats.unwrap().off_rating, Some(133.3));
        }
        other => panic!("oczekiwano MatchDetails::Basketball, otrzymano {:?}", other),
    }

    // Gdy najpierw zapisano stats.nba.com, a potem wchodzi BALLDONTLIE ze zamienionymi stronami,
    // kwarty również zostają odwrócone do kierunku z bazy.
    let existing_neutral_nba = ExistingMatchRow {
        id: 5,
        match_date: dt("2025-01-25T00:00:00Z"),
        status: "FINISHED".to_string(),
        home_participant_id: 54, // IND
        away_participant_id: 59, // SAS
        home_score: Some(136),
        away_score: Some(98),
        winner: Some("HOME".to_string()),
        match_details: Json(inc_neutral_nba.details.clone()),
        external_ids: Json(json!({ "nba_game_id": "0022400633" })),
    };
    let inc_neutral_bdl = sample_nba_ingested(
        "2025-01-25T17:00:00Z",
        "San Antonio Spurs",
        "Indiana Pacers",
        Some((98, 136)),
        json!({ "balldontlie": 2001 }),
        MatchDetails::Basketball {
            quarters: vec![(23, 33), (27, 32), (24, 33), (24, 38)],
            overtime: false,
            season_type: None,
            home_stats: None,
            away_stats: None,
        },
    );
    let merged_flipped_quarters =
        merge_existing_match(&existing_neutral_nba, &inc_neutral_bdl, 59, 54);
    assert_eq!(merged_flipped_quarters.home_score, Some(136));
    assert_eq!(merged_flipped_quarters.away_score, Some(98));
    assert_eq!(merged_flipped_quarters.winner.as_deref(), Some("HOME"));
    match merged_flipped_quarters.details {
        MatchDetails::Basketball {
            quarters,
            home_stats,
            away_stats,
            ..
        } => {
            assert_eq!(quarters, vec![(33, 23), (32, 27), (33, 24), (38, 24)]);
            assert_eq!(home_stats.unwrap().off_rating, Some(133.3));
            assert_eq!(away_stats.unwrap().off_rating, Some(96.1));
        }
        other => panic!("oczekiwano MatchDetails::Basketball, otrzymano {:?}", other),
    }

    // Zakończony mecz nie cofa się do SCHEDULED, ale może zaktualizować dokładną godzinę UTC.
    let inc_stale_scheduled = sample_nba_ingested(
        "2024-11-05T01:45:00Z",
        "Houston Rockets",
        "New York Knicks",
        None,
        json!({ "balldontlie": 1001 }),
        MatchDetails::Basketball {
            quarters: vec![],
            overtime: false,
            season_type: None,
            home_stats: None,
            away_stats: None,
        },
    );
    let cand_day1_midnight = ExistingMatchRow {
        match_date: dt("2024-11-04T00:00:00Z"),
        ..cand_day1_bdl
    };
    let kept = merge_existing_match(&cand_day1_midnight, &inc_stale_scheduled, 10, 20);
    assert_eq!(kept.status, "FINISHED");
    assert_eq!(kept.home_score, Some(109));
    assert_eq!(kept.match_date, dt("2024-11-05T01:45:00Z"));
}

/// Konfiguracja `INGEST_NBA_SOURCE` oraz granice paginacji `GET /api/v1/matches`.
#[test]
fn parses_nba_ingest_source_and_pagination() {
    assert_eq!(NbaIngestSource::parse("auto"), NbaIngestSource::Auto);
    assert_eq!(
        NbaIngestSource::parse(" NBA_STATS "),
        NbaIngestSource::NbaStats
    );
    assert_eq!(
        NbaIngestSource::parse("nba-stats"),
        NbaIngestSource::NbaStats
    );
    assert_eq!(
        NbaIngestSource::parse("balldontlie"),
        NbaIngestSource::Balldontlie
    );
    assert_eq!(NbaIngestSource::parse("unknown"), NbaIngestSource::Auto);

    assert_eq!(normalize_pagination(None, None), (None, 0));
    assert_eq!(normalize_pagination(Some(0), Some(-5)), (Some(1), 0));
    assert_eq!(
        normalize_pagination(Some(10_000), Some(250)),
        (Some(MAX_MATCHES_LIMIT), 250)
    );
}

/// Konfiguracja opcjonalnego proxy `NBA_API_PROXY` nie panikuje przy błędnym adresie.
#[test]
fn builds_nba_stats_client_with_optional_proxy() {
    let fallback = reqwest::Client::new();
    let _direct = build_nba_stats_client(None, &fallback);
    let _proxied = build_nba_stats_client(Some("http://127.0.0.1:8080"), &fallback);
    let _invalid_fallback = build_nba_stats_client(Some("://bad proxy url"), &fallback);
}
