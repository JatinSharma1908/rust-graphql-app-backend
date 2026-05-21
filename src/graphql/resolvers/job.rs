use async_graphql::*;
use chrono::NaiveDateTime;
use sqlx::PgPool;
use uuid::Uuid;

// ---- Types ----

/// Summary card shown in job listings
#[derive(SimpleObject, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub title: String,
    pub company_name: Option<String>,
    pub location: Option<String>,
    pub experience_required: Option<String>,
    pub created_at: Option<NaiveDateTime>,
}

/// Full detail page — joins jobs + jobs_detail
#[derive(SimpleObject)]
pub struct JobDetail {
    pub id: Uuid,
    pub title: String,
    pub company_name: Option<String>,
    pub location: Option<String>,
    pub experience_required: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    // from jobs_detail
    pub description: Option<String>,
    pub requirements: Option<String>,
    pub package: Option<String>,
    pub job_type: Option<String>,
}

// ---- Inputs ----

#[derive(InputObject)]
pub struct JobFilterInput {
    pub location: Option<String>,
    pub experience_required: Option<String>,
    /// matches jobs_detail.type e.g. "full-time", "remote", "contract"
    pub job_type: Option<String>,
    /// simple keyword search on title or company name
    pub search: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

#[derive(InputObject)]
pub struct CreateJobInput {
    pub title: String,
    pub company_name: Option<String>,
    pub location: Option<String>,
    pub experience_required: Option<String>,
    // detail fields
    pub description: Option<String>,
    pub requirements: Option<String>,
    pub package: Option<String>,
    pub job_type: Option<String>,
}

// ---- Queries ----

#[derive(Default)]
pub struct JobQuery;

#[Object]
impl JobQuery {
    /// List jobs with optional filters. Returns summary cards, newest first.
    async fn jobs(
        &self,
        ctx: &Context<'_>,
        filter: Option<JobFilterInput>,
    ) -> Result<Vec<Job>> {
        let pool = ctx.data::<PgPool>()?;

        let limit = filter
            .as_ref()
            .and_then(|f| f.limit)
            .unwrap_or(20)
            .min(100) as i64;

        let offset = filter
            .as_ref()
            .and_then(|f| f.offset)
            .unwrap_or(0) as i64;

        let location   = filter.as_ref().and_then(|f| f.location.clone());
        let experience = filter.as_ref().and_then(|f| f.experience_required.clone());
        let job_type   = filter.as_ref().and_then(|f| f.job_type.clone());
        let search     = filter.as_ref().and_then(|f| f.search.clone())
            .map(|s| format!("%{}%", s.to_lowercase()));

        // We JOIN jobs_detail only when a type filter is requested,
        // otherwise a plain scan on jobs is cheaper.
        let jobs = if job_type.is_some() {
            sqlx::query_as!(
                Job,
                r#"
                SELECT DISTINCT
                    j.id, j.title, j.company_name, j.location,
                    j.experience_required, j.created_at
                FROM jobs j
                LEFT JOIN jobs_detail jd ON jd.job_id = j.id
                WHERE ($1::text IS NULL OR LOWER(j.location)          LIKE LOWER($1))
                  AND ($2::text IS NULL OR LOWER(j.experience_required) LIKE LOWER($2))
                  AND ($3::text IS NULL OR LOWER(jd.type)              LIKE LOWER($3))
                  AND ($4::text IS NULL OR (
                        LOWER(j.title)        LIKE $4
                     OR LOWER(j.company_name) LIKE $4
                  ))
                ORDER BY j.created_at DESC
                LIMIT $5 OFFSET $6
                "#,
                location,
                experience,
                job_type,
                search,
                limit,
                offset
            )
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as!(
                Job,
                r#"
                SELECT
                    j.id, j.title, j.company_name, j.location,
                    j.experience_required, j.created_at
                FROM jobs j
                WHERE ($1::text IS NULL OR LOWER(j.location)            LIKE LOWER($1))
                  AND ($2::text IS NULL OR LOWER(j.experience_required)  LIKE LOWER($2))
                  AND ($3::text IS NULL OR (
                        LOWER(j.title)        LIKE $3
                     OR LOWER(j.company_name) LIKE $3
                  ))
                ORDER BY j.created_at DESC
                LIMIT $4 OFFSET $5
                "#,
                location,
                experience,
                search,
                limit,
                offset
            )
            .fetch_all(pool)
            .await?
        };

        Ok(jobs)
    }

    /// Single job with full detail (description, requirements, package, type).
    async fn job(&self, ctx: &Context<'_>, id: Uuid) -> Result<JobDetail> {
        let pool = ctx.data::<PgPool>()?;

        // LEFT JOIN so the query succeeds even if jobs_detail row is missing
        let row = sqlx::query!(
            r#"
            SELECT
                j.id, j.title, j.company_name, j.location,
                j.experience_required, j.created_at,
                jd.description, jd.requirements, jd.package,
                jd.type AS job_type
            FROM jobs j
            LEFT JOIN jobs_detail jd ON jd.job_id = j.id
            WHERE j.id = $1
            "#,
            id
        )
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Error::new("Job not found"))?;

        Ok(JobDetail {
            id: row.id,
            title: row.title,
            company_name: row.company_name,
            location: row.location,
            experience_required: row.experience_required,
            created_at: row.created_at,
            description: row.description,
            requirements: row.requirements,
            package: row.package,
            job_type: row.job_type,
        })
    }
}

// ---- Mutations ----

#[derive(Default)]
pub struct JobMutation;

#[Object]
impl JobMutation {
    /// Create a job listing with its detail in a single transaction.
    /// Requires authentication. You can add role-gating here later.
    async fn create_job(
        &self,
        ctx: &Context<'_>,
        input: CreateJobInput,
    ) -> Result<JobDetail> {
        let pool = ctx.data::<PgPool>()?;
        // require login — even if we don't store poster_id yet
        let _user_id = ctx.data::<Uuid>()?;

        let mut tx = pool.begin().await?;

        // insert into jobs
        let job = sqlx::query!(
            r#"
            INSERT INTO jobs (title, company_name, location, experience_required)
            VALUES ($1, $2, $3, $4)
            RETURNING id, title, company_name, location, experience_required, created_at
            "#,
            input.title,
            input.company_name,
            input.location,
            input.experience_required,
        )
        .fetch_one(&mut *tx)
        .await?;

        // insert into jobs_detail (always create the row so detail query never returns null)
        sqlx::query!(
            r#"
            INSERT INTO jobs_detail
                (job_id, description, requirements, company_name, package, location, type)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            job.id,
            input.description,
            input.requirements,
            input.company_name,   // denormalised copy — keeps detail self-contained
            input.package,
            input.location,
            input.job_type,
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(JobDetail {
            id: job.id,
            title: job.title,
            company_name: job.company_name,
            location: job.location,
            experience_required: job.experience_required,
            created_at: job.created_at,
            description: input.description,
            requirements: input.requirements,
            package: input.package,
            job_type: input.job_type,
        })
    }

    /// Delete a job and its detail (cascades via FK). Requires authentication.
    async fn delete_job(&self, ctx: &Context<'_>, job_id: Uuid) -> Result<bool> {
        let pool = ctx.data::<PgPool>()?;
        let _user_id = ctx.data::<Uuid>()?;

        let result = sqlx::query!(
            "DELETE FROM jobs WHERE id = $1",
            job_id
        )
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(Error::new("Job not found"));
        }

        Ok(true)
    }
}