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

-- 问诊小结表
CREATE TABLE IF NOT EXISTS consultation_summaries (
    id TEXT PRIMARY KEY,
    consultation_id TEXT NOT NULL UNIQUE,
    chief_complaint TEXT NOT NULL DEFAULT '',
    present_illness TEXT NOT NULL DEFAULT '',
    diagnosis TEXT NOT NULL DEFAULT '',
    treatment_plan TEXT NOT NULL DEFAULT '',
    doctor_advice TEXT NOT NULL DEFAULT '',
    key_points TEXT NOT NULL DEFAULT '',
    generated_by TEXT NOT NULL DEFAULT 'template',
    message_count INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_consultation_summaries_consultation_id
    ON consultation_summaries (consultation_id);

CREATE INDEX IF NOT EXISTS idx_consultation_summaries_created_at
    ON consultation_summaries (created_at);
