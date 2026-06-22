mod db;
mod handlers;
mod upload;
mod summary;

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

use crate::db::{init_db, init_summary_table};
use crate::handlers::{create_message, create_summary, get_message, get_summary, health_check, list_messages, upload_image};
use crate::upload::UploadConfig;
use crate::summary::{SummaryConfig, SummaryService};

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub upload_config: Arc<UploadConfig>,
    pub summary_service: Arc<SummaryService>,
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
        .unwrap_or_else(|_| "0.0.0.0".to_string());
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

    info!("正在初始化小结数据表");
    init_summary_table(&pool).await?;
    info!("小结数据表初始化完成");

    let llm_api_url = std::env::var("LLM_API_URL").ok();
    let llm_api_key = std::env::var("LLM_API_KEY").ok();
    let llm_model = std::env::var("LLM_MODEL")
        .unwrap_or_else(|_| "qwen-plus".to_string());
    let llm_timeout_secs: u64 = std::env::var("LLM_TIMEOUT_SECS")
        .unwrap_or_else(|_| "30".to_string())
        .parse()
        .unwrap_or(30);
    let use_template_fallback = std::env::var("USE_TEMPLATE_FALLBACK")
        .unwrap_or_else(|_| "true".to_string())
        .to_lowercase()
        == "true";

    let summary_config = SummaryConfig {
        llm_api_url,
        llm_api_key,
        llm_model,
        llm_timeout_secs,
        use_template_fallback,
    };

    let summary_service = SummaryService::with_config(summary_config);
    if summary_service.config.llm_api_url.is_some() && summary_service.config.llm_api_key.is_some() {
        info!(
            "LLM 摘要服务已配置: model={}, timeout={}s",
            summary_service.config.llm_model,
            summary_service.config.llm_timeout_secs
        );
    } else {
        info!("未配置 LLM API，将使用模板模式生成问诊小结");
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .max_age(Duration::from_secs(3600));

    let app_state = AppState {
        pool: pool.clone(),
        upload_config: upload_config.clone(),
        summary_service: summary_service.clone(),
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
        .route("/api/summaries", post(create_summary))
        .route("/api/consultations/:consultation_id/summary", get(get_summary))
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
