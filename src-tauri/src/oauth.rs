use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{TimeZone, Utc};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tiny_http::{Header, Response, Server};
use url::Url;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
const ORIGINATOR: &str = "codex_vscode";
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
const SESSION_TTL_SECONDS: i64 = 300;
const PENDING_FILE: &str = "oauth-pending.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthLoginStartResponse {
    pub login_id: String,
    pub auth_url: String,
    pub callback_url: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct OAuthTokens {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingOAuth {
    login_id: String,
    auth_url: String,
    callback_url: String,
    code_verifier: String,
    state: String,
    expires_at: i64,
    code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuthEvent {
    login_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuthTimeoutEvent {
    login_id: String,
    callback_url: String,
    timeout_seconds: i64,
}

static PENDING: OnceLock<Mutex<Option<PendingOAuth>>> = OnceLock::new();

fn pending() -> &'static Mutex<Option<PendingOAuth>> {
    PENDING.get_or_init(|| Mutex::new(None))
}

fn now() -> i64 {
    Utc::now().timestamp()
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn pending_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(PENDING_FILE)
}

fn persist_pending(app_data_dir: &Path, value: Option<&PendingOAuth>) -> Result<(), String> {
    fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let path = pending_path(app_data_dir);
    if let Some(value) = value {
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
        fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
        if path.exists() {
            fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
        fs::rename(temporary, path).map_err(|error| error.to_string())?;
    } else if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn load_pending(app_data_dir: &Path) -> Option<PendingOAuth> {
    let path = pending_path(app_data_dir);
    let value = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<PendingOAuth>(&text).ok());
    match value {
        Some(value) if value.expires_at > now() => Some(value),
        _ => {
            let _ = fs::remove_file(path);
            None
        }
    }
}

fn set_pending(app_data_dir: &Path, value: Option<PendingOAuth>) -> Result<(), String> {
    {
        let mut guard = pending()
            .lock()
            .map_err(|_| "OAuth 状态锁已损坏".to_string())?;
        *guard = value.clone();
    }
    persist_pending(app_data_dir, value.as_ref())
}

fn hydrate_pending(app_data_dir: &Path) {
    if let Ok(mut guard) = pending().lock() {
        if guard.is_none() {
            *guard = load_pending(app_data_dir);
        }
    }
}

pub fn build_authorize_url(redirect_uri: &str, challenge: &str, state: &str) -> String {
    format!(
        "{AUTHORIZE_ENDPOINT}?response_type=code&client_id={CLIENT_ID}&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&id_token_add_organizations=true&codex_cli_simplified_flow=true&state={state}&originator={}",
        urlencoding::encode(redirect_uri),
        urlencoding::encode(SCOPES),
        urlencoding::encode(ORIGINATOR)
    )
}

fn response_for(value: &PendingOAuth) -> OAuthLoginStartResponse {
    OAuthLoginStartResponse {
        login_id: value.login_id.clone(),
        auth_url: value.auth_url.clone(),
        callback_url: value.callback_url.clone(),
        expires_at: Utc
            .timestamp_opt(value.expires_at, 0)
            .single()
            .unwrap_or_else(Utc::now)
            .to_rfc3339(),
    }
}

pub fn start(app: AppHandle, app_data_dir: PathBuf) -> Result<OAuthLoginStartResponse, String> {
    hydrate_pending(&app_data_dir);
    if let Some(value) = pending()
        .lock()
        .map_err(|_| "OAuth 状态锁已损坏".to_string())?
        .as_ref()
        .filter(|value| value.expires_at > now())
        .cloned()
    {
        return Ok(response_for(&value));
    }

    let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT)).map_err(|error| {
        if error.kind() == ErrorKind::AddrInUse {
            format!("CODEX_OAUTH_PORT_IN_USE:{CALLBACK_PORT}")
        } else {
            format!("无法监听 OAuth 回调端口 {CALLBACK_PORT}: {error}")
        }
    })?;
    drop(listener);

    let code_verifier = random_token();
    let state = random_token();
    let login_id = random_token();
    let callback_url = format!("http://localhost:{CALLBACK_PORT}{CALLBACK_PATH}");
    let auth_url = build_authorize_url(&callback_url, &pkce_challenge(&code_verifier), &state);
    let value = PendingOAuth {
        login_id,
        auth_url,
        callback_url,
        code_verifier,
        state,
        expires_at: now() + SESSION_TTL_SECONDS,
        code: None,
    };
    set_pending(&app_data_dir, Some(value.clone()))?;
    spawn_callback_server(app, app_data_dir, value.clone());
    Ok(response_for(&value))
}

pub fn restore_listener(app: AppHandle, app_data_dir: PathBuf) {
    hydrate_pending(&app_data_dir);
    let value = pending()
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());
    if let Some(value) = value.filter(|value| value.expires_at > now()) {
        if TcpListener::bind(("127.0.0.1", CALLBACK_PORT)).is_ok() {
            spawn_callback_server(app, app_data_dir, value);
        }
    }
}

