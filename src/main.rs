mod db;
mod handlers;
mod upload;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    routing::{get, post},
};
use dotenv::dotenv;
use sqlx::SqlitePool;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::db::init_db;
use crate::handlers::{create_message, get_message, health_check, list_messages, upload_image};
use crate::upload::UploadConfig;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub upload_config: Arc<UploadConfig>,
}

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

    let upload_dir = std::env::var("UPLOAD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./uploads"));
    let max_file_size_mb: u64 = std::env::var("MAX_FILE_SIZE_MB")
        .unwrap_or_else(|_| "10".to_string())
        .parse()
        .unwrap_or(10);

    let mut upload_config = UploadConfig::new(upload_dir.clone(), max_file_size_mb);

    let extra_types = std::env::var("ALLOWED_CONTENT_TYPES").ok();
    if let Some(types) = extra_types {
        let extra: Vec<String> = types
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !extra.is_empty() {
            upload_config.allowed_types.extend(extra);
        }
    }

    info!("正在初始化上传目录: {}", upload_dir.display());
    upload_config.ensure_dirs().await?;
    let upload_config = Arc::new(upload_config);
    info!(
        "上传配置初始化完成，目录={}, 最大文件={}MB, 允许类型={:?}",
        upload_dir.display(),
        max_file_size_mb,
        upload_config.allowed_types
    );

    info!("正在初始化数据库连接: {}", database_url);
    let pool = init_db(&database_url).await?;
    info!("数据库初始化完成");

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .max_age(Duration::from_secs(3600));

    let app_state = AppState {
        pool: pool.clone(),
        upload_config: upload_config.clone(),
    };

    let app = Router::new()
        .route("/api/health", get(health_check))
        .route("/api/upload", post(upload_image))
        .route("/api/messages", post(create_message))
        .route("/api/messages/:id", get(get_message))
        .route(
            "/api/consultations/:consultation_id/messages",
            get(list_messages),
        )
        .nest_service("/uploads", ServeDir::new(upload_dir.clone()))
        .with_state(app_state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    info!("服务启动中，监听地址: {}", addr);
    info!("静态文件服务: /uploads -> {}", upload_dir.canonicalize().unwrap_or(upload_dir).display());

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .await?;

    Ok(())
}
