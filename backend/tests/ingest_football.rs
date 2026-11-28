// tests/ingest_football.rs
use std::collections::HashMap;

use backend::{
    config::FootballIngestSource,
    ingest::{
        football::{
            build_pre_match_elo, clubelo_slug, compute_league_calibration_offsets,
            football_data_uk_season_code, football_data_uk_season_url, league_spec,
            lookup_pre_match_elo_point, median, normalize_football_team_name,
            normalize_season_label, parse_clubelo_archive_csv, parse_clubelo_fixtures_html,
            parse_clubelo_home_rankings, parse_clubelo_team_page_history, parse_football_data,
            parse_football_data_for_league, parse_football_data_uk_csv,
            parse_football_data_uk_fixtures_csv, parse_football_season_year,
            resolve_historical_team_elo, ClubEloArchiveMap, ClubEloPoint, TOP5_LEAGUES,
        },
        merge_existing_match, merge_match_details, short_season_label, winner_from_scores,
        ExistingMatchRow, IngestedMatch, IngestedParticipant,
    },
    models::match_model::{FootballPreMatchElo, FootballTeamStats, MatchDetails},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{json, Value};
use sqlx::types::Json;

fn dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn sample_football_ingested(
    date: &str,
    status: &str,
    scores: Option<(i32, i32)>,
    external_ids: Value,
    details: MatchDetails,
) -> IngestedMatch {
    let winner = scores.and_then(|(h, a)| winner_from_scores(h, a, true).map(str::to_string));
    IngestedMatch {
        sport: "FOOTBALL",
        competition_code: "PL".to_string(),
        competition_name: "Premier League".to_string(),
        competition_country: Some("England".to_string()),
        season: "2026-2027".to_string(),
        match_date: dt(date),
        status: status.to_string(),
        home: IngestedParticipant::new("Arsenal FC", Some("ARS".to_string())),
        away: IngestedParticipant::new("Chelsea FC", Some("CHE".to_string())),
        home_score: scores.map(|(h, _)| h),
        away_score: scores.map(|(_, a)| a),
        winner,
        details,
        external_ids,
    }
}

/// Etykiety sezonów z różnych źródeł sprowadzamy do formatu `RRRR-RRRR`,
/// a `INGEST_FOOTBALL_SEASONS` obsługuje formaty `2023-24`, `2023-2024` i `2023`.
#[test]
fn normalizes_season_labels() {
    assert_eq!(normalize_season_label("2023-24"), "2023-2024");
    assert_eq!(normalize_season_label("2023/24"), "2023-2024");
    assert_eq!(normalize_season_label("2023-2024"), "2023-2024");
    assert_eq!(normalize_season_label("2026"), "2026");
    assert_eq!(short_season_label(2024), "2024-25");

    assert_eq!(parse_football_season_year("2023-24"), Some(2023));
    assert_eq!(parse_football_season_year("2023/24"), Some(2023));
    assert_eq!(parse_football_season_year("2023-2024"), Some(2023));
    assert_eq!(parse_football_season_year("2023"), Some(2023));
    assert_eq!(parse_football_season_year("bad"), None);

    assert_eq!(football_data_uk_season_code(2026), "2627");
    assert_eq!(
        football_data_uk_season_url("E0", 2026),
        "https://football-data.co.uk/mmz4281/2627/E0.csv"
    );
}

/// Remis jest dozwolony tylko w piłce nożnej.
#[test]
fn picks_winner_from_scores() {
    assert_eq!(winner_from_scores(2, 1, true), Some("HOME"));
    assert_eq!(winner_from_scores(0, 3, false), Some("AWAY"));
    assert_eq!(winner_from_scores(1, 1, true), Some("DRAW"));
    assert_eq!(winner_from_scores(90, 90, false), None);
}

/// Football-Data.org: wynik zapisujemy tylko dla zakończonych meczów, a mecze bez nazw drużyn pomijamy.
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
    assert_eq!(finished.competition_code, "PL");
    assert_eq!(finished.season, "2025-2026");
    assert_eq!(finished.status, "FINISHED");
    assert_eq!(finished.winner.as_deref(), Some("HOME"));
    assert_eq!(
        (finished.home_score, finished.away_score),
        (Some(4), Some(2))
    );
    assert_eq!(finished.home.name, "Liverpool FC");
    assert_eq!(finished.away.name, "AFC Bournemouth");
    assert_eq!(finished.home.short_code.as_deref(), Some("LIV"));
    assert_eq!(finished.external_ids["football_data"], 1);

    // Trwający mecz nie może wyglądać na rozstrzygnięty.
    let live = &matches[1];
    assert_eq!(live.status, "LIVE");
    assert_eq!((live.home_score, live.away_score), (None, None));
    assert_eq!(live.winner, None);

    // Parsowanie dla innej ligi Top 5 (np. La Liga / PD) ustawia jej kod i kraj.
    let pd = league_spec("PD").unwrap();
    let pd_matches = parse_football_data_for_league(json, pd).unwrap();
    assert_eq!(pd_matches[0].competition_code, "PD");
    assert_eq!(pd_matches[0].competition_country.as_deref(), Some("Spain"));
}

