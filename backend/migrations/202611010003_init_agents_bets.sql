-- ============================================================
-- MIGRACJA 3: AGENCI OBSTAWIAJĄCY I REJESTR ZAKŁADÓW (GIEŁDA)
-- ============================================================

-- 1. Zewnętrzni Agenci obstawiający mecze przez WebSocket
CREATE TABLE agents (
    id SERIAL PRIMARY KEY,
    name VARCHAR(60) NOT NULL UNIQUE,
    api_key VARCHAR(100) NOT NULL UNIQUE, -- Prosty token autoryzacyjny do połączenia WS
    strategy_description TEXT,            -- Opis strategii (np. 'Kelly Criterion 1/4', 'LLM Risk Taker')
    
    initial_bankroll DECIMAL(12,2) NOT NULL DEFAULT 1000.00,
    current_bankroll DECIMAL(12,2) NOT NULL DEFAULT 1000.00 CHECK (current_bankroll >= 0.00),
    
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 2. Zakłady postawione przez Agentów
CREATE TABLE bets (
    id SERIAL PRIMARY KEY,
    agent_id INT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    match_id INT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    
    bet_selection VARCHAR(10) NOT NULL CHECK (bet_selection IN ('HOME', 'DRAW', 'AWAY')),
    stake DECIMAL(10,2) NOT NULL CHECK (stake > 0.00),
    odds_taken DECIMAL(6,2) NOT NULL CHECK (odds_taken > 1.00),
    expected_value DECIMAL(6,4), -- Opcjonalne EV wyliczone w momencie przyjęcia zakładu
    
    status VARCHAR(20) NOT NULL DEFAULT 'PENDING' 
        CHECK (status IN ('PENDING', 'WON', 'LOST', 'VOID')),
        
    -- Wynik finansowy zakładu na czysto:
    -- dla PENDING = 0.00, dla LOST = -stake, dla WON = stake * (odds_taken - 1)
    profit_loss DECIMAL(10,2) NOT NULL DEFAULT 0.00,
    
    reasoning TEXT, -- Uzasadnienie przesłane przez agenta w ramce WebSocket PLACE_BET
    placed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    settled_at TIMESTAMPTZ,

    -- Zabezpieczenie: dany agent może postawić maksymalnie 1 zakład na ten sam mecz
    UNIQUE (agent_id, match_id)
);

CREATE INDEX idx_bets_status_match ON bets(status, match_id);
CREATE INDEX idx_bets_agent_id ON bets(agent_id);