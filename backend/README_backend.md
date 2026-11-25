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
  * `202611260001_add_cs2_sport.sql` — rozszerzenie ograniczeń `CHECK (sport IN (...))` o dyscyplinę `CS2`.

### Warstwa ETL / Ingest (`src/ingest/`)
* **Cykliczny harmonogram w tle** (`spawn_scheduler`) uruchamiany przy starcie serwera (sterowany przez `INGEST_ENABLED` oraz `INGEST_INTERVAL_MINUTES`), z ręcznym wyzwalaniem przez REST API.
* **Wspólna obsługa HTTP, normalizacja i zapis (`src/ingest/mod.rs`):**
  * `fetch_text` (ponawianie zapytań po `429 Too Many Requests` — do 3 razy po 60 s) oraz `fetch_text_with_timeout` (własny limit czasu i ponawianie z rosnącym opóźnieniem przy timeoutcie, błędzie połączenia, `429` i `5xx`).
  * Wspólna normalizacja etykiet sezonów (`normalize_season_label`, `short_season_label`, `full_season_label`).
  * Dwuetapowa deduplikacja i bezstratne scalanie w `store_matches`:
    * **Krok A:** dopasowanie po identyfikatorach zewnętrznych w `external_ids` (`@>` JSONB).
    * **Krok B:** dopasowanie po parze drużyn i oknie ±36 h (dla NBA obsługa przesunięcia strefy USA/UTC przez +23 h dla północy i okno ±12 h, symetryczne dopasowanie meczów na terenie neutralnym oraz ochrona przed scalaniem różnych spotkań przy konfliktujących `external_ids` lub różnych wynikach).
    * Wybór dokładniejszego czasu `match_date` (godzina różna od `00:00:00` UTC wygrywa z samą datą, a `utcDate` z Football-Data.org ma pierwszeństwo przed CSV), bezstratne scalanie `MatchDetails::Football` (wynik HT, kolejka, `FootballTeamStats` i `FootballPreMatchElo`) oraz `MatchDetails::Basketball` (kwarty z BALLDONTLIE + statystyki `home_stats`/`away_stats` i `season_type` ze `stats.nba.com`), ochrona statusu `FINISHED` przed cofnięciem (z dopuszczeniem wzbogacenia `match_details`) oraz fallback `INSERT ... ON CONFLICT DO UPDATE`.
* **Piłka nożna — Top 5 lig Europy (`src/ingest/football/`):** obsługa lig `PL`, `PD`, `BL1`, `SA`, `FL1` (`INGEST_FOOTBALL_LEAGUES`) z trzema źródłami sterowanymi przez `INGEST_FOOTBALL_SOURCE` (`auto`, `football_data_uk`, `football_data`):
  * **`football-data.co.uk` (`football_data_uk.rs`):** historia i bieżący sezon CSV (`E0`, `SP1`, `D1`, `I1`, `F1`) wraz ze statystykami meczowymi (`xG`, strzały, strzały celne, rożne, faule, kartki, wynik HT) oraz terminarz `fixtures.csv` jako fallback bez klucza API.
  * **Football-Data.org v4 (`football/mod.rs`):** opcjonalne uzupełnienie bieżącego sezonu o oficjalne `utcDate`, numery kolejek i statusy `LIVE`/`POSTPONED`/`CANCELLED`.
  * **`clubelo.com` (`clubelo.rs`):** przedmeczowe ratingi Elo z dnia `D - 1` (wyciągane z JSON-a historii klubu przeciwko *data leakage*) oraz aktualne rankingi i prawdopodobieństwa `1/X/2` ze strony `/Fixtures` dla meczów `SCHEDULED` (`INGEST_FOOTBALL_CLUBELO_ENABLED`).
  * Kanoniczna normalizacja nazw drużyn i slugów ClubElo dla wszystkich 5 lig (`teams.rs`).
* **Koszykówka (`src/ingest/basketball.rs`):** integracja trzech źródeł sterowana zmienną `INGEST_NBA_SOURCE` (`auto`, `nba_stats`, `balldontlie`):
  * **`stats.nba.com/stats/teamgamelogs` (`Base` + `Advanced`):** dane dla `Regular Season` i `Playoffs` (konfigurowalny `NBA_STATS_TIMEOUT_SECS` dom. 90 s, pauza `NBA_STATS_DELAY_MS` dom. 1500 ms, opcjonalne proxy `NBA_API_PROXY`, retry `3 s / 10 s / 30 s`, dekompresja `gzip`/`deflate`/`brotli`), łączenie po `(GAME_ID, TEAM_ID)` do `BasketballTeamStats` (21 metryk box-score + 9 metryk zaawansowanych), wykrywanie dogrywki (`MIN > 50.0`), obsługa meczów na terenie neutralnym (oba wiersze z `" @ "`) oraz normalizacja nazw drużyn (`LA Clippers` → `Los Angeles Clippers` i nazwy historyczne).
  * **BALLDONTLIE API (`v1/games`):** terminarz (`SCHEDULED`), mecze `LIVE`, dokładny `datetime` UTC, punkty w kwartach i dogrywkach (`home_q1..4`, `home_ot1..3`, `visitor_q1..4`, `visitor_ot1..3`), obsługa `postponed` i jawnego pola `season`, paginacja kursorowa z pauzą 12 s (dla darmowego limitu 5 req/min) oraz fallback dla historii w trybie `auto`.
  * **Opcjonalna Euroliga z TheSportsDB** (`THESPORTSDB_EUROLEAGUE_ID`).
