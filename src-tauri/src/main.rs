#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod codex_sessions;
mod easytier_mesh;
mod oauth;
mod provider_compat;
mod proxy;
mod routing;
mod routing_anthropic;
mod routing_protocol;
mod routing_sse;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use fs2::FileExt;
use provider_compat::{is_longcat_base_url, is_longcat_model};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
#[cfg(windows)]
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use uuid::Uuid;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

const STORE_FILE: &str = "store.json";
const SWITCH_DIAGNOSTICS_FILE: &str = "switch-diagnostics.log";
const LOCAL_KEY_FILE: &str = "local-profile.key";
const EXPORT_FORMAT: &str = "codex-switcher.bundle";
const EXPORT_VERSION: u32 = 1;
const KEYRING_SERVICE: &str = "codex-account-switcher";
const KEYRING_USER: &str = "profiles";
const LEGACY_APP_IDENTIFIER: &str = "cn.cmscloud.codex-account-switcher";
const CHATGPT_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEFAULT_API_BASE_URL: &str = "https://api.openai.com/v1";
const OFFICIAL_CLIENT_DISPLAY_NAME: &str = "Codex/ChatGPT";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const LOOPBACK_NO_PROXY_VALUES: [&str; 5] = ["localhost", "127.0.0.1", "::1", "[::1]", "0.0.0.0"];
const CODEX_PROXY_ENV_KEYS: [&str; 8] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
];
pub(crate) const ROUTER_PROVIDER_ID: &str = "codex-switcher-router";
static STORE_IO_MUTEX: Mutex<()> = Mutex::new(());
pub(crate) use proxy::{build_probe_client, normalize_proxy_url, ProxySettings, CHATGPT_USAGE_URL};

#[cfg(windows)]
fn hidden_command<S: AsRef<OsStr>>(program: S) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn merge_loopback_no_proxy(existing: Option<&str>) -> String {
    let mut values = existing
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
        .collect::<Vec<_>>();

    for required in LOOPBACK_NO_PROXY_VALUES {
        if !values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(required))
        {
            values.push(required.to_string());
        }
    }

    values.join(",")
}

fn codex_launch_proxy_url(proxy: Option<&ProxySettings>) -> Option<String> {
    let proxy = proxy?;
    if !proxy.enabled {
        return None;
    }
    normalize_proxy_url(&proxy.url)
        .ok()
        .filter(|value| !value.is_empty())
}

fn chromium_proxy_url(proxy: Option<&ProxySettings>) -> Option<String> {
    codex_launch_proxy_url(proxy).map(|value| {
        value
            .strip_prefix("socks5h://")
            .map(|rest| format!("socks5://{rest}"))
            .unwrap_or(value)
    })
}

fn chromium_proxy_bypass_list() -> String {
    LOOPBACK_NO_PROXY_VALUES.join(";")
}

#[cfg(windows)]
fn powershell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
fn windows_proxy_server_value(proxy: Option<&ProxySettings>) -> Option<String> {
    let proxy_url = codex_launch_proxy_url(proxy)?;
    let parsed = url::Url::parse(&proxy_url).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port()?;
    let address = format!("{host}:{port}");
    match parsed.scheme().to_ascii_lowercase().as_str() {
        "socks5" | "socks5h" => Some(format!("socks={address}")),
        _ => Some(address),
    }
}

#[cfg(windows)]
fn windows_proxy_override_value() -> String {
    let mut values = LOOPBACK_NO_PROXY_VALUES
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    values.push("<local>".to_string());
    values.join(";")
}

fn apply_codex_desktop_launch_args(command: &mut Command, proxy: Option<&ProxySettings>) {
    if let Some(proxy_url) = chromium_proxy_url(proxy) {
        command
            .arg(format!("--proxy-server={proxy_url}"))
            .arg(format!(
                "--proxy-bypass-list={}",
                chromium_proxy_bypass_list()
            ));
    }
}

