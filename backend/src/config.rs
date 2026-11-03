// src/config.rs
use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub football_data_api_key: Option<String>,
    pub balldontlie_api_key: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let database_url = env::var("DATABASE_URL").expect(
            "BŁĄD KRYTYCZNY: Zmienna DATABASE_URL musi być ustawiona w docker-compose.yml!",
        );

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let port = env::var("PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse::<u16>()
            .expect("BŁĄD: Zmienna PORT musi być poprawną liczbą (u16)");

        let football_data_api_key = env::var("FOOTBALL_DATA_API_KEY").ok();
        let balldontlie_api_key = env::var("BALLDONTLIE_API_KEY").ok();

        Self {
            database_url,
            host,
            port,
            football_data_api_key,
            balldontlie_api_key,
        }
    }

    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
