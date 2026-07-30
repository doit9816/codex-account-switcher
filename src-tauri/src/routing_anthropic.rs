use crate::routing_protocol::{response_to_sse, ChatStreamingTransformer, TransformedResponse};
use crate::routing_sse::{SseEvent, SseTransformer, TransformingSseReader};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Read;
use uuid::Uuid;

const DEFAULT_MAX_TOKENS: u64 = 8192;

pub(crate) fn responses_request_to_anthropic(body: Value) -> Result<Value, String> {
    let object = body
        .as_object()
        .ok_or_else(|| "Responses 请求体必须是 JSON 对象".to_string())?;
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Responses 请求缺少 model".to_string())?;

    let mut system = Vec::new();
    if let Some(instructions) = object
        .get("instructions")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        system.push(json!({"type": "text", "text": instructions}));
    }

    let mut messages = Vec::new();
    append_input(&mut messages, &mut system, object.get("input"))?;
    if messages.is_empty() {
        return Err("Responses 请求缺少可转换的 input".to_string());
    }

    let requested_max_tokens = object
        .get("max_output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    let mut anthropic = Map::new();
    anthropic.insert("model".to_string(), Value::String(model.to_string()));
    anthropic.insert("messages".to_string(), Value::Array(messages));
    anthropic.insert(
        "stream".to_string(),
        Value::Bool(
            object
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
    );

    let mut max_tokens = requested_max_tokens;
    if let Some(budget) = reasoning_budget(object, requested_max_tokens) {
        max_tokens = max_tokens.max(budget.saturating_add(1));
        anthropic.insert(
            "thinking".to_string(),
            json!({"type": "enabled", "budget_tokens": budget}),
        );
    } else {
        copy_field(object, &mut anthropic, "temperature", "temperature");
        copy_field(object, &mut anthropic, "top_p", "top_p");
    }
    anthropic.insert("max_tokens".to_string(), Value::from(max_tokens));

    if !system.is_empty() {
        anthropic.insert("system".to_string(), Value::Array(system));
    }
    if let Some(tools) = object.get("tools").and_then(Value::as_array) {
        let converted = tools
            .iter()
            .filter_map(response_tool_to_anthropic)
            .collect::<Vec<_>>();
        if !converted.is_empty() {
            anthropic.insert("tools".to_string(), Value::Array(converted));
        }
    }
    if let Some(choice) = object
        .get("tool_choice")
        .and_then(response_tool_choice_to_anthropic)
    {
        anthropic.insert("tool_choice".to_string(), choice);
    }

    Ok(Value::Object(anthropic))
}

pub(crate) fn transform_anthropic_response(
    body: &[u8],
    content_type: Option<&str>,
    streaming: bool,
) -> Result<TransformedResponse, String> {
    let is_sse = content_type
        .unwrap_or_default()
        .to_ascii_lowercase()
        .contains("text/event-stream")
        || body.windows("data:".len()).any(|window| window == b"data:");
    let message = if is_sse {
        aggregate_anthropic_sse(body)?
    } else {
        serde_json::from_slice(body).map_err(crate::display_err)?
    };
    let response = anthropic_message_to_response(message)?;
    if streaming {
        Ok(TransformedResponse {
            body: response_to_sse(&response)?,
            content_type: "text/event-stream",
        })
    } else {
        Ok(TransformedResponse {
            body: serde_json::to_vec(&response).map_err(crate::display_err)?,
            content_type: "application/json",
        })
    }
}

pub(crate) fn anthropic_sse_reader(reader: Box<dyn Read + Send>) -> Box<dyn Read + Send> {
    Box::new(TransformingSseReader::new(
        reader,
        AnthropicStreamingTransformer::default(),
    ))
}

#[derive(Debug, Default)]
struct AnthropicStreamingTransformer {
    chat: ChatStreamingTransformer,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    reasoning_tokens: u64,
    terminal_sent: bool,
}

impl AnthropicStreamingTransformer {
    fn chat_event(&mut self, value: Value) -> Result<Vec<u8>, String> {
        self.chat.transform(SseEvent {
            event: None,
            data: serde_json::to_string(&value).map_err(crate::display_err)?,
        })
    }

    fn usage_value(&self) -> Value {
        let input = self
            .input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_write_tokens);
        json!({
            "prompt_tokens": input,
            "completion_tokens": self.output_tokens,
            "total_tokens": input.saturating_add(self.output_tokens),
            "prompt_tokens_details": {
                "cached_tokens": self.cache_read_tokens,
                "cache_write_tokens": self.cache_write_tokens
            },
            "completion_tokens_details": {
                "reasoning_tokens": self.reasoning_tokens
            }
        })
    }

    fn update_usage(&mut self, usage: Option<&Value>) {
        let Some(usage) = usage else {
            return;
        };
        if let Some(value) = usage.get("input_tokens").and_then(Value::as_u64) {
            self.input_tokens = value;
        }
        if let Some(value) = usage.get("output_tokens").and_then(Value::as_u64) {
            self.output_tokens = value;
        }
        if let Some(value) = usage.get("cache_read_input_tokens").and_then(Value::as_u64) {
            self.cache_read_tokens = value;
        }
        if let Some(value) = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
        {
            self.cache_write_tokens = value;
        }
        if let Some(value) = usage
            .pointer("/output_tokens_details/thinking_tokens")
            .and_then(Value::as_u64)
        {
            self.reasoning_tokens = value;
        }
    }

    fn model_and_usage_event(&self) -> Value {
        json!({
            "model": self.model,
            "choices": [],
            "usage": self.usage_value()
        })
    }

    fn finish_reason(stop_reason: Option<&str>) -> &'static str {
        match stop_reason {
            Some("max_tokens" | "model_context_window_exceeded") => "length",
            Some("refusal") => "content_filter",
            Some("tool_use") => "tool_calls",
            _ => "stop",
        }
    }
}

