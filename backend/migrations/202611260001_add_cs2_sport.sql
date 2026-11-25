-- ============================================================
-- MIGRACJA 5: DODANIE DYSCYPLINY CS2 (Counter-Strike 2)
-- ============================================================

ALTER TABLE competitions
    DROP CONSTRAINT IF EXISTS competitions_sport_check,
    ADD CONSTRAINT competitions_sport_check
        CHECK (sport IN ('FOOTBALL', 'BASKETBALL', 'TENNIS', 'CS2'));

ALTER TABLE participants
    DROP CONSTRAINT IF EXISTS participants_sport_check,
    ADD CONSTRAINT participants_sport_check
        CHECK (sport IN ('FOOTBALL', 'BASKETBALL', 'TENNIS', 'CS2'));

ALTER TABLE prediction_models
    DROP CONSTRAINT IF EXISTS prediction_models_sport_check,
    ADD CONSTRAINT prediction_models_sport_check
        CHECK (sport IN ('FOOTBALL', 'BASKETBALL', 'TENNIS', 'CS2', 'ALL'));
