-- Moderator review state is kept beside the immutable Topics delivery fields,
-- which still identify an idempotent candidate callback.
ALTER TABLE topic_candidates
    ADD COLUMN draft_text text CHECK (draft_text IS NULL OR (length(btrim(draft_text)) > 0 AND length(draft_text) <= 1000)),
    ADD COLUMN edited_at timestamptz,
    ADD COLUMN edited_by uuid REFERENCES memberships(id),
    ADD COLUMN dismissed_at timestamptz,
    ADD COLUMN dismissed_by uuid REFERENCES memberships(id),
    ADD CONSTRAINT topic_candidates_edit_pair CHECK ((draft_text IS NULL) = (edited_at IS NULL)),
    ADD CONSTRAINT topic_candidates_dismiss_pair CHECK ((dismissed_at IS NULL) = (dismissed_by IS NULL));

ALTER TABLE published_questions ADD COLUMN published_by uuid REFERENCES memberships(id);
ALTER TABLE published_question_versions ADD COLUMN created_by uuid REFERENCES memberships(id);

-- The wording version that was active when each response was last saved.
ALTER TABLE question_responses
    ADD COLUMN question_version integer NOT NULL DEFAULT 1 CHECK (question_version > 0);

CREATE TABLE written_question_replies (
 reply_id uuid PRIMARY KEY,
 message_id uuid NOT NULL REFERENCES written_questions(message_id),
 room_id uuid NOT NULL REFERENCES rooms(id),
 membership_id uuid NOT NULL REFERENCES memberships(id),
 body text NOT NULL CHECK (length(btrim(body)) > 0 AND length(body) <= 2000),
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX written_question_replies_message ON written_question_replies(message_id, created_at);

-- 'member' invalidations reach room staff and one audience membership only.
ALTER TABLE room_events
    ADD COLUMN target_membership_id uuid REFERENCES memberships(id),
    DROP CONSTRAINT room_events_visibility_check,
    ADD CONSTRAINT room_events_visibility_check
        CHECK (visibility IN ('all', 'speaker', 'member') AND ((visibility = 'member') = (target_membership_id IS NOT NULL)));