/// `football-data.co.uk`: parsowanie CSV sezonu (HxG/AxG, strzały, rożne, faule, kartki, HT, BOM)
/// oraz terminarza `fixtures.csv` dla wybranych lig.
#[test]
fn parses_football_data_uk_csv_and_fixtures() {
    let csv = include_str!("fixtures/football_data_uk_e0_sample.csv");
    let pl = league_spec("PL").unwrap();
    let matches = parse_football_data_uk_csv(csv, pl, 2026).unwrap();
    // Trzeci wiersz bez wyniku jest pomijany przez parser wyników.
    assert_eq!(matches.len(), 2);

    let first = &matches[0];
    assert_eq!(first.competition_code, "PL");
    assert_eq!(first.season, "2026-2027");
    assert_eq!(first.status, "FINISHED");
    assert_eq!(first.match_date, dt("2026-08-21T20:00:00Z"));
    assert_eq!(first.home.name, "Arsenal FC");
    assert_eq!(first.away.name, "Coventry City FC");
    assert_eq!((first.home_score, first.away_score), (Some(3), Some(0)));
    assert_eq!(first.winner.as_deref(), Some("HOME"));
    assert_eq!(
        first.external_ids["football_data_uk"],
        "E0:2026-08-21:Arsenal FC:Coventry City FC"
    );

    match &first.details {
        MatchDetails::Football {
            ht_home,
            ht_away,
            round,
            home_stats,
            away_stats,
            pre_match_elo,
        } => {
            assert_eq!((*ht_home, *ht_away), (Some(2), Some(0)));
            assert_eq!(round, "");
            assert_eq!(pre_match_elo, &None);

            let h = home_stats.as_ref().unwrap();
            assert_eq!(h.xg, Some(1.88));
            assert_eq!(h.shots, Some(20));
            assert_eq!(h.shots_on_target, Some(6));
            assert_eq!(h.corners, Some(8));
            assert_eq!(h.fouls, Some(10));
            assert_eq!(h.yellow_cards, Some(1));
            assert_eq!(h.red_cards, Some(0));

            let a = away_stats.as_ref().unwrap();
            assert_eq!(a.xg, Some(0.20));
            assert_eq!(a.shots, Some(4));
            assert_eq!(a.shots_on_target, Some(1));
        }
        other => panic!("Nieoczekiwany wariant MatchDetails: {:?}", other),
    }

    let draw = &matches[1];
    assert_eq!(draw.home.name, "Manchester United FC");
    assert_eq!(draw.away.name, "Tottenham Hotspur FC");
    assert_eq!(draw.winner.as_deref(), Some("DRAW"));

    // `fixtures.csv` filtruje po skonfigurowanych ligach i zwraca mecze SCHEDULED.
    let fixtures_csv = "\u{feff}Div,Date,Time,HomeTeam,AwayTeam,AvgH,AvgD,AvgA\n\
        E0,28/11/2026,15:00,Arsenal,Man City,2.10,3.40,3.50\n\
        B1,28/11/2026,18:00,Genk,Gent,1.90,3.50,3.90\n";
    let fixtures = parse_football_data_uk_fixtures_csv(fixtures_csv, &[pl], 2026).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(fixtures[0].status, "SCHEDULED");
    assert_eq!(fixtures[0].home.name, "Arsenal FC");
    assert_eq!(fixtures[0].away.name, "Manchester City FC");
    assert_eq!(
        (fixtures[0].home_score, fixtures[0].away_score),
        (None, None)
    );

    assert!(parse_football_data_uk_csv("", pl, 2026).is_err());
}

