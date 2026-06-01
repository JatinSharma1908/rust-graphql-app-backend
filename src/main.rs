mod db;
mod graphql;

use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_axum::{
    GraphQLProtocol, GraphQLRequest, GraphQLResponse, GraphQLWebSocket,
};
use axum::{
    extract::{Query, State, WebSocketUpgrade},
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use graphql::schema::{build_schema, AppSchema};
use std::collections::HashMap;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

// Decode a raw JWT string → Uuid.  Shared by all three auth paths.
fn decode_jwt(token: &str) -> Option<Uuid> {
    use jsonwebtoken::{decode, DecodingKey, Validation};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct Claims {
        sub: String,
        exp: usize,
    }

    let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "secret".to_string());
    let decoded = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .ok()?;
    Uuid::parse_str(&decoded.claims.sub).ok()
}

// Path 1: "Authorization: Bearer <jwt>" HTTP header
fn extract_user_id(headers: &HeaderMap) -> Option<Uuid> {
    let auth = headers.get("Authorization")?.to_str().ok()?;
    let token = auth.strip_prefix("Bearer ")?;
    decode_jwt(token)
}

// GraphQL Playground handler
async fn playground() -> Html<String> {
    Html(playground_source(
        GraphQLPlaygroundConfig::new("/graphql").subscription_endpoint("/ws"),
    ))
}

// HTTP GraphQL handler — reads JWT from Authorization header
async fn graphql_handler(
    State(schema): State<AppSchema>,
    headers: HeaderMap,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let mut request = req.into_inner();
    if let Some(user_id) = extract_user_id(&headers) {
        request = request.data(user_id);
    }
    schema.execute(request).await.into()
}

// WebSocket subscription handler (async-graphql-axum 7.x API).
//
// Auth priority (first match wins):
//   1. ?token=<jwt> query param   ← most reliable; use this in Postman URL bar
//   2. Authorization: Bearer      ← HTTP upgrade header
//   3. connection_init payload    ← graphql-ws protocol clients (wscat, etc.)
async fn ws_handler(
    State(schema): State<AppSchema>,
    protocol: GraphQLProtocol,
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    // Resolve user identity from HTTP-level sources before the upgrade moves the request.
    let user_id = params
        .get("token")
        .and_then(|t| decode_jwt(t))
        .or_else(|| extract_user_id(&headers));

    ws.protocols(["graphql-transport-ws", "graphql-ws"])
        .on_upgrade(move |socket| async move {
            GraphQLWebSocket::new(socket, schema, protocol)
                .on_connection_init(move |payload: serde_json::Value| async move {
                    let mut data = async_graphql::Data::default();

                    // Priority 1 & 2: query-param or Authorization header
                    if let Some(uid) = user_id {
                        data.insert(uid);
                        return Ok::<_, async_graphql::Error>(data);
                    }

                    // Priority 3: connection_init payload — { "Authorization": "Bearer <jwt>" }
                    if let Some(auth) = payload
                        .get("Authorization")
                        .and_then(|v| v.as_str())
                    {
                        if let Some(token) = auth.strip_prefix("Bearer ") {
                            if let Some(uid) = decode_jwt(token) {
                                data.insert(uid);
                            }
                        }
                    }

                    Ok::<_, async_graphql::Error>(data)
                })
                .serve()
                .await
        })
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
        .route("/ws", get(ws_handler))
        .route("/", get(playground))
        .layer(CorsLayer::permissive())
        .with_state(schema);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000")
        .await
        .unwrap();

    tracing::info!("GraphQL Playground → http://localhost:8000");
    tracing::info!("WebSocket subscriptions → ws://localhost:8000/ws");
    tracing::info!("  auth via query param  → ws://localhost:8000/ws?token=<jwt>");
    axum::serve(listener, app).await.unwrap();
}