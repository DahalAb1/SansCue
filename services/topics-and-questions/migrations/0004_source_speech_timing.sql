ALTER TABLE transcript_events
    ADD COLUMN IF NOT EXISTS source_utterance_id BIGINT,
    ADD COLUMN IF NOT EXISTS source_spoken_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS source_start_millis BIGINT,
    ADD COLUMN IF NOT EXISTS source_end_millis BIGINT;