#[cfg(windows)]
fn enable_windows_system_proxy_temporarily(
    proxy: Option<&ProxySettings>,
    duration: Duration,
) -> Result<(), String> {
    let Some(proxy_server) = windows_proxy_server_value(proxy) else {
        return Ok(());
    };
    let proxy_override = windows_proxy_override_value();
    let ready_path = std::env::temp_dir().join(format!(
        "codex-switcher-system-proxy-{}.ready",
        Uuid::new_v4()
    ));
    let ready = ready_path.to_string_lossy().to_string();
    let seconds = duration.as_secs().max(5);
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
$notify = @'
using System;
using System.Runtime.InteropServices;
public static class NativeInternetOptions {{
  [DllImport("wininet.dll", SetLastError=true)]
  public static extern bool InternetSetOption(IntPtr hInternet, int dwOption, IntPtr lpBuffer, int dwBufferLength);
}}
'@
if (-not ('NativeInternetOptions' -as [type])) {{ Add-Type $notify }}
$props = Get-ItemProperty -Path $key
$hadEnable = $props.PSObject.Properties.Name -contains 'ProxyEnable'
$hadServer = $props.PSObject.Properties.Name -contains 'ProxyServer'
$hadOverride = $props.PSObject.Properties.Name -contains 'ProxyOverride'
$oldEnable = if ($hadEnable) {{ $props.ProxyEnable }} else {{ $null }}
$oldServer = if ($hadServer) {{ $props.ProxyServer }} else {{ $null }}
$oldOverride = if ($hadOverride) {{ $props.ProxyOverride }} else {{ $null }}
Set-ItemProperty -Path $key -Name ProxyEnable -Type DWord -Value 1
Set-ItemProperty -Path $key -Name ProxyServer -Type String -Value {proxy_server}
Set-ItemProperty -Path $key -Name ProxyOverride -Type String -Value {proxy_override}
[NativeInternetOptions]::InternetSetOption([IntPtr]::Zero, 39, [IntPtr]::Zero, 0) | Out-Null
[NativeInternetOptions]::InternetSetOption([IntPtr]::Zero, 37, [IntPtr]::Zero, 0) | Out-Null
New-Item -ItemType File -Path {ready} -Force | Out-Null
Start-Sleep -Seconds {seconds}
if ($hadEnable) {{ Set-ItemProperty -Path $key -Name ProxyEnable -Type DWord -Value $oldEnable }} else {{ Remove-ItemProperty -Path $key -Name ProxyEnable -ErrorAction SilentlyContinue }}
if ($hadServer) {{ Set-ItemProperty -Path $key -Name ProxyServer -Type String -Value $oldServer }} else {{ Remove-ItemProperty -Path $key -Name ProxyServer -ErrorAction SilentlyContinue }}
if ($hadOverride) {{ Set-ItemProperty -Path $key -Name ProxyOverride -Type String -Value $oldOverride }} else {{ Remove-ItemProperty -Path $key -Name ProxyOverride -ErrorAction SilentlyContinue }}
[NativeInternetOptions]::InternetSetOption([IntPtr]::Zero, 39, [IntPtr]::Zero, 0) | Out-Null
[NativeInternetOptions]::InternetSetOption([IntPtr]::Zero, 37, [IntPtr]::Zero, 0) | Out-Null
Remove-Item -Path {ready} -Force -ErrorAction SilentlyContinue
"#,
        proxy_server = powershell_single_quote(&proxy_server),
        proxy_override = powershell_single_quote(&proxy_override),
        ready = powershell_single_quote(&ready),
        seconds = seconds,
    );
    hidden_command("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .spawn()
        .map_err(|error| format!("无法临时启用系统代理: {error}"))?;

    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        if ready_path.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn env_line_key(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let assignment = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    assignment
        .split_once('=')
        .map(|(key, _)| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

fn env_line_value(line: &str) -> Option<&str> {
    line.trim_start()
        .strip_prefix("export ")
        .unwrap_or_else(|| line.trim_start())
        .split_once('=')
        .map(|(_, value)| value.trim())
}

fn is_codex_proxy_env_key(key: &str) -> bool {
    CODEX_PROXY_ENV_KEYS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(key))
}

fn render_codex_proxy_env(content: &str, proxy: &ProxySettings) -> String {
    let existing_no_proxy = content.lines().find_map(|line| {
        env_line_key(line)
            .filter(|key| {
                key.eq_ignore_ascii_case("NO_PROXY") || key.eq_ignore_ascii_case("no_proxy")
            })
            .and_then(|_| env_line_value(line))
    });
    let no_proxy = merge_loopback_no_proxy(existing_no_proxy);
    let mut lines = content
        .lines()
        .filter(|line| {
            env_line_key(line)
                .map(|key| !is_codex_proxy_env_key(&key))
                .unwrap_or(true)
        })
        .map(String::from)
        .collect::<Vec<_>>();

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines
        .push("# Managed by CodexSwitcher. Used by Codex app-server without TUN mode.".to_string());
    if let Some(proxy_url) = codex_launch_proxy_url(Some(proxy)) {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            lines.push(format!("{key}={proxy_url}"));
        }
    }
    lines.push(format!("NO_PROXY={no_proxy}"));
    lines.push(format!("no_proxy={no_proxy}"));
    format!("{}\n", lines.join("\n"))
}

fn sync_codex_proxy_env_file(codex_home: &Path, proxy: &ProxySettings) -> Result<(), String> {
    fs::create_dir_all(codex_home).map_err(display_err)?;
    let env_path = codex_home.join(".env");
    let content = fs::read_to_string(&env_path).unwrap_or_default();
    fs::write(&env_path, render_codex_proxy_env(&content, proxy)).map_err(display_err)
}

fn apply_codex_launch_env(command: &mut Command, proxy: Option<&ProxySettings>) {
    if let Some(proxy_url) = codex_launch_proxy_url(proxy) {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env(key, &proxy_url);
        }
    }
    let existing = std::env::var("NO_PROXY")
        .ok()
        .or_else(|| std::env::var("no_proxy").ok());
    let no_proxy = merge_loopback_no_proxy(existing.as_deref());
    command.env("NO_PROXY", &no_proxy);
    command.env("no_proxy", no_proxy);
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppStore {
    #[serde(default)]
    pub(crate) settings: AppSettings,
    #[serde(default)]
    pub(crate) profiles: Vec<AccountProfile>,
    #[serde(default)]
    pub(crate) events: Vec<AppEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSettings {
    pub(crate) codex_home: Option<String>,
    pub(crate) current_profile_id: Option<String>,
    pub(crate) auto_switch_enabled: bool,
    #[serde(default)]
    pub(crate) probe_proxy: ProxySettings,
    #[serde(default)]
    pub(crate) auto_token_refresh_enabled: bool,
    #[serde(default = "default_auto_refresh_interval_secs")]
    pub(crate) auto_refresh_interval_secs: u64,
    #[serde(default)]
    pub(crate) background_token_refresh_enabled: bool,
    #[serde(default = "default_background_token_refresh_interval_secs")]
    pub(crate) background_token_refresh_interval_secs: u64,
    #[serde(default = "default_token_refresh_threshold_secs")]
    pub(crate) token_refresh_threshold_secs: u64,
    #[serde(default = "default_true")]
    pub(crate) auto_probe_enabled: bool,
    #[serde(default = "default_auto_probe_interval_secs")]
    pub(crate) auto_probe_interval_secs: u64,
    #[serde(default)]
    pub(crate) routing: RoutingSettings,
    #[serde(default)]
    pub(crate) mesh: easytier_mesh::MeshSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            codex_home: default_codex_home().map(|p| p.to_string_lossy().to_string()),
            current_profile_id: None,
            auto_switch_enabled: true,
            probe_proxy: ProxySettings::default(),
            auto_token_refresh_enabled: false,
            auto_refresh_interval_secs: default_auto_refresh_interval_secs(),
            background_token_refresh_enabled: false,
            background_token_refresh_interval_secs: default_background_token_refresh_interval_secs(
            ),
            token_refresh_threshold_secs: default_token_refresh_threshold_secs(),
            auto_probe_enabled: true,
            auto_probe_interval_secs: default_auto_probe_interval_secs(),
            routing: RoutingSettings::default(),
            mesh: easytier_mesh::MeshSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingSettings {
    #[serde(default = "default_routing_listen_host")]
    pub(crate) listen_host: String,
    #[serde(default = "default_routing_port")]
    pub(crate) port: u16,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) risk_confirmed: bool,
    #[serde(default)]
    pub(crate) applied_to_codex: bool,
    #[serde(default)]
    pub(crate) mode: RoutingMode,
    #[serde(default)]
    pub(crate) fixed_profile_id: Option<String>,
    #[serde(default = "default_routing_sticky_ttl_secs")]
    pub(crate) sticky_ttl_secs: u64,
    #[serde(default = "default_routing_log_retention_days")]
    pub(crate) log_retention_days: u32,
    #[serde(default)]
    pub(crate) encrypted_access_key: Option<SecretEnvelope>,
}

impl Default for RoutingSettings {
    fn default() -> Self {
        Self {
            listen_host: default_routing_listen_host(),
            port: default_routing_port(),
            enabled: false,
            risk_confirmed: false,
            applied_to_codex: false,
            mode: RoutingMode::Auto,
            fixed_profile_id: None,
            sticky_ttl_secs: default_routing_sticky_ttl_secs(),
            log_retention_days: default_routing_log_retention_days(),
            encrypted_access_key: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RoutingMode {
    Auto,
    Fixed,
}

impl Default for RoutingMode {
    fn default() -> Self {
        Self::Auto
    }
}

fn default_routing_listen_host() -> String {
    "0.0.0.0".to_string()
}

fn default_routing_port() -> u16 {
    15_722
}

fn default_routing_sticky_ttl_secs() -> u64 {
    3_600
}

pub(crate) fn default_routing_log_retention_days() -> u32 {
    7
}

fn default_auto_refresh_interval_secs() -> u64 {
    600
}

fn default_background_token_refresh_interval_secs() -> u64 {
    3_600
}

fn default_token_refresh_threshold_secs() -> u64 {
    0
}

fn default_auto_probe_interval_secs() -> u64 {
    60
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountProfile {
    pub(crate) id: String,
    pub(crate) alias: String,
    #[serde(default)]
    pub(crate) note: String,
    pub(crate) enabled: bool,
    pub(crate) priority: i32,
    pub(crate) cooldown_until: Option<String>,
    pub(crate) quota_rule: QuotaRule,
    pub(crate) summary: AuthSummary,
    pub(crate) encrypted_auth_json: SecretEnvelope,
    #[serde(default)]
    pub(crate) api_config: Option<ApiProviderConfig>,
    pub(crate) usage: UsageStats,
    #[serde(default)]
    pub(crate) route_health: RouteHealth,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiProviderConfig {
    pub(crate) provider_id: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    #[serde(default = "routing_protocol::default_wire_api")]
    pub(crate) wire_api: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuotaRule {
    pub(crate) hourly_limit: Option<u32>,
    pub(crate) daily_limit: Option<u32>,
    pub(crate) cooldown_minutes: u32,
}

impl Default for QuotaRule {
    fn default() -> Self {
        Self {
            hourly_limit: None,
            daily_limit: None,
            cooldown_minutes: 180,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageStats {
    pub(crate) hourly_used: u32,
    pub(crate) daily_used: u32,
    #[serde(default)]
    pub(crate) detected_limits: Vec<DetectedLimit>,
    pub(crate) detected_summary: Option<String>,
    pub(crate) last_probe_at: Option<String>,
    pub(crate) last_probe_status: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_used_at: Option<String>,
    pub(crate) estimated_reset_at: Option<String>,
    #[serde(default)]
    pub(crate) last_token_refresh_at: Option<String>,
    #[serde(default)]
    pub(crate) last_token_refresh_status: Option<String>,
    #[serde(default)]
    pub(crate) last_token_refresh_error: Option<String>,
    #[serde(default)]
    pub(crate) available_reset_count: Option<i64>,
    #[serde(default)]
    pub(crate) available_reset_expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DetectedLimit {
    pub(crate) window: String,
    pub(crate) used: Option<u32>,
    pub(crate) limit: Option<u32>,
    pub(crate) remaining: Option<u32>,
    pub(crate) used_percent: Option<u32>,
    pub(crate) remaining_percent: Option<u32>,
    pub(crate) reset_at: Option<String>,
    pub(crate) label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthSummary {
    pub(crate) email: Option<String>,
    pub(crate) plan: Option<String>,
    pub(crate) subscription_active_start: Option<String>,
    pub(crate) subscription_active_until: Option<String>,
    pub(crate) subscription_last_checked: Option<String>,
    pub(crate) account_id: Option<String>,
    pub(crate) user_id: Option<String>,
    pub(crate) organization_id: Option<String>,
    pub(crate) access_token_exp: Option<i64>,
    pub(crate) id_token_exp: Option<i64>,
    pub(crate) auth_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SecretEnvelope {
    pub(crate) v: u32,
    pub(crate) alg: String,
    pub(crate) nonce: String,
    pub(crate) ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppEvent {
    pub(crate) ts: String,
    pub(crate) level: String,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexHomeScan {
    codex_home: String,
    exists: bool,
    has_auth: bool,
    current_auth: Option<AuthSummary>,
    migratable: Vec<String>,
    excluded: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreView {
    settings: AppSettings,
    profiles: Vec<AccountProfileView>,
    events: Vec<AppEvent>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountProfileView {
    id: String,
    alias: String,
    note: String,
    enabled: bool,
    priority: i32,
    cooldown_until: Option<String>,
    quota_rule: QuotaRule,
    summary: AuthSummary,
    api_config: Option<ApiProviderConfig>,
    usage: UsageStats,
    route_health: RouteHealth,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RouteHealth {
    #[serde(default)]
    pub(crate) consecutive_failures: u32,
    #[serde(default)]
    pub(crate) active_connections: u32,
    #[serde(default)]
    pub(crate) last_route_at: Option<String>,
    #[serde(default)]
    pub(crate) last_status: Option<String>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
    #[serde(default)]
    pub(crate) cooldown_reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SwitchResult {
    profile_id: String,
    backup_path: Option<String>,
    codex_running: bool,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageProbeResult {
    profile_id: String,
    status: String,
    http_status: Option<u16>,
    raw_json: Option<Value>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageResetResult {
    profile_id: String,
    outcome: String,
    available_reset_count: Option<i64>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenRefreshBatchResult {
    refreshed: u32,
    skipped: u32,
    failed: u32,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexConfigFiles {
    codex_home: String,
    auth_json: ConfigFileView,
    config_toml: ConfigFileView,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigFileView {
    path: String,
    exists: bool,
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportEnvelope {
    format: String,
    version: u32,
    kdf: String,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundlePayload {
    manifest: BundleManifest,
    settings: AppSettings,
    profiles: Vec<ExportProfile>,
    files: Vec<BundleFile>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BundleManifest {
    format: String,
    version: u32,
    exported_at: String,
    platform: String,
    profile_count: usize,
    include_conversations: bool,
    files: Vec<BundleFileMeta>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundleFileMeta {
    path: String,
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundleFile {
    path: String,
    sha256: String,
    bytes_base64: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportProfile {
    id: String,
    alias: String,
    #[serde(default)]
    note: String,
    enabled: bool,
    priority: i32,
    cooldown_until: Option<String>,
    quota_rule: QuotaRule,
    summary: AuthSummary,
    auth_json: String,
    #[serde(default)]
    api_config: Option<ApiProviderConfig>,
    usage: UsageStats,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportResult {
    imported_profiles: usize,
    restored_files: usize,
    skipped_conversation_files: usize,
    message: String,
}

#[tauri::command]
fn get_store(app: AppHandle) -> Result<StoreView, String> {
    let mut store = load_store(&app)?;
    if !store.profiles.is_empty() {
        let key = load_master_key(&app)?;
        let mut changed = false;
        for profile in &mut store.profiles {
            if repair_successfully_reauthorized_profile(profile) {
                changed = true;
            }
            if profile.api_config.is_some() {
                continue;
            }
            let Ok(bytes) = decrypt_secret(&profile.encrypted_auth_json, &key) else {
                continue;
            };
            let Ok(auth_json) = String::from_utf8(bytes) else {
                continue;
            };
            let Ok(summary) = summarize_auth(&auth_json) else {
                continue;
            };
            if summary != profile.summary {
                profile.summary = summary;
                changed = true;
            }
        }
        if changed {
            save_store(&app, &store)?;
        }
    }
    Ok(store_view(store))
}

#[tauri::command]
fn scan_codex_home(app: AppHandle, codex_home: Option<String>) -> Result<CodexHomeScan, String> {
    let path = resolve_codex_home(&app, codex_home)?;
    scan_codex_home_path(&path)
}

#[tauri::command]
fn import_current_auth_as_profile(
    app: AppHandle,
    codex_home: Option<String>,
    alias: Option<String>,
) -> Result<StoreView, String> {
    let path = resolve_codex_home(&app, codex_home)?;
    let auth_path = path.join("auth.json");
    let auth_json =
        fs::read_to_string(&auth_path).map_err(|e| format!("无法读取当前 auth.json: {}", e))?;
    upsert_auth_profile(
        &app,
        auth_json,
        alias,
        Some(path),
        "已导入当前 auth.json 为账号 profile",
    )
}

#[tauri::command]
fn add_auth_json_profile(
    app: AppHandle,
    alias: Option<String>,
    auth_json: String,
) -> Result<StoreView, String> {
    upsert_auth_profile(
        &app,
        auth_json,
        alias,
        None,
        "已从 Token/JSON 添加账号 profile",
    )
}

#[tauri::command]
fn start_codex_oauth_login() -> Result<String, String> {
    start_official_cli_login()?;
    Ok("已启动 Codex/ChatGPT 登录，请在浏览器完成授权后返回导入当前账号".to_string())
}

#[tauri::command]
fn codex_oauth_login_start(app: AppHandle) -> Result<oauth::OAuthLoginStartResponse, String> {
    let response = oauth::start(app.clone(), app_data_dir(&app)?)?;
    let _ = open_external_url(&response.auth_url);
    Ok(response)
}

#[tauri::command]
fn codex_oauth_open_auth_url(login_id: String) -> Result<(), String> {
    let url = oauth::current_auth_url(&login_id)?;
    open_external_url(&url)
}

#[tauri::command]
fn codex_oauth_submit_callback_url(
    app: AppHandle,
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    oauth::accept_callback(&app_data_dir(&app)?, &login_id, &callback_url)?;
    app.emit(
        "codex-oauth-login-completed",
        serde_json::json!({ "loginId": login_id }),
    )
    .map_err(display_err)
}

#[tauri::command]
async fn codex_oauth_login_complete(
    app: AppHandle,
    login_id: String,
    alias: Option<String>,
) -> Result<StoreView, String> {
    let store = load_store(&app)?;
    let client = build_probe_client(&store.settings.probe_proxy)?;
    let tokens = oauth::complete_with_client(&app_data_dir(&app)?, &login_id, &client).await?;
    let access_claims = decode_jwt_claims(&tokens.access_token);
    let account_id = access_claims
        .as_ref()
        .and_then(extract_account_id_from_claims);
    let auth_json = serde_json::to_string_pretty(&serde_json::json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": tokens.id_token,
            "access_token": tokens.access_token,
            "refresh_token": tokens.refresh_token,
            "account_id": account_id
        },
        "last_refresh": now_string()
    }))
    .map_err(display_err)?;
    upsert_auth_profile(
        &app,
        auth_json,
        alias,
        None,
        "已通过原生 OAuth 添加 Codex 账号 profile",
    )
}

#[tauri::command]
fn codex_oauth_login_cancel(app: AppHandle, login_id: Option<String>) -> Result<(), String> {
    oauth::cancel(&app_data_dir(&app)?, login_id.as_deref())
}

fn upsert_auth_profile(
    app: &AppHandle,
    auth_json: String,
    alias: Option<String>,
    codex_home: Option<PathBuf>,
    event_message: &str,
) -> Result<StoreView, String> {
    let summary = summarize_auth(&auth_json)?;
    if summary.auth_mode.as_deref() != Some("chatgpt")
        && summary.access_token_exp.is_none()
        && summary.id_token_exp.is_none()
    {
        return Err("auth.json 未包含有效的 Codex ChatGPT Token".to_string());
    }
    let key = load_master_key(app)?;
    let now = now_string();
    let account_label = summary
        .email
        .clone()
        .or_else(|| summary.account_id.clone())
        .unwrap_or_else(|| "Codex Account".to_string());
    let mut store = load_store(app)?;
    if let Some(path) = codex_home {
        store.settings.codex_home = Some(path.to_string_lossy().to_string());
    }

    let existing_index = store.profiles.iter().position(|p| {
        p.summary.account_id.is_some()
            && p.summary.account_id == summary.account_id
            && summary.account_id.is_some()
    });
    let usage =
        usage_after_auth_replacement(existing_index.map(|idx| store.profiles[idx].usage.clone()));

    let profile = AccountProfile {
        id: existing_index
            .map(|idx| store.profiles[idx].id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        alias: alias.unwrap_or(account_label),
        note: existing_index
            .map(|idx| store.profiles[idx].note.clone())
            .unwrap_or_default(),
        enabled: true,
        priority: existing_index
            .map(|idx| store.profiles[idx].priority)
            .unwrap_or(100),
        cooldown_until: None,
        quota_rule: existing_index
            .map(|idx| store.profiles[idx].quota_rule.clone())
            .unwrap_or_default(),
        summary,
        encrypted_auth_json: encrypt_secret(auth_json.as_bytes(), &key)?,
        api_config: None,
        usage,
        route_health: existing_index
            .map(|idx| store.profiles[idx].route_health.clone())
            .unwrap_or_default(),
        created_at: existing_index
            .map(|idx| store.profiles[idx].created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
    };

    if let Some(idx) = existing_index {
        store.profiles[idx] = profile;
    } else {
        store.profiles.push(profile);
    }
    push_event(&mut store, "info", event_message);
    save_store(app, &store)?;
    Ok(store_view(store))
}

fn usage_after_auth_replacement(existing: Option<UsageStats>) -> UsageStats {
    let mut usage = existing.unwrap_or_default();
    clear_stale_auth_failure(&mut usage);
    usage
}

fn clear_stale_auth_failure(usage: &mut UsageStats) {
    usage.last_token_refresh_at = None;
    usage.last_token_refresh_status = None;
    usage.last_token_refresh_error = None;
    if usage
        .last_error
        .as_deref()
        .is_some_and(refresh_error_requires_relogin)
    {
        usage.last_error = None;
    }
}

fn repair_successfully_reauthorized_profile(profile: &mut AccountProfile) -> bool {
    if profile.usage.last_token_refresh_status.as_deref() != Some("relogin_required") {
        return false;
    }
    if !profile
        .summary
        .access_token_exp
        .is_some_and(|expires_at| expires_at > Utc::now().timestamp())
    {
        return false;
    }
    let Some(updated_at) = parse_time(&profile.updated_at) else {
        return false;
    };
    let Some(refresh_failed_at) = profile
        .usage
        .last_token_refresh_at
        .as_deref()
        .and_then(parse_time)
    else {
        return false;
    };
    let Some(probed_at) = profile.usage.last_probe_at.as_deref().and_then(parse_time) else {
        return false;
    };
    let probe_succeeded = profile
        .usage
        .last_probe_status
        .as_deref()
        .and_then(|status| status.parse::<u16>().ok())
        .is_some_and(|status| (200..300).contains(&status));
    if updated_at <= refresh_failed_at || probed_at < updated_at || !probe_succeeded {
        return false;
    }
    clear_stale_auth_failure(&mut profile.usage);
    true
}

#[tauri::command]
fn add_api_profile(
    app: AppHandle,
    alias: String,
    provider_id: String,
    base_url: String,
    model: String,
    wire_api: String,
    api_key: String,
) -> Result<StoreView, String> {
    let base_url = normalize_api_base_url(&base_url)?;
    let wire_api = routing_protocol::normalize_wire_api(&wire_api)?;
    let model = model.trim();
    let api_key = api_key.trim();
    if model.is_empty() || api_key.is_empty() {
        return Err("模型和 API Key 不能为空".to_string());
    }
    let provider_id = if provider_id.trim().is_empty() {
        normalize_provider_id(&format!("api-{}", Uuid::new_v4().simple()))?
    } else {
        normalize_provider_id(&provider_id)?
    };

    let auth_json = serde_json::to_string_pretty(&serde_json::json!({
        "auth_mode": "apikey",
        "OPENAI_API_KEY": api_key,
    }))
    .map_err(display_err)?;
    let alias = if alias.trim().is_empty() {
        format!("{provider_id} / {model}")
    } else {
        alias.trim().to_string()
    };
    let key = load_master_key(&app)?;
    let mut store = load_store(&app)?;
    if store.profiles.iter().any(|profile| {
        profile
            .api_config
            .as_ref()
            .is_some_and(|config| config.provider_id == provider_id)
    }) {
        return Err(format!("Provider ID {provider_id} 已存在"));
    }
    let now = now_string();
    store.profiles.push(AccountProfile {
        id: Uuid::new_v4().to_string(),
        alias,
        note: String::new(),
        enabled: true,
        priority: 100,
        cooldown_until: None,
        quota_rule: QuotaRule::default(),
        summary: AuthSummary {
            email: None,
            plan: Some("api_key".to_string()),
            subscription_active_start: None,
            subscription_active_until: None,
            subscription_last_checked: None,
            account_id: None,
            user_id: None,
            organization_id: None,
            access_token_exp: None,
            id_token_exp: None,
            auth_mode: Some("apikey".to_string()),
        },
        encrypted_auth_json: encrypt_secret(auth_json.as_bytes(), &key)?,
        api_config: Some(ApiProviderConfig {
            provider_id,
            base_url,
            model: model.to_string(),
            wire_api,
        }),
        usage: UsageStats::default(),
        route_health: RouteHealth::default(),
        created_at: now.clone(),
        updated_at: now,
    });
    push_event(&mut store, "info", "已添加 Codex API Provider");
    save_store(&app, &store)?;
    Ok(store_view(store))
}

#[tauri::command]
fn save_quota_rule(
    app: AppHandle,
    profile_id: String,
    alias: String,
    hourly_limit: Option<u32>,
    daily_limit: Option<u32>,
    cooldown_minutes: u32,
    enabled: bool,
    priority: i32,
) -> Result<StoreView, String> {
    let mut store = load_store(&app)?;
    let profile = store
        .profiles
        .iter_mut()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let alias = alias.trim();
    if alias.is_empty() {
        return Err("账号备注不能为空".to_string());
    }
    profile.alias = alias.to_string();
    profile.quota_rule = QuotaRule {
        hourly_limit,
        daily_limit,
        cooldown_minutes,
    };
    profile.enabled = enabled;
    profile.priority = priority;
    profile.updated_at = now_string();
    push_event(&mut store, "info", "已保存账号额度规则");
    save_store(&app, &store)?;
    Ok(store_view(store))
}

#[tauri::command]
fn update_profile_details(
    app: AppHandle,
    profile_id: String,
    alias: String,
    note: String,
    hourly_limit: Option<u32>,
    daily_limit: Option<u32>,
    cooldown_minutes: u32,
    enabled: bool,
    priority: i32,
    provider_id: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    wire_api: Option<String>,
    api_key: Option<String>,
) -> Result<StoreView, String> {
    let mut store = load_store(&app)?;
    let provider_id = provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_provider_id)
        .transpose()?;
    if let Some(ref provider_id) = provider_id {
        if store.profiles.iter().any(|profile| {
            profile.id != profile_id
                && profile
                    .api_config
                    .as_ref()
                    .is_some_and(|config| config.provider_id == *provider_id)
        }) {
            return Err(format!("Provider ID {provider_id} 已存在"));
        }
    }
    let base_url = base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_api_base_url)
        .transpose()?;
    let model = model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from);
    let wire_api = wire_api
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(routing_protocol::normalize_wire_api)
        .transpose()?;
    let api_key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from);
    let encrypted_api_auth = if let Some(api_key) = api_key {
        let key = load_master_key(&app)?;
        let auth_json = serde_json::to_string_pretty(&serde_json::json!({
            "auth_mode": "apikey",
            "OPENAI_API_KEY": api_key,
        }))
        .map_err(display_err)?;
        Some(encrypt_secret(auth_json.as_bytes(), &key)?)
    } else {
        None
    };
    let profile = store
        .profiles
        .iter_mut()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let alias = alias.trim();
    if alias.is_empty() {
        return Err("账号备注不能为空".to_string());
    }
    profile.alias = alias.to_string();
    profile.note = note.trim().to_string();
    profile.quota_rule = QuotaRule {
        hourly_limit,
        daily_limit,
        cooldown_minutes,
    };
    profile.enabled = enabled;
    profile.priority = priority;
    if let Some(config) = profile.api_config.as_mut() {
        if let Some(provider_id) = provider_id {
            config.provider_id = provider_id;
        }
        if let Some(base_url) = base_url {
            config.base_url = base_url;
        }
        if let Some(model) = model {
            config.model = model;
        }
        if let Some(wire_api) = wire_api {
            config.wire_api = wire_api;
        }
        if let Some(encrypted_api_auth) = encrypted_api_auth {
            profile.encrypted_auth_json = encrypted_api_auth;
        }
    }
    profile.updated_at = now_string();
    push_event(&mut store, "info", "已更新账号信息");
    save_store(&app, &store)?;
    Ok(store_view(store))
}

#[tauri::command]
fn delete_profile(app: AppHandle, profile_id: String) -> Result<StoreView, String> {
    let mut store = load_store(&app)?;
    let deleting_active_api = store.settings.current_profile_id.as_deref() == Some(&profile_id)
        && store
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .and_then(|profile| profile.api_config.as_ref())
            .is_some();
    let codex_home = store.settings.codex_home.clone();
    let alias = delete_profile_from_store(&mut store, &profile_id)?;
    if deleting_active_api {
        let path = resolve_codex_home(&app, codex_home)?;
        restore_api_config_backup(&path)?;
    }
    push_event(&mut store, "info", &format!("已删除账号 profile {}", alias));
    save_store(&app, &store)?;
    Ok(store_view(store))
}

#[tauri::command]
fn save_proxy_settings(app: AppHandle, enabled: bool, url: String) -> Result<StoreView, String> {
    let mut store = load_store(&app)?;
    let normalized = normalize_proxy_url(&url)?;
    if enabled && normalized.is_empty() {
        return Err("启用代理时必须填写代理地址".to_string());
    }
    store.settings.probe_proxy = ProxySettings {
        enabled,
        url: normalized,
    };
    if let Ok(codex_home) = resolve_codex_home(&app, store.settings.codex_home.clone()) {
        sync_codex_proxy_env_file(&codex_home, &store.settings.probe_proxy)?;
    }
    push_event(
        &mut store,
        "info",
        if enabled {
            "已保存代理设置并同步 Codex .env"
        } else {
            "已关闭代理并同步 Codex .env"
        },
    );
    save_store(&app, &store)?;
    Ok(store_view(store))
}

#[tauri::command]
async fn test_proxy_settings(
    enabled: bool,
    url: String,
) -> Result<proxy::ProbeProxyTestResult, String> {
    proxy::test_proxy_settings(enabled, url).await
}

#[tauri::command]
fn open_codex_home(app: AppHandle, codex_home: Option<String>) -> Result<(), String> {
    let path = resolve_codex_home(&app, codex_home)?;
    if !path.exists() {
        return Err(format!("目录不存在：{}", path.to_string_lossy()));
    }
    open_path_in_file_manager(&path)
}

#[tauri::command]
fn save_auto_settings(
    app: AppHandle,
    auto_token_refresh_enabled: bool,
    auto_refresh_interval_secs: u64,
    background_token_refresh_enabled: bool,
    background_token_refresh_interval_secs: u64,
    token_refresh_threshold_secs: u64,
    auto_probe_enabled: bool,
    auto_probe_interval_secs: u64,
) -> Result<StoreView, String> {
    let mut store = load_store(&app)?;
    let _ = auto_token_refresh_enabled;
    store.settings.auto_token_refresh_enabled = false;
    store.settings.auto_refresh_interval_secs = clamp_interval(auto_refresh_interval_secs);
    store.settings.background_token_refresh_enabled = background_token_refresh_enabled;
    store.settings.background_token_refresh_interval_secs =
        clamp_background_token_refresh_interval(background_token_refresh_interval_secs);
    store.settings.token_refresh_threshold_secs =
        clamp_token_refresh_threshold(token_refresh_threshold_secs);
    store.settings.auto_probe_enabled = auto_probe_enabled;
    store.settings.auto_probe_interval_secs = clamp_interval(auto_probe_interval_secs);
    push_event(&mut store, "info", "已保存自动刷新设置");
    save_store(&app, &store)?;
    Ok(store_view(store))
}

#[tauri::command]
fn routing_status(app: AppHandle) -> Result<routing::RoutingStatus, String> {
    routing::status(app)
}

#[tauri::command]
fn routing_save_settings(
    app: AppHandle,
    input: routing::SaveRoutingSettingsInput,
) -> Result<routing::RoutingStatus, String> {
    routing::save_settings(app, input)
}

#[tauri::command]
fn routing_save_log_settings(
    app: AppHandle,
    retention_days: u32,
) -> Result<routing::RoutingStatus, String> {
    routing::save_log_settings(app, retention_days)
}

#[tauri::command]
fn routing_start(app: AppHandle) -> Result<routing::RoutingStatus, String> {
    routing::start(app)
}

#[tauri::command]
fn routing_stop(app: AppHandle) -> Result<routing::RoutingStatus, String> {
    routing::stop(app)
}

#[tauri::command]
fn routing_regenerate_access_key(app: AppHandle) -> Result<routing::RoutingStatus, String> {
    routing::regenerate_access_key(app)
}

#[tauri::command]
fn routing_read_logs(app: AppHandle, limit: usize) -> Vec<routing::RoutingLogEntry> {
    routing::read_logs(app, limit)
}

#[tauri::command]
fn routing_test_request(app: AppHandle) -> Result<routing::RoutingProbeResult, String> {
    routing::test_request(app)
}

#[tauri::command]
fn routing_apply_codex_config(
    app: AppHandle,
    restart_codex: bool,
) -> Result<routing::RoutingStatus, String> {
    let codex_runtime = restart_codex.then(collect_codex_process_snapshot);
    let status = routing::apply_codex_config(app.clone())?;
    let Some(codex_runtime) = codex_runtime.filter(CodexProcessSnapshot::is_running) else {
        return Ok(status);
    };
    let proxy = load_store(&app)?.settings.probe_proxy;
    let mut relaunch_guard = CodexRelaunchGuard::default();
    relaunch_guard.arm(codex_runtime.clone(), Some(proxy));
    terminate_codex_processes(&codex_runtime)?;
    relaunch_guard.relaunch()?;
    thread::sleep(Duration::from_millis(800));
    routing::status(app)
}

#[tauri::command]
fn routing_restore_codex_config(app: AppHandle) -> Result<routing::RoutingStatus, String> {
    routing::restore_codex_config(app)
}

#[tauri::command]
fn mesh_status(app: AppHandle) -> Result<easytier_mesh::MeshStatus, String> {
    easytier_mesh::status(app)
}

#[tauri::command]
fn mesh_save_settings(
    app: AppHandle,
    input: easytier_mesh::MeshSaveSettingsInput,
) -> Result<easytier_mesh::MeshStatus, String> {
    easytier_mesh::save_settings(app, input)
}

#[tauri::command]
fn mesh_start(app: AppHandle) -> Result<easytier_mesh::MeshStatus, String> {
    easytier_mesh::start(app)
}

#[tauri::command]
fn mesh_stop(app: AppHandle) -> Result<easytier_mesh::MeshStatus, String> {
    easytier_mesh::stop(app)
}

#[tauri::command]
fn mesh_refresh_public_nodes(app: AppHandle) -> Result<easytier_mesh::MeshStatus, String> {
    easytier_mesh::refresh_public_nodes(app)
}

#[tauri::command]
fn mesh_create_share_payload(
    app: AppHandle,
    mode: easytier_mesh::MeshShareMode,
) -> Result<String, String> {
    easytier_mesh::create_share_payload(app, mode)
}

#[tauri::command]
fn mesh_import_share_payload(
    app: AppHandle,
    payload_text: String,
) -> Result<easytier_mesh::MeshImportResult, String> {
    easytier_mesh::import_share_payload(app, payload_text)
}

#[tauri::command]
fn mesh_list_devices(app: AppHandle) -> Result<Vec<easytier_mesh::MeshDeviceView>, String> {
    easytier_mesh::list_devices(app)
}

#[tauri::command]
fn mesh_save_device_sync(
    app: AppHandle,
    device_id: String,
    trusted: bool,
    sync_scope: easytier_mesh::MeshSyncScope,
) -> Result<easytier_mesh::MeshStatus, String> {
    easytier_mesh::save_device_sync(app, device_id, trusted, sync_scope)
}

#[tauri::command]
fn mesh_sync_now(
    app: AppHandle,
    device_id: Option<String>,
) -> Result<easytier_mesh::MeshStatus, String> {
    easytier_mesh::sync_now(app, device_id)
}

#[tauri::command]
fn mesh_export_migration_share(
    app: AppHandle,
    output_path: String,
    password: String,
    use_mesh_secret: bool,
    include_conversations: bool,
    profile_ids: Option<Vec<String>>,
) -> Result<BundleManifest, String> {
    let password = easytier_mesh::migration_password_from_mesh(&app, password, use_mesh_secret)?;
    export_all_accounts_bundle(
        app,
        output_path,
        password,
        include_conversations,
        profile_ids,
    )
}

#[tauri::command]
fn mesh_import_migration_share(
    app: AppHandle,
    bundle_path: String,
    password: String,
    use_mesh_secret: bool,
    restore_conversations: bool,
    codex_home: Option<String>,
) -> Result<ImportResult, String> {
    let password = easytier_mesh::migration_password_from_mesh(&app, password, use_mesh_secret)?;
    import_accounts_bundle(
        app,
        bundle_path,
        password,
        restore_conversations,
        codex_home,
    )
}

#[tauri::command]
fn refresh_profile_tokens_from_codex_home(
    app: AppHandle,
    codex_home: Option<String>,
    profile_id: Option<String>,
) -> Result<StoreView, String> {
    let path = resolve_codex_home(&app, codex_home)?;
    let auth_json = fs::read_to_string(path.join("auth.json"))
        .map_err(|e| format!("无法读取当前 auth.json: {}", e))?;
    let summary = summarize_auth(&auth_json)?;
    let key = load_master_key(&app)?;
    let mut store = load_store(&app)?;
    let target_id = profile_id.or_else(|| store.settings.current_profile_id.clone());
    let idx = if let Some(target_id) = target_id {
        let idx = store
            .profiles
            .iter()
            .position(|p| p.id == target_id)
            .ok_or_else(|| "没有找到要同步的账号 profile".to_string())?;
        if store.profiles[idx].summary.account_id.is_some()
            && summary.account_id.is_some()
            && store.profiles[idx].summary.account_id != summary.account_id
        {
            return Err("当前 auth.json 与所选账号不匹配，已跳过 token 同步".to_string());
        }
        idx
    } else {
        store
            .profiles
            .iter()
            .position(|p| {
                summary.account_id.is_some() && p.summary.account_id == summary.account_id
            })
            .ok_or_else(|| "没有找到与当前 auth.json 匹配的账号 profile".to_string())?
    };

    store.profiles[idx].summary = summary;
    store.profiles[idx].encrypted_auth_json = encrypt_secret(auth_json.as_bytes(), &key)?;
    store.profiles[idx].updated_at = now_string();
    push_event(
        &mut store,
        "info",
        "已同步当前 auth.json token 到账号 profile",
    );
    save_store(&app, &store)?;
    Ok(store_view(store))
}

fn sync_current_auth_into_profile(
    app: &AppHandle,
    store: &mut AppStore,
    idx: usize,
    key: &[u8; 32],
) -> Result<Option<String>, String> {
    let path = resolve_codex_home(app, store.settings.codex_home.clone())?;
    let auth_path = path.join("auth.json");
    if !auth_path.exists() {
        return Ok(None);
    }

    let auth_json =
        fs::read_to_string(&auth_path).map_err(|e| format!("无法读取当前 auth.json: {}", e))?;
    let summary = summarize_auth(&auth_json)?;
    let profile_summary = store.profiles[idx].summary.clone();

    if profile_summary.account_id.is_some()
        && summary.account_id.is_some()
        && profile_summary.account_id != summary.account_id
    {
        return Ok(None);
    }

    if let (Some(profile_email), Some(current_email)) =
        (profile_summary.email.as_deref(), summary.email.as_deref())
    {
        if !profile_email.eq_ignore_ascii_case(current_email) {
            return Ok(None);
        }
    }

    store.profiles[idx].summary = summary;
    store.profiles[idx].encrypted_auth_json = encrypt_secret(auth_json.as_bytes(), key)?;
    store.profiles[idx].updated_at = now_string();
    push_event(
        store,
        "info",
        "已从当前 Codex auth.json 同步账号 token 快照",
    );
    Ok(Some(auth_json))
}

fn auth_summaries_match(profile: &AuthSummary, current: &AuthSummary) -> bool {
    if let (Some(profile_account_id), Some(current_account_id)) =
        (profile.account_id.as_deref(), current.account_id.as_deref())
    {
        return profile_account_id == current_account_id;
    }

    if let (Some(profile_email), Some(current_email)) =
        (profile.email.as_deref(), current.email.as_deref())
    {
        return profile_email.eq_ignore_ascii_case(current_email);
    }

    false
}

fn sync_auth_json_into_matching_profile(
    store: &mut AppStore,
    auth_json: &str,
    key: &[u8; 32],
) -> Result<Option<String>, String> {
    let summary = summarize_auth(auth_json)?;
    let Some(idx) = store.profiles.iter().position(|profile| {
        profile.api_config.is_none() && auth_summaries_match(&profile.summary, &summary)
    }) else {
        return Ok(None);
    };

    let profile_id = store.profiles[idx].id.clone();
    store.profiles[idx].summary = summary;
    store.profiles[idx].encrypted_auth_json = encrypt_secret(auth_json.as_bytes(), key)?;
    store.profiles[idx].updated_at = now_string();
    Ok(Some(profile_id))
}

fn sync_codex_auth_into_matching_profile(
    app: &AppHandle,
    store: &mut AppStore,
    key: &[u8; 32],
) -> Result<Option<String>, String> {
    let path = resolve_codex_home(app, store.settings.codex_home.clone())?;
    let auth_path = path.join("auth.json");
    if !auth_path.exists() {
        return Ok(None);
    }

    let auth_json =
        fs::read_to_string(&auth_path).map_err(|e| format!("无法读取当前 auth.json: {e}"))?;
    sync_auth_json_into_matching_profile(store, &auth_json, key)
}

fn should_skip_profile_token_keepalive(
    profile: &AccountProfile,
    current_id: Option<&str>,
    include_current: bool,
) -> bool {
    !profile.enabled
        || (!include_current && current_id == Some(profile.id.as_str()))
        || profile.api_config.is_some()
        || profile.usage.last_token_refresh_status.as_deref() == Some("relogin_required")
}

#[tauri::command]
async fn refresh_all_profile_tokens(
    app: AppHandle,
    include_current: bool,
    threshold_secs: Option<u64>,
) -> Result<TokenRefreshBatchResult, String> {
    let key = load_master_key(&app)?;
    let mut store = load_store(&app)?;
    let client = build_probe_client(&store.settings.probe_proxy)?;
    let current_id = match sync_codex_auth_into_matching_profile(&app, &mut store, &key) {
        Ok(Some(profile_id)) => {
            store.settings.current_profile_id = Some(profile_id.clone());
            push_event(
                &mut store,
                "info",
                "已从当前 Codex auth.json 同步账号 token 快照",
            );
            Some(profile_id)
        }
        Ok(None) => store.settings.current_profile_id.clone(),
        Err(error) => {
            push_event(
                &mut store,
                "warn",
                &format!("同步当前 Codex auth.json 失败：{error}"),
            );
            store.settings.current_profile_id.clone()
        }
    };
    let threshold = clamp_token_refresh_threshold(
        threshold_secs.unwrap_or(store.settings.token_refresh_threshold_secs),
    );
    let mut refreshed = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for idx in 0..store.profiles.len() {
        if should_skip_profile_token_keepalive(
            &store.profiles[idx],
            current_id.as_deref(),
            include_current,
        ) {
            skipped += 1;
            continue;
        }
        if !should_refresh_access_token(store.profiles[idx].summary.access_token_exp, threshold) {
            skipped += 1;
            continue;
        }

        let now = now_string();
        let auth_json = match String::from_utf8(decrypt_secret(
            &store.profiles[idx].encrypted_auth_json,
            &key,
        )?) {
            Ok(value) => value,
            Err(err) => {
                failed += 1;
                store.profiles[idx].usage.last_token_refresh_at = Some(now);
                store.profiles[idx].usage.last_token_refresh_status =
                    Some("decode_error".to_string());
                store.profiles[idx].usage.last_token_refresh_error = Some(err.to_string());
                continue;
            }
        };

        match refresh_auth_json_with_client(&client, &auth_json).await {
            Ok(updated_auth_json) => {
                store.profiles[idx].summary = summarize_auth(&updated_auth_json)?;
                store.profiles[idx].encrypted_auth_json =
                    encrypt_secret(updated_auth_json.as_bytes(), &key)?;
                store.profiles[idx].usage.last_token_refresh_at = Some(now);
                store.profiles[idx].usage.last_token_refresh_status = Some("ok".to_string());
                store.profiles[idx].usage.last_token_refresh_error = None;
                store.profiles[idx].updated_at = now_string();
                refreshed += 1;
            }
            Err(err) => {
                failed += 1;
                store.profiles[idx].usage.last_token_refresh_at = Some(now);
                store.profiles[idx].usage.last_token_refresh_status = Some(
                    if refresh_error_requires_relogin(&err) {
                        "relogin_required"
                    } else {
                        "error"
                    }
                    .to_string(),
                );
                store.profiles[idx].usage.last_token_refresh_error = Some(err);
            }
        }
    }

    push_event(
        &mut store,
        "info",
        &format!(
            "token保活完成：刷新 {} 个，跳过 {} 个，失败 {} 个",
            refreshed, skipped, failed
        ),
    );
    save_store(&app, &store)?;
    Ok(TokenRefreshBatchResult {
        refreshed,
        skipped,
        failed,
        message: "已完成其他账号 token 保活检查".to_string(),
    })
}

#[tauri::command]
fn is_codex_process_running() -> bool {
    is_codex_running()
}

#[tauri::command]
async fn switch_profile(
    app: AppHandle,
    profile_id: String,
    codex_home: Option<String>,
    force: bool,
) -> Result<SwitchResult, String> {
    let switch_id = Uuid::new_v4().to_string();
    append_switch_diagnostic(
        &app,
        &switch_id,
        &profile_id,
        "start",
        format!("force={force}, codex_home={codex_home:?}"),
    );
    let mut store = load_store(&app)?;
    append_switch_diagnostic(&app, &switch_id, &profile_id, "store_loaded", "");
    let path = resolve_codex_home(&app, codex_home)?;
    append_switch_diagnostic(
        &app,
        &switch_id,
        &profile_id,
        "home_resolved",
        path.to_string_lossy(),
    );
    let key = load_master_key(&app)?;
    append_switch_diagnostic(&app, &switch_id, &profile_id, "master_key_loaded", "");
    let mut idx = store
        .profiles
        .iter()
        .position(|p| p.id == profile_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let mut profile = store.profiles[idx].clone();
    append_switch_diagnostic(
        &app,
        &switch_id,
        &profile_id,
        "profile_selected",
        format!(
            "alias={}, api={}",
            profile.alias,
            profile.api_config.is_some()
        ),
    );
    if store.settings.routing.applied_to_codex {
        return Err(
            "路由 API 已接管本机 Codex 配置；请在路由页固定账号或先恢复配置后再切换全局账号"
                .to_string(),
        );
    }
    if let Some(config) = profile.api_config.as_ref() {
        let protocol = routing_protocol::WireProtocol::parse(&config.wire_api)?;
        if protocol != routing_protocol::WireProtocol::Responses {
            return Err(
                "Chat Completions 与 Anthropic Messages 账号不能直接写入 Codex 配置；请启动路由并在路由页固定该账号"
                    .to_string(),
            );
        }
    }
    if !profile.enabled && !force {
        return Err("账号已禁用，不能自动切换；可先启用账号后再切换".to_string());
    }
    if let Some(cooldown) = &profile.cooldown_until {
        if parse_time(cooldown)
            .map(|t| t > Utc::now())
            .unwrap_or(false)
            && !force
        {
            return Err("账号仍在冷却中；如需强制切换请勾选强制切换".to_string());
        }
    }
    let codex_runtime = collect_codex_process_snapshot();
    let codex_running = codex_runtime.is_running();
    append_switch_diagnostic(
        &app,
        &switch_id,
        &profile_id,
        "codex_snapshot",
        format!(
            "running={codex_running}, processes={}",
            codex_runtime.processes.len()
        ),
    );
    if codex_running && !force {
        return Err("检测到 Codex 正在运行。为避免当前会话账号不匹配，请先关闭 Codex，或勾选强制切换后再继续。".to_string());
    }

    fs::create_dir_all(&path).map_err(display_err)?;
    let lock_path = path.join(".account-switcher.lock");
    append_switch_diagnostic(
        &app,
        &switch_id,
        &profile_id,
        "lock_open_start",
        lock_path.to_string_lossy(),
    );
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_path)
        .map_err(display_err)?;
    append_switch_diagnostic(&app, &switch_id, &profile_id, "lock_open_done", "");
    if let Err(error) = lock.try_lock_exclusive() {
        append_switch_diagnostic(
            &app,
            &switch_id,
            &profile_id,
            "lock_busy",
            error.to_string(),
        );
        return Err("已有账号切换任务仍在执行，请关闭并重开 CodexSwitcher 后再试".to_string());
    }
    append_switch_diagnostic(&app, &switch_id, &profile_id, "lock_acquired", "");

    let auth_path = path.join("auth.json");
    append_switch_diagnostic(
        &app,
        &switch_id,
        &profile_id,
        "current_auth_sync_start",
        auth_path.to_string_lossy(),
    );
    if auth_path.exists() {
        let current_auth_json =
            fs::read_to_string(&auth_path).map_err(|e| format!("无法读取当前 auth.json: {e}"))?;
        if let Some(synced_profile_id) =
            sync_auth_json_into_matching_profile(&mut store, &current_auth_json, &key)?
        {
            push_event(
                &mut store,
                "info",
                "切换前已保存当前 Codex 刷新后的 token 快照",
            );
            save_store(&app, &store)?;
            if synced_profile_id == profile_id {
                idx = store
                    .profiles
                    .iter()
                    .position(|p| p.id == profile_id)
                    .ok_or_else(|| "账号不存在".to_string())?;
                profile = store.profiles[idx].clone();
            }
        }
    }
    append_switch_diagnostic(&app, &switch_id, &profile_id, "current_auth_sync_done", "");

    append_switch_diagnostic(&app, &switch_id, &profile_id, "decrypt_profile_start", "");
    let mut auth_json = String::from_utf8(decrypt_secret(&profile.encrypted_auth_json, &key)?)
        .map_err(|e| e.to_string())?;
    append_switch_diagnostic(&app, &switch_id, &profile_id, "decrypt_profile_done", "");
    if profile.api_config.is_none()
        && should_refresh_access_token(profile.summary.access_token_exp, 0)
    {
        append_switch_diagnostic(&app, &switch_id, &profile_id, "token_refresh_start", "");
        let client = build_probe_client(&store.settings.probe_proxy)?;
        auth_json = match refresh_auth_json_with_client(&client, &auth_json).await {
            Ok(updated) => updated,
            Err(error) => {
                append_switch_diagnostic(
                    &app,
                    &switch_id,
                    &profile_id,
                    "token_refresh_failed",
                    &error,
                );
                store.profiles[idx].usage.last_token_refresh_at = Some(now_string());
                store.profiles[idx].usage.last_token_refresh_status = Some(
                    if refresh_error_requires_relogin(&error) {
                        "relogin_required"
                    } else {
                        "error"
                    }
                    .to_string(),
                );
                store.profiles[idx].usage.last_token_refresh_error = Some(error.clone());
                save_store(&app, &store)?;
                return Err(format!("账号 token 已过期且刷新失败：{error}"));
            }
        };
        store.profiles[idx].summary = summarize_auth(&auth_json)?;
        store.profiles[idx].encrypted_auth_json = encrypt_secret(auth_json.as_bytes(), &key)?;
        store.profiles[idx].usage.last_token_refresh_at = Some(now_string());
        store.profiles[idx].usage.last_token_refresh_status = Some("ok".to_string());
        store.profiles[idx].usage.last_token_refresh_error = None;
        append_switch_diagnostic(&app, &switch_id, &profile_id, "token_refresh_done", "");
    }

    let config_path = path.join("config.toml");
    let config_backup_path = path.join("config.toml.account-switcher.backup");
    sync_codex_proxy_env_file(&path, &store.settings.probe_proxy)?;
    append_switch_diagnostic(&app, &switch_id, &profile_id, "proxy_env_synced", ".env");
    append_switch_diagnostic(&app, &switch_id, &profile_id, "auth_write_start", "");
    let backup_path = if let Some(api_config) = profile.api_config.as_ref() {
        if !config_backup_path.exists() {
            if config_path.exists() {
                fs::copy(&config_path, &config_backup_path).map_err(display_err)?;
            } else {
                fs::write(&config_backup_path, []).map_err(display_err)?;
            }
        }
        let _api_key = serde_json::from_str::<Value>(&auth_json)
            .map_err(display_err)?
            .get("OPENAI_API_KEY")
            .and_then(Value::as_str)
            .ok_or_else(|| "API Provider 缺少 OPENAI_API_KEY".to_string())?
            .to_string();
        let auth_backup_path = backup_auth_file(&auth_path)?;
        replace_file_with_rollback(
            &auth_path,
            auth_json.as_bytes(),
            auth_backup_path.as_deref(),
        )?;
        let managed_provider_ids = store
            .profiles
            .iter()
            .filter_map(|profile| {
                profile
                    .api_config
                    .as_ref()
                    .map(|config| config.provider_id.clone())
            })
            .collect::<Vec<_>>();
        write_api_provider_config(
            &config_path,
            api_config,
            &profile.alias,
            &managed_provider_ids,
        )?;
        auth_backup_path
    } else {
        let backup_path = backup_auth_file(&auth_path)?;
        replace_file_with_rollback(&auth_path, auth_json.as_bytes(), backup_path.as_deref())?;
        backup_path
    };
    verify_auth_json_written(&auth_path, &auth_json)?;
    append_switch_diagnostic(&app, &switch_id, &profile_id, "auth_write_done", "");
    let config_cleanup_warning = if profile.api_config.is_none() {
        cleanup_non_api_config_with_timeout(
            app.clone(),
            switch_id.clone(),
            profile_id.clone(),
            path.clone(),
            config_path.clone(),
        )
    } else {
        None
    };

    store.settings.codex_home = Some(path.to_string_lossy().to_string());
    store.settings.current_profile_id = Some(profile_id.clone());
    store.profiles[idx].usage.last_used_at = Some(now_string());
    store.profiles[idx].updated_at = now_string();
    push_event(
        &mut store,
        "info",
        &format!("已写入 Codex auth.json：{}", profile.alias),
    );
    if let Some(warning) = config_cleanup_warning {
        push_event(&mut store, "warn", &warning);
    }
    if !codex_running {
        repair_codex_session_visibility_after_switch(
            &app,
            &mut store,
            &switch_id,
            &profile_id,
            &path,
        );
    }
    append_switch_diagnostic(
        &app,
        &switch_id,
        &profile_id,
        "store_save_after_write_start",
        "",
    );
    save_store(&app, &store)?;
    append_switch_diagnostic(
        &app,
        &switch_id,
        &profile_id,
        "store_save_after_write_done",
        "",
    );

    let mut restart_message = None;
    if codex_running && force {
        let mut relaunch_guard = CodexRelaunchGuard::default();
        append_switch_diagnostic(&app, &switch_id, &profile_id, "codex_terminate_start", "");
        match terminate_codex_processes(&codex_runtime) {
            Ok(()) => {
                append_switch_diagnostic(&app, &switch_id, &profile_id, "codex_terminate_done", "");
                repair_codex_session_visibility_after_switch(
                    &app,
                    &mut store,
                    &switch_id,
                    &profile_id,
                    &path,
                );
                relaunch_guard.arm(
                    codex_runtime.clone(),
                    Some(store.settings.probe_proxy.clone()),
                );
                append_switch_diagnostic(&app, &switch_id, &profile_id, "codex_relaunch_start", "");
                match relaunch_guard.relaunch() {
                    Ok(()) => {
                        append_switch_diagnostic(
                            &app,
                            &switch_id,
                            &profile_id,
                            "codex_relaunch_done",
                            "",
                        );
                        thread::sleep(Duration::from_millis(800));
                        if let Err(error) = verify_auth_json_written(&auth_path, &auth_json) {
                            push_event(
                                &mut store,
                                "warn",
                                &format!("切换后 auth.json 校验失败：{error}"),
                            );
                            save_store(&app, &store)?;
                            return Err(error);
                        }
                        restart_message = Some("已强制切换并重启 Codex".to_string());
                        push_event(
                            &mut store,
                            "info",
                            &format!("已切换到账号 {}，并重启 Codex", profile.alias),
                        );
                    }
                    Err(error) => {
                        append_switch_diagnostic(
                            &app,
                            &switch_id,
                            &profile_id,
                            "codex_relaunch_failed",
                            &error,
                        );
                        restart_message = Some(format!("已切换账号，但重启 Codex 失败：{error}"));
                        push_event(
                            &mut store,
                            "warn",
                            &format!("已切换到账号 {}，但重启 Codex 失败：{error}", profile.alias),
                        );
                    }
                }
            }
            Err(error) => {
                append_switch_diagnostic(
                    &app,
                    &switch_id,
                    &profile_id,
                    "codex_terminate_failed",
                    &error,
                );
                restart_message = Some(format!(
                    "已写入账号，但关闭 Codex 失败；请手动重启 Codex：{error}"
                ));
                push_event(
                    &mut store,
                    "warn",
                    &format!("已写入账号 {}，但关闭 Codex 失败：{error}", profile.alias),
                );
                if let Err(error) = verify_auth_json_written(&auth_path, &auth_json) {
                    push_event(
                        &mut store,
                        "warn",
                        &format!("切换后 auth.json 校验失败：{error}"),
                    );
                    save_store(&app, &store)?;
                    return Err(error);
                }
            }
        }
    }
    if restart_message.is_none() {
        push_event(
            &mut store,
            "info",
            &format!("已切换到账号 {}", profile.alias),
        );
    }
    append_switch_diagnostic(&app, &switch_id, &profile_id, "final_store_save_start", "");
    save_store(&app, &store)?;
    append_switch_diagnostic(&app, &switch_id, &profile_id, "done", "");

    Ok(SwitchResult {
        profile_id,
        backup_path: backup_path.map(|p| p.to_string_lossy().to_string()),
        codex_running,
        message: if let Some(message) = restart_message {
            message
        } else if codex_running {
            "已切换，但检测到 Codex 进程正在运行，当前会话可能仍使用旧账号".to_string()
        } else {
            "已切换当前 Codex 账号".to_string()
        },
    })
}

fn repair_codex_session_visibility_after_switch(
    app: &AppHandle,
    store: &mut AppStore,
    switch_id: &str,
    profile_id: &str,
    codex_home: &Path,
) {
    append_switch_diagnostic(
        app,
        switch_id,
        profile_id,
        "session_visibility_repair_start",
        codex_home.to_string_lossy(),
    );
    match codex_sessions::repair_session_visibility_for_current_provider(codex_home) {
        Ok(report) => {
            append_switch_diagnostic(
                app,
                switch_id,
                profile_id,
                "session_visibility_repair_done",
                format!(
                    "provider={}, checked={}, updated={}, skipped={}",
                    report.target_provider,
                    report.checked_databases,
                    report.updated_rows,
                    report.skipped_databases
                ),
            );
            if report.changed() {
                push_event(store, "info", &report.summary());
            } else if report.skipped_databases > 0 {
                push_event(store, "warn", &report.summary());
            }
        }
        Err(error) => {
            append_switch_diagnostic(
                app,
                switch_id,
                profile_id,
                "session_visibility_repair_failed",
                &error,
            );
            push_event(
                store,
                "warn",
                &format!("Codex 会话可见性修复失败，切换已继续: {error}"),
            );
        }
    }
}

#[tauri::command]
async fn probe_usage(app: AppHandle, profile_id: String) -> Result<UsageProbeResult, String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let idx = store
        .profiles
        .iter()
        .position(|p| p.id == profile_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let mut auth_json = String::from_utf8(decrypt_secret(
        &store.profiles[idx].encrypted_auth_json,
        &key,
    )?)
    .map_err(|e| e.to_string())?;
    let client = build_probe_client(&store.settings.probe_proxy)?;
    if let Some(api_config) = store.profiles[idx].api_config.clone() {
        let auth: Value = serde_json::from_str(&auth_json).map_err(display_err)?;
        let api_key = auth
            .get("OPENAI_API_KEY")
            .and_then(Value::as_str)
            .ok_or_else(|| "API Provider 缺少 OPENAI_API_KEY".to_string())?;
        let response = client
            .get(format!("{}/models", api_config.base_url))
            .bearer_auth(api_key)
            .send()
            .await;
        let now = now_string();
        return match response {
            Ok(response) => {
                let status = response.status().as_u16();
                let body = response.json::<Value>().await.ok();
                store.profiles[idx].usage.last_probe_at = Some(now);
                store.profiles[idx].usage.last_probe_status = Some(status.to_string());
                store.profiles[idx].usage.last_error =
                    (status >= 400).then(|| format!("API HTTP {status}"));
                save_store(&app, &store)?;
                Ok(UsageProbeResult {
                    profile_id,
                    status: if status < 400 { "ok" } else { "http_error" }.to_string(),
                    http_status: Some(status),
                    raw_json: body,
                    message: if status < 400 {
                        "API Provider 连接正常".to_string()
                    } else {
                        "API Provider 连接失败".to_string()
                    },
                })
            }
            Err(error) => {
                store.profiles[idx].usage.last_probe_at = Some(now);
                store.profiles[idx].usage.last_probe_status = Some("network_error".to_string());
                store.profiles[idx].usage.last_error = Some(error.to_string());
                save_store(&app, &store)?;
                Ok(UsageProbeResult {
                    profile_id,
                    status: "network_error".to_string(),
                    http_status: None,
                    raw_json: None,
                    message: format!("API Provider 连接失败: {error}"),
                })
            }
        };
    }
    if should_refresh_access_token(store.profiles[idx].summary.access_token_exp, 0) {
        let is_current_profile =
            store.settings.current_profile_id.as_deref() == Some(profile_id.as_str());
        if is_current_profile {
            if let Some(current_auth_json) =
                sync_current_auth_into_profile(&app, &mut store, idx, &key)?
            {
                auth_json = current_auth_json;
            }

            if should_refresh_access_token(store.profiles[idx].summary.access_token_exp, 0) {
                let message = "当前全局账号 access token 已过期；切换器不会自动刷新当前正在使用的账号 token，请让 Codex 自行刷新或重新登录后重新导入当前 auth.json";
                store.profiles[idx].usage.last_token_refresh_at = Some(now_string());
                store.profiles[idx].usage.last_token_refresh_status =
                    Some("skipped_current".to_string());
                store.profiles[idx].usage.last_token_refresh_error = Some(message.to_string());
                push_event(&mut store, "warn", message);
                save_store(&app, &store)?;
                return Err(message.to_string());
            }
        } else {
            auth_json = match refresh_auth_json_with_client(&client, &auth_json).await {
                Ok(updated) => updated,
                Err(error) => {
                    let message = format!("账号 token 已过期且刷新失败：{error}");
                    store.profiles[idx].usage.last_token_refresh_at = Some(now_string());
                    store.profiles[idx].usage.last_token_refresh_status = Some(
                        if refresh_error_requires_relogin(&error) {
                            "relogin_required"
                        } else {
                            "error"
                        }
                        .to_string(),
                    );
                    store.profiles[idx].usage.last_token_refresh_error = Some(error);
                    store.profiles[idx].usage.last_error = Some(message.clone());
                    push_event(&mut store, "warn", &message);
                    save_store(&app, &store)?;
                    return Err(message);
                }
            };
            store.profiles[idx].summary = summarize_auth(&auth_json)?;
            store.profiles[idx].encrypted_auth_json = encrypt_secret(auth_json.as_bytes(), &key)?;
            store.profiles[idx].usage.last_token_refresh_at = Some(now_string());
            store.profiles[idx].usage.last_token_refresh_status = Some("ok".to_string());
            store.profiles[idx].usage.last_token_refresh_error = None;
        }
    }
    let auth: Value = serde_json::from_str(&auth_json).map_err(display_err)?;
    let access_token = auth
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "auth.json 中没有 access_token，无法探测 usage".to_string())?;

    let response = client
        .get(CHATGPT_USAGE_URL)
        .bearer_auth(access_token)
        .send()
        .await;

    let now = now_string();
    match response {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let json_body = resp.json::<Value>().await.ok();
            store.profiles[idx].usage.last_probe_at = Some(now);
            store.profiles[idx].usage.last_probe_status = Some(status.to_string());
            store.profiles[idx].usage.last_error = None;
            if let Some(body) = &json_body {
                apply_usage_probe_body(&mut store.profiles[idx], body);
            }
            if status == 429 {
                let cooldown = Utc::now()
                    + chrono::Duration::minutes(
                        store.profiles[idx].quota_rule.cooldown_minutes as i64,
                    );
                store.profiles[idx].cooldown_until = Some(cooldown.to_rfc3339());
                store.profiles[idx].usage.estimated_reset_at = Some(cooldown.to_rfc3339());
            }
            push_event(
                &mut store,
                "info",
                &format!("额度探测完成，HTTP {}", status),
            );
            save_store(&app, &store)?;
            Ok(UsageProbeResult {
                profile_id,
                status: if status < 400 { "ok" } else { "http_error" }.to_string(),
                http_status: Some(status),
                raw_json: json_body,
                message: "已完成 usage 探测；若返回结构为空，将继续使用本地估算".to_string(),
            })
        }
        Err(err) => {
            store.profiles[idx].usage.last_probe_at = Some(now);
            store.profiles[idx].usage.last_probe_status = Some("network_error".to_string());
            store.profiles[idx].usage.last_error = Some(err.to_string());
            push_event(&mut store, "warn", "额度探测失败，已回退到本地估算");
            save_store(&app, &store)?;
            Ok(UsageProbeResult {
                profile_id,
                status: "network_error".to_string(),
                http_status: None,
                raw_json: None,
                message: "usage 接口探测失败，已记录为本地估算状态".to_string(),
            })
        }
    }
}

#[tauri::command]
async fn consume_usage_reset(
    app: AppHandle,
    profile_id: String,
) -> Result<UsageResetResult, String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let idx = store
        .profiles
        .iter()
        .position(|profile| profile.id == profile_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    if store.profiles[idx].usage.available_reset_count == Some(0) {
        return Err("没有可用的用量重置次数".to_string());
    }

    let auth_json = String::from_utf8(decrypt_secret(
        &store.profiles[idx].encrypted_auth_json,
        &key,
    )?)
    .map_err(|error| error.to_string())?;
    let auth: Value = serde_json::from_str(&auth_json).map_err(display_err)?;
    let access_token = auth
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "账号凭据中没有 access_token，无法重置用量".to_string())?;
    let client = build_probe_client(&store.settings.probe_proxy)?;
    let idempotency_key = Uuid::new_v4().to_string();
    let response = client
        .post("https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume")
        .bearer_auth(access_token)
        .json(&serde_json::json!({ "redeem_request_id": idempotency_key }))
        .send()
        .await
        .map_err(|error| format!("用量重置请求失败: {error}"))?;
    let status = response.status();
    let body = response
        .json::<Value>()
        .await
        .map_err(|error| format!("无法解析用量重置响应: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "用量重置失败（HTTP {}）: {}",
            status.as_u16(),
            body
        ));
    }

    let outcome = body
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    match outcome.as_str() {
        "reset" | "already_redeemed" => {}
        "nothing_to_reset" => return Err("当前用量窗口尚不需要重置".to_string()),
        "no_credit" => {
            store.profiles[idx].usage.available_reset_count = Some(0);
            save_store(&app, &store)?;
            return Err("没有可用的用量重置次数".to_string());
        }
        _ => return Err(format!("服务端返回未知的重置结果: {outcome}")),
    }

    let usage_response = client
        .get(CHATGPT_USAGE_URL)
        .bearer_auth(access_token)
        .send()
        .await;
    if let Ok(response) = usage_response {
        if let Ok(usage_body) = response.json::<Value>().await {
            apply_usage_probe_body(&mut store.profiles[idx], &usage_body);
        } else {
            store.profiles[idx].usage.available_reset_count = None;
        }
    } else {
        store.profiles[idx].usage.available_reset_count = None;
    }
    store.profiles[idx].usage.last_probe_at = Some(now_string());
    store.profiles[idx].updated_at = now_string();
    let available_reset_count = store.profiles[idx].usage.available_reset_count;
    push_event(&mut store, "info", "已使用一次 Codex 用量重置并刷新额度");
    save_store(&app, &store)?;

    Ok(UsageResetResult {
        profile_id,
        outcome,
        available_reset_count,
        message: "用量已重置".to_string(),
    })
}

#[tauri::command]
fn export_all_accounts_bundle(
    app: AppHandle,
    output_path: String,
    password: String,
    include_conversations: bool,
    profile_ids: Option<Vec<String>>,
) -> Result<BundleManifest, String> {
    export_all_accounts_bundle_internal(
        app,
        output_path,
        password,
        include_conversations,
        profile_ids,
    )
}

pub(crate) fn export_all_accounts_bundle_internal(
    app: AppHandle,
    output_path: String,
    password: String,
    include_conversations: bool,
    profile_ids: Option<Vec<String>>,
) -> Result<BundleManifest, String> {
    export_bundle_internal(
        app,
        output_path,
        password,
        include_conversations,
        profile_ids,
        false,
        false,
    )
}

pub(crate) fn export_mesh_sync_bundle_internal(
    app: AppHandle,
    output_path: String,
    password: String,
    include_conversations: bool,
    include_accounts: bool,
    only_valid_accounts: bool,
) -> Result<BundleManifest, String> {
    export_bundle_internal(
        app,
        output_path,
        password,
        include_conversations,
        if include_accounts {
            None
        } else {
            Some(Vec::new())
        },
        true,
        only_valid_accounts,
    )
}

fn export_bundle_internal(
    app: AppHandle,
    output_path: String,
    password: String,
    include_conversations: bool,
    profile_ids: Option<Vec<String>>,
    allow_empty_profiles: bool,
    only_valid_accounts: bool,
) -> Result<BundleManifest, String> {
    if !password.is_empty() && password.len() < 8 {
        return Err("导出口令至少需要 8 位；如需明文导出请留空".to_string());
    }
    let store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let codex_home = resolve_codex_home(&app, store.settings.codex_home.clone())?;
    let selected_ids = if only_valid_accounts {
        Some(
            store
                .profiles
                .iter()
                .filter(|profile| is_valid_mesh_account(profile))
                .map(|profile| profile.id.clone())
                .collect::<Vec<_>>(),
        )
    } else {
        profile_ids
    };
    let profiles = select_profiles_for_export(
        &store.profiles,
        selected_ids.as_deref(),
        allow_empty_profiles,
    )?;
    let export_profiles = profiles
        .into_iter()
        .map(|p| {
            let auth_json = String::from_utf8(decrypt_secret(&p.encrypted_auth_json, &key)?)
                .map_err(|e| e.to_string())?;
            Ok(ExportProfile {
                id: p.id.clone(),
                alias: p.alias.clone(),
                note: p.note.clone(),
                enabled: p.enabled,
                priority: p.priority,
                cooldown_until: p.cooldown_until.clone(),
                quota_rule: p.quota_rule.clone(),
                summary: p.summary.clone(),
                auth_json,
                api_config: p.api_config.clone(),
                usage: p.usage.clone(),
                created_at: p.created_at.clone(),
                updated_at: p.updated_at.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let files = collect_bundle_files(&codex_home, include_conversations)?;
    let metas = files
        .iter()
        .map(|f| BundleFileMeta {
            path: f.path.clone(),
            sha256: f.sha256.clone(),
            bytes: STANDARD
                .decode(&f.bytes_base64)
                .map(|b| b.len() as u64)
                .unwrap_or(0),
        })
        .collect::<Vec<_>>();

    let manifest = BundleManifest {
        format: EXPORT_FORMAT.to_string(),
        version: EXPORT_VERSION,
        exported_at: now_string(),
        platform: std::env::consts::OS.to_string(),
        profile_count: export_profiles.len(),
        include_conversations,
        files: metas,
    };
    let mut export_settings = store.settings.clone();
    export_settings.routing.enabled = false;
    export_settings.routing.applied_to_codex = false;
    export_settings.routing.encrypted_access_key = None;

    let payload = BundlePayload {
        manifest,
        settings: export_settings,
        profiles: export_profiles,
        files,
    };

    let zip_bytes = zip_payload(&payload)?;
    if password.is_empty() {
        fs::write(&output_path, zip_bytes).map_err(display_err)?;
    } else {
        let encrypted = encrypt_export(&zip_bytes, &password)?;
        let bytes = serde_json::to_vec_pretty(&encrypted).map_err(display_err)?;
        fs::write(&output_path, bytes).map_err(display_err)?;
    }

    Ok(payload.manifest)
}

fn select_profiles_for_export<'a>(
    profiles: &'a [AccountProfile],
    profile_ids: Option<&[String]>,
    allow_empty: bool,
) -> Result<Vec<&'a AccountProfile>, String> {
    let Some(profile_ids) = profile_ids else {
        if profiles.is_empty() && !allow_empty {
            return Err("请选择至少一个账号导出".to_string());
        }
        return Ok(profiles.iter().collect());
    };
    let requested = profile_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    if requested.is_empty() && !allow_empty {
        return Err("请选择至少一个账号导出".to_string());
    }
    if let Some(missing_id) = requested
        .iter()
        .find(|id| !profiles.iter().any(|profile| profile.id.as_str() == **id))
    {
        return Err(format!("导出账号不存在: {}", missing_id));
    }
    Ok(profiles
        .iter()
        .filter(|profile| requested.iter().any(|id| profile.id.as_str() == *id))
        .collect())
}

fn is_valid_mesh_account(profile: &AccountProfile) -> bool {
    profile.enabled
        && profile.usage.last_token_refresh_status.as_deref() != Some("relogin_required")
        && profile
            .summary
            .access_token_exp
            .is_none_or(|expires_at| expires_at > Utc::now().timestamp())
}

#[tauri::command]
fn preview_bundle(bundle_path: String, password: String) -> Result<BundleManifest, String> {
    let payload = read_bundle(&bundle_path, &password)?;
    Ok(payload.manifest)
}

#[tauri::command]
fn import_accounts_bundle(
    app: AppHandle,
    bundle_path: String,
    password: String,
    restore_conversations: bool,
    codex_home: Option<String>,
) -> Result<ImportResult, String> {
    import_accounts_bundle_with_scope(
        app,
        bundle_path,
        password,
        restore_conversations,
        codex_home,
        None,
    )
}

pub(crate) fn import_accounts_bundle_with_scope(
    app: AppHandle,
    bundle_path: String,
    password: String,
    restore_conversations: bool,
    codex_home: Option<String>,
    mesh_scope: Option<easytier_mesh::MeshSyncScope>,
) -> Result<ImportResult, String> {
    let payload = read_bundle(&bundle_path, &password)?;
    let key = load_master_key(&app)?;
    let mut store = load_store(&app)?;
    let target_codex_home = resolve_codex_home(&app, codex_home)?;
    fs::create_dir_all(&target_codex_home).map_err(display_err)?;

    let mut imported = 0usize;
    for profile in payload.profiles {
        if mesh_scope.as_ref().is_some_and(|scope| !scope.accounts) {
            break;
        }
        let encrypted_auth_json = encrypt_secret(profile.auth_json.as_bytes(), &key)?;
        let local_profile = AccountProfile {
            id: profile.id,
            alias: profile.alias,
            note: profile.note,
            enabled: profile.enabled,
            priority: profile.priority,
            cooldown_until: profile.cooldown_until,
            quota_rule: profile.quota_rule,
            summary: profile.summary,
            encrypted_auth_json,
            api_config: profile.api_config,
            usage: profile.usage,
            route_health: RouteHealth::default(),
            created_at: profile.created_at,
            updated_at: now_string(),
        };
        if let Some(idx) = store.profiles.iter().position(|p| {
            p.id == local_profile.id
                || (p.summary.account_id.is_some()
                    && p.summary.account_id == local_profile.summary.account_id)
        }) {
            store.profiles[idx] = local_profile;
        } else {
            store.profiles.push(local_profile);
        }
        imported += 1;
    }

    let mut restored = 0usize;
    let mut skipped_conversations = 0usize;
    for file in payload.files {
        if mesh_scope
            .as_ref()
            .is_some_and(|scope| !mesh_file_in_scope(&file.path, scope))
        {
            continue;
        }
        if is_conversation_path(&file.path) && !restore_conversations {
            skipped_conversations += 1;
            continue;
        }
        if is_excluded_path(&file.path) {
            continue;
        }
        let bytes = STANDARD.decode(&file.bytes_base64).map_err(display_err)?;
        let sha = hex_sha256(&bytes);
        if sha != file.sha256 {
            return Err(format!("迁移包文件校验失败: {}", file.path));
        }
        let out = target_codex_home.join(safe_relative_path(&file.path)?);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(display_err)?;
        }
        fs::write(out, bytes).map_err(display_err)?;
        restored += 1;
    }

    store.settings.codex_home = Some(target_codex_home.to_string_lossy().to_string());
    if store.settings.current_profile_id.is_none() {
        store.settings.current_profile_id = store.profiles.first().map(|p| p.id.clone());
    }
    push_event(
        &mut store,
        "info",
        &format!("已导入迁移包，恢复 {} 个账号", imported),
    );
    save_store(&app, &store)?;

    Ok(ImportResult {
        imported_profiles: imported,
        restored_files: restored,
        skipped_conversation_files: skipped_conversations,
        message: "迁移包已导入；选择账号后可写入当前机器的 auth.json".to_string(),
    })
}

fn mesh_file_in_scope(path: &str, scope: &easytier_mesh::MeshSyncScope) -> bool {
    if path == "config.toml" {
        return scope.routing;
    }
    if path == "rules" || path.starts_with("rules/") {
        return scope.rules;
    }
    if is_conversation_path(path) {
        return scope.conversations;
    }
    true
}

#[tauri::command]
fn restore_backup(
    app: AppHandle,
    codex_home: Option<String>,
    backup_path: Option<String>,
) -> Result<String, String> {
    let home = resolve_codex_home(&app, codex_home)?;
    let backup = if let Some(path) = backup_path {
        PathBuf::from(path)
    } else {
        latest_backup(&home).ok_or_else(|| "没有找到 auth.json 备份".to_string())?
    };
    let target = home.join("auth.json");
    fs::copy(&backup, &target).map_err(display_err)?;
    Ok(format!("已恢复备份 {}", backup.to_string_lossy()))
}

#[tauri::command]
fn load_codex_config_files(
    app: AppHandle,
    codex_home: Option<String>,
) -> Result<CodexConfigFiles, String> {
    let home = resolve_codex_home(&app, codex_home)?;
    fs::create_dir_all(&home).map_err(display_err)?;
    let auth_path = home.join("auth.json");
    let config_path = home.join("config.toml");
    Ok(CodexConfigFiles {
        codex_home: home.to_string_lossy().to_string(),
        auth_json: read_config_file_view(&auth_path)?,
        config_toml: read_config_file_view(&config_path)?,
    })
}

#[tauri::command]
fn save_codex_config_file(
    app: AppHandle,
    codex_home: Option<String>,
    file_name: String,
    content: String,
) -> Result<CodexConfigFiles, String> {
    let home = resolve_codex_home(&app, codex_home)?;
    fs::create_dir_all(&home).map_err(display_err)?;
    let formatted = format_codex_config_content(&file_name, &content)?;
    let target = home.join(&file_name);
    let backup = backup_codex_config_file(&target)?;
    replace_file_with_rollback(&target, formatted.as_bytes(), backup.as_deref())?;
    load_codex_config_files(app, Some(home.to_string_lossy().to_string()))
}

#[tauri::command]
fn format_codex_config_file(file_name: String, content: String) -> Result<String, String> {
    format_codex_config_content(&file_name, &content)
}

fn scan_codex_home_path(path: &Path) -> Result<CodexHomeScan, String> {
    let auth_path = path.join("auth.json");
    let current_auth = if auth_path.exists() {
        Some(summarize_auth(
            &fs::read_to_string(&auth_path).map_err(display_err)?,
        )?)
    } else {
        None
    };

    Ok(CodexHomeScan {
        codex_home: path.to_string_lossy().to_string(),
        exists: path.exists(),
        has_auth: auth_path.exists(),
        current_auth,
        migratable: migratable_roots(false)
            .into_iter()
            .map(String::from)
            .collect(),
        excluded: excluded_roots().into_iter().map(String::from).collect(),
    })
}

fn store_view(store: AppStore) -> StoreView {
    StoreView {
        settings: store.settings,
        profiles: store
            .profiles
            .into_iter()
            .map(|p| AccountProfileView {
                id: p.id,
                alias: p.alias,
                note: p.note,
                enabled: p.enabled,
                priority: p.priority,
                cooldown_until: p.cooldown_until,
                quota_rule: p.quota_rule,
                summary: p.summary,
                api_config: p.api_config,
                usage: p.usage,
                route_health: p.route_health,
                created_at: p.created_at,
                updated_at: p.updated_at,
            })
            .collect(),
        events: store.events,
    }
}

fn delete_profile_from_store(store: &mut AppStore, profile_id: &str) -> Result<String, String> {
    let idx = store
        .profiles
        .iter()
        .position(|p| p.id == profile_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let alias = store.profiles[idx].alias.clone();
    store.profiles.remove(idx);
    if store.settings.current_profile_id.as_deref() == Some(profile_id) {
        store.settings.current_profile_id = None;
    }
    Ok(alias)
}

pub(crate) fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(display_err)?;
    migrate_legacy_app_data_dir(&dir)?;
    fs::create_dir_all(&dir).map_err(display_err)?;
    Ok(dir)
}

fn migrate_legacy_app_data_dir(new_dir: &Path) -> Result<(), String> {
    if new_dir.join(STORE_FILE).exists() {
        return Ok(());
    }

    let Some(parent) = new_dir.parent() else {
        return Ok(());
    };
    let legacy_dir = parent.join(LEGACY_APP_IDENTIFIER);
    if !legacy_dir.exists() || legacy_dir == new_dir {
        return Ok(());
    }

    copy_dir_contents_if_missing(&legacy_dir, new_dir)
}

fn copy_dir_contents_if_missing(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(display_err)?;
    for entry in WalkDir::new(from)
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
    {
        let source = entry.path();
        let rel = source.strip_prefix(from).map_err(display_err)?;
        let target = to.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(display_err)?;
        } else if !target.exists() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(display_err)?;
            }
            fs::copy(source, target).map_err(display_err)?;
        }
    }
    Ok(())
}

pub(crate) fn load_store(app: &AppHandle) -> Result<AppStore, String> {
    let _guard = STORE_IO_MUTEX.lock().map_err(display_err)?;
    read_store_unlocked(app)
}

pub(crate) fn save_store(app: &AppHandle, store: &AppStore) -> Result<(), String> {
    let _guard = STORE_IO_MUTEX.lock().map_err(display_err)?;
    write_store_unlocked(app, store)
}

pub(crate) fn mutate_store<T>(
    app: &AppHandle,
    update: impl FnOnce(&mut AppStore) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = STORE_IO_MUTEX.lock().map_err(display_err)?;
    let mut store = read_store_unlocked(app)?;
    let result = update(&mut store)?;
    write_store_unlocked(app, &store)?;
    Ok(result)
}

fn read_store_unlocked(app: &AppHandle) -> Result<AppStore, String> {
    let path = app_data_dir(app)?.join(STORE_FILE);
    if !path.exists() {
        return Ok(AppStore::default());
    }
    let text = fs::read_to_string(path).map_err(display_err)?;
    serde_json::from_str(&text).map_err(display_err)
}

fn write_store_unlocked(app: &AppHandle, store: &AppStore) -> Result<(), String> {
    let path = app_data_dir(app)?.join(STORE_FILE);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(store).map_err(display_err)?;
    fs::write(&tmp, text).map_err(display_err)?;
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
    fs::rename(tmp, path).map_err(display_err)
}

pub(crate) fn load_master_key(app: &AppHandle) -> Result<[u8; 32], String> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        if let Ok(secret) = entry.get_password() {
            if let Ok(bytes) = STANDARD.decode(secret) {
                if bytes.len() == 32 {
                    let mut key = [0u8; 32];
                    key.copy_from_slice(&bytes);
                    return Ok(key);
                }
            }
        }
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        if entry.set_password(&STANDARD.encode(key)).is_ok() {
            return Ok(key);
        }
    }

    let key_path = app_data_dir(app)?.join(LOCAL_KEY_FILE);
    if key_path.exists() {
        let bytes = fs::read(&key_path).map_err(display_err)?;
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
    }
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    fs::write(key_path, key).map_err(display_err)?;
    Ok(key)
}

pub(crate) fn encrypt_secret(plaintext: &[u8], key: &[u8; 32]) -> Result<SecretEnvelope, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(display_err)?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(display_err)?;
    Ok(SecretEnvelope {
        v: 1,
        alg: "AES-256-GCM".to_string(),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    })
}

pub(crate) fn decrypt_secret(envelope: &SecretEnvelope, key: &[u8; 32]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(display_err)?;
    let nonce = STANDARD.decode(&envelope.nonce).map_err(display_err)?;
    let ciphertext = STANDARD.decode(&envelope.ciphertext).map_err(display_err)?;
    cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(display_err)
}

fn encrypt_export(plaintext: &[u8], password: &str) -> Result<ExportEnvelope, String> {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(display_err)?;
    let secret = encrypt_secret(plaintext, &key)?;
    Ok(ExportEnvelope {
        format: EXPORT_FORMAT.to_string(),
        version: EXPORT_VERSION,
        kdf: "argon2id-default".to_string(),
        salt: STANDARD.encode(salt),
        nonce: secret.nonce,
        ciphertext: secret.ciphertext,
    })
}

fn decrypt_export(envelope: ExportEnvelope, password: &str) -> Result<Vec<u8>, String> {
    if envelope.format != EXPORT_FORMAT {
        return Err("不是 Codex Switcher 迁移包".to_string());
    }
    let salt = STANDARD.decode(envelope.salt).map_err(display_err)?;
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(display_err)?;
    decrypt_secret(
        &SecretEnvelope {
            v: envelope.version,
            alg: "AES-256-GCM".to_string(),
            nonce: envelope.nonce,
            ciphertext: envelope.ciphertext,
        },
        &key,
    )
}

pub(crate) fn summarize_auth(auth_json: &str) -> Result<AuthSummary, String> {
    let auth: Value = serde_json::from_str(auth_json).map_err(display_err)?;
    let id_claims = auth
        .pointer("/tokens/id_token")
        .and_then(Value::as_str)
        .and_then(decode_jwt_claims);
    let access_claims = auth
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .and_then(decode_jwt_claims);

    Ok(AuthSummary {
        email: id_claims
            .as_ref()
            .and_then(|v| v.get("email"))
            .and_then(Value::as_str)
            .map(String::from),
        plan: id_claims
            .as_ref()
            .and_then(extract_plan_type_from_claims)
            .or_else(|| {
                access_claims
                    .as_ref()
                    .and_then(extract_plan_type_from_claims)
            }),
        subscription_active_start: id_claims.as_ref().and_then(|claims| {
            extract_subscription_claim(claims, "chatgpt_subscription_active_start")
        }),
        subscription_active_until: id_claims.as_ref().and_then(|claims| {
            extract_subscription_claim(claims, "chatgpt_subscription_active_until")
        }),
        subscription_last_checked: id_claims.as_ref().and_then(|claims| {
            extract_subscription_claim(claims, "chatgpt_subscription_last_checked")
        }),
        account_id: auth
            .pointer("/tokens/account_id")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| {
                access_claims
                    .as_ref()
                    .and_then(extract_account_id_from_claims)
            }),
        user_id: id_claims
            .as_ref()
            .and_then(|v| v.get("chatgpt_user_id").or_else(|| v.get("sub")))
            .and_then(Value::as_str)
            .map(String::from),
        organization_id: id_claims
            .as_ref()
            .and_then(|v| v.get("organization_id"))
            .and_then(Value::as_str)
            .map(String::from),
        access_token_exp: access_claims
            .as_ref()
            .and_then(|v| v.get("exp"))
            .and_then(Value::as_i64),
        id_token_exp: id_claims
            .as_ref()
            .and_then(|v| v.get("exp"))
            .and_then(Value::as_i64),
        auth_mode: auth
            .get("auth_mode")
            .and_then(Value::as_str)
            .map(String::from),
    })
}

fn extract_plan_type_from_claims(claims: &Value) -> Option<String> {
    claims
        .get("chatgpt_plan_type")
        .or_else(|| claims.get("plan_type"))
        .or_else(|| {
            claims.get("https://api.openai.com/auth").and_then(|auth| {
                auth.get("chatgpt_plan_type")
                    .or_else(|| auth.get("plan_type"))
            })
        })
        .and_then(Value::as_str)
        .map(String::from)
}

fn extract_subscription_claim(claims: &Value, key: &str) -> Option<String> {
    claims
        .get(key)
        .or_else(|| {
            claims
                .get("https://api.openai.com/auth")
                .and_then(|auth| auth.get(key))
        })
        .and_then(Value::as_str)
        .map(String::from)
}

fn extract_account_id_from_claims(claims: &Value) -> Option<String> {
    claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| {
            auth.get("chatgpt_account_id")
                .or_else(|| auth.get("account_id"))
        })
        .and_then(Value::as_str)
        .map(String::from)
}

fn decode_jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) async fn refresh_auth_json_with_client(
    client: &reqwest::Client,
    auth_json: &str,
) -> Result<String, String> {
    let auth: Value = serde_json::from_str(auth_json).map_err(display_err)?;
    let refresh_token = auth
        .pointer("/tokens/refresh_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "auth.json missing refresh_token".to_string())?;

    let response = client
        .post("https://auth.openai.com/oauth/token")
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": CHATGPT_OAUTH_CLIENT_ID,
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .map_err(display_err)?;
    let status = response.status();
    let body = response.text().await.map_err(display_err)?;
    if !status.is_success() {
        return Err(format!(
            "token refresh HTTP {}: {}",
            status.as_u16(),
            compact_error_body(&body)
        ));
    }
    let token_response: Value = serde_json::from_str(&body).map_err(display_err)?;
    apply_token_refresh_response(auth_json, &token_response)
}

fn apply_token_refresh_response(auth_json: &str, token_response: &Value) -> Result<String, String> {
    let mut auth: Value = serde_json::from_str(auth_json).map_err(display_err)?;
    let tokens = auth
        .get_mut("tokens")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "auth.json missing tokens object".to_string())?;
    let access_token = token_response
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "refresh response missing access_token".to_string())?;
    tokens.insert(
        "access_token".to_string(),
        Value::String(access_token.to_string()),
    );
    if let Some(id_token) = token_response.get("id_token").and_then(Value::as_str) {
        tokens.insert("id_token".to_string(), Value::String(id_token.to_string()));
    }
    if let Some(refresh_token) = token_response.get("refresh_token").and_then(Value::as_str) {
        tokens.insert(
            "refresh_token".to_string(),
            Value::String(refresh_token.to_string()),
        );
    }
    auth["last_refresh"] = Value::String(now_string());
    serde_json::to_string_pretty(&auth).map_err(display_err)
}

pub(crate) fn should_refresh_access_token(
    access_token_exp: Option<i64>,
    threshold_secs: u64,
) -> bool {
    let Some(exp) = access_token_exp else {
        return true;
    };
    exp <= Utc::now().timestamp() + threshold_secs as i64
}

fn compact_error_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() > 240 {
        format!("{}...", trimmed.chars().take(240).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn refresh_error_requires_relogin(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("refresh_token_reused")
        || normalized.contains("token_invalidated")
        || normalized.contains("invalid_grant")
        || normalized.contains("invalid refresh token")
}

fn normalize_provider_id(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || !normalized
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("Provider ID 仅支持字母、数字、- 和 _".to_string());
    }
    Ok(normalized)
}

fn normalize_api_base_url(value: &str) -> Result<String, String> {
    let normalized = if value.trim().is_empty() {
        DEFAULT_API_BASE_URL.to_string()
    } else {
        value.trim().trim_end_matches('/').to_string()
    };
    let lower = normalized.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err("API Base URL 必须以 http:// 或 https:// 开头".to_string());
    }
    Ok(normalized)
}

fn write_api_provider_config(
    config_path: &Path,
    api_config: &ApiProviderConfig,
    display_name: &str,
    managed_provider_ids: &[String],
) -> Result<(), String> {
    let current = fs::read_to_string(config_path).unwrap_or_default();
    let mut document = if current.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        current
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| format!("config.toml 解析失败: {error}"))?
    };
    document["model"] = toml_edit::value(&api_config.model);
    document["model_provider"] = toml_edit::value(&api_config.provider_id);
    provider_compat::provider_adapter(
        &api_config.provider_id,
        &api_config.base_url,
        &api_config.model,
    )
    .apply_codex_options(&mut document);
    if !document.as_table().contains_key("model_providers")
        || !document["model_providers"].is_table()
    {
        document["model_providers"] = toml_edit::table();
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or_else(|| "model_providers 不是有效表".to_string())?;
    for provider_id in managed_provider_ids {
        if provider_id != &api_config.provider_id {
            providers.remove(provider_id);
        }
    }
    if !providers.contains_key(&api_config.provider_id) {
        providers.insert(&api_config.provider_id, toml_edit::table());
    }
    let provider = providers
        .get_mut(&api_config.provider_id)
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| "Provider 配置不是有效表".to_string())?;
    provider["name"] = toml_edit::value(display_name);
    provider["base_url"] = toml_edit::value(&api_config.base_url);
    provider["wire_api"] = toml_edit::value(&api_config.wire_api);
    provider["requires_openai_auth"] = toml_edit::value(true);
    provider.remove("experimental_bearer_token");
    replace_file_with_rollback(config_path, document.to_string().as_bytes(), None)
}

fn document_uses_longcat_api(document: &toml_edit::DocumentMut) -> bool {
    if document["model"].as_str().is_some_and(is_longcat_model) {
        return true;
    }
    let Some(provider_id) = document["model_provider"].as_str() else {
        return false;
    };
    document["model_providers"][provider_id]["base_url"]
        .as_str()
        .is_some_and(is_longcat_base_url)
}

fn remove_longcat_config_for_non_api_account(config_path: &Path) -> Result<(), String> {
    let current = fs::read_to_string(config_path).unwrap_or_default();
    if current.trim().is_empty() {
        return Ok(());
    }
    let mut document = current
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 瑙ｆ瀽澶辫触: {error}"))?;
    if !document_uses_longcat_api(&document) {
        return Ok(());
    }

    if document["model"].as_str().is_some_and(is_longcat_model) {
        document.as_table_mut().remove("model");
    }
    if let Some(provider_id) = document["model_provider"].as_str().map(str::to_string) {
        if document["model_providers"][&provider_id]["base_url"]
            .as_str()
            .is_some_and(is_longcat_base_url)
        {
            document.as_table_mut().remove("model_provider");
            if let Some(providers) = document["model_providers"].as_table_mut() {
                providers.remove(&provider_id);
            }
        }
    }
    provider_compat::provider_adapter("generic", "", "").apply_codex_options(&mut document);
    replace_file_with_rollback(config_path, document.to_string().as_bytes(), None)
}

fn cleanup_non_api_config_with_timeout(
    app: AppHandle,
    switch_id: String,
    profile_id: String,
    codex_home: PathBuf,
    config_path: PathBuf,
) -> Option<String> {
    append_switch_diagnostic(
        &app,
        &switch_id,
        &profile_id,
        "config_cleanup_start",
        config_path.to_string_lossy(),
    );
    let (sender, receiver) = mpsc::channel();
    let worker_app = app.clone();
    let worker_switch_id = switch_id.clone();
    let worker_profile_id = profile_id.clone();
    thread::spawn(move || {
        append_switch_diagnostic(
            &worker_app,
            &worker_switch_id,
            &worker_profile_id,
            "config_restore_backup_start",
            codex_home.to_string_lossy(),
        );
        let result = restore_api_config_backup(&codex_home)
            .and_then(|_| {
                append_switch_diagnostic(
                    &worker_app,
                    &worker_switch_id,
                    &worker_profile_id,
                    "config_remove_longcat_start",
                    config_path.to_string_lossy(),
                );
                remove_longcat_config_for_non_api_account(&config_path)
            })
            .map(|_| {
                append_switch_diagnostic(
                    &worker_app,
                    &worker_switch_id,
                    &worker_profile_id,
                    "config_cleanup_done",
                    "",
                );
            });
        if let Err(error) = &result {
            append_switch_diagnostic(
                &worker_app,
                &worker_switch_id,
                &worker_profile_id,
                "config_cleanup_failed",
                error,
            );
        }
        let _ = sender.send(result);
    });

    match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("config.toml 清理失败，auth.json 已写入：{error}")),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            append_switch_diagnostic(
                &app,
                &switch_id,
                &profile_id,
                "config_cleanup_timeout",
                "background cleanup exceeded 2s; switch continues",
            );
            Some("config.toml 清理超过 2 秒，已转后台继续；auth.json 已写入".to_string())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            append_switch_diagnostic(
                &app,
                &switch_id,
                &profile_id,
                "config_cleanup_disconnected",
                "background cleanup worker disconnected",
            );
            Some("config.toml 清理线程异常退出，auth.json 已写入".to_string())
        }
    }
}

fn restore_api_config_backup(codex_home: &Path) -> Result<bool, String> {
    let config_path = codex_home.join("config.toml");
    let backup_path = codex_home.join("config.toml.account-switcher.backup");
    if !backup_path.exists() {
        return Ok(false);
    }
    let backup = fs::read(&backup_path).map_err(display_err)?;
    if backup.is_empty() {
        if config_path.exists() {
            fs::remove_file(&config_path).map_err(display_err)?;
        }
    } else {
        replace_file_with_rollback(&config_path, &backup, None)?;
    }
    fs::remove_file(&backup_path).map_err(display_err)?;
    Ok(true)
}

fn clamp_interval(seconds: u64) -> u64 {
    seconds.clamp(30, 86_400)
}

fn clamp_background_token_refresh_interval(seconds: u64) -> u64 {
    seconds.clamp(3_600, 604_800)
}

fn clamp_token_refresh_threshold(seconds: u64) -> u64 {
    seconds.min(2_592_000)
}

fn open_path_in_file_manager(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(display_err)?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(display_err)?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(display_err)?;
        return Ok(());
    }
}

fn open_external_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://localhost:")) {
        return Err("不允许打开该 URL".to_string());
    }
    #[cfg(windows)]
    {
        Command::new("rundll32.exe")
            .arg("url.dll,FileProtocolHandler")
            .arg(url)
            .spawn()
            .map_err(display_err)?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn().map_err(display_err)?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(display_err)?;
    }
    Ok(())
}

fn start_official_cli_login() -> Result<(), String> {
    let mut last_error = None;
    for command in ["codex", "chatgpt"] {
        let mut launch = Command::new(command);
        apply_codex_launch_env(&mut launch, None);
        match launch.arg("login").spawn() {
            Ok(_) => return Ok(()),
            Err(error) => last_error = Some(format!("{command}: {error}")),
        }
    }
    Err(format!(
        "无法启动 codex/chatgpt login，请确认官方 CLI 已安装并在 PATH 中{}",
        last_error
            .map(|error| format!(": {error}"))
            .unwrap_or_default()
    ))
}

pub(crate) fn resolve_codex_home(
    app: &AppHandle,
    explicit: Option<String>,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(compatible_official_home(PathBuf::from(path)));
    }
    if let Some(path) = load_store(app).ok().and_then(|s| s.settings.codex_home) {
        return Ok(compatible_official_home(PathBuf::from(path)));
    }
    default_codex_home().ok_or_else(|| "无法定位用户主目录".to_string())
}

fn default_codex_home() -> Option<PathBuf> {
    let home = user_home_dir()?;
    let codex = home.join(".codex");
    let chatgpt = home.join(".chatgpt");
    if let Some(preferred) = newer_official_home(&codex, &chatgpt) {
        return Some(preferred);
    }
    if codex.join("auth.json").exists() || codex.join("config.toml").exists() || codex.exists() {
        return Some(codex);
    }
    if chatgpt.exists() {
        return Some(chatgpt);
    }
    Some(codex)
}

fn compatible_official_home(path: PathBuf) -> PathBuf {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return path;
    };
    let Some(parent) = path.parent() else {
        return path;
    };
    let alternate = match name.to_ascii_lowercase().as_str() {
        ".codex" => parent.join(".chatgpt"),
        ".chatgpt" => parent.join(".codex"),
        _ => return path,
    };
    if let Some(preferred) = newer_official_home(&path, &alternate) {
        preferred
    } else if !(path.join("auth.json").exists() || path.join("config.toml").exists())
        && (alternate.join("auth.json").exists() || alternate.join("config.toml").exists())
    {
        alternate
    } else {
        path
    }
}

fn newer_official_home(left: &Path, right: &Path) -> Option<PathBuf> {
    let left_time = official_home_activity_time(left);
    let right_time = official_home_activity_time(right);
    match (left_time, right_time) {
        (Some(left_time), Some(right_time)) if right_time > left_time => Some(right.to_path_buf()),
        (Some(_), Some(_)) => Some(left.to_path_buf()),
        (Some(_), None) => Some(left.to_path_buf()),
        (None, Some(_)) => Some(right.to_path_buf()),
        (None, None) => None,
    }
}

fn official_home_activity_time(path: &Path) -> Option<std::time::SystemTime> {
    ["auth.json", "config.toml"]
        .iter()
        .filter_map(|name| {
            fs::metadata(path.join(name))
                .ok()
                .and_then(|metadata| metadata.modified().ok())
        })
        .max()
}

fn user_home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn backup_auth_file(auth_path: &Path) -> Result<Option<PathBuf>, String> {
    if !auth_path.exists() {
        return Ok(None);
    }
    let backup_dir = auth_path
        .parent()
        .ok_or_else(|| "auth.json 路径无父目录".to_string())?
        .join(".account-switcher-backups");
    fs::create_dir_all(&backup_dir).map_err(display_err)?;
    let backup = backup_dir.join(format!("auth-{}.json", Utc::now().format("%Y%m%d%H%M%S")));
    fs::copy(auth_path, &backup).map_err(display_err)?;
    Ok(Some(backup))
}

fn backup_codex_config_file(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "配置文件名无效".to_string())?;
    let backup_dir = path
        .parent()
        .ok_or_else(|| "配置文件路径无父目录".to_string())?
        .join(".account-switcher-backups");
    fs::create_dir_all(&backup_dir).map_err(display_err)?;
    let backup = backup_dir.join(format!(
        "{}-{}-{}",
        name,
        Utc::now().format("%Y%m%d%H%M%S"),
        Uuid::new_v4()
    ));
    fs::copy(path, &backup).map_err(display_err)?;
    Ok(Some(backup))
}

fn read_config_file_view(path: &Path) -> Result<ConfigFileView, String> {
    let content = if path.exists() {
        fs::read_to_string(path).map_err(display_err)?
    } else {
        String::new()
    };
    Ok(ConfigFileView {
        path: path.to_string_lossy().to_string(),
        exists: path.exists(),
        content,
    })
}

fn format_codex_config_content(file_name: &str, content: &str) -> Result<String, String> {
    match file_name {
        "auth.json" => {
            let value: Value = serde_json::from_str(content)
                .map_err(|error| format!("auth.json JSON 解析失败: {error}"))?;
            let formatted = serde_json::to_string_pretty(&value).map_err(display_err)?;
            Ok(format!("{formatted}\n"))
        }
        "config.toml" => {
            let document = content
                .parse::<toml_edit::DocumentMut>()
                .map_err(|error| format!("config.toml TOML 解析失败: {error}"))?;
            Ok(document.to_string())
        }
        _ => Err("仅支持编辑 auth.json 和 config.toml".to_string()),
    }
}

pub(crate) fn replace_file_with_rollback(
    target: &Path,
    bytes: &[u8],
    backup_path: Option<&Path>,
) -> Result<(), String> {
    let tmp = target.with_extension(format!("json.tmp-{}", Uuid::new_v4()));
    fs::write(&tmp, bytes).map_err(display_err)?;
    if target.exists() {
        fs::remove_file(target).map_err(display_err)?;
    }
    if let Err(err) = fs::rename(&tmp, target) {
        if let Some(backup) = backup_path {
            let _ = fs::copy(backup, target);
        }
        return Err(err.to_string());
    }
    Ok(())
}

fn verify_auth_json_written(auth_path: &Path, expected_auth_json: &str) -> Result<(), String> {
    let actual = fs::read(auth_path).map_err(|error| {
        format!(
            "无法读取切换后的 auth.json {}: {error}",
            auth_path.display()
        )
    })?;
    if actual != expected_auth_json.as_bytes() {
        return Err(format!(
            "auth.json 写入校验失败，目标文件未保持为所选账号：{}",
            auth_path.display()
        ));
    }
    Ok(())
}

fn latest_backup(codex_home: &Path) -> Option<PathBuf> {
    let dir = codex_home.join(".account-switcher-backups");
    let mut entries = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    entries.sort();
    entries.pop()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexLaunchKind {
    Desktop,
    Cli,
}

#[derive(Debug, Clone)]
struct CodexProcessInfo {
    pid: u32,
    executable_path: Option<PathBuf>,
    launch_kind: CodexLaunchKind,
}

#[derive(Debug, Clone, Default)]
struct CodexProcessSnapshot {
    processes: Vec<CodexProcessInfo>,
}

impl CodexProcessSnapshot {
    fn is_running(&self) -> bool {
        !self.processes.is_empty()
    }

    fn launch_kind(&self) -> Option<CodexLaunchKind> {
        self.processes.first().map(|process| process.launch_kind)
    }

    fn executable_path(&self) -> Option<&Path> {
        self.processes
            .iter()
            .find_map(|process| process.executable_path.as_deref())
    }
}

#[derive(Debug, Default)]
struct CodexRelaunchGuard {
    snapshot: Option<CodexProcessSnapshot>,
    proxy: Option<ProxySettings>,
}

impl CodexRelaunchGuard {
    fn arm(&mut self, snapshot: CodexProcessSnapshot, proxy: Option<ProxySettings>) {
        self.snapshot = Some(snapshot);
        self.proxy = proxy;
    }

    fn relaunch(&mut self) -> Result<(), String> {
        let Some(snapshot) = self.snapshot.take() else {
            return Ok(());
        };
        relaunch_codex_processes(&snapshot, self.proxy.as_ref())
    }
}

impl Drop for CodexRelaunchGuard {
    fn drop(&mut self) {
        if let Some(snapshot) = self.snapshot.take() {
            let _ = relaunch_codex_processes(&snapshot, self.proxy.as_ref());
        }
    }
}

fn current_executable_marker() -> String {
    std::env::current_exe()
        .ok()
        .map(|path| path.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_own_switcher_process(text: &str, current_exe: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if !current_exe.is_empty() && lower.contains(current_exe) {
        return true;
    }
    [
        "codexswitcher",
        "codex switcher",
        "codex-account-switcher",
        "codex_account_switcher",
        "local.codex.account-switcher",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn path_ends_with_client_binary(path: &str, binary: &str) -> bool {
    path.ends_with(&format!("/{binary}")) || path.ends_with(&format!("\\{binary}.exe"))
}

fn classify_codex_process(
    name: &str,
    executable_path: Option<&str>,
    command_line: &str,
    current_exe: &str,
) -> Option<CodexLaunchKind> {
    let path = executable_path.unwrap_or("");
    let combined = format!("{name}\n{path}\n{command_line}");
    if is_own_switcher_process(&combined, current_exe) {
        return None;
    }

    let name_lower = name.to_ascii_lowercase();
    let path_lower = path.to_ascii_lowercase();
    let command_lower = command_line.to_ascii_lowercase();
    let explicit_cli = command_lower.contains("@openai/codex")
        || command_lower.contains("@openai/chatgpt")
        || command_lower.contains("codex.js")
        || command_lower.contains("chatgpt.js")
        || command_lower.contains("/bin/codex")
        || command_lower.contains("/bin/chatgpt")
        || command_lower.contains("\\bin\\codex")
        || command_lower.contains("\\bin\\chatgpt")
        || path_lower.contains("node_modules")
        || path_lower.contains("@openai/codex")
        || path_lower.contains("@openai/chatgpt");
    let is_desktop_binary = path_lower.contains(".app/contents/macos/codex")
        || path_lower.contains(".app/contents/macos/chatgpt")
        || path_lower.contains("openai.codex")
        || path_lower.contains("openai.chatgpt")
        || (cfg!(windows)
            && (name_lower == "codex.exe" || name_lower == "chatgpt.exe")
            && !explicit_cli);
    let is_desktop_main = is_desktop_binary
        && !path_lower.contains("\\resources\\")
        && !path_lower.contains("/resources/")
        && !command_lower.contains("--type=")
        && !command_lower.contains("--utility-sub-type=");
    if is_desktop_main {
        return Some(CodexLaunchKind::Desktop);
    }
    if is_desktop_binary {
        return None;
    }

    let is_cli = explicit_cli
        || path_ends_with_client_binary(&path_lower, "codex")
        || path_ends_with_client_binary(&path_lower, "chatgpt")
        || command_lower.starts_with("codex ")
        || command_lower.starts_with("chatgpt ");
    is_cli.then_some(CodexLaunchKind::Cli)
}

#[cfg(windows)]
fn collect_codex_process_snapshot() -> CodexProcessSnapshot {
    let current_exe = current_executable_marker();
    let script = r#"Get-CimInstance Win32_Process |
Where-Object { $_.Name -match '(?i)(codex|chatgpt)' -or $_.ExecutablePath -match '(?i)(OpenAI\.(Codex|ChatGPT)|@openai[\\/](codex|chatgpt))' -or $_.CommandLine -match '(?i)(@openai[\\/](codex|chatgpt)|(codex|chatgpt)\.js|[\\/]bin[\\/](codex|chatgpt))' } |
Select-Object ProcessId,Name,ExecutablePath,CommandLine |
ConvertTo-Json -Compress"#;
    let output = hidden_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            let parsed = parse_windows_codex_process_json(
                String::from_utf8_lossy(&output.stdout).trim(),
                &current_exe,
            );
            if !parsed.processes.is_empty() {
                return parsed;
            }
        }
    }

    let output = hidden_command("wmic")
        .args([
            "process",
            "get",
            "ProcessId,Name,ExecutablePath,CommandLine",
            "/FORMAT:LIST",
        ])
        .output();
    let processes = output
        .ok()
        .map(|output| {
            codex_process_ids_from_wmic_output(
                &String::from_utf8_lossy(&output.stdout),
                &current_exe,
            )
        })
        .unwrap_or_default()
        .into_iter()
        .map(|pid| CodexProcessInfo {
            pid,
            executable_path: None,
            launch_kind: CodexLaunchKind::Cli,
        })
        .collect();
    CodexProcessSnapshot { processes }
}

#[cfg(windows)]
fn parse_windows_codex_process_json(output: &str, current_exe: &str) -> CodexProcessSnapshot {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return CodexProcessSnapshot::default();
    };
    let values = match value {
        Value::Array(values) => values,
        value => vec![value],
    };
    let mut desktop = Vec::new();
    let mut cli = Vec::new();
    for value in values {
        let Some(pid) = value
            .get("ProcessId")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
        else {
            continue;
        };
        let name = value.get("Name").and_then(Value::as_str).unwrap_or("");
        let executable_path = value
            .get("ExecutablePath")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty());
        let command_line = value
            .get("CommandLine")
            .and_then(Value::as_str)
            .unwrap_or("");
        let Some(launch_kind) =
            classify_codex_process(name, executable_path, command_line, current_exe)
        else {
            continue;
        };
        let info = CodexProcessInfo {
            pid,
            executable_path: executable_path.map(PathBuf::from),
            launch_kind,
        };
        match launch_kind {
            CodexLaunchKind::Desktop => desktop.push(info),
            CodexLaunchKind::Cli => cli.push(info),
        }
    }
    CodexProcessSnapshot {
        processes: if desktop.is_empty() { cli } else { desktop },
    }
}

#[cfg(not(windows))]
fn collect_codex_process_snapshot() -> CodexProcessSnapshot {
    let current_exe = current_executable_marker();
    let output = Command::new("ps")
        .args(["-axww", "-o", "pid=,comm=,command="])
        .output();
    let mut desktop = Vec::new();
    let mut cli = Vec::new();
    for line in output
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
        .lines()
    {
        let mut parts = line.trim().splitn(3, char::is_whitespace);
        let Some(pid) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let name = parts.next().unwrap_or("");
        let command_line = parts.next().unwrap_or("");
        let executable = command_line.split_whitespace().next();
        let Some(launch_kind) =
            classify_codex_process(name, executable, command_line, &current_exe)
        else {
            continue;
        };
        let info = CodexProcessInfo {
            pid,
            executable_path: executable.map(PathBuf::from),
            launch_kind,
        };
        match launch_kind {
            CodexLaunchKind::Desktop => desktop.push(info),
            CodexLaunchKind::Cli => cli.push(info),
        }
    }
    CodexProcessSnapshot {
        processes: if desktop.is_empty() { cli } else { desktop },
    }
}

fn is_codex_running() -> bool {
    collect_codex_process_snapshot().is_running()
}

#[cfg(not(windows))]
fn process_is_running(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn wait_for_processes_exit(pids: &[u32], timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if pids.iter().all(|pid| !process_is_running(*pid)) {
            return true;
        }
        thread::sleep(Duration::from_millis(150));
    }
    pids.iter().all(|pid| !process_is_running(*pid))
}

#[cfg(windows)]
fn relaunch_official_cli(
    snapshot: &CodexProcessSnapshot,
    proxy: Option<&ProxySettings>,
) -> Result<(), String> {
    if let Some(path) = snapshot.executable_path().filter(|path| path.exists()) {
        let mut command = Command::new(path);
        apply_codex_launch_env(&mut command, proxy);
        command
            .spawn()
            .map_err(|error| format!("无法重新启动 {OFFICIAL_CLIENT_DISPLAY_NAME} CLI: {error}"))?;
        return Ok(());
    }
    let mut last_error = None;
    for command in ["codex", "chatgpt"] {
        let mut launch = hidden_command("cmd");
        apply_codex_launch_env(&mut launch, proxy);
        match launch.args(["/C", "start", "", command]).spawn() {
            Ok(_) => return Ok(()),
            Err(error) => last_error = Some(format!("{command}: {error}")),
        }
    }
    Err(format!(
        "无法重新启动 {OFFICIAL_CLIENT_DISPLAY_NAME} CLI，请确认 codex 或 chatgpt 已安装并在 PATH 中{}",
        last_error
            .map(|error| format!(": {error}"))
            .unwrap_or_default()
    ))
}

#[cfg(not(windows))]
fn relaunch_official_cli(
    snapshot: &CodexProcessSnapshot,
    proxy: Option<&ProxySettings>,
) -> Result<(), String> {
    if let Some(path) = snapshot.executable_path().filter(|path| path.exists()) {
        let mut command = Command::new(path);
        apply_codex_launch_env(&mut command, proxy);
        command
            .spawn()
            .map_err(|error| format!("无法重新启动 {OFFICIAL_CLIENT_DISPLAY_NAME} CLI: {error}"))?;
        return Ok(());
    }
    let mut last_error = None;
    for command in ["codex", "chatgpt"] {
        let mut launch = Command::new(command);
        apply_codex_launch_env(&mut launch, proxy);
        match launch.spawn() {
            Ok(_) => return Ok(()),
            Err(error) => last_error = Some(format!("{command}: {error}")),
        }
    }
    Err(format!(
        "无法重新启动 {OFFICIAL_CLIENT_DISPLAY_NAME} CLI，请确认 codex 或 chatgpt 已安装并在 PATH 中{}",
        last_error
            .map(|error| format!(": {error}"))
            .unwrap_or_default()
    ))
}

#[cfg(windows)]
fn terminate_codex_processes(snapshot: &CodexProcessSnapshot) -> Result<(), String> {
    for process in &snapshot.processes {
        hidden_command("taskkill")
            .args(["/PID", &process.pid.to_string(), "/T", "/F"])
            .spawn()
            .map_err(display_err)?;
    }
    thread::sleep(Duration::from_millis(1200));
    Ok(())
}

#[cfg(not(windows))]
fn terminate_codex_processes(snapshot: &CodexProcessSnapshot) -> Result<(), String> {
    let pids = snapshot
        .processes
        .iter()
        .map(|process| process.pid)
        .collect::<Vec<_>>();
    for pid in &pids {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
    if wait_for_processes_exit(&pids, Duration::from_secs(3)) {
        return Ok(());
    }
    for pid in &pids {
        if process_is_running(*pid) {
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        }
    }
    if wait_for_processes_exit(&pids, Duration::from_secs(5)) {
        Ok(())
    } else {
        Err("Codex 进程未能完全退出，请手动关闭后重试".to_string())
    }
}

#[cfg(windows)]
fn windows_codex_app_user_model_id() -> Option<String> {
    let script = r#"$entry = Get-StartApps |
Where-Object { $_.Name -match '(?i)^(Codex|ChatGPT)$' -or $_.AppID -match '(?i)OpenAI\.(Codex|ChatGPT)' } |
Select-Object -First 1
if ($entry) { Write-Output $entry.AppID }"#;
    let output = hidden_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(String::from)
}

#[cfg(windows)]
fn relaunch_codex_processes(
    snapshot: &CodexProcessSnapshot,
    proxy: Option<&ProxySettings>,
) -> Result<(), String> {
    match snapshot.launch_kind() {
        Some(CodexLaunchKind::Desktop) => {
            enable_windows_system_proxy_temporarily(proxy, Duration::from_secs(45))?;
            if let Some(path) = snapshot.executable_path().filter(|path| path.exists()) {
                let mut command = Command::new(path);
                apply_codex_launch_env(&mut command, proxy);
                apply_codex_desktop_launch_args(&mut command, proxy);
                match command.spawn() {
                    Ok(_) => return Ok(()),
                    Err(error) => {
                        let fallback_error =
                            format!("无法通过已捕获路径启动 Codex 桌面应用: {error}");
                        if let Some(app_id) = windows_codex_app_user_model_id() {
                            let mut command = Command::new("explorer.exe");
                            apply_codex_launch_env(&mut command, proxy);
                            command
                                .arg(format!("shell:AppsFolder\\{app_id}"))
                                .spawn()
                                .map_err(|error| {
                                    format!(
                                        "{fallback_error}；且无法通过 Windows 应用入口启动 Codex: {error}"
                                    )
                                })?;
                            return Ok(());
                        }
                        return Err(fallback_error);
                    }
                }
            }
            if let Some(app_id) = windows_codex_app_user_model_id() {
                let mut command = Command::new("explorer.exe");
                apply_codex_launch_env(&mut command, proxy);
                command
                    .arg(format!("shell:AppsFolder\\{app_id}"))
                    .spawn()
                    .map_err(|error| format!("无法通过 Windows 应用入口启动 Codex: {error}"))?;
                return Ok(());
            }
            Err("未找到 Codex 桌面应用启动入口".to_string())
        }
        Some(CodexLaunchKind::Cli) => relaunch_official_cli(snapshot, proxy),
        None => Ok(()),
    }
}

#[cfg(target_os = "macos")]
fn relaunch_codex_processes(
    snapshot: &CodexProcessSnapshot,
    proxy: Option<&ProxySettings>,
) -> Result<(), String> {
    match snapshot.launch_kind() {
        Some(CodexLaunchKind::Desktop) => {
            let mut command = Command::new("open");
            apply_codex_launch_env(&mut command, proxy);
            command.args(["-a", "Codex"]);
            if chromium_proxy_url(proxy).is_some() {
                command.arg("--args");
            }
            apply_codex_desktop_launch_args(&mut command, proxy);
            let status = command.status().map_err(display_err)?;
            if status.success() {
                Ok(())
            } else {
                let mut command = Command::new("open");
                apply_codex_launch_env(&mut command, proxy);
                command.args(["-a", "ChatGPT"]);
                if chromium_proxy_url(proxy).is_some() {
                    command.arg("--args");
                }
                apply_codex_desktop_launch_args(&mut command, proxy);
                let status = command.status().map_err(display_err)?;
                if status.success() {
                    Ok(())
                } else {
                    Err("无法重新启动 Codex.app 或 ChatGPT.app".to_string())
                }
            }
        }
        Some(CodexLaunchKind::Cli) => relaunch_official_cli(snapshot, proxy),
        None => Ok(()),
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn relaunch_codex_processes(
    snapshot: &CodexProcessSnapshot,
    proxy: Option<&ProxySettings>,
) -> Result<(), String> {
    relaunch_official_cli(snapshot, proxy)
}

fn codex_process_ids_from_wmic_output(output: &str, current_exe: &str) -> Vec<u32> {
    let mut ids = Vec::new();
    let mut block = Vec::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            if let Some(pid) = codex_process_id_from_wmic_block(&block.join("\n"), current_exe) {
                ids.push(pid);
            }
            block.clear();
        } else {
            block.push(line);
        }
    }
    if let Some(pid) = codex_process_id_from_wmic_block(&block.join("\n"), current_exe) {
        ids.push(pid);
    }
    ids
}

fn codex_process_id_from_wmic_block(block: &str, current_exe: &str) -> Option<u32> {
    let pid = block.lines().find_map(|line| {
        line.strip_prefix("ProcessId=")
            .and_then(|value| value.trim().parse::<u32>().ok())
    })?;
    if looks_like_codex_process(block, current_exe) {
        Some(pid)
    } else {
        None
    }
}

fn looks_like_codex_process(line: &str, current_exe: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if !(lower.contains("codex") || lower.contains("chatgpt")) {
        return false;
    }
    if !current_exe.is_empty() && lower.contains(current_exe) {
        return false;
    }
    let own_process_markers = [
        "codexswitcher",
        "codex switcher",
        "codex-account-switcher",
        "codex_account_switcher",
        "local.codex.account-switcher",
    ];
    if own_process_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return false;
    }
    true
}

fn collect_bundle_files(
    codex_home: &Path,
    include_conversations: bool,
) -> Result<Vec<BundleFile>, String> {
    let mut files = Vec::new();
    let roots = migratable_roots(include_conversations);
    for root in roots {
        let path = codex_home.join(root);
        if !path.exists() {
            continue;
        }
        if path.is_file() {
            push_bundle_file(codex_home, &path, &mut files)?;
        } else {
            for entry in WalkDir::new(&path).into_iter().filter_map(Result::ok) {
                let p = entry.path();
                if p.is_file() {
                    push_bundle_file(codex_home, p, &mut files)?;
                }
            }
        }
    }
    Ok(files)
}

fn push_bundle_file(
    codex_home: &Path,
    path: &Path,
    files: &mut Vec<BundleFile>,
) -> Result<(), String> {
    let rel = path.strip_prefix(codex_home).map_err(display_err)?;
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    if is_excluded_path(&rel_str) {
        return Ok(());
    }
    let bytes = fs::read(path).map_err(display_err)?;
    files.push(BundleFile {
        path: rel_str,
        sha256: hex_sha256(&bytes),
        bytes_base64: STANDARD.encode(bytes),
    });
    Ok(())
}

fn migratable_roots(include_conversations: bool) -> Vec<&'static str> {
    let mut roots = vec!["config.toml", "rules", "memories"];
    if include_conversations {
        roots.extend([
            "sessions",
            "archived_sessions",
            "session_index.jsonl",
            "sqlite",
            "logs_2.sqlite",
            "logs_2.sqlite-shm",
            "logs_2.sqlite-wal",
            "state_5.sqlite",
            "state_5.sqlite-shm",
            "state_5.sqlite-wal",
        ]);
    }
    roots
}

fn excluded_roots() -> Vec<&'static str> {
    vec![
        "installation_id",
        "cap_sid",
        ".sandbox",
        ".sandbox-bin",
        ".sandbox-secrets",
        ".tmp",
        "tmp",
        "sandbox.log",
        "log",
    ]
}

fn is_excluded_path(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or(path);
    excluded_roots().contains(&first)
}

fn is_conversation_path(path: &str) -> bool {
    codex_sessions::is_conversation_metadata_path(path)
}

fn safe_relative_path(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if p.is_absolute() || path.contains("..") {
        return Err(format!("迁移包包含不安全路径: {}", path));
    }
    Ok(p)
}

fn zip_payload(payload: &BundlePayload) -> Result<Vec<u8>, String> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    writer
        .start_file("bundle.json", options)
        .map_err(display_err)?;
    let json = serde_json::to_vec(payload).map_err(display_err)?;
    writer.write_all(&json).map_err(display_err)?;
    let cursor = writer.finish().map_err(display_err)?;
    Ok(cursor.into_inner())
}

fn read_bundle(path: &str, password: &str) -> Result<BundlePayload, String> {
    let bytes = fs::read(path).map_err(display_err)?;
    if let Ok(payload) = read_payload_zip(&bytes) {
        return Ok(payload);
    }

    let envelope: ExportEnvelope = serde_json::from_slice(&bytes)
        .map_err(|_| "迁移包格式不正确，既不是明文 zip，也不是加密迁移包".to_string())?;
    if password.is_empty() {
        return Err("该迁移包已加密，请输入迁移包口令".to_string());
    }
    let zip_bytes = decrypt_export(envelope, password)?;
    read_payload_zip(&zip_bytes)
}

fn read_payload_zip(zip_bytes: &[u8]) -> Result<BundlePayload, String> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes.to_vec())).map_err(display_err)?;
    let mut file = archive.by_name("bundle.json").map_err(display_err)?;
    let mut json = Vec::new();
    file.read_to_end(&mut json).map_err(display_err)?;
    let payload: BundlePayload = serde_json::from_slice(&json).map_err(display_err)?;
    if payload.manifest.format != EXPORT_FORMAT || payload.manifest.version != EXPORT_VERSION {
        return Err("迁移包版本不兼容".to_string());
    }
    Ok(payload)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

