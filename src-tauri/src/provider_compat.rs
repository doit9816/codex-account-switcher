use serde_json::Value;
use toml_edit::DocumentMut;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderKind {
    Generic,
    DeepSeek,
    LongCat,
}

pub(crate) trait ProviderAdapter: Sync {
    fn build_url(&self, base_url: &str, endpoint: &str) -> Result<String, String>;

    fn prepare_responses_request(&self, body: Value) -> Value {
        sanitize_responses_request(body)
    }

    fn apply_codex_options(&self, document: &mut DocumentMut);
}

struct GenericAdapter;
struct DeepSeekAdapter;
struct LongCatAdapter;

static GENERIC_ADAPTER: GenericAdapter = GenericAdapter;
static DEEPSEEK_ADAPTER: DeepSeekAdapter = DeepSeekAdapter;
static LONGCAT_ADAPTER: LongCatAdapter = LongCatAdapter;

pub(crate) fn provider_kind(provider_id: &str, base_url: &str, model: &str) -> ProviderKind {
    let provider_id = provider_id.trim().to_ascii_lowercase();
    let base_url = base_url.trim().to_ascii_lowercase();
    let model = model.trim().to_ascii_lowercase();

    if provider_id == "longcat"
        || base_url.contains("api.longcat.chat")
        || model.starts_with("longcat-")
    {
        ProviderKind::LongCat
    } else if provider_id == "deepseek"
        || base_url.contains("api.deepseek.com")
        || model.starts_with("deepseek-")
    {
        ProviderKind::DeepSeek
    } else {
        ProviderKind::Generic
    }
}

pub(crate) fn provider_adapter(
    provider_id: &str,
    base_url: &str,
    model: &str,
) -> &'static dyn ProviderAdapter {
    match provider_kind(provider_id, base_url, model) {
        ProviderKind::Generic => &GENERIC_ADAPTER,
        ProviderKind::DeepSeek => &DEEPSEEK_ADAPTER,
        ProviderKind::LongCat => &LONGCAT_ADAPTER,
    }
}

pub(crate) fn is_longcat_base_url(value: &str) -> bool {
    value.to_ascii_lowercase().contains("api.longcat.chat")
}

pub(crate) fn is_longcat_model(value: &str) -> bool {
    value.to_ascii_lowercase().starts_with("longcat-")
}

impl ProviderAdapter for GenericAdapter {
    fn build_url(&self, base_url: &str, endpoint: &str) -> Result<String, String> {
        build_url(base_url, endpoint, true)
    }

    fn apply_codex_options(&self, document: &mut DocumentMut) {
        remove_longcat_codex_options(document);
        remove_deepseek_codex_options(document);
    }
}

impl ProviderAdapter for DeepSeekAdapter {
    fn build_url(&self, base_url: &str, endpoint: &str) -> Result<String, String> {
        build_url(base_url, endpoint, false)
    }

    fn prepare_responses_request(&self, mut body: Value) -> Value {
        body = sanitize_responses_request(body);
        remove_reasoning_context(&mut body);
        if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
            for item in input {
                if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                    if let Some(object) = item.as_object_mut() {
                        object.remove("summary");
                        object.remove("encrypted_content");
                    }
                }
            }
        }
        body
    }

    fn apply_codex_options(&self, document: &mut DocumentMut) {
        remove_longcat_codex_options(document);
        document["preferred_auth_method"] = toml_edit::value("apikey");
        document["forced_login_method"] = toml_edit::value("api");
        document["model_reasoning_effort"] = toml_edit::value("high");
    }
}

impl ProviderAdapter for LongCatAdapter {
    fn build_url(&self, base_url: &str, endpoint: &str) -> Result<String, String> {
        build_url(base_url, endpoint, true)
    }

    fn prepare_responses_request(&self, mut body: Value) -> Value {
        body = sanitize_responses_request(body);
        remove_reasoning_context(&mut body);
        body
    }

    fn apply_codex_options(&self, document: &mut DocumentMut) {
        remove_deepseek_codex_options(document);
        document["disable_response_storage"] = toml_edit::value(true);
        document["web_search"] = toml_edit::value("disabled");
        document["model_reasoning_effort"] = toml_edit::value("high");
        document["model_supports_reasoning_summaries"] = toml_edit::value(true);
    }
}

