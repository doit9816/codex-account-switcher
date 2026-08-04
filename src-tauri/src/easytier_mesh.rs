use crate::{
    app_data_dir, decrypt_secret, display_err, encrypt_secret, load_master_key, load_store,
    now_string, push_event, save_store, AppStore, SecretEnvelope,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use easytier::common::config::{
    ConfigFileControl, ConfigLoader, NetworkIdentity, PeerConfig, TomlConfigLoader,
};
use easytier::instance_manager::NetworkInstanceManager;
use rand::rngs::OsRng;
use rand::RngCore;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tiny_http::{Header, Request, Response, StatusCode};

const MESH_SHARE_FORMAT: &str = "codex-switcher.mesh.v1";
const PUBLIC_NODE_CACHE_FILE: &str = "easytier-public-nodes.json";
const DEFAULT_NODE_SOURCE_URL: &str =
    "https://raw.githubusercontent.com/doit9816/codex-account-switcher/main/easytier-nodes.json";
const DEFAULT_STATUS_PATH: &str = "/uptime/status/easytier";
const DEFAULT_HEARTBEAT_PATH: &str = "/uptime/api/status-page/heartbeat/easytier";
const EASYTIER_DEFAULT_PORT: u16 = 11010;
const MAX_BOOTSTRAP_PEERS: usize = 8;
const MAX_NODE_PROBES: usize = 32;
const NODE_PROBE_TIMEOUT: Duration = Duration::from_millis(1200);
const MAX_MESH_SYNC_BYTES: usize = 256 * 1024 * 1024;

static MESH_RUNTIME: OnceLock<Mutex<Option<MeshRuntime>>> = OnceLock::new();
static MESH_LAST_RUNTIME_ERROR: OnceLock<Mutex<Option<String>>> = OnceLock::new();

struct MeshRuntime {
    manager: Arc<NetworkInstanceManager>,
    instance_id: uuid::Uuid,
    runtime_kind: String,
    started_at: String,
    peers: Vec<String>,
    refresh_stop: Arc<AtomicBool>,
    refresh_thread: Option<JoinHandle<()>>,
    account_sync_thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct MeshRuntimeSnapshot {
    running: bool,
    runtime_kind: Option<String>,
    process_id: Option<u32>,
    executable_path: Option<String>,
    peer_count: Option<usize>,
    virtual_ipv4: Option<String>,
    started_at: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshSettings {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) auto_start: bool,
    #[serde(default = "default_network_name")]
    pub(crate) network_name: String,
    #[serde(default)]
    pub(crate) encrypted_network_secret: Option<SecretEnvelope>,
    #[serde(default = "default_node_source_url")]
    pub(crate) node_source_url: String,
    #[serde(default = "default_node_refresh_secs")]
    pub(crate) node_refresh_secs: u64,
    #[serde(default)]
    pub(crate) sync_scope: MeshSyncScope,
    #[serde(default)]
    pub(crate) authorized_devices: Vec<MeshDevice>,
    #[serde(default)]
    pub(crate) cached_nodes: Vec<MeshPublicNode>,
    #[serde(default)]
    pub(crate) last_node_refresh_at: Option<String>,
    #[serde(default)]
    pub(crate) last_node_refresh_error: Option<String>,
}

impl Default for MeshSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_start: false,
            network_name: default_network_name(),
            encrypted_network_secret: None,
            node_source_url: default_node_source_url(),
            node_refresh_secs: default_node_refresh_secs(),
            sync_scope: MeshSyncScope::default(),
            authorized_devices: Vec::new(),
            cached_nodes: Vec::new(),
            last_node_refresh_at: None,
            last_node_refresh_error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshSyncScope {
    #[serde(default = "default_true")]
    pub(crate) accounts: bool,
    #[serde(default = "default_true")]
    pub(crate) rules: bool,
    #[serde(default)]
    pub(crate) routing: bool,
    #[serde(default)]
    pub(crate) conversations: bool,
}

impl Default for MeshSyncScope {
    fn default() -> Self {
        Self {
            accounts: true,
            rules: true,
            routing: false,
            conversations: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshDevice {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) address: Option<String>,
    #[serde(default)]
    pub(crate) last_seen_at: Option<String>,
    #[serde(default)]
    pub(crate) trusted: bool,
    #[serde(default)]
    pub(crate) sync_scope: MeshSyncScope,
    #[serde(default)]
    pub(crate) auto_account_sync: bool,
    #[serde(default)]
    pub(crate) encrypted_routing_api_key: Option<SecretEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshDeviceView {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) address: Option<String>,
    #[serde(default)]
    pub(crate) last_seen_at: Option<String>,
    pub(crate) trusted: bool,
    pub(crate) sync_scope: MeshSyncScope,
    pub(crate) auto_account_sync: bool,
}

impl From<&MeshDevice> for MeshDeviceView {
    fn from(device: &MeshDevice) -> Self {
        Self {
            id: device.id.clone(),
            name: device.name.clone(),
            address: device.address.clone(),
            last_seen_at: device.last_seen_at.clone(),
            trusted: device.trusted,
            sync_scope: device.sync_scope.clone(),
            auto_account_sync: device.auto_account_sync,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshPublicNode {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) address: String,
    #[serde(default)]
    pub(crate) group: Option<String>,
    #[serde(default)]
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) uptime24: Option<f64>,
    #[serde(default)]
    pub(crate) ping_ms: Option<f64>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshStatus {
    pub(crate) running: bool,
    pub(crate) settings: MeshSettingsView,
    pub(crate) public_nodes: Vec<MeshPublicNode>,
    pub(crate) devices: Vec<MeshDeviceView>,
    pub(crate) share_ready: bool,
    pub(crate) local_device_id: String,
    pub(crate) local_device_name: String,
    #[serde(default)]
    pub(crate) routing_base_url: Option<String>,
    #[serde(default)]
    pub(crate) runtime_kind: Option<String>,
    #[serde(default)]
    pub(crate) process_id: Option<u32>,
    #[serde(default)]
    pub(crate) runtime_binary_path: Option<String>,
    #[serde(default)]
    pub(crate) peer_count: Option<usize>,
    #[serde(default)]
    pub(crate) virtual_ipv4: Option<String>,
    #[serde(default)]
    pub(crate) started_at: Option<String>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshSettingsView {
    pub(crate) enabled: bool,
    pub(crate) auto_start: bool,
    pub(crate) network_name: String,
    pub(crate) node_source_url: String,
    pub(crate) node_refresh_secs: u64,
    pub(crate) sync_scope: MeshSyncScope,
    pub(crate) last_node_refresh_at: Option<String>,
    pub(crate) last_node_refresh_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshSaveSettingsInput {
    pub(crate) enabled: bool,
    pub(crate) auto_start: bool,
    pub(crate) network_name: String,
    pub(crate) network_secret: Option<String>,
    pub(crate) node_source_url: String,
    pub(crate) node_refresh_secs: u64,
    pub(crate) sync_scope: MeshSyncScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MeshShareMode {
    JoinOnly,
    MigrationBundle,
    ContinuousSync,
    RoutingApiShare,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshSharePayload {
    pub(crate) format: String,
    pub(crate) version: u32,
    pub(crate) mode: MeshShareMode,
    pub(crate) created_at: String,
    pub(crate) device_id: String,
    pub(crate) device_name: String,
    pub(crate) network_name: String,
    pub(crate) network_secret: String,
    pub(crate) node_source_url: String,
    pub(crate) peers: Vec<String>,
    pub(crate) sync_scope: MeshSyncScope,
    #[serde(default)]
    pub(crate) routing_base_url: Option<String>,
    #[serde(default)]
    pub(crate) routing_api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshImportResult {
    pub(crate) mode: MeshShareMode,
    pub(crate) device_id: String,
    pub(crate) device_name: String,
    pub(crate) imported_nodes: usize,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedPublicNodes {
    refreshed_at: String,
    nodes: Vec<MeshPublicNode>,
}

pub(crate) fn restore_enabled(app: AppHandle) {
    if load_store(&app)
        .map(|store| store.settings.mesh.enabled || store.settings.mesh.auto_start)
        .unwrap_or(false)
    {
        let _ = start(app);
    }
}

pub(crate) fn status(app: AppHandle) -> Result<MeshStatus, String> {
    let mut store = load_store(&app)?;
    if store.settings.mesh.cached_nodes.is_empty() {
        if let Ok(nodes) = read_cached_nodes(&app) {
            store.settings.mesh.cached_nodes = nodes;
        }
    }
    Ok(build_status(&app, &store))
}

pub(crate) fn save_settings(
    app: AppHandle,
    input: MeshSaveSettingsInput,
) -> Result<MeshStatus, String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let network_name = input.network_name.trim();
    if network_name.is_empty() {
        return Err("EasyTier network name cannot be empty".to_string());
    }
    store.settings.mesh.enabled = input.enabled;
    store.settings.mesh.auto_start = input.auto_start;
    store.settings.mesh.network_name = network_name.to_string();
    // The public source is an application-managed online configuration. Keep
    // it out of the user-facing settings so a stale or malformed URL cannot
    // break node discovery.
    store.settings.mesh.node_source_url = default_node_source_url();
    store.settings.mesh.node_refresh_secs = input.node_refresh_secs.clamp(60, 86_400);
    store.settings.mesh.sync_scope = input.sync_scope;
    if let Some(secret) = input
        .network_secret
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        store.settings.mesh.encrypted_network_secret =
            Some(encrypt_secret(secret.as_bytes(), &key)?);
    } else {
        ensure_network_secret(&mut store, &key)?;
    }
    push_event(&mut store, "info", "Mesh sharing settings saved");
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn start(app: AppHandle) -> Result<MeshStatus, String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    store.settings.mesh.node_source_url = default_node_source_url();
    ensure_network_secret(&mut store, &key)?;
    let network_secret = decrypt_network_secret(&store, &key)?;
    if store.settings.mesh.cached_nodes.is_empty() {
        store.settings.mesh.cached_nodes = read_cached_nodes(&app).unwrap_or_default();
    }
    let peers = mesh_peer_urls(&store.settings.mesh.cached_nodes);
    let runtime = start_embedded_runtime(&app, &store, &network_secret, peers)?;
    store.settings.mesh.enabled = true;
    set_runtime(runtime)?;
    set_last_runtime_error(None);
    push_event(&mut store, "info", "EasyTier mesh runtime started");
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn stop(app: AppHandle) -> Result<MeshStatus, String> {
    let mut store = load_store(&app)?;
    store.settings.mesh.enabled = false;
    stop_runtime();
    push_event(&mut store, "info", "EasyTier mesh runtime stopped");
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn refresh_public_nodes(app: AppHandle) -> Result<MeshStatus, String> {
    let mut store = load_store(&app)?;
    store.settings.mesh.node_source_url = default_node_source_url();
    match fetch_public_nodes(&store.settings.mesh.node_source_url) {
        Ok(nodes) => {
            store.settings.mesh.cached_nodes = nodes.clone();
            store.settings.mesh.last_node_refresh_at = Some(now_string());
            store.settings.mesh.last_node_refresh_error = None;
            write_cached_nodes(&app, &nodes)?;
            push_event(
                &mut store,
                "info",
                &format!("Mesh public nodes refreshed: {}", nodes.len()),
            );
        }
        Err(error) => {
            store.settings.mesh.last_node_refresh_at = Some(now_string());
            store.settings.mesh.last_node_refresh_error = Some(error.clone());
            if store.settings.mesh.cached_nodes.is_empty() {
                store.settings.mesh.cached_nodes = read_cached_nodes(&app).unwrap_or_default();
            }
            push_event(&mut store, "warn", "Mesh public node refresh failed");
        }
    }
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn create_share_payload(app: AppHandle, mode: MeshShareMode) -> Result<String, String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    store.settings.mesh.node_source_url = default_node_source_url();
    ensure_network_secret(&mut store, &key)?;
    let network_secret = decrypt_network_secret(&store, &key)?;
    let include_routing = matches!(
        mode,
        MeshShareMode::ContinuousSync | MeshShareMode::RoutingApiShare
    );
    let routing = routing_share(&store, include_routing, &key);
    let payload = MeshSharePayload {
        format: MESH_SHARE_FORMAT.to_string(),
        version: 1,
        mode,
        created_at: now_string(),
        device_id: local_device_id(),
        device_name: local_device_name(),
        network_name: store.settings.mesh.network_name.clone(),
        network_secret,
        node_source_url: default_node_source_url(),
        peers: store
            .settings
            .mesh
            .cached_nodes
            .iter()
            .filter(|node| node.status == "up")
            .filter_map(|node| normalize_peer_url(&node.address))
            .take(12)
            .collect(),
        sync_scope: store.settings.mesh.sync_scope.clone(),
        routing_base_url: routing.0,
        routing_api_key: routing.1,
    };
    save_store(&app, &store)?;
    encode_payload(&payload)
}

pub(crate) fn import_share_payload(
    app: AppHandle,
    payload_text: String,
) -> Result<MeshImportResult, String> {
    let payload = decode_payload(&payload_text)?;
    if payload.format != MESH_SHARE_FORMAT || payload.version != 1 {
        return Err("Unsupported mesh share payload".to_string());
    }
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    store.settings.mesh.network_name = payload.network_name.clone();
    store.settings.mesh.encrypted_network_secret =
        Some(encrypt_secret(payload.network_secret.as_bytes(), &key)?);
    store.settings.mesh.node_source_url = default_node_source_url();
    store.settings.mesh.sync_scope = payload.sync_scope.clone();
    merge_shared_peers(&mut store, &payload);
    upsert_device(
        &mut store,
        MeshDevice {
            id: payload.device_id.clone(),
            name: payload.device_name.clone(),
            address: payload.routing_base_url.clone(),
            last_seen_at: Some(now_string()),
            trusted: true,
            sync_scope: payload.sync_scope.clone(),
            auto_account_sync: payload.mode == MeshShareMode::ContinuousSync,
            encrypted_routing_api_key: payload
                .routing_api_key
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| encrypt_secret(value.as_bytes(), &key))
                .transpose()?,
        },
    );
    push_event(&mut store, "info", "Mesh share payload imported");
    save_store(&app, &store)?;
    Ok(MeshImportResult {
        mode: payload.mode,
        device_id: payload.device_id,
        device_name: payload.device_name,
        imported_nodes: payload.peers.len(),
        message: "Mesh share imported".to_string(),
    })
}

pub(crate) fn list_devices(app: AppHandle) -> Result<Vec<MeshDeviceView>, String> {
    Ok(load_store(&app)?
        .settings
        .mesh
        .authorized_devices
        .iter()
        .map(MeshDeviceView::from)
        .collect())
}

pub(crate) fn save_device_sync(
    app: AppHandle,
    device_id: String,
    trusted: bool,
    auto_account_sync: bool,
    sync_scope: MeshSyncScope,
) -> Result<MeshStatus, String> {
    let mut store = load_store(&app)?;
    let device = store
        .settings
        .mesh
        .authorized_devices
        .iter_mut()
        .find(|device| device.id == device_id)
        .ok_or_else(|| "Mesh device not found".to_string())?;
    device.trusted = trusted;
    device.auto_account_sync = auto_account_sync;
    device.sync_scope = sync_scope;
    push_event(&mut store, "info", "Mesh device sync settings saved");
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn sync_now(app: AppHandle, device_id: Option<String>) -> Result<MeshStatus, String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let targets = store
        .settings
        .mesh
        .authorized_devices
        .iter()
        .filter(|device| device.trusted)
        .filter(|device| {
            device.sync_scope.rules || device.sync_scope.routing || device.sync_scope.conversations
        })
        .filter(|device| device_id.as_deref().is_none_or(|id| id == device.id))
        .cloned()
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Err("没有可同步的受信任设备".to_string());
    }
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for device in targets {
        let scope = MeshSyncScope {
            accounts: false,
            rules: device.sync_scope.rules,
            routing: device.sync_scope.routing,
            conversations: device.sync_scope.conversations,
        };
        match sync_to_device_with_scope(&app, &key, &device, scope, false) {
            Ok(()) => succeeded += 1,
            Err(error) => {
                failed += 1;
                push_event(
                    &mut store,
                    "warn",
                    &format!("Mesh sync failed for {}: {error}", device.name),
                );
            }
        }
    }
    push_event(
        &mut store,
        if failed == 0 { "info" } else { "warn" },
        &format!("Mesh sync completed: {succeeded} succeeded, {failed} failed"),
    );
    save_store(&app, &store)?;
    status(app)
}

fn run_auto_account_sync(app: &AppHandle) {
    let Ok(key) = load_master_key(app) else {
        return;
    };
    let Ok(mut store) = load_store(app) else {
        return;
    };
    let targets = store
        .settings
        .mesh
        .authorized_devices
        .iter()
        .filter(|device| device.trusted && device.auto_account_sync)
        .cloned()
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return;
    }

    let scope = MeshSyncScope {
        accounts: true,
        rules: false,
        routing: false,
        conversations: false,
    };
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for device in targets {
        match sync_to_device_with_scope(app, &key, &device, scope.clone(), true) {
            Ok(()) => succeeded += 1,
            Err(error) => {
                failed += 1;
                push_event(
                    &mut store,
                    "warn",
                    &format!(
                        "Mesh automatic account sync failed for {}: {error}",
                        device.name
                    ),
                );
            }
        }
    }
    if succeeded > 0 || failed > 0 {
        push_event(
            &mut store,
            if failed == 0 { "info" } else { "warn" },
            &format!(
                "Mesh automatic account sync completed: {succeeded} succeeded, {failed} failed"
            ),
        );
        let _ = save_store(app, &store);
    }
}

fn sync_to_device_with_scope(
    app: &AppHandle,
    key: &[u8; 32],
    device: &MeshDevice,
    scope: MeshSyncScope,
    only_valid_accounts: bool,
) -> Result<(), String> {
    let base_url = device
        .address
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "target routing API address is missing".to_string())?;
    let encrypted_key = device
        .encrypted_routing_api_key
        .as_ref()
        .ok_or_else(|| "target routing API key is missing".to_string())?;
    let routing_api_key =
        String::from_utf8(decrypt_secret(encrypted_key, key)?).map_err(display_err)?;
    let password = migration_password_from_mesh(app, String::new(), true)?;
    let temp_path = std::env::temp_dir().join(format!(
        "codex-switcher-mesh-{}.zip.enc",
        uuid::Uuid::new_v4().simple()
    ));
    let export_result = crate::export_mesh_sync_bundle_internal(
        (*app).clone(),
        temp_path.to_string_lossy().to_string(),
        password,
        scope.conversations,
        scope.accounts,
        only_valid_accounts,
    );
    let result = export_result.and_then(|_| {
        let bytes = fs::read(&temp_path).map_err(display_err)?;
        let endpoint = sync_endpoint(base_url)?;
        let scope_header = serde_json::to_string(&scope).map_err(display_err)?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(display_err)?;
        let response = client
            .post(endpoint)
            .bearer_auth(routing_api_key)
            .header("X-Codex-Mesh-Scope", scope_header)
            .body(bytes)
            .send()
            .map_err(display_err)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "target returned HTTP {}",
                response.status().as_u16()
            ))
        }
    });
    let _ = fs::remove_file(temp_path);
    result
}

fn sync_endpoint(base_url: &str) -> Result<String, String> {
    let mut url = url::Url::parse(base_url).map_err(display_err)?;
    let path = url.path().trim_end_matches('/');
    let base_path = path.strip_suffix("/v1").unwrap_or(path);
    url.set_path(&format!("{base_path}/mesh/sync"));
    Ok(url.to_string())
}

pub(crate) fn handle_sync_request(app: AppHandle, mut request: Request) -> Result<(), String> {
    let key = load_master_key(&app)?;
    let store = load_store(&app)?;
    let expected = store
        .settings
        .routing
        .encrypted_access_key
        .as_ref()
        .and_then(|envelope| decrypt_secret(envelope, &key).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let provided = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Authorization"))
        .map(|header| header.value.as_str())
        .unwrap_or_default();
    if expected
        .as_deref()
        .map(|value| provided == format!("Bearer {value}"))
        != Some(true)
    {
        respond_sync_json(
            request,
            401,
            serde_json::json!({ "error": "invalid credentials" }),
        )?;
        return Ok(());
    }

    let scope = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("X-Codex-Mesh-Scope"))
        .and_then(|header| serde_json::from_str::<MeshSyncScope>(header.value.as_str()).ok())
        .unwrap_or_default();
    let mut bytes = Vec::new();
    request
        .as_reader()
        .take((MAX_MESH_SYNC_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(display_err)?;
    if bytes.len() > MAX_MESH_SYNC_BYTES {
        respond_sync_json(
            request,
            413,
            serde_json::json!({ "error": "bundle too large" }),
        )?;
        return Ok(());
    }
    let temp_path = std::env::temp_dir().join(format!(
        "codex-switcher-mesh-incoming-{}.zip.enc",
        uuid::Uuid::new_v4().simple()
    ));
    fs::write(&temp_path, bytes).map_err(display_err)?;
    let password = migration_password_from_mesh(&app, String::new(), true)?;
    let result = crate::import_accounts_bundle_with_scope(
        app,
        temp_path.to_string_lossy().to_string(),
        password,
        scope.conversations,
        None,
        Some(scope),
    );
    let _ = fs::remove_file(temp_path);
    match result {
        Ok(manifest) => respond_sync_json(
            request,
            200,
            serde_json::json!({
                "importedProfiles": manifest.imported_profiles,
                "restoredFiles": manifest.restored_files
            }),
        ),
        Err(error) => respond_sync_json(request, 422, serde_json::json!({ "error": error })),
    }
}

fn respond_sync_json(request: Request, status: u16, body: Value) -> Result<(), String> {
    let json = serde_json::to_string(&body).map_err(display_err)?;
    let content_type = Header::from_bytes("Content-Type", "application/json")
        .map_err(|_| "failed to build response header".to_string())?;
    request
        .respond(
            Response::from_string(json)
                .with_status_code(StatusCode(status))
                .with_header(content_type),
        )
        .map_err(display_err)
}

pub(crate) fn migration_password_from_mesh(
    app: &AppHandle,
    password: String,
    use_mesh_secret: bool,
) -> Result<String, String> {
    if !use_mesh_secret {
        return Ok(password);
    }
    let store = load_store(app)?;
    let key = load_master_key(app)?;
    let secret = decrypt_network_secret(&store, &key)?;
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":codex-switcher-migration-share:v1");
    Ok(URL_SAFE_NO_PAD.encode(hasher.finalize()))
}

fn build_status(_app: &AppHandle, store: &AppStore) -> MeshStatus {
    let runtime = runtime_snapshot();
    let routing_host = advertised_routing_host(
        &store.settings.routing.listen_host,
        runtime.virtual_ipv4.as_deref(),
    );
    MeshStatus {
        running: runtime.running,
        settings: MeshSettingsView {
            enabled: store.settings.mesh.enabled,
            auto_start: store.settings.mesh.auto_start,
            network_name: store.settings.mesh.network_name.clone(),
            node_source_url: default_node_source_url(),
            node_refresh_secs: store.settings.mesh.node_refresh_secs,
            sync_scope: store.settings.mesh.sync_scope.clone(),
            last_node_refresh_at: store.settings.mesh.last_node_refresh_at.clone(),
            last_node_refresh_error: store.settings.mesh.last_node_refresh_error.clone(),
        },
        public_nodes: store.settings.mesh.cached_nodes.clone(),
        devices: store
            .settings
            .mesh
            .authorized_devices
            .iter()
            .map(MeshDeviceView::from)
            .collect(),
        share_ready: store.settings.mesh.encrypted_network_secret.is_some(),
        local_device_id: local_device_id(),
        local_device_name: local_device_name(),
        routing_base_url: Some(format!(
            "http://{}:{}/v1",
            routing_host, store.settings.routing.port
        )),
        runtime_kind: runtime.runtime_kind,
        process_id: runtime.process_id,
        runtime_binary_path: runtime.executable_path,
        peer_count: runtime.peer_count,
        virtual_ipv4: runtime.virtual_ipv4,
        started_at: runtime.started_at,
        last_error: runtime.last_error,
    }
}

fn start_embedded_runtime(
    app: &AppHandle,
    store: &AppStore,
    network_secret: &str,
    peers: Vec<String>,
) -> Result<MeshRuntime, String> {
    stop_runtime();

    let config = build_easytier_config(store, network_secret, &peers)?;
    let manager = Arc::new(NetworkInstanceManager::new());
    let instance_id = manager
        .run_network_instance(config, false, ConfigFileControl::STATIC_CONFIG)
        .map_err(|error| {
            let message = format!("EasyTier runtime failed to start: {error}");
            set_last_runtime_error(Some(message.clone()));
            message
        })?;
    let refresh_stop = Arc::new(AtomicBool::new(false));
    let refresh_thread = Some(spawn_node_refresh_loop(
        app.clone(),
        refresh_stop.clone(),
        store.settings.mesh.node_refresh_secs,
    ));
    let account_sync_thread = Some(spawn_account_sync_loop(
        app.clone(),
        refresh_stop.clone(),
        store.settings.mesh.node_refresh_secs,
    ));

    Ok(MeshRuntime {
        manager,
        instance_id,
        runtime_kind: "embeddedSdk".to_string(),
        started_at: now_string(),
        peers,
        refresh_stop,
        refresh_thread,
        account_sync_thread,
    })
}

fn spawn_node_refresh_loop(
    app: AppHandle,
    stop: Arc<AtomicBool>,
    refresh_secs: u64,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let interval = refresh_secs.clamp(60, 86_400);
        loop {
            for _ in 0..interval {
                if stop.load(AtomicOrdering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            if stop.load(AtomicOrdering::Relaxed) {
                return;
            }
            let _ = refresh_public_nodes(app.clone());
        }
    })
}

fn spawn_account_sync_loop(
    app: AppHandle,
    stop: Arc<AtomicBool>,
    sync_secs: u64,
) -> JoinHandle<()> {
    thread::spawn(move || {
        run_auto_account_sync(&app);
        let interval = sync_secs.clamp(60, 86_400);
        loop {
            for _ in 0..interval {
                if stop.load(AtomicOrdering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_secs(1));
            }
            if stop.load(AtomicOrdering::Relaxed) {
                return;
            }
            run_auto_account_sync(&app);
        }
    })
}

fn build_easytier_config(
    store: &AppStore,
    network_secret: &str,
    peers: &[String],
) -> Result<TomlConfigLoader, String> {
    let config = TomlConfigLoader::new_from_str("").map_err(display_err)?;
    config.set_network_identity(NetworkIdentity::new(
        store.settings.mesh.network_name.clone(),
        network_secret.to_string(),
    ));
    config.set_inst_name(format!("codex-switcher-{}", local_device_id()));
    config.set_hostname(Some(local_device_name()));
    config.set_listeners(
        ["tcp://0.0.0.0:0", "udp://0.0.0.0:0"]
            .into_iter()
            .map(|value| value.parse().map_err(display_err))
            .collect::<Result<Vec<url::Url>, String>>()?,
    );
    let peer_configs = peers
        .iter()
        .filter_map(|peer| url::Url::parse(peer).ok().map(|uri| PeerConfig { uri }))
        .collect::<Vec<_>>();
    config.set_peers(peer_configs);

    // Run the VPN-capable SDK instance in its own EasyTier-managed runtime.
    // The default flags keep TUN enabled so the virtual IPv4 can be used by
    // the routing API and by other applications on this device.
    let mut flags = config.get_flags();
    flags.dev_name = "Codex Switcher Mesh".to_string();
    flags.no_tun = false;
    config.set_flags(flags);
    Ok(config)
}

fn runtime_holder() -> &'static Mutex<Option<MeshRuntime>> {
    MESH_RUNTIME.get_or_init(|| Mutex::new(None))
}

fn last_error_holder() -> &'static Mutex<Option<String>> {
    MESH_LAST_RUNTIME_ERROR.get_or_init(|| Mutex::new(None))
}

fn set_runtime(runtime: MeshRuntime) -> Result<(), String> {
    let mut holder = runtime_holder().lock().map_err(display_err)?;
    *holder = Some(runtime);
    Ok(())
}

fn stop_runtime() {
    let runtime = runtime_holder()
        .lock()
        .ok()
        .and_then(|mut holder| holder.take());
    if let Some(mut runtime) = runtime {
        runtime.refresh_stop.store(true, AtomicOrdering::Relaxed);
        if let Some(thread) = runtime.refresh_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = runtime.account_sync_thread.take() {
            let _ = thread.join();
        }
        let _ = runtime
            .manager
            .delete_network_instance(vec![runtime.instance_id]);
    }
}

fn set_last_runtime_error(error: Option<String>) {
    if let Ok(mut holder) = last_error_holder().lock() {
        *holder = error;
    }
}

fn runtime_snapshot() -> MeshRuntimeSnapshot {
    let last_error = last_error_holder()
        .lock()
        .ok()
        .and_then(|error| error.clone());
    let Ok(mut holder) = runtime_holder().lock() else {
        return MeshRuntimeSnapshot {
            last_error,
            ..MeshRuntimeSnapshot::default()
        };
    };
    let Some(runtime) = holder.as_mut() else {
        return MeshRuntimeSnapshot {
            last_error,
            ..MeshRuntimeSnapshot::default()
        };
    };
    let info = runtime
        .manager
        .collect_network_infos_sync()
        .ok()
        .and_then(|infos| infos.get(&runtime.instance_id).cloned());
    let runtime_error = info.as_ref().and_then(|value| value.error_msg.clone());
    let running = info.as_ref().map(|value| value.running).unwrap_or(true);
    let virtual_ipv4 = info
        .as_ref()
        .and_then(|value| value.my_node_info.as_ref())
        .and_then(|node| node.virtual_ipv4.as_ref())
        .map(ToString::to_string)
        .map(|value| value.split('/').next().unwrap_or(&value).to_string());
    let peer_count = info
        .as_ref()
        .map(|value| value.peers.len())
        .or_else(|| Some(runtime.peers.len()));
    if let Some(error) = runtime_error.clone() {
        set_last_runtime_error(Some(error));
    }
    MeshRuntimeSnapshot {
        running,
        runtime_kind: Some(runtime.runtime_kind.clone()),
        process_id: None,
        executable_path: None,
        peer_count,
        virtual_ipv4,
        started_at: Some(runtime.started_at.clone()),
        last_error: runtime_error.or(last_error),
    }
}

fn fetch_public_nodes(source_url: &str) -> Result<Vec<MeshPublicNode>, String> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(display_err)?;
    let source = source_url.trim().trim_end_matches('/');
    if source.ends_with(".json") || source.contains("raw.githubusercontent.com") {
        let payload: Value = client
            .get(source)
            .header(reqwest::header::USER_AGENT, "codex-account-switcher")
            .send()
            .map_err(display_err)?
            .error_for_status()
            .map_err(display_err)?
            .json()
            .map_err(display_err)?;
        return parse_public_nodes_json(&payload);
    }
    let origin = source
        .strip_suffix("/uptime/easytier")
        .unwrap_or(source)
        .trim_end_matches('/');
    let status_url = format!("{origin}{DEFAULT_STATUS_PATH}");
    let heartbeat_url = format!("{origin}{DEFAULT_HEARTBEAT_PATH}");
    let status_html = client
        .get(status_url)
        .header(reqwest::header::USER_AGENT, "codex-account-switcher")
        .send()
        .map_err(display_err)?
        .error_for_status()
        .map_err(display_err)?
        .text()
        .map_err(display_err)?;
    let heartbeat: Value = client
        .get(heartbeat_url)
        .header(reqwest::header::USER_AGENT, "codex-account-switcher")
        .send()
        .map_err(display_err)?
        .error_for_status()
        .map_err(display_err)?
        .json()
        .map_err(display_err)?;
    parse_public_nodes(&status_html, &heartbeat)
}

fn parse_public_nodes_json(payload: &Value) -> Result<Vec<MeshPublicNode>, String> {
    let nodes = payload
        .get("nodes")
        .ok_or_else(|| "node JSON does not contain nodes".to_string())?
        .clone();
    let nodes: Vec<MeshPublicNode> = serde_json::from_value(nodes).map_err(display_err)?;
    Ok(nodes
        .into_iter()
        .filter(|node| !node.address.trim().is_empty() && !node.address.contains('*'))
        .collect())
}

fn parse_public_nodes(status_html: &str, heartbeat: &Value) -> Result<Vec<MeshPublicNode>, String> {
    let groups = extract_public_group_list(status_html)?;
    let heartbeat_list = heartbeat
        .get("heartbeatList")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let uptime_list = heartbeat
        .get("uptimeList")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut nodes = Vec::new();
    for group in groups {
        let group_name = group
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("EasyTier")
            .to_string();
        let Some(monitors) = group.get("monitorList").and_then(Value::as_array) else {
            continue;
        };
        for monitor in monitors {
            let id = monitor
                .get("id")
                .map(|value| match value {
                    Value::String(value) => value.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| format!("node-{}", nodes.len() + 1));
            let raw_name = monitor
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let parsed = parse_node_name(raw_name, monitor);
            if parsed.address.is_empty() || parsed.address.contains('*') {
                continue;
            }
            let latest = heartbeat_list
                .get(&id)
                .and_then(Value::as_array)
                .and_then(|items| items.last());
            let is_up = latest
                .and_then(|item| item.get("status"))
                .and_then(Value::as_i64)
                == Some(1);
            let ping_ms = latest
                .and_then(|item| item.get("ping"))
                .and_then(Value::as_f64);
            let uptime24 = uptime_list
                .get(&format!("{id}_24"))
                .and_then(Value::as_f64)
                .map(|value| value * 100.0);
            nodes.push(MeshPublicNode {
                id,
                name: parsed.name,
                address: parsed.address,
                group: Some(group_name.clone()),
                status: if is_up { "up" } else { "down" }.to_string(),
                uptime24,
                ping_ms,
                tags: parsed.tags,
            });
        }
    }
    nodes.sort_by(|left, right| {
        right
            .status
            .cmp(&left.status)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(nodes)
}

fn extract_public_group_list(status_html: &str) -> Result<Vec<Value>, String> {
    // The status page emits a JavaScript object literal rather than JSON
    // (single-quoted strings and unquoted keys). Parse only the field we
    // need instead of assuming the whole script block is valid JSON.
    let marker = "publicGroupList";
    let marker_pos = status_html
        .find(marker)
        .ok_or_else(|| "publicGroupList not found".to_string())?;
    let array_start = status_html[marker_pos..]
        .find('[')
        .map(|offset| marker_pos + offset)
        .ok_or_else(|| "publicGroupList array not found".to_string())?;
    let mut parser = JsLiteralParser::new(&status_html[array_start..]);
    let value = parser.parse_value()?;
    value
        .as_array()
        .cloned()
        .ok_or_else(|| "publicGroupList is not an array".to_string())
}

struct JsLiteralParser<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> JsLiteralParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            position: 0,
        }
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'\'') | Some(b'"') => self.parse_string().map(Value::String),
            Some(_) => self.parse_bare_value(),
            None => Err("unexpected end of JavaScript value".to_string()),
        }
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        self.expect(b'{')?;
        let mut object = serde_json::Map::new();
        loop {
            self.skip_whitespace();
            if self.consume(b'}') {
                return Ok(Value::Object(object));
            }
            let key = match self.peek() {
                Some(b'\'') | Some(b'"') => self.parse_string()?,
                Some(_) => self.parse_identifier()?,
                None => return Err("unexpected end of JavaScript object".to_string()),
            };
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.parse_value()?;
            object.insert(key, value);
            self.skip_whitespace();
            if self.consume(b'}') {
                return Ok(Value::Object(object));
            }
            self.expect(b',')?;
        }
    }

    fn parse_array(&mut self) -> Result<Value, String> {
        self.expect(b'[')?;
        let mut values = Vec::new();
        loop {
            self.skip_whitespace();
            if self.consume(b']') {
                return Ok(Value::Array(values));
            }
            values.push(self.parse_value()?);
            self.skip_whitespace();
            if self.consume(b']') {
                return Ok(Value::Array(values));
            }
            self.expect(b',')?;
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        let quote = self
            .next()
            .ok_or_else(|| "unexpected end of JavaScript string".to_string())?;
        let mut value = String::new();
        while let Some(byte) = self.next() {
            if byte == quote {
                return Ok(value);
            }
            if byte != b'\\' {
                let start = self.position - 1;
                let character =
                    std::str::from_utf8(&self.input[start..self.position]).map_err(display_err)?;
                value.push_str(character);
                continue;
            }
            let escaped = self
                .next()
                .ok_or_else(|| "unfinished JavaScript escape".to_string())?;
            let character = match escaped {
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'b' => '\u{0008}',
                b'f' => '\u{000c}',
                b'\\' => '\\',
                b'/' => '/',
                b'\'' => '\'',
                b'"' => '"',
                b'u' => {
                    let code = self.read_hex_u16()?;
                    char::from_u32(code as u32)
                        .ok_or_else(|| "invalid JavaScript unicode escape".to_string())?
                }
                other => other as char,
            };
            value.push(character);
        }
        Err("unterminated JavaScript string".to_string())
    }

    fn parse_identifier(&mut self) -> Result<String, String> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'-') {
                self.position += 1;
            } else {
                break;
            }
        }
        if self.position == start {
            return Err(format!(
                "expected JavaScript identifier at {}",
                self.position
            ));
        }
        String::from_utf8(self.input[start..self.position].to_vec()).map_err(display_err)
    }

    fn parse_bare_value(&mut self) -> Result<Value, String> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_whitespace() || matches!(byte, b',' | b']' | b'}') {
                break;
            }
            self.position += 1;
        }
        if self.position == start {
            return Err(format!("expected JavaScript value at {}", self.position));
        }
        let token =
            String::from_utf8(self.input[start..self.position].to_vec()).map_err(display_err)?;
        match token.as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            "null" => Ok(Value::Null),
            _ => Ok(serde_json::from_str(&token).unwrap_or(Value::String(token))),
        }
    }

    fn read_hex_u16(&mut self) -> Result<u16, String> {
        if self.position + 4 > self.input.len() {
            return Err("short JavaScript unicode escape".to_string());
        }
        let digits = &self.input[self.position..self.position + 4];
        self.position += 4;
        let text = std::str::from_utf8(digits).map_err(display_err)?;
        u16::from_str_radix(text, 16).map_err(display_err)
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(format!(
                "expected '{}' at {}",
                expected as char, self.position
            ))
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.position += 1;
        Some(value)
    }
}

