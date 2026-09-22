//! 内置 AI 适配层（自带 API key）。
//!
//! 支持四类端点：OpenAI 兼容、Anthropic Messages、Gemini 原生、Ollama。
//! 差异（路径、鉴权头、请求/响应结构）全部收敛在本文件里，上层只面对
//! `AiClient::complete(AiRequest)`。
//!
//! 安全约定：
//! - `api_key` 只进请求头，**不出现在错误信息、日志、预览里**；
//! - provider 返回的错误体会被原样保留（用于定位问题），但会先对 key 做打码；
//! - `preview()` 给出的请求预览里 key 已被替换，供「发送前让用户确认要发什么」使用。

pub mod prompt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// 把 prompt 层的类型重导出，让调用方只面对一个模块路径（AiRequest 也在内）
pub use prompt::{AiRequest, AiTask, ArticleText, SummaryLength, PROMPT_VERSION};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// OpenAI 兼容（含 DeepSeek / Kimi / 自建网关 / vLLM 等）
    OpenAiCompatible,
    /// Anthropic Messages（原生协议）
    Anthropic,
    /// Gemini 原生 generateContent
    Gemini,
    /// 本地 Ollama
    Ollama,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: Provider,
    pub model: String,
    /// 服务根地址（含协议与主机，可带自定义前缀）；具体路径由各 provider 拼接
    pub base_url: String,
    pub api_key: Option<String>,
    pub max_output_tokens: u32,
    /// 思考强度（仅 OpenAI 兼容接口生效：reasoning_effort minimal/low/medium/high；
    /// None = 不发这个参数，跟随模型默认）。摘要/翻译这类任务调低可显著提速省 token。
    pub reasoning_effort: Option<String>,
}

impl AiConfig {
    pub fn openai_compatible(base_url: &str, model: &str, api_key: impl Into<String>) -> Self {
        Self {
            provider: Provider::OpenAiCompatible,
            model: model.to_string(),
            base_url: base_url.to_string(),
            api_key: Some(api_key.into()),
            max_output_tokens: 4096,
            reasoning_effort: None,
        }
    }

    pub fn anthropic(model: &str, api_key: impl Into<String>) -> Self {
        Self {
            provider: Provider::Anthropic,
            model: model.to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: Some(api_key.into()),
            max_output_tokens: 4096,
            reasoning_effort: None,
        }
    }

    pub fn gemini(model: &str, api_key: impl Into<String>) -> Self {
        Self {
            provider: Provider::Gemini,
            model: model.to_string(),
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            api_key: Some(api_key.into()),
            max_output_tokens: 4096,
            reasoning_effort: None,
        }
    }

    /// 本地 Ollama：不需要 key
    pub fn ollama(model: &str) -> Self {
        Self {
            provider: Provider::Ollama,
            model: model.to_string(),
            base_url: "http://127.0.0.1:11434".to_string(),
            api_key: None,
            max_output_tokens: 4096,
            reasoning_effort: None,
        }
    }

    pub fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.to_string();
        self
    }

    pub fn with_max_output_tokens(mut self, n: u32) -> Self {
        self.max_output_tokens = n;
        self
    }

    /// 设置思考强度（Some("")/None = 不发参数）。值由 src-tauri 白名单归一化。
    pub fn with_reasoning_effort(mut self, effort: Option<String>) -> Self {
        self.reasoning_effort = effort.filter(|e| !e.trim().is_empty());
        self
    }

    /// 供缓存键使用（不含 key）
    pub fn cache_tag(&self) -> String {
        format!("{:?}/{}", self.provider, self.model)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("未配置 API key")]
    MissingKey,
    #[error("构建请求失败: {0}")]
    Request(String),
    #[error("网络请求失败: {0}")]
    Transport(String),
    #[error("服务端返回 {status}: {message}")]
    Provider { status: u16, message: String },
    #[error("响应结构不符合预期: {0}")]
    BadResponse(String),
    #[error("数据库错误: {0}")]
    Store(String),
    #[error("未找到条目 {0}")]
    EntryNotFound(i64),
}

/// 请求预览：给「发送前确认」用。headers 里的凭据已打码。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequestPreview {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

#[derive(Clone)]
pub struct AiClient {
    http: reqwest::Client,
    config: AiConfig,
}

