-- ============================================================
-- MIGRACJA 1: RDZEŃ SPORTOWY (Ligi, Uczestnicy, Mecze, Kursy)
-- ============================================================

-- 1. Rozgrywki / Turnieje (np. Premier League, NBA, ATP Tour)
CREATE TABLE competitions (
    id SERIAL PRIMARY KEY,
    sport VARCHAR(20) NOT NULL CHECK (sport IN ('FOOTBALL', 'BASKETBALL', 'TENNIS')),
    name VARCHAR(100) NOT NULL,
    country_or_category VARCHAR(50),
    external_code VARCHAR(50) NOT NULL UNIQUE, -- np. 'PL', 'NBA', 'ATP_SINGLES'
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 2. Uczestnicy (Drużyny piłkarskie/koszykarskie ORAZ zawodnicy tenisowi)
CREATE TABLE participants (
    id SERIAL PRIMARY KEY,
    sport VARCHAR(20) NOT NULL CHECK (sport IN ('FOOTBALL', 'BASKETBALL', 'TENNIS')),
    name VARCHAR(120) NOT NULL,
    short_code VARCHAR(20), -- np. 'ARS', 'LAL', 'SWI'
    participant_type VARCHAR(20) NOT NULL CHECK (participant_type IN ('TEAM', 'PLAYER')),
    metadata JSONB NOT NULL DEFAULT '{}', -- np. {"hand": "R", "ioc": "POL"} lub {"crest_url": "..."}
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Zapobiega duplikatom tej samej drużyny/gracza w obrębie jednego sportu
    UNIQUE (sport, name)
);

-- 3. Mecze (Przeszłe, Trwające i Nadchodzące)
CREATE TABLE matches (
    id SERIAL PRIMARY KEY,
    competition_id INT NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
    season VARCHAR(20) NOT NULL, -- np. '2026-2027' lub '2026'
    match_date TIMESTAMPTZ NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'SCHEDULED' 
        CHECK (status IN ('SCHEDULED', 'LIVE', 'FINISHED', 'POSTPONED', 'CANCELLED')),
    
    home_participant_id INT NOT NULL REFERENCES participants(id),
    away_participant_id INT NOT NULL REFERENCES participants(id),
    
    -- Wynik główny: Bramki (Piłka), Punkty (Koszykówka), Wygrane sety (Tenis)
    home_score INT,
    away_score INT,
    winner VARCHAR(10) CHECK (winner IN ('HOME', 'AWAY', 'DRAW')),
    
    -- Szczegóły specyficzne dla dyscypliny (sety, kwarty, nawierzchnia, asy, połowy)
    match_details JSONB NOT NULL DEFAULT '{}',
    
    -- Identyfikatory z zewnętrznych API (ułatwiają aktualizację wyników)
    external_ids JSONB NOT NULL DEFAULT '{}',
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Klucz unikalności pod bezpieczny UPSERT (ON CONFLICT) podczas pobierania danych
    UNIQUE (competition_id, season, match_date, home_participant_id, away_participant_id)
);

-- 4. Kursy bukmacherskie (1X2 dla piłki, 12 dla koszykówki i tenisa)
CREATE TABLE match_odds (
    id SERIAL PRIMARY KEY,
    match_id INT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    bookmaker VARCHAR(50) NOT NULL DEFAULT 'MARKET_AVG',
    odds_home DECIMAL(6,2) NOT NULL CHECK (odds_home > 1.00),
    odds_draw DECIMAL(6,2) CHECK (odds_draw IS NULL OR odds_draw > 1.00),
    odds_away DECIMAL(6,2) NOT NULL CHECK (odds_away > 1.00),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    UNIQUE (match_id, bookmaker)
);

-- Indeksy optymalizujące zapytania z ML i Backtestera
CREATE INDEX idx_matches_status_date ON matches(status, match_date);
CREATE INDEX idx_matches_competition_season ON matches(competition_id, season);
CREATE INDEX idx_matches_details_gin ON matches USING GIN (match_details);