* **Tenis (`src/ingest/tennis.rs`):** archiwalne pliki CSV z mirrora **`Aneeshers/tennis-sackmann-archive`** (ATP/WTA, pomijanie walkowerów `W/O` i `DEF`, rozkładanie rund drabinki na kolejne dni, deterministyczne losowanie strony `home`/`away` na podstawie FNV-1a przeciwko *data leakage*) + bieżące drabinki z **ESPN Scoreboard API** (wyłącznie single, pomijanie zawodników `TBD`).
* **Counter-Strike 2 (`src/ingest/cs2/`):** hybryda dwóch źródeł:
  * **Kaggle CSV (`ektarr/counter-strike-pro-matches`, `kaggle_csv.rs`):** historia i wzbogacanie meczów o dokładne rundy map (`is_total == false`), 5-osobowe składy i statystyki graczy (`kills`, `deaths`, `assists`, `adr`, `kast`, `kddiff`); przy podanym `KAGGLE_API_TOKEN` automatycznie wykrywa nowe wersje datasetu przez Kaggle API i pobiera plik z obsługą ZIP do `data/cs2_games.csv`.
  * **PandaScore Free (`/csgo/*`, `pandascore.rs`):** terminarz (`SCHEDULED`), mecze `LIVE`, wyniki końcowe z ostatnich 14 dni, statusy `CANCELLED`/`POSTPONED` oraz składy turniejowe (`/tournaments/{id}/rosters`).
  * Normalizacja aliasów drużyn CS2 (`teams.rs`) oraz symetryczne, bezstratne scalanie `MatchDetails::Cs2` w `store_matches`.

### Giełda zakładów, kursy, rozliczenia i backtester (`src/domain/`)
* **Wyliczanie kursów (`odds.rs`):** automatyczne przeliczanie prawdopodobieństw na kursy bukmacherskie `MARKET_AVG` z konfigurowalną marżą `BOOKMAKER_MARGIN` (domyślnie `0.05`, wzór $O = \frac{1}{p(1 + m)}$, przycinanie do `[1.01, 100.00]`, obsługa rynku `1X2` dla piłki i `12` dla koszykówki/tenisa) przy każdym `POST /api/v1/predictions` — zarówno dla `SCHEDULED`, jak i `FINISHED` (pod backtester).
* **Transakcyjne przyjmowanie zakładów ACID (`betting_exchange.rs`):**
  * Blokada wiersza portfela (`SELECT ... FOR UPDATE`), serializująca równoległe zakłady jednego agenta.
  * Walidacja stawki (`> 0`, maks. 2 miejsca po przecinku, wystarczające saldo), statusu meczu (`SCHEDULED` i przed czasem rozpoczęcia) oraz dostępności kursu `MARKET_AVG`.
  * Wyliczanie wartości oczekiwanej `EV = prob * odds - 1` na podstawie średniej (`AVG`) z predykcji wszystkich modeli dla danego meczu oraz zamrożenie kursu `odds_taken`.
* **Rozliczanie kuponów (`settlement.rs`):** automatyczne rozliczanie wszystkich `PENDING` zakładów na mecze ze statusem `FINISHED` i `CANCELLED` (`WON`, `LOST`, `VOID` ze zwrotem stawki) — każdy mecz w osobnej transakcji (`FOR UPDATE` po `agent_id` ograniczające ryzyko deadlocków) — oraz powiadamianie właścicieli kuponów przez WebSocket.
* **Silnik Backtestingu („Wehikuł czasu” — `backtester.rs`):** odtwarza historyczne sezony dzień po dniu (grupowanie po dacie UTC) na w pełni izolowanych, wirtualnych portfelach w pamięci serwera (`BacktestStore`, bez modyfikowania produkcyjnych tabel `agents` i `bets`). Otwiera okno zakładów dla danego dnia, rozsyła `MATCH_OPPORTUNITY` bez wyniku, zbiera zakłady przez `bet_window_ms`, natychmiast rozlicza wirtualne kupony i zapisuje raport przebiegu (`leaderboard`, kupony z `match_date` i `bankroll_after`). Posiada zabezpieczenie przed równoległym uruchomieniem (`AtomicBool` + RAII guard -> `409 Conflict`).