/// `clubelo.com`: parsowanie osadzonego JSON-a historii Elo, zasada `D - 1`, ranking strony głównej
/// oraz prawdopodobieństwa `1/X/2` ze strony `/Fixtures`.
#[test]
fn parses_clubelo_html_and_enforces_day_minus_one() {
    let team_html = include_str!("fixtures/clubelo_team_page_sample.html");
    let points = parse_clubelo_team_page_history(team_html).unwrap();
    assert_eq!(points.len(), 3);

    // Mecz 2026-08-17 -> D - 1 = 2026-08-16 -> musi wybrać 1982.50, NIE 1990.00 z 2026-08-17.
    assert_eq!(
        lookup_pre_match_elo_point(&points, dt("2026-08-17T14:00:00Z")),
        Some(1982.50)
    );
    // Mecz 2026-08-18 -> D - 1 = 2026-08-17 -> 1990.00.
    assert_eq!(
        lookup_pre_match_elo_point(&points, dt("2026-08-18T14:00:00Z")),
        Some(1990.00)
    );
    // Mecz wcześniejszy niż pierwszy punkt historii zwraca None.
    assert_eq!(
        lookup_pre_match_elo_point(&points, dt("2026-08-15T14:00:00Z")),
        None
    );

    let home_and_fixtures = include_str!("fixtures/clubelo_home_and_fixtures_sample.html");
    let rankings = parse_clubelo_home_rankings(home_and_fixtures).unwrap();
    assert_eq!(rankings.get("Arsenal FC"), Some(&(2045.0, Some(1))));
    assert_eq!(rankings.get("Brentford FC"), Some(&(1812.0, Some(22))));
    assert_eq!(rankings.get("Everton FC"), Some(&(1735.0, Some(29))));
    assert_eq!(
        rankings.get("Paris Saint-Germain FC"),
        Some(&(1988.0, Some(4)))
    );

    let fixtures = parse_clubelo_fixtures_html(home_and_fixtures).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0].date,
        NaiveDate::from_ymd_opt(2026, 11, 23).unwrap()
    );
    assert_eq!(fixtures[0].home, "Brentford FC");
    assert_eq!(fixtures[0].away, "Everton FC");
    assert_eq!(fixtures[0].home_rank, Some(22));
    assert_eq!(fixtures[0].away_rank, Some(29));
    assert!((fixtures[0].prob_home - 0.449).abs() < 1e-9);
    assert!((fixtures[0].prob_draw - 0.306).abs() < 1e-9);
    assert!((fixtures[0].prob_away - 0.246).abs() < 1e-9);

    let pre = build_pre_match_elo(
        rankings.get("Brentford FC").copied(),
        rankings.get("Everton FC").copied(),
        Some(&fixtures[0]),
    )
    .unwrap();
    assert_eq!(pre.home_elo, Some(1812.0));
    assert_eq!(pre.away_elo, Some(1735.0));
    assert_eq!(pre.elo_diff, Some(77.0));
    assert!((pre.clubelo_prob_home.unwrap() - 0.449).abs() < 1e-9);
}

