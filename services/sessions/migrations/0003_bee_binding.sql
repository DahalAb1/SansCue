CREATE TABLE bee_room_bindings (
 room_id uuid PRIMARY KEY REFERENCES rooms(id), conversation_id uuid NOT NULL,
 source_conversation_id text, status text NOT NULL CHECK(status IN ('binding','bound','unbinding','unbound')),
 revision bigint NOT NULL, updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX bee_one_active_conversation ON bee_room_bindings(conversation_id) WHERE status <> 'unbound';
CREATE TABLE bee_command_outbox (
 command_id uuid PRIMARY KEY, room_id uuid NOT NULL REFERENCES rooms(id), conversation_id uuid NOT NULL,
 command_type text NOT NULL CHECK(command_type IN ('bind','unbind')), revision bigint NOT NULL,
 payload jsonb NOT NULL, created_at timestamptz NOT NULL DEFAULT now(), delivered_at timestamptz,
 UNIQUE(room_id,revision)
);
CREATE TABLE bee_room_status (
 room_id uuid PRIMARY KEY REFERENCES rooms(id), status jsonb NOT NULL, updated_at timestamptz NOT NULL DEFAULT now()
);
