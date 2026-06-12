-- Chat messages for the "chat with transcript" feature.
-- No FK to meetings(id): during a live recording the frontend uses a
-- provisional meeting id that does not exist in meetings until save.
CREATE TABLE IF NOT EXISTS chat_messages (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_chat_messages_meeting_id
    ON chat_messages(meeting_id, created_at);
