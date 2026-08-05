use crate::routing_anthropic::{anthropic_sse_reader, transform_anthropic_response};
use crate::routing_protocol::{
    chat_sse_reader, prepare_api_request, transform_chat_response, WireProtocol,
};
use crate::{
    app_data_dir, build_probe_client, decrypt_secret, default_routing_log_retention_days,
    display_err, encrypt_secret, load_master_key, load_store, mutate_store, now_string, parse_time,
    push_event, refresh_auth_json_with_client, refresh_error_requires_relogin,
    replace_file_with_rollback, resolve_codex_home, save_store, should_refresh_access_token,
    summarize_auth, AccountProfile, AppStore, RouteHealth, RoutingMode, RoutingSettings,
    ROUTER_PROVIDER_ID,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use rand::rngs::OsRng;
use rand::RngCore;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const ROUTER_BACKUP_FILE: &str = "config.toml.account-switcher-router.backup";
const ROUTER_AUTH_BACKUP_FILE: &str = "auth.json.account-switcher-router.backup";
const ROUTER_LOG_FILE: &str = "routing-requests.jsonl";
const MAX_REQUEST_BYTES: usize = 20 * 1024 * 1024;
const MAX_TRANSFORM_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ERROR_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_ERROR_MESSAGE_CHARS: usize = 500;
const MAX_SSE_LINE_BYTES: usize = 64 * 1024;
const TEMP_NETWORK_COOLDOWN_SECS: i64 = 60;
const DEFAULT_ROUTING_KEY: &str = "codex-switcher-default-session";
const ROUTER_PROVIDER_MARKER: &str = "codex_switcher_router";
const RESERVED_PROVIDER_IDS: [&str; 3] = ["openai", "ollama", "lmstudio"];

static ROUTER: OnceLock<Mutex<Option<RouterHandle>>> = OnceLock::new();
static STICKY: OnceLock<Mutex<HashMap<String, StickyBinding>>> = OnceLock::new();
static LOG_FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static LAST_LOG_PRUNE_AT: AtomicI64 = AtomicI64::new(0);
static ACTIVE_CONNECTIONS: AtomicU32 = AtomicU32::new(0);

#[derive(Debug)]
struct RouterHandle {
    host: String,
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
struct StickyBinding {
    profile_id: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingStatus {
    running: bool,
    base_url: String,
    access_key: Option<String>,
    active_connections: u32,
    settings: RoutingSettings,
    recent_logs: Vec<RoutingLogEntry>,
    codex_check: RoutingCodexCheck,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingCodexCheck {
    config_path: String,
    auth_path: String,
    selected_provider: Option<String>,
    provider_present: bool,
    base_url_matches: bool,
    token_present: bool,
    auth_mode_matches: bool,
    service_running: bool,
    health_ok: bool,
    diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingLogEntry {
    ts: String,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    wire_protocol: Option<String>,
    #[serde(default)]
    upstream_url: Option<String>,
    session_hash: Option<String>,
    profile_id: Option<String>,
    alias: Option<String>,
    requested_model: Option<String>,
    actual_model: Option<String>,
    status: String,
    http_status: Option<u16>,
    latency_ms: u128,
    fallback: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingProbeResult {
    ok: bool,
    request_id: String,
    http_status: u16,
    elapsed_ms: u128,
    profile_id: Option<String>,
    actual_model: Option<String>,
    response_status: Option<String>,
    output_items: usize,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveRoutingSettingsInput {
    listen_host: String,
    port: u16,
    enabled: bool,
    mode: RoutingMode,
    fixed_profile_id: Option<String>,
    sticky_ttl_secs: u64,
}

#[derive(Debug)]
struct SelectedProfile {
    profile: AccountProfile,
    auth_json: String,
    fallback: Option<String>,
}

#[derive(Debug)]
struct PreparedRequest {
    request_id: String,
    requested_model: Option<String>,
    actual_model: Option<String>,
    protocol: WireProtocol,
    streaming: bool,
    upstream_url: String,
}

#[derive(Debug)]
struct PreparedUpstream {
    url: String,
    auth_token: String,
    account_id: Option<String>,
    body: Vec<u8>,
    request: PreparedRequest,
}

#[derive(Debug)]
enum AttemptOutcome {
    Response(
        reqwest::blocking::Response,
        SelectedProfile,
        PreparedRequest,
    ),
}

#[derive(Debug)]
struct RouterError {
    status: u16,
    message: String,
}

impl RouterError {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

pub(crate) fn status(app: AppHandle) -> Result<RoutingStatus, String> {
    let store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let access_key = decrypt_access_key(&store.settings.routing, &key).ok();
    let running = is_running();
    let settings = store.settings.routing.clone();
    let retention_days = settings.log_retention_days;
    let _ = prune_logs(&app, retention_days);
    let base_url = format!(
        "http://{}:{}/v1",
        display_host(&settings.listen_host),
        settings.port
    );
    Ok(RoutingStatus {
        running,
        base_url,
        access_key: access_key.clone(),
        active_connections: ACTIVE_CONNECTIONS.load(AtomicOrdering::Relaxed),
        settings,
        recent_logs: read_recent_logs(&app, 200, retention_days),
        codex_check: codex_config_check(&app, &store, running, access_key.as_deref()),
    })
}

pub(crate) fn save_settings(
    app: AppHandle,
    input: SaveRoutingSettingsInput,
) -> Result<RoutingStatus, String> {
    validate_listen(&input.listen_host, input.port)?;
    let mut store = load_store(&app)?;
    let fixed_profile_id = input.fixed_profile_id.filter(|id| !id.is_empty());
    validate_fixed_profile(&store, input.mode, fixed_profile_id.as_deref())?;
    store.settings.routing.listen_host = input.listen_host.trim().to_string();
    store.settings.routing.port = input.port;
    store.settings.routing.enabled = input.enabled;
    store.settings.routing.mode = input.mode;
    store.settings.routing.fixed_profile_id = fixed_profile_id;
    store.settings.routing.sticky_ttl_secs = input.sticky_ttl_secs.clamp(60, 86_400);
    ensure_access_key(&app, &mut store)?;
    push_event(&mut store, "info", "已保存路由 API 设置");
    save_store(&app, &store)?;
    if input.enabled {
        start(app.clone())?;
    } else {
        stop(app.clone())?;
    }
    status(app)
}

pub(crate) fn regenerate_access_key(app: AppHandle) -> Result<RoutingStatus, String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let access_key = generate_access_key();
    store.settings.routing.encrypted_access_key =
        Some(encrypt_secret(access_key.as_bytes(), &key)?);
    push_event(&mut store, "info", "已重新生成路由 API Key");
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn save_log_settings(
    app: AppHandle,
    retention_days: u32,
) -> Result<RoutingStatus, String> {
    let retention_days = retention_days.clamp(1, 365);
    mutate_store(&app, |store| {
        store.settings.routing.log_retention_days = retention_days;
        push_event(
            store,
            "info",
            &format!("路由请求日志保留天数已设为 {retention_days} 天"),
        );
        Ok(())
    })?;
    prune_logs(&app, retention_days)?;
    status(app)
}

pub(crate) fn start(app: AppHandle) -> Result<RoutingStatus, String> {
    {
        let mut store = load_store(&app)?;
        validate_fixed_profile(
            &store,
            store.settings.routing.mode,
            store.settings.routing.fixed_profile_id.as_deref(),
        )?;
        validate_listen(
            &store.settings.routing.listen_host,
            store.settings.routing.port,
        )?;
        ensure_access_key(&app, &mut store)?;
        store.settings.routing.enabled = true;
        save_store(&app, &store)?;
    }

    let settings = load_store(&app)?.settings.routing;
    let slot = ROUTER.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().map_err(display_err)?;
    if guard
        .as_ref()
        .is_some_and(|handle| handle.host == settings.listen_host && handle.port == settings.port)
    {
        return status(app);
    }
    drop_existing(&mut guard);

    let host = settings.listen_host.clone();
    let port = settings.port;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread_app = app.clone();
    let server = Server::http((host.as_str(), port)).map_err(|error| {
        let message = format!("路由 API 启动失败: {error}");
        if let Ok(mut store) = load_store(&app) {
            store.settings.routing.enabled = false;
            push_event(&mut store, "warn", &message);
            let _ = save_store(&app, &store);
        }
        message
    })?;
    let thread = thread::spawn(move || run_server(thread_app, server, thread_stop));
    *guard = Some(RouterHandle {
        host: settings.listen_host,
        port,
        stop,
        thread: Some(thread),
    });
    status(app)
}

pub(crate) fn start_for_mesh_share(app: AppHandle) -> Result<RoutingStatus, String> {
    {
        let mut store = load_store(&app)?;
        store.settings.routing.listen_host = "0.0.0.0".to_string();
        store.settings.routing.enabled = true;
        ensure_access_key(&app, &mut store)?;
        save_store(&app, &store)?;
    }
    start(app)
}

pub(crate) fn stop(app: AppHandle) -> Result<RoutingStatus, String> {
    if let Some(slot) = ROUTER.get() {
        let mut guard = slot.lock().map_err(display_err)?;
        drop_existing(&mut guard);
    }
    let mut store = load_store(&app)?;
    store.settings.routing.enabled = false;
    push_event(&mut store, "info", "已停止路由 API 服务");
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn read_logs(app: AppHandle, limit: usize) -> Vec<RoutingLogEntry> {
    let retention_days = load_store(&app)
        .map(|store| store.settings.routing.log_retention_days)
        .unwrap_or_else(|_| default_routing_log_retention_days());
    read_recent_logs(&app, limit.clamp(1, 500), retention_days)
}

pub(crate) fn test_request(app: AppHandle) -> Result<RoutingProbeResult, String> {
    let store = load_store(&app)?;
    validate_fixed_profile(
        &store,
        store.settings.routing.mode,
        store.settings.routing.fixed_profile_id.as_deref(),
    )?;
    let model = test_request_model(&store);
    if !is_running() {
        start(app.clone())?;
    }
    let store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let access_key = decrypt_access_key(&store.settings.routing, &key)?;
    let host = display_host(&store.settings.routing.listen_host);
    let request_id = format!("probe_{}", uuid::Uuid::new_v4().simple());
    let started = Instant::now();
    let response = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(display_err)?
        .post(format!(
            "http://{}:{}/v1/responses",
            host, store.settings.routing.port
        ))
        .bearer_auth(access_key)
        .header("x-client-request-id", &request_id)
        .json(&serde_json::json!({
            "model": model,
            "input": "Reply with exactly OK.",
            "stream": true,
            "store": false
        }))
        .send()
        .map_err(|error| format!("路由测试请求失败: {error}"))?;
    let http_status = response.status().as_u16();
    let profile_id = response
        .headers()
        .get("x-codex-switcher-profile-id")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let actual_model = response
        .headers()
        .get("x-codex-switcher-model")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let is_event_stream = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"));
    let body = response
        .bytes()
        .map_err(|error| format!("无法读取路由测试响应: {error}"))?;
    let body_json = serde_json::from_slice::<Value>(&body).ok();
    let body_text = std::str::from_utf8(&body).unwrap_or_default();
    let mut stream_inspector = ResponseStreamInspector::default();
    stream_inspector.observe(&body);
    stream_inspector.finish();
    let response_status = body_json
        .as_ref()
        .and_then(|body| body.get("status"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| stream_inspector.completed.then(|| "completed".to_string()));
    let output_items = body_json
        .as_ref()
        .and_then(|body| body.get("output"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_else(|| {
            body_text
                .matches("event: response.output_item.done")
                .count()
        });
    let error_message = body_json
        .as_ref()
        .and_then(|body| body.pointer("/error/message"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or(stream_inspector.error)
        .or_else(|| {
            (http_status < 400 && is_event_stream && !stream_inspector.completed)
                .then(|| "上游响应流未完成".to_string())
        });
    Ok(RoutingProbeResult {
        ok: http_status < 400 && error_message.is_none(),
        request_id,
        http_status,
        elapsed_ms: started.elapsed().as_millis(),
        profile_id,
        actual_model,
        response_status,
        output_items,
        message: error_message.unwrap_or_else(|| {
            if http_status < 400 {
                "路由测试请求成功".to_string()
            } else {
                format!("路由测试返回 HTTP {http_status}")
            }
        }),
    })
}

fn validate_fixed_profile(
    store: &AppStore,
    mode: RoutingMode,
    fixed_profile_id: Option<&str>,
) -> Result<(), String> {
    if mode != RoutingMode::Fixed {
        return Ok(());
    }
    let profile_id =
        fixed_profile_id.ok_or_else(|| "固定账号模式必须先选择一个账号".to_string())?;
    if store
        .profiles
        .iter()
        .any(|profile| profile.id == profile_id)
    {
        Ok(())
    } else {
        Err("固定账号不存在，请重新选择".to_string())
    }
}

fn config_selects_router(document: &toml_edit::DocumentMut) -> bool {
    selected_provider_id(document).is_some_and(|provider_id| {
        provider_id == ROUTER_PROVIDER_ID
            || selected_provider(document).is_some_and(provider_marked_router)
    })
}

fn selected_provider_id(document: &toml_edit::DocumentMut) -> Option<&str> {
    document
        .get("model_provider")
        .and_then(|item| item.as_str())
}

fn selected_provider<'a>(document: &'a toml_edit::DocumentMut) -> Option<&'a toml_edit::Item> {
    let provider_id = selected_provider_id(document)?;
    document
        .get("model_providers")
        .and_then(|providers| providers.get(provider_id))
}

fn provider_marked_router(provider: &toml_edit::Item) -> bool {
    provider
        .get(ROUTER_PROVIDER_MARKER)
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
}

fn refresh_router_backup(
    config_path: &std::path::Path,
    backup_path: &std::path::Path,
    document: &toml_edit::DocumentMut,
) -> Result<(), String> {
    if config_selects_router(document) && backup_path.exists() {
        return Ok(());
    }
    if config_path.exists() {
        fs::copy(config_path, backup_path).map_err(display_err)?;
    } else {
        fs::write(backup_path, []).map_err(display_err)?;
    }
    Ok(())
}

fn router_token_from_config(document: &toml_edit::DocumentMut) -> Option<&str> {
    if !config_selects_router(document) {
        return None;
    }
    selected_provider(document)
        .and_then(|provider| provider.get("experimental_bearer_token"))
        .and_then(|token| token.as_str())
}

fn selected_provider_id_from_file(path: &std::path::Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()?
        .parse::<toml_edit::DocumentMut>()
        .ok()
        .and_then(|document| selected_provider_id(&document).map(ToString::to_string))
}

fn custom_takeover_provider_id(
    current_provider_id: Option<&str>,
    backup_provider_id: Option<&str>,
) -> String {
    let candidate = if current_provider_id == Some(ROUTER_PROVIDER_ID) {
        backup_provider_id
    } else {
        current_provider_id
    };
    candidate
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .filter(|id| !RESERVED_PROVIDER_IDS.contains(id))
        .unwrap_or(ROUTER_PROVIDER_ID)
        .to_string()
}

fn auth_selects_router(contents: &str, access_key: &str) -> bool {
    serde_json::from_str::<Value>(contents)
        .ok()
        .is_some_and(|auth| {
            auth.get("auth_mode").and_then(Value::as_str) == Some("apikey")
                && auth.get("OPENAI_API_KEY").and_then(Value::as_str) == Some(access_key)
        })
}

fn refresh_router_auth_backup(
    auth_path: &std::path::Path,
    backup_path: &std::path::Path,
    current_router_token: Option<&str>,
) -> Result<(), String> {
    let current = fs::read_to_string(auth_path).unwrap_or_default();
    let currently_routed =
        current_router_token.is_some_and(|token| auth_selects_router(&current, token));
    if currently_routed && backup_path.exists() {
        return Ok(());
    }
    if auth_path.exists() {
        fs::copy(auth_path, backup_path).map_err(display_err)?;
    } else {
        fs::write(backup_path, []).map_err(display_err)?;
    }
    Ok(())
}

fn router_auth_json(access_key: &str) -> Result<Vec<u8>, String> {
    let mut auth = serde_json::to_vec_pretty(&serde_json::json!({
        "auth_mode": "apikey",
        "OPENAI_API_KEY": access_key,
    }))
    .map_err(display_err)?;
    auth.push(b'\n');
    Ok(auth)
}

fn restore_backup_file(
    target_path: &std::path::Path,
    backup_path: &std::path::Path,
) -> Result<(), String> {
    let backup = fs::read(backup_path).map_err(display_err)?;
    if backup.is_empty() {
        if target_path.exists() {
            fs::remove_file(target_path).map_err(display_err)?;
        }
    } else {
        replace_file_with_rollback(target_path, &backup, None)?;
    }
    Ok(())
}

pub(crate) fn apply_codex_config(app: AppHandle) -> Result<RoutingStatus, String> {
    let mut store = load_store(&app)?;
    ensure_access_key(&app, &mut store)?;
    let key = load_master_key(&app)?;
    let access_key = decrypt_access_key(&store.settings.routing, &key)?;
    let codex_home = resolve_codex_home(&app, store.settings.codex_home.clone())?;
    fs::create_dir_all(&codex_home).map_err(display_err)?;
    let config_path = codex_home.join("config.toml");
    let backup_path = codex_home.join(ROUTER_BACKUP_FILE);
    let auth_path = codex_home.join("auth.json");
    let auth_backup_path = codex_home.join(ROUTER_AUTH_BACKUP_FILE);
    let current = fs::read_to_string(&config_path).unwrap_or_default();
    let mut document = if current.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        current
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| format!("config.toml 解析失败: {error}"))?
    };
    let current_router_token = router_token_from_config(&document).map(ToString::to_string);
    refresh_router_backup(&config_path, &backup_path, &document)?;
    refresh_router_auth_backup(
        &auth_path,
        &auth_backup_path,
        current_router_token.as_deref(),
    )?;
    let current_provider_id = selected_provider_id(&document)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string);
    let backup_provider_id = (current_provider_id.as_deref() == Some(ROUTER_PROVIDER_ID))
        .then(|| selected_provider_id_from_file(&backup_path))
        .flatten();
    let provider_id = custom_takeover_provider_id(
        current_provider_id.as_deref(),
        backup_provider_id.as_deref(),
    );
    document["model_provider"] = toml_edit::value(&provider_id);
    if !document.as_table().contains_key("model_providers")
        || !document["model_providers"].is_table()
    {
        document["model_providers"] = toml_edit::table();
    }
    document["model_providers"][&provider_id] = toml_edit::Item::Table(toml_edit::Table::new());
    let provider = document["model_providers"][&provider_id]
        .as_table_mut()
        .ok_or_else(|| "无法写入 Codex 路由 provider 配置".to_string())?;
    provider["name"] = toml_edit::value("CodexSwitcher Router");
    provider["base_url"] = toml_edit::value(format!(
        "http://{}:{}/v1",
        display_host(&store.settings.routing.listen_host),
        store.settings.routing.port
    ));
    provider["wire_api"] = toml_edit::value("responses");
    provider["experimental_bearer_token"] = toml_edit::value(&access_key);
    provider["requires_openai_auth"] = toml_edit::value(false);
    provider["request_max_retries"] = toml_edit::value(0);
    provider["stream_max_retries"] = toml_edit::value(0);
    provider["supports_websockets"] = toml_edit::value(false);
    provider[ROUTER_PROVIDER_MARKER] = toml_edit::value(true);
    let auth_json = router_auth_json(&access_key)?;
    replace_file_with_rollback(&auth_path, &auth_json, Some(&auth_backup_path))?;
    if let Err(error) = replace_file_with_rollback(
        &config_path,
        document.to_string().as_bytes(),
        Some(&backup_path),
    ) {
        let _ = restore_backup_file(&auth_path, &auth_backup_path);
        return Err(error);
    }

    store.settings.routing.applied_to_codex = true;
    store.settings.codex_home = Some(codex_home.to_string_lossy().to_string());
    match crate::codex_sessions::repair_session_visibility(&codex_home, &provider_id) {
        Ok(report) => {
            if report.changed() {
                push_event(&mut store, "info", &report.summary());
            } else if report.skipped_databases > 0 {
                push_event(&mut store, "warn", &report.summary());
            }
        }
        Err(error) => {
            push_event(
                &mut store,
                "warn",
                &format!("Codex 会话可见性修复失败，路由接管已继续: {error}"),
            );
        }
    }
    push_event(
        &mut store,
        "info",
        "已将本机 Codex provider 和认证模式接管到路由 API",
    );
    save_store(&app, &store)?;
    status(app)
}

fn codex_config_check(
    app: &AppHandle,
    store: &AppStore,
    running: bool,
    access_key: Option<&str>,
) -> RoutingCodexCheck {
    let codex_home = resolve_codex_home(app, store.settings.codex_home.clone())
        .unwrap_or_else(|_| app_data_dir(app).unwrap_or_else(|_| std::path::PathBuf::from(".")));
    let config_path = codex_home.join("config.toml");
    let expected_base_url = format!(
        "http://{}:{}/v1",
        display_host(&store.settings.routing.listen_host),
        store.settings.routing.port
    );
    let auth_path = codex_home.join("auth.json");
    let mut check = RoutingCodexCheck {
        config_path: config_path.to_string_lossy().to_string(),
        auth_path: auth_path.to_string_lossy().to_string(),
        selected_provider: None,
        provider_present: false,
        base_url_matches: false,
        token_present: false,
        auth_mode_matches: false,
        service_running: running,
        health_ok: false,
        diagnostics: Vec::new(),
    };

    match fs::read_to_string(&config_path) {
        Ok(contents) if contents.trim().is_empty() => {
            check.diagnostics.push("config.toml 为空".to_string());
        }
        Ok(contents) => match contents.parse::<toml_edit::DocumentMut>() {
            Ok(document) => {
                check.selected_provider = document
                    .get("model_provider")
                    .and_then(|item| item.as_str())
                    .map(ToString::to_string);
                let Some(provider_id) = check.selected_provider.clone() else {
                    check
                        .diagnostics
                        .push("当前未设置 model_provider".to_string());
                    check.health_ok = probe_router_health(&expected_base_url);
                    return check;
                };
                let Some(provider) = document
                    .get("model_providers")
                    .and_then(|providers| providers.get(&provider_id))
                else {
                    check
                        .diagnostics
                        .push(format!("未找到当前 provider {provider_id} 的路由配置"));
                    check.health_ok = probe_router_health(&expected_base_url);
                    return check;
                };

                let base_url = provider
                    .get("base_url")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default();
                check.base_url_matches = base_url == expected_base_url;
                check.provider_present = provider_id == ROUTER_PROVIDER_ID
                    || provider_marked_router(provider)
                    || check.base_url_matches;
                check.token_present = provider
                    .get("experimental_bearer_token")
                    .and_then(|item| item.as_str())
                    .is_some_and(|token| !token.trim().is_empty());

                if !check.provider_present {
                    check
                        .diagnostics
                        .push("当前 model_provider 未指向路由配置".to_string());
                }
                if !check.base_url_matches {
                    check
                        .diagnostics
                        .push("Codex 配置里的 Base URL 与当前路由地址不一致".to_string());
                }
                if !check.token_present {
                    check
                        .diagnostics
                        .push("路由 API Key 未写入 Codex 配置".to_string());
                }
            }
            Err(error) => check
                .diagnostics
                .push(format!("config.toml 解析失败: {error}")),
        },
        Err(error) => check
            .diagnostics
            .push(format!("无法读取 config.toml: {error}")),
    }

    check.auth_mode_matches = access_key.is_some_and(|access_key| {
        fs::read_to_string(&auth_path)
            .ok()
            .is_some_and(|contents| auth_selects_router(&contents, access_key))
    });
    if !check.auth_mode_matches {
        check
            .diagnostics
            .push("Codex auth.json 未切换到路由 API Key 模式".to_string());
    }

    check.health_ok = probe_router_health(&expected_base_url);
    if check.provider_present
        && check.base_url_matches
        && check.token_present
        && check.auth_mode_matches
        && check.service_running
        && check.health_ok
        && check.diagnostics.is_empty()
    {
        check.diagnostics.push(
            "配置与服务自检通过；若仍无日志，请重启 Codex 或新建会话后再发送一次请求".to_string(),
        );
    }
    check
}

fn probe_router_health(base_url: &str) -> bool {
    let health_url = base_url.trim_end_matches("/v1").to_string() + "/health";
    Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .and_then(|client| client.get(health_url).send())
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

pub(crate) fn restore_codex_config(app: AppHandle) -> Result<RoutingStatus, String> {
    let mut store = load_store(&app)?;
    let codex_home = resolve_codex_home(&app, store.settings.codex_home.clone())?;
    let config_path = codex_home.join("config.toml");
    let backup_path = codex_home.join(ROUTER_BACKUP_FILE);
    let auth_path = codex_home.join("auth.json");
    let auth_backup_path = codex_home.join(ROUTER_AUTH_BACKUP_FILE);
    let current = fs::read_to_string(&config_path).unwrap_or_default();
    let current_document = current.parse::<toml_edit::DocumentMut>().ok();
    let currently_routed = current_document.as_ref().is_some_and(config_selects_router);
    let current_router_token = current_document
        .as_ref()
        .and_then(router_token_from_config)
        .map(ToString::to_string);
    let auth_currently_routed = current_router_token.is_some_and(|token| {
        fs::read_to_string(&auth_path)
            .ok()
            .is_some_and(|contents| auth_selects_router(&contents, &token))
    });
    if auth_backup_path.exists() {
        if auth_currently_routed {
            restore_backup_file(&auth_path, &auth_backup_path)?;
        }
        fs::remove_file(&auth_backup_path).map_err(display_err)?;
    }
    if backup_path.exists() {
        if currently_routed {
            restore_backup_file(&config_path, &backup_path)?;
        }
        fs::remove_file(&backup_path).map_err(display_err)?;
    }
    store.settings.routing.applied_to_codex = false;
    push_event(
        &mut store,
        "info",
        "已恢复接管前的 Codex provider 和认证配置",
    );
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn restore_enabled(app: AppHandle) {
    let Ok(store) = load_store(&app) else {
        return;
    };
    if store.settings.routing.enabled {
        let _ = thread::Builder::new()
            .name("routing-restore".to_string())
            .spawn(move || {
                if let Err(error) = start(app.clone()) {
                    if let Ok(mut store) = load_store(&app) {
                        push_event(
                            &mut store,
                            "warn",
                            &format!("路由 API 自动恢复失败：{error}"),
                        );
                        let _ = save_store(&app, &store);
                    }
                }
            });
    }
}

fn run_server(app: AppHandle, server: Server, stop: Arc<AtomicBool>) {
    while !stop.load(AtomicOrdering::Relaxed) {
        match server.recv_timeout(Duration::from_millis(400)) {
            Ok(Some(request)) => handle_request(app.clone(), request),
            Ok(None) => {}
            Err(_) => break,
        }
    }
}

fn handle_request(app: AppHandle, mut request: Request) {
    let started = Instant::now();
    let method = request.method().clone();
    let url = request.url().to_string();
    let result = if method == Method::Get && url == "/health" {
        respond_json(
            request,
            200,
            &serde_json::json!({
                "status": "ok",
                "activeConnections": ACTIVE_CONNECTIONS.load(AtomicOrdering::Relaxed)
            }),
        )
    } else if method == Method::Post && url == "/mesh/sync" {
        crate::easytier_mesh::handle_sync_request(app, request).map_err(|error| {
            Box::new(std::io::Error::other(error)) as Box<dyn std::error::Error + Send + Sync>
        })
    } else if method == Method::Post && url == "/mesh/pull" {
        crate::easytier_mesh::handle_pull_request(app, request).map_err(|error| {
            Box::new(std::io::Error::other(error)) as Box<dyn std::error::Error + Send + Sync>
        })
    } else if method == Method::Post && url == "/v1/responses" {
        match proxy_responses(app.clone(), &mut request, started) {
            Ok(response) => {
                let _ = request.respond(response);
                Ok(())
            }
            Err(error) => respond_error(app, request, started, error),
        }
    } else {
        respond_error(
            app,
            request,
            started,
            RouterError::new(404, "route not found"),
        )
    };
    let _ = result;
}

fn proxy_responses(
    app: AppHandle,
    request: &mut Request,
    started: Instant,
) -> Result<Response<Box<dyn Read + Send>>, RouterError> {
    let headers = request_headers(request);
    let auth = header_value(&headers, "authorization").unwrap_or_default();
    let store = load_store(&app).map_err(|e| RouterError::new(500, e))?;
    let key = load_master_key(&app).map_err(|e| RouterError::new(500, e))?;
    let expected = decrypt_access_key(&store.settings.routing, &key)
        .map_err(|_| RouterError::new(500, "routing API key is not configured"))?;
    if auth != format!("Bearer {expected}") {
        return Err(RouterError::new(401, "invalid routing API key"));
    }
    let body = read_body(request)?;
    let decoded = decode_body(&body, header_value(&headers, "content-encoding").as_deref())?;
    let routing_key = routing_key(&headers, &decoded);
    let request_id = header_value(&headers, "x-client-request-id")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("route_{}", uuid::Uuid::new_v4().simple()));
    let session_hash = routing_key.as_deref().map(hash_text);
    let mut attempted = HashSet::new();
    let mut last_error = None;
    let mut fallback_note = None;

    for attempt in 0..4 {
        let selected = select_profile(
            &app,
            routing_key.as_deref(),
            &attempted,
            fallback_note.clone(),
        )
        .map_err(|message| RouterError::new(503, message))?;
        attempted.insert(selected.profile.id.clone());
        match prepare_and_send(&headers, &decoded, selected, &request_id) {
            Ok(AttemptOutcome::Response(response, selected, prepared)) => {
                let status = response.status().as_u16();
                if matches!(status, 401 | 403)
                    && selected.profile.api_config.is_none()
                    && attempt < 3
                    && refresh_profile_access(&app, &selected.profile.id).is_ok()
                {
                    attempted.remove(&selected.profile.id);
                    fallback_note = Some("token refreshed".to_string());
                    continue;
                }
                if is_retriable_status(status) && attempt < 3 {
                    mark_route_failure(
                        &app,
                        &selected.profile.id,
                        Some(status),
                        "upstream returned retryable status",
                    );
                    last_error = Some(format!("HTTP {status}"));
                    fallback_note = Some(format!("HTTP {status}"));
                    continue;
                }
                mark_route_started(&app, &selected.profile.id);
                bind_sticky(
                    routing_key.as_deref(),
                    &selected.profile.id,
                    store.settings.routing.sticky_ttl_secs,
                );
                return build_stream_response(
                    app,
                    response,
                    selected,
                    prepared,
                    session_hash,
                    started,
                );
            }
            Err(error) => {
                mark_route_failure(&app, &error.0, None, &error.1);
                last_error = Some(error.1.clone());
                fallback_note = Some(error.1);
            }
        }
    }

    Err(RouterError::new(
        503,
        last_error.unwrap_or_else(|| "no available upstream account".to_string()),
    ))
}

fn prepare_and_send(
    headers: &HashMap<String, String>,
    decoded_body: &[u8],
    selected: SelectedProfile,
    request_id: &str,
) -> Result<AttemptOutcome, (String, String)> {
    let prepared = prepare_upstream(&selected, decoded_body, request_id)
        .map_err(|error| (selected.profile.id.clone(), error))?;
    let accept = if prepared.request.streaming {
        "text/event-stream"
    } else {
        "application/json"
    };
    let mut builder = blocking_client()
        .post(&prepared.url)
        .header("accept", accept)
        .header("content-type", "application/json");
    builder = if prepared.request.protocol == WireProtocol::AnthropicMessages {
        builder
            .header("x-api-key", &prepared.auth_token)
            .header("anthropic-version", "2023-06-01")
    } else {
        builder.bearer_auth(&prepared.auth_token)
    };
    if let Some(account_id) = &prepared.account_id {
        builder = builder.header("ChatGPT-Account-Id", account_id);
    }
    for (name, value) in headers {
        if preserve_client_header(name) {
            builder = builder.header(name, value);
        }
    }
    let response = builder
        .body(prepared.body)
        .send()
        .map_err(|error| (selected.profile.id.clone(), error.to_string()))?;
    Ok(AttemptOutcome::Response(
        response,
        selected,
        prepared.request,
    ))
}

fn refresh_profile_access(app: &AppHandle, profile_id: &str) -> Result<(), String> {
    let store = load_store(app)?;
    let key = load_master_key(app)?;
    let proxy = store.settings.probe_proxy.clone();
    let profile = store
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| "profile not found".to_string())?;
    if profile.api_config.is_some() {
        return Err("API Provider does not support OAuth refresh".to_string());
    }
    let auth_json = String::from_utf8(decrypt_secret(&profile.encrypted_auth_json, &key)?)
        .map_err(display_err)?;
    let client = build_probe_client(&proxy)?;
    let runtime = tokio::runtime::Runtime::new().map_err(display_err)?;
    let updated = runtime.block_on(refresh_auth_json_with_client(&client, &auth_json))?;
    let summary = summarize_auth(&updated)?;
    let encrypted_auth_json = encrypt_secret(updated.as_bytes(), &key)?;
    mutate_store(app, |store| {
        let profile = store
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| "profile not found".to_string())?;
        profile.summary = summary;
        profile.encrypted_auth_json = encrypted_auth_json;
        profile.usage.last_token_refresh_at = Some(now_string());
        profile.usage.last_token_refresh_status = Some("ok".to_string());
        profile.usage.last_token_refresh_error = None;
        push_event(store, "info", "路由请求已刷新 OAuth access token");
        Ok(())
    })
}

fn prepare_upstream(
    selected: &SelectedProfile,
    decoded_body: &[u8],
    request_id: &str,
) -> Result<PreparedUpstream, String> {
    let mut json: Value = serde_json::from_slice(decoded_body).map_err(display_err)?;
    let requested_model = json
        .get("model")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    if let Some(api_config) = &selected.profile.api_config {
        let auth: Value = serde_json::from_str(&selected.auth_json).map_err(display_err)?;
        let api_key = auth
            .get("OPENAI_API_KEY")
            .and_then(Value::as_str)
            .ok_or_else(|| "API Provider missing OPENAI_API_KEY".to_string())?
            .to_string();
        let prepared = prepare_api_request(
            &api_config.provider_id,
            &api_config.base_url,
            &api_config.model,
            &api_config.wire_api,
            json,
        )?;
        return Ok(PreparedUpstream {
            url: prepared.endpoint.clone(),
            auth_token: api_key,
            account_id: None,
            body: prepared.body,
            request: PreparedRequest {
                request_id: request_id.to_string(),
                requested_model,
                actual_model: Some(api_config.model.clone()),
                protocol: prepared.protocol,
                streaming: prepared.streaming,
                upstream_url: prepared.endpoint,
            },
        });
    }

    let auth: Value = serde_json::from_str(&selected.auth_json).map_err(display_err)?;
    let access_token = auth
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "auth.json missing access_token".to_string())?
        .to_string();
    normalize_oauth_input(&mut json);
    json = crate::provider_compat::sanitize_responses_request(json);
    let streaming = json.get("stream").and_then(Value::as_bool).unwrap_or(false);
    Ok(PreparedUpstream {
        url: "https://chatgpt.com/backend-api/codex/responses".to_string(),
        auth_token: access_token,
        account_id: selected.profile.summary.account_id.clone(),
        body: serde_json::to_vec(&json).map_err(display_err)?,
        request: PreparedRequest {
            request_id: request_id.to_string(),
            requested_model: requested_model.clone(),
            actual_model: requested_model,
            protocol: WireProtocol::Responses,
            streaming,
            upstream_url: "https://chatgpt.com/backend-api/codex/responses".to_string(),
        },
    })
}

fn build_stream_response(
    app: AppHandle,
    mut upstream: reqwest::blocking::Response,
    selected: SelectedProfile,
    prepared: PreparedRequest,
    session_hash: Option<String>,
    started: Instant,
) -> Result<Response<Box<dyn Read + Send>>, RouterError> {
    let status = upstream.status().as_u16();
    let upstream_content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let mut headers = upstream
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            let lower = name.as_str().to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "content-length"
                    | "content-encoding"
                    | "connection"
                    | "transfer-encoding"
                    | "content-type"
            ) {
                return None;
            }
            Header::from_bytes(name.as_str().as_bytes(), value.as_bytes()).ok()
        })
        .collect::<Vec<_>>();
    let mut upstream_error = None;
    let body_reader: Box<dyn Read + Send> = if status >= 400 {
        let mut body = Vec::new();
        (&mut upstream)
            .take(MAX_ERROR_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|error| RouterError::new(502, format!("读取上游错误响应失败: {error}")))?;
        if body.len() > MAX_ERROR_RESPONSE_BYTES {
            upstream_error = Some("上游错误响应过大，详情已省略".to_string());
        } else {
            upstream_error = summarize_upstream_error(&body);
        }
        if let Some(content_type) = upstream_content_type {
            if let Ok(header) =
                Header::from_bytes(b"content-type".as_slice(), content_type.as_bytes())
            {
                headers.push(header);
            }
        }
        Box::new(Cursor::new(body).chain(upstream))
    } else if matches!(
        prepared.protocol,
        WireProtocol::ChatCompletions | WireProtocol::AnthropicMessages
    ) {
        let upstream_is_sse = prepared.streaming
            && upstream_content_type
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"));
        if upstream_is_sse {
            if let Ok(header) = Header::from_bytes(b"content-type".as_slice(), b"text/event-stream")
            {
                headers.push(header);
            }
            match prepared.protocol {
                WireProtocol::ChatCompletions => chat_sse_reader(Box::new(upstream)),
                WireProtocol::AnthropicMessages => anthropic_sse_reader(Box::new(upstream)),
                WireProtocol::Responses => {
                    unreachable!("Responses responses are passed through")
                }
            }
        } else {
            let mut body = Vec::new();
            (&mut upstream)
                .take(MAX_TRANSFORM_RESPONSE_BYTES as u64 + 1)
                .read_to_end(&mut body)
                .map_err(|error| RouterError::new(502, format!("读取上游响应失败: {error}")))?;
            if body.len() > MAX_TRANSFORM_RESPONSE_BYTES {
                return Err(RouterError::new(502, "上游响应过大，无法转换协议"));
            }
            let transformed = match prepared.protocol {
                WireProtocol::ChatCompletions => transform_chat_response(
                    &body,
                    upstream_content_type.as_deref(),
                    prepared.streaming,
                )
                .map_err(|error| {
                    RouterError::new(502, format!("Chat Completions 响应转换失败: {error}"))
                })?,
                WireProtocol::AnthropicMessages => transform_anthropic_response(
                    &body,
                    upstream_content_type.as_deref(),
                    prepared.streaming,
                )
                .map_err(|error| {
                    RouterError::new(502, format!("Anthropic Messages 响应转换失败: {error}"))
                })?,
                WireProtocol::Responses => {
                    unreachable!("Responses responses are passed through")
                }
            };
            if let Ok(header) = Header::from_bytes(
                b"content-type".as_slice(),
                transformed.content_type.as_bytes(),
            ) {
                headers.push(header);
            }
            Box::new(Cursor::new(transformed.body))
        }
    } else {
        if let Some(content_type) = upstream_content_type {
            if let Ok(header) =
                Header::from_bytes(b"content-type".as_slice(), content_type.as_bytes())
            {
                headers.push(header);
            }
        }
        Box::new(upstream)
    };
    if let Ok(header) = Header::from_bytes(
        b"x-codex-switcher-profile-id".as_slice(),
        selected.profile.id.as_bytes(),
    ) {
        headers.push(header);
    }
    if let Ok(header) = Header::from_bytes(
        b"x-codex-switcher-model".as_slice(),
        prepared.actual_model.clone().unwrap_or_default().as_bytes(),
    ) {
        headers.push(header);
    }
    if let Ok(header) = Header::from_bytes(
        b"x-codex-switcher-fallback".as_slice(),
        selected.fallback.clone().unwrap_or_default().as_bytes(),
    ) {
        headers.push(header);
    }
    let inspect_response_stream = status < 400 && prepared.streaming;
    let reader = RouteResponseReader {
        inner: body_reader,
        app,
        request_id: prepared.request_id,
        profile_id: selected.profile.id.clone(),
        alias: selected.profile.alias.clone(),
        requested_model: prepared.requested_model,
        actual_model: prepared.actual_model,
        wire_protocol: prepared.protocol.canonical().to_string(),
        upstream_url: prepared.upstream_url,
        fallback: selected.fallback,
        session_hash,
        started,
        status,
        upstream_error,
        stream_inspector: inspect_response_stream.then(ResponseStreamInspector::default),
        finished: false,
    };
    Ok(Response::new(
        StatusCode(status),
        headers,
        Box::new(reader) as Box<dyn Read + Send>,
        None,
        None,
    ))
}

struct RouteResponseReader {
    inner: Box<dyn Read + Send>,
    app: AppHandle,
    request_id: String,
    profile_id: String,
    alias: String,
    requested_model: Option<String>,
    actual_model: Option<String>,
    wire_protocol: String,
    upstream_url: String,
    fallback: Option<String>,
    session_hash: Option<String>,
    started: Instant,
    status: u16,
    upstream_error: Option<String>,
    stream_inspector: Option<ResponseStreamInspector>,
    finished: bool,
}

impl Read for RouteResponseReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = match self.inner.read(buf) {
            Ok(read) => read,
            Err(error) => {
                self.finish(Some(error.to_string()));
                return Err(error);
            }
        };
        if read > 0 {
            if let Some(inspector) = &mut self.stream_inspector {
                inspector.observe(&buf[..read]);
            }
        }
        if read == 0 {
            if let Some(inspector) = &mut self.stream_inspector {
                inspector.finish();
            }
            self.finish(None);
        }
        Ok(read)
    }
}

impl Drop for RouteResponseReader {
    fn drop(&mut self) {
        self.finish(None);
    }
}

impl RouteResponseReader {
    fn finish(&mut self, error: Option<String>) {
        if self.finished {
            return;
        }
        self.finished = true;
        let error = error
            .or_else(|| {
                self.stream_inspector
                    .as_ref()
                    .and_then(|inspector| inspector.error.clone())
            })
            .or_else(|| self.upstream_error.clone());
        let status = if self.status >= 400 {
            "http_error"
        } else if error.is_some() {
            "stream_error"
        } else if self.fallback.is_some() {
            "fallback_ok"
        } else {
            "ok"
        };
        mark_route_finished(&self.app, &self.profile_id, self.status, error.clone());
        append_log(
            &self.app,
            RoutingLogEntry {
                ts: now_string(),
                request_id: Some(self.request_id.clone()),
                method: Some("POST".to_string()),
                path: Some("/v1/responses".to_string()),
                wire_protocol: Some(self.wire_protocol.clone()),
                upstream_url: Some(self.upstream_url.clone()),
                session_hash: self.session_hash.clone(),
                profile_id: Some(self.profile_id.clone()),
                alias: Some(self.alias.clone()),
                requested_model: self.requested_model.clone(),
                actual_model: self.actual_model.clone(),
                status: status.to_string(),
                http_status: Some(self.status),
                latency_ms: self.started.elapsed().as_millis(),
                fallback: self.fallback.clone(),
                error,
            },
        );
    }
}

fn summarize_upstream_error(body: &[u8]) -> Option<String> {
    let message = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .or_else(|| value.get("detail"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .map(ToString::to_string)
        })?;
    Some(message.chars().take(MAX_ERROR_MESSAGE_CHARS).collect())
}

#[derive(Debug, Default)]
struct ResponseStreamInspector {
    pending: Vec<u8>,
    event: Option<String>,
    error: Option<String>,
    completed: bool,
}

impl ResponseStreamInspector {
    fn observe(&mut self, chunk: &[u8]) {
        self.pending.extend_from_slice(chunk);
        while let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=index).collect::<Vec<_>>();
            self.observe_line(&line[..line.len().saturating_sub(1)]);
        }
        if self.pending.len() > MAX_SSE_LINE_BYTES {
            self.pending.clear();
            self.event = None;
        }
    }

    fn finish(&mut self) {
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.observe_line(&line);
        }
    }

    fn observe_line(&mut self, line: &[u8]) {
        let line = std::str::from_utf8(line)
            .unwrap_or_default()
            .trim_end_matches('\r');
        if line.is_empty() {
            self.event = None;
            return;
        }
        if let Some(event) = line.strip_prefix("event:") {
            let event = event.trim();
            self.completed |= event == "response.completed";
            self.event = Some(event.to_string());
            return;
        }
        let data = if let Some(data) = line.strip_prefix("data:") {
            data.trim()
        } else if line.trim_start().starts_with('{') {
            line.trim()
        } else {
            return;
        };
        if data == "[DONE]" {
            self.completed = true;
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            if self.event.as_deref().is_some_and(is_error_stream_event) {
                self.error = Some("上游响应流返回错误事件".to_string());
            }
            return;
        };
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .or(self.event.as_deref());
        self.completed |= event_type == Some("response.completed");
        let has_error = value.get("error").is_some_and(|error| !error.is_null())
            || value
                .pointer("/response/error")
                .is_some_and(|error| !error.is_null())
            || event_type.is_some_and(is_error_stream_event);
        if has_error {
            self.error = response_stream_error_message(&value)
                .or_else(|| Some("上游响应流返回错误事件".to_string()));
        }
    }
}

