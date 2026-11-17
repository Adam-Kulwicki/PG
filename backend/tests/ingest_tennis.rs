// tests/ingest_tennis.rs
use backend::{
    ingest::tennis::{
        parse_espn_scoreboard, parse_sackmann_csv, parse_sets, round_day_offset, split_csv_line,
        winner_is_home,
    },
    models::match_model::MatchDetails,
};

/// Nagłówek zawierający kolumny używane przez parser (pozostałe kolumny Sackmanna pomijamy).
const CSV_HEADER: &str = "tourney_id,tourney_name,surface,draw_size,tourney_date,match_num,winner_id,winner_hand,winner_name,winner_ioc,loser_id,loser_hand,loser_name,loser_ioc,score,best_of,round,w_ace,l_ace";

/// Pola w cudzysłowie mogą zawierać przecinki, a `""` oznacza pojedynczy cudzysłów.
#[test]
fn splits_csv_with_quotes() {
    assert_eq!(split_csv_line("a,b,,c\r\n"), vec!["a", "b", "", "c"]);
    assert_eq!(
        split_csv_line(r#"1,"Tour, Final","say ""hi""",x"#),
        vec!["1", "Tour, Final", r#"say "hi""#, "x"]
    );
}

/// Parser setów ignoruje wynik tiebreaka i znaczniki RET, ale liczy super tiebreak.
#[test]
fn parses_set_scores() {
    assert_eq!(parse_sets("6-4 3-6 7-6(5)"), vec![(6, 4), (3, 6), (7, 6)]);
    assert_eq!(parse_sets("6-3 2-0 RET"), vec![(6, 3), (2, 0)]);
    assert_eq!(
        parse_sets("6-7(4) 6-4 [10-7]"),
        vec![(6, 7), (6, 4), (10, 7)]
    );
    assert!(parse_sets("W/O").is_empty());
}

/// Dzień meczu zależy od rundy względem pierwszej rundy drabinki.
#[test]
fn offsets_rounds_by_draw_size() {
    assert_eq!(round_day_offset("R128", Some(128)), 0);
    assert_eq!(round_day_offset("R32", Some(128)), 2);
    assert_eq!(round_day_offset("F", Some(128)), 6);
    assert_eq!(round_day_offset("R32", Some(32)), 0);
    assert_eq!(round_day_offset("F", Some(32)), 4);
    // Mecz o brąz gra razem z finałem, a nieznana runda nie przesuwa daty.
    assert_eq!(round_day_offset("BR", Some(32)), 4);
    assert_eq!(round_day_offset("???", Some(32)), 0);
}

/// Losowanie stron jest deterministyczne i niezależne od wyniku, więc zwycięzca bywa po obu stronach.
#[test]
fn home_side_assignment_is_stable_and_balanced() {
    assert_eq!(winner_is_home("2024-580-1"), winner_is_home("2024-580-1"));

    let home_wins = (0..200)
        .filter(|n| winner_is_home(&format!("2024-580-{}", n)))
        .count();
    assert!(
        (60..=140).contains(&home_wins),
        "niezrównoważone losowanie: {}",
        home_wins
    );
}

/// Sackmann: zwycięzca i sety są spójne po losowaniu stron, a walkowery są pomijane.
#[test]
fn parses_sackmann_csv() {
    let csv = format!(
        "{}\n{}\n{}\n{}\n",
        CSV_HEADER,
        "2024-580,Australian Open,Hard,128,20240114,1,100,R,Jannik Sinner,ITA,200,L,Alex de Minaur,AUS,6-3 6-2 6-1,5,R128,12,3",
        "2024-580,Australian Open,Hard,128,20240114,2,101,R,Player W,ESP,201,R,Player L,FRA,W/O,5,R128,,",
        "2024-580,Australian Open,Hard,128,20240114,3,102,L,Player A,USA,202,R,Player B,GBR,7-6(5) 3-6 6-4,5,R64,5,8",
    );

    let matches = parse_sackmann_csv(&csv, "ATP").unwrap();
    // Walkower (W/O) nie jest rozegranym meczem.
    assert_eq!(matches.len(), 2);

    for m in &matches {
        assert_eq!(m.sport, "TENNIS");
        assert_eq!(m.competition_code, "ATP");
        assert_eq!(m.season, "2024");
        assert_eq!(m.status, "FINISHED");
    }

    let first = &matches[0];
    // Sinner jest zwycięzcą niezależnie od tego, po której stronie wylosowano.
    let sinner_home = first.home.name == "Jannik Sinner";
    assert_eq!(
        first.winner.as_deref(),
        Some(if sinner_home { "HOME" } else { "AWAY" })
    );
    let (winner_sets, loser_sets) = if sinner_home {
        (first.home_score, first.away_score)
    } else {
        (first.away_score, first.home_score)
    };
    assert_eq!((winner_sets, loser_sets), (Some(3), Some(0)));
    assert_eq!(first.match_date.to_rfc3339(), "2024-01-14T12:00:00+00:00");
    assert_eq!(first.home.metadata.get("ioc").is_some(), true);
    match &first.details {
        MatchDetails::Tennis {
            surface,
            best_of,
            sets,
            home_aces,
            away_aces,
        } => {
            assert_eq!(surface, "Hard");
            assert_eq!(*best_of, 5);
            assert_eq!(sets.len(), 3);
            // Asy przypisane do właściwych stron: Sinner miał 12, de Minaur 3.
            let (sinner_aces, minaur_aces) = if sinner_home {
                (home_aces, away_aces)
            } else {
                (away_aces, home_aces)
            };
            assert_eq!((*sinner_aces, *minaur_aces), (Some(12), Some(3)));
            // Sety zapisane z perspektywy gospodarza.
            let expected: Vec<(u8, u8)> = if sinner_home {
                vec![(6, 3), (6, 2), (6, 1)]
            } else {
                vec![(3, 6), (2, 6), (1, 6)]
            };
            assert_eq!(sets, &expected);
        }
        other => panic!("oczekiwano szczegółów tenisa, otrzymano {:?}", other),
    }

    // Kolejna runda jest rozgrywana dzień później (draw_size 128: R128 → R64).
    assert_eq!(
        matches[1].match_date.to_rfc3339(),
        "2024-01-15T12:00:00+00:00"
    );
}

/// Nieoczekiwany nagłówek lub nieznany tour kończą się błędem.
#[test]
fn rejects_invalid_sackmann_input() {
    assert!(parse_sackmann_csv("foo,bar\n1,2\n", "ATP").is_err());
    assert!(parse_sackmann_csv("", "ATP").is_err());
    assert!(parse_sackmann_csv(CSV_HEADER, "ITF").is_err());
}

/// ESPN: bierzemy tylko single, pomijamy zawodników TBD, a wynik zapisujemy dla zakończonych meczów.
#[test]
fn parses_espn_scoreboard() {
    let json = r#"{
        "events": [{
            "id": "1", "season": {"year": 2025},
            "groupings": [
                {"grouping": {"slug": "mens-singles"}, "competitions": [
                    {"id": "m1", "date": "2025-01-20T11:00Z",
                     "status": {"type": {"state": "post"}},
                     "competitors": [
                        {"athlete": {"displayName": "Carlos Alcaraz"}, "winner": true,
                         "linescores": [{"value": 6.0}, {"value": 3.0}, {"value": 7.0}]},
                        {"athlete": {"displayName": "Jannik Sinner"}, "winner": false,
                         "linescores": [{"value": 4.0}, {"value": 6.0}, {"value": 5.0}]}]},
                    {"id": "m2", "date": "2025-01-21T11:00:00Z",
                     "status": {"type": {"state": "pre"}},
                     "competitors": [
                        {"athlete": {"displayName": "Carlos Alcaraz"}},
                        {"athlete": {"displayName": "TBD"}}]},
                    {"id": "m3", "date": "2025-01-21T13:00:00Z",
                     "status": {"type": {"state": "pre"}},
                     "competitors": [
                        {"athlete": {"displayName": "Player A"}},
                        {"athlete": {"displayName": "Player B"}}]}
                ]},
                {"grouping": {"slug": "mens-doubles"}, "competitions": [
                    {"id": "d1", "date": "2025-01-21T11:00:00Z",
                     "status": {"type": {"state": "pre"}},
                     "competitors": [
                        {"athlete": {"displayName": "X"}}, {"athlete": {"displayName": "Y"}}]}
                ]}
            ]
        }]
    }"#;

    let matches = parse_espn_scoreboard(json, "ATP").unwrap();
    // Debel i mecz z zawodnikiem TBD są pomijane.
    assert_eq!(matches.len(), 2);

    let finished = &matches[0];
    assert_eq!(finished.season, "2025");
    assert_eq!(finished.status, "FINISHED");
    assert_eq!(finished.winner.as_deref(), Some("HOME"));
    assert_eq!(
        (finished.home_score, finished.away_score),
        (Some(2), Some(1))
    );
    assert_eq!(
        finished.match_date.to_rfc3339(),
        "2025-01-20T11:00:00+00:00"
    );

    let upcoming = &matches[1];
    assert_eq!(upcoming.status, "SCHEDULED");
    assert_eq!(upcoming.winner, None);
    assert_eq!((upcoming.home_score, upcoming.away_score), (None, None));
}
