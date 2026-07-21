use crate::{
    app_data_dir, build_probe_client, decrypt_secret, display_err, encrypt_secret, load_master_key,
    load_store, mutate_store, now_string, parse_time, push_event, refresh_auth_json_with_client,
    refresh_error_requires_relogin, replace_file_with_rollback, resolve_codex_home, save_store,
    should_refresh_access_token, summarize_auth, AccountProfile, AppStore, RouteHealth,
    RoutingMode, RoutingSettings, ROUTER_PROVIDER_ID,
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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const ROUTER_BACKUP_FILE: &str = "config.toml.account-switcher-router.backup";
const ROUTER_LOG_FILE: &str = "routing-requests.jsonl";
const MAX_REQUEST_BYTES: usize = 20 * 1024 * 1024;
const TEMP_NETWORK_COOLDOWN_SECS: i64 = 60;

static ROUTER: OnceLock<Mutex<Option<RouterHandle>>> = OnceLock::new();
static STICKY: OnceLock<Mutex<HashMap<String, StickyBinding>>> = OnceLock::new();
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
    selected_provider: Option<String>,
    provider_present: bool,
    base_url_matches: bool,
    token_present: bool,
    service_running: bool,
    health_ok: bool,
    diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingLogEntry {
    ts: String,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveRoutingSettingsInput {
    listen_host: String,
    port: u16,
    enabled: bool,
    risk_confirmed: bool,
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
    requested_model: Option<String>,
    actual_model: Option<String>,
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
    let running = is_running();
    let settings = store.settings.routing.clone();
    let base_url = format!(
        "http://{}:{}/v1",
        display_host(&settings.listen_host),
        settings.port
    );
    Ok(RoutingStatus {
        running,
        base_url,
        access_key: decrypt_access_key(&settings, &key).ok(),
        active_connections: ACTIVE_CONNECTIONS.load(AtomicOrdering::Relaxed),
        settings,
        recent_logs: read_recent_logs(&app, 40),
        codex_check: codex_config_check(&app, &store, running),
    })
}

pub(crate) fn save_settings(
    app: AppHandle,
    input: SaveRoutingSettingsInput,
) -> Result<RoutingStatus, String> {
    validate_listen(&input.listen_host, input.port)?;
    let mut store = load_store(&app)?;
    store.settings.routing.listen_host = input.listen_host.trim().to_string();
    store.settings.routing.port = input.port;
    store.settings.routing.enabled = input.enabled;
    store.settings.routing.risk_confirmed = input.risk_confirmed;
    store.settings.routing.mode = input.mode;
    store.settings.routing.fixed_profile_id = input.fixed_profile_id.filter(|id| !id.is_empty());
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

pub(crate) fn start(app: AppHandle) -> Result<RoutingStatus, String> {
    {
        let mut store = load_store(&app)?;
        if !store.settings.routing.risk_confirmed && has_oauth_profiles(&store) {
            return Err("启用 OAuth 账号路由前需要确认账号风险".to_string());
        }
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
    read_recent_logs(&app, limit.clamp(1, 500))
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
    if !backup_path.exists() {
        if config_path.exists() {
            fs::copy(&config_path, &backup_path).map_err(display_err)?;
        } else {
            fs::write(&backup_path, []).map_err(display_err)?;
        }
    }

    let current = fs::read_to_string(&config_path).unwrap_or_default();
    let mut document = if current.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        current
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| format!("config.toml 解析失败: {error}"))?
    };
    document["model_provider"] = toml_edit::value(ROUTER_PROVIDER_ID);
    if !document.as_table().contains_key("model_providers")
        || !document["model_providers"].is_table()
    {
        document["model_providers"] = toml_edit::table();
    }
    document["model_providers"][ROUTER_PROVIDER_ID] =
        toml_edit::Item::Table(toml_edit::Table::new());
    let provider = document["model_providers"][ROUTER_PROVIDER_ID]
        .as_table_mut()
        .ok_or_else(|| "无法写入 Codex 路由 provider 配置".to_string())?;
    provider["name"] = toml_edit::value("CodexSwitcher Router");
    provider["base_url"] = toml_edit::value(format!(
        "http://{}:{}/v1",
        display_host(&store.settings.routing.listen_host),
        store.settings.routing.port
    ));
    provider["wire_api"] = toml_edit::value("responses");
    provider["experimental_bearer_token"] = toml_edit::value(access_key);
    provider["requires_openai_auth"] = toml_edit::value(false);
    provider["request_max_retries"] = toml_edit::value(0);
    provider["stream_max_retries"] = toml_edit::value(0);
    provider["supports_websockets"] = toml_edit::value(false);
    replace_file_with_rollback(&config_path, document.to_string().as_bytes(), None)?;

    store.settings.routing.applied_to_codex = true;
    store.settings.codex_home = Some(codex_home.to_string_lossy().to_string());
    push_event(&mut store, "info", "已将本机 Codex 配置接管到路由 API");
    save_store(&app, &store)?;
    status(app)
}

fn codex_config_check(app: &AppHandle, store: &AppStore, running: bool) -> RoutingCodexCheck {
    let codex_home = resolve_codex_home(app, store.settings.codex_home.clone())
        .unwrap_or_else(|_| app_data_dir(app).unwrap_or_else(|_| std::path::PathBuf::from(".")));
    let config_path = codex_home.join("config.toml");
    let expected_base_url = format!(
        "http://{}:{}/v1",
        display_host(&store.settings.routing.listen_host),
        store.settings.routing.port
    );
    let mut check = RoutingCodexCheck {
        config_path: config_path.to_string_lossy().to_string(),
        selected_provider: None,
        provider_present: false,
        base_url_matches: false,
        token_present: false,
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
                let Some(provider) = document
                    .get("model_providers")
                    .and_then(|providers| providers.get(ROUTER_PROVIDER_ID))
                else {
                    check
                        .diagnostics
                        .push("未找到 codex-switcher-router provider".to_string());
                    check.health_ok = probe_router_health(&expected_base_url);
                    return check;
                };

                check.provider_present = true;
                let base_url = provider
                    .get("base_url")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default();
                check.base_url_matches = base_url == expected_base_url;
                check.token_present = provider
                    .get("experimental_bearer_token")
                    .and_then(|item| item.as_str())
                    .is_some_and(|token| !token.trim().is_empty());

                if check.selected_provider.as_deref() != Some(ROUTER_PROVIDER_ID) {
                    check
                        .diagnostics
                        .push("当前 model_provider 未指向路由 provider".to_string());
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

    check.health_ok = probe_router_health(&expected_base_url);
    if check.provider_present
        && check.base_url_matches
        && check.token_present
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
    if backup_path.exists() {
        let backup = fs::read(&backup_path).map_err(display_err)?;
        if backup.is_empty() {
            if config_path.exists() {
                fs::remove_file(&config_path).map_err(display_err)?;
            }
        } else {
            replace_file_with_rollback(&config_path, &backup, None)?;
        }
        fs::remove_file(&backup_path).map_err(display_err)?;
    }
    store.settings.routing.applied_to_codex = false;
    push_event(&mut store, "info", "已恢复接管前的 Codex 配置");
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
    let routing_key = routing_key(&headers);
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
        match prepare_and_send(&headers, &decoded, selected) {
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
                return Ok(build_stream_response(
                    app,
                    response,
                    selected,
                    prepared,
                    session_hash,
                    started,
                ));
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
) -> Result<AttemptOutcome, (String, String)> {
    let (upstream_url, auth_token, account_id, body, requested_model, actual_model) =
        prepare_upstream(&selected, decoded_body)
            .map_err(|error| (selected.profile.id.clone(), error))?;
    let mut builder = blocking_client()
        .post(&upstream_url)
        .header("accept", "text/event-stream")
        .header("content-type", "application/json")
        .bearer_auth(&auth_token);
    if let Some(account_id) = &account_id {
        builder = builder.header("ChatGPT-Account-Id", account_id);
    }
    for (name, value) in headers {
        if preserve_client_header(name) {
            builder = builder.header(name, value);
        }
    }
    let response = builder
        .body(body.clone())
        .send()
        .map_err(|error| (selected.profile.id.clone(), error.to_string()))?;
    Ok(AttemptOutcome::Response(
        response,
        selected,
        PreparedRequest {
            requested_model,
            actual_model,
        },
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
) -> Result<
    (
        String,
        String,
        Option<String>,
        Vec<u8>,
        Option<String>,
        Option<String>,
    ),
    String,
> {
    let mut json: Value = serde_json::from_slice(decoded_body).map_err(display_err)?;
    let requested_model = json
        .get("model")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    if let Some(api_config) = &selected.profile.api_config {
        json["model"] = Value::String(api_config.model.clone());
        let auth: Value = serde_json::from_str(&selected.auth_json).map_err(display_err)?;
        let api_key = auth
            .get("OPENAI_API_KEY")
            .and_then(Value::as_str)
            .ok_or_else(|| "API Provider missing OPENAI_API_KEY".to_string())?
            .to_string();
        return Ok((
            format!("{}/responses", api_config.base_url.trim_end_matches('/')),
            api_key,
            None,
            serde_json::to_vec(&json).map_err(display_err)?,
            requested_model,
            Some(api_config.model.clone()),
        ));
    }

    let auth: Value = serde_json::from_str(&selected.auth_json).map_err(display_err)?;
    let access_token = auth
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "auth.json missing access_token".to_string())?
        .to_string();
    Ok((
        "https://chatgpt.com/backend-api/codex/responses".to_string(),
        access_token,
        selected.profile.summary.account_id.clone(),
        serde_json::to_vec(&json).map_err(display_err)?,
        requested_model.clone(),
        requested_model,
    ))
}

fn build_stream_response(
    app: AppHandle,
    upstream: reqwest::blocking::Response,
    selected: SelectedProfile,
    prepared: PreparedRequest,
    session_hash: Option<String>,
    started: Instant,
) -> Response<Box<dyn Read + Send>> {
    let status = upstream.status().as_u16();
    let mut headers = upstream
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            let lower = name.as_str().to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "content-length" | "content-encoding" | "connection" | "transfer-encoding"
            ) {
                return None;
            }
            Header::from_bytes(name.as_str().as_bytes(), value.as_bytes()).ok()
        })
        .collect::<Vec<_>>();
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
    let reader = RouteResponseReader {
        inner: Some(upstream),
        app,
        profile_id: selected.profile.id.clone(),
        alias: selected.profile.alias.clone(),
        requested_model: prepared.requested_model,
        actual_model: prepared.actual_model,
        fallback: selected.fallback,
        session_hash,
        started,
        status,
        finished: false,
    };
    Response::new(
        StatusCode(status),
        headers,
        Box::new(reader) as Box<dyn Read + Send>,
        None,
        None,
    )
}

struct RouteResponseReader {
    inner: Option<reqwest::blocking::Response>,
    app: AppHandle,
    profile_id: String,
    alias: String,
    requested_model: Option<String>,
    actual_model: Option<String>,
    fallback: Option<String>,
    session_hash: Option<String>,
    started: Instant,
    status: u16,
    finished: bool,
}

impl Read for RouteResponseReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(inner) = &mut self.inner else {
            return Ok(0);
        };
        let read = match inner.read(buf) {
            Ok(read) => read,
            Err(error) => {
                self.finish(Some(error.to_string()));
                return Err(error);
            }
        };
        if read == 0 {
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
        mark_route_finished(&self.app, &self.profile_id, self.status, error.clone());
        append_log(
            &self.app,
            RoutingLogEntry {
                ts: now_string(),
                session_hash: self.session_hash.clone(),
                profile_id: Some(self.profile_id.clone()),
                alias: Some(self.alias.clone()),
                requested_model: self.requested_model.clone(),
                actual_model: self.actual_model.clone(),
                status: if self.status < 400 {
                    "ok"
                } else {
                    "http_error"
                }
                .to_string(),
                http_status: Some(self.status),
                latency_ms: self.started.elapsed().as_millis(),
                fallback: self.fallback.clone(),
                error,
            },
        );
    }
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
        health.last_status = Some(status.to_string());
        health.last_error = error;
        if status < 400 {
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

fn routing_key(headers: &HashMap<String, String>) -> Option<String> {
    ["thread-id", "session-id", "x-client-request-id"]
        .iter()
        .find_map(|name| header_value(headers, name))
        .filter(|value| !value.trim().is_empty())
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
            | "content-length"
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

fn has_oauth_profiles(store: &AppStore) -> bool {
    store
        .profiles
        .iter()
        .any(|profile| profile.api_config.is_none())
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
    append_log(
        &app,
        RoutingLogEntry {
            ts: now_string(),
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
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > 1_048_576 {
            let _ = fs::rename(&path, path.with_extension("jsonl.1"));
        }
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        if let Ok(line) = serde_json::to_string(&entry) {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn read_recent_logs(app: &AppHandle, limit: usize) -> Vec<RoutingLogEntry> {
    let Ok(path) = app_data_dir(app).map(|dir| dir.join(ROUTER_LOG_FILE)) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut rows = text
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<RoutingLogEntry>(line).ok())
        .take(limit)
        .collect::<Vec<_>>();
    rows.reverse();
    rows
}

fn hash_text(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    URL_SAFE_NO_PAD.encode(&hasher.finalize()[..12])
}

fn is_running() -> bool {
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
}