/// Normalizacja nazw drużyn i lig Top 5 oraz konfiguracja `FootballIngestSource`.
#[test]
fn normalizes_top5_teams_leagues_and_source_config() {
    assert_eq!(
        normalize_football_team_name("Man City"),
        "Manchester City FC"
    );
    assert_eq!(
        normalize_football_team_name("Nott'm Forest"),
        "Nottingham Forest FC"
    );
    assert_eq!(normalize_football_team_name("Coventry"), "Coventry City FC");
    assert_eq!(clubelo_slug("Manchester City FC"), Some("ManCity"));
    assert_eq!(clubelo_slug("Wolverhampton Wanderers FC"), Some("Wolves"));

    // Pozostałe ligi Top 5 (`football-data.co.uk` + `clubelo.com`)
    assert_eq!(
        normalize_football_team_name("Ath Madrid"),
        "Club Atlético de Madrid"
    );
    assert_eq!(clubelo_slug("Club Atlético de Madrid"), Some("Atletico"));
    assert_eq!(
        normalize_football_team_name("Bayern Munich"),
        "FC Bayern München"
    );
    assert_eq!(clubelo_slug("FC Bayern München"), Some("Bayern"));
    assert_eq!(
        normalize_football_team_name("Ein Frankfurt"),
        "Eintracht Frankfurt"
    );
    assert_eq!(
        normalize_football_team_name("Inter"),
        "FC Internazionale Milano"
    );
    assert_eq!(
        normalize_football_team_name("Paris SG"),
        "Paris Saint-Germain FC"
    );
    assert_eq!(clubelo_slug("Paris Saint-Germain FC"), Some("ParisSG"));
    assert_eq!(
        normalize_football_team_name("Greuther Furth"),
        "SpVgg Greuther Fürth"
    );
    assert_eq!(normalize_football_team_name("Nurnberg"), "1. FC Nürnberg");
    assert_eq!(
        normalize_football_team_name("Sp Gijon"),
        "Real Sporting de Gijón"
    );
    assert_eq!(normalize_football_team_name("Nancy"), "AS Nancy-Lorraine");
    assert_eq!(clubelo_slug("AS Nancy-Lorraine"), Some("Nancy"));
    assert_eq!(
        normalize_football_team_name("RC Deportivo La Coruña"),
        "RC Deportivo de La Coruña"
    );
    assert_eq!(normalize_football_team_name("Lille OSC"), "LOSC Lille");
    assert_eq!(normalize_football_team_name("ES Troyes AC"), "ESTAC Troyes");
    assert_eq!(
        normalize_football_team_name("Racing Club de Lens"),
        "RC Lens"
    );

    assert_eq!(TOP5_LEAGUES.len(), 5);
    assert_eq!(league_spec("PL").unwrap().football_data_uk, "E0");
    assert_eq!(league_spec("SP1").unwrap().code, "PD");
    assert_eq!(league_spec("bl1").unwrap().clubelo_country, "GER");
    assert_eq!(league_spec("SA").unwrap().football_data_uk, "I1");
    assert_eq!(league_spec("FL1").unwrap().clubelo_country, "FRA");

    assert_eq!(
        FootballIngestSource::parse("auto"),
        FootballIngestSource::Auto
    );
    assert_eq!(
        FootballIngestSource::parse("football_data_uk"),
        FootballIngestSource::FootballDataUk
    );
    assert_eq!(
        FootballIngestSource::parse("csv"),
        FootballIngestSource::FootballDataUk
    );
    assert_eq!(
        FootballIngestSource::parse("football-data"),
        FootballIngestSource::FootballData
    );
    assert_eq!(
        FootballIngestSource::parse("unknown"),
        FootballIngestSource::Auto
    );
}

/// Zgodność wsteczna JSONB `MatchDetails::Football`: stare rekordy bez nowych pól
/// deserializują się poprawnie, a pola `None` są pomijane przy serializacji.
#[test]
fn match_details_football_serde_compat() {
    let legacy = r#"{"sport_type":"football","ht_home":1,"ht_away":1,"round":"Matchday 10"}"#;
    let parsed: MatchDetails = serde_json::from_str(legacy).unwrap();
    assert_eq!(
        parsed,
        MatchDetails::Football {
            ht_home: Some(1),
            ht_away: Some(1),
            round: "Matchday 10".to_string(),
            home_stats: None,
            away_stats: None,
            pre_match_elo: None,
        }
    );

    let serialized = serde_json::to_value(&parsed).unwrap();
    assert!(serialized.get("home_stats").is_none());
    assert!(serialized.get("away_stats").is_none());
    assert!(serialized.get("pre_match_elo").is_none());
}

