use axum::{extract::Query, Json};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MatchStatus {
    Open,
    Locked,
    Settled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Match {
    match_id: uuid::Uuid,
    home_team: String,
    away_team: String,
    status: MatchStatus,
    odds: Odds,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Odds {
    home_win: f64,
    draw: f64,
    away_win: f64,
}

// Struktura do przechwytywania Query Params: ?status=OPEN
#[derive(Debug, Deserialize)]
pub struct MatchFilter {
    pub status: Option<MatchStatus>,
}

pub async fn get_matches(Query(filter): Query<MatchFilter>) -> Json<Vec<Match>> {
    let mut data = vec![
        Match {
            match_id: uuid::Uuid::new_v4(),
            home_team: "Team A".to_string(),
            away_team: "Team B".to_string(),
            status: MatchStatus::Open,
            odds: Odds {
                home_win: 1.5,
                draw: 3.0,
                away_win: 2.5,
            },
        },
        Match {
            match_id: uuid::Uuid::new_v4(),
            home_team: "Team C".to_string(),
            away_team: "Team D".to_string(),
            status: MatchStatus::Locked,
            odds: Odds {
                home_win: 2.1,
                draw: 3.2,
                away_win: 3.4,
            },
        },
    ];

    if let Some(expected_status) = filter.status {
        data.retain(|m| m.status == expected_status);
    }

    Json(data)
}
