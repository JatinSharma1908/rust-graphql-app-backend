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
    // get logged in user
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

    // get any user by id
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

        // hash password
        let salt = SaltString::generate(&mut OsRng);
        let hashed = Argon2::default()
            .hash_password(input.password.as_bytes(), &salt)
            .map_err(|e| Error::new(e.to_string()))?
            .to_string();

        // insert user
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

        // generate JWT
        #[derive(Serialize, Deserialize)]
        struct Claims {
            sub: String,
            exp: usize,
        }

        let claims = Claims {
            sub: user.id.to_string(),
            exp: 9999999999,
        };

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

        // fetch user + password hash
        let row = sqlx::query!(
            "SELECT id, password FROM users WHERE email = $1",
            input.email
        )
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Error::new("Invalid email or password"))?;

        // verify password
        let parsed_hash =
            PasswordHash::new(&row.password).map_err(|e| Error::new(e.to_string()))?;
        Argon2::default()
            .verify_password(input.password.as_bytes(), &parsed_hash)
            .map_err(|_| Error::new("Invalid email or password"))?;

        // fetch full user
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
        struct Claims {
            sub: String,
            exp: usize,
        }

        let claims = Claims {
            sub: user.id.to_string(),
            exp: 9999999999,
        };

        let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "secret".to_string());
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .map_err(|e| Error::new(e.to_string()))?;

        Ok(AuthPayload { token, user })
    }

    async fn follow_user(&self, ctx: &Context<'_>, following_id: Uuid) -> Result<bool> {
        let pool = ctx.data::<PgPool>()?;
        let user_id = ctx.data::<Uuid>()?;

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
}