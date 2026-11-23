// src/models/match_model.rs
//! Modele meczów, kursów oraz wzbogacony widok meczu zwracany przez REST API.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json, FromRow};
use utoipa::ToSchema;

/// Statystyki drużyny z jednego meczu NBA (połączenie `Base` oraz `Advanced` z `teamgamelogs`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct BasketballTeamStats {
    // --- Base (box-score) ---
    pub min: Option<f64>,
    pub fgm: Option<f64>,
    pub fga: Option<f64>,
    pub fg_pct: Option<f64>,
    pub fg3m: Option<f64>,
    pub fg3a: Option<f64>,
    pub fg3_pct: Option<f64>,
    pub ftm: Option<f64>,
    pub fta: Option<f64>,
    pub ft_pct: Option<f64>,
    pub oreb: Option<f64>,
    pub dreb: Option<f64>,
    pub reb: Option<f64>,
    pub ast: Option<f64>,
    pub tov: Option<f64>,
    pub stl: Option<f64>,
    pub blk: Option<f64>,
    pub blka: Option<f64>,
    pub pf: Option<f64>,
    pub pfd: Option<f64>,
    pub plus_minus: Option<f64>,
    // --- Advanced (metryki efektywności) ---
    pub off_rating: Option<f64>,
    pub def_rating: Option<f64>,
    pub net_rating: Option<f64>,
    pub pace: Option<f64>,
    pub efg_pct: Option<f64>,
    pub ts_pct: Option<f64>,
    pub tm_tov_pct: Option<f64>,
    pub oreb_pct: Option<f64>,
    pub pie: Option<f64>,
}

/// Statystyki drużyny z jednego meczu piłkarskiego (`football-data.co.uk`, post-match).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FootballTeamStats {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shots: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shots_on_target: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corners: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fouls: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yellow_cards: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red_cards: Option<i32>,
}

/// Przedmeczowy ranking siły drużyn na dzień przed meczem (ClubElo, pre-match).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FootballPreMatchElo {
    pub home_elo: Option<f64>,
    pub away_elo: Option<f64>,
    pub elo_diff: Option<f64>,
    pub home_rank: Option<i32>,
    pub away_rank: Option<i32>,
    /// Opcjonalne prawdopodobieństwa wyliczone z endpointu `/Fixtures` dla meczów `SCHEDULED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clubelo_prob_home: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clubelo_prob_draw: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clubelo_prob_away: Option<f64>,
}

/// Szczegóły specyficzne dla dyscypliny, przechowywane w kolumnie JSONB `match_details`.
/// Wariant rozpoznawany jest po polu `sport_type` (np. `"football"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "sport_type", rename_all = "snake_case")]
pub enum MatchDetails {
    /// Piłka nożna: wynik do przerwy, kolejka oraz opcjonalne statystyki meczowe i Elo przedmeczowe.
    Football {
        ht_home: Option<i32>,
        ht_away: Option<i32>,
        round: String,
        /// Box ogranicza rozmiar wariantu enuma `MatchDetails` dla pozostałych dyscyplin.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        home_stats: Option<Box<FootballTeamStats>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        away_stats: Option<Box<FootballTeamStats>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pre_match_elo: Option<Box<FootballPreMatchElo>>,
    },
    /// Koszykówka: punkty w kwartach (gospodarz, gość), dogrywka oraz opcjonalne statystyki NBA.
    Basketball {
        #[schema(value_type = Vec<[i32; 2]>)]
        quarters: Vec<(i32, i32)>,
        overtime: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        season_type: Option<String>,
        /// Box ogranicza rozmiar wariantu enuma `MatchDetails` dla pozostałych dyscyplin.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        home_stats: Option<Box<BasketballTeamStats>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        away_stats: Option<Box<BasketballTeamStats>>,
    },
    /// Tenis: nawierzchnia, format meczu, gemy w setach oraz liczba asów.
    Tennis {
        surface: String,
        best_of: u8,
        #[schema(value_type = Vec<[u8; 2]>)]
        sets: Vec<(u8, u8)>,
        home_aces: Option<u16>,
        away_aces: Option<u16>,
    },
}

/// Mecz (przeszły, trwający lub nadchodzący) – wiersz tabeli `matches`.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Match {
    pub id: i32,
    pub competition_id: i32,
    pub season: String,
    pub match_date: DateTime<Utc>,
    /// "SCHEDULED", "LIVE", "FINISHED", "POSTPONED" lub "CANCELLED".
    pub status: String,
    pub home_participant_id: i32,
    pub away_participant_id: i32,
    /// Wynik główny: bramki, punkty lub wygrane sety (zależnie od sportu).
    pub home_score: Option<i32>,
    pub away_score: Option<i32>,
    /// "HOME", "AWAY" lub "DRAW" – ustawiane po zakończeniu meczu.
    pub winner: Option<String>,
    pub match_details: Json<MatchDetails>,
    /// Identyfikatory z zewnętrznych API (ułatwiają aktualizację wyników).
    pub external_ids: Json<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Kursy bukmacherskie dla meczu (1X2 dla piłki, 12 dla koszykówki i tenisa).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MatchOdds {
    pub id: i32,
    pub match_id: i32,
    /// Źródło kursu; giełda używa kursu "MARKET_AVG".
    pub bookmaker: String,
    pub odds_home: Decimal,
    /// `None` dla sportów bez remisu.
    pub odds_draw: Option<Decimal>,
    pub odds_away: Decimal,
    pub updated_at: DateTime<Utc>,
}

/// Widok meczu zwracany przez `GET /api/v1/matches`, wzbogacony o nazwy uczestników i kursy.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct MatchWithDetailsDto {
    pub id: i32,
    pub sport: String,
    pub competition_name: String,
    pub season: String,
    pub match_date: DateTime<Utc>,
    pub status: String,
    pub home_participant_id: i32,
    pub home_name: String,
    pub away_participant_id: i32,
    pub away_name: String,
    pub home_score: Option<i32>,
    pub away_score: Option<i32>,
    pub winner: Option<String>,
    #[schema(value_type = MatchDetails)]
    pub match_details: Json<MatchDetails>,
    pub odds_home: Option<Decimal>,
    pub odds_draw: Option<Decimal>,
    pub odds_away: Option<Decimal>,
}
