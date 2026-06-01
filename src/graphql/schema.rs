use async_graphql::*;
use tokio::sync::broadcast;
use super::resolvers::user::{UserQuery, UserMutation};
use super::resolvers::post::{PostQuery, PostMutation};
use super::resolvers::job::{JobQuery, JobMutation};
use super::resolvers::message::{MessageQuery, MessageMutation, MessageSubscription, Message};

// Merge all queries here
#[derive(MergedObject, Default)]
pub struct QueryRoot(UserQuery, PostQuery, JobQuery, MessageQuery);

// Merge all mutations here
#[derive(MergedObject, Default)]
pub struct MutationRoot(UserMutation, PostMutation, JobMutation, MessageMutation);

pub type AppSchema = Schema<QueryRoot, MutationRoot, MessageSubscription>;

pub fn build_schema(pool: sqlx::PgPool) -> AppSchema {
    // Channel capacity: 100 means up to 100 messages can be queued
    // before a slow subscriber starts dropping (lagged error).
    let (tx, _rx) = broadcast::channel::<Message>(100);

    Schema::build(
        QueryRoot::default(),
        MutationRoot::default(),
        MessageSubscription::default(),
    )
    .data(pool)
    .data(tx)   // injected as broadcast::Sender<Message> — both mutation and subscription use this
    .finish()
}