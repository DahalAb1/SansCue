CREATE TABLE IF NOT EXISTS bee_numeric_conversation_mapping (
    source_numeric_id BIGINT PRIMARY KEY,
    source_conversation_uuid UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE bee_numeric_conversation_mapping IS
    'Maps external Bee integer conversation IDs to source conversation UUIDs';