struct ParsedNodeName {
    name: String,
    address: String,
    tags: Vec<String>,
}

fn parse_node_name(raw_name: &str, monitor: &Value) -> ParsedNodeName {
    let mut tags = Vec::new();
    let mut name = raw_name.trim().to_string();
    while let Some(start) = name.find('[') {
        let Some(end) = name[start + 1..].find(']') else {
            break;
        };
        let tag = name[start + 1..start + 1 + end].trim();
        if !tag.is_empty() {
            tags.push(tag.to_string());
        }
        name.replace_range(start..start + end + 2, "");
    }
    let cleaned = clean_endpoint(&name);
    let address = normalize_peer_url(&cleaned)
        .or_else(|| {
            monitor
                .get("url")
                .and_then(Value::as_str)
                .filter(|_| monitor.get("sendUrl").and_then(Value::as_i64) == Some(1))
                .and_then(peer_url_from_url)
        })
        .unwrap_or(cleaned.clone());
    ParsedNodeName {
        name: cleaned,
        address,
        tags,
    }
}

fn peer_url_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port().unwrap_or(EASYTIER_DEFAULT_PORT);
    Some(format!("tcp://{host}:{port}"))
}

fn mesh_peer_urls(nodes: &[MeshPublicNode]) -> Vec<String> {
    let candidates = nodes
        .iter()
        .filter(|node| node.status == "up")
        .filter_map(|node| normalize_peer_url(&node.address))
        .fold(Vec::new(), |mut peers, peer| {
            if !peers.iter().any(|existing| existing == &peer) {
                peers.push(peer);
            }
            peers
        });
    let fallback = candidates
        .iter()
        .take(MAX_BOOTSTRAP_PEERS)
        .cloned()
        .collect::<Vec<_>>();

    let mut probe_handles = Vec::new();
    for (index, peer) in candidates.iter().take(MAX_NODE_PROBES).cloned().enumerate() {
        probe_handles.push(thread::spawn(move || {
            probe_peer_latency(&peer).map(|latency| (latency, index, peer))
        }));
    }

    let mut reachable = probe_handles
        .into_iter()
        .filter_map(|handle| handle.join().ok().flatten())
        .collect::<Vec<_>>();
    reachable.sort_by_key(|(latency, index, _)| (*latency, *index));

    let mut peers = reachable
        .into_iter()
        .map(|(_, _, peer)| peer)
        .take(MAX_BOOTSTRAP_PEERS)
        .collect::<Vec<_>>();
    for peer in fallback {
        if peers.len() >= MAX_BOOTSTRAP_PEERS {
            break;
        }
        if !peers.iter().any(|existing| existing == &peer) {
            peers.push(peer);
        }
    }
    peers
}

