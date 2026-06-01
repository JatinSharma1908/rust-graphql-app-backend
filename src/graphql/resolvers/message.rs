use async_graphql::*;
use chrono::NaiveDateTime;
use sqlx::PgPool;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use uuid::Uuid;

// ---- Types ----

/// A conversation summary shown in the inbox list
#[derive(SimpleObject)]
pub struct ConversationSummary {
    pub id: Uuid,
    /// "dm" or "group"
    pub conversation_type: String,
    /// group name — None for DMs
    pub name: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub last_message_at: Option<NaiveDateTime>,
    /// preview text of the last message (None if no messages yet or last message is deleted)
    pub last_message_preview: Option<String>,
    /// how many messages after the user's last_read_at
    pub unread_count: i64,
    /// other participants (excludes the requesting user)
    pub participants: Vec<Participant>,
}

#[derive(SimpleObject, sqlx::FromRow)]
pub struct Participant {
    pub user_id: Uuid,
    pub username: String,
    pub profile_pic: Option<String>,
    pub role: Option<String>,
}

/// A single message in a conversation
#[derive(SimpleObject, Clone)]
pub struct Message {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub sender_id: Uuid,
    /// None when is_deleted = true
    pub content: Option<String>,
    /// "text" | "media" | "shared"
    pub message_type: String,
    pub media_url: Option<String>,
    pub shared_post_id: Option<Uuid>,
    pub shared_job_id: Option<Uuid>,
    pub reply_to_id: Option<Uuid>,
    pub is_deleted: bool,
    pub created_at: Option<NaiveDateTime>,
}

// ---- Inputs ----

#[derive(InputObject)]
pub struct CreateConversationInput {
    /// UUIDs of the other participants (don't include yourself — added automatically)
    pub participant_ids: Vec<Uuid>,
    /// "dm" or "group"
    pub conversation_type: String,
    /// required for group, ignored for dm
    pub name: Option<String>,
}

#[derive(InputObject)]
pub struct SendMessageInput {
    pub conversation_id: Uuid,
    pub content: Option<String>,
    /// "text" | "media" | "shared" — defaults to "text"
    pub message_type: Option<String>,
    pub media_url: Option<String>,
    pub shared_post_id: Option<Uuid>,
    pub shared_job_id: Option<Uuid>,
    /// reply to another message in the same conversation
    pub reply_to_id: Option<Uuid>,
}

#[derive(InputObject)]
pub struct MessagesInput {
    pub conversation_id: Uuid,
    /// cursor: pass created_at of the oldest message you received for next page
    pub before: Option<NaiveDateTime>,
    pub limit: Option<i32>,
}

// ---- Queries ----

#[derive(Default)]
pub struct MessageQuery;

#[Object]
impl MessageQuery {
    /// List all conversations the logged-in user is part of,
    /// ordered by most recent activity first.
    async fn conversations(&self, ctx: &Context<'_>) -> Result<Vec<ConversationSummary>> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        // Fetch all conversations the user is in, with their last_read_at
        let conv_rows = sqlx::query!(
            r#"
            SELECT
                c.id,
                c.type AS conversation_type,
                c.name,
                c.created_at,
                c.last_message_at,
                cp.last_read_at
            FROM conversations c
            JOIN conversation_participants cp
                ON cp.conversation_id = c.id
               AND cp.user_id = $1
            ORDER BY c.last_message_at DESC NULLS LAST
            "#,
            user_id
        )
        .fetch_all(pool)
        .await?;

        let mut summaries = Vec::new();

        for row in conv_rows {
            // Fetch other participants (not the current user)
            let participants = sqlx::query_as!(
                Participant,
                r#"
                SELECT u.id AS user_id, u.username, u.profile_pic, cp.role
                FROM conversation_participants cp
                JOIN users u ON u.id = cp.user_id
                WHERE cp.conversation_id = $1
                  AND cp.user_id != $2
                "#,
                row.id,
                user_id
            )
            .fetch_all(pool)
            .await?;

            // Last non-deleted message for preview
            let last_msg = sqlx::query!(
                r#"
                SELECT content FROM messages
                WHERE conversation_id = $1
                  AND is_deleted = false
                ORDER BY created_at DESC
                LIMIT 1
                "#,
                row.id
            )
            .fetch_optional(pool)
            .await?;

            let last_message_preview = last_msg.and_then(|m| m.content);

            // Unread count: messages after last_read_at, not sent by the user
            let unread_count = if let Some(last_read) = row.last_read_at {
                sqlx::query_scalar!(
                    r#"
                    SELECT COUNT(*) FROM messages
                    WHERE conversation_id = $1
                      AND sender_id != $2
                      AND is_deleted = false
                      AND created_at > $3
                    "#,
                    row.id,
                    user_id,
                    last_read
                )
                .fetch_one(pool)
                .await?
                .unwrap_or(0)
            } else {
                // Never read — count all messages not from self
                sqlx::query_scalar!(
                    r#"
                    SELECT COUNT(*) FROM messages
                    WHERE conversation_id = $1
                      AND sender_id != $2
                      AND is_deleted = false
                    "#,
                    row.id,
                    user_id,
                )
                .fetch_one(pool)
                .await?
                .unwrap_or(0)
            };

            summaries.push(ConversationSummary {
                id: row.id,
                conversation_type: row.conversation_type,
                name: row.name,
                created_at: row.created_at,
                last_message_at: row.last_message_at,
                last_message_preview,
                unread_count,
                participants,
            });
        }

        Ok(summaries)
    }