fn apply_usage_probe_body(profile: &mut AccountProfile, body: &Value) {
    if let Some(plan_type) = body
        .get("plan_type")
        .or_else(|| body.get("planType"))
        .and_then(Value::as_str)
    {
        profile.summary.plan = Some(plan_type.to_string());
    }
    profile.usage.available_reset_count = body
        .pointer("/rate_limit_reset_credits/available_count")
        .or_else(|| body.pointer("/rateLimitResetCredits/availableCount"))
        .and_then(Value::as_i64);
    profile.usage.available_reset_expires_at = pick_reset_credit_expires_at(body);
    let detected = detect_usage_limits(body);
    if detected.is_empty() {
        profile.usage.detected_summary = Some(summarize_usage_body(body));
        return;
    }

    for item in &detected {
        if is_hour_window(&item.window) {
            if let Some(used) = item.used {
                profile.usage.hourly_used = used;
            } else if let (Some(limit), Some(remaining)) = (item.limit, item.remaining) {
                profile.usage.hourly_used = limit.saturating_sub(remaining);
            }
            if item.limit.is_some() {
                profile.quota_rule.hourly_limit = item.limit;
            }
        }

        if is_day_window(&item.window) {
            if let Some(used) = item.used {
                profile.usage.daily_used = used;
            } else if let (Some(limit), Some(remaining)) = (item.limit, item.remaining) {
                profile.usage.daily_used = limit.saturating_sub(remaining);
            }
            if item.limit.is_some() {
                profile.quota_rule.daily_limit = item.limit;
            }
        }

        if profile.usage.estimated_reset_at.is_none() {
            profile.usage.estimated_reset_at = item.reset_at.clone();
        }
    }

    profile.usage.detected_summary = Some(format_detected_limits(&detected));
    profile.usage.detected_limits = detected;
}

