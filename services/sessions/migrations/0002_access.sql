CREATE TABLE browser_sessions (
 id uuid PRIMARY KEY, token_hash bytea NOT NULL UNIQUE, csrf text NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(), last_seen_at timestamptz NOT NULL DEFAULT now(),
 operator_epoch text, login_failures integer NOT NULL DEFAULT 0, failure_window timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE rooms (
 id uuid PRIMARY KEY, title text NOT NULL, status text NOT NULL DEFAULT 'open' CHECK(status IN ('open','ended')),
 sequence bigint NOT NULL DEFAULT 0 CHECK(sequence BETWEEN 0 AND 9007199254740991),
 join_hash bytea UNIQUE, created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE memberships (
 id uuid PRIMARY KEY, room_id uuid NOT NULL REFERENCES rooms(id), session_id uuid REFERENCES browser_sessions(id),
 role text NOT NULL CHECK(role IN ('speaker','ta','audience')), status text NOT NULL DEFAULT 'active' CHECK(status IN ('active','revoked')),
 label text NOT NULL, current_grant_id uuid, created_at timestamptz NOT NULL DEFAULT now(), last_seen_at timestamptz,
 UNIQUE(room_id,session_id), CHECK(role = 'ta' OR current_grant_id IS NULL)
);
CREATE UNIQUE INDEX one_speaker ON memberships(room_id) WHERE role='speaker';
CREATE TABLE invitations (
 id uuid PRIMARY KEY, room_id uuid NOT NULL REFERENCES rooms(id), token_hash bytea NOT NULL UNIQUE,
 status text NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','redeemed','revoked')),
 membership_id uuid REFERENCES memberships(id), grant_id uuid, redeemer_session_id uuid REFERENCES browser_sessions(id),
 created_at timestamptz NOT NULL DEFAULT now(), expires_at timestamptz NOT NULL DEFAULT now()+interval '24 hours'
);
CREATE TABLE idempotency (
 session_id uuid NOT NULL REFERENCES browser_sessions(id), target text NOT NULL, key uuid NOT NULL,
 fingerprint bytea NOT NULL, status integer NOT NULL, body jsonb NOT NULL, epoch text,
 room_id uuid REFERENCES rooms(id),
 PRIMARY KEY(session_id,target,key)
);
-- Durable invalidations; delivery is owned by the later live-state stage.
CREATE TABLE room_events (
 event_id uuid PRIMARY KEY, room_id uuid NOT NULL REFERENCES rooms(id), sequence bigint NOT NULL,
 event_type text NOT NULL DEFAULT 'room.changed', schema_version integer NOT NULL DEFAULT 1,
 occurred_at timestamptz NOT NULL DEFAULT now(), payload jsonb NOT NULL,
 visibility text NOT NULL CHECK(visibility IN ('all','speaker')), UNIQUE(room_id,sequence)
);
