-- Bee-owned database only. No foreign keys into sessions or shared tables.
CREATE TABLE bee_conversations (
    conversation_id UUID PRIMARY KEY,
    state JSONB NOT NULL
);
COMMENT ON TABLE bee_conversations IS
    'Atomic conversation aggregate: binding, original observations, canonical events and recovery evidence';