fn is_error_stream_event(event: &str) -> bool {
    matches!(event, "error" | "response.failed" | "response.incomplete")
}

fn response_stream_error_message(value: &Value) -> Option<String> {
    value
        .pointer("/error/message")
        .or_else(|| value.pointer("/response/error/message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(|message| message.chars().take(MAX_ERROR_MESSAGE_CHARS).collect())
}

fn test_request_model(store: &AppStore) -> String {
    if store.settings.routing.mode != RoutingMode::Fixed {
        return "gpt-5.4".to_string();
    }
    store
        .settings
        .routing
        .fixed_profile_id
        .as_deref()
        .and_then(|profile_id| {
            store
                .profiles
                .iter()
                .find(|profile| profile.id == profile_id)
        })
        .and_then(|profile| profile.api_config.as_ref())
        .map(|config| config.model.clone())
        .unwrap_or_else(|| "gpt-5.4".to_string())
}

fn normalize_oauth_input(body: &mut Value) {
    let Some(input) = body
        .get("input")
        .and_then(Value::as_str)
        .map(ToString::to_string)
    else {
        return;
    };
    body["input"] = serde_json::json!([{
        "type": "message",
        "role": "user",
        "content": [{
            "type": "input_text",
            "text": input
        }]
    }]);
}

fn select_profile(
    app: &AppHandle,
    routing_key: Option<&str>,
    attempted: &HashSet<String>,
    fallback: Option<String>,
) -> Result<SelectedProfile, String> {
    let mut store = load_store(app)?;
    let key = load_master_key(app)?;
    refresh_due_profiles(app, &mut store, &key)?;
    let settings = store.settings.routing.clone();
    if let Some(routing_key) = routing_key {
        if let Some(binding) = sticky_binding(routing_key) {
            if !attempted.contains(&binding.profile_id) {
                if let Some(profile) = store
                    .profiles
                    .iter()
                    .find(|profile| profile.id == binding.profile_id)
                    .cloned()
                {
                    if profile_available(&profile) {
                        return selected_with_auth(profile, &key, fallback);
                    }
                }
            }
        }
    }

    if settings.mode == RoutingMode::Fixed {
        if let Some(fixed_id) = &settings.fixed_profile_id {
            if !attempted.contains(fixed_id) {
                if let Some(profile) = store.profiles.iter().find(|p| p.id == *fixed_id).cloned() {
                    if profile_available(&profile) {
                        return selected_with_auth(profile, &key, fallback);
                    }
                }
            }
        }
    }

    let mut candidates = store
        .profiles
        .iter()
        .filter(|profile| !attempted.contains(&profile.id) && profile_available(profile))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(compare_profiles);
    let profile = candidates
        .into_iter()
        .next()
        .ok_or_else(|| "没有可用的路由账号".to_string())?;
    selected_with_auth(profile, &key, fallback)
}

fn selected_with_auth(
    profile: AccountProfile,
    key: &[u8; 32],
    fallback: Option<String>,
) -> Result<SelectedProfile, String> {
    let auth_json = String::from_utf8(decrypt_secret(&profile.encrypted_auth_json, key)?)
        .map_err(display_err)?;
    Ok(SelectedProfile {
        profile,
        auth_json,
        fallback,
    })
}

fn refresh_due_profiles(
    app: &AppHandle,
    store: &mut AppStore,
    key: &[u8; 32],
) -> Result<(), String> {
    let client = build_probe_client(&store.settings.probe_proxy)?;
    let runtime = tokio::runtime::Runtime::new().map_err(display_err)?;
    let mut changed = false;
    for profile in &mut store.profiles {
        if profile.api_config.is_some() {
            continue;
        }
        if profile.usage.last_token_refresh_status.as_deref() == Some("relogin_required") {
            continue;
        }
        if !should_refresh_access_token(profile.summary.access_token_exp, 0) {
            continue;
        }
        let auth_json = match String::from_utf8(decrypt_secret(&profile.encrypted_auth_json, key)?)
        {
            Ok(value) => value,
            Err(error) => {
                profile.usage.last_token_refresh_error = Some(error.to_string());
                continue;
            }
        };
        match runtime.block_on(refresh_auth_json_with_client(&client, &auth_json)) {
            Ok(updated) => {
                profile.summary = summarize_auth(&updated)?;
                profile.encrypted_auth_json = encrypt_secret(updated.as_bytes(), key)?;
                profile.usage.last_token_refresh_at = Some(now_string());
                profile.usage.last_token_refresh_status = Some("ok".to_string());
                profile.usage.last_token_refresh_error = None;
                changed = true;
            }
            Err(error) => {
                profile.usage.last_token_refresh_at = Some(now_string());
                profile.usage.last_token_refresh_status = Some(
                    if refresh_error_requires_relogin(&error) {
                        "relogin_required"
                    } else {
                        "error"
                    }
                    .to_string(),
                );
                profile.usage.last_token_refresh_error = Some(error);
                changed = true;
            }
        }
    }
    if changed {
        save_store(app, store)?;
    }
    Ok(())
}

fn profile_available(profile: &AccountProfile) -> bool {
    if !profile.enabled {
        return false;
    }
    if profile
        .cooldown_until
        .as_deref()
        .and_then(parse_time)
        .is_some_and(|time| time > Utc::now())
    {
        return false;
    }
    if profile
        .summary
        .subscription_active_until
        .as_deref()
        .and_then(parse_time)
        .is_some_and(|time| time <= Utc::now())
    {
        return false;
    }
    if profile.usage.last_token_refresh_status.as_deref() == Some("relogin_required") {
        return false;
    }
    if profile.route_health.cooldown_reason.as_deref() == Some("temporary_network")
        && profile
            .cooldown_until
            .as_deref()
            .and_then(parse_time)
            .is_some_and(|time| time > Utc::now())
    {
        return false;
    }
    quota_remaining(profile) > 0
}

fn compare_profiles(left: &AccountProfile, right: &AccountProfile) -> Ordering {
    expiry_rank(left)
        .cmp(&expiry_rank(right))
        .then_with(|| right.priority.cmp(&left.priority))
        .then_with(|| quota_remaining(right).cmp(&quota_remaining(left)))
        .then_with(|| {
            left.route_health
                .active_connections
                .cmp(&right.route_health.active_connections)
        })
        .then_with(|| {
            let left_used = left.usage.last_used_at.as_deref().and_then(parse_time);
            let right_used = right.usage.last_used_at.as_deref().and_then(parse_time);
            left_used.cmp(&right_used)
        })
}

fn expiry_rank(profile: &AccountProfile) -> i64 {
    profile
        .summary
        .subscription_active_until
        .as_deref()
        .and_then(parse_time)
        .map(|time| time.timestamp())
        .unwrap_or(i64::MAX)
}

fn quota_remaining(profile: &AccountProfile) -> i64 {
    let hourly = profile
        .quota_rule
        .hourly_limit
        .map(|limit| limit.saturating_sub(profile.usage.hourly_used) as i64)
        .unwrap_or(10_000);
    let daily = profile
        .quota_rule
        .daily_limit
        .map(|limit| limit.saturating_sub(profile.usage.daily_used) as i64)
        .unwrap_or(10_000);
    hourly.min(daily)
}

fn mark_route_started(app: &AppHandle, profile_id: &str) {
    ACTIVE_CONNECTIONS.fetch_add(1, AtomicOrdering::Relaxed);
    update_profile_health(app, profile_id, |health, profile| {
        health.active_connections = health.active_connections.saturating_add(1);
        health.last_route_at = Some(now_string());
        profile.usage.last_used_at = Some(now_string());
        profile.updated_at = now_string();
    });
}

fn mark_route_finished(app: &AppHandle, profile_id: &str, status: u16, error: Option<String>) {
    ACTIVE_CONNECTIONS.fetch_sub(1, AtomicOrdering::Relaxed);
    update_profile_health(app, profile_id, |health, profile| {
        health.active_connections = health.active_connections.saturating_sub(1);
        let succeeded = status < 400 && error.is_none();
        health.last_status = Some(if status < 400 && !succeeded {
            "stream_error".to_string()
        } else {
            status.to_string()
        });
        health.last_error = error;
        if succeeded {
            health.consecutive_failures = 0;
            health.cooldown_reason = None;
        } else {
            health.consecutive_failures = health.consecutive_failures.saturating_add(1);
        }
        if status == 429 {
            let cooldown =
                Utc::now() + chrono::Duration::minutes(profile.quota_rule.cooldown_minutes as i64);
            profile.cooldown_until = Some(cooldown.to_rfc3339());
            health.cooldown_reason = Some("rate_limited".to_string());
        }
        profile.updated_at = now_string();
    });
}

fn mark_route_failure(app: &AppHandle, profile_id: &str, status: Option<u16>, error: &str) {
    update_profile_health(app, profile_id, |health, profile| {
        health.consecutive_failures = health.consecutive_failures.saturating_add(1);
        health.last_route_at = Some(now_string());
        health.last_status = status.map(|value| value.to_string());
        health.last_error = Some(error.to_string());
        if status == Some(429) {
            let cooldown =
                Utc::now() + chrono::Duration::minutes(profile.quota_rule.cooldown_minutes as i64);
            profile.cooldown_until = Some(cooldown.to_rfc3339());
            health.cooldown_reason = Some("rate_limited".to_string());
        } else if status.is_none() && health.consecutive_failures >= 3 {
            let cooldown = Utc::now() + chrono::Duration::seconds(TEMP_NETWORK_COOLDOWN_SECS);
            profile.cooldown_until = Some(cooldown.to_rfc3339());
            health.cooldown_reason = Some("temporary_network".to_string());
        }
        profile.updated_at = now_string();
    });
}

fn update_profile_health(
    app: &AppHandle,
    profile_id: &str,
    update: impl FnOnce(&mut RouteHealth, &mut AccountProfile),
) {
    let _ = mutate_store(app, |store| {
        if let Some(profile) = store
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
        {
            let mut health = std::mem::take(&mut profile.route_health);
            update(&mut health, profile);
            profile.route_health = health;
        }
        Ok(())
    });
}

fn request_headers(request: &Request) -> HashMap<String, String> {
    request
        .headers()
        .iter()
        .filter_map(|header| {
            Some((
                header.field.as_str().to_ascii_lowercase().to_string(),
                header.value.as_str().to_string(),
            ))
        })
        .collect()
}

fn header_value(headers: &HashMap<String, String>, name: &str) -> Option<String> {
    headers.get(&name.to_ascii_lowercase()).cloned()
}

fn routing_key(headers: &HashMap<String, String>, decoded_body: &[u8]) -> Option<String> {
    [
        "thread-id",
        "x-thread-id",
        "x-codex-thread-id",
        "session-id",
        "x-session-id",
        "x-codex-session-id",
        "conversation-id",
        "x-conversation-id",
    ]
    .iter()
    .find_map(|name| header_value(headers, name))
    .filter(|value| !value.trim().is_empty())
    .or_else(|| routing_key_from_body(decoded_body))
    .or_else(|| Some(DEFAULT_ROUTING_KEY.to_string()))
}

fn routing_key_from_body(decoded_body: &[u8]) -> Option<String> {
    let body = serde_json::from_slice::<Value>(decoded_body).ok()?;
    [
        "/thread_id",
        "/threadId",
        "/session_id",
        "/sessionId",
        "/conversation_id",
        "/conversationId",
        "/metadata/thread_id",
        "/metadata/threadId",
        "/metadata/session_id",
        "/metadata/sessionId",
        "/metadata/conversation_id",
        "/metadata/conversationId",
    ]
    .iter()
    .find_map(|path| body.pointer(path).and_then(Value::as_str))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(ToString::to_string)
}

fn read_body(request: &mut Request) -> Result<Vec<u8>, RouterError> {
    let mut body = Vec::new();
    request
        .as_reader()
        .take(MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|error| RouterError::new(400, error.to_string()))?;
    if body.len() > MAX_REQUEST_BYTES {
        return Err(RouterError::new(413, "request body is too large"));
    }
    Ok(body)
}

fn decode_body(body: &[u8], content_encoding: Option<&str>) -> Result<Vec<u8>, RouterError> {
    if content_encoding
        .unwrap_or_default()
        .split(',')
        .any(|part| part.trim().eq_ignore_ascii_case("zstd"))
    {
        zstd::stream::decode_all(Cursor::new(body))
            .map_err(|error| RouterError::new(400, format!("invalid zstd body: {error}")))
    } else {
        Ok(body.to_vec())
    }
}

fn preserve_client_header(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization"
            | "accept"
            | "content-length"
            | "content-type"
            | "content-encoding"
            | "connection"
            | "cookie"
            | "host"
            | "transfer-encoding"
            | "accept-encoding"
    )
}

fn is_retriable_status(status: u16) -> bool {
    matches!(status, 401 | 403 | 429 | 500..=599)
}

fn blocking_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .expect("reqwest blocking client")
}

