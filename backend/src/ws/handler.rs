// src/ws/handler.rs
//! Obsługa dwukierunkowego połączenia WebSocket agenta (`/ws/agent`).

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::Response,
};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use crate::{
    domain::betting_exchange,
    error::{AppError, AppResult},
    models::bet::Agent,
    state::AppState,
    ws::{
        protocol::{AgentCommand, ServerMessage},
        session,
    },
};

#[derive(Debug, Deserialize)]
pub struct WsAuthParams {
    pub agent_id: i32,
    pub api_key: String,
}

/// GET /ws/agent?agent_id=1&api_key=...
///
/// Uwierzytelnienie następuje przed upgrade'em połączenia – błędne dane dają `401`.
pub async fn ws_agent(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(params): Query<WsAuthParams>,
) -> AppResult<Response> {
    let agent = sqlx::query_as::<_, Agent>(
        "SELECT * FROM agents WHERE id = $1 AND api_key = $2 AND is_active = TRUE",
    )
    .bind(params.agent_id)
    .bind(&params.api_key)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Nieprawidłowy agent_id lub api_key".to_string()))?;

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state, agent)))
}

async fn send_msg(socket: &mut WebSocket, msg: &ServerMessage) -> Result<(), axum::Error> {
    let json =
        serde_json::to_string(msg).expect("serializacja ServerMessage nie może się nie udać");
    socket.send(Message::Text(json.into())).await
}

/// Komunikat bezpieczny do pokazania agentowi (bez szczegółów błędów SQL/HTTP).
fn client_message(err: &AppError) -> String {
    match err {
        AppError::Database(_) | AppError::ExternalApi(_) => {
            "Wewnętrzny błąd serwera, spróbuj ponownie".to_string()
        }
        other => other.to_string(),
    }
}

async fn handle_socket(mut socket: WebSocket, state: AppState, agent: Agent) {
    let agent_id = agent.id;
    tracing::info!(agent_id, name = %agent.name, "Agent połączony przez WebSocket");

    // Subskrypcja PRZED wysłaniem zaległych okazji – dzięki temu żadna predykcja nie zginie
    // (w najgorszym razie okazja przyjdzie dwa razy).
    let mut rx = state.ws_tx.subscribe();

    // 1. Powitanie (świeże saldo z bazy)
    let bankroll = betting_exchange::get_bankroll(&state.db, agent_id)
        .await
        .unwrap_or(agent.current_bankroll);
    let welcome = ServerMessage::Welcome {
        agent_id,
        agent_name: agent.name.clone(),
        current_bankroll: bankroll,
    };
    if send_msg(&mut socket, &welcome).await.is_err() {
        return;
    }

    // 2. Zaległe okazje (Late Joiners)
    match session::pending_opportunities(&state.db, agent_id).await {
        Ok(opportunities) => {
            for opportunity in opportunities {
                if send_msg(&mut socket, &opportunity).await.is_err() {
                    return;
                }
            }
        }
        Err(err) => tracing::error!(agent_id, "Nie udało się pobrać zaległych okazji: {}", err),
    }

    // 3. Główna pętla: komendy od agenta  <->  zdarzenia z kanału broadcast
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(frame)) = incoming else { break };
                match frame {
                    Message::Text(text) => {
                        let reply = handle_command(&state, agent_id, text.as_str()).await;
                        if send_msg(&mut socket, &reply).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    // Ping/Pong obsługuje axum automatycznie; ramki binarne ignorujemy
                    _ => {}
                }
            }
            event = rx.recv() => {
                match event {
                    Ok(envelope) if envelope.is_for(agent_id) => {
                        if send_msg(&mut socket, &envelope.message).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(agent_id, skipped, "Agent nie nadąża za kanałem broadcast");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }

    tracing::info!(agent_id, "Agent rozłączony");
}

/// Przetwarza pojedynczą komendę tekstową i zwraca odpowiedź dla agenta.
async fn handle_command(state: &AppState, agent_id: i32, raw: &str) -> ServerMessage {
    let command = match serde_json::from_str::<AgentCommand>(raw) {
        Ok(cmd) => cmd,
        Err(err) => {
            return ServerMessage::Error {
                code: "INVALID_COMMAND".to_string(),
                message: format!("Niepoprawna komenda: {}", err),
            }
        }
    };

    match command {
        AgentCommand::PlaceBet {
            match_id,
            selection,
            stake,
            reasoning,
        } => {
            match betting_exchange::place_bet(
                state, agent_id, match_id, &selection, stake, reasoning,
            )
            .await
            {
                Ok(placed) => ServerMessage::BetResult {
                    match_id,
                    accepted: true,
                    new_bankroll: placed.new_bankroll,
                    message: "Zakład przyjęty pomyślnie".to_string(),
                },
                Err(err) => {
                    if matches!(err, AppError::Database(_)) {
                        tracing::error!(
                            agent_id,
                            match_id,
                            "Błąd przy przyjmowaniu zakładu: {:?}",
                            err
                        );
                    }
                    let bankroll = betting_exchange::get_bankroll(&state.db, agent_id)
                        .await
                        .unwrap_or_default();
                    ServerMessage::BetResult {
                        match_id,
                        accepted: false,
                        new_bankroll: bankroll,
                        message: client_message(&err),
                    }
                }
            }
        }
        AgentCommand::GetBankroll => {
            match betting_exchange::get_bankroll(&state.db, agent_id).await {
                Ok(current_bankroll) => ServerMessage::Bankroll { current_bankroll },
                Err(err) => ServerMessage::Error {
                    code: "INTERNAL_ERROR".to_string(),
                    message: client_message(&err),
                },
            }
        }
    }
}
