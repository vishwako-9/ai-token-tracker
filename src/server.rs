use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::Query,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::Deserialize;

use crate::db::Database;

const DASHBOARD_HTML: &str = include_str!("../assets/dashboard.html");

#[derive(Deserialize)]
struct DaysParam {
    days: Option<u32>,
    provider: Option<String>,
}

fn open_db(path: &str) -> Result<Database> {
    // Reuse the same Database wrapper; queries are cheap and the local tool is
    // single-user, so a fresh read connection per request is safe and simple.
    Database::open(path)
}

async fn handle_index() -> impl IntoResponse {
    axum::response::Html(DASHBOARD_HTML)
}

async fn handle_health() -> impl IntoResponse {
    axum::Json(serde_json::json!({ "ok": true }))
}

async fn handle_usage(Query(q): Query<DaysParam>, axum::extract::State(state): axum::extract::State<Arc<AppState>>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let db = open_db(&state.db_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let days = q.days.unwrap_or(30);
    let rows = db
        .query_summary(days, q.provider.as_deref(), None)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::Json(serde_json::json!({ "days": days, "rows": rows })))
}

async fn handle_daily(Query(q): Query<DaysParam>, axum::extract::State(state): axum::extract::State<Arc<AppState>>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let db = open_db(&state.db_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let days = q.days.unwrap_or(90);
    let rows = db
        .query_daily(days, q.provider.as_deref(), None, None, None)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::Json(serde_json::json!({ "days": days, "rows": rows })))
}

#[derive(Clone)]
struct AppState {
    db_path: String,
}

pub async fn serve(db_path: &str, port: Option<u16>) -> Result<()> {
    let port = port.unwrap_or(7680);
    let state = Arc::new(AppState {
        db_path: db_path.to_string(),
    });

    let app = Router::new()
        .route("/", get(handle_index))
        .route("/api/health", get(handle_health))
        .route("/api/usage", get(handle_usage))
        .route("/api/daily", get(handle_daily))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("tokentracker dashboard: http://{}", listener.local_addr()?);
    println!("  usage:   /api/usage?days=30");
    println!("  daily:   /api/daily?days=90");
    println!("Press Ctrl+C to stop.");

    axum::serve(listener, app).await?;
    Ok(())
}