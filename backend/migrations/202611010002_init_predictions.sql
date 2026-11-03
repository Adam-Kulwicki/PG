-- ============================================================
-- MIGRACJA 2: SILNIKI PREDYKCYJNE I ICH PROGNOZY
-- ============================================================

-- 1. Zarejestrowane silniki predykcyjne (np. XGBoost, Elo, Poisson)
CREATE TABLE prediction_models (
    id SERIAL PRIMARY KEY,
    name VARCHAR(60) NOT NULL,
    version VARCHAR(20) NOT NULL DEFAULT '1.0',
    sport VARCHAR(20) NOT NULL CHECK (sport IN ('FOOTBALL', 'BASKETBALL', 'TENNIS', 'ALL')),
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    UNIQUE (name, version)
);

-- 2. Predykcje wygenerowane dla konkretnych meczów
CREATE TABLE predictions (
    id SERIAL PRIMARY KEY,
    match_id INT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    model_id INT NOT NULL REFERENCES prediction_models(id) ON DELETE CASCADE,
    
    -- Prawdopodobieństwa (od 0.0000 do 1.0000, np. 0.5840 = 58.4%)
    prob_home DECIMAL(5,4) NOT NULL CHECK (prob_home >= 0.0 AND prob_home <= 1.0),
    prob_draw DECIMAL(5,4) NOT NULL DEFAULT 0.0000 CHECK (prob_draw >= 0.0 AND prob_draw <= 1.0),
    prob_away DECIMAL(5,4) NOT NULL CHECK (prob_away >= 0.0 AND prob_away <= 1.0),
    
    -- Dodatkowe metryki modelu (np. expected_goals, przewidywana liczba setów, confidence)
    extra_metrics JSONB NOT NULL DEFAULT '{}',
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    -- Jeden model w danej wersji wystawia jedną główną predykcję na dany mecz
    UNIQUE (match_id, model_id)
);

CREATE INDEX idx_predictions_match_id ON predictions(match_id);