### Warstwa REST API i WebSocket (`src/api/`, `src/ws/`)
* Kompletny router REST pod `/api/v1` (w tym paginacja i zakres dat w `GET /api/v1/matches`: `limit`, `offset`, `date_from`, `date_to`), diagnostyka `/health` oraz interaktywna dokumentacja **Swagger UI** pod `/docs` i specyfikacja OpenAPI 3 pod `/api-docs/openapi.json` (`utoipa` + `utoipa-swagger-ui`).
* Hub WebSocket pod `/ws/agent` oparty o `tokio::sync::broadcast`, uwierzytelnianie przed upgrade'em (`401 Unauthorized` przy złym `agent_id`/`api_key`), obsługę komend `PLACE_BET` i `GET_BANKROLL` oraz mechanizm **Late Joiners** (`src/ws/session.rs` – wysyłanie zaległych okazji meczowych po nawiązaniu połączenia).
* Jednolita obsługa błędów `AppError` (`src/error.rs`) mapowana na odpowiedzi JSON `{"error": "...", "code": "..."}`.

### Testy i CI/CD
* Zestaw testów w `tests/`: `backtester.rs`, `betting_exchange.rs`, `openapi.rs`, `predictions.rs`, `settlement.rs`, `ingest_football.rs`, `ingest_basketball.rs`, `ingest_tennis.rs`, `ingest_cs2.rs`, `logging.rs` oraz `tests/fixtures/` (`nba_base_sample.json`, `nba_advanced_sample.json`, `football_data_uk_e0_sample.csv`, `clubelo_team_page_sample.html`, `clubelo_home_and_fixtures_sample.html`, `cs2_kaggle_sample.csv`) — obejmujący m.in. `football-data.co.uk`, `clubelo.com` (`D - 1`), `stats.nba.com`, BALLDONTLIE, Kaggle CS2 CSV, PandaScore, deduplikację/scalanie, wyliczanie kursów i kontrakt OpenAPI.
* Workflow GitHub Actions:
  * `.github/workflows/backend-check.yml` (`pull_request` na `dev`/`prod` oraz `workflow_call`): `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --locked`.
  * `.github/workflows/backend-build.yml` (`push` na `dev`/`prod`): wywołuje `backend-check.yml`, buduje natywne obrazy Multi-Arch (`linux/amd64`, `linux/arm64`) oraz scala manifest w `ghcr.io` pod tagi `:dev`, `:prod` i `:latest` dla `prod`.

---

## 2. Struktura katalogu `backend/`

