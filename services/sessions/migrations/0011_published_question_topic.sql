-- A published question names one topic. Later check-ins with the same name
-- stay one row on the speaker's confusion chart.
ALTER TABLE published_questions ADD COLUMN topic_label text;

UPDATE published_questions AS p
SET topic_label = src.label
FROM (
  SELECT question_id,
    left(btrim(regexp_replace(
      regexp_replace(btrim(question_text), '\s*\?+\s*$', ''),
      '^[Hh]ow[[:space:]]+clear[[:space:]]+is[[:space:]]+',
      ''
    )), 80) AS label
  FROM published_question_versions
  WHERE version = 1
) AS src
WHERE p.question_id = src.question_id;

UPDATE published_questions
SET topic_label = 'Topic'
WHERE topic_label IS NULL OR btrim(topic_label) = '';

ALTER TABLE published_questions
  ALTER COLUMN topic_label SET NOT NULL;

ALTER TABLE published_questions
  ADD CONSTRAINT published_questions_topic_label_bounds
  CHECK (char_length(btrim(topic_label)) BETWEEN 1 AND 80);
