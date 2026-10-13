ALTER TABLE topic_candidates
    DROP CONSTRAINT topic_candidates_generator_version_check,
    ADD CONSTRAINT topic_candidates_generator_version_valid
        CHECK (
            length(btrim(generator_version)) BETWEEN 1 AND 128
            AND generator_version !~ '[[:cntrl:]]'
        );
