-- Sessions room revisions are monotonic; retain a Bee-local fence to reject
-- delayed commands even when their source conversation has changed.
CREATE TABLE bee_room_revision (
 room_id uuid PRIMARY KEY,
 revision bigint NOT NULL,
 conversation_id uuid,
 updated_at timestamptz NOT NULL DEFAULT now()
);