fn bind_sticky(key: Option<&str>, profile_id: &str, ttl_secs: u64) {
    let Some(key) = key else {
        return;
    };
    let expires_at = Utc::now() + chrono::Duration::seconds(ttl_secs as i64);
    let sticky = STICKY.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut map) = sticky.lock() {
        map.insert(
            key.to_string(),
            StickyBinding {
                profile_id: profile_id.to_string(),
                expires_at,
            },
        );
    }
}

fn sticky_binding(key: &str) -> Option<StickyBinding> {
    let sticky = STICKY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = sticky.lock().ok()?;
    let binding = map.get(key).cloned()?;
    if binding.expires_at <= Utc::now() {
        map.remove(key);
        return None;
    }
    Some(binding)
}

fn decrypt_access_key(settings: &RoutingSettings, key: &[u8; 32]) -> Result<String, String> {
    let envelope = settings
        .encrypted_access_key
        .as_ref()
        .ok_or_else(|| "routing API key is missing".to_string())?;
    String::from_utf8(decrypt_secret(envelope, key)?).map_err(display_err)
}

fn ensure_access_key(app: &AppHandle, store: &mut AppStore) -> Result<(), String> {
    if store.settings.routing.encrypted_access_key.is_some() {
        return Ok(());
    }
    let key = load_master_key(app)?;
    let access_key = generate_access_key();
    store.settings.routing.encrypted_access_key =
        Some(encrypt_secret(access_key.as_bytes(), &key)?);
    Ok(())
}

