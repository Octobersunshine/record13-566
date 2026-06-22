use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::{fs, io::AsyncWriteExt};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedFile {
    pub file_name: String,
    pub original_name: String,
    pub stored_path: String,
    pub access_url: String,
    pub size: u64,
    pub content_type: String,
}

#[derive(Debug, Clone)]
pub struct UploadConfig {
    pub upload_dir: PathBuf,
    pub max_file_size: u64,
    pub allowed_types: Vec<String>,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            upload_dir: PathBuf::from("./uploads"),
            max_file_size: 10 * 1024 * 1024,
            allowed_types: vec![
                "image/jpeg".to_string(),
                "image/png".to_string(),
                "image/gif".to_string(),
                "image/webp".to_string(),
                "image/bmp".to_string(),
            ],
        }
    }
}

impl UploadConfig {
    pub fn new(upload_dir: impl Into<PathBuf>, max_file_size_mb: u64) -> Self {
        let mut config = Self::default();
        config.upload_dir = upload_dir.into();
        config.max_file_size = max_file_size_mb * 1024 * 1024;
        config
    }

    pub async fn ensure_dirs(&self) -> Result<()> {
        if !self.upload_dir.exists() {
            fs::create_dir_all(&self.upload_dir).await?;
        }
        let today_dir = self.get_today_dir();
        if !today_dir.exists() {
            fs::create_dir_all(&today_dir).await?;
        }
        Ok(())
    }

    fn get_today_dir(&self) -> PathBuf {
        let now = Utc::now();
        let date_str = now.format("%Y/%m/%d").to_string();
        self.upload_dir.join(date_str)
    }

    fn is_allowed_type(&self, content_type: &str) -> bool {
        self.allowed_types.iter().any(|t| t == content_type)
            || self
                .allowed_types
                .iter()
                .any(|t| content_type.starts_with(t.split('/').next().unwrap_or("")))
    }
}

fn get_extension_from_filename(filename: &str) -> String {
    Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_else(|| "bin".to_string())
}

fn get_extension_from_mime(content_type: &str, original_name: &str) -> String {
    let ext_from_name = get_extension_from_filename(original_name);
    if !ext_from_name.is_empty() && ext_from_name != "bin" {
        return ext_from_name;
    }
    match content_type {
        "image/jpeg" => "jpg".to_string(),
        "image/png" => "png".to_string(),
        "image/gif" => "gif".to_string(),
        "image/webp" => "webp".to_string(),
        "image/bmp" => "bmp".to_string(),
        _ => ext_from_name,
    }
}

pub async fn save_uploaded_file(
    config: &UploadConfig,
    field_name: &str,
    file_name: &str,
    content_type: &str,
    mut data_stream: axum::extract::multipart::Field<'_>,
) -> Result<UploadedFile> {
    if !config.is_allowed_type(content_type) {
        return Err(anyhow!(
            "不支持的文件类型: {}，仅支持 JPG/PNG/GIF/WebP/BMP 图片",
            content_type
        ));
    }

    config.ensure_dirs().await?;

    let today_dir = config.get_today_dir();
    let ext = get_extension_from_mime(content_type, file_name);
    let stored_filename = format!("{}.{}", Uuid::new_v4(), ext);
    let file_path = today_dir.join(&stored_filename);

    let mut file = fs::File::create(&file_path).await?;
    let mut total_size: u64 = 0;

    while let Some(chunk) = data_stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("读取上传数据失败: {}", e))?;
        total_size += chunk.len() as u64;
        if total_size > config.max_file_size {
            drop(file);
            let _ = fs::remove_file(&file_path).await;
            return Err(anyhow!(
                "文件大小超过限制，最大允许 {} MB",
                config.max_file_size / (1024 * 1024)
            ));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| anyhow!("写入文件失败: {}", e))?;
    }

    file.flush().await?;
    drop(file);

    if total_size == 0 {
        let _ = fs::remove_file(&file_path).await;
        return Err(anyhow!("上传的文件为空"));
    }

    let relative_path = file_path
        .strip_prefix(&config.upload_dir)
        .unwrap_or(file_path.as_path())
        .to_string_lossy()
        .replace('\\', "/");

    let access_url = format!("/uploads/{}", relative_path);
    let stored_path = file_path.to_string_lossy().replace('\\', "/");

    Ok(UploadedFile {
        file_name: stored_filename,
        original_name: file_name.to_string(),
        stored_path,
        access_url,
        size: total_size,
        content_type: content_type.to_string(),
    })
}

pub async fn cleanup_empty_file(file_path: &Path) {
    if file_path.exists() {
        let _ = fs::remove_file(file_path).await;
    }
}