impl SseTransformer for AnthropicStreamingTransformer {
    fn transform(&mut self, event: SseEvent) -> Result<Vec<u8>, String> {
        if self.terminal_sent || event.data.trim().is_empty() {
            return Ok(Vec::new());
        }
        let value: Value = serde_json::from_str(&event.data).map_err(crate::display_err)?;
        match value.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(model) = value.pointer("/message/model").and_then(Value::as_str) {
                    self.model = model.to_string();
                }
                self.update_usage(value.pointer("/message/usage"));
                self.chat_event(self.model_and_usage_event())
            }
            Some("content_block_start") => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                let Some(block) = value.get("content_block") else {
                    return Ok(Vec::new());
                };
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                        if text.is_empty() {
                            Ok(Vec::new())
                        } else {
                            self.chat_event(json!({
                                "model": self.model,
                                "choices": [{"delta": {"content": text}}]
                            }))
                        }
                    }
                    Some("thinking") => {
                        let thinking = block.get("thinking").and_then(Value::as_str).unwrap_or("");
                        if thinking.is_empty() {
                            Ok(Vec::new())
                        } else {
                            self.chat_event(json!({
                                "model": self.model,
                                "choices": [{"delta": {"reasoning_content": thinking}}]
                            }))
                        }
                    }
                    Some("tool_use") => {
                        let id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("tool_unknown");
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown");
                        let arguments = block
                            .get("input")
                            .filter(|input| {
                                input.as_object().is_none_or(|object| !object.is_empty())
                            })
                            .map(serde_json::to_string)
                            .transpose()
                            .map_err(crate::display_err)?
                            .unwrap_or_default();
                        self.chat_event(json!({
                            "model": self.model,
                            "choices": [{
                                "delta": {
                                    "tool_calls": [{
                                        "index": index,
                                        "id": id,
                                        "type": "function",
                                        "function": {"name": name, "arguments": arguments}
                                    }]
                                }
                            }]
                        }))
                    }
                    _ => Ok(Vec::new()),
                }
            }
            Some("content_block_delta") => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                let delta = value.get("delta").cloned().unwrap_or_else(|| json!({}));
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => self.chat_event(json!({
                        "model": self.model,
                        "choices": [{
                            "delta": {
                                "content": delta.get("text").and_then(Value::as_str).unwrap_or("")
                            }
                        }]
                    })),
                    Some("thinking_delta") => self.chat_event(json!({
                        "model": self.model,
                        "choices": [{
                            "delta": {
                                "reasoning_content": delta
                                    .get("thinking")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                            }
                        }]
                    })),
                    Some("input_json_delta") => self.chat_event(json!({
                        "model": self.model,
                        "choices": [{
                            "delta": {
                                "tool_calls": [{
                                    "index": index,
                                    "function": {
                                        "arguments": delta
                                            .get("partial_json")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                    }
                                }]
                            }
                        }]
                    })),
                    _ => Ok(Vec::new()),
                }
            }
            Some("message_delta") => {
                self.update_usage(value.get("usage"));
                let finish_reason = Self::finish_reason(
                    value.pointer("/delta/stop_reason").and_then(Value::as_str),
                );
                self.chat_event(json!({
                    "model": self.model,
                    "choices": [{"delta": {}, "finish_reason": finish_reason}],
                    "usage": self.usage_value()
                }))
            }
            Some("message_stop") => {
                self.terminal_sent = true;
                self.chat.transform(SseEvent {
                    event: None,
                    data: "[DONE]".to_string(),
                })
            }
            Some("error") => {
                self.terminal_sent = true;
                self.chat_event(json!({
                    "error": {
                        "message": value
                            .pointer("/error/message")
                            .and_then(Value::as_str)
                            .unwrap_or("Anthropic 上游返回错误")
                    }
                }))
            }
            _ => Ok(Vec::new()),
        }
    }

    fn finish(&mut self) -> Result<Vec<u8>, String> {
        if self.terminal_sent {
            Ok(Vec::new())
        } else {
            self.terminal_sent = true;
            self.chat.finish()
        }
    }
}

