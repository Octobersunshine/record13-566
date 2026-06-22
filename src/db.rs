use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SenderRole {
    Doctor,
    Patient,
}

impl sqlx::Type<sqlx::Sqlite> for SenderRole {
    fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
        <String as sqlx::Type<sqlx::Sqlite>>::type_info()
    }

    fn compatible(ty: &sqlx::sqlite::SqliteTypeInfo) -> bool {
        <String as sqlx::Type<sqlx::Sqlite>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for SenderRole {
    fn decode(value: sqlx::sqlite::SqliteValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Sqlite>>::decode(value)?;
        match s.as_str() {
            "Doctor" => Ok(SenderRole::Doctor),
            "Patient" => Ok(SenderRole::Patient),
            other => Err(format!("无效的 SenderRole: {}", other).into()),
        }
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for SenderRole {
    fn encode_by_ref(&self, buf: &mut sqlx::sqlite::SqliteArgumentValue<'q>) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        let s = match self {
            SenderRole::Doctor => "Doctor",
            SenderRole::Patient => "Patient",
        };
        <String as sqlx::Encode<sqlx::Sqlite>>::encode(s.to_string(), buf)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageType {
    Text,
    Image,
}

impl sqlx::Type<sqlx::Sqlite> for MessageType {
    fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
        <String as sqlx::Type<sqlx::Sqlite>>::type_info()
    }

    fn compatible(ty: &sqlx::sqlite::SqliteTypeInfo) -> bool {
        <String as sqlx::Type<sqlx::Sqlite>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for MessageType {
    fn decode(value: sqlx::sqlite::SqliteValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Sqlite>>::decode(value)?;
        match s.as_str() {
            "Text" => Ok(MessageType::Text),
            "Image" => Ok(MessageType::Image),
            other => Err(format!("无效的 MessageType: {}", other).into()),
        }
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for MessageType {
    fn encode_by_ref(&self, buf: &mut sqlx::sqlite::SqliteArgumentValue<'q>) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        let s = match self {
            MessageType::Text => "Text",
            MessageType::Image => "Image",
        };
        <String as sqlx::Encode<sqlx::Sqlite>>::encode(s.to_string(), buf)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChatMessage {
    pub id: String,
    pub consultation_id: String,
    pub sender_id: String,
    pub receiver_id: String,
    pub sender_role: SenderRole,
    pub message_type: MessageType,
    pub content: String,
    pub image_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMessageRequest {
    pub consultation_id: String,
    pub sender_id: String,
    pub receiver_id: String,
    pub sender_role: SenderRole,
    pub message_type: MessageType,
    pub content: String,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedMessages {
    pub messages: Vec<ChatMessage>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub total_pages: i64,
}

pub async fn init_db(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            consultation_id TEXT NOT NULL,
            sender_id TEXT NOT NULL,
            receiver_id TEXT NOT NULL,
            sender_role TEXT NOT NULL CHECK(sender_role IN ('Doctor', 'Patient')),
            message_type TEXT NOT NULL CHECK(message_type IN ('Text', 'Image')),
            content TEXT NOT NULL,
            image_url TEXT,
            created_at DATETIME NOT NULL
        )
        "#
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_chat_messages_consultation_id ON chat_messages (consultation_id)"
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_chat_messages_created_at ON chat_messages (created_at)"
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_chat_messages_consultation_created ON chat_messages (consultation_id, created_at)"
    )
    .execute(&pool)
    .await?;

    Ok(pool)
}

pub async fn save_message(
    pool: &SqlitePool,
    req: &CreateMessageRequest,
) -> Result<ChatMessage, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO chat_messages (
            id, consultation_id, sender_id, receiver_id,
            sender_role, message_type, content, image_url, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#
    )
    .bind(&id)
    .bind(&req.consultation_id)
    .bind(&req.sender_id)
    .bind(&req.receiver_id)
    .bind(&req.sender_role)
    .bind(&req.message_type)
    .bind(&req.content)
    .bind(&req.image_url)
    .bind(now)
    .execute(pool)
    .await?;

    let message = sqlx::query_as::<_, ChatMessage>(
        r#"
        SELECT id, consultation_id, sender_id, receiver_id,
               sender_role, message_type, content, image_url, created_at
        FROM chat_messages WHERE id = ?
        "#
    )
    .bind(&id)
    .fetch_one(pool)
    .await?;

    Ok(message)
}

pub async fn get_messages_by_consultation(
    pool: &SqlitePool,
    consultation_id: &str,
    page: i64,
    page_size: i64,
) -> Result<PaginatedMessages, sqlx::Error> {
    let page = page.max(1);
    let page_size = page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    let total: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM chat_messages WHERE consultation_id = ?
        "#
    )
    .bind(consultation_id)
    .fetch_one(pool)
    .await?;

    let total = total.0;
    let total_pages = if total == 0 { 0 } else { (total + page_size - 1) / page_size };

    let messages = sqlx::query_as::<_, ChatMessage>(
        r#"
        SELECT id, consultation_id, sender_id, receiver_id,
               sender_role, message_type, content, image_url, created_at
        FROM chat_messages
        WHERE consultation_id = ?
        ORDER BY created_at ASC, id ASC
        LIMIT ? OFFSET ?
        "#
    )
    .bind(consultation_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(PaginatedMessages {
        messages,
        total,
        page,
        page_size,
        total_pages,
    })
}

pub async fn get_message_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<ChatMessage>, sqlx::Error> {
    let message = sqlx::query_as::<_, ChatMessage>(
        r#"
        SELECT id, consultation_id, sender_id, receiver_id,
               sender_role, message_type, content, image_url, created_at
        FROM chat_messages WHERE id = ?
        "#
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(message)
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConsultationSummary {
    pub id: String,
    pub consultation_id: String,
    pub chief_complaint: String,
    pub present_illness: String,
    pub diagnosis: String,
    pub treatment_plan: String,
    pub doctor_advice: String,
    pub key_points: String,
    pub generated_by: String,
    pub message_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSummaryRequest {
    pub consultation_id: String,
    pub force_regenerate: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryContent {
    pub chief_complaint: String,
    pub present_illness: String,
    pub diagnosis: String,
    pub treatment_plan: String,
    pub doctor_advice: String,
    pub key_points: String,
}

pub async fn init_summary_table(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
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
        )
        "#
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_consultation_summaries_consultation_id ON consultation_summaries (consultation_id)"
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_consultation_summaries_created_at ON consultation_summaries (created_at)"
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn save_summary(
    pool: &SqlitePool,
    consultation_id: &str,
    content: &SummaryContent,
    generated_by: &str,
    message_count: i64,
) -> Result<ConsultationSummary, sqlx::Error> {
    let now = Utc::now();

    let existing = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM consultation_summaries WHERE consultation_id = ?"
    )
    .bind(consultation_id)
    .fetch_optional(pool)
    .await?;

    let summary = if let Some((existing_id,)) = existing {
        sqlx::query(
            r#"
            UPDATE consultation_summaries SET
                chief_complaint = ?,
                present_illness = ?,
                diagnosis = ?,
                treatment_plan = ?,
                doctor_advice = ?,
                key_points = ?,
                generated_by = ?,
                message_count = ?,
                updated_at = ?
            WHERE id = ?
            "#
        )
        .bind(&content.chief_complaint)
        .bind(&content.present_illness)
        .bind(&content.diagnosis)
        .bind(&content.treatment_plan)
        .bind(&content.doctor_advice)
        .bind(&content.key_points)
        .bind(generated_by)
        .bind(message_count)
        .bind(now)
        .bind(&existing_id)
        .execute(pool)
        .await?;

        sqlx::query_as::<_, ConsultationSummary>(
            r#"
            SELECT id, consultation_id, chief_complaint, present_illness,
                   diagnosis, treatment_plan, doctor_advice, key_points,
                   generated_by, message_count, created_at, updated_at
            FROM consultation_summaries WHERE id = ?
            "#
        )
        .bind(&existing_id)
        .fetch_one(pool)
        .await?
    } else {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            r#"
            INSERT INTO consultation_summaries (
                id, consultation_id, chief_complaint, present_illness,
                diagnosis, treatment_plan, doctor_advice, key_points,
                generated_by, message_count, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&id)
        .bind(consultation_id)
        .bind(&content.chief_complaint)
        .bind(&content.present_illness)
        .bind(&content.diagnosis)
        .bind(&content.treatment_plan)
        .bind(&content.doctor_advice)
        .bind(&content.key_points)
        .bind(generated_by)
        .bind(message_count)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

        sqlx::query_as::<_, ConsultationSummary>(
            r#"
            SELECT id, consultation_id, chief_complaint, present_illness,
                   diagnosis, treatment_plan, doctor_advice, key_points,
                   generated_by, message_count, created_at, updated_at
            FROM consultation_summaries WHERE id = ?
            "#
        )
        .bind(&id)
        .fetch_one(pool)
        .await?
    };

    Ok(summary)
}

pub async fn get_summary_by_consultation(
    pool: &SqlitePool,
    consultation_id: &str,
) -> Result<Option<ConsultationSummary>, sqlx::Error> {
    let summary = sqlx::query_as::<_, ConsultationSummary>(
        r#"
        SELECT id, consultation_id, chief_complaint, present_illness,
               diagnosis, treatment_plan, doctor_advice, key_points,
               generated_by, message_count, created_at, updated_at
        FROM consultation_summaries WHERE consultation_id = ?
        "#
    )
    .bind(consultation_id)
    .fetch_optional(pool)
    .await?;

    Ok(summary)
}

pub async fn get_all_messages_for_summary(
    pool: &SqlitePool,
    consultation_id: &str,
) -> Result<Vec<ChatMessage>, sqlx::Error> {
    let messages = sqlx::query_as::<_, ChatMessage>(
        r#"
        SELECT id, consultation_id, sender_id, receiver_id,
               sender_role, message_type, content, image_url, created_at
        FROM chat_messages
        WHERE consultation_id = ?
        ORDER BY created_at ASC, id ASC
        "#
    )
    .bind(consultation_id)
    .fetch_all(pool)
    .await?;

    Ok(messages)
}