/// Scalanie danych piłkarskich między `football-data.co.uk`, Football-Data.org i ClubElo w obu kolejnościach.
#[test]
fn merges_football_sources_symmetrically() {
    let from_fduk = MatchDetails::Football {
        ht_home: Some(1),
        ht_away: Some(0),
        round: String::new(),
        home_stats: Some(Box::new(FootballTeamStats {
            xg: Some(1.85),
            shots: Some(16),
            shots_on_target: Some(6),
            corners: Some(7),
            ..Default::default()
        })),
        away_stats: Some(Box::new(FootballTeamStats {
            xg: Some(0.75),
            shots: Some(8),
            ..Default::default()
        })),
        pre_match_elo: None,
    };
    let from_football_data = MatchDetails::Football {
        ht_home: Some(1),
        ht_away: Some(0),
        round: "Matchday 12".to_string(),
        home_stats: None,
        away_stats: None,
        pre_match_elo: None,
    };
    let from_clubelo = MatchDetails::Football {
        ht_home: None,
        ht_away: None,
        round: String::new(),
        home_stats: None,
        away_stats: None,
        pre_match_elo: Some(Box::new(FootballPreMatchElo {
            home_elo: Some(1950.0),
            away_elo: Some(1840.0),
            elo_diff: Some(110.0),
            home_rank: Some(4),
            away_rank: Some(11),
            ..Default::default()
        })),
    };

    let order_1 = merge_match_details(
        merge_match_details(from_fduk.clone(), from_football_data.clone()),
        from_clubelo.clone(),
    );
    let order_2 = merge_match_details(
        merge_match_details(from_football_data.clone(), from_clubelo.clone()),
        from_fduk.clone(),
    );
    assert_eq!(order_1, order_2);
    match &order_1 {
        MatchDetails::Football {
            ht_home,
            ht_away,
            round,
            home_stats,
            pre_match_elo,
            ..
        } => {
            assert_eq!((*ht_home, *ht_away), (Some(1), Some(0)));
            assert_eq!(round, "Matchday 12");
            assert_eq!(home_stats.as_ref().unwrap().xg, Some(1.85));
            assert_eq!(home_stats.as_ref().unwrap().shots, Some(16));
            assert_eq!(pre_match_elo.as_ref().unwrap().elo_diff, Some(110.0));
        }
        other => panic!("Nieoczekiwany wariant MatchDetails: {:?}", other),
    }

    // Zakończony mecz w DB nie cofa się do SCHEDULED, zachowuje `utcDate` z Football-Data,
    // ale przyjmuje wzbogacenie z ClubElo.
    let existing_finished_fd = ExistingMatchRow {
        id: 1,
        match_date: dt("2026-11-10T16:30:00Z"),
        status: "FINISHED".to_string(),
        home_participant_id: 1,
        away_participant_id: 2,
        home_score: Some(2),
        away_score: Some(1),
        winner: Some("HOME".to_string()),
        match_details: Json(from_football_data),
        external_ids: Json(json!({ "football_data": 500 })),
    };
    let incoming_fduk_stale = sample_football_ingested(
        "2026-11-10T15:00:00Z",
        "SCHEDULED",
        None,
        json!({ "football_data_uk": "E0:2026-11-10:Arsenal FC:Chelsea FC" }),
        from_clubelo,
    );
    let merged = merge_existing_match(&existing_finished_fd, &incoming_fduk_stale, 1, 2);
    assert_eq!(merged.status, "FINISHED");
    assert_eq!((merged.home_score, merged.away_score), (Some(2), Some(1)));
    assert_eq!(merged.match_date, dt("2026-11-10T16:30:00Z"));
    if let MatchDetails::Football {
        ht_home,
        round,
        pre_match_elo,
        ..
    } = merged.details
    {
        assert_eq!(ht_home, Some(1));
        assert_eq!(round, "Matchday 12");
        assert_eq!(pre_match_elo.unwrap().home_elo, Some(1950.0));
    } else {
        panic!("Oczekiwano MatchDetails::Football");
    }
}

