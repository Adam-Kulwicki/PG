// tests/ingest_cs2.rs
use std::io::Write;

use backend::{
    ingest::{
        choose_match_date,
        cs2::{
            build_pandascore_past_range, extract_csv_bytes, infer_tier_from_filename,
            normalize_cs2_team_name, parse_kaggle_cs2_csv, parse_kaggle_dataset_version,
            parse_pandascore_matches, parse_pandascore_tournament_rosters, KAGGLE_CS2_FILE,
        },
        merge_existing_match, merge_match_details, select_matching_row, ExistingMatchRow,
        IngestedMatch, IngestedParticipant,
    },
    models::match_model::{Cs2MapResult, Cs2Player, Cs2PlayerStats, Cs2VetoAction, MatchDetails},
};
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::types::Json;
use zip::{write::SimpleFileOptions, ZipWriter};

fn dt(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .unwrap()
        .with_timezone(&Utc)
}

/// Parsowanie Kaggle CSV: grupowanie po `match_id`, rozdzielenie `is_total`, składy, statystyki graczy i aliasy drużyn.
#[test]
fn parses_kaggle_cs2_csv_fixture() {
    let csv_content = include_str!("fixtures/cs2_kaggle_sample.csv");
    let tier = infer_tier_from_filename(KAGGLE_CS2_FILE);
    assert_eq!(tier.as_deref(), Some("tier1"));

    let matches = parse_kaggle_cs2_csv(csv_content, tier.as_deref()).unwrap();
    assert_eq!(matches.len(), 2);

    // 1. IEM Cologne 2025 (Bo1 bez osobnego wiersza is_total=True, spada do jedynego wiersza mapy)
    let bo1 = &matches[0];
    assert_eq!(bo1.sport, "CS2");
    assert_eq!(bo1.season, "2025");
    assert_eq!(bo1.status, "FINISHED");
    assert_eq!(bo1.home.name, "Team Vitality");
    assert_eq!(bo1.away.name, "Virtus.pro");
    assert_eq!((bo1.home_score, bo1.away_score), (Some(1), Some(0)));
    assert_eq!(bo1.winner.as_deref(), Some("HOME"));
    assert_eq!(bo1.external_ids["cs2_kaggle"], 1002);

    // 2. ESL Pro League Season 20 (Bo3: 2 mapy + wiersz podsumowania)
    let bo3 = &matches[1];
    assert_eq!(bo3.home.name, "Natus Vincere");
    assert_eq!(bo3.away.name, "FaZe Clan");
    assert_eq!(bo3.home.metadata["cs2_kaggle_id"], 10);
    assert_eq!(bo3.away.metadata["cs2_kaggle_id"], 20);
    assert_eq!((bo3.home_score, bo3.away_score), (Some(2), Some(0)));
    assert_eq!(bo3.winner.as_deref(), Some("HOME"));
    assert_eq!(bo3.external_ids["cs2_kaggle"], 1001);

    match &bo3.details {
        MatchDetails::Cs2 {
            best_of,
            tournament_name,
            tournament_tier,
            maps,
            home_roster,
            away_roster,
            player_stats,
            veto,
        } => {
            assert_eq!(*best_of, Some(3));
            assert_eq!(tournament_name.as_deref(), Some("ESL Pro League Season 20"));
            assert_eq!(tournament_tier.as_deref(), Some("tier1"));
            assert_eq!(maps.len(), 2);
            assert_eq!(maps[0].map_name, "Mirage");
            assert_eq!(
                (maps[0].home_score, maps[0].away_score),
                (Some(13), Some(9))
            );
            assert_eq!(maps[0].winner.as_deref(), Some("HOME"));
            assert_eq!(maps[1].map_name, "Ancient");
            assert_eq!(
                (maps[1].home_score, maps[1].away_score),
                (Some(13), Some(11))
            );
            assert_eq!(home_roster.len(), 5);
            assert_eq!(away_roster.len(), 5);
            assert_eq!(home_roster[0].name, "b1t");
            assert_eq!(home_roster[0].external_id, Some(101));
            assert_eq!(player_stats.len(), 2);
            assert_eq!(player_stats[0].team_side, "HOME");
            assert_eq!(player_stats[0].adr, Some(89.4));
            assert_eq!(player_stats[1].team_side, "AWAY");
            assert_eq!(player_stats[1].kddiff, Some(-8));
            assert!(veto.is_empty());
        }
        other => panic!("oczekiwano MatchDetails::Cs2, otrzymano {other:?}"),
    }
}

