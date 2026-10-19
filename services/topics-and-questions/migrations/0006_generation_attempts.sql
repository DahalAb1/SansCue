-- Speech time of the latest generator call per room, used to throttle model requests.
CREATE TABLE room_generation_attempts (
    room_id UUID PRIMARY KEY,
    last_attempt_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
