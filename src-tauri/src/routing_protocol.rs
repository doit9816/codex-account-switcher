use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireProtocol {
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

impl WireProtocol {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "responses" | "openai_responses" | "openai-responses" => Ok(Self::Responses),
            "chat"
            | "chat_completions"
            | "chat-completions"
            | "openai_chat"
            | "openai-chat"
            | "openai_chat_completions" => Ok(Self::ChatCompletions),
            "anthropic" | "anthropic_messages" | "anthropic-messages" | "claude" | "messages" => {
                Ok(Self::AnthropicMessages)
            }
            _ => Err(
                "API 协议仅支持 Responses API、Chat Completions 或 Anthropic Messages".to_string(),
            ),
        }
    }

    pub(crate) fn canonical(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::ChatCompletions => "chat_completions",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }
}

pub(crate) fn default_wire_api() -> String {
    WireProtocol::Responses.canonical().to_string()
}

pub(crate) fn normalize_wire_api(value: &str) -> Result<String, String> {
    WireProtocol::parse(value).map(|protocol| protocol.canonical().to_string())
}

pub(crate) struct PreparedApiRequest {
    pub(crate) endpoint: String,
    pub(crate) body: Vec<u8>,
    pub(crate) protocol: WireProtocol,
    pub(crate) streaming: bool,
}

pub(crate) struct TransformedResponse {
    pub(crate) body: Vec<u8>,
    pub(crate) content_type: &'static str,
}

pub(crate) fn prepare_api_request(
    base_url: &str,
    model: &str,
    wire_api: &str,
    mut body: Value,
) -> Result<PreparedApiRequest, String> {
    let protocol = WireProtocol::parse(wire_api)?;
    body["model"] = Value::String(model.to_string());
    let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let body = match protocol {
        WireProtocol::Responses => body,
        WireProtocol::ChatCompletions => responses_request_to_chat(body)?,
        WireProtocol::AnthropicMessages => {
            crate::routing_anthropic::responses_request_to_anthropic(body)?
        }
    };
    Ok(PreparedApiRequest {
        endpoint: endpoint_url(base_url, protocol)?,
        body: serde_json::to_vec(&body).map_err(crate::display_err)?,
        protocol,
        streaming,
    })
}

pub(crate) fn transform_chat_response(
    body: &[u8],
    content_type: Option<&str>,
    streaming: bool,
) -> Result<TransformedResponse, String> {
    let is_sse = content_type
        .unwrap_or_default()
        .to_ascii_lowercase()
        .contains("text/event-stream")
        || body.windows("data:".len()).any(|window| window == b"data:");
    let chat = if is_sse {
        aggregate_chat_sse(body)?
    } else {
        serde_json::from_slice(body).map_err(crate::display_err)?
    };
    let response = chat_completion_to_response(chat)?;
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

fn endpoint_url(base_url: &str, protocol: WireProtocol) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let parsed = Url::parse(trimmed).map_err(|_| "API Base URL 格式不正确".to_string())?;
    let endpoint = match protocol {
        WireProtocol::Responses => "responses",
        WireProtocol::ChatCompletions => "chat/completions",
        WireProtocol::AnthropicMessages => "messages",
    };
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with(&format!("/{endpoint}")) {
        return Ok(trimmed.to_string());
    }
    let path = parsed.path().trim_end_matches('/');
    if path.is_empty() || path == "/" {
        Ok(format!("{trimmed}/v1/{endpoint}"))
    } else {
        Ok(format!("{trimmed}/{endpoint}"))
    }
}