```text
backend/
├── Cargo.toml                  # Zależności (axum 0.8, sqlx 0.9.1, tokio, rust_decimal, reqwest, utoipa)
├── Cargo.lock                  # Zamrożone wersje zależności
├── .dockerignore               # Wykluczenia kontekstu budowania obrazu Docker
├── Dockerfile                  # Wieloetapowy obraz produkcyjny (rust:1-slim-bookworm -> debian:bookworm-slim)
├── README_backend.md           # Ten dokument
├── migrations/                 # Migracje SQLx wykonywane automatycznie przy starcie
│   ├── 202611010001_init_sports_core.sql
│   ├── 202611010002_init_predictions.sql
│   ├── 202611010003_init_agents_bets.sql
│   ├── 202611010004_seed_dev_data.sql
│   └── 202611260001_add_cs2_sport.sql
├── tests/                      # Testy jednostkowe i domenowe (cargo test)
│   ├── fixtures/
│   │   ├── clubelo_home_and_fixtures_sample.html
│   │   ├── clubelo_team_page_sample.html
│   │   ├── cs2_kaggle_sample.csv
│   │   ├── football_data_uk_e0_sample.csv
│   │   ├── nba_advanced_sample.json
│   │   └── nba_base_sample.json
│   ├── backtester.rs
│   ├── betting_exchange.rs
│   ├── ingest_basketball.rs
│   ├── ingest_cs2.rs
│   ├── ingest_football.rs
│   ├── ingest_tennis.rs
│   ├── logging.rs
│   ├── openapi.rs
│   ├── predictions.rs
│   └── settlement.rs
└── src/
    ├── lib.rs                  # Publiczny interfejs biblioteki dla binarki i testów
    ├── main.rs                 # Punkt wejścia: logowanie, baza, migracje, scheduler ingestu, Axum
    ├── config.rs               # Konfiguracja ze zmiennych środowiskowych
    ├── state.rs                # AppState (PgPool, Config, http_client, nba_stats_client, ws_tx, stan backtestu)
    ├── error.rs                # Globalny enum AppError -> odpowiedzi HTTP JSON
    ├── models/
    │   ├── mod.rs
    │   ├── match_model.rs      # Match, MatchOdds, MatchWithDetailsDto, FootballTeamStats, FootballPreMatchElo, BasketballTeamStats, MatchDetails
    │   ├── participant.rs      # Competition, Participant
    │   ├── prediction.rs       # PredictionModel, Prediction, CreatePredictionDto
    │   └── bet.rs              # Agent, Bet
    ├── api/
    │   ├── mod.rs              # Główny router /api/v1
    │   ├── docs.rs             # Swagger UI (/docs) i specyfikacja OpenAPI 3 (/api-docs/openapi.json)
    │   ├── health.rs           # GET /health
    │   ├── matches.rs          # GET /matches (filtry, paginacja, zakres dat), GET /matches/{id}
    │   ├── agents.rs           # GET/POST /agents, /leaderboard, /{id}/bets
    │   ├── predictions.rs      # POST /predictions + walidacja + wyliczanie MARKET_AVG + broadcast WS
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
    │   ├── odds.rs             # Wyliczanie kursów MARKET_AVG z prawdopodobieństw i marży
    │   ├── settlement.rs       # Rozliczanie meczów i publikacja MATCH_SETTLED
    │   └── backtester.rs       # Symulator historyczny dzień po dniu
    └── ingest/
        ├── mod.rs              # Harmonogram, retry 429/timeout/5xx, deduplikacja i scalanie w store_matches
        ├── football/           # Top 5 lig Europy
        │   ├── mod.rs          # Orkiestracja sync() i Football-Data.org v4
        │   ├── teams.rs        # Normalizacja nazw drużyn, slugi ClubElo, mapa lig
        │   ├── football_data_uk.rs # Parser CSV football-data.co.uk i fixtures.csv
        │   └── clubelo.rs      # Parser clubelo.com (D - 1 i /Fixtures)
        ├── basketball.rs       # stats.nba.com (Base+Advanced) + BALLDONTLIE + TheSportsDB
        ├── tennis.rs           # Mirror Sackmanna + ESPN Scoreboard
        └── cs2/                # Counter-Strike 2 (hybryda)
            ├── mod.rs          # Orkiestracja sync()
            ├── kaggle_csv.rs   # Pobieranie z kontrolą wersji (ZIP/CSV) i parser Kaggle CSV
            ├── pandascore.rs   # Klient PandaScore Free (/csgo/*)
            └── teams.rs        # Normalizacja aliasów drużyn CS2
```

---

## 3. Wszystkie endpointy REST API

### Podsumowanie tras

| Metoda | Ścieżka | Status sukcesu | Opis |
| :--- | :--- | :--- | :--- |
| `GET` | `/docs` | `200 OK` | Interaktywna dokumentacja Swagger UI (OpenAPI 3) |
| `GET` | `/api-docs/openapi.json` | `200 OK` | Specyfikacja OpenAPI 3 w formacie JSON |
| `GET` | `/health` | `200 OK` | Stan serwera, połączenia z bazą i liczba subskrybentów WS |
| `GET` | `/api/v1/matches` | `200 OK` | Lista meczów z opcjonalnym filtrowaniem (`sport`, `status`, `season`), paginacją (`limit`, `offset`) i zakresem dat (`date_from`, `date_to`) |
| `GET` | `/api/v1/matches/{id}` | `200 OK` | Szczegóły pojedynczego meczu z kursami `MARKET_AVG` i `match_details` |
| `GET` | `/api/v1/agents` | `200 OK` | Lista zarejestrowanych agentów (bez `api_key`) |
| `POST` | `/api/v1/agents` | `201 Created` | Rejestracja nowego agenta |
| `GET` | `/api/v1/agents/leaderboard` | `200 OK` | Ranking agentów posortowany malejąco wg `current_bankroll` |
| `GET` | `/api/v1/agents/{id}/bets` | `200 OK` | Historia zakładów wybranego agenta (od najnowszych) |
| `POST` | `/api/v1/predictions` | `201 Created` | Zapis/aktualizacja predykcji modelu ML, wyliczenie kursów `MARKET_AVG` i broadcast okazji na WS |
| `POST` | `/api/v1/ingest/{sport}` | `202 Accepted` | Ręczne uruchomienie importu danych sportowych w tle |
| `POST` | `/api/v1/simulation/settle` | `200 OK` | Rozliczenie wszystkich nierozliczonych zakładów na zakończone mecze |
| `POST` | `/api/v1/simulation/backtest` | `202 Accepted` | Uruchomienie symulacji historycznej w tle na izolowanych wirtualnych portfelach |
| `GET` | `/api/v1/simulation/backtest` | `200 OK` | Pełny raport aktualnego lub ostatniego przebiegu backtestu |
| `GET` | `/api/v1/simulation/backtest/runs` | `200 OK` | Lista zapisanych w pamięci przebiegów backtestu (od najnowszego) |
| `GET` | `/api/v1/simulation/backtest/runs/{run_id}` | `200 OK` | Pełny raport wskazanego przebiegu backtestu |
| `GET` | `/api/v1/simulation/backtest/runs/{run_id}/leaderboard` | `200 OK` | Ranking wirtualnych portfeli agentów dla wskazanego przebiegu |
| `GET` | `/api/v1/simulation/backtest/runs/{run_id}/bets` | `200 OK` | Wirtualne kupony postawione w danym przebiegu (opcjonalnie `?agent_id=...`) |

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

