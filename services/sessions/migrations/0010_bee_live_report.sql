-- Latest heartbeat from the trusted host's live Bee worker: whether its Bee
-- stream is running and which recordings it hears that no room has selected.
-- Only identifiers, times and counts are stored; never speech.
CREATE TABLE bee_live_report (
 id boolean PRIMARY KEY DEFAULT true CHECK (id),
 transport text NOT NULL CHECK (transport IN ('connected', 'reconnecting')),
 recordings jsonb NOT NULL,
 reported_at timestamptz NOT NULL DEFAULT now()
);