fn responses_request_to_chat(body: Value) -> Result<Value, String> {
    let object = body
        .as_object()
        .ok_or_else(|| "Responses 请求体必须是 JSON 对象".to_string())?;
    let mut messages = Vec::new();
    if let Some(instructions) = object.get("instructions").and_then(Value::as_str) {
        if !instructions.is_empty() {
            messages.push(json!({"role": "system", "content": instructions}));
        }
    }
    append_chat_input(&mut messages, object.get("input"))?;
    if messages.is_empty() {
        return Err("Responses 请求缺少 input".to_string());
    }

    let mut chat = Map::new();
    copy_field(object, &mut chat, "model", "model");
    copy_field(object, &mut chat, "temperature", "temperature");
    copy_field(object, &mut chat, "top_p", "top_p");
    copy_field(
        object,
        &mut chat,
        "parallel_tool_calls",
        "parallel_tool_calls",
    );
    copy_field(object, &mut chat, "max_output_tokens", "max_tokens");
    if let Some(effort) = object
        .get("reasoning")
        .and_then(|value| value.get("effort"))
        .and_then(Value::as_str)
    {
        chat.insert(
            "reasoning_effort".to_string(),
            Value::String(effort.to_string()),
        );
    }
    if let Some(tools) = object.get("tools").and_then(Value::as_array) {
        let converted = tools
            .iter()
            .filter_map(response_tool_to_chat)
            .collect::<Vec<_>>();
        if !converted.is_empty() {
            chat.insert("tools".to_string(), Value::Array(converted));
        }
    }
    if let Some(choice) = object.get("tool_choice") {
        chat.insert(
            "tool_choice".to_string(),
            response_tool_choice_to_chat(choice),
        );
    }
    let streaming = object
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    chat.insert("stream".to_string(), Value::Bool(streaming));
    if streaming {
        chat.insert("stream_options".to_string(), json!({"include_usage": true}));
    }
    chat.insert("messages".to_string(), Value::Array(messages));
    Ok(Value::Object(chat))
}

fn append_chat_input(messages: &mut Vec<Value>, input: Option<&Value>) -> Result<(), String> {
    match input {
        Some(Value::String(text)) => {
            messages.push(json!({"role": "user", "content": text}));
        }
        Some(Value::Array(items)) => {
            let mut pending_reasoning = None;
            for item in items {
                if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                    if let Some(reasoning) = reasoning_item_text(item) {
                        pending_reasoning = Some(reasoning);
                    }
                    continue;
                }
                let previous_len = messages.len();
                append_chat_item(messages, item)?;
                if messages.len() > previous_len {
                    if let Some(reasoning) = pending_reasoning.as_ref() {
                        if messages
                            .last()
                            .and_then(|message| message.get("role"))
                            .and_then(Value::as_str)
                            == Some("assistant")
                        {
                            if let Some(message) = messages.last_mut() {
                                message["reasoning_content"] = Value::String(reasoning.to_string());
                            }
                            pending_reasoning = None;
                        }
                    }
                }
            }
        }
        Some(Value::Object(_)) => append_chat_item(messages, input.unwrap())?,
        Some(_) => return Err("Responses input 格式不受支持".to_string()),
        None => {}
    }
    Ok(())
}

fn append_chat_item(messages: &mut Vec<Value>, item: &Value) -> Result<(), String> {
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    match item_type {
        "message" => {
            let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
            let role = if role == "developer" { "system" } else { role };
            let content = response_content_to_chat(item.get("content"))?;
            messages.push(json!({"role": role, "content": content}));
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
            let arguments = item
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| Value::String("{}".to_string()));
            let arguments = if let Some(value) = arguments.as_str() {
                value.to_string()
            } else {
                serde_json::to_string(&arguments).map_err(crate::display_err)?
            };
            messages.push(json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments}
                }]
            }));
        }
        "function_call_output" => {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "function_call_output 缺少 call_id".to_string())?;
            let output = tool_output_text(item.get("output"));
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": output
            }));
        }
        "reasoning" => {}
        _ => {
            if item.get("role").is_some() {
                let content = response_content_to_chat(item.get("content"))?;
                let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                messages.push(json!({"role": role, "content": content}));
            }
        }
    }
    Ok(())
}

fn response_content_to_chat(content: Option<&Value>) -> Result<Value, String> {
    match content {
        None | Some(Value::Null) => Ok(Value::String(String::new())),
        Some(Value::String(text)) => Ok(Value::String(text.clone())),
        Some(Value::Array(parts)) => {
            let mut converted = Vec::new();
            for part in parts {
                let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
                match part_type {
                    "input_text" | "output_text" | "text" => {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            converted.push(json!({"type": "text", "text": text}));
                        }
                    }
                    "input_image" | "image_url" => {
                        let url = part
                            .get("image_url")
                            .and_then(|value| {
                                value
                                    .as_str()
                                    .or_else(|| value.get("url").and_then(Value::as_str))
                            })
                            .or_else(|| part.get("url").and_then(Value::as_str));
                        if let Some(url) = url {
                            converted.push(json!({
                                "type": "image_url",
                                "image_url": {"url": url}
                            }));
                        }
                    }
                    _ => {}
                }
            }
            Ok(Value::Array(converted))
        }
        Some(value) => serde_json::to_string(value)
            .map(Value::String)
            .map_err(crate::display_err),
    }
}