/// Archiwum `EloRatings.csv`: filtracja lig Top 5, twarde odcięcie `[2015-01-01, 2025-06-01]`,
/// odrzucenie niepoprawnych liczb oraz sortowanie rosnące po dacie.
#[test]
fn parses_clubelo_archive_csv_and_filters_leagues_and_dates() {
    let sample_csv = include_str!("fixtures/clubelo_archive_sample.csv");
    let archive = parse_clubelo_archive_csv(sample_csv).unwrap();

    // Wiersz spoza Top 5 (`POR`) oraz `NaN`/puste wartości nie mogą trafić do mapy.
    assert!(!archive.contains_key(&("POR".to_string(), "Porto".to_string())));
    assert!(!archive.contains_key(&("ENG".to_string(), "Arsenal FC".to_string())));

    // Wiersze sprzed 2015-01-01 i po 2025-06-01 (provisional) są odcięte, a granice są włączne.
    let man_utd = archive
        .get(&("ENG".to_string(), "Manchester United FC".to_string()))
        .unwrap();
    let expected_dates = [
        NaiveDate::from_ymd_opt(2015, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2017, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2018, 9, 15).unwrap(),
        NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
    ];
    let actual_dates: Vec<NaiveDate> = man_utd.iter().map(|p| p.date).collect();
    assert_eq!(actual_dates, expected_dates);
    assert_eq!(man_utd[0].elo, 1870.50);
    assert_eq!(man_utd[3].elo, 1835.40);

    // Wszystkie 5 lig i znormalizowane aliasy są poprawnie indeksowane po `(country_code, canonical)`.
    assert!(archive.contains_key(&("GER".to_string(), "FC Bayern München".to_string())));
    assert!(archive.contains_key(&("GER".to_string(), "SpVgg Greuther Fürth".to_string())));
    assert!(archive.contains_key(&("GER".to_string(), "1. FC Nürnberg".to_string())));
    assert!(archive.contains_key(&("ESP".to_string(), "Real Sociedad de Fútbol".to_string())));
    assert!(archive.contains_key(&("ESP".to_string(), "Real Sporting de Gijón".to_string())));
    assert!(archive.contains_key(&("ITA".to_string(), "FC Internazionale Milano".to_string())));
    assert!(archive.contains_key(&("FRA".to_string(), "AC Ajaccio".to_string())));
    assert!(archive.contains_key(&("FRA".to_string(), "AS Nancy-Lorraine".to_string())));
    assert!(archive.contains_key(&("FRA".to_string(), "Paris Saint-Germain FC".to_string())));

    // Pusty CSV lub brak wymaganej kolumny zwraca błąd.
    assert!(parse_clubelo_archive_csv("").is_err());
    assert!(parse_clubelo_archive_csv("date,club,country\n2020-01-01,Arsenal,ENG\n").is_err());
}

/// Archiwum `EloRatings.csv`: warunek `D - 1` nie dopuszcza wycieku danych z dnia meczu.
#[test]
fn enforces_day_minus_one_for_clubelo_archive_points() {
    let sample_csv = include_str!("fixtures/clubelo_archive_sample.csv");
    let archive = parse_clubelo_archive_csv(sample_csv).unwrap();
    let man_utd = archive
        .get(&("ENG".to_string(), "Manchester United FC".to_string()))
        .unwrap();

    // W dniu snapshotu `2017-03-01` mecz musi wziąć poprzedni punkt (`2015-01-01` -> 1870.50).
    assert_eq!(
        lookup_pre_match_elo_point(man_utd, dt("2017-03-01T15:00:00Z")),
        Some(1870.50)
    );
    // Dzień po snapshocie (`2017-03-02`) korzysta już z `2017-03-01` (1881.00).
    assert_eq!(
        lookup_pre_match_elo_point(man_utd, dt("2017-03-02T15:00:00Z")),
        Some(1881.00)
    );
    // Przed pierwszym snapshotem zwraca `None`.
    assert_eq!(
        lookup_pre_match_elo_point(man_utd, dt("2015-01-01T12:00:00Z")),
        None
    );
}