impl AiClient {
    pub fn new(config: AiConfig) -> Result<Self, AiError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| AiError::Request(e.to_string()))?;
        Ok(Self { http, config })
    }

    pub fn config(&self) -> &AiConfig {
        &self.config
    }

    /// 将要发送的请求长什么样（key 已打码）——UI 上让用户确认的就是这个
    ///
    /// 打码覆盖 url / headers / body 三处：Gemini 这类原生协议把 key 放在
    /// 查询串里，只处理 header 会泄露（这个 bug 就是被测试抓出来的）。
    pub fn preview(&self, req: &AiRequest) -> Result<RequestPreview, AiError> {
        let planned = self.plan(req)?;
        let mut headers: Vec<(String, String)> = planned
            .headers
            .iter()
            .map(|(k, v)| {
                let sensitive = k.eq_ignore_ascii_case("authorization")
                    || k.eq_ignore_ascii_case("x-api-key");
                let value = if sensitive {
                    "***已隐藏***".to_string()
                } else {
                    self.scrub(v)
                };
                (k.clone(), value)
            })
            .collect();
        // `complete()` 用 `.json()` 发请求，reqwest 会补上 Content-Type。
        // 预览里如实补一行，否则「看到的请求头」比实际发出的少一条。
        if !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        {
            headers.push(("Content-Type".to_string(), "application/json".to_string()));
        }
        Ok(RequestPreview {
            url: self.scrub(&planned.url),
            headers,
            body: self.scrub_value(&planned.body),
        })
    }

    /// 发一次请求，取回文本结果
    pub async fn complete(&self, req: AiRequest) -> Result<String, AiError> {
        let planned = self.plan(&req)?;
        let mut builder = self.http.post(&planned.url).json(&planned.body);
        for (name, value) in &planned.headers {
            builder = builder.header(name, value);
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| AiError::Transport(e.to_string()))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            // 保留 provider 的原始错误信息（定位问题必需），但先对 key 打码
            return Err(AiError::Provider {
                status: status.as_u16(),
                message: self.scrub(&shorten(&text, 600)),
            });
        }

        let value: Value = serde_json::from_str(&text).map_err(|e| {
            AiError::BadResponse(format!("{}（原始响应: {}）", e, self.scrub(&shorten(&text, 300))))
        })?;
        self.extract_text(&value)
    }

    /// 把凭据从任意文本里抹掉（错误信息、日志、预览共用同一套规则）
    fn scrub(&self, text: &str) -> String {
        match self.config.api_key.as_deref() {
            Some(key) if !key.is_empty() => text.replace(key, "***"),
            _ => text.to_string(),
        }
    }

    /// 递归打码 JSON 里的字符串（正文内容不会被误伤，因为只替换 key 本身）
    fn scrub_value(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.scrub(s)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|v| self.scrub_value(v)).collect())
            }
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.scrub_value(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn require_key(&self) -> Result<&str, AiError> {
        match self.config.api_key.as_deref() {
            Some(k) if !k.is_empty() => Ok(k),
            _ => Err(AiError::MissingKey),
        }
    }

    fn plan(&self, req: &AiRequest) -> Result<Planned, AiError> {
        let base = self.config.base_url.trim_end_matches('/');
        let model = self.config.model.as_str();
        let max = self.config.max_output_tokens;

        match self.config.provider {
            Provider::OpenAiCompatible => {
                let key = self.require_key()?;
                let mut messages = Vec::new();
                if let Some(system) = req.system.as_deref() {
                    messages.push(json!({ "role": "system", "content": system }));
                }
                messages.push(json!({ "role": "user", "content": req.user }));
                Ok(Planned {
                    url: format!("{base}/chat/completions"),
                    // content-type 由 .json() 负责，不在这里重复设置（避免发出两个同名头）
                    headers: vec![("authorization".into(), format!("Bearer {key}"))],
                    body: {
                        let mut b = json!({
                            "model": model,
                            "messages": messages,
                            // 推理模型（deepseek-r1 等）会先输出 reasoning_content 再输出正文：
                            // 上限太小会把预算全烧在思考链上（实测 1024 时 finish_reason=length
                            // 且 content 为空），默认给到 4096，可在设置里调
                            "max_tokens": max,
                            "stream": false,
                        });
                        // 思考强度：仅推理模型有意义；deepseek-r1 固定思考不受控，
                        // 非推理模型多数端点会忽略，严格端点可能 4xx（用户自己选的档位，默认不发）
                        if let Some(effort) = &self.config.reasoning_effort {
                            b["reasoning_effort"] = json!(effort);
                        }
                        b
                    },
                })
            }
            Provider::Anthropic => {
                let key = self.require_key()?;
                let mut body = json!({
                    "model": model,
                    "max_tokens": max,
                    "messages": [{ "role": "user", "content": req.user }],
                });
                if let Some(system) = req.system.as_deref() {
                    body["system"] = json!(system);
                }
                Ok(Planned {
                    url: format!("{base}/v1/messages"),
                    headers: vec![
                        ("x-api-key".into(), key.to_string()),
                        // Anthropic 要求显式版本头
                        ("anthropic-version".into(), "2023-06-01".into()),
                    ],
                    body,
                })
            }
            Provider::Gemini => {
                let key = self.require_key()?;
                let mut body = json!({
                    "contents": [{ "parts": [{ "text": req.user }] }],
                    "generationConfig": { "maxOutputTokens": max },
                });
                if let Some(system) = req.system.as_deref() {
                    body["systemInstruction"] = json!({ "parts": [{ "text": system }] });
                }
                Ok(Planned {
                    // 模型名进路径；key 走查询串（Gemini 原生协议如此）
                    url: format!("{base}/v1beta/models/{model}:generateContent?key={key}"),
                    headers: vec![],
                    body,
                })
            }
            Provider::Ollama => {
                // Ollama 无系统角色，系统提示拼进 prompt
                let prompt = match req.system.as_deref() {
                    Some(system) => format!("{system}\n\n{}", req.user),
                    None => req.user.clone(),
                };
                Ok(Planned {
                    url: format!("{base}/api/generate"),
                    headers: vec![],
                    body: json!({
                        "model": model,
                        "prompt": prompt,
                        "stream": false,
                        "options": { "num_predict": max },
                    }),
                })
            }
        }
    }

    fn extract_text(&self, value: &Value) -> Result<String, AiError> {
        let text = match self.config.provider {
            Provider::OpenAiCompatible => value["choices"][0]["message"]["content"].as_str(),
            Provider::Anthropic => value["content"][0]["text"].as_str(),
            Provider::Gemini => value["candidates"][0]["content"]["parts"][0]["text"].as_str(),
            Provider::Ollama => value["response"].as_str(),
        };
        match text {
            Some(t) if !t.trim().is_empty() => Ok(t.trim().to_string()),
            _ => Err(AiError::BadResponse(format!(
                "{}{}",
                self.empty_content_diagnosis(value),
                shorten(&value.to_string(), 300)
            ))),
        }
    }

    /// 正文为空时的补充诊断：区分「截断」（可自救：调大上限/换模型）与真异常，
    /// 拼在原始响应前面让用户一眼看懂该怎么处理。
    fn empty_content_diagnosis(&self, value: &Value) -> &'static str {
        if self.config.provider != Provider::OpenAiCompatible {
            return "响应里没有文本内容: ";
        }
        let finish = value["choices"][0]["finish_reason"].as_str().unwrap_or("");
        let reasoning = value["choices"][0]["message"]["reasoning_content"]
            .as_str()
            .map(str::is_empty)
            .unwrap_or(true);
        match (finish, reasoning) {
            // 推理模型把 token 预算全烧在思考链上，正文一个字没产出
            ("length", false) => "输出被 token 上限截断：模型思考链占满了预算，正文未产出。请在设置里调大「输出上限」或换非推理模型。原始响应: ",
            ("length", true) => "输出被 token 上限截断（finish_reason=length）。请在设置里调大「输出上限」或缩短正文。原始响应: ",
            (_, false) => "模型只返回了思考链没有正文（推理模型异常输出）。原始响应: ",
            _ => "响应里没有文本内容: ",
        }
    }
}