fn spawn_callback_server(app: AppHandle, app_data_dir: PathBuf, expected: PendingOAuth) {
    std::thread::spawn(move || {
        let server = match Server::http(("127.0.0.1", CALLBACK_PORT)) {
            Ok(server) => server,
            Err(_) => return,
        };
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(SESSION_TTL_SECONDS as u64) {
            let still_active = pending()
                .lock()
                .ok()
                .and_then(|guard| {
                    guard
                        .as_ref()
                        .map(|value| value.login_id == expected.login_id)
                })
                .unwrap_or(false);
            if !still_active {
                return;
            }
            let request = match server.recv_timeout(Duration::from_millis(200)) {
                Ok(Some(request)) => request,
                Ok(None) => continue,
                Err(_) => continue,
            };
            let request_url = request.url().to_string();
            if request_url.starts_with("/cancel") {
                let _ = request.respond(Response::from_string("Cancelled"));
                return;
            }
            if !request_url.starts_with(CALLBACK_PATH) {
                let _ = request.respond(Response::from_string("Not Found").with_status_code(404));
                continue;
            }
            match accept_callback(&app_data_dir, &expected.login_id, &request_url) {
                Ok(()) => {
                    let html = "<!doctype html><meta charset=\"utf-8\"><title>Codex 授权成功</title><style>body{font-family:system-ui;display:grid;place-items:center;height:100vh;margin:0;background:#f4f7fb;color:#162033}main{text-align:center}</style><main><h1>授权成功</h1><p>可以关闭此页面并返回 CodexSwitcher。</p></main>";
                    let header = Header::from_bytes("Content-Type", "text/html; charset=utf-8")
                        .expect("valid content type");
                    let _ = request.respond(Response::from_string(html).with_header(header));
                    let _ = app.emit(
                        "codex-oauth-login-completed",
                        OAuthEvent {
                            login_id: expected.login_id.clone(),
                        },
                    );
                    return;
                }
                Err(error) => {
                    let _ = request.respond(Response::from_string(error).with_status_code(400));
                }
            }
        }
        let _ = set_pending(&app_data_dir, None);
        let _ = app.emit(
            "codex-oauth-login-timeout",
            OAuthTimeoutEvent {
                login_id: expected.login_id,
                callback_url: expected.callback_url,
                timeout_seconds: SESSION_TTL_SECONDS,
            },
        );
    });
}

fn decode_component(value: &str) -> String {
    urlencoding::decode(value)
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| value.to_string())
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let mut parts = pair.splitn(2, '=');
        let candidate = parts.next()?;
        (candidate == key).then(|| decode_component(parts.next().unwrap_or_default()))
    })
}

pub fn parse_callback(input: &str) -> Result<(String, String), String> {
    let trimmed = input.trim();
    let parsed = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Url::parse(trimmed).map_err(|error| format!("回调地址无效: {error}"))?
    } else if trimmed.starts_with('/') {
        Url::parse(&format!("http://localhost:{CALLBACK_PORT}{trimmed}"))
            .map_err(|error| format!("回调地址无效: {error}"))?
    } else {
        Url::parse(&format!(
            "http://localhost:{CALLBACK_PORT}{CALLBACK_PATH}?{}",
            trimmed.trim_start_matches('?')
        ))
        .map_err(|error| format!("回调地址无效: {error}"))?
    };
    if parsed.path() != CALLBACK_PATH {
        return Err(format!("回调路径必须为 {CALLBACK_PATH}"));
    }
    let query = parsed.query().unwrap_or_default();
    let code = query_value(query, "code").filter(|value| !value.trim().is_empty());
    let state = query_value(query, "state").filter(|value| !value.trim().is_empty());
    match (code, state) {
        (Some(code), Some(state)) => Ok((code, state)),
        _ => Err("回调地址缺少 code 或 state 参数".to_string()),
    }
}