fn append_input(
    messages: &mut Vec<Value>,
    system: &mut Vec<Value>,
    input: Option<&Value>,
) -> Result<(), String> {
    match input {
        Some(Value::String(text)) => {
            push_message(
                messages,
                "user",
                vec![json!({"type": "text", "text": text})],
            );
        }
        Some(Value::Array(items)) => {
            for item in items {
                append_input_item(messages, system, item)?;
            }
        }
        Some(Value::Object(_)) => {
            append_input_item(messages, system, input.expect("object input exists"))?;
        }
        Some(_) => return Err("Responses input 格式不受支持".to_string()),
        None => {}
    }
    Ok(())
}

fn append_input_item(
    messages: &mut Vec<Value>,
    system: &mut Vec<Value>,
    item: &Value,
) -> Result<(), String> {
    match item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message")
    {
        "message" => {
            let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
            let blocks = response_content_to_anthropic(item.get("content"))?;
            if matches!(role, "system" | "developer") {
                system.extend(blocks);
            } else if !blocks.is_empty() {
                push_message(
                    messages,
                    if role == "assistant" {
                        "assistant"
                    } else {
                        "user"
                    },
                    blocks,
                );
            }
        }
        "function_call" => {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "function_call 缺少 name".to_string())?;
            let call_id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("call_unknown");
            let input = item
                .get("arguments")
                .map(parse_tool_arguments)
                .transpose()?
                .unwrap_or_else(|| json!({}));
            push_message(
                messages,
                "assistant",
                vec![json!({
                    "type": "tool_use",
                    "id": call_id,
                    "name": name,
                    "input": input
                })],
            );
        }
        "function_call_output" => {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "function_call_output 缺少 call_id".to_string())?;
            push_message(
                messages,
                "user",
                vec![json!({
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": tool_output_text(item.get("output"))
                })],
            );
        }
        "reasoning" => {}
        _ => {
            if item.get("role").is_some() {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                let blocks = response_content_to_anthropic(item.get("content"))?;
                if !blocks.is_empty() {
                    push_message(
                        messages,
                        if role == "assistant" {
                            "assistant"
                        } else {
                            "user"
                        },
                        blocks,
                    );
                }
            }
        }
    }
    Ok(())
}

fn push_message(messages: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }
    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some(role) {
            if let Some(content) = last.get_mut("content").and_then(Value::as_array_mut) {
                content.extend(blocks);
                return;
            }
        }
    }
    messages.push(json!({"role": role, "content": blocks}));
}

fn response_content_to_anthropic(content: Option<&Value>) -> Result<Vec<Value>, String> {
    match content {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(text)) => Ok(vec![json!({"type": "text", "text": text})]),
        Some(Value::Array(parts)) => {
            let mut converted = Vec::new();
            for part in parts {
                match part.get("type").and_then(Value::as_str).unwrap_or("") {
                    "input_text" | "output_text" | "text" => {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            converted.push(json!({"type": "text", "text": text}));
                        }
                    }
                    "input_image" | "image_url" => {
                        if let Some(url) = image_url(part) {
                            converted.push(anthropic_image_block(url));
                        }
                    }
                    _ => {}
                }
            }
            Ok(converted)
        }
        Some(value) => Ok(vec![json!({
            "type": "text",
            "text": serde_json::to_string(value).map_err(crate::display_err)?
        })]),
    }
}