fn detect_usage_limits(body: &Value) -> Vec<DetectedLimit> {
    let mut out = Vec::new();
    collect_codex_rate_limit_windows(body, &mut out);
    collect_usage_limits(body, "", &mut out);
    dedupe_detected_limits(out)
}

fn collect_codex_rate_limit_windows(body: &Value, out: &mut Vec<DetectedLimit>) {
    let Some(rate_limit) = body.get("rate_limit").and_then(Value::as_object) else {
        return;
    };

    for (key, label) in [
        ("primary_window", "primary"),
        ("secondary_window", "secondary"),
    ] {
        let Some(window) = rate_limit.get(key).and_then(Value::as_object) else {
            continue;
        };
        let window_seconds = pick_u32(window, &["limit_window_seconds"]);
        let used_percent = pick_u32(window, &["used_percent"]);
        let remaining_percent = used_percent.map(|value| 100_u32.saturating_sub(value.min(100)));
        let reset_at = pick_reset_at(window);
        let window_name = window_seconds
            .map(window_name_from_seconds)
            .unwrap_or_else(|| label.to_string());

        out.push(DetectedLimit {
            label: Some(window_name.clone()),
            window: window_name,
            used: None,
            limit: None,
            remaining: None,
            used_percent,
            remaining_percent,
            reset_at,
        });
    }
}