fn response_tool_to_chat(tool: &Value) -> Option<Value> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return None;
    }
    let name = tool.get("name").and_then(Value::as_str)?;
    let mut function = Map::new();
    function.insert("name".to_string(), Value::String(name.to_string()));
    if let Some(description) = tool.get("description") {
        function.insert("description".to_string(), description.clone());
    }
    function.insert(
        "parameters".to_string(),
        tool.get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
    );
    if let Some(strict) = tool.get("strict") {
        function.insert("strict".to_string(), strict.clone());
    }
    Some(json!({"type": "function", "function": function}))
}

fn response_tool_choice_to_chat(choice: &Value) -> Value {
    if let Some(value) = choice.as_str() {
        return Value::String(value.to_string());
    }
    if choice.get("type").and_then(Value::as_str) == Some("function") {
        if let Some(name) = choice.get("name").and_then(Value::as_str) {
            return json!({"type": "function", "function": {"name": name}});
        }
    }
    choice.clone()
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

fn aggregate_chat_sse(body: &[u8]) -> Result<Value, String> {
    let text = String::from_utf8(body.to_vec()).map_err(crate::display_err)?;
    let mut id = None;
    let mut model = None;
    let mut created = None;
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish_reason = None;
    let mut usage = None;
    let mut tool_calls: BTreeMap<u64, Value> = BTreeMap::new();

    for line in text.lines() {
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(data).map_err(crate::display_err)?;
        if event.get("error").is_some() {
            return Err(event
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Chat Completions 上游返回错误")
                .to_string());
        }
        id = id.or_else(|| {
            event
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });
        model = model.or_else(|| {
            event
                .get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });
        created = created.or_else(|| event.get("created").and_then(Value::as_i64));
        if event.get("usage").is_some() {
            usage = event.get("usage").cloned();
        }
        let Some(choice) = event
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            continue;
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            finish_reason = Some(reason.to_string());
        }
        let Some(delta) = choice.get("delta") else {
            continue;
        };
        if let Some(value) = delta.get("content").and_then(Value::as_str) {
            content.push_str(value);
        }
        if let Some(value) = reasoning_text(delta) {
            reasoning.push_str(&value);
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                let entry = tool_calls.entry(index).or_insert_with(|| {
                    json!({
                        "id": "",
                        "type": "function",
                        "function": {"name": "", "arguments": ""}
                    })
                });
                if let Some(value) = call.get("id").and_then(Value::as_str) {
                    if !value.is_empty() {
                        entry["id"] = Value::String(value.to_string());
                    }
                }
                if let Some(value) = call.pointer("/function/name").and_then(Value::as_str) {
                    if !value.is_empty() {
                        entry["function"]["name"] = Value::String(value.to_string());
                    }
                }
                if let Some(value) = call.pointer("/function/arguments").and_then(Value::as_str) {
                    let current = entry["function"]["arguments"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    entry["function"]["arguments"] = Value::String(format!("{current}{value}"));
                }
            }
        }
    }

    let calls = tool_calls.into_values().collect::<Vec<_>>();
    Ok(json!({
        "id": id.unwrap_or_else(|| format!("chatcmpl_{}", Uuid::new_v4().simple())),
        "object": "chat.completion",
        "created": created.unwrap_or_else(|| chrono::Utc::now().timestamp()),
        "model": model.unwrap_or_default(),
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": if content.is_empty() { Value::Null } else { Value::String(content) },
                "reasoning_content": if reasoning.is_empty() { Value::Null } else { Value::String(reasoning) },
                "tool_calls": if calls.is_empty() { Value::Null } else { Value::Array(calls) }
            },
            "finish_reason": finish_reason.unwrap_or_else(|| "stop".to_string())
        }],
        "usage": usage
    }))
}

