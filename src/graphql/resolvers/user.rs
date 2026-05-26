use async_graphql::*;
use chrono::NaiveDateTime;
use sqlx::PgPool;
use uuid::Uuid;

// ---- Types ----

#[derive(SimpleObject, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub profile_pic: Option<String>,
    pub phone_number: Option<String>,
    pub status: Option<String>,
    pub role: Option<String>,
    pub follower_count: Option<i32>,
    pub following_count: Option<i32>,
    pub created_at: Option<NaiveDateTime>,
}

#[derive(SimpleObject, sqlx::FromRow)]
pub struct UserDetail {
    pub user_id: Uuid,
    pub tech_stack: Option<String>,
    pub projects: Option<String>,
    pub experience: Option<String>,
    pub certificates: Option<String>,
    pub cv: Option<String>,
    pub roles: Option<String>,
}

/// User card returned in search results
#[derive(SimpleObject, sqlx::FromRow)]
pub struct UserSearchResult {
    pub id: Uuid,
    pub username: String,
    pub profile_pic: Option<String>,
    pub follower_count: Option<i32>,
}

// ---- Inputs ----

#[derive(InputObject)]
pub struct RegisterInput {
    pub username: String,
    pub email: String,
    pub password: String,
    pub phone_number: Option<String>,
}

#[derive(InputObject)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
}

#[derive(InputObject)]
pub struct UpdateProfileInput {
    pub username: Option<String>,
    pub profile_pic: Option<String>,
    pub phone_number: Option<String>,
}

#[derive(InputObject)]
pub struct UpdateUserDetailInput {
    pub tech_stack: Option<String>,
    pub projects: Option<String>,
    pub experience: Option<String>,
    pub certificates: Option<String>,
    pub cv: Option<String>,
    pub roles: Option<String>,
}

#[derive(SimpleObject)]
pub struct AuthPayload {
    pub token: String,
    pub user: User,
}

// ---- Queries ----

#[derive(Default)]
pub struct UserQuery;

#[Object]
impl UserQuery {
    /// Get the logged-in user's profile
    async fn me(&self, ctx: &Context<'_>) -> Result<User> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        let user = sqlx::query_as!(
            User,
            r#"SELECT id, username, email, profile_pic, phone_number,
                    status, role, follower_count, following_count, created_at
             FROM users WHERE id = $1"#,
            user_id
        )
        .fetch_one(pool)
        .await?;

        Ok(user)
    }

    /// Get any user by id
    async fn user(&self, ctx: &Context<'_>, id: Uuid) -> Result<User> {
        let pool = ctx.data::<PgPool>()?;

        let user = sqlx::query_as!(
            User,
            r#"SELECT id, username, email, profile_pic, phone_number,
                    status, role, follower_count, following_count, created_at
             FROM users WHERE id = $1"#,
            id
        )
        .fetch_one(pool)
        .await?;

        Ok(user)
    }

    /// Get the detail profile (tech stack, CV, projects etc.) for any user
    async fn user_detail(&self, ctx: &Context<'_>, user_id: Uuid) -> Result<UserDetail> {
        let pool = ctx.data::<PgPool>()?;

        let detail = sqlx::query_as!(
            UserDetail,
            r#"SELECT user_id, tech_stack, projects, experience, certificates, cv, roles
               FROM user_detail WHERE user_id = $1"#,
            user_id
        )
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Error::new("User detail not found"))?;

        Ok(detail)
    }

    /// Search users by username (case-insensitive, partial match).
    /// Returns up to 20 results by default.
    async fn search_users(
        &self,
        ctx: &Context<'_>,
        query: String,
        limit: Option<i32>,
    ) -> Result<Vec<UserSearchResult>> {
        let pool = ctx.data::<PgPool>()?;

        let limit = limit.unwrap_or(20).min(50) as i64;
        let pattern = format!("%{}%", query.to_lowercase());

        let results = sqlx::query_as!(
            UserSearchResult,
            r#"
            SELECT id, username, profile_pic, follower_count
            FROM users
            WHERE LOWER(username) LIKE $1
              AND status = 'active'
            ORDER BY follower_count DESC NULLS LAST
            LIMIT $2
            "#,
            pattern,
            limit
        )
        .fetch_all(pool)
        .await?;

        Ok(results)
    }

    /// Check if the logged-in user follows another user
    async fn is_following(&self, ctx: &Context<'_>, target_id: Uuid) -> Result<bool> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        let exists = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM user_follows
                WHERE follower_id = $1 AND following_id = $2
            )
            "#,
            user_id,
            target_id
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);

        Ok(exists)
    }

    /// List users that a given user follows
    async fn following(
        &self,
        ctx: &Context<'_>,
        user_id: Uuid,
    ) -> Result<Vec<UserSearchResult>> {
        let pool = ctx.data::<PgPool>()?;

        let users = sqlx::query_as!(
            UserSearchResult,
            r#"
            SELECT u.id, u.username, u.profile_pic, u.follower_count
            FROM user_follows uf
            JOIN users u ON u.id = uf.following_id
            WHERE uf.follower_id = $1
            ORDER BY uf.created_at DESC
            "#,
            user_id
        )
        .fetch_all(pool)
        .await?;

        Ok(users)
    }

    /// List followers of a given user
    async fn followers(
        &self,
        ctx: &Context<'_>,
        user_id: Uuid,
    ) -> Result<Vec<UserSearchResult>> {
        let pool = ctx.data::<PgPool>()?;

        let users = sqlx::query_as!(
            UserSearchResult,
            r#"
            SELECT u.id, u.username, u.profile_pic, u.follower_count
            FROM user_follows uf
            JOIN users u ON u.id = uf.follower_id
            WHERE uf.following_id = $1
            ORDER BY uf.created_at DESC
            "#,
            user_id
        )
        .fetch_all(pool)
        .await?;

        Ok(users)
    }
}

