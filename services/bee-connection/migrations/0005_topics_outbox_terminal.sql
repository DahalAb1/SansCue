ALTER TABLE bee_topics_outbox
    ADD COLUMN terminal_at TIMESTAMPTZ,
    ADD COLUMN terminal_code TEXT,
    ADD CONSTRAINT bee_topics_outbox_terminal_pair
        CHECK ((terminal_at IS NULL) = (terminal_code IS NULL)),
    ADD CONSTRAINT bee_topics_outbox_terminal_or_delivered
        CHECK (delivered_at IS NULL OR terminal_at IS NULL);
