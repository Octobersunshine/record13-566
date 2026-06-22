use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::error;
use uuid::Uuid;

use crate::db::{
    ChatMessage, CreateMessageRequest, PaginatedMessages,
    get_message_by_id, get_messages_by_consultation, save_message,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub code: i32,
    pub message: String,
    pub data: Option<T>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            code: 0,
            message: "success".to_string(),
            data: Some(data),
        }
    }

    pub fn error(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

fn validate_create_request(req: &CreateMessageRequest) -> Result<(), String> {
    if req.consultation_id.trim().is_empty() {
        return Err("问诊ID不能为空".to_string());
    }
    if req.sender_id.trim().is_empty() {
        return Err("发送者ID不能为空".to_string());
    }
    if req.receiver_id.trim().is_empty() {
        return Err("接收者ID不能为空".to_string());
    }
    if req.content.trim().is_empty() && req.image_url.is_none() {
        return Err("消息内容和图片URL不能同时为空".to_string());
    }
    if let Some(url) = &req.image_url {
        if url.trim().is_empty() {
            return Err("图片URL不能为空字符串".to_string());
        }
    }
    Ok(())
}

pub async fn create_message(
    State(pool): State<SqlitePool>,
    Json(req): Json<CreateMessageRequest>,
) -> impl IntoResponse {
    if let Err(err_msg) = validate_create_request(&req) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<ChatMessage>::error(400, err_msg)),
        );
    }

    match save_message(&pool, &req).await {
        Ok(message) => (
            StatusCode::CREATED,
            Json(ApiResponse::success(message)),
        ),
        Err(e) => {
            error!("保存消息失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<ChatMessage>::error(500, format!("保存消息失败: {}", e))),
            )
        }
    }
}

pub async fn list_messages(
    State(pool): State<SqlitePool>,
    axum::extract::Path(consultation_id): axum::extract::Path<String>,
    Query(query): Query<PaginationQuery>,
) -> impl IntoResponse {
    if consultation_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<PaginatedMessages>::error(400, "问诊ID不能为空")),
        );
    }

    let page = query.page.unwrap_or(1);
    let page_size = query.page_size.unwrap_or(20);

    if page < 1 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<PaginatedMessages>::error(400, "页码必须大于等于1")),
        );
    }
    if page_size < 1 || page_size > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<PaginatedMessages>::error(400, "每页数量必须在1到100之间")),
        );
    }

    match get_messages_by_consultation(&pool, &consultation_id, page, page_size).await {
        Ok(paginated) => (
            StatusCode::OK,
            Json(ApiResponse::success(paginated)),
        ),
        Err(e) => {
            error!("查询消息列表失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<PaginatedMessages>::error(500, format!("查询消息失败: {}", e))),
            )
        }
    }
}

pub async fn get_message(
    State(pool): State<SqlitePool>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    if id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<ChatMessage>::error(400, "消息ID不能为空")),
        );
    }

    if Uuid::parse_str(&id).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<ChatMessage>::error(400, "消息ID格式无效")),
        );
    }

    match get_message_by_id(&pool, &id).await {
        Ok(Some(message)) => (
            StatusCode::OK,
            Json(ApiResponse::success(message)),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<ChatMessage>::error(404, "消息不存在")),
        ),
        Err(e) => {
            error!("查询消息失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<ChatMessage>::error(500, format!("查询消息失败: {}", e))),
            )
        }
    }
}

pub async fn health_check() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(ApiResponse::success(serde_json::json!({ "status": "ok" }))),
    )
}
