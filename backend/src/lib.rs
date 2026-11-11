// src/lib.rs
//! Biblioteka backendu – moduły są wystawione publicznie, aby testy integracyjne
//! z katalogu `tests/` mogły z nich korzystać (`main.rs` to tylko cienki punkt wejścia).
pub mod api;
pub mod config;
pub mod domain;
pub mod error;
pub mod models;
pub mod state;
pub mod ws;