struct Planned {
    url: String,
    headers: Vec<(String, String)>,
    body: Value,
}

/// 缓存策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    /// 命中缓存就不请求（默认）
    UseCache,
    /// 忽略缓存重新生成（结果仍会写入缓存）
    Refresh,
}

/// 一次 AI 任务的结果
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskOutcome {
    pub output: String,
    /// 是否来自缓存（用于向用户交代「没有重复花钱」）
    pub from_cache: bool,
    /// 送入模型的正文是否因超长被截断
    pub truncated: bool,
    /// 实际使用的 provider/model（缓存键的一部分）
    pub provider_model: String,
}

/// 一次 AI 任务的「计划」：请求已构造、缓存键已算出，只等发请求。
///
/// 拆出这一步是为了在 Tauri 的 async 命令里能「取任务（持锁）→ 发请求（不持锁）→ 存结果（持锁）」，
/// 与抓取层的三阶段同构：`!Sync` 的连接不跨越 await。
pub struct AiTaskPlan {
    pub request: prompt::AiRequest,
    pub truncated: bool,
    pub provider_model: String,
    cache: OwnedCacheKey,
    /// 缓存命中时的结果（此时不该再发请求）
    pub cached: Option<String>,
}

impl AiTaskPlan {
    pub fn cache_hit(&self) -> bool {
        self.cached.is_some()
    }
}

struct OwnedCacheKey {
    entry_id: i64,
    task: String,
    params: String,
    provider_model: String,
}

