CREATE INDEX IF NOT EXISTS question_candidates_room_created
    ON question_candidates(room_id, created_at DESC);

CREATE INDEX IF NOT EXISTS transcript_events_room_conv_ordinal
    ON transcript_events(room_id, conversation_id, ingest_ordinal DESC);