    /// Paginated message history for a conversation, newest first.
    /// Pass `before` (created_at of the oldest message you have) for older pages.
    async fn messages(
        &self,
        ctx: &Context<'_>,
        input: MessagesInput,
    ) -> Result<Vec<Message>> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        // Guard: user must be a participant
        let is_participant = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM conversation_participants
                WHERE conversation_id = $1 AND user_id = $2
            )
            "#,
            input.conversation_id,
            user_id
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);

        if !is_participant {
            return Err(Error::new("You are not part of this conversation"));
        }

        let limit = input.limit.unwrap_or(30).min(100) as i64;

        // Fetch rows — split into two separate async blocks to avoid the
        // "if and else have incompatible types" error from sqlx::query! macros.
        let messages: Vec<Message> = if let Some(before_ts) = input.before {
            sqlx::query!(
                r#"
                SELECT id, conversation_id, sender_id, content,
                       message_type, media_url, shared_post_id,
                       shared_job_id, reply_to_id, is_deleted, created_at
                FROM messages
                WHERE conversation_id = $1
                  AND created_at < $2
                ORDER BY created_at DESC
                LIMIT $3
                "#,
                input.conversation_id,
                before_ts,
                limit
            )
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|r| {
                let deleted = r.is_deleted.unwrap_or(false);
                Message {
                    id: r.id,
                    conversation_id: r.conversation_id,
                    sender_id: r.sender_id,
                    content: if deleted { None } else { r.content },
                    message_type: r.message_type,
                    media_url: if deleted { None } else { r.media_url },
                    shared_post_id: if deleted { None } else { r.shared_post_id },
                    shared_job_id: if deleted { None } else { r.shared_job_id },
                    reply_to_id: r.reply_to_id,
                    is_deleted: deleted,
                    created_at: r.created_at,
                }
            })
            .collect()
        } else {
            sqlx::query!(
                r#"
                SELECT id, conversation_id, sender_id, content,
                       message_type, media_url, shared_post_id,
                       shared_job_id, reply_to_id, is_deleted, created_at
                FROM messages
                WHERE conversation_id = $1
                ORDER BY created_at DESC
                LIMIT $2
                "#,
                input.conversation_id,
                limit
            )
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|r| {
                let deleted = r.is_deleted.unwrap_or(false);
                Message {
                    id: r.id,
                    conversation_id: r.conversation_id,
                    sender_id: r.sender_id,
                    content: if deleted { None } else { r.content },
                    message_type: r.message_type,
                    media_url: if deleted { None } else { r.media_url },
                    shared_post_id: if deleted { None } else { r.shared_post_id },
                    shared_job_id: if deleted { None } else { r.shared_job_id },
                    reply_to_id: r.reply_to_id,
                    is_deleted: deleted,
                    created_at: r.created_at,
                }
            })
            .collect()
        };

        Ok(messages)
    }
}

