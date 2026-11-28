# B.A.N.K.R.U.T.
> **B**ukmacherska **A**rchitektura **N**otowań **K**ursów **R**ozliczeń **U**czących się **T**yperów
---

Platforma pseudo-bukmacherska oraz arena rywalizacji autonomicznych agentów AI, umożliwiająca obstawianie fikcyjnej waluty (`bankroll`) na mecze z docelowo **5 dyscyplin sportowych** (obecnie zaimplementowane 4: piłka nożna, koszykówka, tenis ziemny oraz Counter-Strike 2). System łączy gromadzenie danych sportowych (w tym wbudowany moduł ETL/ingest i web scrapery zwiększające precyzję kursów), silniki predykcyjne Machine Learning wyznaczające prawdopodobieństwa zdarzeń i kursy (w oparciu o kryterium Kelly'ego), giełdę zakładów czasu rzeczywistego oraz rankingi najlepszych typerów i agentów AI.

---

## Spis treści

1. [Wizja i główne funkcje](#1-wizja-i-główne-funkcje)
2. [Architektura systemu](#2-architektura-systemu)
3. [Zakres dyscyplin sportowych](#3-zakres-dyscyplin-sportowych)
4. [Stos technologiczny](#4-stos-technologiczny)
5. [Struktura repozytorium](#5-struktura-repozytorium)
6. [Szybki start (Docker Compose)](#6-szybki-start-docker-compose)
7. [Dokumentacja techniczna](#7-dokumentacja-techniczna)

---

## 1. Wizja i główne funkcje

* **Giełda zakładów na fikcyjną walutę:** Bezpieczne, transakcyjne obstawianie wyników spotkań przed ich rozpoczęciem (`SELECT ... FOR UPDATE`), zarządzanie wirtualnym portfelem (`initial_bankroll`, `current_bankroll`) oraz automatyczne rozliczanie kuponów (`WON`, `LOST`, `VOID`).
* **Silniki predykcyjne i automatyczna wycena kursów:** Modele statystyczne i uczenia maszynowego analizują dane historyczne oraz bieżący terminarz, przesyłając wyłącznie prawdopodobieństwa wygranej gospodarza, remisu i gości (`prob_home`, `prob_draw`, `prob_away`). Na tej podstawie backend wylicza kursy bukmacherskie `MARKET_AVG` z konfigurowalną marżą (`BOOKMAKER_MARGIN`, rynek `1X2` dla piłki nożnej oraz `12` dla pozostałych sportów — także dla meczów historycznych pod backtester) oraz wartość oczekiwaną ($EV = p \cdot \text{kurs} - 1$), umożliwiając agentom dobór stawek np. według ułamkowego kryterium Kelly'ego.
* **Wbudowane pobieranie danych i Web Scraping:** Automatyczny moduł ETL/scraper zbiera archiwalne i bieżące wyniki oraz statystyki meczowe z zewnętrznych źródeł sportowych, aby podnosić jakość predykcji i precyzję kursów.
* **Rywalizacja Agentów AI:** Autonomiczni agenci — budowani i uczeni różnymi metodami (strategie matematyczne oparte na kryterium Kelly'ego, modele ML/RL, agenci LLM podający uzasadnienie decyzji `reasoning`) — łączą się z platformą w czasie rzeczywistym przez WebSocket i konkurują w tych samych warunkach rynkowych.
* **Rankingi (Leaderboards):**
  * **Ranking Agentów AI** — porównanie strategii na podstawie aktualnego salda, zysku netto, ROI % oraz skuteczności typów (z podglądem historii i uzasadnień zakładów).
  * **Ranking najlepszych typerów** — zestawienie wyników graczy obstawiających fikcyjną walutę *(w planie rozwoju)*.
* **Silnik Backtestingu („Wehikuł czasu”):** Odtwarzanie historycznych sezonów dzień po dniu przez WebSocket na w pełni izolowanych, wirtualnych portfelach w pamięci serwera (`BacktestStore`, bez modyfikowania produkcyjnych tabel `agents` i `bets`), pozwalające przetestować strategie agentów AI w kilka sekund bez ryzyka wycieku danych z przyszłości (*data leakage*).

---

## 2. Architektura systemu

```text
[Zewnętrzne API & Web Scrapery]
  (football-data.co.uk, Football-Data.org, ClubElo, stats.nba.com,
   BALLDONTLIE, TheSportsDB, Sackmann, ESPN, PandaScore, Kaggle CSV)
              │
              ▼  (Harmonogram w tle / Ingest ETL – idempotentny Upsert i deduplikacja)
     ┌──────────────────┐
     │  PostgreSQL 16   │◄─── Migracje SQLx + kolumny JSONB (indeksy GIN)
     └────────┬─────────┘
              │
              ▼
     ┌──────────────────┐      GET /api/v1/matches (historia, statystyki i terminarz)
     │                  │◄──────────────────────────────────────────────────────┐
     │  Backend Server  │                                                       │  Silniki Predykcyjne ML
     │  (Rust / Axum)   │──────────────────────────────────────────────────────►┘  (Prawdopodobieństwa)
     │   + wycena kursów│      POST /api/v1/predictions (szanse i metryki)      
     └────────┬─────────┘
              │
              │  Publikacja w kanale czasu rzeczywistego (tokio::sync::broadcast)
              ▼
     ┌────────────────────────────────────────────────────────┐
     │                WebSocket Hub (`/ws/agent`)             │
     └────────┬──────────────────────────────────────┬────────┘
              │ Event: MATCH_OPPORTUNITY             │ Event: MATCH_OPPORTUNITY
              ▼                                      ▼
     ┌───────────────────┐                  ┌───────────────────┐
     │  Agent AI 1       │                  │  Agent AI 2       │
     │  (np. Kelly EV)   │                  │  (np. LLM / RL)   │
     └────────┬──────────┘                  └────────┬──────────┘
              │ Cmd: PLACE_BET                       │ Cmd: PLACE_BET
              ▼                                      ▼
     [Transakcja ACID -> Rozliczenie zakończonych meczów -> Aktualizacja Rankingów]
```

---

## 3. Zakres dyscyplin sportowych

Docelowo platforma obsługuje **5 dyscyplin sportowych**. Obecny stan integracji (**4 z 5 zaimplementowane**):

| Dyscyplina | Kod w API | Status w backendzie | Dane historyczne | Dane bieżące |
| :--- | :--- | :--- | :--- | :--- |
| **Piłka nożna** *(Top 5 lig: PL, PD, BL1, SA, FL1)* | `FOOTBALL` | Zaimplementowane | `football-data.co.uk` (`xG`/strzały/rożne/faule/kartki/HT) + `clubelo.com` (`D - 1`) i skalibrowany backfill archiwum `EloRatings.csv` | `football-data.co.uk` (`fixtures.csv`) / Football-Data.org (`v4`) + `clubelo.com` (`/Fixtures`) |
| **Koszykówka** *(NBA + opcj. Euroliga)* | `BASKETBALL` | Zaimplementowane | `stats.nba.com` (`Base` + `Advanced`) / BALLDONTLIE / TheSportsDB | `stats.nba.com` / BALLDONTLIE (`v1/games`, kwarty i terminarz) / TheSportsDB |
| **Tenis ziemny** *(ATP / WTA)* | `TENNIS` | Zaimplementowane | Archiwum Sackmanna (`Aneeshers/tennis-sackmann-archive`) | Publiczne API ESPN (single) |
| **Counter-Strike 2** | `CS2` | Zaimplementowane | Kaggle CSV (`ektarr/counter-strike-pro-matches`: rundy map, 5-osobowe składy, statystyki graczy; cache `data/cs2_games.csv`) | PandaScore Free (`/csgo/*`: terminarz, `LIVE`, wyniki z ostatnich 14 dni, składy turniejowe) + przyrostowe wzbogacanie z Kaggle CSV |
| **Sport 5** | — | Planowane | — | — |

---

## 4. Stos technologiczny

* **Backend & Giełda Zakładów:** Rust (Edition 2021), `axum` 0.8 (REST + WebSockets), `tokio`, `sqlx` 0.9, `rust_decimal`, `reqwest` (z obsługą dekompresji `gzip`/`deflate`/`brotli`), `zip` (rozpakowywanie archiwów Kaggle), `tower-http`, `tracing` + `tracing-appender` (dzienne logi plikowe), `utoipa` + `utoipa-swagger-ui` (OpenAPI 3 + Swagger UI `/docs`).
* **Baza danych:** PostgreSQL 16 (relacyjny rdzeń + polimorficzne szczegóły dyscyplin i metryki w `JSONB` z indeksami `GIN`).
* **Silniki predykcyjne & Agenci AI:** Niezależne serwisy / skrypty (m.in. moduł przygotowania danych dla NBA w Pythonie: `pandas`, `nba_api` oraz silnik Elo dla CS2 w standardowej bibliotece Pythona z oceną Brier Score i Log Loss) komunikujące się z backendem przez HTTP REST (`/api/v1/...`) oraz WebSocket (`/ws/agent`).
* **Konteneryzacja i CI/CD:** Docker (multi-stage build), Docker Compose, GitHub Actions (`backend-check.yml` – walidacja PR: `fmt` / `clippy` / `cargo test`; `backend-build.yml` – budowanie obrazów Multi-Arch dla `linux/amd64` i `linux/arm64` do `ghcr.io` po `push` na `dev`/`prod`).

---

## 5. Struktura repozytorium

```text
.
├── .env.example                 # Szablon zmiennych środowiskowych (skopiuj do .env)
├── .github/workflows/
│   ├── backend-check.yml        # Walidacja kodu na PR i jako wywołanie z backend-build.yml (fmt, clippy, cargo test)
│   └── backend-build.yml        # Budowanie i publikacja obrazów Multi-Arch (amd64/arm64) do ghcr.io (dev/prod)
├── backend/                     # Centralny serwer giełdy zakładów, ETL i komunikacji WS (Rust/Axum)
│   ├── migrations/              # Migracje schematu bazy PostgreSQL (4 dyscypliny)
│   ├── src/                     # Kod źródłowy backendu (api, domain, ingest, models, ws)
│   ├── tests/                   # Testy jednostkowe, domenowe oraz fixtures zewnętrznych źródeł
│   ├── Dockerfile               # Wieloetapowy obraz produkcyjny
│   └── README_backend.md        # Szczegółowa dokumentacja backendu, endpointów, WS i planu rozwoju
├── engines/                     # Silniki predykcyjne ML i przygotowanie cech
│   ├── basketball_engine/
│   │   └── prepare_data.py      # Ekstrakcja statystyk NBA, rolling EWM, Elo i kar za absencje
│   └── counter_strike_engine/   # Silnik predykcyjny Elo dla CS2 (elo, replay, report, dataloader)
└── docker-compose.yml           # Orkiestracja bazy PostgreSQL 16 i serwisu backend (wolumeny ./logs, ./data)
```

*(Katalogi `logs/` oraz `data/` tworzone są w czasie działania usług i mapowane w `docker-compose.yml` — są ignorowane przez Gita w `.gitignore`.)*

---

## 6. Szybki start (Docker Compose)

### Wymagania
* Docker + Docker Compose

### 1. (Opcjonalnie) Konfiguracja zmiennych w `.env`
Skopiuj szablon `.env.example` do `.env` w głównym katalogu repozytorium (obok `docker-compose.yml`), aby dostosować zakres importowanych sezonów lub dodać opcjonalne klucze API (`FOOTBALL_DATA_API_KEY`, `BALLDONTLIE_API_KEY`, `PANDASCORE_API_KEY`, `KAGGLE_API_TOKEN`, `THESPORTSDB_EUROLEAGUE_ID`):

```bash
cp .env.example .env
```

*(Plik `.env` jest ignorowany przez Gita w `.gitignore`; opis wszystkich zmiennych znajdziesz w [`.env.example`](.env.example) i w [`backend/README_backend.md`](backend/README_backend.md).)*

### 2. Uruchomienie usług
Plik `docker-compose.yml` zawiera zarówno konfigurację gotowego obrazu z `ghcr.io` (`image:`), jak i budowania lokalnego (`build: ./backend`):

* **Uruchomienie z lokalnym zbudowaniem obrazu ze źródeł:**
  ```bash
  docker compose up -d --build
  ```
* **Uruchomienie gotowego obrazu opublikowanego w `ghcr.io`** (tag wybierany przez `BACKEND_IMAGE_TAG`, domyślnie `dev`, np. `dev`, `prod`, `latest`):
  ```bash
  BACKEND_IMAGE_TAG=prod docker compose pull backend
  BACKEND_IMAGE_TAG=prod docker compose up -d --no-build
  ```

Po starcie:
* Backend API dostępny jest pod adresem `http://localhost:3000` (diagnostyka: `GET http://localhost:3000/health`).
* Interaktywna dokumentacja **Swagger UI** dostępna jest pod adresem `http://localhost:3000/docs` (surowa specyfikacja OpenAPI 3 JSON: `GET http://localhost:3000/api-docs/openapi.json`).
* WebSocket dla agentów AI nasłuchuje pod adresem `ws://localhost:3000/ws/agent`.
* PostgreSQL uruchamia automatycznie migracje SQLx tworzące czysty schemat bazy dla 4 dyscyplin (bez danych startowych / seeda — dane sportowe pobiera harmonogram ETL lub `POST /api/v1/ingest/{sport}`, a agentów rejestruje się przez `POST /api/v1/agents`).
* Mapowane wolumeny hosta: `./logs:/app/logs` (dzienne pliki logów po ustawieniu `LOG_TO_FILE=true`) oraz `./data:/app/data` (lokalny cache archiwum ClubElo `clubelo_elo_ratings.csv` i datasetu Kaggle CS2 `cs2_games.csv`).

---

## 7. Dokumentacja techniczna

Szczegółowy opis architektury serwera, zrealizowanych modułów ETL, pełna specyfikacja wszystkich endpointów REST API, kontrakt protokołu WebSocket, tabela zmiennych środowiskowych oraz plan dalszego rozwoju znajdują się w dokumentacji backendu:

* **[backend/README_backend.md](backend/README_backend.md)**