fn build_url(base_url: &str, endpoint: &str, add_v1_for_origin: bool) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let parsed = Url::parse(trimmed).map_err(|_| "API Base URL 格式不正确".to_string())?;
    let endpoint = endpoint.trim_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with(&format!("/{endpoint}")) {
        return Ok(trimmed.to_string());
    }
    let path = parsed.path().trim_end_matches('/');
    if (path.is_empty() || path == "/") && add_v1_for_origin {
        Ok(format!("{trimmed}/v1/{endpoint}"))
    } else {
        Ok(format!("{trimmed}/{endpoint}"))
    }
}

fn remove_reasoning_context(body: &mut Value) {
    if let Some(reasoning) = body.get_mut("reasoning").and_then(Value::as_object_mut) {
        reasoning.remove("context");
    }
}

pub(crate) fn sanitize_responses_request(mut body: Value) -> Value {
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            if item.get("type").and_then(Value::as_str) != Some("reasoning") {
                continue;
            }
            let Some(object) = item.as_object_mut() else {
                continue;
            };
            if object.get("content").is_some_and(Value::is_array) {
                object.insert("content".to_string(), Value::Null);
            }
        }
    }
    body
}

fn remove_longcat_codex_options(document: &mut DocumentMut) {
    remove_bool_root_if_matches(document, "disable_response_storage", true);
    remove_str_root_if_matches(document, "web_search", "disabled");
    remove_str_root_if_matches(document, "model_reasoning_effort", "high");
    remove_bool_root_if_matches(document, "model_supports_reasoning_summaries", true);
}

fn remove_deepseek_codex_options(document: &mut DocumentMut) {
    remove_str_root_if_matches(document, "preferred_auth_method", "apikey");
    remove_str_root_if_matches(document, "forced_login_method", "api");
    remove_str_root_if_matches(document, "model_reasoning_effort", "high");
}

fn remove_bool_root_if_matches(document: &mut DocumentMut, key: &str, expected: bool) {
    if document.as_table().get(key).and_then(|item| item.as_bool()) == Some(expected) {
        document.as_table_mut().remove(key);
    }
}

fn remove_str_root_if_matches(document: &mut DocumentMut, key: &str, expected: &str) {
    if document.as_table().get(key).and_then(|item| item.as_str()) == Some(expected) {
        document.as_table_mut().remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_provider_kind_from_id_url_and_model() {
        assert_eq!(
            provider_kind("deepseek", "https://example.com", "custom"),
            ProviderKind::DeepSeek
        );
        assert_eq!(
            provider_kind("custom", "https://api.deepseek.com", "custom"),
            ProviderKind::DeepSeek
        );
        assert_eq!(
            provider_kind("custom", "https://example.com", "LongCat-2.0"),
            ProviderKind::LongCat
        );
        assert_eq!(
            provider_kind("custom", "https://example.com/v1", "gpt-5"),
            ProviderKind::Generic
        );
    }

    #[test]
    fn deepseek_uses_origin_responses_endpoint() {
        let adapter = provider_adapter("deepseek", "https://api.deepseek.com", "deepseek-v4-flash");
        assert_eq!(
            adapter
                .build_url("https://api.deepseek.com", "responses")
                .unwrap(),
            "https://api.deepseek.com/responses"
        );
    }

    #[test]
    fn deepseek_removes_unsupported_reasoning_history() {
        let adapter = provider_adapter("deepseek", "https://api.deepseek.com", "deepseek-v4-flash");
        let body = adapter.prepare_responses_request(json!({
            "reasoning": {"effort": "high", "context": "private"},
            "input": [{
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "private"}],
                "encrypted_content": "private"
            }]
        }));

        assert!(body["reasoning"].get("context").is_none());
        assert!(body["input"][0].get("summary").is_none());
        assert!(body["input"][0].get("encrypted_content").is_none());
    }

    #[test]
    fn sanitizes_reasoning_content_arrays_without_touching_messages() {
        let body = sanitize_responses_request(json!({
            "input": [
                {
                    "type": "reasoning",
                    "content": [{"type": "reasoning_text", "text": "private"}]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "hello"}]
                }
            ]
        }));

        assert_eq!(body["input"][0]["content"], Value::Null);
        assert_eq!(body["input"][1]["content"][0]["text"], "hello");
    }

    #[test]
    fn generic_options_cleanup_removes_provider_specific_values() {
        let adapter = provider_adapter("generic", "https://example.com/v1", "gpt-5");
        let mut document = r#"
disable_response_storage = true
web_search = "disabled"
preferred_auth_method = "apikey"
forced_login_method = "api"
model_reasoning_effort = "high"
model_supports_reasoning_summaries = true
"#
        .parse::<DocumentMut>()
        .unwrap();

        adapter.apply_codex_options(&mut document);

        assert!(document.is_empty());
    }
}