fn image_url(part: &Value) -> Option<&str> {
    part.get("image_url")
        .and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("url").and_then(Value::as_str))
        })
        .or_else(|| part.get("url").and_then(Value::as_str))
}

fn anthropic_image_block(url: &str) -> Value {
    if let Some(data) = url.strip_prefix("data:") {
        if let Some((metadata, payload)) = data.split_once(',') {
            if let Some(media_type) = metadata.strip_suffix(";base64") {
                return json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type,
                        "data": payload
                    }
                });
            }
        }
    }
    json!({
        "type": "image",
        "source": {"type": "url", "url": url}
    })
}

fn response_tool_to_anthropic(tool: &Value) -> Option<Value> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return None;
    }
    let name = tool.get("name").and_then(Value::as_str)?;
    let mut converted = Map::new();
    converted.insert("name".to_string(), Value::String(name.to_string()));
    if let Some(description) = tool.get("description") {
        converted.insert("description".to_string(), description.clone());
    }
    converted.insert(
        "input_schema".to_string(),
        tool.get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
    );
    Some(Value::Object(converted))
}

fn response_tool_choice_to_anthropic(choice: &Value) -> Option<Value> {
    if let Some(value) = choice.as_str() {
        return match value {
            "auto" => Some(json!({"type": "auto"})),
            "required" => Some(json!({"type": "any"})),
            "none" => None,
            _ => None,
        };
    }
    if choice.get("type").and_then(Value::as_str) == Some("function") {
        return choice
            .get("name")
            .and_then(Value::as_str)
            .map(|name| json!({"type": "tool", "name": name}));
    }
    None
}

fn reasoning_budget(object: &Map<String, Value>, max_tokens: u64) -> Option<u64> {
    let effort = object
        .get("reasoning")
        .and_then(|value| value.get("effort"))
        .and_then(Value::as_str)?;
    let desired = match effort {
        "none" | "minimal" => return None,
        "low" => 1024,
        "medium" => 2048,
        "high" => 4096,
        "xhigh" => 8192,
        _ => return None,
    };
    let available = max_tokens.saturating_sub(1);
    (available >= 1024).then_some(desired.min(available))
}

fn parse_tool_arguments(value: &Value) -> Result<Value, String> {
    if let Some(arguments) = value.as_str() {
        return serde_json::from_str(arguments)
            .map_err(|error| format!("工具参数不是有效 JSON: {error}"));
    }
    Ok(value.clone())
}

fn tool_output_text(output: Option<&Value>) -> String {
    match output {
        Some(Value::String(value)) => value.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
        None => String::new(),
    }
}

fn copy_field(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    source_key: &str,
    target_key: &str,
) {
    if let Some(value) = source.get(source_key) {
        target.insert(target_key.to_string(), value.clone());
    }
}

fn aggregate_anthropic_sse(body: &[u8]) -> Result<Value, String> {
    let text = String::from_utf8(body.to_vec()).map_err(crate::display_err)?;
    let mut message = json!({
        "id": format!("msg_{}", Uuid::new_v4().simple()),
        "type": "message",
        "role": "assistant",
        "model": "",
        "content": [],
        "stop_reason": Value::Null,
        "usage": {"input_tokens": 0, "output_tokens": 0}
    });
    let mut blocks = BTreeMap::<usize, Value>::new();
    let mut tool_arguments = BTreeMap::<usize, String>::new();

    for line in text.lines() {
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(data).map_err(crate::display_err)?;
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(started) = event.get("message") {
                    for key in ["id", "model", "role", "usage"] {
                        if let Some(value) = started.get(key) {
                            message[key] = value.clone();
                        }
                    }
                }
            }
            Some("content_block_start") => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                if let Some(block) = event.get("content_block") {
                    blocks.insert(index, block.clone());
                }
            }
            Some("content_block_delta") => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = event.get("delta").cloned().unwrap_or_else(|| json!({}));
                let block = blocks.entry(index).or_insert_with(|| {
                    match delta.get("type").and_then(Value::as_str) {
                        Some("thinking_delta" | "signature_delta") => {
                            json!({"type": "thinking", "thinking": ""})
                        }
                        Some("input_json_delta") => {
                            json!({"type": "tool_use", "id": "tool_unknown", "name": "unknown", "input": {}})
                        }
                        _ => json!({"type": "text", "text": ""}),
                    }
                });
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => append_json_string(
                        block,
                        "text",
                        delta.get("text").and_then(Value::as_str).unwrap_or(""),
                    ),
                    Some("thinking_delta") => append_json_string(
                        block,
                        "thinking",
                        delta.get("thinking").and_then(Value::as_str).unwrap_or(""),
                    ),
                    Some("input_json_delta") => {
                        tool_arguments.entry(index).or_default().push_str(
                            delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                        );
                    }
                    _ => {}
                }
            }
            Some("message_delta") => {
                if let Some(stop_reason) = event.pointer("/delta/stop_reason") {
                    message["stop_reason"] = stop_reason.clone();
                }
                if let Some(output_tokens) = event.pointer("/usage/output_tokens") {
                    message["usage"]["output_tokens"] = output_tokens.clone();
                }
            }
            Some("error") => {
                return Err(event
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Anthropic 上游返回错误")
                    .to_string());
            }
            _ => {}
        }
    }

    for (index, arguments) in tool_arguments {
        if let Some(block) = blocks.get_mut(&index) {
            block["input"] = if arguments.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&arguments)
                    .map_err(|error| format!("Anthropic 工具参数流解析失败: {error}"))?
            };
        }
    }
    message["content"] = Value::Array(blocks.into_values().collect());
    Ok(message)
}

