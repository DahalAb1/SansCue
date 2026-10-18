CREATE UNIQUE INDEX bee_one_active_source_conversation
    ON bee_room_bindings(lower(source_conversation_id))
    WHERE source_conversation_id IS NOT NULL AND status <> 'unbound';