fn chat_completion_to_response(chat: Value) -> Result<Value, String> {
    if chat.get("error").is_some() {
        return Ok(chat);
    }
    let choice = chat
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| "Chat Completions 响应缺少 choices".to_string())?;
    let message = choice
        .get("message")
        .ok_or_else(|| "Chat Completions 响应缺少 message".to_string())?;
    let mut output = Vec::new();
    if let Some(reasoning) = reasoning_text(message).filter(|value| !value.is_empty()) {
        output.push(json!({
            "id": format!("rs_{}", Uuid::new_v4().simple()),
            "type": "reasoning",
            "summary": [{"type": "summary_text", "text": reasoning}]
        }));
    }
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        if !content.is_empty() {
            output.push(json!({
                "id": format!("msg_{}", Uuid::new_v4().simple()),
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "annotations": [],
                    "text": content
                }]
            }));
        }
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("call_unknown");
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let arguments = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            output.push(json!({
                "id": format!("fc_{}", Uuid::new_v4().simple()),
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": name,
                "arguments": arguments
            }));
        }
    }

    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("stop");
    let completed = !matches!(finish_reason, "length" | "content_filter");
    let usage = chat_usage_to_responses(chat.get("usage"));
    Ok(json!({
        "id": response_id(chat.get("id").and_then(Value::as_str)),
        "object": "response",
        "created_at": chat.get("created").and_then(Value::as_i64).unwrap_or_else(|| chrono::Utc::now().timestamp()),
        "status": if completed { "completed" } else { "incomplete" },
        "model": chat.get("model").cloned().unwrap_or(Value::String(String::new())),
        "output": output,
        "usage": usage,
        "error": Value::Null,
        "incomplete_details": if completed {
            Value::Null
        } else {
            json!({"reason": if finish_reason == "content_filter" { "content_filter" } else { "max_output_tokens" }})
        }
    }))
}