#### `GET /docs` oraz `GET /api-docs/openapi.json`
Interaktywna dokumentacja Swagger UI (`/docs`) ładująca specyfikację OpenAPI 3 z `/api-docs/openapi.json`.

#### `GET /api/v1/matches`
Zwraca listę meczów (`MatchWithDetailsDto`) posortowaną rosnąco po `match_date`, a następnie `id` (`ORDER BY m.match_date ASC, m.id ASC`), wraz z nazwami uczestników i kursami `MARKET_AVG`.
* **Opcjonalne parametry query:**
  * `sport` — np. `FOOTBALL`, `BASKETBALL`, `TENNIS`, `CS2` (wielkość liter bez znaczenia)
  * `status` — np. `SCHEDULED`, `LIVE`, `FINISHED`, `POSTPONED`, `CANCELLED`
  * `season` — np. `2024-2025`, `2026`
  * `limit` — maksymalna liczba wyników na stronę (przycinana do zakresu `1..=5000`, `MAX_MATCHES_LIMIT = 5000`; brak parametru = wszystkie pasujące mecze)
  * `offset` — przesunięcie wyników do paginacji (domyślnie `0`; wartości ujemne sprowadzane do `0`)
  * `date_from` — dolna granica daty meczu w UTC w formacie RFC 3339 (`match_date >= date_from`), np. `2024-10-01T00:00:00Z`
  * `date_to` — górna granica daty meczu w UTC w formacie RFC 3339 (`match_date <= date_to`), np. `2025-05-01T00:00:00Z`
* **Uwaga dot. `match_details`:**
  * dla piłki nożnej (`sport_type: "football"`): `ht_home`, `ht_away`, `round` oraz opcjonalnie `home_stats` / `away_stats` (`FootballTeamStats`: `xg`, `shots`, `shots_on_target`, `corners`, `fouls`, `yellow_cards`, `red_cards`) i `pre_match_elo` (`FootballPreMatchElo`: `home_elo`, `away_elo`, `elo_diff`, `home_rank`, `away_rank`, `clubelo_prob_home/draw/away`),
  * dla koszykówki (`sport_type: "basketball"`): oprócz pól `quarters` i `overtime` obiekt może zawierać opcjonalne pola `season_type` (`"Regular Season"` / `"Playoffs"`), `home_stats` oraz `away_stats` (`BasketballTeamStats` — 21 statystyk `Base`: `min`, `fgm`, `fga`, `fg_pct`, `fg3m`, `fg3a`, `fg3_pct`, `ftm`, `fta`, `ft_pct`, `oreb`, `dreb`, `reb`, `ast`, `tov`, `stl`, `blk`, `blka`, `pf`, `pfd`, `plus_minus` oraz 9 metryk `Advanced`: `off_rating`, `def_rating`, `net_rating`, `pace`, `efg_pct`, `ts_pct`, `tm_tov_pct`, `oreb_pct`, `pie`),
  * dla tenisa (`sport_type: "tennis"`): `surface`, `best_of`, `sets`, `home_aces`, `away_aces`,
  * dla CS2 (`sport_type: "cs2"`): `best_of`, `tournament_name`, `tournament_tier`, `maps` (`Cs2MapResult`), `home_roster`/`away_roster` (`Cs2Player`), opcjonalnie `player_stats` (`Cs2PlayerStats`) i `veto` (`Cs2VetoAction`).

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
Przyjmuje predykcję z silnika ML (automatycznie rejestruje model po parze `(model_name, model_version)` i wykonuje upsert po `(match_id, model_id)`), a następnie automatycznie wylicza i zapisuje (upsert) kursy `MARKET_AVG` w `match_odds` z użyciem `BOOKMAKER_MARGIN` (zarówno dla meczów `SCHEDULED`, jak i `FINISHED` pod backtester). Jeśli mecz ma status `SCHEDULED`, wysyła zdarzenie `MATCH_OPPORTUNITY` do wszystkich agentów podpiętych przez WebSocket.
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
* **Parametr ścieżki `{sport}`:** `football`, `basketball`, `tennis`, `cs2` albo `all` (inna wartość zwraca `400 Bad Request`).
* **Opcjonalny parametr query:** `?history=true` (domyślnie `false` — pobiera również sezony historyczne).
* **Odpowiedź (`202 Accepted`):**
  ```json
  {
    "started": ["football", "basketball", "tennis", "cs2"],
    "history": true
  }
  ```

