use std::sync::Arc;

use axum::{
    extract::{Multipart, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use uuid::Uuid;

use crate::AppState;
use crate::db::{
    ChatMessage, CreateMessageRequest, PaginatedMessages,
    get_message_by_id, get_messages_by_consultation, save_message,
};
use crate::upload::{UploadedFile, save_uploaded_file};

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
    State(state): State<AppState>,
    Json(req): Json<CreateMessageRequest>,
) -> impl IntoResponse {
    if let Err(err_msg) = validate_create_request(&req) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<ChatMessage>::error(400, err_msg)),
        );
    }

    match save_message(&state.pool, &req).await {
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
    State(state): State<AppState>,
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

    match get_messages_by_consultation(&state.pool, &consultation_id, page, page_size).await {
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
    State(state): State<AppState>,
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

    match get_message_by_id(&state.pool, &id).await {
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

pub async fn upload_image(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut uploaded_files: Vec<UploadedFile> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let field_name = field.name().unwrap_or("file").to_string();
        let file_name = field.file_name().unwrap_or("unknown").to_string();
        let content_type = field.content_type().unwrap_or("application/octet-stream").to_string();

        info!(
            "收到上传请求: field={}, filename={}, content_type={}",
            field_name, file_name, content_type
        );

        match save_uploaded_file(&state.upload_config, &field_name, &file_name, &content_type, field).await {
            Ok(uploaded) => {
                info!(
                    "文件上传成功: {}, size={} bytes, url={}",
                    uploaded.original_name, uploaded.size, uploaded.access_url
                );
                uploaded_files.push(uploaded);
            }
            Err(e) => {
                error!("文件上传失败: {} - {}", file_name, e);
                errors.push(format!("{}: {}", file_name, e));
            }
        }
    }

    if uploaded_files.is_empty() {
        return (
            if errors.is_empty() { StatusCode::BAD_REQUEST } else { StatusCode::BAD_REQUEST },
            Json(ApiResponse::<Vec<UploadedFile>>::error(
                400,
                if errors.is_empty() {
                    "未找到上传的文件".to_string()
                } else {
                    errors.join("; ")
                },
            )),
        );
    }

    if !errors.is_empty() {
        (
            StatusCode::MULTI_STATUS,
            Json(ApiResponse {
                code: 207,
                message: format!("部分文件上传成功，{} 个失败: {}", errors.len(), errors.join("; ")),
                data: Some(uploaded_files),
            }),
        )
    } else {
        (
            StatusCode::OK,
            Json(ApiResponse::success(uploaded_files)),
        )
    }
}