pub fn accept_callback(app_data_dir: &Path, login_id: &str, callback: &str) -> Result<(), String> {
    let (code, callback_state) = parse_callback(callback)?;
    let mut guard = pending()
        .lock()
        .map_err(|_| "OAuth 状态锁已损坏".to_string())?;
    let value = guard
        .as_mut()
        .ok_or_else(|| "OAuth 会话不存在，请重新授权".to_string())?;
    if value.expires_at <= now() {
        return Err("OAuth 会话已超时，请重新授权".to_string());
    }
    if value.login_id != login_id {
        return Err("OAuth loginId 不匹配".to_string());
    }
    if value.state != callback_state {
        return Err("OAuth state 校验失败".to_string());
    }
    value.code = Some(code);
    persist_pending(app_data_dir, Some(value))
}

pub fn current_auth_url(login_id: &str) -> Result<String, String> {
    let guard = pending()
        .lock()
        .map_err(|_| "OAuth 状态锁已损坏".to_string())?;
    let value = guard
        .as_ref()
        .ok_or_else(|| "OAuth 会话不存在，请重新授权".to_string())?;
    if value.login_id != login_id {
        return Err("OAuth loginId 不匹配".to_string());
    }
    Ok(value.auth_url.clone())
}

pub fn cancel(app_data_dir: &Path, login_id: Option<&str>) -> Result<(), String> {
    let port = {
        let guard = pending()
            .lock()
            .map_err(|_| "OAuth 状态锁已损坏".to_string())?;
        if let (Some(expected), Some(value)) = (login_id, guard.as_ref()) {
            if expected != value.login_id {
                return Err("OAuth loginId 不匹配".to_string());
            }
        }
        CALLBACK_PORT
    };
    set_pending(app_data_dir, None)?;
    if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
        let _ = stream
            .write_all(b"GET /cancel HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    }
    Ok(())
}

#[allow(dead_code)]
pub async fn complete(app_data_dir: &Path, login_id: &str) -> Result<OAuthTokens, String> {
    let (code, verifier, callback_url) = {
        let guard = pending()
            .lock()
            .map_err(|_| "OAuth 状态锁已损坏".to_string())?;
        let value = guard
            .as_ref()
            .ok_or_else(|| "OAuth 会话不存在，请重新授权".to_string())?;
        if value.expires_at <= now() {
            return Err("OAuth 会话已超时，请重新授权".to_string());
        }
        if value.login_id != login_id {
            return Err("OAuth loginId 不匹配".to_string());
        }
        (
            value
                .code
                .clone()
                .ok_or_else(|| "浏览器授权尚未完成".to_string())?,
            value.code_verifier.clone(),
            value.callback_url.clone(),
        )
    };
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|error| format!("无法创建 OAuth 客户端: {error}"))?;
    let tokens =
        exchange_code_for_tokens_at(&client, TOKEN_ENDPOINT, &code, &verifier, &callback_url)
            .await?;
    set_pending(app_data_dir, None)?;
    Ok(tokens)
}

pub async fn complete_with_client(
    app_data_dir: &Path,
    login_id: &str,
    client: &reqwest::Client,
) -> Result<OAuthTokens, String> {
    let (code, verifier, callback_url) = {
        let guard = pending()
            .lock()
            .map_err(|_| "OAuth state lock poisoned".to_string())?;
        let value = guard
            .as_ref()
            .ok_or_else(|| "OAuth session does not exist. Start authorization again.".to_string())?;
        if value.expires_at <= now() {
            return Err("OAuth session expired. Start authorization again.".to_string());
        }
        if value.login_id != login_id {
            return Err("OAuth loginId mismatch".to_string());
        }
        (
            value
                .code
                .clone()
                .ok_or_else(|| "Browser authorization has not completed.".to_string())?,
            value.code_verifier.clone(),
            value.callback_url.clone(),
        )
    };
    let tokens =
        exchange_code_for_tokens_at(client, TOKEN_ENDPOINT, &code, &verifier, &callback_url)
            .await?;
    set_pending(app_data_dir, None)?;
    Ok(tokens)
}