// ---- Mutations ----

#[derive(Default)]
pub struct UserMutation;

#[Object]
impl UserMutation {
    async fn register(
        &self,
        ctx: &Context<'_>,
        input: RegisterInput,
    ) -> Result<AuthPayload> {
        use argon2::password_hash::{rand_core::OsRng, SaltString};
        use argon2::{Argon2, PasswordHasher};
        use jsonwebtoken::{encode, EncodingKey, Header};
        use serde::{Deserialize, Serialize};

        let pool = ctx.data::<PgPool>()?;

        let salt = SaltString::generate(&mut OsRng);
        let hashed = Argon2::default()
            .hash_password(input.password.as_bytes(), &salt)
            .map_err(|e| Error::new(e.to_string()))?
            .to_string();

        let user = sqlx::query_as!(
            User,
            r#"INSERT INTO users (username, email, password, phone_number)
             VALUES ($1, $2, $3, $4)
             RETURNING id, username, email, profile_pic, phone_number,
                       status, role, follower_count, following_count, created_at"#,
            input.username,
            input.email,
            hashed,
            input.phone_number
        )
        .fetch_one(pool)
        .await?;

        #[derive(Serialize, Deserialize)]
        struct Claims { sub: String, exp: usize }

        let claims = Claims { sub: user.id.to_string(), exp: 9999999999 };
        let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "secret".to_string());
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .map_err(|e| Error::new(e.to_string()))?;

