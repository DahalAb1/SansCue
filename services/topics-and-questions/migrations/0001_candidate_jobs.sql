CREATE TABLE transcript_events (
    event_id UUID PRIMARY KEY,
    schema_version SMALLINT NOT NULL CHECK (schema_version = 1),
    conversation_id UUID NOT NULL,
    room_id UUID NOT NULL,
    session_id UUID NOT NULL,
    source_conversation_id TEXT,
    ingest_ordinal BIGINT NOT NULL CHECK (ingest_ordinal > 0),
    received_at TIMESTAMPTZ NOT NULL,
    source_id TEXT,
    source_sequence TEXT,
    text TEXT NOT NULL CHECK (length(btrim(text)) > 0),
    projection_hash BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE generation_jobs (
    event_id UUID PRIMARY KEY REFERENCES transcript_events(event_id),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','complete')),
    attempts INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE question_candidates (
    candidate_id UUID PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE REFERENCES transcript_events(event_id),
    room_id UUID NOT NULL,
    session_id UUID NOT NULL,
    text TEXT NOT NULL CHECK (length(btrim(text)) > 0),
    generator_version TEXT NOT NULL CHECK (generator_version = 'stub-v1'),
    evidence JSONB NOT NULL,
    published BOOLEAN NOT NULL DEFAULT FALSE CHECK (published = FALSE),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE candidate_outbox (
    candidate_id UUID PRIMARY KEY REFERENCES question_candidates(candidate_id),
    attempts INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ,
    terminal_at TIMESTAMPTZ,
    terminal_code TEXT,
    CHECK ((terminal_at IS NULL) = (terminal_code IS NULL)),
    CHECK (delivered_at IS NULL OR terminal_at IS NULL)
);