fn collect_usage_limits(value: &Value, path: &str, out: &mut Vec<DetectedLimit>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_usage_limits(item, path, out);
            }
        }
        Value::Object(map) => {
            let window = detect_window_name(map, path);
            let limit = pick_u32(
                map,
                &["limit", "quota", "max", "total", "cap", "limit_amount"],
            );
            let remaining = pick_u32(map, &["remaining", "available", "left", "remaining_amount"]);
            let used = pick_u32(
                map,
                &[
                    "used",
                    "usage",
                    "consumed",
                    "current",
                    "count",
                    "used_amount",
                ],
            )
            .or_else(|| {
                limit
                    .zip(remaining)
                    .map(|(limit, remaining)| limit.saturating_sub(remaining))
            });
            let reset_at = pick_string(
                map,
                &[
                    "reset_at",
                    "resets_at",
                    "resetAfter",
                    "reset_after",
                    "next_reset_at",
                ],
            );
            let label = pick_string(
                map,
                &["name", "label", "type", "bucket", "model", "category"],
            );

            if (limit.is_some() || remaining.is_some() || used.is_some()) && window.is_some() {
                out.push(DetectedLimit {
                    window: window.unwrap(),
                    used,
                    limit,
                    remaining,
                    used_percent: pick_u32(map, &["used_percent", "usage_percent"]),
                    remaining_percent: pick_u32(map, &["remaining_percent", "available_percent"]),
                    reset_at,
                    label,
                });
            }

            for (key, child) in map {
                let next_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                collect_usage_limits(child, &next_path, out);
            }
        }
        _ => {}
    }
}