/// Kalibracja offsetu per liga: mediana różnic dla `>= 20` próbek (ignorując punkty spoza okna
/// i dalej niż ±3 dni) oraz bezpieczny fallback `0.0` przy `< 20` próbkach i wsparcie próbek SQL.
#[test]
fn computes_league_calibration_offsets_median_and_fallback() {
    assert_eq!(median(&[]), None);
    assert_eq!(median(&[f64::NAN]), None);
    assert_eq!(median(&[10.0, 30.0, 20.0]), Some(20.0));
    assert_eq!(median(&[10.0, 40.0, 20.0, 30.0]), Some(25.0));

    let base_date = NaiveDate::from_ymd_opt(2023, 2, 1).unwrap();
    let mut archive: ClubEloArchiveMap = HashMap::new();
    let mut club_histories: HashMap<String, Vec<ClubEloPoint>> = HashMap::new();

    // GER: 20 próbek w oknie `[2023-01-01, 2025-06-01]` (różnica ±2 dni, mediana = +45.0)
    // oraz 2 punkty odrzucone (spoza okna w 2022 oraz odległość 5 dni > 3 dni).
    let mut ger_arch = Vec::new();
    let mut ger_live = Vec::new();
    for i in 0..20_i64 {
        let arch_date = base_date + chrono::Duration::days(i * 15);
        let live_date = arch_date + chrono::Duration::days(if i % 2 == 0 { 1 } else { -2 });
        let diff = if i == 19 { 500.0 } else { 45.0 };
        ger_arch.push(ClubEloPoint {
            date: arch_date,
            elo: 1800.0,
        });
        ger_live.push(ClubEloPoint {
            date: live_date,
            elo: 1800.0 + diff,
        });
    }
    // Punkt spoza okna kalibracyjnego (2022) i punkt oddalony o 5 dni nie powinny zaburzyć wyniku.
    ger_arch.push(ClubEloPoint {
        date: NaiveDate::from_ymd_opt(2022, 6, 1).unwrap(),
        elo: 1700.0,
    });
    ger_live.push(ClubEloPoint {
        date: NaiveDate::from_ymd_opt(2022, 6, 1).unwrap(),
        elo: 2500.0,
    });
    let far_date = NaiveDate::from_ymd_opt(2024, 12, 1).unwrap();
    ger_arch.push(ClubEloPoint {
        date: far_date,
        elo: 1800.0,
    });
    ger_live.push(ClubEloPoint {
        date: far_date + chrono::Duration::days(5),
        elo: 2600.0,
    });

    archive.insert(
        ("GER".to_string(), "FC Bayern München".to_string()),
        ger_arch,
    );
    club_histories.insert("FC Bayern München".to_string(), ger_live);

    // FRA: tylko 5 próbek live (< 20 -> bez SQL fallbacku dałoby 0.0, ale z fallbackiem >= 20 daje medianę).
    let mut fra_arch = Vec::new();
    let mut fra_live = Vec::new();
    for i in 0..5_i64 {
        let date = base_date + chrono::Duration::days(i * 15);
        fra_arch.push(ClubEloPoint { date, elo: 1500.0 });
        fra_live.push(ClubEloPoint { date, elo: 1580.0 });
    }
    archive.insert(("FRA".to_string(), "AC Ajaccio".to_string()), fra_arch);
    club_histories.insert("AC Ajaccio".to_string(), fra_live);

    // Bez próbek SQL: GER = 45.0, FRA/ENG/ESP/ITA = 0.0.
    let offsets_no_sql =
        compute_league_calibration_offsets(&club_histories, &archive, &HashMap::new());
    assert_eq!(offsets_no_sql.get("GER"), Some(&45.0));
    assert_eq!(offsets_no_sql.get("FRA"), Some(&0.0));
    assert_eq!(offsets_no_sql.get("ENG"), Some(&0.0));

    // Z 15 dodatkowymi próbkami SQL dla FRA (łącznie 5 + 15 = 20): mediana wynosi +80.0.
    let mut fallback_samples = HashMap::new();
    fallback_samples.insert("FRA".to_string(), vec![80.0; 15]);
    let offsets_with_sql =
        compute_league_calibration_offsets(&club_histories, &archive, &fallback_samples);
    assert_eq!(offsets_with_sql.get("GER"), Some(&45.0));
    assert_eq!(offsets_with_sql.get("FRA"), Some(&80.0));
}

