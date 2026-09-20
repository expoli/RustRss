//! AI 任务手动验证工具。
//!
//! 用法：
//!   ai_demo <db.sqlite> preview [summarize|translate]   # 只打印将要发送的请求（不打模型、零花费）
//!   ai_demo <db.sqlite> run     [summarize|translate]   # 真正调用模型
//!
//! 配置来自环境变量（key 永远不会被打印）：
//!   RUSTSS_AI_PROVIDER = openai | anthropic | gemini | ollama   （默认 ollama）
//!   RUSTSS_AI_MODEL    = 模型名                                  （必填）
//!   RUSTSS_AI_BASE_URL = 覆盖默认地址（可选）
//!   RUSTSS_AI_KEY      = API key；不设则按 provider 回退到
//!                        ANTHROPIC_API_KEY / OPENAI_API_KEY / GEMINI_API_KEY
//!   RUSTSS_AI_TARGET   = 翻译目标语言（默认「中文」）
//!   RUSTSS_AI_ENTRY    = 条目 id；不设则取最新一条

use rustrss_core::ai::prompt::{AiTask, ArticleText, SummaryLength};
use rustrss_core::ai::{run_task, AiClient, AiConfig, CachePolicy};
use rustrss_core::{EntryQuery, Store};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("用法: ai_demo <db.sqlite> preview|run [summarize|translate]");
        std::process::exit(2);
    }
    let (db, mode, task_kind) = (
        args[0].clone(),
        args[1].clone(),
        args.get(2).cloned().unwrap_or_else(|| "summarize".into()),
    );

    let store = Store::open(&db).expect("打开数据库失败");
    let entry = match std::env::var("RUSTSS_AI_ENTRY")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
    {
        Some(id) => store
            .get_entry(id)
            .expect("查询失败")
            .unwrap_or_else(|| panic!("未找到条目 {id}")),
        None => store
            .list_entries(&EntryQuery {
                limit: Some(1),
                ..Default::default()
            })
            .expect("查询失败")
            .into_iter()
            .next()
            .expect("库里没有条目，先跑 refresh_real 拉点数据"),
    };

    let config = config_from_env();
    println!("provider/model : {}", config.cache_tag());
    println!("base_url       : {}", config.base_url);
    println!(
        "api key        : {}",
        if config.api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false) {
            "已设置（不显示）"
        } else {
            "未设置"
        }
    );

    let task = match task_kind.as_str() {
        "translate" => AiTask::Translate {
            target: std::env::var("RUSTSS_AI_TARGET").unwrap_or_else(|_| "中文".into()),
        },
        _ => AiTask::Summarize {
            length: SummaryLength::Medium,
            language: std::env::var("RUSTSS_AI_TARGET").unwrap_or_else(|_| "中文".into()),
        },
    };

    let body = entry
        .content_text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| entry.summary.clone())
        .unwrap_or_default();
    let (prepared, truncated) = rustrss_core::ai::prompt::prepare(&body);

    println!("\n条目 #{} 《{}》", entry.id, entry.title);
    println!("正文 {} 字（截断后 {} 字，截断={}）", body.chars().count(), prepared.chars().count(), truncated);

    let client = AiClient::new(config).expect("构建 AI 客户端失败");
    let request = rustrss_core::ai::prompt::build(
        &task,
        &ArticleText {
            title: &entry.title,
            body: &prepared,
        },
    );

    match mode.as_str() {
        "preview" => {
            match client.preview(&request) {
                Ok(p) => {
                    println!("\n=== 将要发送的请求（凭据已打码；UI 上让用户确认的就是这个）===");
                    println!("POST {}", p.url);
                    for (k, v) in &p.headers {
                        println!("  {k}: {v}");
                    }
                    let body_json = serde_json::to_string_pretty(&p.body).unwrap_or_default();
                    let show = body_json.chars().take(700).collect::<String>();
                    println!("body:\n{show}\n…（总长 {} 字符）", body_json.chars().count());
                }
                Err(e) => eprintln!("生成预览失败: {e}"),
            }
        }
        "run" => {
            println!("\n=== 调用模型 ===");
            match run_task(&store, &client, entry.id, &task, CachePolicy::UseCache).await {
                Ok(outcome) => {
                    println!(
                        "来源: {}｜模型: {}｜输出 {} 字",
                        if outcome.from_cache { "缓存（未重复请求）" } else { "本次新请求" },
                        outcome.provider_model,
                        outcome.output.chars().count()
                    );
                    println!("\n{}", outcome.output);
                }
                Err(e) => eprintln!("调用失败: {e}"),
            }
        }
        other => eprintln!("未知模式 {other}；用 preview 或 run"),
    }
}

fn config_from_env() -> AiConfig {
    let provider = std::env::var("RUSTSS_AI_PROVIDER").unwrap_or_else(|_| "ollama".into());
    let model = std::env::var("RUSTSS_AI_MODEL").unwrap_or_else(|_| "llama3.2".into());
    let base_override = std::env::var("RUSTSS_AI_BASE_URL").ok();

    let pick_key = |names: &[&str]| -> Option<String> {
        if let Ok(k) = std::env::var("RUSTSS_AI_KEY") {
            if !k.trim().is_empty() {
                return Some(k);
            }
        }
        for name in names {
            if let Ok(v) = std::env::var(name) {
                if !v.trim().is_empty() {
                    return Some(v);
                }
            }
        }
        None
    };

    match provider.as_str() {
        "openai" => {
            let key = pick_key(&["OPENAI_API_KEY"]).unwrap_or_default();
            let base = base_override
                .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            AiConfig::openai_compatible(&base, &model, key)
        }
        "anthropic" => {
            let key = pick_key(&["ANTHROPIC_API_KEY"]).unwrap_or_default();
            let base = base_override
                .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            AiConfig::anthropic(&model, key).with_base_url(&base)
        }
        "gemini" => {
            let key = pick_key(&["GEMINI_API_KEY", "GOOGLE_API_KEY"]).unwrap_or_default();
            let config = AiConfig::gemini(&model, key);
            match base_override {
                Some(base) => config.with_base_url(&base),
                None => config,
            }
        }
        _ => {
            let config = AiConfig::ollama(&model);
            match base_override {
                Some(base) => config.with_base_url(&base),
                None => config,
            }
        }
    }
}