fn detect_window_name(map: &serde_json::Map<String, Value>, path: &str) -> Option<String> {
    for key in [
        "window",
        "period",
        "bucket",
        "duration",
        "interval",
        "reset_period",
        "timeframe",
    ] {
        if let Some(value) = map.get(key).and_then(Value::as_str) {
            return Some(normalize_window(value));
        }
    }

    let haystack = path.to_ascii_lowercase();
    if haystack.contains("hour") || haystack.contains("3h") || haystack.contains("3_h") {
        return Some("hour".to_string());
    }
    if haystack.contains("day") || haystack.contains("daily") || haystack.contains("24h") {
        return Some("day".to_string());
    }
    if haystack.contains("week") || haystack.contains("7d") {
        return Some("week".to_string());
    }
    None
}

fn normalize_window(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("hour") || lower.contains("3h") || lower == "h" {
        "hour".to_string()
    } else if lower.contains("day") || lower.contains("24h") || lower == "d" {
        "day".to_string()
    } else if lower.contains("week") || lower.contains("7d") || lower == "w" {
        "week".to_string()
    } else {
        lower
    }
}

fn window_name_from_seconds(seconds: u32) -> String {
    match seconds {
        0..=5400 => format!("{}分钟", (seconds / 60).max(1)),
        5401..=172800 => {
            let hours = ((seconds as f64) / 3600.0).round() as u32;
            format!("{hours}小时")
        }
        172801..=1_209_600 => {
            let days = ((seconds as f64) / 86_400.0).round() as u32;
            if days == 7 {
                "1周".to_string()
            } else {
                format!("{days}天")
            }
        }
        _ => format!("{}天", (seconds / 86_400).max(1)),
    }
}

fn is_hour_window(window: &str) -> bool {
    let lower = window.to_ascii_lowercase();
    lower.contains("hour") || lower.contains("3h") || lower == "h" || lower.contains("小时")
}

fn is_day_window(window: &str) -> bool {
    let lower = window.to_ascii_lowercase();
    lower.contains("day") || lower.contains("24h") || lower == "d" || lower.contains("天")
}

