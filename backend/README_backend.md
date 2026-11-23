# B.A.N.K.R.U.T. — Backend (`Rust / Axum`)

Centralny serwis backendowy napisany w języku **Rust** (`axum` + `sqlx` + `tokio`), odpowiadający za przechowywanie i synchronizację danych sportowych, przyjmowanie prognoz z silników predykcyjnych, transakcyjną giełdę zakładów, rozgrywkę w czasie rzeczywistym przez WebSocket oraz backtesting historycznych sezonów.

---

## Spis treści

1. [Co jest już zrobione](#1-co-jest-już-zrobione)
2. [Struktura katalogu `backend/`](#2-struktura-katalogu-backend)
3. [Wszystkie endpointy REST API](#3-wszystkie-endpointy-rest-api)
4. [Protokół WebSocket (`/ws/agent`)](#4-protokół-websocket-wsagent)
5. [Zmienne środowiskowe](#5-zmienne-środowiskowe)
6. [Uruchamianie i walidacja kodu](#6-uruchamianie-i-walidacja-kodu)
7. [Dalszy plan rozwoju (Roadmap & TODO)](#7-dalszy-plan-rozwoju-roadmap--todo)

---

## 1. Co jest już zrobione

### Baza danych i migracje (`migrations/`)
* **Schemat PostgreSQL 16** uruchamiany automatycznie przy starcie przez `sqlx::migrate!`:
  * `202611010001_init_sports_core.sql` — tabele `competitions`, `participants`, `matches`, `match_odds`, indeksy po `(status, match_date)`, `(competition_id, season)` oraz indeks `GIN` na kolumnie `JSONB match_details`.
  * `202611010002_init_predictions.sql` — tabele `prediction_models` oraz `predictions` z unikalnością jednej prognozy na parę `(match_id, model_id)`.
  * `202611010003_init_agents_bets.sql` — tabele `agents` (`CHECK (current_bankroll >= 0.00)`) oraz `bets` (maksymalnie jeden zakład agenta na jeden mecz).
  * `202611010004_seed_dev_data.sql` — dane startowe dla 3 sportów (6 meczów, kursy `MARKET_AVG`, 2 modele predykcyjne, 2 agentów testowych).

### Warstwa ETL / Ingest (`src/ingest/`)
* **Cykliczny harmonogram w tle** (`spawn_scheduler`) uruchamiany przy starcie serwera (sterowany przez `INGEST_ENABLED` oraz `INGEST_INTERVAL_MINUTES`), z ręcznym wyzwalaniem przez REST API.
* **Wspólna obsługa HTTP i zapisu (`src/ingest/mod.rs`):** ponawianie zapytań po `429 Too Many Requests` (do 3 razy po 60 s) oraz idempotentny zapis `store_matches` (`INSERT ... ON CONFLICT DO UPDATE`), chroniący zakończone mecze (`FINISHED`) przed nadpisaniem statusem niezakończonym.
* **Piłka nożna (`src/ingest/football/`):** historia i statystyki meczowe (`xG`, strzały, strzały celne, rożne, faule, kartki, wynik HT) z **`football-data.co.uk`** (CSV + `fixtures.csv`), przedmeczowe ratingi Elo (`D - 1`) i prawdopodobieństwa `/Fixtures` z **`clubelo.com`** oraz opcjonalne uzupełnienie bieżącego sezonu z **Football-Data.org v4** (dokładne `utcDate`, kolejki, statusy `LIVE`/`POSTPONED`/`CANCELLED`) dla lig Top 5 (`PL, PD, BL1, SA, FL1`).
* **Koszykówka (`src/ingest/basketball.rs`):** **BALLDONTLIE API (`v1/games`)** z paginacją kursorową i limitem 12 s między stronami (dla darmowego limitu 5 req/min) + opcjonalna **Euroliga** z **TheSportsDB**.
* **Tenis (`src/ingest/tennis.rs`):** archiwalne pliki CSV z mirrora **`Aneeshers/tennis-sackmann-archive`** (ATP/WTA, pomijanie walkowerów `W/O` i `DEF`, rozkładanie rund drabinki na kolejne dni, deterministyczne losowanie strony `home`/`away` na podstawie FNV-1a przeciwko *data leakage*) + bieżące drabinki z **ESPN Scoreboard API** (wyłącznie single, pomijanie zawodników `TBD`).

### Giełda zakładów, rozliczenia i backtester (`src/domain/`)
* **Transakcyjne przyjmowanie zakładów ACID (`betting_exchange.rs`):**
  * Blokada wiersza portfela (`SELECT ... FOR UPDATE`), serializująca równoległe zakłady jednego agenta.
  * Walidacja stawki (`> 0`, maks. 2 miejsca po przecinku, wystarczające saldo), statusu meczu (`SCHEDULED` i przed czasem rozpoczęcia) oraz dostępności kursu `MARKET_AVG`.
  * Wyliczanie wartości oczekiwanej `EV = prob * odds - 1` na podstawie najnowszej predykcji w bazie i zamrożenie kursu `odds_taken`.
* **Rozliczanie kuponów (`settlement.rs`):** automatyczne rozliczanie wszystkich `PENDING` zakładów na mecze `FINISHED`, `CANCELLED` i `POSTPONED` (`WON`, `LOST`, `VOID` ze zwrotem stawki) w jednej transakcji oraz powiadamianie właścicieli kuponów przez WebSocket.
* **Silnik Backtestingu („Wehikuł czasu” — `backtester.rs`):** odtwarza historyczne sezony dzień po dniu (grupowanie po dacie UTC), otwiera w pamięci okno zakładów (`open_backtest_matches`), rozsyła `MATCH_OPPORTUNITY` bez wyniku, zbiera zakłady przez `bet_window_ms` i natychmiast rozlicza dzień. Posiada zabezpieczenie przed równoległym uruchomieniem (`AtomicBool` + RAII guard -> `409 Conflict`) oraz opcję `reset_agents` cofającą poprzednie rozliczone zakłady.

### Warstwa REST API i WebSocket (`src/api/`, `src/ws/`)
* Kompletny router REST pod `/api/v1` oraz diagnostyka `/health`.
* Hub WebSocket pod `/ws/agent` oparty o `tokio::sync::broadcast`, uwierzytelnianie przed upgrade'em (`401 Unauthorized` przy złym `agent_id`/`api_key`), obsługę komend `PLACE_BET` i `GET_BANKROLL` oraz mechanizm **Late Joiners** (`src/ws/session.rs` – wysyłanie zaległych okazji meczowych po nawiązaniu połączenia).
* Jednolita obsługa błędów `AppError` (`src/error.rs`) mapowana na odpowiedzi JSON `{"error": "...", "code": "..."}`.

### Testy i CI/CD
* Zestaw testów w `tests/`: `backtester.rs`, `betting_exchange.rs`, `predictions.rs`, `settlement.rs`, `ingest_football.rs`, `ingest_basketball.rs`, `ingest_tennis.rs`.
* Workflow GitHub Actions (`.github/workflows/backend-docker.yml`): `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --locked` oraz budowanie obrazów Multi-Arch (`linux/amd64`, `linux/arm64`) do `ghcr.io`.

---

## 2. Struktura katalogu `backend/`

```text
backend/
├── Cargo.toml                  # Zależności (axum 0.8, sqlx 0.9, tokio, rust_decimal, reqwest)
├── Cargo.lock                  # Zamrożone wersje zależności
├── Dockerfile                  # Wieloetapowy obraz produkcyjny (rust:1-slim-bookworm -> debian:bookworm-slim)
├── plan.md                     # Archiwalna dokumentacja faz realizacji projektu
├── README.md                   # Ten dokument
├── migrations/                 # Migracje SQLx wykonywane automatycznie przy starcie
│   ├── 202611010001_init_sports_core.sql
│   ├── 202611010002_init_predictions.sql
│   ├── 202611010003_init_agents_bets.sql
│   └── 202611010004_seed_dev_data.sql
├── tests/                      # Testy jednostkowe i domenowe (cargo test)
│   ├── backtester.rs
│   ├── betting_exchange.rs
│   ├── ingest_basketball.rs
│   ├── ingest_football.rs
│   ├── ingest_tennis.rs
│   ├── predictions.rs
│   └── settlement.rs
└── src/
    ├── lib.rs                  # Publiczny interfejs biblioteki dla binarki i testów
    ├── main.rs                 # Punkt wejścia: logowanie, baza, migracje, scheduler ingestu, Axum
    ├── config.rs               # Konfiguracja ze zmiennych środowiskowych
    ├── state.rs                # AppState (PgPool, Config, reqwest::Client, broadcast::Sender, stan backtestu)
    ├── error.rs                # Globalny enum AppError -> odpowiedzi HTTP JSON
    ├── models/
    │   ├── mod.rs
    │   ├── match_model.rs      # Match, MatchOdds, MatchWithDetailsDto, enum MatchDetails
    │   ├── participant.rs      # Competition, Participant
    │   ├── prediction.rs       # PredictionModel, Prediction, CreatePredictionDto
    │   └── bet.rs              # Agent, Bet
    ├── api/
    │   ├── mod.rs              # Główny router /api/v1
    │   ├── matches.rs          # GET /matches, GET /matches/{id}
    │   ├── agents.rs           # GET/POST /agents, /leaderboard, /{id}/bets
    │   ├── predictions.rs      # POST /predictions + walidacja + broadcast WS
    │   ├── ingest.rs           # POST /ingest/{sport}
    │   └── simulation.rs       # POST /simulation/settle, POST /simulation/backtest
    ├── ws/
    │   ├── mod.rs              # Router /ws
    │   ├── protocol.rs         # Kontrakt JSON: ServerMessage, AgentCommand, WsEnvelope
    │   ├── handler.rs          # Obsługa połączenia WS (tokio::select!)
    │   └── session.rs          # Budowanie MATCH_OPPORTUNITY i dosłanie zaległych okazji (Late Joiners)
    ├── domain/
    │   ├── mod.rs
    │   ├── betting_exchange.rs # Logika zawierania zakładów (ACID) i obliczania EV
    │   ├── settlement.rs       # Rozliczanie meczów i publikacja MATCH_SETTLED
    │   └── backtester.rs       # Symulator historyczny dzień po dniu
    └── ingest/
        ├── mod.rs              # Harmonogram, retry 429, transakcyjny store_matches
        ├── football/           # football-data.co.uk + Football-Data.org + clubelo.com (Top 5 lig)
        │   ├── mod.rs          # Orkiestracja sync() i Football-Data.org v4
        │   ├── teams.rs        # Normalizacja nazw drużyn, slugi ClubElo, mapa lig
        │   ├── football_data_uk.rs # Klient i parser CSV football-data.co.uk + fixtures.csv
        │   └── clubelo.rs      # Parser clubelo.com (D - 1 z wykresu i /Fixtures)
        ├── basketball.rs       # BALLDONTLIE + TheSportsDB
        └── tennis.rs           # Mirror Sackmanna + ESPN Scoreboard
```

---

## 3. Wszystkie endpointy REST API

### Podsumowanie tras

| Metoda | Ścieżka | Status sukcesu | Opis |
| :--- | :--- | :--- | :--- |
| `GET` | `/health` | `200 OK` | Stan serwera, połączenia z bazą i liczba subskrybentów WS |
| `GET` | `/api/v1/matches` | `200 OK` | Lista meczów z opcjonalnym filtrowaniem (`sport`, `status`, `season`) |
| `GET` | `/api/v1/matches/{id}` | `200 OK` | Szczegóły pojedynczego meczu z kursami `MARKET_AVG` i `match_details` |
| `GET` | `/api/v1/agents` | `200 OK` | Lista zarejestrowanych agentów (bez `api_key`) |
| `POST` | `/api/v1/agents` | `201 Created` | Rejestracja nowego agenta |
| `GET` | `/api/v1/agents/leaderboard` | `200 OK` | Ranking agentów posortowany malejąco wg `current_bankroll` |
| `GET` | `/api/v1/agents/{id}/bets` | `200 OK` | Historia zakładów wybranego agenta (od najnowszych) |
| `POST` | `/api/v1/predictions` | `201 Created` | Zapis/aktualizacja predykcji modelu ML i broadcast okazji na WS |
| `POST` | `/api/v1/ingest/{sport}` | `202 Accepted` | Ręczne uruchomienie importu danych sportowych w tle |
| `POST` | `/api/v1/simulation/settle` | `200 OK` | Rozliczenie wszystkich nierozliczonych zakładów na zakończone mecze |
| `POST` | `/api/v1/simulation/backtest` | `202 Accepted` | Uruchomienie symulacji historycznej w tle |

---

### Szczegóły endpointów

#### `GET /health`
Sprawdza połączenie z PostgreSQL (`SELECT 1`) i zwraca liczbę aktywnych odbiorców kanału broadcast WebSocket.
```json
{
  "status": "ok",
  "database": "connected",
  "connected_ws_agents": 2
}
```

#### `GET /api/v1/matches`
Zwraca listę meczów (`MatchWithDetailsDto`) posortowaną rosnąco po `match_date` wraz z nazwami uczestników i kursami `MARKET_AVG`.
* **Opcjonalne parametry query:**
  * `sport` — np. `FOOTBALL`, `BASKETBALL`, `TENNIS` (wielkość liter bez znaczenia)
  * `status` — np. `SCHEDULED`, `LIVE`, `FINISHED`, `POSTPONED`, `CANCELLED`
  * `season` — np. `2024-2025`, `2026`

#### `GET /api/v1/matches/{id}`
Zwraca szczegóły pojedynczego meczu (`MatchWithDetailsDto`) lub `404 Not Found`, jeśli mecz nie istnieje.

#### `GET /api/v1/agents`
Zwraca listę wszystkich zarejestrowanych agentów posortowaną rosnąco po `id` (pole `api_key` ma `#[serde(skip_serializing)]`, więc nie jest zwracane).

#### `POST /api/v1/agents`
Rejestruje nowego agenta. Zwraca `201 Created` (to jedyny moment, w którym zwracany jest `api_key`).
* **Request body:**
  ```json
  {
    "name": "Agent_Kelly_v1",
    "api_key": "tajny_klucz",
    "strategy_description": "Ułamkowe kryterium Kelly'ego",
    "initial_bankroll": 1000.00
  }
  ```
  *(Pola `strategy_description` i `initial_bankroll` są opcjonalne; domyślny kapitał początkowy to `1000.00`.)*

#### `GET /api/v1/agents/leaderboard`
Zwraca ranking agentów posortowany malejąco po `current_bankroll`:
* Pola: `id`, `name`, `strategy_description`, `initial_bankroll`, `current_bankroll`, `total_profit`, `roi_pct`, `total_bets`, `won_bets`.

#### `GET /api/v1/agents/{id}/bets`
Zwraca pełną listę zakładów (`Bet`) postawionych przez agenta o podanym `id`, posortowaną malejąco po `placed_at`.

#### `POST /api/v1/predictions`
Przyjmuje predykcję z silnika ML (automatycznie rejestruje model po parze `(model_name, model_version)` i wykonuje upsert po `(match_id, model_id)`). Jeśli mecz ma status `SCHEDULED`, wysyła zdarzenie `MATCH_OPPORTUNITY` do wszystkich agentów podpiętych przez WebSocket.
* **Walidacja:** każde prawdopodobieństwo w `[0, 1]`, suma `prob_home + prob_draw + prob_away` w przedziale `1.0 ± 0.01`, `model_name` niepusty.
* **Request body:**
  ```json
  {
    "match_id": 4,
    "model_name": "XGBoost_Football",
    "model_version": "1.0",
    "prob_home": 0.34,
    "prob_draw": 0.28,
    "prob_away": 0.38,
    "extra_metrics": { "expected_goals_home": 1.25, "expected_goals_away": 1.42 }
  }
  ```
  *(Pola `model_version`, `prob_draw` i `extra_metrics` są opcjonalne; brak `prob_draw` oznacza `0`.)*

#### `POST /api/v1/ingest/{sport}`
Uruchamia w tle (`tokio::spawn`) ręczny import danych sportowych i od razu zwraca `202 Accepted`.
* **Parametr ścieżki `{sport}`:** `football`, `basketball`, `tennis` albo `all` (inna wartość zwraca `400 Bad Request`).
* **Opcjonalny parametr query:** `?history=true` (domyślnie `false` — pobiera również sezony historyczne).
* **Odpowiedź (`202 Accepted`):**
  ```json
  {
    "started": ["football", "basketball", "tennis"],
    "history": true
  }
  ```

#### `POST /api/v1/simulation/settle`
Rozlicza wszystkie zakłady ze statusem `PENDING` dla meczów o statusie `FINISHED`, `CANCELLED` lub `POSTPONED` i publikuje zdarzenia `MATCH_SETTLED` do odpowiednich agentów.
* **Odpowiedź (`200 OK`):**
  ```json
  { "settled_bets": 12 }
  ```

#### `POST /api/v1/simulation/backtest`
Uruchamia symulację historyczną w tle (`202 Accepted`). Jeśli inny backtest aktualnie trwa, zwraca `409 Conflict`. Jeśli dla wybranego sezonu/sportu brak meczów `FINISHED` z wyznaczonym zwycięzcą i kursami `MARKET_AVG`, zwraca `404 Not Found`.
* **Request body:**
  ```json
  {
    "season": "2025-2026",
    "sport": "FOOTBALL",
    "bet_window_ms": 500,
    "reset_agents": false
  }
  ```
  *(Tylko `season` jest wymagane; `bet_window_ms` domyślnie `500`, maksymalnie `30000`.)*

### Format błędów HTTP
Każdy błąd aplikacji (`AppError`) zwracany jest jako JSON w postaci:
```json
{
  "error": "Czytelny opis błędu",
  "code": "BAD_REQUEST"
}
```
Dostępne kody: `DATABASE_ERROR` (`500`), `EXTERNAL_API_ERROR` (`502`), `EXTERNAL_DATA_ERROR` (`502`), `NOT_FOUND` (`404`), `INSUFFICIENT_FUNDS` (`422`), `BET_REJECTED` (`409`), `BAD_REQUEST` (`400`), `UNAUTHORIZED` (`401`), `CONFLICT` (`409`).

---

## 4. Protokół WebSocket (`/ws/agent`)

Połączenie wymaga parametrów uwierzytelniających w query stringu (aktywny agent z bazy `is_active = TRUE`):
```text
ws://localhost:3000/ws/agent?agent_id=1&api_key=secret_kelly_token_1
```
Błędne dane odrzucają handshake kodem `401 Unauthorized`. Po połączeniu serwer wysyła `WELCOME`, a następnie zaległe `MATCH_OPPORTUNITY` dla meczów `SCHEDULED` posiadających predykcje, na które ten agent jeszcze nie obstawiał.

### Wiadomości z serwera do agenta (`ServerMessage`)
Format koperty: `{"event": "...", "data": {...}}`.
* **`WELCOME`** *(do danego agenta)* — `{ "agent_id", "agent_name", "current_bankroll" }`
* **`MATCH_OPPORTUNITY`** *(broadcast)* — `{ "match_id", "sport", "competition", "home_name", "away_name", "odds_home", "odds_draw", "odds_away", "predictions": [...] }`
* **`BET_RESULT`** *(do danego agenta)* — `{ "match_id", "accepted", "new_bankroll", "message" }`
* **`MATCH_SETTLED`** *(do danego agenta)* — `{ "match_id", "winner", "bet_status", "profit_loss", "current_bankroll" }`
* **`BANKROLL`** *(do danego agenta)* — `{ "current_bankroll" }`
* **`BACKTEST_STARTED`** *(broadcast)* — `{ "season", "sport", "total_matches", "total_days" }`
* **`BACKTEST_FINISHED`** *(broadcast)* — `{ "season", "total_matches", "settled_bets" }`
* **`ERROR`** *(do danego agenta)* — `{ "code", "message" }`

### Komendy od agenta do serwera (`AgentCommand`)
* **Postawienie zakładu:**
  ```json
  {
    "action": "PLACE_BET",
    "payload": {
      "match_id": 4,
      "selection": "AWAY",
      "stake": 50.00,
      "reasoning": "Dodatnie EV wg modelu XGBoost"
    }
  }
  ```
* **Sprawdzenie salda:**
  ```json
  { "action": "GET_BANKROLL" }
  ```

---

## 5. Zmienne środowiskowe

Wszystkie ustawienia wczytywane są w `src/config.rs`:

| Zmienna | Wymagana | Domyślna wartość | Opis |
| :--- | :--- | :--- | :--- |
| `DATABASE_URL` | Tak | — | Adres połączenia z PostgreSQL |
| `HOST` | Nie | `0.0.0.0` | Adres nasłuchu serwera Axum |
| `PORT` | Nie | `3000` | Port serwera HTTP/WS |
| `RUST_LOG` | Nie | `backend=debug,tower_http=info,sqlx=warn` | Poziom logowania (`tracing`) |
| `FOOTBALL_DATA_API_KEY` | Nie | `None` | Klucz do Football-Data.org (opcjonalne uzupełnienie bieżącego sezonu piłki nożnej; bez niego działa `football-data.co.uk` + `clubelo.com`) |
| `BALLDONTLIE_API_KEY` | Nie | `None` | Klucz do BALLDONTLIE (NBA) |
| `THESPORTSDB_EUROLEAGUE_ID` | Nie | `None` | ID ligi Euroligi w TheSportsDB (bez niego Euroliga jest pomijana) |
| `INGEST_ENABLED` | Nie | `true` | Czy uruchamiać automatyczny harmonogram ETL w tle |
| `INGEST_INTERVAL_MINUTES` | Nie | `60` | Odstęp (w minutach) między cyklami ingestu |
| `INGEST_FOOTBALL_SOURCE` | Nie | `auto` | Źródło meczów piłki nożnej: `auto`, `football_data_uk`, `football_data` |
| `INGEST_FOOTBALL_SEASONS` | Nie | `[]` *(2 ostatnie sezony)* | Lista sezonów historycznych piłki nożnej, np. `2023-24,2024-25`, `2023-2024` lub `2023` |
| `INGEST_FOOTBALL_LEAGUES` | Nie | `PL` | Lista kodów lig Top 5 rozdzielona przecinkami (`PL,PD,BL1,SA,FL1`) |
| `INGEST_FOOTBALL_CLUBELO_ENABLED` | Nie | `true` | Czy wzbogacać mecze piłkarskie o przedmeczowe ratingi ClubElo (`D - 1`) |
| `INGEST_NBA_SEASONS` | Nie | `[]` *(2 ostatnie sezony)* | Lista lat startu sezonów NBA, np. `2023,2024` |
| `INGEST_NBA_SOURCE` | Nie | `auto` | Źródło meczów NBA: `auto`, `nba_stats`, `balldontlie` |
| `NBA_API_PROXY` | Nie | `None` | Opcjonalny adres proxy HTTP/HTTPS dla `stats.nba.com` (np. `http://user:pass@host:port`) |
| `NBA_STATS_TIMEOUT_SECS` | Nie | `90` | Limit czasu (w sekundach) pojedynczego zapytania do `stats.nba.com` |
| `NBA_STATS_DELAY_MS` | Nie | `1500` | Pauza (w milisekundach) między zapytaniami do `stats.nba.com` |
| `INGEST_TENNIS_YEARS` | Nie | `[]` *(2 ostatnie lata)* | Lista lat dla archiwum Sackmanna, np. `2023,2024` |
| `INGEST_TENNIS_TOURS` | Nie | `ATP` | Toury tenisowe do synchronizacji (`ATP`, `WTA`) |

---

## 6. Uruchamianie i walidacja kodu

### Lokalna kompilacja i testy (zgodne z CI)
```bash
cd backend
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

### Uruchomienie przez Docker Compose
Z poziomu głównego katalogu repozytorium:
```bash
docker compose up -d
```

---

## 7. Dalszy plan rozwoju (Roadmap & TODO)

### 1. Niezawodność i deduplikacja w warstwie Ingest / ETL
* [ ] **Zaawansowana deduplikacja meczów w `store_matches` (`src/ingest/mod.rs`):** Przed `INSERT INTO matches` wyszukiwać istniejący rekord po identyfikatorach w `external_ids` lub po parze `(competition_id, season, home_participant_id, away_participant_id)` w tym samym dniu kalendarzowym (`(match_date AT TIME ZONE 'UTC')::date`). Pozwoli to wykonywać `UPDATE` (wraz z aktualizacją godziny `match_date`) przy zmianach terminarza lub rozbieżnościach godzinowych między źródłami.
* [ ] **Poprawa parsowania zmiennych listowych w `src/config.rs`:** Odfiltrowanie pustych stringów w `env_list` (aby brak lub pusta wartość zmiennej nie tworzyły jednoelementowego `vec![""]`) oraz traktowanie pustych kluczy API jako `None`.
* [ ] **Separacja sezonów archiwalnych od bieżących:** Ograniczenie domyślnych źródeł historycznych wyłącznie do zamkniętych sezonów, aby nie nakładały się na dane live.

### 2. Rozszerzenie danych i dyscyplin sportowych
* [ ] **Obsługa pełnych 5 dyscyplin sportowych:** Dodanie kolejnych 2 dyscyplin do migracji (`CHECK` w `competitions` / `participants` / `prediction_models`), wariantów `MatchDetails` oraz nowych workerów w `src/ingest/`.
* [ ] **Wzbogacenie statystyk koszykówki (`src/ingest/basketball.rs`):** Odczytywanie punktów w kwartach (`quarters`) z odpowiedzi `balldontlie.io` oraz opcjonalna integracja historycznych statystyk meczowych (`leaguegamelog` z `stats.nba.com` / `nba_api`).
* [ ] **Integracja web scrapera kursów i automatyczne wyliczanie kursów:** Zasilanie tabeli `match_odds` danymi ze scrapera oraz wyliczanie kursów bazowych na podstawie predykcji modeli ML (marża + kryterium Kelly'ego).

### 3. Rozwój giełdy, typerów i backtestera
* [ ] **Konta graczy (typerów) i ranking użytkowników:** Rozszerzenie modelu o graczy ludzkich (obok agentów AI), endpointy obstawiania przez REST dla aplikacji frontendowej oraz osobny ranking najlepszych typerów.
* [ ] **Wirtualna symulacja portfela w backtesterze:** Opcja uruchamiania backtestu w izolacji (bez modyfikowania bieżącego salda produkcyjnego agentów).
* [ ] **Testy integracyjne z bazą PostgreSQL:** Rozbudowa testów w `tests/` o scenariusze bazodanowe (transakcje `place_bet`, rozliczenia `settle_matches`, `store_matches`).