fn chat_usage_to_responses(usage: Option<&Value>) -> Value {
    let input = usage
        .and_then(|value| value.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .and_then(|value| value.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached = usage
        .and_then(|value| value.pointer("/prompt_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let reasoning = usage
        .and_then(|value| value.pointer("/completion_tokens_details/reasoning_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "input_tokens": input,
        "input_tokens_details": {"cached_tokens": cached},
        "output_tokens": output,
        "output_tokens_details": {"reasoning_tokens": reasoning},
        "total_tokens": usage
            .and_then(|value| value.get("total_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(input + output)
    })
}

fn response_id(chat_id: Option<&str>) -> String {
    chat_id
        .filter(|value| value.starts_with("resp_"))
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("resp_{}", Uuid::new_v4().simple()))
}

fn reasoning_item_text(item: &Value) -> Option<String> {
    item.get("summary")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|value| !value.is_empty())
        .or_else(|| reasoning_text(item))
}

fn reasoning_text(container: &Value) -> Option<String> {
    ["reasoning_content", "reasoning", "reasoning_details"]
        .iter()
        .find_map(|key| container.get(*key).and_then(reasoning_value_text))
        .filter(|value| !value.is_empty())
}

fn reasoning_value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(reasoning_value_text)
                .collect::<Vec<_>>()
                .join("");
            (!text.is_empty()).then_some(text)
        }
        Value::Object(object) => ["text", "content", "reasoning"]
            .iter()
            .find_map(|key| object.get(*key).and_then(reasoning_value_text)),
        _ => None,
    }
}

pub(crate) fn response_to_sse(response: &Value) -> Result<Vec<u8>, String> {
    let mut events = Vec::new();
    let mut sequence = 0_u64;
    let mut created = response.clone();
    created["status"] = Value::String("in_progress".to_string());
    created["output"] = Value::Array(Vec::new());
    push_sse_event(
        &mut events,
        "response.created",
        json!({"type": "response.created", "sequence_number": sequence, "response": created}),
    )?;
    sequence += 1;

    if let Some(output) = response.get("output").and_then(Value::as_array) {
        for (output_index, item) in output.iter().enumerate() {
            let mut added_item = item.clone();
            if item.get("type").and_then(Value::as_str) == Some("message") {
                added_item["content"] = Value::Array(Vec::new());
            } else if item.get("type").and_then(Value::as_str) == Some("function_call") {
                added_item["arguments"] = Value::String(String::new());
            } else if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                added_item["summary"] = Value::Array(Vec::new());
            }
            push_sse_event(
                &mut events,
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "sequence_number": sequence,
                    "output_index": output_index,
                    "item": added_item
                }),
            )?;
            sequence += 1;
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    if let Some(part) = item
                        .get("content")
                        .and_then(Value::as_array)
                        .and_then(|parts| parts.first())
                    {
                        let text = part.get("text").and_then(Value::as_str).unwrap_or("");
                        let empty_part =
                            json!({"type": "output_text", "annotations": [], "text": ""});
                        push_sse_event(
                            &mut events,
                            "response.content_part.added",
                            json!({
                                "type": "response.content_part.added",
                                "sequence_number": sequence,
                                "item_id": item["id"],
                                "output_index": output_index,
                                "content_index": 0,
                                "part": empty_part
                            }),
                        )?;
                        sequence += 1;
                        push_sse_event(
                            &mut events,
                            "response.output_text.delta",
                            json!({
                                "type": "response.output_text.delta",
                                "sequence_number": sequence,
                                "item_id": item["id"],
                                "output_index": output_index,
                                "content_index": 0,
                                "delta": text
                            }),
                        )?;
                        sequence += 1;
                        push_sse_event(
                            &mut events,
                            "response.output_text.done",
                            json!({
                                "type": "response.output_text.done",
                                "sequence_number": sequence,
                                "item_id": item["id"],
                                "output_index": output_index,
                                "content_index": 0,
                                "text": text
                            }),
                        )?;
                        sequence += 1;
                        push_sse_event(
                            &mut events,
                            "response.content_part.done",
                            json!({
                                "type": "response.content_part.done",
                                "sequence_number": sequence,
                                "item_id": item["id"],
                                "output_index": output_index,
                                "content_index": 0,
                                "part": part
                            }),
                        )?;
                        sequence += 1;
                    }
                }
                Some("function_call") => {
                    let arguments = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    push_sse_event(
                        &mut events,
                        "response.function_call_arguments.delta",
                        json!({
                            "type": "response.function_call_arguments.delta",
                            "sequence_number": sequence,
                            "item_id": item["id"],
                            "output_index": output_index,
                            "delta": arguments
                        }),
                    )?;
                    sequence += 1;
                    push_sse_event(
                        &mut events,
                        "response.function_call_arguments.done",
                        json!({
                            "type": "response.function_call_arguments.done",
                            "sequence_number": sequence,
                            "item_id": item["id"],
                            "output_index": output_index,
                            "arguments": arguments
                        }),
                    )?;
                    sequence += 1;
                }
                Some("reasoning") => {
                    if let Some(summary) = item
                        .get("summary")
                        .and_then(Value::as_array)
                        .and_then(|parts| parts.first())
                    {
                        let text = summary.get("text").and_then(Value::as_str).unwrap_or("");
                        push_sse_event(
                            &mut events,
                            "response.reasoning_summary_part.added",
                            json!({
                                "type": "response.reasoning_summary_part.added",
                                "sequence_number": sequence,
                                "item_id": item["id"],
                                "output_index": output_index,
                                "summary_index": 0,
                                "part": {"type": "summary_text", "text": ""}
                            }),
                        )?;
                        sequence += 1;
                        push_sse_event(
                            &mut events,
                            "response.reasoning_summary_text.delta",
                            json!({
                                "type": "response.reasoning_summary_text.delta",
                                "sequence_number": sequence,
                                "item_id": item["id"],
                                "output_index": output_index,
                                "summary_index": 0,
                                "delta": text
                            }),
                        )?;
                        sequence += 1;
                        push_sse_event(
                            &mut events,
                            "response.reasoning_summary_text.done",
                            json!({
                                "type": "response.reasoning_summary_text.done",
                                "sequence_number": sequence,
                                "item_id": item["id"],
                                "output_index": output_index,
                                "summary_index": 0,
                                "text": text
                            }),
                        )?;
                        sequence += 1;
                        push_sse_event(
                            &mut events,
                            "response.reasoning_summary_part.done",
                            json!({
                                "type": "response.reasoning_summary_part.done",
                                "sequence_number": sequence,
                                "item_id": item["id"],
                                "output_index": output_index,
                                "summary_index": 0,
                                "part": summary
                            }),
                        )?;
                        sequence += 1;
                    }
                }
                _ => {}
            }
            push_sse_event(
                &mut events,
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "sequence_number": sequence,
                    "output_index": output_index,
                    "item": item
                }),
            )?;
            sequence += 1;
        }
    }
    let terminal = if response.get("status").and_then(Value::as_str) == Some("completed") {
        "response.completed"
    } else {
        "response.incomplete"
    };
    push_sse_event(
        &mut events,
        terminal,
        json!({"type": terminal, "sequence_number": sequence, "response": response}),
    )?;
    Ok(events)
}

