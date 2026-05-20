mod db;
mod graphql;

use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    extract::State,
    http::HeaderMap,
    response::Html,
    routing::{get, post},
    Router,
};
use graphql::schema::{build_schema, AppSchema};
use tower_http::cors::CorsLayer;

// GraphQL Playground handler
async fn playground() -> Html<String> {
    Html(playground_source(GraphQLPlaygroundConfig::new("/graphql")))
}

// GraphQL handler
async fn graphql_handler(
    State(schema): State<AppSchema>,
    headers: HeaderMap,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let mut request = req.into_inner();

    // extract JWT from Authorization header and inject user_id
    if let Some(auth) = headers.get("Authorization") {
        if let Ok(auth_str) = auth.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                use jsonwebtoken::{decode, DecodingKey, Validation};
                use serde::{Deserialize, Serialize};
                use uuid::Uuid;

                #[derive(Serialize, Deserialize)]
                struct Claims {
                    sub: String,
                    exp: usize,
                }

                let secret = std::env::var("JWT_SECRET").unwrap_or_default();
                if let Ok(data) = decode::<Claims>(
                    token,
                    &DecodingKey::from_secret(secret.as_bytes()),
                    &Validation::default(),
                ) {
                    if let Ok(user_id) = Uuid::parse_str(&data.claims.sub) {
                        request = request.data(user_id);
                    }
                }
            }
        }
    }

    schema.execute(request).await.into()
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let pool = db::create_pool().await;
    tracing::info!("Connected to database ✓");

    let schema = build_schema(pool);

    let app = Router::new()
        .route("/graphql", post(graphql_handler))
        .route("/", get(playground))
        .layer(CorsLayer::permissive())
        .with_state(schema);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000")
        .await
        .unwrap();

    tracing::info!("GraphQL Playground → http://localhost:8000");
    axum::serve(listener, app).await.unwrap();
}