// ---- Mutations ----

#[derive(Default)]
pub struct MessageMutation;

#[Object]
impl MessageMutation {
    /// Create a DM or group conversation.
    /// The caller is automatically added as a participant with role = 'admin'.
    async fn create_conversation(
        &self,
        ctx: &Context<'_>,
        input: CreateConversationInput,
    ) -> Result<ConversationSummary> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        // Validate type
        if input.conversation_type != "dm" && input.conversation_type != "group" {
            return Err(Error::new("conversation_type must be 'dm' or 'group'"));
        }

        // DMs must have exactly one other participant
        if input.conversation_type == "dm" && input.participant_ids.len() != 1 {
            return Err(Error::new("DM conversations must have exactly one other participant"));
        }

        // For DMs: check if a conversation already exists between these two users
        if input.conversation_type == "dm" {
            let other_id = input.participant_ids[0];
            let existing = sqlx::query_scalar!(
                r#"
                SELECT c.id FROM conversations c
                JOIN conversation_participants cp1
                    ON cp1.conversation_id = c.id AND cp1.user_id = $1
                JOIN conversation_participants cp2
                    ON cp2.conversation_id = c.id AND cp2.user_id = $2
                WHERE c.type = 'dm'
                LIMIT 1
                "#,
                user_id,
                other_id
            )
            .fetch_optional(pool)
            .await?;

            if existing.is_some() {
                return Err(Error::new("DM conversation already exists with this user"));
            }
        }

        let mut tx = pool.begin().await?;

        // Create conversation
        let conv = sqlx::query!(
            r#"
            INSERT INTO conversations (type, name)
            VALUES ($1, $2)
            RETURNING id, type AS conversation_type, name, created_at, last_message_at
            "#,
            input.conversation_type,
            input.name
        )
        .fetch_one(&mut *tx)
        .await?;

        // Add creator as admin
        sqlx::query!(
            r#"
            INSERT INTO conversation_participants (conversation_id, user_id, role)
            VALUES ($1, $2, 'admin')
            "#,
            conv.id,
            user_id
        )
        .execute(&mut *tx)
        .await?;

