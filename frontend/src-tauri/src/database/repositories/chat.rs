use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, SqlitePool};
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChatMessageModel {
    pub id: String,
    pub meeting_id: String,
    pub role: String, // "user" | "assistant"
    pub content: String,
    pub created_at: String, // RFC3339
}

pub struct ChatMessagesRepository;

impl ChatMessagesRepository {
    /// Inserts a new chat message and returns the stored row.
    pub async fn insert_message(
        pool: &SqlitePool,
        meeting_id: &str,
        role: &str,
        content: &str,
    ) -> Result<ChatMessageModel, SqlxError> {
        let message = ChatMessageModel {
            id: format!("chatmsg-{}", Uuid::new_v4()),
            meeting_id: meeting_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            created_at: Utc::now().to_rfc3339(),
        };

        sqlx::query(
            "INSERT INTO chat_messages (id, meeting_id, role, content, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&message.id)
        .bind(&message.meeting_id)
        .bind(&message.role)
        .bind(&message.content)
        .bind(&message.created_at)
        .execute(pool)
        .await?;

        Ok(message)
    }

    /// Returns all chat messages for a meeting in chronological order.
    pub async fn get_messages_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<ChatMessageModel>, SqlxError> {
        sqlx::query_as::<_, ChatMessageModel>(
            "SELECT id, meeting_id, role, content, created_at
             FROM chat_messages
             WHERE meeting_id = ?
             ORDER BY created_at ASC, id ASC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    /// Updates the content of a message (used to finalize streamed responses).
    pub async fn update_message_content(
        pool: &SqlitePool,
        message_id: &str,
        content: &str,
    ) -> Result<bool, SqlxError> {
        let result = sqlx::query("UPDATE chat_messages SET content = ? WHERE id = ?")
            .bind(content)
            .bind(message_id)
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Deletes a single message by id (used to discard failed assistant rows).
    pub async fn delete_message(pool: &SqlitePool, message_id: &str) -> Result<bool, SqlxError> {
        let result = sqlx::query("DELETE FROM chat_messages WHERE id = ?")
            .bind(message_id)
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Deletes all chat messages for a meeting.
    pub async fn delete_messages_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<u64, SqlxError> {
        let result = sqlx::query("DELETE FROM chat_messages WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// Re-keys chat messages from a provisional live-recording meeting id to the
    /// final meeting id generated when the meeting is saved.
    pub async fn reassign_meeting_id(
        pool: &SqlitePool,
        old_meeting_id: &str,
        new_meeting_id: &str,
    ) -> Result<u64, SqlxError> {
        let result = sqlx::query("UPDATE chat_messages SET meeting_id = ? WHERE meeting_id = ?")
            .bind(new_meeting_id)
            .bind(old_meeting_id)
            .execute(pool)
            .await?;

        if result.rows_affected() > 0 {
            info!(
                "Reassigned {} chat messages from {} to {}",
                result.rows_affected(),
                old_meeting_id,
                new_meeting_id
            );
        }

        Ok(result.rows_affected())
    }
}