impl OwnedCacheKey {
    fn borrowed(&self) -> crate::store::AiCacheKey<'_> {
        crate::store::AiCacheKey {
            entry_id: self.entry_id,
            task: &self.task,
            params: &self.params,
            provider_model: &self.provider_model,
            prompt_version: prompt::PROMPT_VERSION,
        }
    }
}

/// 读文章正文、构造 prompt、算缓存键（同步，可持锁调用）
pub fn plan_task(
    store: &crate::store::Store,
    client: &AiClient,
    entry_id: i64,
    task: &prompt::AiTask,
    policy: CachePolicy,
) -> Result<AiTaskPlan, AiError> {
    let entry = store
        .get_entry(entry_id)
        .map_err(|e| AiError::Store(e.to_string()))?
        .ok_or(AiError::EntryNotFound(entry_id))?;

    let body = entry
        .content_text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| entry.summary.clone())
        .unwrap_or_default();
    let truncated = prompt::was_truncated(&body);

    let provider_model = client.config().cache_tag();
    let cache = OwnedCacheKey {
        entry_id,
        task: task.name().to_string(),
        params: task.cache_params(),
        provider_model: provider_model.clone(),
    };

    let cached = match policy {
        CachePolicy::UseCache => store
            .ai_cached(&cache.borrowed())
            .map_err(|e| AiError::Store(e.to_string()))?,
        CachePolicy::Refresh => None,
    };

    Ok(AiTaskPlan {
        request: prompt::build(
            task,
            &prompt::ArticleText {
                title: &entry.title,
                body: &body,
            },
        ),
        truncated,
        provider_model,
        cache,
        cached,
    })
}

/// 把模型输出写进缓存（同步，可持锁调用）
pub fn save_task_output(
    store: &crate::store::Store,
    plan: &AiTaskPlan,
    output: &str,
) -> Result<(), AiError> {
    store
        .ai_store(&plan.cache.borrowed(), output)
        .map_err(|e| AiError::Store(e.to_string()))
}

/// 跑一个 AI 任务（三阶段串起来；CLI 示例与测试用它）
pub async fn run_task(
    store: &crate::store::Store,
    client: &AiClient,
    entry_id: i64,
    task: &prompt::AiTask,
    policy: CachePolicy,
) -> Result<TaskOutcome, AiError> {
    let plan = plan_task(store, client, entry_id, task, policy)?;
    if let Some(hit) = plan.cached.clone() {
        return Ok(TaskOutcome {
            output: hit,
            from_cache: true,
            truncated: plan.truncated,
            provider_model: plan.provider_model,
        });
    }
    let output = client.complete(plan.request.clone()).await?;
    save_task_output(store, &plan, &output)?;
    Ok(TaskOutcome {
        output,
        from_cache: false,
        truncated: plan.truncated,
        provider_model: plan.provider_model,
    })
}

fn shorten(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(user: &str) -> AiRequest {
        AiRequest {
            system: Some("系统提示".into()),
            user: user.into(),
        }
    }

    #[test]
    fn preview_hides_credentials_for_every_provider() {
        let key = "sk-super-secret-123";
        let cases = [
            AiConfig::openai_compatible("https://api.example.com/v1", "gpt-x", key),
            AiConfig::anthropic("claude-x", key),
            AiConfig::gemini("gemini-x", key),
            // ollama 无 key，但也必须走同一条 JSON 发送路径
            AiConfig::ollama("llama-x"),
        ];
        for config in cases {
            let client = AiClient::new(config).unwrap();
            let preview = client.preview(&req("正文")).unwrap();
            let dumped = format!(
                "{} {:?} {}",
                preview.url, preview.headers, preview.body
            );
            assert!(!dumped.contains(key), "预览里泄露了 key: {dumped}");
            // 实际发送时 reqwest 会补 Content-Type，预览必须说全
            assert!(
                preview
                    .headers
                    .iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case("content-type")
                        && v == "application/json"),
                "预览缺少 Content-Type: {:?}",
                preview.headers
            );
        }
    }

    #[test]
    fn provider_errors_do_not_leak_the_key() {
        // provider 把 key 回显在错误里（真实网关偶见）
        let key = "sk-leak-me";
        let client = AiClient::new(AiConfig::openai_compatible(
            "https://api.example.com/v1",
            "m",
            key,
        ))
        .unwrap();
        let scrubbed = client.scrub(&format!("invalid api key: {key}"));
        assert!(!scrubbed.contains(key));
        assert!(scrubbed.contains("***"));
    }

    #[test]
    fn missing_key_is_reported_before_any_request() {
        let mut config = AiConfig::ollama("llama");
        config.provider = Provider::OpenAiCompatible;
        let client = AiClient::new(config).unwrap();
        assert!(matches!(
            client.preview(&req("x")),
            Err(AiError::MissingKey)
        ));
    }
}