fn generate_access_key() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn validate_listen(host: &str, port: u16) -> Result<(), String> {
    if host.trim().is_empty() {
        return Err("监听地址不能为空".to_string());
    }
    if port == 0 {
        return Err("监听端口无效".to_string());
    }
    Ok(())
}

fn display_host(host: &str) -> String {
    if host == "0.0.0.0" {
        "127.0.0.1".to_string()
    } else {
        host.to_string()
    }
}

fn respond_json(
    request: Request,
    status: u16,
    body: &Value,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bytes = serde_json::to_vec(body)?;
    let response = Response::from_data(bytes).with_status_code(StatusCode(status));
    request.respond(response)?;
    Ok(())
}

fn respond_error(
    app: AppHandle,
    request: Request,
    started: Instant,
    error: RouterError,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let headers = request_headers(&request);
    append_log(
        &app,
        RoutingLogEntry {
            ts: now_string(),
            request_id: header_value(&headers, "x-client-request-id"),
            method: Some(request.method().as_str().to_string()),
            path: Some(request.url().to_string()),
            wire_protocol: None,
            upstream_url: None,
            session_hash: None,
            profile_id: None,
            alias: None,
            requested_model: None,
            actual_model: None,
            status: "error".to_string(),
            http_status: Some(error.status),
            latency_ms: started.elapsed().as_millis(),
            fallback: None,
            error: Some(error.message.clone()),
        },
    );
    let response = Response::from_string(
        serde_json::json!({ "error": { "message": error.message } }).to_string(),
    )
    .with_status_code(StatusCode(error.status));
    request.respond(response)?;
    Ok(())
}

