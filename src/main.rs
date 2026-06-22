mod db;
mod handlers;

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    Router,
    routing::{get, post},
};
use dotenv::dotenv;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::db::init_db;
use crate::handlers::{create_message, get_message, health_check, list_messages};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive(LevelFilter::INFO.into());

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(env_filter)
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:./medical_chat.db?mode=rwc".to_string());
    let host = std::env::var("HOST")
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()?;

    info!("正在初始化数据库连接: {}", database_url);
    let pool = init_db(&database_url).await?;
    info!("数据库初始化完成");

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .max_age(Duration::from_secs(3600));

    let app = Router::new()
        .route("/api/health", get(health_check))
        .route("/api/messages", post(create_message))
        .route("/api/messages/:id", get(get_message))
        .route(
            "/api/consultations/:consultation_id/messages",
            get(list_messages),
        )
        .with_state(pool)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    info!("服务启动中，监听地址: {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .await?;

    Ok(())
}