/// Kontrola wersji Kaggle oraz rozpakowanie surowego CSV i archiwum ZIP z nagłówkiem `PK`.
#[test]
fn parses_kaggle_version_and_extracts_zip_or_raw_csv() {
    let version = parse_kaggle_dataset_version(r#"{"currentVersionNumber": 42}"#).unwrap();
    assert_eq!(version, 42);
    assert!(parse_kaggle_dataset_version(r#"{"currentVersionNumber": 0}"#).is_err());

    let raw_csv = b"match_id,datetime,team1,team2\n1,2025-01-01,NAVI,FaZe\n";
    assert_eq!(
        extract_csv_bytes(raw_csv, KAGGLE_CS2_FILE).unwrap(),
        raw_csv
    );

    let mut buffer = Vec::new();
    {
        let mut zip = ZipWriter::new(std::io::Cursor::new(&mut buffer));
        zip.start_file("other.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"ignore").unwrap();
        zip.start_file(KAGGLE_CS2_FILE, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(raw_csv).unwrap();
        zip.finish().unwrap();
    }

    let extracted = extract_csv_bytes(&buffer, KAGGLE_CS2_FILE).unwrap();
    assert_eq!(extracted, raw_csv);
    assert!(extract_csv_bytes(&[], KAGGLE_CS2_FILE).is_err());
}

/// PandaScore: składy turniejowe, mecze `SCHEDULED`, `FINISHED`, `DRAW` i walkower bez zwycięzcy (`CANCELLED`).
#[test]
fn parses_pandascore_matches_and_rosters() {
    let rosters_json = r#"{
        "rosters": [
            {
                "id": 501,
                "players": [
                    {"id": 101, "name": "b1t"},
                    {"id": 102, "name": "Aleksib"}
                ]
            },
            {
                "id": 502,
                "players": [
                    {"id": 201, "name": "karrigan"}
                ]
            }
        ]
    }"#;
    let rosters = parse_pandascore_tournament_rosters(rosters_json).unwrap();
    assert_eq!(rosters.get(&501).unwrap().len(), 2);

    let matches_json = r#"[
        {
            "id": 9001,
            "status": "finished",
            "forfeit": false,
            "draw": false,
            "number_of_games": 3,
            "begin_at": "2025-09-15T17:35:00Z",
            "scheduled_at": "2025-09-15T17:30:00Z",
            "winner_id": 501,
            "opponents": [
                {"opponent": {"id": 501, "name": "Natus Vincere", "acronym": "NAVI"}},
                {"opponent": {"id": 502, "name": "FaZe Clan", "acronym": "FaZe"}}
            ],
            "results": [
                {"team_id": 501, "score": 2},
                {"team_id": 502, "score": 0}
            ],
            "games": [
                {"position": 2, "map": {"name": "Ancient", "slug": "de_ancient"}, "winner": {"id": 501}},
                {"position": 1, "map": {"name": "Mirage", "slug": "de_mirage"}, "winner": {"id": 501}}
            ],
            "league": {"name": "ESL Pro League"},
            "serie": {"full_name": "Season 20"},
            "tournament": {"id": 77, "name": "Playoffs", "tier": "s"}
        },
        {
            "id": 9002,
            "status": "not_started",
            "forfeit": false,
            "draw": false,
            "number_of_games": 3,
            "scheduled_at": "2026-12-01T18:00:00Z",
            "opponents": [
                {"opponent": {"id": 501, "name": "NAVI", "acronym": "NAVI"}},
                {"opponent": {"id": 503, "name": "G2", "acronym": "G2"}}
            ],
            "results": [],
            "games": []
        },
        {
            "id": 9003,
            "status": "finished",
            "forfeit": true,
            "draw": false,
            "begin_at": "2025-10-01T12:00:00Z",
            "opponents": [
                {"opponent": {"id": 501, "name": "NAVI", "acronym": "NAVI"}},
                {"opponent": {"id": 502, "name": "FaZe Clan", "acronym": "FaZe"}}
            ],
            "results": []
        },
        {
            "id": 9004,
            "status": "not_started",
            "begin_at": "2026-12-02T12:00:00Z",
            "opponents": [{"opponent": {"id": 501, "name": "NAVI"}}]
        }
    ]"#;

    let parsed = parse_pandascore_matches(matches_json, &rosters).unwrap();
    // Rekord 9004 z nieobsadzonym przeciwnikiem (TBD) jest pomijany.
    assert_eq!(parsed.len(), 3);

    let finished = &parsed[0];
    assert_eq!(finished.status, "FINISHED");
    assert_eq!(finished.winner.as_deref(), Some("HOME"));
    assert_eq!(
        (finished.home_score, finished.away_score),
        (Some(2), Some(0))
    );
    assert_eq!(finished.external_ids["pandascore"], 9001);
    match &finished.details {
        MatchDetails::Cs2 {
            tournament_name,
            tournament_tier,
            maps,
            home_roster,
            away_roster,
            ..
        } => {
            assert_eq!(
                tournament_name.as_deref(),
                Some("ESL Pro League — Season 20 — Playoffs")
            );
            assert_eq!(tournament_tier.as_deref(), Some("s"));
            // Kolejność map posortowana po `position` (1 = Mirage, 2 = Ancient).
            assert_eq!(maps[0].map_name, "Mirage");
            assert_eq!(maps[1].map_name, "Ancient");
            assert_eq!(home_roster.len(), 2);
            assert_eq!(away_roster.len(), 1);
        }
        other => panic!("oczekiwano MatchDetails::Cs2, otrzymano {other:?}"),
    }

    let scheduled = &parsed[1];
    assert_eq!(scheduled.status, "SCHEDULED");
    assert_eq!(scheduled.home.name, "Natus Vincere");
    assert_eq!(scheduled.away.name, "G2 Esports");
    assert_eq!(scheduled.winner, None);

    let void_forfeit = &parsed[2];
    assert_eq!(void_forfeit.status, "CANCELLED");
    assert_eq!(void_forfeit.winner, None);
}

/// Bezstratne scalanie PandaScore + Kaggle CSV z odwróconą kolejnością drużyn i normalizacją aliasów.
#[test]
fn deduplicates_and_enriches_pandascore_with_kaggle_csv() {
    assert_eq!(normalize_cs2_team_name("NAVI"), "Natus Vincere");
    assert_eq!(normalize_cs2_team_name("vp"), "Virtus.pro");

    // 1. Najpierw PandaScore zapisał mecz jako FaZe (HOME) vs NAVI (AWAY), wynik 0:2 (bez rund i statystyk).
    let existing_pandascore = ExistingMatchRow {
        id: 10,
        match_date: dt("2025-09-15T17:35:00Z"),
        status: "FINISHED".to_string(),
        home_participant_id: 20, // FaZe
        away_participant_id: 10, // NAVI
        home_score: Some(0),
        away_score: Some(2),
        winner: Some("AWAY".to_string()),
        match_details: Json(MatchDetails::Cs2 {
            best_of: Some(3),
            tournament_name: Some("ESL Pro League — Season 20".to_string()),
            tournament_tier: Some("s".to_string()),
            maps: vec![Cs2MapResult {
                map_name: "Mirage".to_string(),
                home_score: None,
                away_score: None,
                winner: Some("AWAY".to_string()),
            }],
            home_roster: vec![Cs2Player {
                external_id: Some(201),
                name: "karrigan".to_string(),
            }],
            away_roster: vec![],
            player_stats: vec![],
            veto: vec![],
        }),
        external_ids: Json(json!({ "pandascore": 9001 })),
    };

    // 2. Później Kaggle CSV wchodzi z odwróconą kolejnością: NAVI (HOME=10) vs FaZe (AWAY=20), wynik 2:0.
    let incoming_kaggle = IngestedMatch {
        sport: "CS2",
        competition_code: "CS2".to_string(),
        competition_name: "Counter-Strike 2".to_string(),
        competition_country: Some("International".to_string()),
        season: "2025".to_string(),
        match_date: dt("2025-09-15T17:30:00Z"),
        status: "FINISHED".to_string(),
        home: IngestedParticipant::new("Natus Vincere", None),
        away: IngestedParticipant::new("FaZe Clan", None),
        home_score: Some(2),
        away_score: Some(0),
        winner: Some("HOME".to_string()),
        details: MatchDetails::Cs2 {
            best_of: Some(3),
            tournament_name: Some("ESL Pro League Season 20".to_string()),
            tournament_tier: Some("tier1".to_string()),
            maps: vec![Cs2MapResult {
                map_name: "Mirage".to_string(),
                home_score: Some(13),
                away_score: Some(9),
                winner: Some("HOME".to_string()),
            }],
            home_roster: vec![Cs2Player {
                external_id: Some(101),
                name: "b1t".to_string(),
            }],
            away_roster: vec![Cs2Player {
                external_id: Some(201),
                name: "karrigan".to_string(),
            }],
            player_stats: vec![Cs2PlayerStats {
                team_side: "HOME".to_string(),
                player_id: Some(101),
                name: "b1t".to_string(),
                kills: Some(20),
                deaths: Some(12),
                assists: Some(4),
                adr: Some(95.0),
                kast: Some(80.0),
                kddiff: Some(8),
            }],
            veto: vec![Cs2VetoAction {
                order: Some(1),
                team_side: "HOME".to_string(),
                action: "ban".to_string(),
                map_name: "Vertigo".to_string(),
            }],
        },
        external_ids: json!({ "cs2_kaggle": 1001 }),
    };

    assert_eq!(
        select_matching_row(
            std::slice::from_ref(&existing_pandascore),
            &incoming_kaggle,
            10,
            20
        ),
        Some(0)
    );

    let merged = merge_existing_match(&existing_pandascore, &incoming_kaggle, 10, 20);
    assert_eq!((merged.home_score, merged.away_score), (Some(0), Some(2)));
    assert_eq!(merged.winner.as_deref(), Some("AWAY"));
    assert_eq!(
        choose_match_date(dt("2025-09-15T17:35:00Z"), dt("2025-09-15T00:00:00Z")),
        dt("2025-09-15T17:35:00Z")
    );

    match merged.details {
        MatchDetails::Cs2 {
            maps,
            home_roster,
            away_roster,
            player_stats,
            veto,
            ..
        } => {
            // Po odwróceniu stron do kierunku z bazy (FaZe=HOME, NAVI=AWAY):
            assert_eq!(
                (maps[0].home_score, maps[0].away_score),
                (Some(9), Some(13))
            );
            assert_eq!(maps[0].winner.as_deref(), Some("AWAY"));
            assert_eq!(home_roster[0].name, "karrigan");
            assert_eq!(away_roster[0].name, "b1t");
            assert_eq!(player_stats[0].team_side, "AWAY");
            assert_eq!(veto[0].team_side, "AWAY");
        }
        other => panic!("oczekiwano MatchDetails::Cs2, otrzymano {other:?}"),
    }

    // Gdy najpierw wchodzi Kaggle CSV, a potem ponownie PandaScore, statystyki graczy i rundy zostają zachowane.
    let reverse = merge_match_details(
        incoming_kaggle.details.clone(),
        existing_pandascore.match_details.0,
    );
    match reverse {
        MatchDetails::Cs2 {
            maps, player_stats, ..
        } => {
            assert_eq!(
                (maps[0].home_score, maps[0].away_score),
                (Some(13), Some(9))
            );
            assert_eq!(player_stats.len(), 1);
        }
        other => panic!("oczekiwano MatchDetails::Cs2, otrzymano {other:?}"),
    }
}

/// Parametr `range[begin_at]` dla PandaScore używa sufiksu `Z` bez `+00:00` i bez nanosekund.
#[test]
fn formats_pandascore_past_range_as_clean_utc_seconds() {
    let sample = dt("2026-11-25T23:31:34.403906165Z");
    assert_eq!(
        build_pandascore_past_range(sample),
        "range[begin_at]=2026-11-11T23:31:34Z,2026-11-25T23:31:34Z"
    );
}

/// Zgodność serializacji JSONB `MatchDetails::Cs2` (opcjonalne pola pomijane, gdy puste).
#[test]
fn match_details_cs2_serde_compat() {
    let minimal_json = r#"{"sport_type":"cs2"}"#;
    let parsed: MatchDetails = serde_json::from_str(minimal_json).unwrap();
    assert_eq!(
        parsed,
        MatchDetails::Cs2 {
            best_of: None,
            tournament_name: None,
            tournament_tier: None,
            maps: vec![],
            home_roster: vec![],
            away_roster: vec![],
            player_stats: vec![],
            veto: vec![],
        }
    );

    let serialized = serde_json::to_value(&parsed).unwrap();
    assert_eq!(serialized, serde_json::json!({ "sport_type": "cs2" }));
}
