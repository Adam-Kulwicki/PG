// src/api/docs.rs
//! Specyfikacja OpenAPI 3 i interaktywny Swagger UI (`/docs`).

use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// Główna definicja OpenAPI 3 dla REST API B.A.N.K.R.U.T.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "B.A.N.K.R.U.T. Backend API",
        version = "0.1.0",
        description = "REST API giełdy zakładów sportowych, modułu ETL/Ingest, silników predykcyjnych i backtestera.\n\n### WebSocket dla agentów AI\n* **Adres:** `ws://localhost:3000/ws/agent?agent_id={id}&api_key={key}`\n\n* **Zdarzenia serwera (`event`):** `WELCOME`, `MATCH_OPPORTUNITY`, `BET_RESULT`, `MATCH_SETTLED`, `BANKROLL`, `BACKTEST_STARTED`, `BACKTEST_FINISHED`, `ERROR`\n\n* **Komendy agenta (`action`):** `PLACE_BET`, `GET_BANKROLL`"
    ),
    paths(
        crate::api::health::health_check,
        crate::api::matches::list_matches,
        crate::api::matches::get_match_by_id,
        crate::api::agents::list_agents,
        crate::api::agents::register_agent,
        crate::api::agents::get_leaderboard,
        crate::api::agents::get_agent_bets,
        crate::api::predictions::create_prediction,
        crate::api::ingest::trigger_ingest,
        crate::api::simulation::settle,
        crate::api::simulation::backtest,
        crate::api::simulation::get_latest_backtest,
        crate::api::simulation::list_backtest_runs,
        crate::api::simulation::get_backtest_run,
        crate::api::simulation::get_backtest_leaderboard,
        crate::api::simulation::get_backtest_bets,
    ),
    components(
        schemas(
            crate::error::ErrorResponse,
            crate::api::health::HealthResponse,
            crate::models::match_model::MatchWithDetailsDto,
            crate::models::match_model::MatchDetails,
            crate::models::match_model::FootballTeamStats,
            crate::models::match_model::FootballPreMatchElo,
            crate::models::match_model::BasketballTeamStats,
            crate::models::match_model::Cs2MapResult,
            crate::models::match_model::Cs2Player,
            crate::models::match_model::Cs2PlayerStats,
            crate::models::match_model::Cs2VetoAction,
            crate::models::bet::Agent,
            crate::api::agents::CreateAgentDto,
            crate::api::agents::AgentCreatedResponse,
            crate::api::agents::LeaderboardEntryDto,
            crate::models::bet::Bet,
            crate::models::prediction::Prediction,
            crate::models::prediction::CreatePredictionDto,
            crate::api::ingest::IngestStartedResponse,
            crate::api::simulation::SettleResponse,
            crate::domain::backtester::BacktestParams,
            crate::domain::backtester::BacktestPlan,
            crate::domain::backtester::BacktestStatus,
            crate::domain::backtester::VirtualBet,
            crate::domain::backtester::BacktestLeaderboardEntry,
            crate::domain::backtester::BacktestRunSummary,
            crate::domain::backtester::BacktestReport,
        )
    ),
    tags(
        (name = "Health", description = "Diagnostyka serwera i połączenia z bazą PostgreSQL"),
        (name = "Matches", description = "Przeglądanie i filtrowanie meczów sportowych"),
        (name = "Agents", description = "Rejestracja agentów AI, ranking ROI i historia zakładów"),
        (name = "Predictions", description = "Przyjmowanie prognoz z silników predykcyjnych ML"),
        (name = "Ingest", description = "Ręczne wyzwalanie synchronizacji danych sportowych"),
        (name = "Simulation", description = "Rozliczanie kuponów i historyczny backtesting")
    )
)]
pub struct ApiDoc;

/// Zwraca router Swagger UI pod `/docs` powiązany ze specyfikacją `/api-docs/openapi.json`.
pub fn swagger_ui() -> SwaggerUi {
    SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi())
}