fn push_sse_event(output: &mut Vec<u8>, event: &str, value: Value) -> Result<(), String> {
    output.extend_from_slice(format!("event: {event}\n").as_bytes());
    output.extend_from_slice(b"data: ");
    output.extend_from_slice(
        serde_json::to_string(&value)
            .map_err(crate::display_err)?
            .as_bytes(),
    );
    output.extend_from_slice(b"\n\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_wire_protocols() {
        assert_eq!(normalize_wire_api("responses").unwrap(), "responses");
        assert_eq!(
            normalize_wire_api("openai_chat").unwrap(),
            "chat_completions"
        );
        assert_eq!(
            normalize_wire_api("anthropic").unwrap(),
            "anthropic_messages"
        );
    }

    #[test]
    fn builds_protocol_specific_endpoints() {
        assert_eq!(
            endpoint_url("https://api.example.com", WireProtocol::Responses).unwrap(),
            "https://api.example.com/v1/responses"
        );
        assert_eq!(
            endpoint_url(
                "https://api.example.com/openai/v1",
                WireProtocol::ChatCompletions
            )
            .unwrap(),
            "https://api.example.com/openai/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url(
                "https://api.example.com/v1/chat/completions",
                WireProtocol::ChatCompletions
            )
            .unwrap(),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("https://api.anthropic.com", WireProtocol::AnthropicMessages).unwrap(),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn converts_responses_request_to_chat_completions() {
        let prepared = prepare_api_request(
            "https://api.example.com/v1",
            "glm-5",
            "chat_completions",
            json!({
                "model": "gpt-5",
                "instructions": "Be concise",
                "input": [
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]},
                    {"type": "function_call", "call_id": "call_1", "name": "clock", "arguments": "{}"},
                    {"type": "function_call_output", "call_id": "call_1", "output": "12:00"}
                ],
                "tools": [{"type": "function", "name": "clock", "parameters": {"type": "object"}}],
                "stream": true
            }),
        )
        .unwrap();
        let body: Value = serde_json::from_slice(&prepared.body).unwrap();

        assert_eq!(prepared.protocol, WireProtocol::ChatCompletions);
        assert_eq!(body["model"], "glm-5");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"][0]["text"], "hello");
        assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["tools"][0]["function"]["name"], "clock");
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn converts_chat_sse_to_responses_sse() {
        let body = concat!(
            "data: {\"id\":\"chatcmpl_1\",\"created\":123,\"model\":\"glm-5\",\"choices\":[{\"delta\":{\"content\":\"hello \"}}]}\n\n",
            "data: {\"id\":\"chatcmpl_1\",\"created\":123,\"model\":\"glm-5\",\"choices\":[{\"delta\":{\"content\":\"world\",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"clock\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":4,\"total_tokens\":7}}\n\n",
            "data: [DONE]\n\n"
        );
        let transformed =
            transform_chat_response(body.as_bytes(), Some("text/event-stream"), true).unwrap();
        let text = String::from_utf8(transformed.body).unwrap();

        assert_eq!(transformed.content_type, "text/event-stream");
        assert!(text.contains("event: response.output_text.delta"));
        assert!(text.contains("hello world"));
        assert!(text.contains("event: response.function_call_arguments.done"));
        assert!(text.contains("\"call_id\":\"call_1\""));
        assert!(text.contains("event: response.completed"));
        assert!(text.contains("\"input_tokens\":3"));
    }

    #[test]
    fn keeps_reasoning_history_for_chat_tool_calls() {
        let converted = responses_request_to_chat(json!({
            "model": "kimi-k2.5",
            "input": [
                {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "Need to inspect files."}]
                },
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "read_file",
                    "arguments": "{\"path\":\"a.txt\"}"
                }
            ]
        }))
        .unwrap();

        assert_eq!(
            converted["messages"][0]["reasoning_content"],
            "Need to inspect files."
        );
    }

    #[test]
    fn reads_reasoning_details_from_chat_streams() {
        let body = concat!(
            "data: {\"id\":\"chatcmpl_1\",\"model\":\"mimo\",\"choices\":[{\"delta\":{\"reasoning_details\":[{\"type\":\"reasoning.text\",\"text\":\"think\"}]}}]}\n\n",
            "data: {\"id\":\"chatcmpl_1\",\"model\":\"mimo\",\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let transformed =
            transform_chat_response(body.as_bytes(), Some("text/event-stream"), false).unwrap();
        let response: Value = serde_json::from_slice(&transformed.body).unwrap();

        assert_eq!(response["output"][0]["type"], "reasoning");
        assert_eq!(response["output"][0]["summary"][0]["text"], "think");
        assert_eq!(response["output"][1]["content"][0]["text"], "done");
    }
}
