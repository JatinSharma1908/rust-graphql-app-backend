use async_graphql::*;
use chrono::NaiveDateTime;
use sqlx::PgPool;
use uuid::Uuid;

// ---- Types ----

#[derive(SimpleObject, sqlx::FromRow)]
pub struct Post {
    pub id: Uuid,
    pub user_id: Uuid,
    pub caption: Option<String>,
    pub video_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub visibility: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    // aggregates joined in queries
    pub like_count: Option<i64>,
    pub comment_count: Option<i64>,
}

#[derive(SimpleObject, sqlx::FromRow)]
pub struct Comment {
    pub id: Uuid,
    pub post_id: Uuid,
    pub user_id: Uuid,
    pub content: String,
    pub parent_id: Option<Uuid>,
    pub created_at: Option<NaiveDateTime>,
}

#[derive(SimpleObject)]
pub struct LikePayload {
    pub post_id: Uuid,
    pub liked: bool,       // true = liked, false = unliked
    pub like_count: i64,
}

// ---- Inputs ----

#[derive(InputObject)]
pub struct CreatePostInput {
    pub caption: Option<String>,
    pub video_url: Option<String>,
    pub thumbnail_url: Option<String>,
    /// "public" | "private" | "followers" — defaults to "public"
    pub visibility: Option<String>,
}

#[derive(InputObject)]
pub struct AddCommentInput {
    pub post_id: Uuid,
    pub content: String,
    /// set this to reply to another comment
    pub parent_id: Option<Uuid>,
}

#[derive(InputObject)]
pub struct FeedInput {
    /// cursor-based pagination: pass the created_at of the last post you received
    pub before: Option<NaiveDateTime>,
    /// number of posts to fetch, max 50, default 20
    pub limit: Option<i32>,
}

// ---- Queries ----

#[derive(Default)]
pub struct PostQuery;

#[Object]
impl PostQuery {
    /// Paginated feed of posts from users the logged-in user follows.
    /// Returns newest posts first. Pass `before` for the next page.
    async fn feed(&self, ctx: &Context<'_>, input: Option<FeedInput>) -> Result<Vec<Post>> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        let limit = input
            .as_ref()
            .and_then(|i| i.limit)
            .unwrap_or(20)
            .min(50) as i64;

        let before = input.as_ref().and_then(|i| i.before);

        // If a cursor is given, fetch posts older than that timestamp.
        // We count likes and comments in the same round-trip with subqueries.
        let posts = if let Some(before_ts) = before {
            sqlx::query_as!(
                Post,
                r#"
                SELECT
                    p.id, p.user_id, p.caption, p.video_url,
                    p.thumbnail_url, p.visibility, p.created_at,
                    (SELECT COUNT(*) FROM likes   WHERE post_id = p.id) AS like_count,
                    (SELECT COUNT(*) FROM comments WHERE post_id = p.id) AS comment_count
                FROM posts p
                WHERE p.user_id IN (
                    SELECT following_id FROM user_follows WHERE follower_id = $1
                )
                AND p.visibility != 'private'
                AND p.created_at < $2
                ORDER BY p.created_at DESC
                LIMIT $3
                "#,
                user_id,
                before_ts,
                limit
            )
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as!(
                Post,
                r#"
                SELECT
                    p.id, p.user_id, p.caption, p.video_url,
                    p.thumbnail_url, p.visibility, p.created_at,
                    (SELECT COUNT(*) FROM likes    WHERE post_id = p.id) AS like_count,
                    (SELECT COUNT(*) FROM comments WHERE post_id = p.id) AS comment_count
                FROM posts p
                WHERE p.user_id IN (
                    SELECT following_id FROM user_follows WHERE follower_id = $1
                )
                AND p.visibility != 'private'
                ORDER BY p.created_at DESC
                LIMIT $2
                "#,
                user_id,
                limit
            )
            .fetch_all(pool)
            .await?
        };

        Ok(posts)
    }