pub async fn exchange_code_for_tokens_at(
    client: &reqwest::Client,
    endpoint: &str,
    code: &str,
    verifier: &str,
    callback_url: &str,
) -> Result<OAuthTokens, String> {
    let response = client
        .post(endpoint)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, "codex-account-switcher")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", callback_url),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|error| format!("OAuth Token 请求失败: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 OAuth Token 响应失败: {error}"))?;
    if !status.is_success() {
        let error_code = serde_json::from_str::<Value>(&body).ok().and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| value.get("code").and_then(Value::as_str))
                .map(String::from)
        });
        return Err(match error_code {
            Some(code) => format!(
                "OAuth Token exchange failed: HTTP {status}, {code}: {}",
                compact_oauth_error_body(&body)
            ),
            None => format!(
                "OAuth Token exchange failed: HTTP {status}: {}",
                compact_oauth_error_body(&body)
            ),
        });
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|error| format!("OAuth Token 响应不是有效 JSON: {error}"))?;
    let id_token = value
        .get("id_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "OAuth Token 响应缺少 id_token".to_string())?;
    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "OAuth Token 响应缺少 access_token".to_string())?;
    Ok(OAuthTokens {
        id_token: id_token.to_string(),
        access_token: access_token.to_string(),
        refresh_token: value
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(String::from),
    })
}


fn compact_oauth_error_body(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return "empty response body".to_string();
    }
    compact.chars().take(240).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::tempdir;

    fn mock_token_endpoint(status: u16, body: &'static str) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let reason = if status < 400 { "OK" } else { "Bad Request" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}/oauth/token")
    }

    #[test]
    fn builds_pkce_and_authorize_url() {
        assert_eq!(
            pkce_challenge("verifier"),
            "iMnq5o6zALKXGivsnlom_0F5_WYda32GHkxlV7mq7hQ"
        );
        let url = build_authorize_url("http://localhost:1455/auth/callback", "challenge", "state");
        assert!(url.contains("code_challenge=challenge"));
        assert!(url.contains("state=state"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
    }

    #[test]
    fn parses_full_and_query_only_callbacks() {
        let expected = ("abc".to_string(), "xyz".to_string());
        assert_eq!(
            parse_callback("http://localhost:1455/auth/callback?code=abc&state=xyz").unwrap(),
            expected
        );
        assert_eq!(parse_callback("code=abc&state=xyz").unwrap(), expected);
        assert!(parse_callback("http://localhost:1455/wrong?code=a&state=b").is_err());
        assert!(parse_callback("code=abc").is_err());
    }

    #[tokio::test]
    async fn exchanges_token_response_and_rejects_invalid_responses() {
        let client = reqwest::Client::new();
        let endpoint = mock_token_endpoint(
            200,
            r#"{"id_token":"id","access_token":"access","refresh_token":"refresh"}"#,
        );
        let tokens = exchange_code_for_tokens_at(
            &client,
            &endpoint,
            "code",
            "verifier",
            "http://localhost:1455/auth/callback",
        )
        .await
        .unwrap();
        assert_eq!(tokens.id_token, "id");
        assert_eq!(tokens.refresh_token.as_deref(), Some("refresh"));

        let missing = mock_token_endpoint(200, r#"{"access_token":"access"}"#);
        assert!(exchange_code_for_tokens_at(
            &client,
            &missing,
            "code",
            "verifier",
            "http://localhost:1455/auth/callback",
        )
        .await
        .unwrap_err()
        .contains("id_token"));

        let rejected = mock_token_endpoint(400, r#"{"error":"invalid_grant"}"#);
        assert!(exchange_code_for_tokens_at(
            &client,
            &rejected,
            "code",
            "verifier",
            "http://localhost:1455/auth/callback",
        )
        .await
        .unwrap_err()
        .contains("invalid_grant"));
    }

    #[test]
    fn validates_login_and_state_before_accepting_callback() {
        let dir = tempdir().unwrap();
        let value = PendingOAuth {
            login_id: "login-1".to_string(),
            auth_url: "https://auth.openai.com/oauth/authorize".to_string(),
            callback_url: "http://localhost:1455/auth/callback".to_string(),
            code_verifier: "verifier".to_string(),
            state: "state-1".to_string(),
            expires_at: now() + 60,
            code: None,
        };
        set_pending(dir.path(), Some(value)).unwrap();
        assert!(accept_callback(dir.path(), "other-login", "code=abc&state=state-1").is_err());
        assert!(accept_callback(dir.path(), "login-1", "code=abc&state=wrong").is_err());
        accept_callback(dir.path(), "login-1", "code=abc&state=state-1").unwrap();
        assert_eq!(
            pending()
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|value| value.code.as_deref()),
            Some("abc")
        );
        cancel(dir.path(), Some("login-1")).unwrap();
        assert!(pending().lock().unwrap().is_none());
        assert!(!pending_path(dir.path()).exists());
    }
}
