use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::Utc;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, error, info, warn};

use crate::db::{ChatMessage, SummaryContent, SenderRole};

#[derive(Debug, Clone)]
pub struct SummaryConfig {
    pub llm_api_url: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_model: String,
    pub llm_timeout_secs: u64,
    pub use_template_fallback: bool,
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            llm_api_url: None,
            llm_api_key: None,
            llm_model: "qwen-plus".to_string(),
            llm_timeout_secs: 30,
            use_template_fallback: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SummaryService {
    pub config: SummaryConfig,
    client: Option<Client>,
}

impl SummaryService {
    pub fn new(config: SummaryConfig) -> Self {
        let client = if config.llm_api_url.is_some() && config.llm_api_key.is_some() {
            let client = Client::builder()
                .timeout(Duration::from_secs(config.llm_timeout_secs))
                .build()
                .ok();
            client
        } else {
            None
        };

        Self { config, client }
    }

    pub fn with_config(config: SummaryConfig) -> Arc<Self> {
        Arc::new(Self::new(config))
    }

    pub async fn generate_summary(
        &self,
        messages: &[ChatMessage],
    ) -> Result<(SummaryContent, String)> {
        if messages.is_empty() {
            return Err(anyhow!("对话消息为空，无法生成摘要"));
        }

        let message_count = messages.len();
        info!(
            "开始生成问诊小结，消息数量: {} 条",
            message_count
        );

        if let Some(client) = &self.client {
            match self.call_llm_api(client, messages).await {
                Ok(content) => {
                    info!("LLM API 生成摘要成功");
                    return Ok((content, "llm".to_string()));
                }
                Err(e) => {
                    error!("LLM API 调用失败: {}", e);
                    if !self.config.use_template_fallback {
                        return Err(e);
                    }
                    warn!("启用模板模式 fallback 生成摘要");
                }
            }
        } else {
            debug!("未配置 LLM API，使用模板模式生成摘要");
        }

        let content = self.generate_template_summary(messages)?;
        Ok((content, "template".to_string()))
    }

    async fn call_llm_api(
        &self,
        client: &Client,
        messages: &[ChatMessage],
    ) -> Result<SummaryContent> {
        let api_url = self
            .config
            .llm_api_url
            .as_ref()
            .ok_or_else(|| anyhow!("LLM API URL 未配置"))?;
        let api_key = self
            .config
            .llm_api_key
            .as_ref()
            .ok_or_else(|| anyhow!("LLM API Key 未配置"))?;

        let conversation_text = format_conversation_for_llm(messages);

        let system_prompt = r#"你是一名专业的医疗助手，请根据以下医患对话内容，生成结构化的问诊小结。
要求：
1. 严格基于对话内容，不要编造信息
2. 语言简洁专业，符合医疗文书规范
3. 按以下字段输出 JSON 格式：
   - chief_complaint: 患者主诉（主要症状和就诊原因）
   - present_illness: 现病史（病情描述、持续时间、加重/缓解因素等）
   - diagnosis: 医生诊断意见
   - treatment_plan: 治疗方案（用药、检查、治疗建议）
   - doctor_advice: 医生嘱咐（注意事项、复查建议等）
   - key_points: 本次问诊的关键点摘要（3-5条，用分号分隔）

注意：仅输出 JSON，不要有额外的解释文字或 markdown 标记。"#;

        let user_prompt = format!(
            "以下是医患对话内容，请生成问诊小结：\n\n{}",
            conversation_text
        );

        let request_body = json!({
            "model": self.config.llm_model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "temperature": 0.3,
            "response_format": {"type": "json_object"}
        });

        debug!("发送 LLM 请求: model={}", self.config.llm_model);

        let resp = client
            .post(api_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| anyhow!("LLM API 请求失败: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("LLM API 返回错误: {} - {}", status, text));
        }

        let resp_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| anyhow!("解析 LLM 响应失败: {}", e))?;

        debug!("LLM 响应: {}", resp_json);

        let content_text = resp_json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| anyhow!("LLM 响应格式异常，缺少 choices[0].message.content"))?;

        let summary: SummaryContent = serde_json::from_str(content_text)
            .map_err(|e| anyhow!("解析 LLM 返回的 JSON 失败: {}, 原始内容: {}", e, content_text))?;

        Ok(summary)
    }