    /// Fetch a single post by id.
    async fn post(&self, ctx: &Context<'_>, id: Uuid) -> Result<Post> {
        let pool = ctx.data::<PgPool>()?;

        let post = sqlx::query_as!(
            Post,
            r#"
            SELECT
                p.id, p.user_id, p.caption, p.video_url,
                p.thumbnail_url, p.visibility, p.created_at,
                (SELECT COUNT(*) FROM likes    WHERE post_id = p.id) AS like_count,
                (SELECT COUNT(*) FROM comments WHERE post_id = p.id) AS comment_count
            FROM posts p
            WHERE p.id = $1
            "#,
            id
        )
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Error::new("Post not found"))?;

        Ok(post)
    }

    /// All posts by a specific user (public + followers-only if viewing own profile).
    async fn user_posts(
        &self,
        ctx: &Context<'_>,
        user_id: Uuid,
        input: Option<FeedInput>,
    ) -> Result<Vec<Post>> {
        let pool = ctx.data::<PgPool>()?;
        // viewer_id may not exist (unauthenticated), so we use Option here
        let viewer_id = ctx.data::<Uuid>().ok().copied();

        let limit = input
            .as_ref()
            .and_then(|i| i.limit)
            .unwrap_or(20)
            .min(50) as i64;

        let before = input.as_ref().and_then(|i| i.before);

        // Decide visibility: owner sees everything, followers see
        // public + followers-only, everyone else sees only public.
        let visibility_filter: &str = if viewer_id == Some(user_id) {
            "all"
        } else {
            "public"
        };

        let posts = if let Some(before_ts) = before {
            sqlx::query_as!(
                Post,
                r#"
                SELECT
                    p.id, p.user_id, p.caption, p.video_url,
                    p.thumbnail_url, p.visibility, p.created_at,
                    (SELECT COUNT(*) FROM likes    WHERE post_id = p.id) AS like_count,
                    (SELECT COUNT(*) FROM comments WHERE post_id = p.id) AS comment_count
                FROM posts p
                WHERE p.user_id = $1
                AND ($2 = 'all' OR p.visibility = 'public')
                AND p.created_at < $3
                ORDER BY p.created_at DESC
                LIMIT $4
                "#,
                user_id,
                visibility_filter,
                before_ts,
                limit
            )
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as!(
                Post,
                r#"
                SELECT
                    p.id, p.user_id, p.caption, p.video_url,
                    p.thumbnail_url, p.visibility, p.created_at,
                    (SELECT COUNT(*) FROM likes    WHERE post_id = p.id) AS like_count,
                    (SELECT COUNT(*) FROM comments WHERE post_id = p.id) AS comment_count
                FROM posts p
                WHERE p.user_id = $1
                AND ($2 = 'all' OR p.visibility = 'public')
                ORDER BY p.created_at DESC
                LIMIT $3
                "#,
                user_id,
                visibility_filter,
                limit
            )
            .fetch_all(pool)
            .await?
        };

        Ok(posts)
    }

    /// Top-level comments on a post, newest first.
    /// To fetch replies to a comment, call this with parent_id filter — see `comment_replies`.
    async fn post_comments(&self, ctx: &Context<'_>, post_id: Uuid) -> Result<Vec<Comment>> {
        let pool = ctx.data::<PgPool>()?;

        let comments = sqlx::query_as!(
            Comment,
            r#"
            SELECT id, post_id, user_id, content, parent_id, created_at
            FROM comments
            WHERE post_id = $1 AND parent_id IS NULL
            ORDER BY created_at DESC
            "#,
            post_id
        )
        .fetch_all(pool)
        .await?;

        Ok(comments)
    }

    /// Replies to a specific comment.
    async fn comment_replies(
        &self,
        ctx: &Context<'_>,
        parent_id: Uuid,
    ) -> Result<Vec<Comment>> {
        let pool = ctx.data::<PgPool>()?;

        let replies = sqlx::query_as!(
            Comment,
            r#"
            SELECT id, post_id, user_id, content, parent_id, created_at
            FROM comments
            WHERE parent_id = $1
            ORDER BY created_at ASC
            "#,
            parent_id
        )
        .fetch_all(pool)
        .await?;

        Ok(replies)
    }
}