        Ok(AuthPayload { token, user })
    }

    async fn login(&self, ctx: &Context<'_>, input: LoginInput) -> Result<AuthPayload> {
        use argon2::password_hash::PasswordHash;
        use argon2::{Argon2, PasswordVerifier};
        use jsonwebtoken::{encode, EncodingKey, Header};
        use serde::{Deserialize, Serialize};

        let pool = ctx.data::<PgPool>()?;

        let row = sqlx::query!(
            "SELECT id, password FROM users WHERE email = $1",
            input.email
        )
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Error::new("Invalid email or password"))?;

        let parsed_hash =
            PasswordHash::new(&row.password).map_err(|e| Error::new(e.to_string()))?;
        Argon2::default()
            .verify_password(input.password.as_bytes(), &parsed_hash)
            .map_err(|_| Error::new("Invalid email or password"))?;

        let user = sqlx::query_as!(
            User,
            r#"SELECT id, username, email, profile_pic, phone_number,
                    status, role, follower_count, following_count, created_at
             FROM users WHERE id = $1"#,
            row.id
        )
        .fetch_one(pool)
        .await?;

        #[derive(Serialize, Deserialize)]
        struct Claims { sub: String, exp: usize }

        let claims = Claims { sub: user.id.to_string(), exp: 9999999999 };
        let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "secret".to_string());
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .map_err(|e| Error::new(e.to_string()))?;

        Ok(AuthPayload { token, user })
    }

    /// Follow a user. Idempotent — following twice returns an error.
    async fn follow_user(&self, ctx: &Context<'_>, following_id: Uuid) -> Result<bool> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        if user_id == &following_id {
            return Err(Error::new("You cannot follow yourself"));
        }

        // Check not already following
        let already = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM user_follows WHERE follower_id = $1 AND following_id = $2)",
            user_id,
            following_id
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);

        if already {
            return Err(Error::new("Already following this user"));
        }

        let mut tx = pool.begin().await?;

        sqlx::query!(
            "INSERT INTO user_follows (follower_id, following_id) VALUES ($1, $2)",
            user_id,
            following_id
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            "UPDATE users SET following_count = following_count + 1 WHERE id = $1",
            user_id
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            "UPDATE users SET follower_count = follower_count + 1 WHERE id = $1",
            following_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(true)
    }

    /// Unfollow a user. Decrements both follower/following counts atomically.
    async fn unfollow_user(&self, ctx: &Context<'_>, following_id: Uuid) -> Result<bool> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        let mut tx = pool.begin().await?;

        let result = sqlx::query!(
            "DELETE FROM user_follows WHERE follower_id = $1 AND following_id = $2",
            user_id,
            following_id
        )
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() == 0 {
            return Err(Error::new("You are not following this user"));
        }

        sqlx::query!(
            "UPDATE users SET following_count = GREATEST(following_count - 1, 0) WHERE id = $1",
            user_id
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            "UPDATE users SET follower_count = GREATEST(follower_count - 1, 0) WHERE id = $1",
            following_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(true)
    }

    /// Update basic profile fields. Only updates fields that are provided.
    async fn update_profile(
        &self,
        ctx: &Context<'_>,
        input: UpdateProfileInput,
    ) -> Result<User> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        // Check username uniqueness if changing it
        if let Some(ref new_username) = input.username {
            let taken = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM users WHERE username = $1 AND id != $2)",
                new_username,
                user_id
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(false);

            if taken {
                return Err(Error::new("Username already taken"));
            }
        }

        let user = sqlx::query_as!(
            User,
            r#"
            UPDATE users SET
                username     = COALESCE($1, username),
                profile_pic  = COALESCE($2, profile_pic),
                phone_number = COALESCE($3, phone_number)
            WHERE id = $4
            RETURNING id, username, email, profile_pic, phone_number,
                      status, role, follower_count, following_count, created_at
            "#,
            input.username,
            input.profile_pic,
            input.phone_number,
            user_id
        )
        .fetch_one(pool)
        .await?;

        Ok(user)
    }

    /// Upsert the detail profile (tech stack, CV, projects etc.).
    /// Creates the row if it doesn't exist, updates it if it does.
    async fn update_user_detail(
        &self,
        ctx: &Context<'_>,
        input: UpdateUserDetailInput,
    ) -> Result<UserDetail> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

        let detail = sqlx::query_as!(
            UserDetail,
            r#"
            INSERT INTO user_detail (user_id, tech_stack, projects, experience, certificates, cv, roles)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (user_id) DO UPDATE SET
                tech_stack   = COALESCE(EXCLUDED.tech_stack,   user_detail.tech_stack),
                projects     = COALESCE(EXCLUDED.projects,     user_detail.projects),
                experience   = COALESCE(EXCLUDED.experience,   user_detail.experience),
                certificates = COALESCE(EXCLUDED.certificates, user_detail.certificates),
                cv           = COALESCE(EXCLUDED.cv,           user_detail.cv),
                roles        = COALESCE(EXCLUDED.roles,        user_detail.roles)
            RETURNING user_id, tech_stack, projects, experience, certificates, cv, roles
            "#,
            user_id,
            input.tech_stack,
            input.projects,
            input.experience,
            input.certificates,
            input.cv,
            input.roles,
        )
        .fetch_one(pool)
        .await?;

        Ok(detail)
    }
}