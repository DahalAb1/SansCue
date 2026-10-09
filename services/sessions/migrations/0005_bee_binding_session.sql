ALTER TABLE bee_room_bindings ADD COLUMN session_id UUID;
UPDATE bee_room_bindings b
SET session_id=m.session_id
FROM memberships m
WHERE m.room_id=b.room_id AND m.role='speaker' AND m.status='active';
ALTER TABLE bee_room_bindings ALTER COLUMN session_id SET NOT NULL;