fn probe_peer_latency(peer: &str) -> Option<Duration> {
    let parsed = url::Url::parse(peer).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port().unwrap_or(EASYTIER_DEFAULT_PORT);
    let addresses = (host, port).to_socket_addrs().ok()?;
    let started = Instant::now();
    for address in addresses {
        if TcpStream::connect_timeout(&address, NODE_PROBE_TIMEOUT).is_ok() {
            return Some(started.elapsed());
        }
    }
    None
}

fn normalize_peer_url(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value.contains('*') {
        return None;
    }
    if let Ok(url) = url::Url::parse(value) {
        return is_supported_peer_scheme(url.scheme())
            .then(|| normalize_peer_url_with_default_port(url))
            .flatten();
    }
    if value.contains("://") {
        return None;
    }
    let candidate = if value.rsplit_once(':').is_some() {
        format!("tcp://{value}")
    } else {
        format!("tcp://{value}:{EASYTIER_DEFAULT_PORT}")
    };
    url::Url::parse(&candidate)
        .ok()
        .filter(|url| url.host_str().is_some())
        .map(|url| url.to_string())
}

fn normalize_peer_url_with_default_port(mut url: url::Url) -> Option<String> {
    url.host_str()?;
    if url.port().is_none() && url.set_port(Some(EASYTIER_DEFAULT_PORT)).is_err() {
        return None;
    }
    Some(url.to_string())
}

