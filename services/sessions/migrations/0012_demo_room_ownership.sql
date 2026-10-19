-- Demo organizers are scoped to the browser session that created each room.
-- Existing host-created rooms remain under host operator control.
ALTER TABLE rooms ADD COLUMN demo_owner_session_id uuid REFERENCES browser_sessions(id);
CREATE INDEX rooms_demo_owner ON rooms(demo_owner_session_id) WHERE demo_owner_session_id IS NOT NULL;
