CREATE TABLE published_questions (
 question_id uuid PRIMARY KEY,
 room_id uuid NOT NULL REFERENCES rooms(id),
 candidate_id uuid NOT NULL UNIQUE REFERENCES topic_candidates(candidate_id),
 current_version integer NOT NULL DEFAULT 1 CHECK(current_version > 0),
 active boolean NOT NULL DEFAULT true,
 created_at timestamptz NOT NULL DEFAULT now(),
 updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX one_active_published_question_per_room
 ON published_questions(room_id) WHERE active;
CREATE INDEX published_questions_room_updated
 ON published_questions(room_id,updated_at DESC);

CREATE TABLE published_question_versions (
 question_id uuid NOT NULL REFERENCES published_questions(question_id),
 version integer NOT NULL CHECK(version > 0),
 candidate_id uuid NOT NULL REFERENCES topic_candidates(candidate_id),
 question_text text NOT NULL CHECK(length(btrim(question_text)) > 0 AND length(question_text) <= 1000),
 evidence jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(question_id,version)
);
CREATE FUNCTION reject_published_question_version_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 RAISE EXCEPTION 'published question versions are immutable';
END;
$$;
CREATE TRIGGER published_question_versions_immutable
 BEFORE UPDATE OR DELETE ON published_question_versions
 FOR EACH ROW EXECUTE FUNCTION reject_published_question_version_mutation();

CREATE TABLE question_responses (
 question_id uuid NOT NULL REFERENCES published_questions(question_id),
 membership_id uuid NOT NULL REFERENCES memberships(id),
 response text NOT NULL CHECK(response IN ('clear','partly_clear','need_help')),
 created_at timestamptz NOT NULL DEFAULT now(),
 updated_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(question_id,membership_id)
);
CREATE INDEX question_responses_question_response
 ON question_responses(question_id,response);

CREATE TABLE written_questions (
 message_id uuid PRIMARY KEY,
 room_id uuid NOT NULL REFERENCES rooms(id),
 membership_id uuid NOT NULL REFERENCES memberships(id),
 question_id uuid REFERENCES published_questions(question_id),
 body text NOT NULL CHECK(length(btrim(body)) > 0 AND length(body) <= 4000),
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX written_questions_room_created ON written_questions(room_id,created_at DESC);