fn is_supported_peer_scheme(scheme: &str) -> bool {
    matches!(
        scheme,
        "tcp" | "udp" | "ws" | "wss" | "tcp+tls" | "quic" | "kcp"
    )
}

fn clean_endpoint(value: &str) -> String {
    value
        .replace("（", "(")
        .split('(')
        .next()
        .unwrap_or(value)
        .trim()
        .to_string()
}

fn ensure_network_secret(store: &mut AppStore, key: &[u8; 32]) -> Result<(), String> {
    if store.settings.mesh.encrypted_network_secret.is_none() {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        store.settings.mesh.encrypted_network_secret = Some(encrypt_secret(
            URL_SAFE_NO_PAD.encode(bytes).as_bytes(),
            key,
        )?);
    }
    Ok(())
}

fn decrypt_network_secret(store: &AppStore, key: &[u8; 32]) -> Result<String, String> {
    let envelope = store
        .settings
        .mesh
        .encrypted_network_secret
        .as_ref()
        .ok_or_else(|| "Mesh network secret has not been generated".to_string())?;
    String::from_utf8(decrypt_secret(envelope, key)?).map_err(display_err)
}

fn encode_payload(payload: &MeshSharePayload) -> Result<String, String> {
    let bytes = serde_json::to_vec(payload).map_err(display_err)?;
    Ok(format!(
        "codex-switcher-mesh:{}",
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

fn decode_payload(value: &str) -> Result<MeshSharePayload, String> {
    let trimmed = value.trim();
    let encoded = trimmed
        .strip_prefix("codex-switcher-mesh:")
        .unwrap_or(trimmed);
    let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(display_err)?;
    serde_json::from_slice(&bytes).map_err(display_err)
}

fn merge_shared_peers(store: &mut AppStore, payload: &MeshSharePayload) {
    for peer in &payload.peers {
        let Some(peer) = normalize_peer_url(peer) else {
            continue;
        };
        if store
            .settings
            .mesh
            .cached_nodes
            .iter()
            .any(|node| normalize_peer_url(&node.address).as_ref() == Some(&peer))
        {
            continue;
        }
        store.settings.mesh.cached_nodes.push(MeshPublicNode {
            id: hash_id(&peer),
            name: peer.clone(),
            address: peer,
            group: Some("Shared".to_string()),
            status: "up".to_string(),
            uptime24: None,
            ping_ms: None,
            tags: vec!["shared".to_string()],
        });
    }
}

fn upsert_device(store: &mut AppStore, device: MeshDevice) {
    if let Some(existing) = store
        .settings
        .mesh
        .authorized_devices
        .iter_mut()
        .find(|existing| existing.id == device.id)
    {
        *existing = device;
    } else {
        store.settings.mesh.authorized_devices.push(device);
    }
}

fn routing_share(
    store: &AppStore,
    include_key: bool,
    key: &[u8; 32],
) -> (Option<String>, Option<String>) {
    if !include_key {
        return (None, None);
    }
    let runtime = runtime_snapshot();
    let host = advertised_routing_host(
        &store.settings.routing.listen_host,
        runtime.virtual_ipv4.as_deref(),
    );
    let base_url = Some(format!(
        "http://{}:{}/v1",
        host, store.settings.routing.port
    ));
    let api_key = store
        .settings
        .routing
        .encrypted_access_key
        .as_ref()
        .and_then(|envelope| decrypt_secret(envelope, key).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok());
    (base_url, api_key)
}

fn display_mesh_host(host: &str) -> String {
    if host == "0.0.0.0" {
        "127.0.0.1".to_string()
    } else {
        host.to_string()
    }
}

fn advertised_routing_host(listen_host: &str, virtual_ipv4: Option<&str>) -> String {
    if matches!(listen_host, "0.0.0.0" | "::") {
        virtual_ipv4
            .map(ToString::to_string)
            .unwrap_or_else(|| display_mesh_host(listen_host))
    } else {
        display_mesh_host(listen_host)
    }
}

fn local_device_id() -> String {
    let name = local_device_name();
    hash_id(&format!("{}:{}", std::env::consts::OS, name))
}

fn local_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "CodexSwitcher Desktop".to_string())
}

fn hash_id(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    URL_SAFE_NO_PAD.encode(&hasher.finalize()[..12])
}

fn read_cached_nodes(app: &AppHandle) -> Result<Vec<MeshPublicNode>, String> {
    let path = app_data_dir(app)?.join(PUBLIC_NODE_CACHE_FILE);
    let text = std::fs::read_to_string(path).map_err(display_err)?;
    let cached: CachedPublicNodes = serde_json::from_str(&text).map_err(display_err)?;
    Ok(cached.nodes)
}

fn write_cached_nodes(app: &AppHandle, nodes: &[MeshPublicNode]) -> Result<(), String> {
    let path = app_data_dir(app)?.join(PUBLIC_NODE_CACHE_FILE);
    let cached = CachedPublicNodes {
        refreshed_at: now_string(),
        nodes: nodes.to_vec(),
    };
    let text = serde_json::to_string_pretty(&cached).map_err(display_err)?;
    std::fs::write(path, text).map_err(display_err)
}

fn default_network_name() -> String {
    "codex-switcher".to_string()
}

fn default_node_source_url() -> String {
    DEFAULT_NODE_SOURCE_URL.to_string()
}

fn default_node_refresh_secs() -> u64 {
    120
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_share_payload_round_trips() {
        let payload = MeshSharePayload {
            format: MESH_SHARE_FORMAT.to_string(),
            version: 1,
            mode: MeshShareMode::ContinuousSync,
            created_at: "2026-08-04T00:00:00Z".to_string(),
            device_id: "device".to_string(),
            device_name: "desktop".to_string(),
            network_name: "net".to_string(),
            network_secret: "secret".to_string(),
            node_source_url: DEFAULT_NODE_SOURCE_URL.to_string(),
            peers: vec!["tcp://example.com:11010".to_string()],
            sync_scope: MeshSyncScope::default(),
            routing_base_url: None,
            routing_api_key: None,
        };

        let encoded = encode_payload(&payload).unwrap();
        let decoded = decode_payload(&encoded).unwrap();

        assert_eq!(decoded.format, MESH_SHARE_FORMAT);
        assert_eq!(decoded.mode, MeshShareMode::ContinuousSync);
        assert_eq!(decoded.network_secret, "secret");
    }

    #[test]
    fn rejects_invalid_payload_text() {
        assert!(decode_payload("not-a-payload").is_err());
    }

    #[test]
    fn builds_mesh_sync_endpoint_from_routing_url() {
        assert_eq!(
            sync_endpoint("http://10.126.126.2:15722/v1").unwrap(),
            "http://10.126.126.2:15722/mesh/sync"
        );
    }

    #[test]
    fn parses_single_quoted_status_page_literal() {
        let html = r#"<script>window.preloadData = {'config':{},'publicGroupList':[{'name':'EasyTier','monitorList':[{'id':1,'name':'tcp://node.example.com:11010','sendUrl':0}]}]};</script>"#;
        let heartbeat = serde_json::json!({
            "heartbeatList": {"1": [{"status": 1, "ping": 42.0}]},
            "uptimeList": {"1_24": 0.98}
        });

        let nodes = parse_public_nodes(html, &heartbeat).unwrap();

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].status, "up");
        assert_eq!(nodes[0].ping_ms, Some(42.0));
    }

    #[test]
    fn parses_published_node_json() {
        let payload = serde_json::json!({
            "format": "codex-switcher.easytier-nodes.v1",
            "nodes": [{
                "id": "1",
                "name": "tcp://node.example.com:11010",
                "address": "tcp://node.example.com:11010",
                "status": "up",
                "uptime24": 99.0,
                "pingMs": 12.0,
                "tags": []
            }]
        });

        let nodes = parse_public_nodes_json(&payload).unwrap();

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].address, "tcp://node.example.com:11010");
    }

    #[test]
    fn parses_public_nodes_from_preload_and_heartbeat() {
        let html = r#"<script>window.preloadData = {"publicGroupList":[{"name":"EasyTier","monitorList":[{"id":1,"name":"[海外] tcp://node.example.com:11010（可中转）"}]}]};</script>"#;
        let heartbeat = serde_json::json!({
            "heartbeatList": {"1": [{"status": 1, "ping": 33.0}]},
            "uptimeList": {"1_24": 0.99}
        });

        let nodes = parse_public_nodes(html, &heartbeat).unwrap();

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].status, "up");
        assert!(nodes[0].address.contains("node.example.com"));
    }
}
