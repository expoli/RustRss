//! Byte-framed SSE/NDJSON chat streams. Only complete frames are decoded as UTF-8.
use super::chat::{decode_chat_response, ChatResponse};
use super::{AiError, Provider};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

pub struct ChatStreamParser {
    provider: Provider,
    pending: Vec<u8>,
    data: Vec<u8>,
    event_id: Option<String>,
    seen_ids: HashSet<String>,
    text: String,
    calls: BTreeMap<u64, Value>,
    parts: Vec<Value>,
    usage: Value,
    reason: Value,
    terminated: bool,
}
impl ChatStreamParser {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            pending: vec![],
            data: vec![],
            event_id: None,
            seen_ids: HashSet::new(),
            text: String::new(),
            calls: BTreeMap::new(),
            parts: vec![],
            usage: json!({}),
            reason: Value::Null,
            terminated: false,
        }
    }
    pub fn received_chars(&self) -> usize {
        self.text.chars().count()
    }
    pub fn interrupted(&self, cause: &str) -> AiError {
        AiError::Transport(format!(
            "{cause}（已收到 {} 字符，流未完整结束）",
            self.received_chars()
        ))
    }
    /// An empty input chunk is not EOF. Explicit `finish` checks termination.
    pub fn push(&mut self, bytes: &[u8], mut delta: impl FnMut(&str)) -> Result<(), AiError> {
        self.pending.extend_from_slice(bytes);
        let mut consumed = 0;
        while let Some(end) = self.pending[consumed..].iter().position(|b| *b == b'\n') {
            let end = consumed + end;
            let mut line = self.pending[consumed..end].to_vec();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.line(&line, &mut delta)?;
            consumed = end + 1;
        }
        self.pending.drain(..consumed);
        Ok(())
    }
    fn line(&mut self, line: &[u8], delta: &mut impl FnMut(&str)) -> Result<(), AiError> {
        if self.provider == Provider::Ollama {
            if !line.is_empty() {
                self.frame(line, delta)?;
            }
        } else if line.is_empty() {
            let data = std::mem::take(&mut self.data);
            let duplicate = self
                .event_id
                .take()
                .is_some_and(|id| !self.seen_ids.insert(id));
            if !data.is_empty() && !duplicate {
                self.frame(&data, delta)?;
            }
        } else if let Some(data) = line.strip_prefix(b"data:") {
            if !self.data.is_empty() {
                self.data.push(b'\n');
            }
            self.data
                .extend_from_slice(data.strip_prefix(b" ").unwrap_or(data));
        } else if let Some(id) = line.strip_prefix(b"id:") {
            self.event_id = Some(
                String::from_utf8(id.strip_prefix(b" ").unwrap_or(id).to_vec())
                    .map_err(|_| AiError::BadResponse("SSE id 不是 UTF-8".into()))?,
            );
        }
        Ok(())
    }
    fn append(&mut self, text: Option<&str>, delta: &mut impl FnMut(&str)) {
        if let Some(text) = text.filter(|s| !s.is_empty()) {
            self.text.push_str(text);
            delta(text);
        }
    }
    fn frame(&mut self, bytes: &[u8], delta: &mut impl FnMut(&str)) -> Result<(), AiError> {
        if bytes == b"[DONE]" {
            if self.provider != Provider::OpenAiCompatible {
                return Err(AiError::BadResponse(
                    "当前 provider 不支持 [DONE] 终止符".into(),
                ));
            }
            self.terminated = true;
            return Ok(());
        }
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| AiError::BadResponse(format!("流式 JSON 无效: {e}")))?;
        if !value["error"].is_null() || value["type"] == "error" {
            return Err(self.interrupted("服务商流式错误"));
        }
        // Terminal frames may be followed by usage-only frames; never append late text.
        match self.provider {
            Provider::OpenAiCompatible => {
                if value["usage"].is_object() {
                    self.usage = value["usage"].clone();
                }
                if self.terminated {
                    return Ok(());
                }
                let choice = &value["choices"][0];
                self.append(choice["delta"]["content"].as_str(), delta);
                if let Some(calls) = choice["delta"]["tool_calls"].as_array() {
                    for call in calls {
                        let index = call["index"]
                            .as_u64()
                            .ok_or_else(|| AiError::BadResponse("tool delta 缺少 index".into()))?;
                        let entry = self.calls.entry(index).or_insert_with(
                            || json!({"id":"", "function":{"name":"", "arguments":""}}),
                        );
                        if let Some(id) = call["id"].as_str() {
                            entry["id"] = json!(id);
                        }
                        for field in ["name", "arguments"] {
                            if let Some(fragment) = call["function"][field].as_str() {
                                let mut text =
                                    entry["function"][field].as_str().unwrap_or("").to_string();
                                text.push_str(fragment);
                                entry["function"][field] = json!(text);
                            }
                        }
                    }
                }
                if !choice["finish_reason"].is_null() {
                    self.reason = choice["finish_reason"].clone();
                }
            }
            Provider::Anthropic => {
                match value["type"].as_str() {
                    Some("message_start") => {
                        if value["message"]["usage"].is_object() {
                            self.usage = value["message"]["usage"].clone();
                        }
                    }
                    Some("message_delta") => {
                        if !value["delta"]["stop_reason"].is_null() {
                            self.reason = value["delta"]["stop_reason"].clone();
                        }
                        if let Some(usage) = value["usage"].as_object() {
                            for (key, v) in usage {
                                self.usage[key] = v.clone();
                            }
                        }
                    }
                    Some("message_stop") => self.terminated = true,
                    Some("content_block_start") if !self.terminated => {
                        let index = value["index"].as_u64().ok_or_else(|| {
                            AiError::BadResponse("content block 缺少 index".into())
                        })?;
                        let block = &value["content_block"];
                        if block["type"] == "tool_use" {
                            // Preserve earlier partial_json if start arrives after a delta.
                            let entry = self
                                .calls
                                .entry(index)
                                .or_insert_with(|| json!({"partial":""}));
                            entry["id"] = block["id"].clone();
                            entry["name"] = block["name"].clone();
                            entry["input"] = block["input"].clone();
                        } else if block["type"] == "text" {
                            self.append(block["text"].as_str(), delta);
                        }
                    }
                    Some("content_block_delta") if !self.terminated => {
                        let d = &value["delta"];
                        if d["type"] == "text_delta" {
                            self.append(d["text"].as_str(), delta);
                        } else if d["type"] == "input_json_delta" {
                            let index = value["index"].as_u64().ok_or_else(|| {
                                AiError::BadResponse("content delta 缺少 index".into())
                            })?;
                            let entry = self
                                .calls
                                .entry(index)
                                .or_insert_with(|| json!({"partial":""}));
                            let text = format!(
                                "{}{}",
                                entry["partial"].as_str().unwrap_or(""),
                                d["partial_json"].as_str().unwrap_or("")
                            );
                            entry["partial"] = json!(text);
                        }
                    }
                    _ => {}
                }
            }
            Provider::Gemini => {
                if value["usageMetadata"].is_object() {
                    self.usage = value["usageMetadata"].clone();
                }
                if self.terminated {
                    return Ok(());
                }
                if let Some(parts) = value["candidates"][0]["content"]["parts"].as_array() {
                    for part in parts {
                        if part["functionCall"].is_object() {
                            self.parts.push(part.clone());
                        } else if part["thought"] != true {
                            self.append(part["text"].as_str(), delta);
                        }
                    }
                }
                if value["candidates"][0]["finishReason"].is_string() {
                    self.reason = value["candidates"][0]["finishReason"].clone();
                    self.terminated = true;
                }
            }
            Provider::Ollama => {
                if value["eval_count"].is_number() {
                    self.usage["eval_count"] = value["eval_count"].clone();
                }
                if value["prompt_eval_count"].is_number() {
                    self.usage["prompt_eval_count"] = value["prompt_eval_count"].clone();
                }
                if self.terminated {
                    return Ok(());
                }
                self.append(value["message"]["content"].as_str(), delta);
                if let Some(calls) = value["message"]["tool_calls"].as_array() {
                    for call in calls {
                        self.calls.insert(self.calls.len() as u64, call.clone());
                    }
                }
                if value["done"] == true {
                    self.reason = value["done_reason"].clone();
                    self.terminated = true;
                }
            }
        }
        Ok(())
    }
    pub fn finish(mut self, mut delta: impl FnMut(&str)) -> Result<ChatResponse, AiError> {
        // NDJSON can end without a newline; SSE can end after a complete data line.
        if !self.pending.is_empty() {
            let pending = std::mem::take(&mut self.pending);
            self.line(&pending, &mut delta)
                .map_err(|_| self.interrupted("连接 EOF：不完整流式事件"))?;
        }
        if !self.data.is_empty() {
            self.line(b"", &mut delta)
                .map_err(|_| self.interrupted("连接 EOF：不完整流式事件"))?;
        }
        if !self.terminated {
            return Err(self.interrupted("连接 EOF"));
        }
        let calls: Vec<_> = self.calls.into_values().collect();
        let value = match self.provider {
            Provider::OpenAiCompatible => {
                json!({"choices":[{"message":{"content":self.text,"tool_calls":calls},"finish_reason":self.reason}],"usage":self.usage})
            }
            Provider::Ollama => {
                json!({"message":{"content":self.text,"tool_calls":calls},"done":true,"done_reason":self.reason,"eval_count":self.usage["eval_count"],"prompt_eval_count":self.usage["prompt_eval_count"]})
            }
            Provider::Anthropic => {
                let mut content = vec![json!({"type":"text","text":self.text})];
                for call in calls {
                    let input =
                        if let Some(partial) = call["partial"].as_str().filter(|s| !s.is_empty()) {
                            serde_json::from_str::<Value>(partial)
                                .map_err(|_| AiError::BadResponse("工具流式参数不是 JSON".into()))?
                        } else {
                            call["input"].clone()
                        };
                    content.push(json!({"type":"tool_use","id":call["id"],"name":call["name"],"input":input}));
                }
                json!({"content":content,"stop_reason":self.reason,"usage":self.usage})
            }
            Provider::Gemini => {
                self.parts.insert(0, json!({"text":self.text}));
                json!({"candidates":[{"content":{"parts":self.parts},"finishReason":self.reason}],"usageMetadata":self.usage})
            }
        };
        decode_chat_response(self.provider, &value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat::ChatBlock;
    #[test]
    fn utf8_split_empty_chunk_and_missing_terminator() {
        let bytes = "data: {\"choices\":[{\"delta\":{\"content\":\"你好🦀\"}}]}\n\n".as_bytes();
        for split in 0..=bytes.len() {
            let mut p = ChatStreamParser::new(Provider::OpenAiCompatible);
            let mut text = String::new();
            p.push(&bytes[..split], |s| text.push_str(s)).unwrap();
            p.push(&[], |_| panic!("empty chunk delta")).unwrap();
            p.push(&bytes[split..], |s| text.push_str(s)).unwrap();
            assert_eq!(text, "你好🦀");
            assert!(p.finish(|_| {}).unwrap_err().to_string().contains("3 字符"));
        }
        let mut p = ChatStreamParser::new(Provider::OpenAiCompatible);
        p.push(bytes, |_| {}).unwrap();
        p.push(b"", |_| {}).unwrap();
        p.push(b"data: [DONE]\n\n", |_| {}).unwrap();
        assert!(p.finish(|_| {}).is_ok());
    }
    #[test]
    fn two_indexed_tools_and_explicit_duplicate_ids() {
        let mut p = ChatStreamParser::new(Provider::OpenAiCompatible);
        let mut stream = String::new();
        for (event, calls) in [
            (
                "a",
                json!([{"index":1,"id":"b","function":{"name":"second","arguments":"{\"b\":"}},{"index":0,"id":"a","function":{"name":"first","arguments":"{\"a\":"}}]),
            ),
            (
                "b",
                json!([{"index":0,"function":{"arguments":"1}"}},{"index":1,"function":{"arguments":"2}"}}]),
            ),
        ] {
            let frame = format!(
                "id: {event}\ndata: {}\n\n",
                json!({"choices":[{"delta":{"tool_calls":calls}}]})
            );
            stream.push_str(&frame);
            stream.push_str(&frame);
        }
        stream.push_str("data: [DONE]\n\n");
        p.push(stream.as_bytes(), |_| {}).unwrap();
        let blocks = p.finish(|_| {}).unwrap().blocks;
        assert!(
            matches!(&blocks[0], ChatBlock::ToolCall {name,args_json,..} if name=="first" && args_json=="{\"a\":1}")
        );
        assert!(
            matches!(&blocks[1], ChatBlock::ToolCall {name,args_json,..} if name=="second" && args_json=="{\"b\":2}")
        );
    }
    #[test]
    fn truncated_last_event_reports_already_received_characters() {
        let mut p = ChatStreamParser::new(Provider::OpenAiCompatible);
        p.push(
            "data: {\"choices\":[{\"delta\":{\"content\":\"字\"}}]}\n\ndata: {".as_bytes(),
            |_| {},
        )
        .unwrap();
        let error = p.finish(|_| {}).unwrap_err().to_string();
        assert!(error.contains("EOF") && error.contains("1 字符"), "{error}");
        for provider in [Provider::Anthropic, Provider::Gemini, Provider::Ollama] {
            let mut parser = ChatStreamParser::new(provider);
            let frame = if provider == Provider::Ollama {
                b"[DONE]\n".as_slice()
            } else {
                b"data: [DONE]\n\n".as_slice()
            };
            assert!(parser.push(frame, |_| {}).is_err());
        }
    }
    #[test]
    fn identical_tokens_without_event_ids_are_not_duplicates_and_late_usage_is_kept() {
        let mut p = ChatStreamParser::new(Provider::OpenAiCompatible);
        let mut text = String::new();
        for _ in 0..2 {
            p.push(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"ha\"}}]}\n\n",
                |s| text.push_str(s),
            )
            .unwrap();
        }
        p.push(b"data: [DONE]\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"late\"}}],\"usage\":{\"completion_tokens\":2}}\n\n", |s|text.push_str(s)).unwrap();
        assert_eq!(text, "haha");
        assert_eq!(p.finish(|_| {}).unwrap().usage.output_tokens, Some(2));
    }
    #[test]
    fn anthropic_delta_before_start_and_late_text() {
        let mut p = ChatStreamParser::new(Provider::Anthropic);
        for value in [
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"x\":1}"}}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"c","name":"search","input":{}}}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}),
            json!({"type":"message_stop"}),
            json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"late"}}),
        ] {
            p.push(format!("data: {value}\n\n").as_bytes(), |_| {
                panic!("late text")
            })
            .unwrap();
        }
        let response = p.finish(|_| {}).unwrap();
        assert_eq!(response.usage.output_tokens, Some(3));
        assert!(response
            .blocks
            .iter()
            .any(|b| matches!(b,ChatBlock::ToolCall {args_json,..} if args_json=="{\"x\":1}")));
    }
}