fn append_log(app: &AppHandle, entry: RoutingLogEntry) {
    let Ok(path) = app_data_dir(app).map(|dir| dir.join(ROUTER_LOG_FILE)) else {
        return;
    };
    let retention_days = load_store(app)
        .map(|store| store.settings.routing.log_retention_days)
        .unwrap_or_else(|_| default_routing_log_retention_days());
    let log_lock = LOG_FILE_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = log_lock.lock().unwrap_or_else(|error| error.into_inner());
    maybe_prune_logs_locked(&path, retention_days);
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > 1_048_576 {
            let rotated_path = path.with_extension("jsonl.1");
            let _ = fs::remove_file(&rotated_path);
            let _ = fs::rename(&path, rotated_path);
        }
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        if let Ok(line) = serde_json::to_string(&entry) {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn read_recent_logs(app: &AppHandle, limit: usize, retention_days: u32) -> Vec<RoutingLogEntry> {
    let Ok(path) = app_data_dir(app).map(|dir| dir.join(ROUTER_LOG_FILE)) else {
        return Vec::new();
    };
    let log_lock = LOG_FILE_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = log_lock.lock().unwrap_or_else(|error| error.into_inner());
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let cutoff = log_retention_cutoff(retention_days, Utc::now());
    let mut rows = text
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<RoutingLogEntry>(line).ok())
        .filter(|entry| log_entry_is_recent(entry, cutoff))
        .take(limit)
        .collect::<Vec<_>>();
    rows.reverse();
    rows
}

fn prune_logs(app: &AppHandle, retention_days: u32) -> Result<(), String> {
    let path = app_data_dir(app)?.join(ROUTER_LOG_FILE);
    let log_lock = LOG_FILE_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = log_lock.lock().unwrap_or_else(|error| error.into_inner());
    prune_log_path(&path, retention_days, Utc::now())?;
    prune_log_path(&path.with_extension("jsonl.1"), retention_days, Utc::now())?;
    LAST_LOG_PRUNE_AT.store(Utc::now().timestamp(), AtomicOrdering::Relaxed);
    Ok(())
}

fn maybe_prune_logs_locked(path: &std::path::Path, retention_days: u32) {
    let now = Utc::now();
    let last_prune_at = LAST_LOG_PRUNE_AT.load(AtomicOrdering::Relaxed);
    if now.timestamp().saturating_sub(last_prune_at) < 3_600 {
        return;
    }
    if LAST_LOG_PRUNE_AT
        .compare_exchange(
            last_prune_at,
            now.timestamp(),
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
        )
        .is_err()
    {
        return;
    }
    let _ = prune_log_path(path, retention_days, now);
    let _ = prune_log_path(&path.with_extension("jsonl.1"), retention_days, now);
}

fn prune_log_path(
    path: &std::path::Path,
    retention_days: u32,
    now: DateTime<Utc>,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let text = fs::read_to_string(path).map_err(display_err)?;
    let cutoff = log_retention_cutoff(retention_days, now);
    let retained = text
        .lines()
        .filter_map(|line| {
            let entry = serde_json::from_str::<RoutingLogEntry>(line).ok()?;
            log_entry_is_recent(&entry, cutoff).then_some(line)
        })
        .collect::<Vec<_>>();
    let output = if retained.is_empty() {
        String::new()
    } else {
        format!("{}\n", retained.join("\n"))
    };
    fs::write(path, output).map_err(display_err)
}

fn log_retention_cutoff(retention_days: u32, now: DateTime<Utc>) -> DateTime<Utc> {
    now - chrono::Duration::days(retention_days.clamp(1, 365) as i64)
}

fn log_entry_is_recent(entry: &RoutingLogEntry, cutoff: DateTime<Utc>) -> bool {
    parse_time(&entry.ts).is_some_and(|timestamp| timestamp >= cutoff)
}

fn hash_text(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    URL_SAFE_NO_PAD.encode(&hasher.finalize()[..12])
}

pub(crate) fn is_running() -> bool {
    ROUTER
        .get()
        .and_then(|slot| slot.try_lock().ok())
        .and_then(|guard| guard.as_ref().map(|_| ()))
        .is_some()
}

fn drop_existing(handle: &mut Option<RouterHandle>) {
    if let Some(mut existing) = handle.take() {
        existing.stop.store(true, AtomicOrdering::Relaxed);
        let addr = format!("{}:{}", display_host(&existing.host), existing.port);
        if let Ok(mut addrs) = addr.to_socket_addrs() {
            if let Some(addr) = addrs.next() {
                let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(150));
            }
        }
        if let Some(thread) = existing.thread.take() {
            let _ = thread::Builder::new()
                .name("routing-stop-join".to_string())
                .spawn(move || {
                    let _ = thread.join();
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn profile(id: &str, priority: i32, expiry: Option<&str>, hourly_used: u32) -> AccountProfile {
        AccountProfile {
            id: id.to_string(),
            alias: id.to_string(),
            note: String::new(),
            enabled: true,
            priority,
            cooldown_until: None,
            quota_rule: crate::QuotaRule {
                hourly_limit: Some(100),
                daily_limit: None,
                cooldown_minutes: 180,
            },
            summary: crate::AuthSummary {
                subscription_active_until: expiry.map(ToString::to_string),
                ..Default::default()
            },
            encrypted_auth_json: crate::SecretEnvelope {
                v: 1,
                alg: "aes-256-gcm".to_string(),
                nonce: String::new(),
                ciphertext: String::new(),
            },
            api_config: None,
            usage: crate::UsageStats {
                hourly_used,
                ..Default::default()
            },
            route_health: crate::RouteHealth::default(),
            created_at: now_string(),
            updated_at: now_string(),
        }
    }

    #[test]
    fn refreshes_takeover_backup_after_external_config_change() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let backup_path = dir.path().join(ROUTER_BACKUP_FILE);
        fs::write(&config_path, "model_provider = \"external\"\n").unwrap();
        fs::write(&backup_path, "model_provider = \"stale\"\n").unwrap();
        let document = fs::read_to_string(&config_path)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();

        refresh_router_backup(&config_path, &backup_path, &document).unwrap();

        assert_eq!(
            fs::read_to_string(&backup_path).unwrap(),
            "model_provider = \"external\"\n"
        );
    }

    #[test]
    fn marked_current_provider_counts_as_router_takeover() {
        let document = r#"
model_provider = "openai-custom"

[model_providers.openai-custom]
base_url = "http://127.0.0.1:15722/v1"
experimental_bearer_token = "local-token"
codex_switcher_router = true
"#
        .parse::<toml_edit::DocumentMut>()
        .unwrap();

        assert!(config_selects_router(&document));
        assert_eq!(router_token_from_config(&document), Some("local-token"));
    }

    #[test]
    fn refresh_router_backup_preserves_original_while_marked_takeover_is_active() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let backup_path = dir.path().join(ROUTER_BACKUP_FILE);
        fs::write(&backup_path, "model_provider = \"original\"\n").unwrap();
        let document = r#"
model_provider = "openai"

[model_providers.openai]
codex_switcher_router = true
"#
        .parse::<toml_edit::DocumentMut>()
        .unwrap();

        refresh_router_backup(&config_path, &backup_path, &document).unwrap();

        assert_eq!(
            fs::read_to_string(&backup_path).unwrap(),
            "model_provider = \"original\"\n"
        );
    }

    #[test]
    fn reads_original_provider_id_from_router_backup() {
        let dir = tempdir().unwrap();
        let backup_path = dir.path().join(ROUTER_BACKUP_FILE);
        fs::write(&backup_path, "model_provider = \"openai\"\n").unwrap();

        assert_eq!(
            selected_provider_id_from_file(&backup_path).as_deref(),
            Some("openai")
        );
    }

    #[test]
    fn takeover_provider_id_never_overrides_reserved_builtin() {
        assert_eq!(
            custom_takeover_provider_id(Some("openai"), None),
            ROUTER_PROVIDER_ID
        );
        assert_eq!(
            custom_takeover_provider_id(Some(ROUTER_PROVIDER_ID), Some("openai")),
            ROUTER_PROVIDER_ID
        );
        assert_eq!(
            custom_takeover_provider_id(Some("openai-custom"), None),
            "openai-custom"
        );
    }

    #[test]
    fn takeover_auth_uses_api_key_mode() {
        let access_key = uuid::Uuid::new_v4().to_string();
        let auth_json = String::from_utf8(router_auth_json(&access_key).unwrap()).unwrap();

        assert!(auth_selects_router(&auth_json, &access_key));
        assert!(!auth_selects_router(&auth_json, "different-token"));
    }

    #[test]
    fn preserves_original_auth_backup_while_takeover_is_active() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        let backup_path = dir.path().join(ROUTER_AUTH_BACKUP_FILE);
        let access_key = uuid::Uuid::new_v4().to_string();
        fs::write(&auth_path, router_auth_json(&access_key).unwrap()).unwrap();
        fs::write(&backup_path, b"original-auth").unwrap();

        refresh_router_auth_backup(&auth_path, &backup_path, Some(&access_key)).unwrap();

        assert_eq!(fs::read(&backup_path).unwrap(), b"original-auth");
    }

    #[test]
    fn refreshes_auth_backup_after_external_auth_change() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        let backup_path = dir.path().join(ROUTER_AUTH_BACKUP_FILE);
        let access_key = uuid::Uuid::new_v4().to_string();
        let external_auth = br#"{"auth_mode":"chatgpt","tokens":{}}"#;
        fs::write(&auth_path, external_auth).unwrap();
        fs::write(&backup_path, b"stale-auth").unwrap();

        refresh_router_auth_backup(&auth_path, &backup_path, Some(&access_key)).unwrap();

        assert_eq!(fs::read(&backup_path).unwrap(), external_auth);
    }

    #[test]
    fn expiring_subscription_sorts_before_open_ended_provider() {
        let mut profiles = vec![
            profile("api", 500, None, 0),
            profile("soon", 100, Some("2099-01-01T00:00:00Z"), 0),
        ];
        profiles.sort_by(compare_profiles);
        assert_eq!(profiles[0].id, "soon");
    }

    #[test]
    fn higher_priority_breaks_equal_expiry_ties() {
        let mut profiles = vec![
            profile("low", 10, Some("2099-01-01T00:00:00Z"), 0),
            profile("high", 20, Some("2099-01-01T00:00:00Z"), 0),
        ];
        profiles.sort_by(compare_profiles);
        assert_eq!(profiles[0].id, "high");
    }

    #[test]
    fn quota_exhausted_profile_is_unavailable() {
        let profile = profile("used", 100, Some("2099-01-01T00:00:00Z"), 100);
        assert!(!profile_available(&profile));
    }

    #[test]
    fn fixed_oauth_probe_does_not_borrow_api_provider_model() {
        let oauth = profile("oauth", 100, None, 0);
        let mut api = profile("api", 90, None, 0);
        api.api_config = Some(crate::ApiProviderConfig {
            provider_id: "longcat".to_string(),
            base_url: "https://example.com".to_string(),
            model: "LongCat-2.0".to_string(),
            wire_api: "responses".to_string(),
        });
        let mut store = AppStore {
            profiles: vec![oauth, api],
            ..Default::default()
        };
        store.settings.routing.mode = RoutingMode::Fixed;
        store.settings.routing.fixed_profile_id = Some("oauth".to_string());

        assert_eq!(test_request_model(&store), "gpt-5.4");
    }

    #[test]
    fn fixed_mode_requires_an_existing_profile() {
        let store = AppStore {
            profiles: vec![profile("selected", 100, None, 0)],
            ..Default::default()
        };

        assert!(validate_fixed_profile(&store, RoutingMode::Fixed, None).is_err());
        assert!(validate_fixed_profile(&store, RoutingMode::Fixed, Some("missing")).is_err());
        assert!(validate_fixed_profile(&store, RoutingMode::Fixed, Some("selected")).is_ok());
        assert!(validate_fixed_profile(&store, RoutingMode::Auto, None).is_ok());
    }

    #[test]
    fn upstream_error_summary_uses_message_without_full_body() {
        let body = br#"{"error":{"message":"unsupported model","internal":"secret"}}"#;
        assert_eq!(
            summarize_upstream_error(body).as_deref(),
            Some("unsupported model")
        );
        assert_eq!(summarize_upstream_error(b"plain response body"), None);
    }

    #[test]
    fn proxy_owns_upstream_content_negotiation_headers() {
        assert!(!preserve_client_header("content-type"));
        assert!(!preserve_client_header("accept"));
        assert!(preserve_client_header("x-client-request-id"));
    }

    #[test]
    fn routing_key_ignores_per_request_id() {
        let mut headers = HashMap::new();
        headers.insert("x-client-request-id".to_string(), "request-one".to_string());
        let first = routing_key(&headers, br#"{"input":"hello"}"#);
        headers.insert("x-client-request-id".to_string(), "request-two".to_string());
        let second = routing_key(&headers, br#"{"input":"hello"}"#);

        assert_eq!(first.as_deref(), Some(DEFAULT_ROUTING_KEY));
        assert_eq!(second.as_deref(), Some(DEFAULT_ROUTING_KEY));
    }

    #[test]
    fn routing_key_prefers_explicit_session_sources() {
        let mut headers = HashMap::new();
        headers.insert("session-id".to_string(), "header-session".to_string());
        let body = br#"{"metadata":{"thread_id":"body-thread"}}"#;
        assert_eq!(
            routing_key(&headers, body).as_deref(),
            Some("header-session")
        );

        headers.clear();
        assert_eq!(routing_key(&headers, body).as_deref(), Some("body-thread"));
    }

    #[test]
    fn oauth_upstream_normalizes_string_input_to_message_list() {
        let mut body = serde_json::json!({"input": "Reply with OK"});
        normalize_oauth_input(&mut body);

        assert!(body["input"].is_array());
        assert_eq!(body["input"][0]["type"], "message");
        assert_eq!(body["input"][0]["content"][0]["text"], "Reply with OK");
    }

    #[test]
    fn response_stream_inspector_detects_failed_event_across_chunks() {
        let mut inspector = ResponseStreamInspector::default();
        inspector.observe(b"event: response.failed\ndata: {\"type\":\"response.");
        inspector
            .observe(b"failed\",\"response\":{\"error\":{\"message\":\"provider failed\"}}}\n\n");
        inspector.finish();

        assert_eq!(inspector.error.as_deref(), Some("provider failed"));
        assert!(!inspector.completed);
    }

    #[test]
    fn response_stream_inspector_detects_completion() {
        let mut inspector = ResponseStreamInspector::default();
        inspector
            .observe(b"event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n");
        inspector.finish();

        assert!(inspector.completed);
        assert_eq!(inspector.error, None);
    }

    #[test]
    fn response_stream_inspector_detects_json_error_with_http_success() {
        let mut inspector = ResponseStreamInspector::default();
        inspector.observe(b"{\"error\":{\"message\":\"quota unavailable\"}}");
        inspector.finish();

        assert_eq!(inspector.error.as_deref(), Some("quota unavailable"));
    }

    #[test]
    fn pruning_logs_removes_expired_and_invalid_rows() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(ROUTER_LOG_FILE);
        let recent = RoutingLogEntry {
            ts: "2026-07-27T12:00:00Z".to_string(),
            request_id: Some("recent".to_string()),
            method: None,
            path: None,
            wire_protocol: None,
            upstream_url: None,
            session_hash: None,
            profile_id: None,
            alias: None,
            requested_model: None,
            actual_model: None,
            status: "ok".to_string(),
            http_status: Some(200),
            latency_ms: 1,
            fallback: None,
            error: None,
        };
        let expired = RoutingLogEntry {
            ts: "2026-07-10T12:00:00Z".to_string(),
            request_id: Some("expired".to_string()),
            ..recent.clone()
        };
        fs::write(
            &path,
            format!(
                "{}\n{}\nnot-json\n",
                serde_json::to_string(&expired).unwrap(),
                serde_json::to_string(&recent).unwrap()
            ),
        )
        .unwrap();

        prune_log_path(
            &path,
            7,
            DateTime::parse_from_rfc3339("2026-07-28T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();

        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("recent"));
        assert!(!text.contains("expired"));
        assert!(!text.contains("not-json"));
    }
}
