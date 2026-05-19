use async_graphql::*;
use super::resolvers::user::{UserQuery, UserMutation};

// Merge all queries here
#[derive(MergedObject, Default)]
pub struct QueryRoot(UserQuery);

// Merge all mutations here
#[derive(MergedObject, Default)]
pub struct MutationRoot(UserMutation);

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