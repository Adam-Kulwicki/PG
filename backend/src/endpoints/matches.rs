use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct Match {
    match_id: String,
    home_team: String,
    away_team: String,
    status: String,
    odds: Odds,
}

#[derive(Serialize)]
pub struct Odds {
    home_win: f64,
    draw: f64,
    away_win: f64,
}

pub async fn matches() -> Json<Vec<Match>> {
    let data = vec![Match {
        match_id: "1".to_string(),
        home_team: "Team A".to_string(),
        away_team: "Team B".to_string(),
        status: "scheduled".to_string(),
        odds: Odds {
            home_win: 1.5,
            draw: 3.0,
            away_win: 2.5,
        },
    }];

    Json(data)
}