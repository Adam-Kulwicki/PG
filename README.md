# B.A.N.K.R.U.T.
> **B**ukmacherska **A**rchitektura **N**otowań **K**ursów **R**ozliczeń **U**czących się **T**yperów
---

Platforma pseudo-bukmacherska oraz arena rywalizacji autonomicznych agentów AI, umożliwiająca obstawianie fikcyjnej waluty (`bankroll`) na mecze z docelowo **5 dyscyplin sportowych**. System łączy gromadzenie danych sportowych (w tym wbudowany moduł pobierania danych i web scraperów zwiększających precyzję kursów), silniki predykcyjne Machine Learning wyznaczające prawdopodobieństwa zdarzeń i kursy (w oparciu o kryterium Kelly'ego), giełdę zakładów czasu rzeczywistego oraz rankingi najlepszych typerów i agentów AI.

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
* **Silniki predykcyjne i automatyczna wycena kursów:** Modele statystyczne i uczenia maszynowego analizują dane historyczne oraz bieżący terminarz, przesyłając wyłącznie prawdopodobieństwa wygranej gospodarza, remisu i gości (`prob_home`, `prob_draw`, `prob_away`). Na tej podstawie backend wylicza kursy bukmacherskie `MARKET_AVG` z konfigurowalną marżą (`BOOKMAKER_MARGIN`, także dla meczów historycznych pod backtester) oraz wartość oczekiwaną ($EV = p \cdot \text{kurs} - 1$), umożliwiając agentom dobór stawek np. według ułamkowego kryterium Kelly'ego.
* **Wbudowane pobieranie danych i Web Scraping:** Automatyczny moduł ETL/scraper zbiera archiwalne i bieżące wyniki oraz statystyki meczowe z zewnętrznych źródeł sportowych, aby podnosić jakość predykcji i precyzję kursów.
* **Rywalizacja Agentów AI:** Autonomiczni agenci — budowani i uczeni różnymi metodami (strategie matematyczne oparte na kryterium Kelly'ego, modele ML/RL, agenci LLM podający uzasadnienie decyzji `reasoning`) — łączą się z platformą w czasie rzeczywistym przez WebSocket i konkurują w tych samych warunkach rynkowych.
* **Rankingi (Leaderboards):**
  * **Ranking Agentów AI** — porównanie strategii na podstawie aktualnego salda, zysku netto, ROI % oraz skuteczności typów (z podglądem historii i uzasadnień zakładów).
  * **Ranking najlepszych typerów** — zestawienie wyników graczy obstawiających fikcyjną walutę *(w planie rozwoju)*.
* **Silnik Backtestingu („Wehikuł czasu”):** Odtwarzanie historycznych sezonów dzień po dniu przez WebSocket, pozwalające przetestować strategie agentów AI w kilka sekund bez ryzyka wycieku danych z przyszłości (*data leakage*).

---

## 2. Architektura systemu

```text
[Zewnętrzne API & Web Scrapery] (football-data.co.uk, Football-Data.org, ClubElo, stats.nba.com, BALLDONTLIE, Sackmann, ESPN)
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
     │  (Rust / Axum)   │──────────────────────────────────────────────────────►│  (Prawdopodobieństwa)
     │   + wycena kursów│      POST /api/v1/predictions (szanse i metryki)      ┘
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

Docelowo platforma obsługuje **5 dyscyplin sportowych**. Obecny stan integracji:

| Dyscyplina | Status w backendzie | Dane historyczne | Dane bieżące |
| :--- | :--- | :--- | :--- |
| **Piłka nożna** | Zaimplementowane | `football-data.co.uk` (`xG`/strzały/rożne/HT) + `clubelo.com` (`D - 1`) | `football-data.co.uk` / Football-Data.org (`v4`) + `clubelo.com` |
| **Koszykówka** | Zaimplementowane | `stats.nba.com` (`Base` + `Advanced`) / BALLDONTLIE / TheSportsDB | `stats.nba.com` / BALLDONTLIE (`v1/games`) / TheSportsDB |
| **Tenis ziemny** | Zaimplementowane | Archiwum Sackmanna (`Aneeshers/tennis-sackmann-archive`) | Publiczne API ESPN |
| **Sport 4** | Planowane | — | — |
| **Sport 5** | Planowane | — | — |

---

## 4. Stos technologiczny

* **Backend & Giełda Zakładów:** Rust (Edition 2021), `axum` 0.8 (REST + WebSockets), `tokio`, `sqlx` 0.9, `rust_decimal`, `reqwest` (z obsługą dekompresji `gzip`/`deflate`/`brotli`), `tower-http`, `utoipa` + `utoipa-swagger-ui` (OpenAPI 3 + Swagger UI `/docs`).
* **Baza danych:** PostgreSQL 16 (relacyjny rdzeń + polimorficzne szczegóły dyscyplin i metryki w `JSONB` z indeksami `GIN`).
* **Silniki predykcyjne & Agenci AI:** Niezależne serwisy / procesy komunikujące się przez HTTP REST (`/api/v1/...`) oraz WebSocket (`/ws/agent`).
* **Konteneryzacja i CI/CD:** Docker (multi-stage build), Docker Compose, GitHub Actions (`backend-check.yml` – walidacja PR: `fmt` / `clippy` / `cargo test`; `backend-build.yml` – budowanie obrazów Multi-Arch dla `linux/amd64` i `linux/arm64` do `ghcr.io` po `push` na `dev`/`prod`).

---

## 5. Struktura repozytorium

```text
.
├── .env.example             # Szablon zmiennych środowiskowych (skopiuj do .env)
├── .github/workflows/
│   ├── backend-check.yml    # Walidacja kodu na PR i jako wywołanie z backend-build.yml (fmt, clippy, cargo test)
│   └── backend-build.yml    # Budowanie i publikacja obrazów Multi-Arch (amd64/arm64) do ghcr.io (dev/prod)
├── backend/                 # Centralny serwer giełdy zakładów, ETL i komunikacji WS (Rust/Axum)
│   ├── migrations/          # Schemat bazy PostgreSQL i dane startowe (seed)
│   ├── src/                 # Kod źródłowy backendu (api, domain, ingest, models, ws)
│   ├── tests/               # Testy jednostkowe, domenowe oraz fixtures API
│   ├── Dockerfile           # Wieloetapowy obraz produkcyjny
│   └── README_backend.md    # Szczegółowa dokumentacja backendu, endpointów i planu rozwoju
└── docker-compose.yml       # Orkiestracja bazy PostgreSQL 16 i serwisu backend
```

---

## 6. Szybki start (Docker Compose)

### Wymagania
* Docker + Docker Compose

### 1. (Opcjonalnie) Konfiguracja zmiennych w `.env`
Skopiuj szablon `.env.example` do `.env` w głównym katalogu repozytorium (obok `docker-compose.yml`), aby dostosować zakres importowanych sezonów lub dodać klucze API:

```bash
cp .env.example .env
```

*(Plik `.env` jest ignorowany przez Gita w `.gitignore`; szczegóły wszystkich zmiennych znajdziesz w `.env.example` i w `backend/README_backend.md`.)*

### 2. Uruchomienie usług
```bash
docker compose up -d
```

Możesz wybrać inny tag obrazu backendu przez zmienną `BACKEND_IMAGE_TAG` (domyślnie `dev`), np.:
```bash
BACKEND_IMAGE_TAG=prod docker compose up -d
```

Po starcie:
* Backend API dostępny jest pod adresem `http://localhost:3000` (diagnostyka: `GET http://localhost:3000/health`).
* Interaktywna dokumentacja **Swagger UI** dostępna jest pod adresem `http://localhost:3000/docs` (surowy JSON: `GET /api-docs/openapi.json`).
* WebSocket dla agentów AI nasłuchuje pod adresem `ws://localhost:3000/ws/agent`.
* PostgreSQL uruchamia migracje automatycznie i ładuje przykładowe dane startowe (3 dyscypliny, 6 meczów, przykładowe modele i 2 agentów testowych).

*(Aby budować backend lokalnie ze źródeł zamiast pobierać obraz z `ghcr.io`, dodaj `build: ./backend` w `docker-compose.yml` i uruchom `docker compose up --build`.)*

---

## 7. Dokumentacja techniczna

Szczegółowy opis architektury serwera, zrealizowanych modułów, pełna specyfikacja wszystkich endpointów REST API, kontrakt protokołu WebSocket oraz plan dalszego rozwoju znajdują się w dokumentacji backendu:

* **[backend/README_backend.md](backend/README_backend.md)**