/// Priorytet źródeł dla meczów historycznych: live `club_histories` wygrywa z archiwum (zachowując rank),
/// a fallback archiwalny dodaje offset ligowy i wymusza `rank = None` (bez wycieku bieżącego rankingu).
#[test]
fn prioritizes_live_club_histories_over_archive_with_no_rank_leak() {
    let mut club_histories: HashMap<String, Vec<ClubEloPoint>> = HashMap::new();
    club_histories.insert(
        "Arsenal FC".to_string(),
        vec![ClubEloPoint {
            date: NaiveDate::from_ymd_opt(2023, 3, 1).unwrap(),
            elo: 1995.0,
        }],
    );

    let mut current_rankings: HashMap<String, (f64, Option<i32>)> = HashMap::new();
    current_rankings.insert("Arsenal FC".to_string(), (2050.0, Some(1)));
    current_rankings.insert("AC Ajaccio".to_string(), (1550.0, Some(99)));

    let mut archive: ClubEloArchiveMap = HashMap::new();
    archive.insert(
        ("ENG".to_string(), "Arsenal FC".to_string()),
        vec![ClubEloPoint {
            date: NaiveDate::from_ymd_opt(2023, 3, 1).unwrap(),
            elo: 1900.0,
        }],
    );
    archive.insert(
        ("FRA".to_string(), "AC Ajaccio".to_string()),
        vec![ClubEloPoint {
            date: NaiveDate::from_ymd_opt(2022, 9, 1).unwrap(),
            elo: 1498.40,
        }],
    );

    let mut offsets = HashMap::new();
    offsets.insert("ENG".to_string(), 10.0);
    offsets.insert("FRA".to_string(), 75.0);

    // 1. Gdy live ma punkt `D - 1`, wygrywa nad archiwum i zachowuje rank z `current_rankings`.
    assert_eq!(
        resolve_historical_team_elo(
            "Arsenal FC",
            Some("ENG"),
            dt("2023-03-02T15:00:00Z"),
            &club_histories,
            &current_rankings,
            Some((&archive, &offsets)),
        ),
        Some((1995.0, Some(1)))
    );

    // 2. Gdy live nie ma pokrycia w dacie meczu, fallback bierze archiwum + offset i `rank = None`.
    assert_eq!(
        resolve_historical_team_elo(
            "Arsenal FC",
            Some("ENG"),
            dt("2023-03-02T15:00:00Z"),
            &HashMap::new(),
            &current_rankings,
            Some((&archive, &offsets)),
        ),
        Some((1910.0, None))
    );
    assert_eq!(
        resolve_historical_team_elo(
            "AC Ajaccio",
            Some("FRA"),
            dt("2022-09-02T19:00:00Z"),
            &club_histories,
            &current_rankings,
            Some((&archive, &offsets)),
        ),
        Some((1573.40, None))
    );

    // 3. Bez przekazanego archiwum (np. ochrona istniejącego wpisu live) nie zwraca fallbacku.
    assert_eq!(
        resolve_historical_team_elo(
            "AC Ajaccio",
            Some("FRA"),
            dt("2022-09-02T19:00:00Z"),
            &club_histories,
            &current_rankings,
            None,
        ),
        None
    );
}
