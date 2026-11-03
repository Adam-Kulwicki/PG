-- ============================================================
-- MIGRACJA 4: DANE TESTOWE (SEED) DLA ZESPOŁU (ML + AGENCI)
-- ============================================================

-- 1. Rozgrywki dla 3 dyscyplin
INSERT INTO competitions (id, sport, name, country_or_category, external_code) VALUES
    (1, 'FOOTBALL', 'Premier League', 'England', 'PL'),
    (2, 'BASKETBALL', 'NBA', 'USA', 'NBA'),
    (3, 'TENNIS', 'ATP World Tour', 'International', 'ATP');

SELECT setval('competitions_id_seq', 3);

-- 2. Uczestnicy (Drużyny i Tenisiści)
INSERT INTO participants (id, sport, name, short_code, participant_type, metadata) VALUES
    (1, 'FOOTBALL', 'Arsenal FC', 'ARS', 'TEAM', '{"stadium": "Emirates Stadium"}'),
    (2, 'FOOTBALL', 'Chelsea FC', 'CHE', 'TEAM', '{"stadium": "Stamford Bridge"}'),
    (3, 'BASKETBALL', 'Los Angeles Lakers', 'LAL', 'TEAM', '{"conference": "West"}'),
    (4, 'BASKETBALL', 'Boston Celtics', 'BOS', 'TEAM', '{"conference": "East"}'),
    (5, 'TENNIS', 'Carlos Alcaraz', 'ALC', 'PLAYER', '{"hand": "R", "ioc": "ESP"}'),
    (6, 'TENNIS', 'Jannik Sinner', 'SIN', 'PLAYER', '{"hand": "R", "ioc": "ITA"}');

SELECT setval('participants_id_seq', 6);

-- 3. Przykładowe Mecze (3 zakończone + 3 nadchodzące)
INSERT INTO matches (
    id, competition_id, season, match_date, status,
    home_participant_id, away_participant_id,
    home_score, away_score, winner, match_details
) VALUES
    -- ZAKOŃCZONE MECZE (do testów rozliczania i historii)
    (1, 1, '2026-2027', NOW() - INTERVAL '3 days', 'FINISHED', 1, 2, 2, 1, 'HOME',
     '{"sport_type": "football", "ht_home": 1, "ht_away": 1, "round": "Matchday 10"}'),
     
    (2, 2, '2026-2027', NOW() - INTERVAL '2 days', 'FINISHED', 3, 4, 112, 118, 'AWAY',
     '{"sport_type": "basketball", "quarters": [[28,30], [25,29], [31,27], [28,32]], "overtime": false}'),
     
    (3, 3, '2026', NOW() - INTERVAL '1 day', 'FINISHED', 5, 6, 2, 1, 'HOME',
     '{"sport_type": "tennis", "surface": "Hard", "best_of": 3, "sets": [[6,4], [3,6], [7,6]], "home_aces": 9, "away_aces": 14}'),

    -- NADCHODZĄCE MECZE (SCHEDULED - gotowe pod predykcje ML i zakłady Agentów)
    (4, 1, '2026-2027', NOW() + INTERVAL '2 days', 'SCHEDULED', 2, 1, NULL, NULL, NULL,
     '{"sport_type": "football", "ht_home": null, "ht_away": null, "round": "Matchday 11"}'),
     
    (5, 2, '2026-2027', NOW() + INTERVAL '3 days', 'SCHEDULED', 4, 3, NULL, NULL, NULL,
     '{"sport_type": "basketball", "quarters": [], "overtime": false}'),
     
    (6, 3, '2026', NOW() + INTERVAL '4 days', 'SCHEDULED', 6, 5, NULL, NULL, NULL,
     '{"sport_type": "tennis", "surface": "Indoor Hard", "best_of": 3, "sets": [], "home_aces": null, "away_aces": null}');

SELECT setval('matches_id_seq', 6);

-- 4. Kursy bukmacherskie dla nadchodzących meczów
INSERT INTO match_odds (match_id, bookmaker, odds_home, odds_draw, odds_away) VALUES
    (4, 'MARKET_AVG', 2.65, 3.40, 2.60), -- Chelsea vs Arsenal (Piłka ma remis)
    (5, 'MARKET_AVG', 1.55, NULL, 2.45), -- Celtics vs Lakers (Brak remisu)
    (6, 'MARKET_AVG', 1.85, NULL, 1.95); -- Sinner vs Alcaraz (Brak remisu)

-- 5. Przykładowe silniki predykcyjne i predykcje na start
INSERT INTO prediction_models (id, name, version, sport, description) VALUES
    (1, 'Elo_Baseline', '1.0', 'ALL', 'Bazowy model rankingowy Elo'),
    (2, 'XGBoost_Football', '1.0', 'FOOTBALL', 'Gradient Boosting na statystykach bramkowych');

SELECT setval('prediction_models_id_seq', 2);

INSERT INTO predictions (match_id, model_id, prob_home, prob_draw, prob_away, extra_metrics) VALUES
    (4, 2, 0.3400, 0.2800, 0.3800, '{"expected_goals_home": 1.25, "expected_goals_away": 1.42}'),
    (5, 1, 0.6800, 0.0000, 0.3200, '{"elo_diff": 115}'),
    (6, 1, 0.5500, 0.0000, 0.4500, '{"surface_adjusted": true}');

-- 6. Dwóch testowych Agentów dla kolegów z zespołu
INSERT INTO agents (id, name, api_key, strategy_description, initial_bankroll, current_bankroll) VALUES
    (1, 'Agent_Kelly_Value', 'secret_kelly_token_1', 'Stawia wg ułamkowego Kryterium Kellyego przy EV > 5%', 1000.00, 1000.00),
    (2, 'Agent_Flat_Favorite', 'secret_flat_token_2', 'Stawia płaską stawkę 50 PLN na każdego faworyta > 60%', 1000.00, 1000.00);

SELECT setval('agents_id_seq', 2);