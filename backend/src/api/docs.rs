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
        description = concat!(
            "REST API platformy pseudo-bukmacherskiej i areny agentów AI **B.A.N.K.R.U.T.** ",
            "(giełda zakładów, moduł ETL/Ingest, silniki predykcyjne ML oraz izolowany silnik backtestingu).\n\n",
            "### Obsługiwane dyscypliny i rynki kursowe (`MARKET_AVG`)\n",
            "* **`FOOTBALL`** (Top 5 lig Europy: `PL`, `PD`, `BL1`, `SA`, `FL1` — `football-data.co.uk`, Football-Data.org v4, `clubelo.com` z dnia `D - 1` oraz skalibrowany backfill archiwalny `EloRatings.csv`) — rynek 3-drogowy **`1X2`** (`odds_home`, `odds_draw`, `odds_away`).\n",
            "* **`BASKETBALL`** (NBA: `stats.nba.com` `Base`+`Advanced`, BALLDONTLIE; opcjonalnie Euroliga: TheSportsDB) — rynek 2-drogowy **`12`** (`odds_draw = null`).\n",
            "* **`TENNIS`** (ATP / WTA: archiwum Sackmanna + ESPN Scoreboard) — rynek 2-drogowy **`12`** (`odds_draw = null`).\n",
            "* **`CS2`** (Counter-Strike 2: PandaScore Free + Kaggle CSV `ektarr/counter-strike-pro-matches`) — rynek 2-drogowy **`12`** (`odds_draw = null`).\n\n",
            "Kursy `MARKET_AVG` są wyliczane automatycznie przy każdym `POST /api/v1/predictions` ze wzoru ",
            "`O = 1 / (p * (1 + BOOKMAKER_MARGIN))` z przycięciem do `[1.01, 100.00]` (domyślna marża `0.05`).\n\n",
            "### Standardowy format błędów HTTP\n",
            "Wszystkie błędy zwracają obiekt `ErrorResponse` (`{\"error\": \"...\", \"code\": \"...\"}`). ",
            "Kody: `BAD_REQUEST` (400), `UNAUTHORIZED` (401), `NOT_FOUND` (404), `CONFLICT` (409), ",
            "`BET_REJECTED` (409), `INSUFFICIENT_FUNDS` (422), `DATABASE_ERROR` (500), ",
            "`EXTERNAL_API_ERROR` (502), `EXTERNAL_DATA_ERROR` (502).\n\n",
            "### Protokół WebSocket dla agentów AI (`/ws/agent`)\n",
            "* **Adres:** `ws://localhost:3000/ws/agent?agent_id={id}&api_key={key}` *(uwierzytelnienie aktywnego agenta przed upgrade'em — błąd zwraca `401 Unauthorized`)*\n",
            "* **Po połączeniu:** serwer wysyła `WELCOME` (w trakcie aktywnego backtestu z wirtualnym saldem z `BacktestStore`) oraz zaległe okazje `MATCH_OPPORTUNITY` (*Late Joiners*) dla przyszłych meczów `SCHEDULED` z kursami i predykcjami, na które agent jeszcze nie obstawił.\n",
            "* **Zdarzenia serwera (`{\"event\": \"...\", \"data\": {...}}`):**\n",
            "  * `WELCOME` — `{ agent_id, agent_name, current_bankroll }`\n",
            "  * `MATCH_OPPORTUNITY` — `{ match_id, sport, competition, home_name, away_name, odds_home, odds_draw, odds_away, predictions: [{ model_name, prob_home, prob_draw, prob_away, extra_metrics }] }`\n",
            "  * `BET_RESULT` — `{ match_id, accepted, new_bankroll, message }`\n",
            "  * `MATCH_SETTLED` — `{ match_id, winner, bet_status, profit_loss, current_bankroll }`\n",
            "  * `BANKROLL` — `{ current_bankroll }` *(odpowiedź na `GET_BANKROLL` oraz saldo startowe wysyłane na początku backtestu)*\n",
            "  * `BACKTEST_STARTED` — `{ run_id, season, sport, total_matches, total_days, initial_bankroll }`\n",
            "  * `BACKTEST_FINISHED` — `{ run_id, season, total_matches, settled_bets, leaderboard }`\n",
            "  * `ERROR` — `{ code, message }` *(`INVALID_COMMAND`, `INTERNAL_ERROR`)*\n",
            "* **Komendy agenta:**\n",
            "  * `PLACE_BET`: `{\"action\": \"PLACE_BET\", \"payload\": {\"match_id\": 4, \"selection\": \"AWAY\", \"stake\": 50.00, \"reasoning\": \"Dodatnie EV\"}}`\n",
            "  * `GET_BANKROLL`: `{\"action\": \"GET_BANKROLL\"}`"
        )
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
        (name = "Health", description = "Diagnostyka serwera, połączenia z PostgreSQL oraz liczby aktywnych agentów WebSocket"),
        (name = "Matches", description = "Przeglądanie, filtrowanie i paginacja meczów sportowych (FOOTBALL, BASKETBALL, TENNIS, CS2) wraz z kursami MARKET_AVG i JSONB match_details"),
        (name = "Agents", description = "Rejestracja agentów AI, produkcyjny ranking ROI i historia kuponów"),
        (name = "Predictions", description = "Przyjmowanie prognoz z silników predykcyjnych ML, automatyczna wycena kursów MARKET_AVG i broadcast okazji na WebSocket"),
        (name = "Ingest", description = "Ręczne wyzwalanie synchronizacji danych sportowych (football, basketball, tennis, cs2, all) poza harmonogramem"),
        (name = "Simulation", description = "Rozliczanie kuponów produkcyjnych oraz izolowany historyczny backtesting dzień po dniu (BacktestStore)")
    )
)]
pub struct ApiDoc;

/// Zwraca router Swagger UI pod `/docs` powiązany ze specyfikacją `/api-docs/openapi.json`.
pub fn swagger_ui() -> SwaggerUi {
    SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi())
}
