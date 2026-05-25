use async_graphql::*;
use super::resolvers::user::{UserQuery, UserMutation};
use super::resolvers::post::{PostQuery, PostMutation};
use super::resolvers::job::{JobQuery, JobMutation};
use super::resolvers::message::{MessageQuery, MessageMutation};

// Merge all queries here
#[derive(MergedObject, Default)]
pub struct QueryRoot(UserQuery, PostQuery, JobQuery, MessageQuery);

// Merge all mutations here
#[derive(MergedObject, Default)]
pub struct MutationRoot(UserMutation, PostMutation, JobMutation, MessageMutation);

pub type AppSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

pub fn build_schema(pool: sqlx::PgPool) -> AppSchema {
    Schema::build(
        QueryRoot::default(),
        MutationRoot::default(),
        EmptySubscription,
    )
    .data(pool)
    .finish()
}