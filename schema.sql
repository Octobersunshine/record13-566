CREATE TABLE IF NOT EXISTS chat_messages (
    id TEXT PRIMARY KEY,
    consultation_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    receiver_id TEXT NOT NULL,
    sender_role TEXT NOT NULL CHECK(sender_role IN ('Doctor', 'Patient')),
    message_type TEXT NOT NULL CHECK(message_type IN ('Text', 'Image')),
    content TEXT NOT NULL,
    image_url TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_chat_messages_consultation_id
    ON chat_messages (consultation_id);

CREATE INDEX IF NOT EXISTS idx_chat_messages_created_at
    ON chat_messages (created_at);

CREATE INDEX IF NOT EXISTS idx_chat_messages_consultation_created
    ON chat_messages (consultation_id, created_at);
