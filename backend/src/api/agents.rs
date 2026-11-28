// src/api/agents.rs
//! Rejestracja agentów, ranking (leaderboard) i historia ich zakładów.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

use crate::{
    error::{AppError, AppResult},
    models::bet::{Agent, Bet},
    state::AppState,
};

/// Trasy: lista/rejestracja agentów, `/leaderboard` oraz `/{id}/bets`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_agents).post(register_agent))
        .route("/leaderboard", get(get_leaderboard))
        .route("/{id}/bets", get(get_agent_bets))
}

/// Payload rejestracji agenta (`POST /api/v1/agents`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAgentDto {
    /// Unikalna nazwa agenta (niepusta).
    #[schema(example = "Agent_Kelly_v1")]
    pub name: String,
    /// Unikalny klucz autoryzacyjny do połączeń WebSocket (`/ws/agent`, niepusty).
    #[schema(example = "tajny_klucz_agenta")]
    pub api_key: String,
    /// Opcjonalny opis strategii obstawiania wyświetlany w rankingu.
    #[schema(example = "Ułamkowe kryterium Kelly'ego przy EV > 5%")]
    pub strategy_description: Option<String>,
    /// Opcjonalny kapitał początkowy (domyślnie `1000.00`).
    #[schema(example = 1000.00)]
    pub initial_bankroll: Option<Decimal>,
}

/// Odpowiedź po rejestracji – jedyny moment, w którym `api_key` wraca w odpowiedzi API.
#[derive(Debug, Serialize, ToSchema)]
pub struct AgentCreatedResponse {
    /// Nadany identyfikator agenta.
    #[schema(example = 3)]
    pub id: i32,
    /// Nazwa zarejestrowanego agenta.
    #[schema(example = "Agent_Kelly_v1")]
    pub name: String,
    /// Klucz API potrzebny do połączenia z `/ws/agent`.
    #[schema(example = "tajny_klucz_agenta")]
    pub api_key: String,
    /// Początkowe saldo portfela.
    #[schema(example = 1000.00)]
    pub current_bankroll: Decimal,
}

/// Wiersz rankingu: saldo, zysk netto, ROI % oraz statystyki zakładów agenta.
#[derive(Debug, Serialize, FromRow, ToSchema)]
pub struct LeaderboardEntryDto {
    #[schema(example = 1)]
    pub id: i32,
    #[schema(example = "Agent_Kelly_Value")]
    pub name: String,
    #[schema(example = "Stawia wg ułamkowego Kryterium Kelly'ego przy EV > 5%")]
    pub strategy_description: Option<String>,
    #[schema(example = 1000.00)]
    pub initial_bankroll: Decimal,
    #[schema(example = 1185.50)]
    pub current_bankroll: Decimal,
    /// Zysk lub strata netto (`current_bankroll - initial_bankroll`).
    #[schema(example = 185.50)]
    pub total_profit: Decimal,
    /// Zwrot z kapitału w procentach (`round(total_profit / initial_bankroll * 100, 2)`).
    #[schema(example = 18.55)]
    pub roi_pct: Decimal,
    /// Łączna liczba postawionych kuponów (wszystkie statusy).
    #[schema(example = 24)]
    pub total_bets: i64,
    /// Liczba wygranych kuponów (`status = 'WON'`).
    #[schema(example = 14)]
    pub won_bets: i64,
}

/// Lista zarejestrowanych agentów AI.
///
/// Zwraca wszystkich agentów posortowanych rosnąco po `id`. Pole `api_key` jest ukryte
/// (`#[serde(skip_serializing)]`) i nigdy nie pojawia się w odpowiedzi tego endpointu.
#[utoipa::path(
    get,
    path = "/api/v1/agents",
    tag = "Agents",
    responses(
        (status = 200, description = "Lista zarejestrowanych agentów (bez kluczy API)", body = [Agent]),
        (status = 500, description = "Błąd bazy danych", body = crate::error::ErrorResponse)
    )
)]
pub async fn list_agents(State(state): State<AppState>) -> AppResult<Json<Vec<Agent>>> {
    let agents = sqlx::query_as::<_, Agent>("SELECT * FROM agents ORDER BY id ASC")
        .fetch_all(&state.db)
        .await?;

    Ok(Json(agents))
}

