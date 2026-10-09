CREATE TABLE bee_command_inbox (
 command_id uuid PRIMARY KEY, room_id uuid NOT NULL, conversation_id uuid NOT NULL,
 revision bigint NOT NULL, command_type text NOT NULL CHECK(command_type IN ('bind','unbind')),
 result jsonb NOT NULL, processed_at timestamptz NOT NULL DEFAULT now()
);