fn pick_u32(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u32> {
    for expected in keys {
        for (key, value) in map {
            if key.eq_ignore_ascii_case(expected) {
                if let Some(parsed) = value_to_u32(value) {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

fn value_to_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .or_else(|| value.as_str().and_then(|v| v.parse::<u32>().ok()))
}

fn pick_string(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for expected in keys {
        for (key, value) in map {
            if key.eq_ignore_ascii_case(expected) {
                if let Some(text) = value.as_str() {
                    return Some(text.to_string());
                }
                if value.is_number() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn pick_reset_at(map: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(text) = pick_string(
        map,
        &[
            "reset_at",
            "resets_at",
            "resetAfter",
            "reset_after",
            "next_reset_at",
        ],
    ) {
        if let Ok(epoch) = text.parse::<i64>() {
            return Utc
                .timestamp_opt(epoch, 0)
                .single()
                .map(|dt| dt.to_rfc3339());
        }
        return Some(text);
    }
    None
}

fn pick_reset_credit_expires_at(body: &Value) -> Option<String> {
    let credits = body
        .pointer("/rate_limit_reset_credits")
        .or_else(|| body.pointer("/rateLimitResetCredits"))?;
    let map = credits.as_object()?;
    let text = pick_string(
        map,
        &[
            "expires_at",
            "expiresAt",
            "expire_at",
            "expireAt",
            "expiration",
            "expiration_at",
            "expirationAt",
            "valid_until",
            "validUntil",
            "reset_expires_at",
            "resetExpiresAt",
        ],
    )?;
    if let Ok(epoch) = text.parse::<i64>() {
        return Utc
            .timestamp_opt(epoch, 0)
            .single()
            .map(|dt| dt.to_rfc3339());
    }
    Some(text)
}

fn dedupe_detected_limits(items: Vec<DetectedLimit>) -> Vec<DetectedLimit> {
    let mut deduped: Vec<DetectedLimit> = Vec::new();
    for item in items {
        let exists = deduped.iter().any(|existing| {
            existing.window == item.window
                && existing.label == item.label
                && existing.limit == item.limit
                && existing.used == item.used
                && existing.remaining == item.remaining
                && existing.used_percent == item.used_percent
                && existing.remaining_percent == item.remaining_percent
        });
        if !exists {
            deduped.push(item);
        }
    }
    deduped
}

fn format_detected_limits(items: &[DetectedLimit]) -> String {
    items
        .iter()
        .map(|item| {
            let label = item.label.as_deref().unwrap_or(&item.window);
            if let Some(remaining_percent) = item.remaining_percent {
                let reset = item
                    .reset_at
                    .as_deref()
                    .map(format_short_time)
                    .unwrap_or_else(|| "-".to_string());
                format!("{label}: 剩余 {remaining_percent}% · {reset}")
            } else {
                let used = item
                    .used
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".to_string());
                let limit = item
                    .limit
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".to_string());
                format!("{label}: {used}/{limit}")
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_short_time(value: &str) -> String {
    parse_time(value)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|| value.to_string())
}

fn summarize_usage_body(body: &Value) -> String {
    let text = serde_json::to_string(body).unwrap_or_default();
    if text.chars().count() > 240 {
        format!(
            "unparsed: {}...",
            text.chars().take(240).collect::<String>()
        )
    } else {
        format!("unparsed: {text}")
    }
}

pub(crate) fn push_event(store: &mut AppStore, level: &str, message: &str) {
    store.events.insert(
        0,
        AppEvent {
            ts: now_string(),
            level: level.to_string(),
            message: message.to_string(),
        },
    );
    store.events.truncate(100);
}

fn append_switch_diagnostic(
    app: &AppHandle,
    switch_id: &str,
    profile_id: &str,
    stage: &str,
    detail: impl AsRef<str>,
) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let _ = fs::create_dir_all(&dir);
    let line = serde_json::json!({
        "ts": now_string(),
        "switchId": switch_id,
        "profileId": profile_id,
        "stage": stage,
        "detail": detail.as_ref(),
    });
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(SWITCH_DIAGNOSTICS_FILE))
    {
        let _ = writeln!(file, "{line}");
    }
}

pub(crate) fn now_string() -> String {
    Utc::now().to_rfc3339()
}

pub(crate) fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub(crate) fn display_err<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            setup_tray(app)?;
            if let Ok(data_dir) = app_data_dir(app.handle()) {
                oauth::restore_listener(app.handle().clone(), data_dir);
            }
            routing::restore_enabled(app.handle().clone());
            easytier_mesh::restore_enabled(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_store,
            scan_codex_home,
            import_current_auth_as_profile,
            add_auth_json_profile,
            start_codex_oauth_login,
            codex_oauth_login_start,
            codex_oauth_open_auth_url,
            codex_oauth_submit_callback_url,
            codex_oauth_login_complete,
            codex_oauth_login_cancel,
            add_api_profile,
            is_codex_process_running,
            switch_profile,
            probe_usage,
            consume_usage_reset,
            save_quota_rule,
            update_profile_details,
            delete_profile,
            save_proxy_settings,
            test_proxy_settings,
            open_codex_home,
            save_auto_settings,
            routing_status,
            routing_save_settings,
            routing_save_log_settings,
            routing_start,
            routing_stop,
            routing_regenerate_access_key,
            routing_read_logs,
            routing_test_request,
            routing_apply_codex_config,
            routing_restore_codex_config,
            mesh_status,
            mesh_save_settings,
            mesh_start,
            mesh_stop,
            mesh_refresh_public_nodes,
            mesh_create_share_payload,
            mesh_import_share_payload,
            mesh_list_devices,
            mesh_save_device_sync,
            mesh_sync_now,
            mesh_export_migration_share,
            mesh_import_migration_share,
            refresh_profile_tokens_from_codex_home,
            refresh_all_profile_tokens,
            export_all_accounts_bundle,
            preview_bundle,
            import_accounts_bundle,
            restore_backup,
            load_codex_config_files,
            save_codex_config_file,
            format_codex_config_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running Codex Account Switcher");
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开主界面", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let probe = MenuItem::with_id(app, "probe-current", "探测当前账号额度", true, None::<&str>)?;
    let auto_switch = MenuItem::with_id(app, "auto-switch", "自动选择账号", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "刷新数据", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, Some("Ctrl+Q"))?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &settings,
            &separator,
            &probe,
            &auto_switch,
            &refresh,
            &separator,
            &hide,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::new()
        .tooltip("Codex Account Switcher")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "settings" => {
                show_main_window(app);
                let _ = app.emit("tray-action", "settings");
            }
            "probe-current" => {
                let _ = app.emit("tray-action", "probe-current");
            }
            "auto-switch" => {
                let _ = app.emit("tray-action", "auto-switch");
            }
            "refresh" => {
                let _ = app.emit("tray-action", "refresh");
            }
            "hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fake_jwt(claims: Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        format!("{}.{}.sig", header, payload)
    }

    fn fake_auth(account_id: &str, email: &str) -> String {
        let id_token = fake_jwt(serde_json::json!({
            "email": email,
            "chatgpt_plan_type": "plus",
            "https://api.openai.com/auth": {
                "chatgpt_subscription_active_start": "2026-06-07T11:57:51+00:00",
                "chatgpt_subscription_active_until": "2026-07-07T11:57:51+00:00",
                "chatgpt_subscription_last_checked": "2026-06-28T14:28:03.424632+00:00"
            },
            "chatgpt_user_id": "user-123",
            "organization_id": "org-123",
            "exp": 4102444800_i64
        }));
        let access_token = fake_jwt(serde_json::json!({
            "client_id": "app_test",
            "sub": "user-123",
            "exp": 4102441200_i64
        }));
        serde_json::json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": "rt_test",
                "account_id": account_id
            },
            "last_refresh": "2026-05-06T00:00:00Z"
        })
        .to_string()
    }

    fn test_profile(id: &str, account_id: &str, email: &str) -> AccountProfile {
        let key = [7u8; 32];
        let auth = fake_auth(account_id, email);
        AccountProfile {
            id: id.to_string(),
            alias: email.to_string(),
            note: String::new(),
            enabled: true,
            priority: 100,
            cooldown_until: None,
            quota_rule: QuotaRule::default(),
            summary: summarize_auth(&auth).unwrap(),
            encrypted_auth_json: encrypt_secret(auth.as_bytes(), &key).unwrap(),
            api_config: None,
            usage: UsageStats::default(),
            route_health: RouteHealth::default(),
            created_at: now_string(),
            updated_at: now_string(),
        }
    }

    fn write_text(path: &Path, value: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, value).unwrap();
    }

    fn bundle_payload_with_profile(
        codex_home: &Path,
        include_conversations: bool,
    ) -> BundlePayload {
        let files = collect_bundle_files(codex_home, include_conversations).unwrap();
        let metas = files
            .iter()
            .map(|f| BundleFileMeta {
                path: f.path.clone(),
                sha256: f.sha256.clone(),
                bytes: STANDARD.decode(&f.bytes_base64).unwrap().len() as u64,
            })
            .collect::<Vec<_>>();
        BundlePayload {
            manifest: BundleManifest {
                format: EXPORT_FORMAT.to_string(),
                version: EXPORT_VERSION,
                exported_at: now_string(),
                platform: "test".to_string(),
                profile_count: 1,
                include_conversations,
                files: metas,
            },
            settings: AppSettings {
                codex_home: Some(codex_home.to_string_lossy().to_string()),
                current_profile_id: None,
                auto_switch_enabled: true,
                probe_proxy: ProxySettings::default(),
                auto_token_refresh_enabled: false,
                auto_refresh_interval_secs: 60,
                background_token_refresh_enabled: false,
                background_token_refresh_interval_secs: 3_600,
                token_refresh_threshold_secs: 0,
                auto_probe_enabled: true,
                auto_probe_interval_secs: 60,
                routing: RoutingSettings::default(),
                mesh: easytier_mesh::MeshSettings::default(),
            },
            profiles: vec![ExportProfile {
                id: "profile-1".to_string(),
                alias: "primary".to_string(),
                note: String::new(),
                enabled: true,
                priority: 100,
                cooldown_until: None,
                quota_rule: QuotaRule::default(),
                summary: summarize_auth(&fake_auth("acc-1", "one@example.com")).unwrap(),
                auth_json: fake_auth("acc-1", "one@example.com"),
                api_config: None,
                usage: UsageStats::default(),
                created_at: now_string(),
                updated_at: now_string(),
            }],
            files,
        }
    }

    #[test]
    fn excludes_machine_bound_paths() {
        assert!(is_excluded_path("installation_id"));
        assert!(is_excluded_path(".sandbox/setup_marker.json"));
        assert!(is_excluded_path("cap_sid"));
        assert!(!is_excluded_path("config.toml"));
        assert!(!is_excluded_path("rules/default.rules"));
    }

    #[test]
    fn detects_conversation_paths() {
        assert!(is_conversation_path("sessions/abc.jsonl"));
        assert!(is_conversation_path("logs_2.sqlite"));
        assert!(!is_conversation_path("memories/user.json"));
    }

    #[test]
    fn secret_envelope_round_trips() {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let encrypted = encrypt_secret(b"hello", &key).unwrap();
        let decrypted = decrypt_secret(&encrypted, &key).unwrap();
        assert_eq!(decrypted, b"hello");
    }

    #[test]
    fn codex_launch_env_keeps_loopback_out_of_proxy() {
        let merged = merge_loopback_no_proxy(Some("example.com,127.0.0.1"));

        assert!(merged.contains("example.com"));
        assert!(merged.contains("127.0.0.1"));
        assert!(merged.contains("localhost"));
        assert!(merged.contains("::1"));
    }

    #[test]
    fn codex_launch_env_uses_saved_proxy_url() {
        assert_eq!(
            codex_launch_proxy_url(Some(&ProxySettings {
                enabled: true,
                url: "127.0.0.1:7890".to_string(),
            }))
            .as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert!(codex_launch_proxy_url(Some(&ProxySettings {
            enabled: false,
            url: "127.0.0.1:7890".to_string(),
        }))
        .is_none());
    }

    #[test]
    fn chromium_proxy_url_uses_chrome_compatible_scheme() {
        assert_eq!(
            chromium_proxy_url(Some(&ProxySettings {
                enabled: true,
                url: "socks5h://127.0.0.1:7898".to_string(),
            }))
            .as_deref(),
            Some("socks5://127.0.0.1:7898")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_system_proxy_uses_wininet_format() {
        assert_eq!(
            windows_proxy_server_value(Some(&ProxySettings {
                enabled: true,
                url: "socks5h://127.0.0.1:7898".to_string(),
            }))
            .as_deref(),
            Some("socks=127.0.0.1:7898")
        );
    }

    #[test]
    fn codex_env_file_replaces_only_proxy_keys() {
        let rendered = render_codex_proxy_env(
            "OPENAI_API_KEY=keep\nALL_PROXY=socks5://127.0.0.1:1080\nNO_PROXY=example.com\n",
            &ProxySettings {
                enabled: true,
                url: "socks5h://127.0.0.1:7898".to_string(),
            },
        );

        assert!(rendered.contains("OPENAI_API_KEY=keep"));
        assert!(rendered.contains("ALL_PROXY=socks5h://127.0.0.1:7898"));
        assert!(rendered.contains("HTTPS_PROXY=socks5h://127.0.0.1:7898"));
        assert!(rendered.contains("NO_PROXY=example.com,localhost,127.0.0.1,::1,[::1],0.0.0.0"));
        assert!(!rendered.contains("ALL_PROXY=socks5://127.0.0.1:1080"));
    }

    #[test]
    fn rejects_unsafe_relative_paths() {
        assert!(safe_relative_path("../auth.json").is_err());
        assert!(safe_relative_path("rules/default.rules").is_ok());
    }

    #[test]
    fn summarizes_auth_from_jwt_claims() {
        let summary = summarize_auth(&fake_auth("acc-1", "one@example.com")).unwrap();
        assert_eq!(summary.auth_mode.as_deref(), Some("chatgpt"));
        assert_eq!(summary.email.as_deref(), Some("one@example.com"));
        assert_eq!(summary.plan.as_deref(), Some("plus"));
        assert_eq!(
            summary.subscription_active_until.as_deref(),
            Some("2026-07-07T11:57:51+00:00")
        );
        assert_eq!(
            summary.subscription_last_checked.as_deref(),
            Some("2026-06-28T14:28:03.424632+00:00")
        );
        assert_eq!(summary.account_id.as_deref(), Some("acc-1"));
        assert_eq!(summary.user_id.as_deref(), Some("user-123"));
        assert_eq!(summary.organization_id.as_deref(), Some("org-123"));
        assert_eq!(summary.id_token_exp, Some(4102444800));
        assert_eq!(summary.access_token_exp, Some(4102441200));
    }

    #[test]
    fn extracts_plan_from_namespaced_jwt_claims() {
        let claims = serde_json::json!({
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": "pro"
            }
        });

        assert_eq!(
            extract_plan_type_from_claims(&claims).as_deref(),
            Some("pro")
        );
    }

    #[test]
    fn extracts_account_id_from_access_token_claims() {
        let claims = serde_json::json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-native-oauth"
            }
        });
        assert_eq!(
            extract_account_id_from_claims(&claims).as_deref(),
            Some("acc-native-oauth")
        );
    }

    #[test]
    fn scans_codex_home_without_reading_secrets_into_output() {
        let dir = tempdir().unwrap();
        write_text(
            &dir.path().join("auth.json"),
            &fake_auth("acc-1", "one@example.com"),
        );

        let scan = scan_codex_home_path(dir.path()).unwrap();

        assert!(scan.exists);
        assert!(scan.has_auth);
        assert_eq!(
            scan.current_auth.unwrap().email.as_deref(),
            Some("one@example.com")
        );
        assert!(scan.migratable.contains(&"config.toml".to_string()));
        assert!(scan.excluded.contains(&"installation_id".to_string()));
    }

    #[test]
    fn store_view_omits_encrypted_auth_json() {
        let store = AppStore {
            settings: AppSettings::default(),
            profiles: vec![test_profile("profile-1", "acc-1", "one@example.com")],
            events: vec![],
        };

        let json = serde_json::to_string(&store_view(store)).unwrap();

        assert!(json.contains("one@example.com"));
        assert!(!json.contains("encryptedAuthJson"));
        assert!(!json.contains("rt_test"));
    }

    #[test]
    fn delete_profile_clears_current_selection_without_touching_others() {
        let mut store = AppStore::default();
        store.settings.current_profile_id = Some("profile-1".to_string());
        store.profiles = vec![
            test_profile("profile-1", "acc-1", "one@example.com"),
            test_profile("profile-2", "acc-2", "two@example.com"),
        ];

        let deleted_alias = delete_profile_from_store(&mut store, "profile-1").unwrap();

        assert_eq!(deleted_alias, "one@example.com");
        assert_eq!(store.settings.current_profile_id, None);
        assert_eq!(store.profiles.len(), 1);
        assert_eq!(store.profiles[0].id, "profile-2");
    }

    #[test]
    fn delete_profile_rejects_missing_profile() {
        let mut store = AppStore::default();

        let result = delete_profile_from_store(&mut store, "missing");

        assert!(result.is_err());
    }

    #[test]
    fn export_profile_selection_filters_by_requested_ids() {
        let profiles = vec![
            test_profile("profile-1", "acc-1", "one@example.com"),
            test_profile("profile-2", "acc-2", "two@example.com"),
            test_profile("profile-3", "acc-3", "three@example.com"),
        ];
        let requested = vec!["profile-3".to_string(), "profile-1".to_string()];

        let selected = select_profiles_for_export(&profiles, Some(&requested), false).unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["profile-1", "profile-3"]
        );
        assert_eq!(
            select_profiles_for_export(&profiles, None, false)
                .unwrap()
                .len(),
            3
        );
        assert!(select_profiles_for_export(&profiles, Some(&Vec::new()), false).is_err());
        assert!(
            select_profiles_for_export(&profiles, Some(&vec!["missing".to_string()]), false)
                .is_err()
        );
    }

    #[test]
    fn codex_process_detection_excludes_switcher_processes() {
        assert!(!looks_like_codex_process(
            r#"CodexSwitcher.exe 123 Console"#,
            ""
        ));
        assert!(!looks_like_codex_process(
            r#"D:\go\src\cmsCloud\tools\codex-account-switcher\target\debug\codex-account-switcher.exe"#,
            ""
        ));
        assert!(!looks_like_codex_process(
            r#"CommandLine=target/debug/codex_account_switcher.exe"#,
            ""
        ));
        assert!(looks_like_codex_process(
            r#"CommandLine=C:\Users\me\AppData\Roaming\npm\codex.cmd"#,
            ""
        ));
        assert!(looks_like_codex_process(
            "9821 /usr/local/bin/codex --model gpt-5.5",
            ""
        ));
        assert!(looks_like_codex_process(
            r#"CommandLine=C:\Users\me\AppData\Roaming\npm\chatgpt.cmd"#,
            ""
        ));
    }

    #[cfg(windows)]
    #[test]
    fn classifies_windows_desktop_root_without_matching_helpers() {
        assert_eq!(
            classify_codex_process(
                "Codex.exe",
                Some(r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0_x64__test\app\Codex.exe"),
                r#""C:\Program Files\WindowsApps\OpenAI.Codex_1.0_x64__test\app\Codex.exe""#,
                ""
            ),
            Some(CodexLaunchKind::Desktop)
        );
        assert_eq!(
            classify_codex_process(
                "Codex.exe",
                Some(r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0_x64__test\app\Codex.exe"),
                r#""C:\Program Files\WindowsApps\OpenAI.Codex_1.0_x64__test\app\Codex.exe" --type=renderer"#,
                ""
            ),
            None
        );
        assert_eq!(
            classify_codex_process(
                "codex.exe",
                Some(r"C:\npm\node_modules\@openai\codex\vendor\x86_64-pc-windows-msvc\codex.exe"),
                r#"codex.exe --version"#,
                ""
            ),
            Some(CodexLaunchKind::Cli)
        );
        assert_eq!(
            classify_codex_process(
                "ChatGPT.exe",
                Some(r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_1.0_x64__test\app\ChatGPT.exe"),
                r#""C:\Program Files\WindowsApps\OpenAI.ChatGPT_1.0_x64__test\app\ChatGPT.exe""#,
                ""
            ),
            Some(CodexLaunchKind::Desktop)
        );
    }

    #[test]
    fn classifies_cli_without_matching_switcher() {
        assert_eq!(
            classify_codex_process(
                "node.exe",
                Some(r"C:\Program Files\nodejs\node.exe"),
                r#"node.exe C:\npm\node_modules\@openai\codex\bin\codex.js"#,
                ""
            ),
            Some(CodexLaunchKind::Cli)
        );
        assert_eq!(
            classify_codex_process(
                "codex-account-switcher.exe",
                Some(r"G:\CodexSwitcher\codex-account-switcher.exe"),
                r#""G:\CodexSwitcher\codex-account-switcher.exe""#,
                ""
            ),
            None
        );
        assert_eq!(
            classify_codex_process(
                "node.exe",
                Some(r"C:\Program Files\nodejs\node.exe"),
                r#"node.exe C:\npm\node_modules\@openai\chatgpt\bin\chatgpt.js"#,
                ""
            ),
            Some(CodexLaunchKind::Cli)
        );
    }

    #[cfg(windows)]
    #[test]
    fn parses_windows_process_snapshot_using_only_desktop_root() {
        let snapshot = parse_windows_codex_process_json(
            r#"[
                {
                    "ProcessId": 101,
                    "Name": "Codex.exe",
                    "ExecutablePath": "C:\\Program Files\\WindowsApps\\OpenAI.Codex_1.0_x64__test\\app\\Codex.exe",
                    "CommandLine": "\"C:\\Program Files\\WindowsApps\\OpenAI.Codex_1.0_x64__test\\app\\Codex.exe\""
                },
                {
                    "ProcessId": 102,
                    "Name": "Codex.exe",
                    "ExecutablePath": "C:\\Program Files\\WindowsApps\\OpenAI.Codex_1.0_x64__test\\app\\Codex.exe",
                    "CommandLine": "\"C:\\Program Files\\WindowsApps\\OpenAI.Codex_1.0_x64__test\\app\\Codex.exe\" --type=renderer"
                },
                {
                    "ProcessId": 103,
                    "Name": "codex.exe",
                    "ExecutablePath": "C:\\Program Files\\WindowsApps\\OpenAI.Codex_1.0_x64__test\\app\\resources\\codex.exe",
                    "CommandLine": "\"C:\\Program Files\\WindowsApps\\OpenAI.Codex_1.0_x64__test\\app\\resources\\codex.exe\" app-server"
                }
            ]"#,
            "",
        );

        assert_eq!(snapshot.processes.len(), 1);
        assert_eq!(snapshot.processes[0].pid, 101);
        assert_eq!(snapshot.launch_kind(), Some(CodexLaunchKind::Desktop));
    }

    #[cfg(windows)]
    #[test]
    fn parses_windows_chatgpt_process_snapshot() {
        let snapshot = parse_windows_codex_process_json(
            r#"[{
                "ProcessId": 201,
                "Name": "ChatGPT.exe",
                "ExecutablePath": "C:\\Program Files\\WindowsApps\\OpenAI.ChatGPT_1.0_x64__test\\app\\ChatGPT.exe",
                "CommandLine": "\"C:\\Program Files\\WindowsApps\\OpenAI.ChatGPT_1.0_x64__test\\app\\ChatGPT.exe\""
            }]"#,
            "",
        );

        assert_eq!(snapshot.processes.len(), 1);
        assert_eq!(snapshot.processes[0].pid, 201);
        assert_eq!(snapshot.launch_kind(), Some(CodexLaunchKind::Desktop));
    }

    #[test]
    fn codex_process_id_parsing_excludes_switcher_processes() {
        let output = r#"
CommandLine=C:\Users\me\AppData\Roaming\npm\codex.cmd --model gpt-5
ExecutablePath=C:\Users\me\AppData\Roaming\npm\codex.cmd
Name=node.exe
ProcessId=1234

CommandLine=G:\codex_learn\codex-account-switcher\target\debug\codex-account-switcher.exe
ExecutablePath=G:\codex_learn\codex-account-switcher\target\debug\codex-account-switcher.exe
Name=codex-account-switcher.exe
ProcessId=5678

"#;

        assert_eq!(codex_process_ids_from_wmic_output(output, ""), vec![1234]);
    }

    #[test]
    fn compatible_home_uses_chatgpt_when_saved_codex_home_has_no_config() {
        let dir = tempdir().unwrap();
        let codex = dir.path().join(".codex");
        let chatgpt = dir.path().join(".chatgpt");
        fs::create_dir_all(&codex).unwrap();
        fs::create_dir_all(&chatgpt).unwrap();
        write_text(&chatgpt.join("auth.json"), "{}");

        assert_eq!(compatible_official_home(codex), chatgpt);
    }

    #[test]
    fn compatible_home_prefers_newer_official_home() {
        let dir = tempdir().unwrap();
        let codex = dir.path().join(".codex");
        let chatgpt = dir.path().join(".chatgpt");
        fs::create_dir_all(&codex).unwrap();
        fs::create_dir_all(&chatgpt).unwrap();
        write_text(&codex.join("auth.json"), "{}");
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_text(&chatgpt.join("auth.json"), "{}");

        assert_eq!(compatible_official_home(codex), chatgpt);
    }

    #[test]
    fn verify_auth_json_written_detects_mismatch() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        write_text(&auth_path, r#"{"account":"one"}"#);

        verify_auth_json_written(&auth_path, r#"{"account":"one"}"#).unwrap();
        assert!(verify_auth_json_written(&auth_path, r#"{"account":"two"}"#).is_err());
    }

    #[test]
    fn collect_bundle_files_respects_default_and_conversation_scope() {
        let dir = tempdir().unwrap();
        write_text(&dir.path().join("config.toml"), "model = \"gpt-5.5\"");
        write_text(&dir.path().join("rules/default.rules"), "allow");
        write_text(&dir.path().join("memories/user.json"), "{}");
        write_text(&dir.path().join("sessions/session-1.jsonl"), "{}");
        write_text(&dir.path().join("logs_2.sqlite"), "sqlite");
        write_text(&dir.path().join("installation_id"), "machine-id");
        write_text(&dir.path().join(".sandbox/setup_marker.json"), "{}");

        let default_files = collect_bundle_files(dir.path(), false).unwrap();
        let default_paths = default_files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>();
        assert!(default_paths.contains(&"config.toml"));
        assert!(default_paths.contains(&"rules/default.rules"));
        assert!(default_paths.contains(&"memories/user.json"));
        assert!(!default_paths.contains(&"sessions/session-1.jsonl"));
        assert!(!default_paths.contains(&"installation_id"));
        assert!(!default_paths.contains(&".sandbox/setup_marker.json"));

        let full_files = collect_bundle_files(dir.path(), true).unwrap();
        let full_paths = full_files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>();
        assert!(full_paths.contains(&"sessions/session-1.jsonl"));
        assert!(full_paths.contains(&"logs_2.sqlite"));
        assert!(!full_paths.contains(&"installation_id"));
    }

    #[test]
    fn encrypted_export_bundle_round_trips_and_rejects_wrong_password() {
        let dir = tempdir().unwrap();
        let bundle_path = dir.path().join("codex-switcher.zip.enc");
        write_text(&dir.path().join("config.toml"), "model = \"gpt-5.5\"");
        write_text(&dir.path().join("rules/default.rules"), "allow");
        let payload = bundle_payload_with_profile(dir.path(), false);
        let zip_bytes = zip_payload(&payload).unwrap();
        let encrypted = encrypt_export(&zip_bytes, "strong-password").unwrap();
        fs::write(&bundle_path, serde_json::to_vec(&encrypted).unwrap()).unwrap();

        let restored = read_bundle(bundle_path.to_str().unwrap(), "strong-password").unwrap();

        assert_eq!(restored.manifest.profile_count, 1);
        assert_eq!(
            restored.profiles[0].summary.email.as_deref(),
            Some("one@example.com")
        );
        assert!(restored.files.iter().any(|f| f.path == "config.toml"));
        assert!(read_bundle(bundle_path.to_str().unwrap(), "").is_err());
        assert!(read_bundle(bundle_path.to_str().unwrap(), "wrong-password").is_err());
    }

    #[test]
    fn plaintext_export_bundle_round_trips_without_password() {
        let dir = tempdir().unwrap();
        let bundle_path = dir.path().join("codex-switcher.zip");
        write_text(&dir.path().join("config.toml"), "model = \"gpt-5.5\"");
        let payload = bundle_payload_with_profile(dir.path(), false);
        let zip_bytes = zip_payload(&payload).unwrap();
        fs::write(&bundle_path, zip_bytes).unwrap();

        let restored = read_bundle(bundle_path.to_str().unwrap(), "").unwrap();

        assert_eq!(restored.manifest.profile_count, 1);
        assert_eq!(
            restored.profiles[0].summary.email.as_deref(),
            Some("one@example.com")
        );
        assert!(restored.files.iter().any(|f| f.path == "config.toml"));
    }

    #[test]
    fn backup_replace_and_restore_auth_file() {
        let dir = tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        write_text(&auth_path, "old-auth");

        let backup = backup_auth_file(&auth_path).unwrap().unwrap();
        replace_file_with_rollback(&auth_path, b"new-auth", Some(&backup)).unwrap();
        assert_eq!(fs::read_to_string(&auth_path).unwrap(), "new-auth");

        let latest = latest_backup(dir.path()).unwrap();
        fs::copy(latest, &auth_path).unwrap();
        assert_eq!(fs::read_to_string(&auth_path).unwrap(), "old-auth");
    }

    #[test]
    fn import_bundle_payload_semantics_are_safe() {
        let dir = tempdir().unwrap();
        write_text(&dir.path().join("config.toml"), "model = \"gpt-5.5\"");
        let mut payload = bundle_payload_with_profile(dir.path(), true);
        payload.files.push(BundleFile {
            path: "sessions/session-1.jsonl".to_string(),
            sha256: hex_sha256(b"conversation"),
            bytes_base64: STANDARD.encode(b"conversation"),
        });
        payload.files.push(BundleFile {
            path: "installation_id".to_string(),
            sha256: hex_sha256(b"bad"),
            bytes_base64: STANDARD.encode(b"bad"),
        });
        let target = tempdir().unwrap();
        let mut restored = 0usize;
        let mut skipped = 0usize;

        for file in payload.files {
            if is_conversation_path(&file.path) {
                skipped += 1;
                continue;
            }
            if is_excluded_path(&file.path) {
                continue;
            }
            let bytes = STANDARD.decode(&file.bytes_base64).unwrap();
            assert_eq!(hex_sha256(&bytes), file.sha256);
            let out = target.path().join(safe_relative_path(&file.path).unwrap());
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(out, bytes).unwrap();
            restored += 1;
        }

        assert_eq!(restored, 1);
        assert_eq!(skipped, 1);
        assert!(target.path().join("config.toml").exists());
        assert!(!target.path().join("sessions/session-1.jsonl").exists());
        assert!(!target.path().join("installation_id").exists());
    }

    #[test]
    fn push_event_keeps_latest_100_entries() {
        let mut store = AppStore::default();

        for i in 0..150 {
            push_event(&mut store, "info", &format!("event-{i}"));
        }

        assert_eq!(store.events.len(), 100);
        assert_eq!(store.events[0].message, "event-149");
        assert_eq!(store.events[99].message, "event-50");
    }

    #[test]
    fn legacy_app_data_migration_copies_old_identifier_dir_once() {
        let root = tempdir().unwrap();
        let legacy = root.path().join(LEGACY_APP_IDENTIFIER);
        let new = root.path().join("local.codex.account-switcher");
        write_text(&legacy.join(STORE_FILE), r#"{"profiles":[]}"#);
        write_text(&legacy.join(LOCAL_KEY_FILE), "legacy-key");

        migrate_legacy_app_data_dir(&new).unwrap();

        assert_eq!(
            fs::read_to_string(new.join(STORE_FILE)).unwrap(),
            r#"{"profiles":[]}"#
        );
        assert_eq!(
            fs::read_to_string(new.join(LOCAL_KEY_FILE)).unwrap(),
            "legacy-key"
        );

        write_text(&legacy.join(STORE_FILE), r#"{"profiles":["changed"]}"#);
        migrate_legacy_app_data_dir(&new).unwrap();
        assert_eq!(
            fs::read_to_string(new.join(STORE_FILE)).unwrap(),
            r#"{"profiles":[]}"#
        );
    }

    #[test]
    fn writes_api_provider_without_removing_mcp_config() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        write_text(
            &config_path,
            "[mcp_servers.demo]\ncommand = \"demo-server\"\n\n[model_providers.old-provider]\nexperimental_bearer_token = \"old-secret\"\n",
        );
        let api_config = ApiProviderConfig {
            provider_id: "my-provider".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model: "gpt-custom".to_string(),
            wire_api: "responses".to_string(),
        };

        write_api_provider_config(
            &config_path,
            &api_config,
            "My API",
            &["old-provider".to_string(), "my-provider".to_string()],
        )
        .unwrap();

        let text = fs::read_to_string(config_path).unwrap();
        let document = text.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(document["model"].as_str(), Some("gpt-custom"));
        assert_eq!(document["model_provider"].as_str(), Some("my-provider"));
        assert_eq!(
            document["model_providers"]["my-provider"]["requires_openai_auth"].as_bool(),
            Some(true)
        );
        assert!(document["model_providers"]["my-provider"]
            .as_table()
            .unwrap()
            .get("experimental_bearer_token")
            .is_none());
        assert!(document
            .as_table()
            .get("disable_response_storage")
            .is_none());
        assert!(document.as_table().get("web_search").is_none());
        assert!(document.as_table().get("model_reasoning_effort").is_none());
        assert!(document
            .as_table()
            .get("model_supports_reasoning_summaries")
            .is_none());
        assert_eq!(
            document["mcp_servers"]["demo"]["command"].as_str(),
            Some("demo-server")
        );
        assert!(document["model_providers"]
            .as_table()
            .unwrap()
            .get("old-provider")
            .is_none());
    }

    #[test]
    fn writes_official_longcat_codex_options() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let api_config = ApiProviderConfig {
            provider_id: "longcat".to_string(),
            base_url: "https://api.longcat.chat/openai/v1".to_string(),
            model: "LongCat-2.0".to_string(),
            wire_api: "responses".to_string(),
        };

        write_api_provider_config(
            &config_path,
            &api_config,
            "LongCat",
            &["longcat".to_string()],
        )
        .unwrap();

        let text = fs::read_to_string(config_path).unwrap();
        let document = text.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(document["disable_response_storage"].as_bool(), Some(true));
        assert_eq!(document["web_search"].as_str(), Some("disabled"));
        assert_eq!(document["model_reasoning_effort"].as_str(), Some("high"));
        assert_eq!(
            document["model_supports_reasoning_summaries"].as_bool(),
            Some(true)
        );
        assert_eq!(
            document["model_providers"]["longcat"]["requires_openai_auth"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn removes_longcat_options_for_other_api_provider() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        write_text(
            &config_path,
            r#"model_provider = "codex"
model = "LongCat-2.0"
disable_response_storage = true
web_search = "disabled"
model_reasoning_effort = "high"
model_supports_reasoning_summaries = true

[model_providers.codex]
name = "codex"
base_url = "https://api.longcat.chat/openai/v1"
wire_api = "responses"
requires_openai_auth = true
"#,
        );
        let api_config = ApiProviderConfig {
            provider_id: "other-api".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model: "gpt-custom".to_string(),
            wire_api: "responses".to_string(),
        };

        write_api_provider_config(
            &config_path,
            &api_config,
            "Other API",
            &["codex".to_string(), "other-api".to_string()],
        )
        .unwrap();

        let text = fs::read_to_string(config_path).unwrap();
        let document = text.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(document["model"].as_str(), Some("gpt-custom"));
        assert_eq!(document["model_provider"].as_str(), Some("other-api"));
        assert!(document
            .as_table()
            .get("disable_response_storage")
            .is_none());
        assert!(document.as_table().get("web_search").is_none());
        assert!(document.as_table().get("model_reasoning_effort").is_none());
        assert!(document
            .as_table()
            .get("model_supports_reasoning_summaries")
            .is_none());
        assert!(document
            .as_table()
            .get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|providers| providers.get("codex"))
            .is_none());
        assert_eq!(
            document["model_providers"]["other-api"]["requires_openai_auth"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn keeps_non_official_root_options_for_other_api_provider() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        write_text(
            &config_path,
            r#"web_search = "enabled"
model_reasoning_effort = "medium"
"#,
        );
        let api_config = ApiProviderConfig {
            provider_id: "other-api".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model: "gpt-custom".to_string(),
            wire_api: "responses".to_string(),
        };

        write_api_provider_config(
            &config_path,
            &api_config,
            "Other API",
            &["other-api".to_string()],
        )
        .unwrap();

        let text = fs::read_to_string(config_path).unwrap();
        let document = text.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(document["web_search"].as_str(), Some("enabled"));
        assert_eq!(document["model_reasoning_effort"].as_str(), Some("medium"));
    }

    #[test]
    fn removes_longcat_config_for_non_api_account() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        write_text(
            &config_path,
            r#"model_provider = "codex"
model = "LongCat-2.0"
disable_response_storage = true
web_search = "disabled"
model_reasoning_effort = "high"
model_supports_reasoning_summaries = true

[model_providers.codex]
name = "codex"
base_url = "https://api.longcat.chat/openai/v1"
wire_api = "responses"
requires_openai_auth = true

[mcp_servers.demo]
command = "demo-server"
"#,
        );

        remove_longcat_config_for_non_api_account(&config_path).unwrap();

        let text = fs::read_to_string(config_path).unwrap();
        let document = text.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(document.as_table().get("model").is_none());
        assert!(document.as_table().get("model_provider").is_none());
        assert!(document
            .as_table()
            .get("disable_response_storage")
            .is_none());
        assert!(document.as_table().get("web_search").is_none());
        assert!(document.as_table().get("model_reasoning_effort").is_none());
        assert!(document
            .as_table()
            .get("model_supports_reasoning_summaries")
            .is_none());
        assert!(document
            .as_table()
            .get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|providers| providers.get("codex"))
            .is_none());
        assert_eq!(
            document["mcp_servers"]["demo"]["command"].as_str(),
            Some("demo-server")
        );
    }

    #[test]
    fn validates_api_provider_identifiers_and_urls() {
        assert_eq!(normalize_provider_id(" My-API ").unwrap(), "my-api");
        assert!(normalize_provider_id("bad provider").is_err());
        assert_eq!(
            normalize_api_base_url("https://api.example.com/v1/").unwrap(),
            "https://api.example.com/v1"
        );
        assert_eq!(normalize_api_base_url("").unwrap(), DEFAULT_API_BASE_URL);
        assert_eq!(normalize_api_base_url("   ").unwrap(), DEFAULT_API_BASE_URL);
        assert!(normalize_api_base_url("ftp://api.example.com").is_err());
    }

    #[test]
    fn formats_codex_config_files_and_rejects_unknown_targets() {
        let auth =
            format_codex_config_content("auth.json", "{\"tokens\":{\"access_token\":\"a\"}}")
                .unwrap();
        assert!(auth.contains("\n  \"tokens\""));
        assert!(format_codex_config_content("auth.json", "{bad").is_err());

        let config = format_codex_config_content(
            "config.toml",
            "model = \"gpt-5\"\n[model_providers.openai]\nbase_url = \"https://api.openai.com/v1\"\n",
        )
        .unwrap();
        assert!(config.contains("model = \"gpt-5\""));
        assert!(format_codex_config_content("settings.json", "{}").is_err());
    }

    #[test]
    fn reads_config_file_view_and_creates_timestamped_backup() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        write_text(&config_path, "model = \"gpt-5\"");

        let view = read_config_file_view(&config_path).unwrap();
        assert!(view.exists);
        assert!(view.content.contains("gpt-5"));

        let backup = backup_codex_config_file(&config_path).unwrap().unwrap();
        assert!(backup.exists());
        assert!(backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("config.toml-"));
    }

    #[test]
    fn clamps_auto_refresh_intervals() {
        assert_eq!(clamp_interval(1), 30);
        assert_eq!(clamp_interval(60), 60);
        assert_eq!(clamp_interval(99_999), 86_400);
        assert_eq!(clamp_background_token_refresh_interval(60), 3_600);
        assert_eq!(clamp_background_token_refresh_interval(7_200), 7_200);
        assert_eq!(clamp_background_token_refresh_interval(999_999), 604_800);
        assert_eq!(clamp_token_refresh_threshold(0), 0);
        assert_eq!(clamp_token_refresh_threshold(60), 60);
        assert_eq!(clamp_token_refresh_threshold(86_400), 86_400);
    }

    #[test]
    fn refresh_due_defaults_to_expired_or_unknown_access_token() {
        let now = Utc::now().timestamp();

        assert!(should_refresh_access_token(None, 0));
        assert!(should_refresh_access_token(Some(now - 1), 0));
        assert!(!should_refresh_access_token(Some(now + 3_600), 0));
        assert!(should_refresh_access_token(Some(now + 3_600), 3_600));
    }

    #[test]
    fn token_keepalive_skips_only_current_and_relogin_required_profiles() {
        let current = test_profile("current", "acc-1", "one@example.com");
        let mut relogin = test_profile("relogin", "acc-2", "two@example.com");
        relogin.usage.last_token_refresh_status = Some("relogin_required".to_string());
        let other = test_profile("other", "acc-3", "three@example.com");

        assert!(should_skip_profile_token_keepalive(
            &current,
            Some("current"),
            false
        ));
        assert!(should_skip_profile_token_keepalive(
            &relogin,
            Some("current"),
            false
        ));
        assert!(!should_skip_profile_token_keepalive(
            &other,
            Some("current"),
            false
        ));
        assert!(!should_skip_profile_token_keepalive(
            &current,
            Some("current"),
            true
        ));
    }

    #[test]
    fn reauthorization_clears_stale_refresh_failure_without_losing_usage() {
        let mut usage = UsageStats {
            hourly_used: 3,
            daily_used: 7,
            last_probe_status: Some("200".to_string()),
            last_error: Some("refresh_token_reused".to_string()),
            last_token_refresh_at: Some("2026-07-29T07:13:45Z".to_string()),
            last_token_refresh_status: Some("relogin_required".to_string()),
            last_token_refresh_error: Some("refresh_token_reused".to_string()),
            ..UsageStats::default()
        };

        clear_stale_auth_failure(&mut usage);

        assert_eq!(usage.hourly_used, 3);
        assert_eq!(usage.daily_used, 7);
        assert_eq!(usage.last_probe_status.as_deref(), Some("200"));
        assert!(usage.last_error.is_none());
        assert!(usage.last_token_refresh_at.is_none());
        assert!(usage.last_token_refresh_status.is_none());
        assert!(usage.last_token_refresh_error.is_none());
    }

    #[test]
    fn repairs_profile_reauthorized_before_successful_probe() {
        let mut profile = test_profile("reauthorized", "acc-1", "one@example.com");
        profile.updated_at = "2026-07-29T07:14:45Z".to_string();
        profile.summary.access_token_exp = Some(4_102_444_800);
        profile.usage.last_probe_at = Some("2026-07-29T07:20:01Z".to_string());
        profile.usage.last_probe_status = Some("200".to_string());
        profile.usage.last_token_refresh_at = Some("2026-07-29T07:13:45Z".to_string());
        profile.usage.last_token_refresh_status = Some("relogin_required".to_string());
        profile.usage.last_token_refresh_error = Some("refresh_token_reused".to_string());

        assert!(repair_successfully_reauthorized_profile(&mut profile));
        assert!(profile.usage.last_token_refresh_status.is_none());
        assert!(profile.usage.last_token_refresh_error.is_none());
        assert_eq!(profile.usage.last_probe_status.as_deref(), Some("200"));
    }

    #[test]
    fn syncs_rotated_current_auth_into_matching_profile() {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let first_auth = fake_auth("acc-1", "one@example.com");
        let second_auth = fake_auth("acc-2", "two@example.com");
        let mut store = AppStore::default();
        store.profiles = vec![
            AccountProfile {
                id: "profile-1".to_string(),
                alias: "one".to_string(),
                note: String::new(),
                enabled: true,
                priority: 100,
                cooldown_until: None,
                quota_rule: QuotaRule::default(),
                summary: summarize_auth(&first_auth).unwrap(),
                encrypted_auth_json: encrypt_secret(first_auth.as_bytes(), &key).unwrap(),
                api_config: None,
                usage: UsageStats::default(),
                route_health: RouteHealth::default(),
                created_at: now_string(),
                updated_at: now_string(),
            },
            AccountProfile {
                id: "profile-2".to_string(),
                alias: "two".to_string(),
                note: String::new(),
                enabled: true,
                priority: 90,
                cooldown_until: None,
                quota_rule: QuotaRule::default(),
                summary: summarize_auth(&second_auth).unwrap(),
                encrypted_auth_json: encrypt_secret(second_auth.as_bytes(), &key).unwrap(),
                api_config: None,
                usage: UsageStats::default(),
                route_health: RouteHealth::default(),
                created_at: now_string(),
                updated_at: now_string(),
            },
        ];
        let rotated_auth = apply_token_refresh_response(
            &first_auth,
            &serde_json::json!({
                "access_token": fake_jwt(serde_json::json!({
                    "client_id": "app_test",
                    "sub": "user-123",
                    "exp": 4102449999_i64
                })),
                "refresh_token": "rt_rotated"
            }),
        )
        .unwrap();

        let synced = sync_auth_json_into_matching_profile(&mut store, &rotated_auth, &key).unwrap();

        assert_eq!(synced.as_deref(), Some("profile-1"));
        let saved = String::from_utf8(
            decrypt_secret(&store.profiles[0].encrypted_auth_json, &key).unwrap(),
        )
        .unwrap();
        let saved: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            saved
                .pointer("/tokens/refresh_token")
                .and_then(Value::as_str),
            Some("rt_rotated")
        );
        let untouched = String::from_utf8(
            decrypt_secret(&store.profiles[1].encrypted_auth_json, &key).unwrap(),
        )
        .unwrap();
        assert_eq!(untouched, second_auth);
    }

    #[test]
    fn skips_current_auth_when_no_profile_identity_matches() {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let stored_auth = fake_auth("acc-1", "one@example.com");
        let mut store = AppStore::default();
        store.profiles.push(AccountProfile {
            id: "profile-1".to_string(),
            alias: "one".to_string(),
            note: String::new(),
            enabled: true,
            priority: 100,
            cooldown_until: None,
            quota_rule: QuotaRule::default(),
            summary: summarize_auth(&stored_auth).unwrap(),
            encrypted_auth_json: encrypt_secret(stored_auth.as_bytes(), &key).unwrap(),
            api_config: None,
            usage: UsageStats::default(),
            route_health: RouteHealth::default(),
            created_at: now_string(),
            updated_at: now_string(),
        });

        let synced = sync_auth_json_into_matching_profile(
            &mut store,
            &fake_auth("acc-other", "other@example.com"),
            &key,
        )
        .unwrap();

        assert_eq!(synced, None);
        let saved = String::from_utf8(
            decrypt_secret(&store.profiles[0].encrypted_auth_json, &key).unwrap(),
        )
        .unwrap();
        assert_eq!(saved, stored_auth);
    }

    #[test]
    fn applies_token_refresh_response_without_losing_existing_refresh_token() {
        let original = fake_auth("acc-1", "one@example.com");
        let new_access = fake_jwt(serde_json::json!({
            "client_id": "app_test",
            "sub": "user-123",
            "exp": 4102449999_i64
        }));
        let updated = apply_token_refresh_response(
            &original,
            &serde_json::json!({
                "access_token": new_access
            }),
        )
        .unwrap();
        let auth: Value = serde_json::from_str(&updated).unwrap();

        assert_eq!(
            auth.pointer("/tokens/access_token").and_then(Value::as_str),
            Some(new_access.as_str())
        );
        assert_eq!(
            auth.pointer("/tokens/refresh_token")
                .and_then(Value::as_str),
            Some("rt_test")
        );
        assert!(auth.get("last_refresh").and_then(Value::as_str).is_some());
        assert_eq!(
            summarize_auth(&updated).unwrap().access_token_exp,
            Some(4102449999)
        );
    }

    #[test]
    fn applies_rotated_refresh_token_when_present() {
        let original = fake_auth("acc-1", "one@example.com");
        let new_access = fake_jwt(serde_json::json!({
            "client_id": "app_test",
            "sub": "user-123",
            "exp": 4102449999_i64
        }));
        let updated = apply_token_refresh_response(
            &original,
            &serde_json::json!({
                "access_token": new_access,
                "refresh_token": "rt_new"
            }),
        )
        .unwrap();
        let auth: Value = serde_json::from_str(&updated).unwrap();

        assert_eq!(
            auth.pointer("/tokens/refresh_token")
                .and_then(Value::as_str),
            Some("rt_new")
        );
    }

    #[test]
    fn classifies_terminal_refresh_errors_as_relogin_required() {
        assert!(refresh_error_requires_relogin("refresh_token_reused"));
        assert!(refresh_error_requires_relogin(
            r#"{\"error\":\"invalid_grant\"}"#
        ));
        assert!(refresh_error_requires_relogin("token_invalidated"));
        assert!(!refresh_error_requires_relogin("network timeout"));
    }

    #[test]
    fn detects_usage_limits_from_nested_probe_body() {
        let body = serde_json::json!({
            "rate_limits": [
                {"window": "3h", "used": 12, "limit": 80, "remaining": 68, "reset_at": "2026-05-06T16:00:00Z"},
                {"period": "day", "usage": 30, "quota": 300, "available": 270}
            ]
        });

        let detected = detect_usage_limits(&body);

        assert_eq!(detected.len(), 2);
        assert!(detected
            .iter()
            .any(|item| item.window == "hour" && item.used == Some(12) && item.limit == Some(80)));
        assert!(detected
            .iter()
            .any(|item| item.window == "day" && item.used == Some(30) && item.limit == Some(300)));
    }

    #[test]
    fn detects_codex_wham_rate_limit_windows() {
        let body = serde_json::json!({
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                "primary_window": {
                    "used_percent": 53,
                    "limit_window_seconds": 18000,
                    "reset_at": 1778051942
                },
                "secondary_window": {
                    "used_percent": 8,
                    "limit_window_seconds": 604800,
                    "reset_at": 1778638742
                }
            }
        });

        let detected = detect_usage_limits(&body);

        assert!(detected.iter().any(|item| {
            item.window == "5小时"
                && item.used_percent == Some(53)
                && item.remaining_percent == Some(47)
                && item.reset_at.is_some()
        }));
        assert!(detected.iter().any(|item| {
            item.window == "1周"
                && item.used_percent == Some(8)
                && item.remaining_percent == Some(92)
        }));
    }

    #[test]
    fn applies_available_usage_reset_count() {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let mut profile = AccountProfile {
            id: "profile-1".to_string(),
            alias: "primary".to_string(),
            note: String::new(),
            enabled: true,
            priority: 100,
            cooldown_until: None,
            quota_rule: QuotaRule::default(),
            summary: summarize_auth(&fake_auth("acc-1", "one@example.com")).unwrap(),
            encrypted_auth_json: encrypt_secret(
                fake_auth("acc-1", "one@example.com").as_bytes(),
                &key,
            )
            .unwrap(),
            api_config: None,
            usage: UsageStats::default(),
            route_health: RouteHealth::default(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let body = serde_json::json!({
            "plan_type": "plus",
            "rate_limit": { "allowed": true, "limit_reached": false },
            "rate_limit_reset_credits": {
                "available_count": 2,
                "expires_at": "2026-08-01T00:00:00Z"
            }
        });

        apply_usage_probe_body(&mut profile, &body);

        assert_eq!(profile.usage.available_reset_count, Some(2));
        assert_eq!(
            profile.usage.available_reset_expires_at.as_deref(),
            Some("2026-08-01T00:00:00Z")
        );
        assert_eq!(profile.summary.plan.as_deref(), Some("plus"));
    }

    #[test]
    fn applies_probe_body_to_profile_usage_and_rules() {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let mut profile = AccountProfile {
            id: "profile-1".to_string(),
            alias: "primary".to_string(),
            note: String::new(),
            enabled: true,
            priority: 100,
            cooldown_until: None,
            quota_rule: QuotaRule::default(),
            summary: summarize_auth(&fake_auth("acc-1", "one@example.com")).unwrap(),
            encrypted_auth_json: encrypt_secret(
                fake_auth("acc-1", "one@example.com").as_bytes(),
                &key,
            )
            .unwrap(),
            api_config: None,
            usage: UsageStats::default(),
            route_health: RouteHealth::default(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let body = serde_json::json!({
            "hourly": {"used": 9, "limit": 50},
            "daily": {"remaining": 120, "limit": 200, "reset_at": "2026-05-07T00:00:00Z"}
        });

        apply_usage_probe_body(&mut profile, &body);

        assert_eq!(profile.usage.hourly_used, 9);
        assert_eq!(profile.quota_rule.hourly_limit, Some(50));
        assert_eq!(profile.usage.daily_used, 80);
        assert_eq!(profile.quota_rule.daily_limit, Some(200));
        assert_eq!(profile.usage.detected_limits.len(), 2);
        assert!(profile
            .usage
            .detected_summary
            .as_deref()
            .unwrap()
            .contains("hour"));
    }
}