    fn generate_template_summary(&self, messages: &[ChatMessage]) -> Result<SummaryContent> {
        let patient_messages: Vec<&ChatMessage> = messages
            .iter()
            .filter(|m| m.sender_role == SenderRole::Patient)
            .collect();

        let doctor_messages: Vec<&ChatMessage> = messages
            .iter()
            .filter(|m| m.sender_role == SenderRole::Doctor)
            .collect();

        let patient_texts: Vec<&str> = patient_messages
            .iter()
            .map(|m| m.content.as_str())
            .filter(|s| !s.is_empty())
            .collect();

        let doctor_texts: Vec<&str> = doctor_messages
            .iter()
            .map(|m| m.content.as_str())
            .filter(|s| !s.is_empty())
            .collect();

        let image_count = messages
            .iter()
            .filter(|m| m.message_type == crate::db::MessageType::Image)
            .count();

        let chief_complaint = extract_chief_complaint(&patient_texts);
        let present_illness = extract_present_illness(&patient_texts);
        let diagnosis = extract_diagnosis(&doctor_texts);
        let treatment_plan = extract_treatment_plan(&doctor_texts);
        let doctor_advice = extract_doctor_advice(&doctor_texts);

        let mut key_points = Vec::new();
        key_points.push(format!("对话共 {} 轮，患者 {} 条，医生 {} 条",
            messages.len(), patient_messages.len(), doctor_messages.len()));
        if image_count > 0 {
            key_points.push(format!("含 {} 张图片资料", image_count));
        }
        if !chief_complaint.is_empty() {
            key_points.push(format!("主诉: {}", truncate_text(&chief_complaint, 30)));
        }
        if !diagnosis.is_empty() {
            key_points.push(format!("诊断: {}", truncate_text(&diagnosis, 30)));
        }

        Ok(SummaryContent {
            chief_complaint,
            present_illness,
            diagnosis,
            treatment_plan,
            doctor_advice,
            key_points: key_points.join("；"),
        })
    }
}

fn format_conversation_for_llm(messages: &[ChatMessage]) -> String {
    let mut result = String::new();
    let time_format = "%Y-%m-%d %H:%M:%S";

    for msg in messages {
        let role = match msg.sender_role {
            SenderRole::Doctor => "医生",
            SenderRole::Patient => "患者",
        };

        let time_str = msg.created_at.format(time_format).to_string();

        result.push_str(&format!("[{}] {}:\n", time_str, role));

        if msg.message_type == crate::db::MessageType::Image {
            result.push_str(&format!("[图片] {}\n", msg.content));
            if let Some(url) = &msg.image_url {
                result.push_str(&format!("图片链接: {}\n", url));
            }
        } else {
            result.push_str(&format!("{}\n", msg.content));
        }

        result.push('\n');
    }

    result
}

fn truncate_text(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_len).collect();
        format!("{}...", truncated)
    }
}

fn extract_chief_complaint(texts: &[&str]) -> String {
    let complaint_keywords = &["咳嗽", "发热", "头痛", "腹痛", "胸闷", "胸痛", "腹泻", "呕吐",
        "头晕", "乏力", "皮疹", "瘙痒", "关节痛", "腰痛", "尿频", "尿急", "尿痛",
        "咽痛", "鼻塞", "流涕", "喷嚏", "耳鸣", "耳痛", "眼痛", "视物模糊",
        "受伤", "撞伤", "摔伤", "扭伤", "烫伤", "过敏", "失眠", "焦虑"];

    for &text in texts {
        for kw in complaint_keywords {
            if text.contains(kw) {
                let re = Regex::new(&format!(r"[^\n。.!！?？]*{}[^\n。.!！?？]*", regex::escape(kw))).unwrap();
                if let Some(cap) = re.find(text) {
                    let extracted = cap.as_str().trim();
                    if extracted.len() >= 2 && extracted.len() <= 100 {
                        return extracted.to_string();
                    }
                }
            }
        }
    }

    texts.get(0).map(|&s| {
        let trimmed = s.trim();
        if trimmed.len() > 100 {
            truncate_text(trimmed, 100)
        } else {
            trimmed.to_string()
        }
    }).unwrap_or_default()
}