/// Rejestracja nowego agenta AI.
///
/// Tworzy aktywne konto agenta z podanym kapitałem początkowym (domyślnie `1000.00`).
/// To jedyny endpoint zwracający `api_key` w treści odpowiedzi. Pola `name` i `api_key`
/// muszą być niepuste oraz unikalne w bazie.
#[utoipa::path(
    post,
    path = "/api/v1/agents",
    tag = "Agents",
    request_body = CreateAgentDto,
    responses(
        (status = 201, description = "Agent utworzony pomyślnie", body = AgentCreatedResponse),
        (status = 400, description = "Pusta nazwa agenta lub pusty api_key", body = crate::error::ErrorResponse),
        (status = 500, description = "Błąd bazy danych (w tym duplikat name lub api_key)", body = crate::error::ErrorResponse)
    )
)]
pub async fn register_agent(
    State(state): State<AppState>,
    Json(payload): Json<CreateAgentDto>,
) -> AppResult<(StatusCode, Json<AgentCreatedResponse>)> {
    if payload.name.trim().is_empty() || payload.api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Nazwa agenta oraz api_key nie mogą być puste".to_string(),
        ));
    }

    // Domyślny kapitał początkowy to 1000
    let bankroll = payload
        .initial_bankroll
        .unwrap_or_else(|| Decimal::from(1000));

    // Saldo bieżące startuje równe początkowemu ($4 użyte dwukrotnie)
    let agent = sqlx::query_as::<_, Agent>(
        r#"
        INSERT INTO agents (name, api_key, strategy_description, initial_bankroll, current_bankroll)
        VALUES ($1, $2, $3, $4, $4)
        RETURNING *
        "#,
    )
    .bind(payload.name.trim())
    .bind(payload.api_key.trim())
    .bind(payload.strategy_description)
    .bind(bankroll)
    .fetch_one(&state.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(AgentCreatedResponse {
            id: agent.id,
            name: agent.name,
            api_key: agent.api_key,
            current_bankroll: agent.current_bankroll,
        }),
    ))
}

/// Ranking produkcyjny agentów AI (leaderboard).
///
/// Zwraca zestawienie wszystkich zarejestrowanych agentów (włącznie z agentami bez zakładów)
/// posortowane malejąco według `current_bankroll`, wraz z zyskiem netto, ROI % i liczbą trafień.
#[utoipa::path(
    get,
    path = "/api/v1/agents/leaderboard",
    tag = "Agents",
    responses(
        (status = 200, description = "Ranking agentów posortowany malejąco wg salda", body = [LeaderboardEntryDto]),
        (status = 500, description = "Błąd bazy danych", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_leaderboard(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<LeaderboardEntryDto>>> {
    // ROI = zysk / kapitał początkowy * 100; NULLIF chroni przed dzieleniem przez zero.
    // LEFT JOIN, aby agenci bez zakładów też trafili do rankingu.
    let leaderboard = sqlx::query_as::<_, LeaderboardEntryDto>(
        r#"
        SELECT 
            a.id,
            a.name,
            a.strategy_description,
            a.initial_bankroll,
            a.current_bankroll,
            (a.current_bankroll - a.initial_bankroll) AS total_profit,
            ROUND(((a.current_bankroll - a.initial_bankroll) / NULLIF(a.initial_bankroll, 0)) * 100, 2) AS roi_pct,
            COUNT(b.id) AS total_bets,
            COUNT(b.id) FILTER (WHERE b.status = 'WON') AS won_bets
        FROM agents a
        LEFT JOIN bets b ON b.agent_id = a.id
        GROUP BY a.id
        ORDER BY a.current_bankroll DESC
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(leaderboard))
}

/// Historia produkcyjnych zakładów wybranego agenta.
///
/// Zwraca listę wszystkich kuponów postawionych przez wskazanego agenta w tabeli `bets`,
/// posortowaną od najnowszych (`placed_at DESC`). Nie obejmuje wirtualnych kuponów z backtestów.
#[utoipa::path(
    get,
    path = "/api/v1/agents/{id}/bets",
    tag = "Agents",
    params(
        ("id" = i32, Path, description = "Identyfikator agenta", example = 1)
    ),
    responses(
        (status = 200, description = "Historia zakładów agenta (od najnowszych)", body = [Bet]),
        (status = 500, description = "Błąd bazy danych", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_agent_bets(
    State(state): State<AppState>,
    Path(agent_id): Path<i32>,
) -> AppResult<Json<Vec<Bet>>> {
    let bets =
        sqlx::query_as::<_, Bet>("SELECT * FROM bets WHERE agent_id = $1 ORDER BY placed_at DESC")
            .bind(agent_id)
            .fetch_all(&state.db)
            .await?;

    Ok(Json(bets))
}
