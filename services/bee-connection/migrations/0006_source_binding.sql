ALTER TABLE bee_room_revision
    ADD COLUMN source_conversation_id TEXT;

UPDATE bee_room_revision AS room
SET source_conversation_id = conversation.state #>> '{binding,source_conversation_id}'
FROM bee_conversations AS conversation
WHERE room.conversation_id = conversation.conversation_id
  AND conversation.state #>> '{binding,source_conversation_id}' IS NOT NULL;

CREATE UNIQUE INDEX bee_one_active_source_conversation
    ON bee_room_revision(source_conversation_id)
    WHERE source_conversation_id IS NOT NULL;