#### `POST /api/v1/simulation/settle`
Rozlicza wszystkie zakłady ze statusem `PENDING` dla meczów o statusie `FINISHED` lub `CANCELLED` i publikuje zdarzenia `MATCH_SETTLED` do odpowiednich agentów.
* **Odpowiedź (`200 OK`):**
  ```json
  { "settled_bets": 12 }
  ```

#### `POST /api/v1/simulation/backtest`
Uruchamia symulację historyczną w tle na w pełni izolowanych wirtualnych portfelach i zwraca `202 Accepted` z obiektem planu `BacktestPlan` (`run_id`, `season`, `sport`, `total_matches`, `total_days`, `bet_window_ms`, `initial_bankroll`, `estimated_seconds`). Jeśli inny backtest aktualnie trwa, zwraca `409 Conflict`. Jeśli dla wybranego sezonu/sportu brak meczów `FINISHED` z wyznaczonym zwycięzcą i kursami `MARKET_AVG`, zwraca `404 Not Found`. Symulacja nigdy nie modyfikuje produkcyjnych tabel `agents` ani `bets`.
* **Request body:**
  ```json
  {
    "season": "2025-2026",
    "sport": "FOOTBALL",
    "bet_window_ms": 500,
    "initial_bankroll": 1000.00
  }
  ```
  *(Tylko `season` jest wymagane; `bet_window_ms` domyślnie `500`, maksymalnie `30000`; `initial_bankroll` opcjonalnie nadpisuje kapitał początkowy wszystkich wirtualnych portfeli w tej symulacji — brak oznacza użycie `agents.initial_bankroll` każdego agenta).*

#### `GET /api/v1/simulation/backtest` oraz `/api/v1/simulation/backtest/runs...`
Pozwalają pobrać wirtualne wyniki symulacji przechowywane w pamięci serwera (`MAX_STORED_RUNS = 20`):
* `GET /api/v1/simulation/backtest` — pełny raport (`BacktestReport`: metadane przebiegu, `leaderboard` oraz chronologiczne `bets` z `match_date` i `bankroll_after`) dla aktywnego lub najnowszego przebiegu (`404`, jeśli nie uruchomiono żadnego backtestu).
* `GET /api/v1/simulation/backtest/runs` — lista skrótów (`BacktestRunSummary`) od najnowszego przebiegu.
* `GET /api/v1/simulation/backtest/runs/{run_id}` — pełny raport wybranego przebiegu.
* `GET /api/v1/simulation/backtest/runs/{run_id}/leaderboard` — ranking wirtualnych portfeli agentów (`BacktestLeaderboardEntry`) dla wybranego `run_id`.
* `GET /api/v1/simulation/backtest/runs/{run_id}/bets?agent_id=1` — lista wirtualnych kuponów (`VirtualBet`) z wybranego przebiegu (z opcjonalnym filtrem `agent_id`).

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
Błędne dane odrzucają handshake kodem `401 Unauthorized`. Po połączeniu serwer wysyła `WELCOME`, a następnie zaległe `MATCH_OPPORTUNITY` dla przyszłych meczów `SCHEDULED` (`match_date > NOW()`) posiadających kursy `MARKET_AVG` i predykcje, na które ten agent jeszcze nie obstawiał.

### Wiadomości z serwera do agenta (`ServerMessage`)
Format koperty: `{"event": "...", "data": {...}}`.
* **`WELCOME`** *(do danego agenta)* — `{ "agent_id", "agent_name", "current_bankroll" }`
* **`MATCH_OPPORTUNITY`** *(broadcast)* — `{ "match_id", "sport", "competition", "home_name", "away_name", "odds_home", "odds_draw", "odds_away", "predictions": [...] }`
* **`BET_RESULT`** *(do danego agenta)* — `{ "match_id", "accepted", "new_bankroll", "message" }`
* **`MATCH_SETTLED`** *(do danego agenta)* — `{ "match_id", "winner", "bet_status", "profit_loss", "current_bankroll" }`
* **`BANKROLL`** *(do danego agenta)* — `{ "current_bankroll" }`
* **`BACKTEST_STARTED`** *(broadcast)* — `{ "run_id", "season", "sport", "total_matches", "total_days", "initial_bankroll" }`
* **`BACKTEST_FINISHED`** *(broadcast)* — `{ "run_id", "season", "total_matches", "settled_bets", "leaderboard" }`
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

Wszystkie ustawienia aplikacji wczytywane są w `src/config.rs` (oraz w `src/main.rs` dla `RUST_LOG` i `docker-compose.yml` dla `BACKEND_IMAGE_TAG`). Gotowy szablon znajduje się w [`.env.example`](../.env.example) w katalogu głównym repozytorium.