        // Add other participants as members
        for participant_id in &input.participant_ids {
            sqlx::query!(
                r#"
                INSERT INTO conversation_participants (conversation_id, user_id, role)
                VALUES ($1, $2, 'member')
                ON CONFLICT (conversation_id, user_id) DO NOTHING
                "#,
                conv.id,
                participant_id
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        // Fetch participants for the response (excludes self)
        let participants = sqlx::query_as!(
            Participant,
            r#"
            SELECT u.id AS user_id, u.username, u.profile_pic, cp.role
            FROM conversation_participants cp
            JOIN users u ON u.id = cp.user_id
            WHERE cp.conversation_id = $1
              AND cp.user_id != $2
            "#,
            conv.id,
            user_id
        )
        .fetch_all(pool)
        .await?;

        Ok(ConversationSummary {
            id: conv.id,
            conversation_type: conv.conversation_type,
            name: conv.name,
            created_at: conv.created_at,
            last_message_at: conv.last_message_at,
            last_message_preview: None,
            unread_count: 0,
            participants,
        })
    }

    /// Send a message to a conversation.
    /// Automatically updates conversations.last_message_at.
    async fn send_message(
        &self,
        ctx: &Context<'_>,
        input: SendMessageInput,
    ) -> Result<Message> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        // Guard: must be a participant
        let is_participant = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM conversation_participants
                WHERE conversation_id = $1 AND user_id = $2
            )
            "#,
            input.conversation_id,
            user_id
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);

        if !is_participant {
            return Err(Error::new("You are not part of this conversation"));
        }

        // Validate: must have content or media or a shared item
        let has_content = input.content.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
        let has_media = input.media_url.is_some();
        let has_shared = input.shared_post_id.is_some() || input.shared_job_id.is_some();

        if !has_content && !has_media && !has_shared {
            return Err(Error::new("Message must have content, media, or a shared item"));
        }

        // Validate reply_to belongs to the same conversation
        if let Some(reply_to_id) = input.reply_to_id {
            let valid_reply = sqlx::query_scalar!(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM messages
                    WHERE id = $1 AND conversation_id = $2
                )
                "#,
                reply_to_id,
                input.conversation_id
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(false);

            if !valid_reply {
                return Err(Error::new("reply_to message not found in this conversation"));
            }
        }

        let message_type = input.message_type
            .unwrap_or_else(|| "text".to_string());

        let mut tx = pool.begin().await?;

        // Insert message
        let msg = sqlx::query!(
            r#"
            INSERT INTO messages (
                conversation_id, sender_id, content, message_type,
                media_url, shared_post_id, shared_job_id, reply_to_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, conversation_id, sender_id, content,
                      message_type, media_url, shared_post_id,
                      shared_job_id, reply_to_id, is_deleted, created_at
            "#,
            input.conversation_id,
            user_id,
            input.content,
            message_type,
            input.media_url,
            input.shared_post_id,
            input.shared_job_id,
            input.reply_to_id
        )
        .fetch_one(&mut *tx)
        .await?;

        // Update last_message_at on the conversation
        sqlx::query!(
            "UPDATE conversations SET last_message_at = $1 WHERE id = $2",
            msg.created_at,
            input.conversation_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        let new_message = Message {
            id: msg.id,
            conversation_id: msg.conversation_id,
            sender_id: msg.sender_id,
            content: msg.content,
            message_type: msg.message_type,
            media_url: msg.media_url,
            shared_post_id: msg.shared_post_id,
            shared_job_id: msg.shared_job_id,
            reply_to_id: msg.reply_to_id,
            is_deleted: msg.is_deleted.unwrap_or(false),
            created_at: msg.created_at,
        };

        // Broadcast to all active subscribers — errors only if no subscribers, safe to ignore
        if let Ok(sender) = ctx.data::<broadcast::Sender<Message>>() {
            let _ = sender.send(new_message.clone());
        }

        Ok(new_message)
    }

    /// Soft-delete a message. Only the sender can delete their own message.
    /// Content is hidden in the messages query but the row stays for reply threading.
    async fn delete_message(
        &self,
        ctx: &Context<'_>,
        message_id: Uuid,
    ) -> Result<bool> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        let result = sqlx::query!(
            r#"
            UPDATE messages SET is_deleted = true
            WHERE id = $1 AND sender_id = $2
            "#,
            message_id,
            user_id
        )
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(Error::new("Message not found or you didn't send it"));
        }

        Ok(true)
    }

    /// Mark all messages in a conversation as read.
    /// Updates last_read_at on conversation_participants, zeroing the unread count.
    async fn mark_as_read(
        &self,
        ctx: &Context<'_>,
        conversation_id: Uuid,
    ) -> Result<bool> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        let result = sqlx::query!(
            r#"
            UPDATE conversation_participants
            SET last_read_at = NOW()
            WHERE conversation_id = $1 AND user_id = $2
            "#,
            conversation_id,
            user_id
        )
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(Error::new("You are not part of this conversation"));
        }

        Ok(true)
    }
}
// ---- Subscription ----

#[derive(Default)]
pub struct MessageSubscription;

#[Subscription]
impl MessageSubscription {
    /// Subscribe to new messages in a conversation.
    /// The connection is authenticated via the same JWT header used for mutations.
    /// Only participants of the conversation receive events.
    async fn message_received(
        &self,
        ctx: &Context<'_>,
        conversation_id: Uuid,
    ) -> Result<impl tokio_stream::Stream<Item = Message>> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        // Guard: user must be a participant of the conversation
        let is_participant = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM conversation_participants
                WHERE conversation_id = $1 AND user_id = $2
            )
            "#,
            conversation_id,
            user_id
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);

        if !is_participant {
            return Err(Error::new("You are not part of this conversation"));
        }

        let sender = ctx.data::<broadcast::Sender<Message>>()?;
        let receiver = sender.subscribe();

        // Wrap the broadcast receiver in a stream, drop receive errors (lagged),
        // and filter to only messages belonging to this conversation.
        let stream = BroadcastStream::new(receiver)
            .filter_map(|result: Result<Message, _>| result.ok())
            .filter(move |msg| msg.conversation_id == conversation_id);

        Ok(stream)
    }
}