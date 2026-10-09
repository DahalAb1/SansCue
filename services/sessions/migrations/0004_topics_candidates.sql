CREATE TABLE topic_candidates (
 candidate_id uuid PRIMARY KEY,
 event_id uuid NOT NULL UNIQUE,
 room_id uuid NOT NULL REFERENCES rooms(id),
 conversation_id uuid NOT NULL,
 session_id uuid NOT NULL,
 candidate_text text NOT NULL CHECK(length(btrim(candidate_text))>0),
 generator_version text NOT NULL CHECK(generator_version='stub-v1'),
 evidence jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(room_id,event_id)
);
CREATE INDEX topic_candidates_room_created ON topic_candidates(room_id,created_at DESC);