| Zmienna | Wymagana | Domyślna wartość | Opis |
| :--- | :--- | :--- | :--- |
| `DATABASE_URL` | Tak | — | Adres połączenia z PostgreSQL |
| `HOST` | Nie | `0.0.0.0` | Adres nasłuchu serwera Axum |
| `PORT` | Nie | `3000` | Port serwera HTTP/WS |
| `RUST_LOG` | Nie | `backend=debug,tower_http=info,sqlx=warn` | Poziom logowania (`tracing`); w `docker-compose.yml` ustawiana na `info` |
| `LOG_TO_FILE` | Nie | `false` | Włącza jednoczesny zapis logów do dziennych plików `backend.log.YYYY-MM-DD` (`true`/`1`/`yes`) |
| `LOG_DIR` | Nie | `logs` | Katalog na pliki logów (w `docker-compose.yml` mapowany wolumenem `./logs:/app/logs`) |
| `LOG_MAX_FILES` | Nie | `14` | Maksymalna liczba przechowywanych dziennych plików logów (`0` = bez limitu) |
| `BACKEND_IMAGE_TAG` | Nie *(Docker Compose)* | `dev` | Tag obrazu backendu pobieranego w `docker-compose.yml` (`ghcr.io/adam-kulwicki/bankrut-backend:${BACKEND_IMAGE_TAG:-dev}`) |
| `FOOTBALL_DATA_API_KEY` | Nie | `None` | Klucz do Football-Data.org (opcjonalne uzupełnienie bieżącego sezonu piłki nożnej o `utcDate`, kolejki i statusy `LIVE`; pusta wartość traktowana jako `None`) |
| `BALLDONTLIE_API_KEY` | Nie | `None` | Klucz do BALLDONTLIE (terminarz, kwarty, dokładny `datetime` UTC NBA oraz fallback historii; pusta wartość z `.env` traktowana jako `None`) |
| `THESPORTSDB_EUROLEAGUE_ID` | Nie | `None` | ID ligi Euroligi w TheSportsDB (bez niego lub przy pustej wartości Euroliga jest pomijana) |
| `PANDASCORE_API_KEY` | Nie | `None` | Klucz Bearer do PandaScore Free (`/csgo/*`) — terminarz, mecze `LIVE` i bieżące wyniki CS2 |
| `KAGGLE_API_TOKEN` | Nie | `None` | Access token Bearer (`KGAT_...`) do automatycznego pobierania CSV CS2 (`ektarr/counter-strike-pro-matches`) |
| `BOOKMAKER_MARGIN` | Nie | `0.05` | Marża bukmacherska stosowana przy wyliczaniu kursów `MARKET_AVG` z prawdopodobieństw modeli (`[0.00, 1.00)`) |
| `INGEST_ENABLED` | Nie | `true` | Czy uruchamiać automatyczny harmonogram ETL w tle |
| `INGEST_INTERVAL_MINUTES` | Nie | `60` | Odstęp (w minutach) między cyklami ingestu |
| `INGEST_FOOTBALL_SOURCE` | Nie | `auto` | Strategia źródła meczów piłki nożnej: `auto` (`football-data.co.uk` + Football-Data.org + `fixtures.csv`), `football_data_uk` (akceptuje też `football-data-uk`, `csv`), `football_data` |
| `INGEST_FOOTBALL_LEAGUES` | Nie | `PL` | Lista kodów lig Top 5 rozdzielona przecinkami (`PL,PD,BL1,SA,FL1`) |
| `INGEST_FOOTBALL_CLUBELO_ENABLED` | Nie | `true` | Czy wzbogacać mecze piłkarskie o przedmeczowe ratingi Elo (`D - 1`) i prognozy z `clubelo.com` |
| `INGEST_FOOTBALL_SEASONS` | Nie | `[]` *(2 ostatnie zakończone sezony)* | Lista sezonów historycznych piłki nożnej rozdzielona przecinkami, np. `2023-24,2024-25`, `2023-2024` lub `2023` |
| `INGEST_NBA_SOURCE` | Nie | `auto` | Strategia wyboru źródła danych NBA: `auto` (`stats.nba.com` + BALLDONTLIE jako terminarz/kwarty/fallback), `nba_stats` (akceptuje też `nba-stats`, `nbastats` — wyłącznie `stats.nba.com`), `balldontlie` (wyłącznie BALLDONTLIE, wymaga klucza API) |
| `NBA_API_PROXY` | Nie | `None` | Opcjonalne proxy HTTP/HTTPS wyłącznie dla `stats.nba.com` (np. `http://user:pass@host:port`) |
| `NBA_STATS_TIMEOUT_SECS` | Nie | `90` | Limit czasu (w sekundach) pojedynczego zapytania do `stats.nba.com` |
| `NBA_STATS_DELAY_MS` | Nie | `1500` | Pauza (w milisekundach) między zapytaniami do `stats.nba.com` |
| `INGEST_NBA_SEASONS` | Nie | `[]` *(2 ostatnie zakończone sezony)* | Lista lat startu sezonów historycznych NBA, np. `2023,2024` (bieżący sezon jest synchronizowany osobno i nie jest powielany) |
| `INGEST_TENNIS_YEARS` | Nie | `[]` *(2 ostatnie zakończone lata)* | Lista lat historycznych dla archiwum Sackmanna, np. `2023,2024` |
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
cp .env.example .env
docker compose up -d
```

Możesz nadpisać tag pobieranego obrazu przez zmienną `BACKEND_IMAGE_TAG` (domyślnie `dev`), np. `BACKEND_IMAGE_TAG=prod docker compose up -d`.

---

## 7. Dalszy plan rozwoju (Roadmap & TODO)

### 1. Niezawodność i deduplikacja w warstwie Ingest / ETL
* [x] **Zaawansowana deduplikacja meczów w `store_matches` (`src/ingest/mod.rs`):** Wdrożono dopasowanie po `external_ids @>` oraz po parze drużyn i oknie ±36 h (dla NBA ±12 h z przesunięciem +23 h i obsługą meczów na terenie neutralnym), preferowanie dokładniejszego czasu `match_date` oraz bezstratne scalanie `MatchDetails::Football` i `MatchDetails::Basketball`.
* [x] **Poprawa parsowania zmiennych listowych i pustych kluczy w `src/config.rs`:** Wdrożono przycinanie i filtrowanie pustych elementów w `env_list` oraz helper `env_non_empty` traktujący puste klucze API jako `None`.
* [ ] **Separacja sezonów archiwalnych od bieżących:** Domyślne zakresy historyczne oraz ręczne `INGEST_NBA_SEASONS` i `INGEST_FOOTBALL_SEASONS` (`season != current`) są już odseparowane od sezonu bieżącego; do dodania pozostaje analogiczny strażnik przy ręcznym ustawieniu `INGEST_TENNIS_YEARS`.

### 2. Rozszerzenie danych i dyscyplin sportowych
* [ ] **Obsługa pełnych 5 dyscyplin sportowych:** Dodanie kolejnych 2 dyscyplin do migracji (`CHECK` w `competitions` / `participants` / `prediction_models`), wariantów `MatchDetails` oraz nowych workerów w `src/ingest/`.
* [x] **Zaawansowane statystyki i rankingi dla piłki nożnej (`src/ingest/football/`):** Wdrożono pobieranie `xG`, strzałów, rożnych, fauli i kartek z `football-data.co.uk` oraz przedmeczowych ratingów Elo (`D - 1`) z `clubelo.com` dla Top 5 lig Europy (`PL, PD, BL1, SA, FL1`).
* [x] **Wzbogacenie statystyk koszykówki (`src/ingest/basketball.rs`):** Wdrożono odczyt punktów w kwartach i dogrywkach (`quarters`) z `balldontlie.io` oraz integrację statystyk meczowych `Base` i `Advanced` ze `stats.nba.com/stats/teamgamelogs` (`BasketballTeamStats`, z opcją `NBA_API_PROXY`).
* [x] **Automatyczne wyliczanie kursów `MARKET_AVG` (`src/domain/odds.rs`):** Wdrożono wyliczanie kursów bukmacherskich w `POST /api/v1/predictions` na podstawie prawdopodobieństw modeli i marży `BOOKMAKER_MARGIN` (także dla meczów `FINISHED` pod backtester); opcjonalny web scraper kursów pozostaje kierunkiem rozwojowym.

### 3. Rozwój giełdy, typerów, API i backtestera
* [x] **Interaktywna dokumentacja OpenAPI / Swagger UI (`src/api/docs.rs`):** Udostępniono `/docs` oraz `/api-docs/openapi.json` generowane automatycznie przez `utoipa`.
* [ ] **Konta graczy (typerów) i ranking użytkowników:** Rozszerzenie modelu o graczy ludzkich (obok agentów AI), endpointy obstawiania przez REST dla aplikacji frontendowej oraz osobny ranking najlepszych typerów.
* [x] **Wirtualna symulacja portfela w backtesterze:** Backtester działa wyłącznie na izolowanych wirtualnych portfelach w pamięci (`BacktestStore`), z dedykowanymi endpointami wyników `GET /api/v1/simulation/backtest*` (bez modyfikowania bieżącego salda produkcyjnego agentów).
* [ ] **Testy integracyjne z bazą PostgreSQL:** Rozbudowa testów w `tests/` o scenariusze bazodanowe (transakcje `place_bet`, rozliczenia `settle_matches`, `store_matches`).