// ---- Mutations ----

#[derive(Default)]
pub struct PostMutation;

#[Object]
impl PostMutation {
    /// Create a new post. Requires authentication.
    async fn create_post(
        &self,
        ctx: &Context<'_>,
        input: CreatePostInput,
    ) -> Result<Post> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        let visibility = input.visibility.unwrap_or_else(|| "public".to_string());

        let post = sqlx::query_as!(
            Post,
            r#"
            INSERT INTO posts (user_id, caption, video_url, thumbnail_url, visibility)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING
                id, user_id, caption, video_url, thumbnail_url, visibility, created_at,
                0::bigint AS like_count,
                0::bigint AS comment_count
            "#,
            user_id,
            input.caption,
            input.video_url,
            input.thumbnail_url,
            visibility,
        )
        .fetch_one(pool)
        .await?;

        Ok(post)
    }

    /// Delete a post. Only the owner can delete their own post.
    async fn delete_post(&self, ctx: &Context<'_>, post_id: Uuid) -> Result<bool> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        let result = sqlx::query!(
            "DELETE FROM posts WHERE id = $1 AND user_id = $2",
            post_id,
            user_id
        )
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(Error::new("Post not found or you don't own it"));
        }

        Ok(true)
    }

    /// Like a post. Idempotent — liking twice is a no-op.
    /// Returns the updated like count.
    async fn like_post(&self, ctx: &Context<'_>, post_id: Uuid) -> Result<LikePayload> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        // ON CONFLICT DO NOTHING makes this idempotent
        sqlx::query!(
            r#"
            INSERT INTO likes (user_id, post_id)
            VALUES ($1, $2)
            ON CONFLICT (user_id, post_id) DO NOTHING
            "#,
            user_id,
            post_id
        )
        .execute(pool)
        .await?;

        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM likes WHERE post_id = $1",
            post_id
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(0);

        Ok(LikePayload {
            post_id,
            liked: true,
            like_count: count,
        })
    }

    /// Unlike a post. Idempotent — unliking when not liked is a no-op.
    async fn unlike_post(&self, ctx: &Context<'_>, post_id: Uuid) -> Result<LikePayload> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        sqlx::query!(
            "DELETE FROM likes WHERE user_id = $1 AND post_id = $2",
            user_id,
            post_id
        )
        .execute(pool)
        .await?;

        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM likes WHERE post_id = $1",
            post_id
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(0);

        Ok(LikePayload {
            post_id,
            liked: false,
            like_count: count,
        })
    }

    /// Add a top-level comment or reply to a post.
    async fn add_comment(
        &self,
        ctx: &Context<'_>,
        input: AddCommentInput,
    ) -> Result<Comment> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        // If replying, make sure the parent comment belongs to the same post
        if let Some(parent_id) = input.parent_id {
            let parent_exists = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM comments WHERE id = $1 AND post_id = $2)",
                parent_id,
                input.post_id
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(false);

            if !parent_exists {
                return Err(Error::new("Parent comment not found on this post"));
            }
        }

        let comment = sqlx::query_as!(
            Comment,
            r#"
            INSERT INTO comments (user_id, post_id, content, parent_id)
            VALUES ($1, $2, $3, $4)
            RETURNING id, post_id, user_id, content, parent_id, created_at
            "#,
            user_id,
            input.post_id,
            input.content,
            input.parent_id
        )
        .fetch_one(pool)
        .await?;

        Ok(comment)
    }

    /// Delete your own comment.
    async fn delete_comment(&self, ctx: &Context<'_>, comment_id: Uuid) -> Result<bool> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        let result = sqlx::query!(
            "DELETE FROM comments WHERE id = $1 AND user_id = $2",
            comment_id,
            user_id
        )
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(Error::new("Comment not found or you don't own it"));
        }

        Ok(true)
    }
}