fn append_json_string(target: &mut Value, key: &str, value: &str) {
    let current = target.get(key).and_then(Value::as_str).unwrap_or_default();
    target[key] = Value::String(format!("{current}{value}"));
}

fn anthropic_message_to_response(message: Value) -> Result<Value, String> {
    if message.get("type").and_then(Value::as_str) == Some("error")
        || message.get("error").is_some()
    {
        return Err(message
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("Anthropic 上游返回错误")
            .to_string());
    }
    let mut output = Vec::new();
    for block in message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match block.get("type").and_then(Value::as_str) {
            Some("thinking") => {
                if let Some(thinking) = block
                    .get("thinking")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                {
                    output.push(json!({
                        "id": format!("rs_{}", Uuid::new_v4().simple()),
                        "type": "reasoning",
                        "summary": [{"type": "summary_text", "text": thinking}]
                    }));
                }
            }
            Some("text") => {
                if let Some(text) = block
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                {
                    output.push(json!({
                        "id": format!("msg_{}", Uuid::new_v4().simple()),
                        "type": "message",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "annotations": [],
                            "text": text
                        }]
                    }));
                }
            }
            Some("tool_use") => {
                let call_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool_unknown");
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let arguments =
                    serde_json::to_string(block.get("input").unwrap_or(&Value::Object(Map::new())))
                        .map_err(crate::display_err)?;
                output.push(json!({
                    "id": format!("fc_{}", Uuid::new_v4().simple()),
                    "type": "function_call",
                    "status": "completed",
                    "call_id": call_id,
                    "name": name,
                    "arguments": arguments
                }));
            }
            _ => {}
        }
    }

    let stop_reason = message.get("stop_reason").and_then(Value::as_str);
    let (status, incomplete_reason) = match stop_reason {
        Some("max_tokens" | "model_context_window_exceeded") => {
            ("incomplete", Some("max_output_tokens"))
        }
        Some("refusal") => ("incomplete", Some("content_filter")),
        _ => ("completed", None),
    };
    let usage = responses_usage(message.get("usage"));
    Ok(json!({
        "id": response_id(message.get("id").and_then(Value::as_str)),
        "object": "response",
        "created_at": chrono::Utc::now().timestamp(),
        "status": status,
        "model": message.get("model").cloned().unwrap_or(Value::String(String::new())),
        "output": output,
        "usage": usage,
        "error": Value::Null,
        "incomplete_details": incomplete_reason.map(|reason| json!({"reason": reason}))
    }))
}

