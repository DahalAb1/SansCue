ALTER TABLE generation_jobs
    DROP CONSTRAINT generation_jobs_status_check,
    ADD CONSTRAINT generation_jobs_status_check
        CHECK (status IN ('pending', 'processing', 'complete', 'failed')),
    ADD COLUMN max_attempts INTEGER NOT NULL DEFAULT 5 CHECK (max_attempts > 0),
    ADD COLUMN next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN lease_token UUID,
    ADD COLUMN lease_until TIMESTAMPTZ,
    ADD COLUMN last_error_code TEXT,
    ADD CONSTRAINT generation_jobs_lease_pair
        CHECK ((lease_token IS NULL) = (lease_until IS NULL));

CREATE INDEX generation_jobs_claimable
    ON generation_jobs(next_attempt_at, updated_at)
    WHERE status IN ('pending', 'processing');

ALTER TABLE question_candidates
    DROP CONSTRAINT question_candidates_generator_version_check,
    ADD CONSTRAINT question_candidates_generator_version_nonempty
        CHECK (length(btrim(generator_version)) > 0);