fn extract_present_illness(texts: &[&str]) -> String {
    let duration_keywords = &["天", "周", "星期", "月", "年", "小时", "分钟", "日"];
    let severity_keywords = &["严重", "轻微", "剧烈", "持续", "间断", "阵发性", "偶尔", "经常"];
    let symptom_keywords = &["加重", "缓解", "伴", "伴随", "无", "不", "未"];

    let mut relevant_parts = Vec::new();

    for &text in texts {
        let sentences: Vec<&str> = text.split(|c: char| c == '。' || c == '！' || c == '!' || c == '?' || c == '\n').collect();
        for sent in sentences {
            let has_relevance = duration_keywords.iter().any(|k| sent.contains(k))
                || severity_keywords.iter().any(|k| sent.contains(k))
                || symptom_keywords.iter().any(|k| sent.contains(k));
            if has_relevance && !sent.trim().is_empty() {
                relevant_parts.push(sent.trim().to_string());
            }
        }
    }

    if !relevant_parts.is_empty() {
        return relevant_parts.join("。") + "。";
    }

    if texts.len() >= 2 {
        return texts.iter().take(3)
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>()
            .join("。");
    }

    String::new()
}

fn extract_diagnosis(texts: &[&str]) -> String {
    let diagnosis_keywords = &["诊断", "考虑", "可能", "怀疑", "应该是", "是", "属于", "符合"];
    let disease_keywords = &["感冒", "流感", "肺炎", "支气管炎", "胃炎", "肠炎",
        "高血压", "糖尿病", "冠心病", "脑梗", "脑出血", "关节炎",
        "过敏", "湿疹", "荨麻疹", "颈椎病", "腰椎病", "肾结石",
        "胆囊炎", "阑尾炎", "痔疮", "便秘", "偏头痛", "神经衰弱",
        "抑郁", "焦虑", "甲亢", "甲减", "贫血"];

    for &text in texts {
        let sentences: Vec<&str> = text.split(|c: char| c == '。' || c == '！' || c == '!' || c == '?' || c == '\n').collect();
        for sent in sentences {
            let has_diagnosis_hint = diagnosis_keywords.iter().any(|k| sent.contains(k));
            let has_disease_hint = disease_keywords.iter().any(|k| sent.contains(k));
            if (has_diagnosis_hint || has_disease_hint) && !sent.trim().is_empty() {
                return sent.trim().to_string();
            }
        }
    }

    String::new()
}

fn extract_treatment_plan(texts: &[&str]) -> String {
    let treatment_keywords = &["吃", "服用", "口服", "外用", "涂抹", "输液", "打针",
        "检查", "化验", "拍片", "CT", "B超", "彩超", "心电图", "抽血",
        "复查", "复诊", "住院", "手术", "治疗", "理疗", "按摩", "针灸"];
    let drug_keywords = &["阿莫西林", "头孢", "布洛芬", "对乙酰氨基酚", "阿司匹林",
        "奥美拉唑", "氯雷他定", "西替利嗪", "蒙脱石散", "益生菌",
        "维生素", "钙片", "鱼油", "降压药", "降糖药"];

    let mut treatments = Vec::new();

    for &text in texts {
        let sentences: Vec<&str> = text.split(|c: char| c == '。' || c == '！' || c == '!' || c == '?' || c == '\n').collect();
        for sent in sentences {
            let has_treatment = treatment_keywords.iter().any(|k| sent.contains(k))
                || drug_keywords.iter().any(|k| sent.contains(k));
            if has_treatment && !sent.trim().is_empty() {
                treatments.push(sent.trim().to_string());
            }
        }
    }

    if !treatments.is_empty() {
        return treatments.join("。") + "。";
    }

    String::new()
}

fn extract_doctor_advice(texts: &[&str]) -> String {
    let advice_keywords = &["注意", "避免", "不要", "忌", "应该", "建议", "需要",
        "多喝", "休息", "清淡", "戒烟", "戒酒", "规律", "避免劳累",
        "注意保暖", "饮食", "作息", "运动", "锻炼", "情绪", "保持"];

    let mut advices = Vec::new();

    for &text in texts {
        let sentences: Vec<&str> = text.split(|c: char| c == '。' || c == '！' || c == '!' || c == '?' || c == '\n').collect();
        for sent in sentences {
            let has_advice = advice_keywords.iter().any(|k| sent.contains(k));
            if has_advice && !sent.trim().is_empty() {
                advices.push(sent.trim().to_string());
            }
        }
    }

    if !advices.is_empty() {
        return advices.join("。") + "。";
    }

    String::new()
}
