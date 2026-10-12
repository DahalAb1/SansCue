ALTER TABLE question_candidates
    DROP CONSTRAINT question_candidates_generator_version_nonempty,
    ADD CONSTRAINT question_candidates_generator_version_valid
        CHECK (
            length(btrim(generator_version)) BETWEEN 1 AND 128
            AND generator_version !~ '[[:cntrl:]]'
        );
