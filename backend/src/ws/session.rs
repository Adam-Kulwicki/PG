// src/ws/session.rs
//! Budowanie wiadomości `MATCH_OPPORTUNITY` oraz synchronizacja zaległych okazji
//! dla agentów, którzy podłączyli się po wyliczeniu predykcji (Late Joiners).

use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{types::Json, FromRow, PgPool};

use crate::{
    error::AppResult,
    ws::protocol::{ModelPredictionSummary, ServerMessage},
};

#[derive(Debug, FromRow)]
struct OpportunityRow {
    sport: String,
    competition_name: String,
    home_name: String,
    away_name: String,
    odds_home: Decimal,
    odds_draw: Option<Decimal>,
    odds_away: Decimal,
}

#[derive(Debug, FromRow)]
struct PredictionRow {
    model_name: String,
    prob_home: Decimal,
    prob_draw: Decimal,
    prob_away: Decimal,
    extra_metrics: Json<Value>,
}

/// Buduje `MATCH_OPPORTUNITY` dla meczu (kursy `MARKET_AVG` + wszystkie predykcje).
/// Zwraca `None`, gdy mecz nie istnieje lub nie ma jeszcze kursów.
/// Nie sprawdza statusu meczu – robi to wywołujący (produkcja: SCHEDULED, backtest: FINISHED).
pub async fn build_opportunity(db: &PgPool, match_id: i32) -> AppResult<Option<ServerMessage>> {
    let row = sqlx::query_as::<_, OpportunityRow>(
        r#"
        SELECT
            c.sport,
            c.name AS competition_name,
            p_home.name AS home_name,
            p_away.name AS away_name,
            o.odds_home,
            o.odds_draw,
            o.odds_away
        FROM matches m
        JOIN competitions c ON c.id = m.competition_id
        JOIN participants p_home ON p_home.id = m.home_participant_id
        JOIN participants p_away ON p_away.id = m.away_participant_id
        JOIN match_odds o ON o.match_id = m.id AND o.bookmaker = 'MARKET_AVG'
        WHERE m.id = $1
        "#,
    )
    .bind(match_id)
    .fetch_optional(db)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let predictions = sqlx::query_as::<_, PredictionRow>(
        r#"
        SELECT pm.name AS model_name, p.prob_home, p.prob_draw, p.prob_away, p.extra_metrics
        FROM predictions p
        JOIN prediction_models pm ON pm.id = p.model_id
        WHERE p.match_id = $1
        ORDER BY p.created_at, p.id
        "#,
    )
    .bind(match_id)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|p| ModelPredictionSummary {
        model_name: p.model_name,
        prob_home: p.prob_home,
        prob_draw: p.prob_draw,
        prob_away: p.prob_away,
        extra_metrics: p.extra_metrics.0,
    })
    .collect();

    Ok(Some(ServerMessage::MatchOpportunity {
        match_id,
        sport: row.sport,
        competition: row.competition_name,
        home_name: row.home_name,
        away_name: row.away_name,
        odds_home: row.odds_home,
        odds_draw: row.odds_draw,
        odds_away: row.odds_away,
        predictions,
    }))
}

/// Okazje dla agenta, który dołączył "w trakcie": przyszłe mecze z kursami i predykcjami,
/// na które ten agent jeszcze nie postawił zakładu.
pub async fn pending_opportunities(db: &PgPool, agent_id: i32) -> AppResult<Vec<ServerMessage>> {
    let ids: Vec<(i32,)> = sqlx::query_as(
        r#"
        SELECT m.id
        FROM matches m
        WHERE m.status = 'SCHEDULED'
          AND m.match_date > NOW()
          AND EXISTS (SELECT 1 FROM predictions p WHERE p.match_id = m.id)
          AND EXISTS (SELECT 1 FROM match_odds o WHERE o.match_id = m.id AND o.bookmaker = 'MARKET_AVG')
          AND NOT EXISTS (SELECT 1 FROM bets b WHERE b.match_id = m.id AND b.agent_id = $1)
        ORDER BY m.match_date, m.id
        "#,
    )
    .bind(agent_id)
    .fetch_all(db)
    .await?;

    let mut messages = Vec::with_capacity(ids.len());
    for (match_id,) in ids {
        if let Some(msg) = build_opportunity(db, match_id).await? {
            messages.push(msg);
        }
    }
    Ok(messages)
}