fn responses_usage(usage: Option<&Value>) -> Value {
    let fresh_input = usage
        .and_then(|value| value.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .and_then(|value| value.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = usage
        .and_then(|value| value.get("cache_read_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = usage
        .and_then(|value| value.get("cache_creation_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input = fresh_input
        .saturating_add(cache_read)
        .saturating_add(cache_write);
    json!({
        "input_tokens": input,
        "input_tokens_details": {
            "cached_tokens": cache_read,
            "cache_write_tokens": cache_write
        },
        "output_tokens": output,
        "output_tokens_details": {"reasoning_tokens": 0},
        "total_tokens": input.saturating_add(output)
    })
}

fn response_id(message_id: Option<&str>) -> String {
    message_id
        .filter(|value| value.starts_with("resp_"))
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("resp_{}", Uuid::new_v4().simple()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::{Read, Result as IoResult};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct ChunkedReader {
        chunks: VecDeque<Vec<u8>>,
        reads: Arc<AtomicUsize>,
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buffer: &mut [u8]) -> IoResult<usize> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let Some(mut chunk) = self.chunks.pop_front() else {
                return Ok(0);
            };
            let count = chunk.len().min(buffer.len());
            buffer[..count].copy_from_slice(&chunk[..count]);
            if count < chunk.len() {
                chunk.drain(..count);
                self.chunks.push_front(chunk);
            }
            Ok(count)
        }
    }

    #[test]
    fn converts_responses_request_to_anthropic_messages() {
        let converted = responses_request_to_anthropic(json!({
            "model": "claude-sonnet-4-5",
            "instructions": "Be concise.",
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "time?"}]},
                {"type": "function_call", "call_id": "call_1", "name": "clock", "arguments": "{\"zone\":\"UTC\"}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "12:00"}
            ],
            "tools": [{"type": "function", "name": "clock", "parameters": {"type": "object"}}],
            "reasoning": {"effort": "high"},
            "stream": true
        }))
        .unwrap();

        assert_eq!(converted["system"][0]["text"], "Be concise.");
        assert_eq!(converted["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(
            converted["messages"][2]["content"][0]["type"],
            "tool_result"
        );
        assert_eq!(converted["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(converted["thinking"]["budget_tokens"], 4096);
        assert_eq!(converted["stream"], true);
    }

    #[test]
    fn converts_anthropic_sse_to_responses_sse() {
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4-5\",\"role\":\"assistant\",\"usage\":{\"input_tokens\":3,\"output_tokens\":0}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let converted =
            transform_anthropic_response(body.as_bytes(), Some("text/event-stream"), true).unwrap();
        let text = String::from_utf8(converted.body).unwrap();

        assert_eq!(converted.content_type, "text/event-stream");
        assert!(text.contains("event: response.output_text.delta"));
        assert!(text.contains("\"delta\":\"hello\""));
        assert!(text.contains("event: response.completed"));
    }

    #[test]
    fn streams_anthropic_events_before_upstream_eof() {
        let reads = Arc::new(AtomicUsize::new(0));
        let upstream = ChunkedReader {
            chunks: VecDeque::from([
                b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4-5\",\"role\":\"assistant\",\"usage\":{\"input_tokens\":3,\"output_tokens\":0}}}\n\n".to_vec(),
                b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n".to_vec(),
                b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n".to_vec(),
                b"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n".to_vec(),
                b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_vec(),
            ]),
            reads: reads.clone(),
        };
        let mut reader = anthropic_sse_reader(Box::new(upstream));
        let mut first = vec![0_u8; 8192];
        let first_read = reader.read(&mut first).unwrap();
        let first = String::from_utf8(first[..first_read].to_vec()).unwrap();

        assert_eq!(reads.load(Ordering::SeqCst), 3);
        assert!(first.contains("event: response.created"));
        assert!(first.contains("\"delta\":\"hello\""));

        let mut remaining = String::new();
        reader.read_to_string(&mut remaining).unwrap();
        assert!(remaining.contains("event: response.completed"));
        assert!(remaining.contains("\"output_tokens\":2"));
    }

    #[test]
    fn streams_anthropic_thinking_and_tool_arguments() {
        let body = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4-5\",\"role\":\"assistant\",\"usage\":{\"input_tokens\":3,\"output_tokens\":0}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"check\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"tool_1\",\"name\":\"clock\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"zone\\\":\\\"UTC\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":4}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let mut reader =
            anthropic_sse_reader(Box::new(std::io::Cursor::new(body.as_bytes().to_vec())));
        let mut output = String::new();
        reader.read_to_string(&mut output).unwrap();

        assert!(output.contains("event: response.reasoning_summary_text.delta"));
        assert!(output.contains("\"delta\":\"check\""));
        assert!(output.contains("event: response.function_call_arguments.delta"));
        assert!(output.contains("\"call_id\":\"tool_1\""));
        assert!(output.contains(r#"\"zone\":\"UTC\""#));
        assert!(output.contains("\"output_tokens\":4"));
        assert!(output.contains("event: response.completed"));
    }
}
