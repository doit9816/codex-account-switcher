use crate::{
    app_data_dir, append_app_log, decrypt_secret, display_err, encrypt_secret, load_master_key,
    load_store, now_string, parse_time, push_event, save_store, AppStore, SecretEnvelope,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use easytier::common::config::{
    ConfigFileControl, ConfigLoader, NetworkIdentity, PeerConfig, TomlConfigLoader,
};
use easytier::instance_manager::NetworkInstanceManager;
use easytier::launcher::NetworkInstanceRunningInfo;
use rand::rngs::OsRng;
use rand::RngCore;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
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
const MESH_GROUP_LEGACY_ID: &str = "legacy";
const MESH_GROUP_DEFAULT_CIDR: &str = "10.126.126.0/24";
const MESH_GROUP_PORT_STEP: u16 = 10;
const MESH_GROUP_SOCKS5_PORT_BASE: u16 = 22333;
const MAX_BOOTSTRAP_PEERS: usize = 8;
const MAX_NODE_PROBES: usize = 32;
const NODE_PROBE_TIMEOUT: Duration = Duration::from_millis(1200);
const MAX_MESH_SYNC_BYTES: usize = 256 * 1024 * 1024;
const MESH_HELLO_PATH: &str = "/mesh/hello";
const MESH_HELLO_TOKEN_HEADER: &str = "X-Codex-Mesh-Token";
const MESH_GROUP_ID_HEADER: &str = "X-Codex-Mesh-Group-Id";
const MESH_HELLO_DEVICE_ID_HEADER: &str = "X-Codex-Mesh-Device-Id";
const MESH_DEVICE_CREDENTIAL_HEADER: &str = "X-Codex-Mesh-Device-Credential";
const MESH_HELLO_DEVICE_NAME_HEADER: &str = "X-Codex-Mesh-Device-Name";
const MESH_HELLO_VIRTUAL_IPV4_HEADER: &str = "X-Codex-Mesh-Virtual-Ipv4";
const MESH_HELLO_ROUTING_URL_HEADER: &str = "X-Codex-Mesh-Routing-Url";
const MESH_HELLO_ROUTING_KEY_HEADER: &str = "X-Codex-Mesh-Routing-Key";
const MESH_HELLO_SYNC_SCOPE_HEADER: &str = "X-Codex-Mesh-Sync-Scope";
const MESH_HELLO_TOKEN_SUFFIX: &[u8] = b":codex-switcher-mesh-hello:v1";
const MESH_HELLO_TIMEOUT: Duration = Duration::from_secs(4);
// Discovery runs every three seconds. Keep a successful hello visible for a
// few missed probes so a transient SDK/status failure does not make a device
// disappear from the UI immediately.
const MESH_ONLINE_TTL_SECS: i64 = 15;
// Internal-only loopback proxy used for outbound requests to mesh virtual IPs.
// It is intentionally not persisted or exposed in the UI/share payload.
static MESH_RUNTIMES: OnceLock<Mutex<BTreeMap<String, MeshRuntime>>> = OnceLock::new();
static MESH_MANAGER: OnceLock<Arc<NetworkInstanceManager>> = OnceLock::new();
static MESH_LAST_RUNTIME_ERRORS: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
static MESH_AUTO_ACCOUNT_SYNC_RUNNING: AtomicBool = AtomicBool::new(false);
static MESH_RUNTIME_INFO_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
// Mesh account sync and token refresh must not run concurrently. Refresh
// tokens can be rotated by the provider; exporting/importing a stale copy
// while another task refreshes it can make the other copy look reused.
static MESH_DATA_SYNC_ACTIVE: AtomicBool = AtomicBool::new(false);
static PROFILE_REFRESH_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
// Importing a share code can race with startup restoration or a manual start.
// EasyTier does not support creating/replacing the same network instance from
// two tasks at once, so serialize lifecycle operations at the app boundary.
static MESH_LIFECYCLE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) struct ProfileRefreshGuard;

impl Drop for ProfileRefreshGuard {
    fn drop(&mut self) {
        PROFILE_REFRESH_IN_FLIGHT.fetch_sub(1, AtomicOrdering::AcqRel);
    }
}

pub(crate) struct MeshDataSyncGuard;

impl Drop for MeshDataSyncGuard {
    fn drop(&mut self) {
        MESH_DATA_SYNC_ACTIVE.store(false, AtomicOrdering::Release);
    }
}

pub(crate) fn begin_profile_refresh() -> Option<ProfileRefreshGuard> {
    if MESH_DATA_SYNC_ACTIVE.load(AtomicOrdering::Acquire) {
        return None;
    }
    PROFILE_REFRESH_IN_FLIGHT.fetch_add(1, AtomicOrdering::AcqRel);
    if MESH_DATA_SYNC_ACTIVE.load(AtomicOrdering::Acquire) {
        PROFILE_REFRESH_IN_FLIGHT.fetch_sub(1, AtomicOrdering::AcqRel);
        return None;
    }
    Some(ProfileRefreshGuard)
}

fn begin_mesh_data_sync() -> Result<MeshDataSyncGuard, String> {
    MESH_DATA_SYNC_ACTIVE
        .compare_exchange(false, true, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
        .map_err(|_| "账号同步正在进行，请稍后重试".to_string())?;

    while PROFILE_REFRESH_IN_FLIGHT.load(AtomicOrdering::Acquire) != 0 {
        thread::sleep(Duration::from_millis(50));
    }
    Ok(MeshDataSyncGuard)
}

struct MeshRuntime {
    group_id: String,
    manager: Arc<NetworkInstanceManager>,
    instance_id: uuid::Uuid,
    runtime_kind: String,
    started_at: String,
    virtual_ipv4: String,
    virtual_cidr: String,
    socks5_port: u16,
    peers: Vec<String>,
    refresh_stop: Arc<AtomicBool>,
    refresh_thread: Option<JoinHandle<()>>,
    account_sync_thread: Option<JoinHandle<()>>,
    peer_discovery_thread: Option<JoinHandle<()>>,
    routing_thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct MeshRuntimeSnapshot {
    running: bool,
    runtime_kind: Option<String>,
    process_id: Option<u32>,
    executable_path: Option<String>,
    peer_count: Option<usize>,
    peers: Vec<MeshPeerView>,
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
    #[serde(default)]
    pub(crate) encrypted_local_device_credential: Option<SecretEnvelope>,
    #[serde(default = "default_node_source_url")]
    pub(crate) node_source_url: String,
    #[serde(default = "default_node_refresh_secs")]
    pub(crate) node_refresh_secs: u64,
    #[serde(default)]
    pub(crate) sync_scope: MeshSyncScope,
    #[serde(default)]
    pub(crate) sync_scope_initialized: bool,
    #[serde(default)]
    pub(crate) authorized_devices: Vec<MeshDevice>,
    #[serde(default)]
    pub(crate) cached_nodes: Vec<MeshPublicNode>,
    #[serde(default)]
    pub(crate) last_node_refresh_at: Option<String>,
    #[serde(default)]
    pub(crate) last_node_refresh_error: Option<String>,
    /// New multi-group configuration. The fields above are retained as the
    /// legacy-group compatibility surface and are synchronized on load/save.
    #[serde(default)]
    pub(crate) groups: Vec<MeshShareGroup>,
}

impl Default for MeshSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_start: false,
            network_name: default_network_name(),
            encrypted_network_secret: None,
            encrypted_local_device_credential: None,
            node_source_url: default_node_source_url(),
            node_refresh_secs: default_node_refresh_secs(),
            sync_scope: MeshSyncScope::default(),
            sync_scope_initialized: false,
            authorized_devices: Vec::new(),
            cached_nodes: Vec::new(),
            last_node_refresh_at: None,
            last_node_refresh_error: None,
            groups: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshGroupRuntimeState {
    #[serde(default)]
    pub(crate) running: bool,
    #[serde(default)]
    pub(crate) instance_id: Option<String>,
    #[serde(default)]
    pub(crate) virtual_ipv4: Option<String>,
    #[serde(default)]
    pub(crate) started_at: Option<String>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
}

impl Default for MeshGroupRuntimeState {
    fn default() -> Self {
        Self {
            running: false,
            instance_id: None,
            virtual_ipv4: None,
            started_at: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshCredentialGrant {
    pub(crate) fingerprint: String,
    pub(crate) created_at: String,
    #[serde(default)]
    pub(crate) bound_device_id: Option<String>,
    #[serde(default)]
    pub(crate) revoked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshShareGroup {
    pub(crate) group_id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) encrypted_network_secret: Option<SecretEnvelope>,
    #[serde(default)]
    pub(crate) nodes: Vec<MeshPublicNode>,
    #[serde(default)]
    pub(crate) authorized_devices: Vec<MeshDevice>,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) auto_start: bool,
    #[serde(default)]
    pub(crate) sync_scope: MeshSyncScope,
    #[serde(default = "default_mesh_group_cidr")]
    pub(crate) virtual_cidr: String,
    #[serde(default = "default_mesh_group_port")]
    pub(crate) listen_port: u16,
    #[serde(default = "default_mesh_group_socks5_port")]
    pub(crate) socks5_port: u16,
    #[serde(default)]
    pub(crate) runtime: MeshGroupRuntimeState,
    /// Only hashes are persisted; the credential itself is encrypted below.
    #[serde(default)]
    pub(crate) credential_grants: Vec<MeshCredentialGrant>,
    #[serde(default)]
    pub(crate) encrypted_local_device_credential: Option<SecretEnvelope>,
    #[serde(default)]
    pub(crate) legacy_compat: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshGroupView {
    pub(crate) group_id: String,
    pub(crate) name: String,
    pub(crate) enabled: bool,
    pub(crate) auto_start: bool,
    pub(crate) sync_scope: MeshSyncScope,
    pub(crate) node_count: usize,
    pub(crate) device_count: usize,
    pub(crate) online_device_count: usize,
    pub(crate) virtual_cidr: String,
    pub(crate) listen_port: u16,
    pub(crate) socks5_port: u16,
    pub(crate) runtime: MeshGroupRuntimeState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshGroupStatus {
    pub(crate) group: MeshGroupView,
    pub(crate) devices: Vec<MeshDeviceView>,
    pub(crate) peers: Vec<MeshPeerView>,
    pub(crate) local_device_id: String,
    pub(crate) local_device_name: String,
    #[serde(default)]
    pub(crate) routing_base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshSaveGroupInput {
    #[serde(default)]
    pub(crate) group_id: Option<String>,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) network_secret: Option<String>,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) auto_start: bool,
    #[serde(default)]
    pub(crate) sync_scope: MeshSyncScope,
    #[serde(default)]
    pub(crate) virtual_cidr: Option<String>,
    #[serde(default)]
    pub(crate) listen_port: Option<u16>,
    #[serde(default)]
    pub(crate) socks5_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshSyncScope {
    #[serde(default = "default_true")]
    pub(crate) accounts: bool,
    #[serde(default)]
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
            rules: false,
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
    pub(crate) allowed_sync_scope: Option<MeshSyncScope>,
    #[serde(default)]
    pub(crate) auto_account_sync: bool,
    #[serde(default)]
    pub(crate) encrypted_routing_api_key: Option<SecretEnvelope>,
    #[serde(default)]
    pub(crate) encrypted_mesh_credential: Option<SecretEnvelope>,
    #[serde(default)]
    pub(crate) credential_fingerprint: Option<String>,
    #[serde(default)]
    pub(crate) revoked_at: Option<String>,
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
    pub(crate) allowed_sync_scope: MeshSyncScope,
    pub(crate) auto_account_sync: bool,
    #[serde(default)]
    pub(crate) revoked_at: Option<String>,
    #[serde(default)]
    pub(crate) online: bool,
    #[serde(default)]
    pub(crate) ip: Option<String>,
    #[serde(default)]
    pub(crate) latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshPeerView {
    pub(crate) peer_id: u32,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) ip: Option<String>,
    #[serde(default)]
    pub(crate) latency_ms: Option<f64>,
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
            allowed_sync_scope: device
                .allowed_sync_scope
                .clone()
                .unwrap_or_else(|| device.sync_scope.clone()),
            auto_account_sync: device.auto_account_sync,
            revoked_at: device.revoked_at.clone(),
            online: false,
            ip: None,
            latency_ms: None,
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
    #[serde(default)]
    pub(crate) peers: Vec<MeshPeerView>,
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
    #[serde(default)]
    pub(crate) groups: Vec<MeshGroupView>,
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
    #[serde(default)]
    pub(crate) group_id: Option<String>,
    #[serde(default)]
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) device_credential: Option<String>,
    #[serde(default)]
    pub(crate) credential_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeshHello {
    format: String,
    version: u32,
    device_id: String,
    device_name: String,
    virtual_ipv4: Option<String>,
    routing_base_url: Option<String>,
    routing_api_key: Option<String>,
    sync_scope: MeshSyncScope,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    device_credential: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshImportResult {
    pub(crate) group_id: String,
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
    let group_ids = load_store(&app)
        .map(|store| {
            store
                .settings
                .mesh
                .groups
                .iter()
                .filter(|group| group.enabled || group.auto_start)
                .map(|group| group.group_id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for group_id in group_ids {
        let app = app.clone();
        thread::spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| start_group(app.clone(), group_id)));
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => append_app_log(
                    &app,
                    "error",
                    &format!("automatic mesh startup failed: {error}"),
                ),
                Err(panic) => append_app_log(
                    &app,
                    "error",
                    &format!(
                        "automatic mesh startup panicked: {}",
                        panic_payload_message(panic.as_ref())
                    ),
                ),
            }
        });
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
        return Err("共享名称不能为空".to_string());
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
    store.settings.mesh.sync_scope_initialized = true;
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
    push_event(&mut store, "info", "多设备共享设置已保存");
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn start(app: AppHandle) -> Result<MeshStatus, String> {
    let _lifecycle_guard = mesh_lifecycle_guard();
    crate::routing::start_for_mesh_share(app.clone())?;
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    store.settings.mesh.node_source_url = default_node_source_url();
    ensure_network_secret(&mut store, &key)?;
    ensure_local_device_credential(&mut store, &key)?;
    let network_secret = decrypt_network_secret(&store, &key)?;
    if store.settings.mesh.cached_nodes.is_empty() {
        store.settings.mesh.cached_nodes = read_cached_nodes(&app).unwrap_or_default();
    }
    let peers = mesh_peer_urls(&store.settings.mesh.cached_nodes);
    let runtime = start_embedded_runtime(&app, &store, &network_secret, peers)?;
    store.settings.mesh.enabled = true;
    set_runtime(runtime)?;
    set_last_runtime_error(None);
    push_event(&mut store, "info", "设备连接已建立");
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn stop(app: AppHandle) -> Result<MeshStatus, String> {
    let _lifecycle_guard = mesh_lifecycle_guard();
    let mut store = load_store(&app)?;
    store.settings.mesh.enabled = false;
    stop_runtime();
    push_event(&mut store, "info", "设备连接已断开");
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn list_groups(app: AppHandle) -> Result<Vec<MeshGroupView>, String> {
    let store = load_store(&app)?;
    Ok(mesh_group_views(&store))
}

pub(crate) fn group_status(app: AppHandle, group_id: String) -> Result<MeshGroupStatus, String> {
    let store = load_store(&app)?;
    build_group_status(&store, &group_id)
}

pub(crate) fn create_group(
    app: AppHandle,
    input: MeshSaveGroupInput,
) -> Result<MeshGroupStatus, String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let group_id = input
        .group_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("group-{}", uuid::Uuid::new_v4().simple()));
    if group_id == MESH_GROUP_LEGACY_ID {
        return Err("the legacy group is managed by the compatibility commands".to_string());
    }
    let name = input.name.trim();
    if name.is_empty() {
        return Err("group name cannot be empty".to_string());
    }
    if store
        .settings
        .mesh
        .groups
        .iter()
        .any(|group| group.group_id != group_id && group.name.eq_ignore_ascii_case(name))
    {
        return Err("network name is already used by another group".to_string());
    }
    let existing_index = store
        .settings
        .mesh
        .groups
        .iter()
        .position(|group| group.group_id == group_id);
    let virtual_cidr = input.virtual_cidr.clone().unwrap_or_else(|| {
        existing_index
            .map(|index| store.settings.mesh.groups[index].virtual_cidr.clone())
            .unwrap_or_else(|| allocate_group_cidr(&store.settings.mesh.groups))
    });
    validate_group_cidr(&virtual_cidr, &store.settings.mesh.groups, Some(&group_id))?;
    let listen_port = input.listen_port.unwrap_or_else(|| {
        existing_index
            .map(|index| store.settings.mesh.groups[index].listen_port)
            .unwrap_or_else(|| allocate_group_listen_port(&store.settings.mesh.groups))
    });
    let socks5_port = input.socks5_port.unwrap_or_else(|| {
        existing_index
            .map(|index| store.settings.mesh.groups[index].socks5_port)
            .unwrap_or_else(|| allocate_group_socks5_port(&store.settings.mesh.groups))
    });
    validate_group_ports(
        listen_port,
        socks5_port,
        &store.settings.mesh.groups,
        Some(&group_id),
    )?;
    let encrypted_network_secret = match input
        .network_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(secret) => Some(encrypt_secret(secret.as_bytes(), &key)?),
        None => existing_index
            .and_then(|index| {
                store.settings.mesh.groups[index]
                    .encrypted_network_secret
                    .clone()
            })
            .or_else(|| random_encrypted_secret(&key).ok()),
    };
    let encrypted_network_secret = encrypted_network_secret
        .ok_or_else(|| "failed to generate group network secret".to_string())?;
    let mut group = existing_index
        .map(|index| store.settings.mesh.groups[index].clone())
        .unwrap_or_else(|| MeshShareGroup {
            group_id: group_id.clone(),
            name: name.to_string(),
            encrypted_network_secret: None,
            nodes: store.settings.mesh.cached_nodes.clone(),
            authorized_devices: Vec::new(),
            enabled: false,
            auto_start: false,
            sync_scope: MeshSyncScope::default(),
            virtual_cidr: virtual_cidr.clone(),
            listen_port,
            socks5_port,
            runtime: MeshGroupRuntimeState::default(),
            credential_grants: Vec::new(),
            encrypted_local_device_credential: None,
            legacy_compat: false,
        });
    group.name = name.to_string();
    group.encrypted_network_secret = Some(encrypted_network_secret);
    group.enabled = input.enabled;
    group.auto_start = input.auto_start;
    group.sync_scope = input.sync_scope;
    group.virtual_cidr = virtual_cidr;
    group.listen_port = listen_port;
    group.socks5_port = socks5_port;
    let should_schedule_account_sync = group.sync_scope.accounts;
    if let Some(index) = existing_index {
        store.settings.mesh.groups[index] = group;
    } else {
        store.settings.mesh.groups.push(group);
    }
    push_event(&mut store, "info", &format!("mesh group saved: {name}"));
    save_store(&app, &store)?;
    if input.enabled {
        start_group(app.clone(), group_id.clone())?;
    }
    if should_schedule_account_sync && group_runtime_snapshot(&group_id).running {
        let sync_app = app.clone();
        let sync_group_id = group_id.clone();
        let _ = thread::Builder::new()
            .name("mesh-group-account-sync-now".to_string())
            .spawn(move || run_group_auto_account_sync(&sync_app, &sync_group_id));
    }
    group_status(app, group_id)
}

pub(crate) fn start_group(app: AppHandle, group_id: String) -> Result<MeshGroupStatus, String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        start(app.clone())?;
        return group_status(app, group_id);
    }
    let _lifecycle_guard = mesh_lifecycle_guard();
    crate::routing::start_for_mesh_share(app.clone())?;
    if group_runtime_snapshot(&group_id).running {
        return group_status(app, group_id);
    }
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let group = store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())?;
    let secret = decrypt_group_network_secret(group, &key)?;
    let _ = ensure_group_local_device_credential(group, &key)?;
    let peers = mesh_peer_urls(&group.nodes);
    let runtime = start_embedded_group_runtime(&app, group, &secret, peers)?;
    group.enabled = true;
    group.runtime = runtime_state_from_runtime(&runtime);
    set_runtime(runtime)?;
    set_group_runtime_error(&group_id, None);
    push_event(
        &mut store,
        "info",
        &format!("mesh group started: {group_id}"),
    );
    save_store(&app, &store)?;
    group_status(app, group_id)
}

pub(crate) fn stop_group(app: AppHandle, group_id: String) -> Result<MeshGroupStatus, String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        stop(app.clone())?;
        return group_status(app, group_id);
    }
    let _lifecycle_guard = mesh_lifecycle_guard();
    let mut store = load_store(&app)?;
    let group = store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())?;
    group.enabled = false;
    group.runtime = MeshGroupRuntimeState::default();
    stop_group_runtime(&group_id);
    push_event(
        &mut store,
        "info",
        &format!("mesh group stopped: {group_id}"),
    );
    save_store(&app, &store)?;
    group_status(app, group_id)
}

pub(crate) fn revoke_group_device(
    app: AppHandle,
    group_id: String,
    device_id: String,
    revoked: bool,
) -> Result<MeshGroupStatus, String> {
    let mut store = load_store(&app)?;
    let devices = group_devices_mut(&mut store, &group_id)?;
    let device = devices
        .iter_mut()
        .find(|device| device.id == device_id)
        .ok_or_else(|| "device not found in mesh group".to_string())?;
    device.trusted = !revoked;
    device.revoked_at = revoked.then(now_string);
    let fingerprint = device.credential_fingerprint.clone();
    if let Some(fingerprint) = fingerprint.as_deref() {
        if let Some(group) = store
            .settings
            .mesh
            .groups
            .iter_mut()
            .find(|group| group.group_id == group_id)
        {
            if let Some(grant) = group
                .credential_grants
                .iter_mut()
                .find(|grant| grant.fingerprint == fingerprint)
            {
                grant.revoked_at = revoked.then(now_string);
            }
        }
    }
    save_store(&app, &store)?;
    group_status(app, group_id)
}

pub(crate) fn remove_group_device(
    app: AppHandle,
    group_id: String,
    device_id: String,
) -> Result<MeshGroupStatus, String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        return Err("legacy mesh group does not support removing devices".to_string());
    }
    let mut store = load_store(&app)?;
    let removed_fingerprint = {
        let group = store
            .settings
            .mesh
            .groups
            .iter_mut()
            .find(|group| group.group_id == group_id)
            .ok_or_else(|| "mesh group not found".to_string())?;
        let index = group
            .authorized_devices
            .iter()
            .position(|device| device.id == device_id)
            .ok_or_else(|| "device not found in mesh group".to_string())?;
        group
            .authorized_devices
            .remove(index)
            .credential_fingerprint
    };
    if let Some(group) = store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
    {
        group.credential_grants.retain(|grant| {
            grant.bound_device_id.as_deref() != Some(device_id.as_str())
                && removed_fingerprint
                    .as_deref()
                    .map(|fingerprint| grant.fingerprint != fingerprint)
                    .unwrap_or(true)
        });
    }
    push_event(
        &mut store,
        "info",
        &format!("设备已从分享组移除：{group_id} / {device_id}"),
    );
    save_store(&app, &store)?;
    group_status(app, group_id)
}

pub(crate) fn save_group_device_sync(
    app: AppHandle,
    group_id: String,
    device_id: String,
    auto_account_sync: bool,
    sync_scope: MeshSyncScope,
) -> Result<MeshGroupStatus, String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        let trusted = load_store(&app)?
            .settings
            .mesh
            .authorized_devices
            .iter()
            .find(|device| device.id == device_id)
            .map(|device| device.trusted)
            .ok_or_else(|| "device not found in mesh group".to_string())?;
        save_device_sync(
            app.clone(),
            device_id,
            trusted,
            auto_account_sync,
            sync_scope,
        )?;
        return group_status(app, group_id);
    }
    let mut store = load_store(&app)?;
    let devices = group_devices_mut(&mut store, &group_id)?;
    let device = devices
        .iter_mut()
        .find(|device| device.id == device_id)
        .ok_or_else(|| "device not found in mesh group".to_string())?;
    let allowed_scope = device_allowed_scope(device);
    device.sync_scope = intersect_mesh_scopes(&sync_scope, &allowed_scope);
    device.allowed_sync_scope = Some(allowed_scope.clone());
    device.auto_account_sync = auto_account_sync && allowed_scope.accounts;
    save_store(&app, &store)?;
    group_status(app, group_id)
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
                &format!("连接节点配置已刷新：{}", nodes.len()),
            );
        }
        Err(error) => {
            store.settings.mesh.last_node_refresh_at = Some(now_string());
            store.settings.mesh.last_node_refresh_error = Some(error.clone());
            if store.settings.mesh.cached_nodes.is_empty() {
                store.settings.mesh.cached_nodes = read_cached_nodes(&app).unwrap_or_default();
            }
            push_event(&mut store, "warn", "连接节点配置刷新失败");
        }
    }
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn create_share_payload(app: AppHandle, mode: MeshShareMode) -> Result<String, String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    ensure_local_device_credential(&mut store, &key)?;
    let sync_scope = if store.settings.mesh.sync_scope_initialized {
        store.settings.mesh.sync_scope.clone()
    } else {
        MeshSyncScope::default()
    };
    // The routing API is the private transport used for all mesh sync types.
    // It must travel with a normal share code even when the sender does not
    // share routing configuration itself. The scope still controls which
    // data the receiver may pull; the API key is never shown in the UI.
    let include_routing = !matches!(mode, MeshShareMode::MigrationBundle);
    if include_routing {
        let runtime = runtime_snapshot();
        if !runtime.running {
            start(app.clone())?;
        }
        // The no-TUN runtime uses a stable internal address, so the routing
        // endpoint can be included in the first share code as well.
        if runtime_snapshot().virtual_ipv4.is_some() {
            crate::routing::start_for_mesh_share(app.clone())?;
        }
    }
    store.settings.mesh.node_source_url = default_node_source_url();
    ensure_network_secret(&mut store, &key)?;
    let local_device_credential = ensure_local_device_credential(&mut store, &key)?;
    let network_secret = decrypt_network_secret(&store, &key)?;
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
        sync_scope,
        routing_base_url: routing.0,
        routing_api_key: routing.1,
        group_id: Some(MESH_GROUP_LEGACY_ID.to_string()),
        group_name: Some(store.settings.mesh.network_name.clone()),
        device_credential: Some(local_device_credential.clone()),
        credential_fingerprint: Some(credential_fingerprint(&local_device_credential)),
    };
    save_store(&app, &store)?;
    encode_payload(&payload)
}

pub(crate) fn create_group_share_payload(
    app: AppHandle,
    group_id: String,
    mode: MeshShareMode,
) -> Result<String, String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        return create_share_payload(app, mode);
    }
    if !group_runtime_snapshot(&group_id).running {
        start_group(app.clone(), group_id.clone())?;
    }
    crate::routing::start_for_mesh_share(app.clone())?;
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let routing = routing_share_for_group(&store, &group_id, true, &key);
    let group = store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())?;
    let network_secret = decrypt_group_network_secret(group, &key)?;
    let credential = ensure_group_local_device_credential(group, &key)?;
    let payload = MeshSharePayload {
        format: MESH_SHARE_FORMAT.to_string(),
        version: 1,
        mode,
        created_at: now_string(),
        device_id: local_device_id(),
        device_name: local_device_name(),
        network_name: group.name.clone(),
        network_secret,
        node_source_url: default_node_source_url(),
        peers: group
            .nodes
            .iter()
            .filter(|node| node.status == "up")
            .filter_map(|node| normalize_peer_url(&node.address))
            .take(12)
            .collect(),
        sync_scope: group.sync_scope.clone(),
        routing_base_url: routing.0,
        routing_api_key: routing.1,
        group_id: Some(group_id),
        group_name: Some(group.name.clone()),
        device_credential: Some(credential.clone()),
        credential_fingerprint: Some(credential_fingerprint(&credential)),
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
    let group_id = payload
        .group_id
        .clone()
        .unwrap_or_else(|| MESH_GROUP_LEGACY_ID.to_string());
    if group_id != MESH_GROUP_LEGACY_ID {
        return import_group_payload(app, payload, group_id);
    }
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    store.settings.mesh.network_name = payload.network_name.clone();
    store.settings.mesh.encrypted_network_secret =
        Some(encrypt_secret(payload.network_secret.as_bytes(), &key)?);
    store.settings.mesh.node_source_url = default_node_source_url();
    // The imported device controls what it shares. Keep this device's own
    // sharing preferences unchanged so the connection becomes two-way
    // without silently enabling rules, routing or conversations locally.
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
            allowed_sync_scope: Some(payload.sync_scope.clone()),
            auto_account_sync: !matches!(payload.mode, MeshShareMode::MigrationBundle)
                && payload.sync_scope.accounts,
            encrypted_routing_api_key: payload
                .routing_api_key
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| encrypt_secret(value.as_bytes(), &key))
                .transpose()?,
            encrypted_mesh_credential: payload
                .device_credential
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| encrypt_secret(value.as_bytes(), &key))
                .transpose()?,
            credential_fingerprint: payload.credential_fingerprint.clone().or_else(|| {
                payload
                    .device_credential
                    .as_deref()
                    .map(credential_fingerprint)
            }),
            revoked_at: None,
        },
    );
    push_event(&mut store, "info", "设备分享码已导入");
    store.settings.mesh.enabled = true;
    save_store(&app, &store)?;
    start(app.clone())
        .map_err(|error| format!("分享码已导入，但自动启动多设备共享失败：{error}"))?;
    Ok(MeshImportResult {
        group_id: MESH_GROUP_LEGACY_ID.to_string(),
        mode: payload.mode,
        device_id: payload.device_id,
        device_name: payload.device_name,
        imported_nodes: payload.peers.len(),
        message: "设备分享码已导入".to_string(),
    })
}

fn import_group_payload(
    app: AppHandle,
    payload: MeshSharePayload,
    group_id: String,
) -> Result<MeshImportResult, String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    if store.settings.mesh.groups.iter().any(|group| {
        group.group_id != group_id && group.name.eq_ignore_ascii_case(&payload.network_name)
    }) {
        return Err("imported network name conflicts with another mesh group".to_string());
    }
    let index = store
        .settings
        .mesh
        .groups
        .iter()
        .position(|group| group.group_id == group_id);
    if index.is_none() {
        let virtual_cidr = allocate_group_cidr(&store.settings.mesh.groups);
        let listen_port = allocate_group_listen_port(&store.settings.mesh.groups);
        let socks5_port = allocate_group_socks5_port(&store.settings.mesh.groups);
        store.settings.mesh.groups.push(MeshShareGroup {
            group_id: group_id.clone(),
            name: payload.network_name.clone(),
            encrypted_network_secret: None,
            nodes: Vec::new(),
            authorized_devices: Vec::new(),
            enabled: true,
            auto_start: true,
            sync_scope: MeshSyncScope::default(),
            virtual_cidr,
            listen_port,
            socks5_port,
            runtime: MeshGroupRuntimeState::default(),
            credential_grants: Vec::new(),
            encrypted_local_device_credential: None,
            legacy_compat: false,
        });
    }
    let group = store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())?;
    group.name = payload.network_name.clone();
    group.encrypted_network_secret = Some(encrypt_secret(payload.network_secret.as_bytes(), &key)?);
    group.enabled = true;
    merge_shared_peers_into_group(group, &payload);
    let fingerprint = payload.credential_fingerprint.clone().or_else(|| {
        payload
            .device_credential
            .as_deref()
            .map(credential_fingerprint)
    });
    if let Some(fingerprint) = fingerprint.clone() {
        if !group
            .credential_grants
            .iter()
            .any(|grant| grant.fingerprint == fingerprint)
        {
            group.credential_grants.push(MeshCredentialGrant {
                fingerprint,
                created_at: now_string(),
                bound_device_id: Some(payload.device_id.clone()),
                revoked_at: None,
            });
        }
    }
    let device = MeshDevice {
        id: payload.device_id.clone(),
        name: payload.device_name.clone(),
        address: payload.routing_base_url.clone(),
        last_seen_at: Some(now_string()),
        trusted: true,
        sync_scope: payload.sync_scope.clone(),
        allowed_sync_scope: Some(payload.sync_scope.clone()),
        auto_account_sync: !matches!(payload.mode, MeshShareMode::MigrationBundle)
            && payload.sync_scope.accounts,
        encrypted_routing_api_key: payload
            .routing_api_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| encrypt_secret(value.as_bytes(), &key))
            .transpose()?,
        encrypted_mesh_credential: payload
            .device_credential
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| encrypt_secret(value.as_bytes(), &key))
            .transpose()?,
        credential_fingerprint: fingerprint,
        revoked_at: None,
    };
    if let Some(existing) = group
        .authorized_devices
        .iter_mut()
        .find(|existing| existing.id == device.id)
    {
        *existing = device;
    } else {
        group.authorized_devices.push(device);
    }
    let imported_nodes = payload.peers.len();
    save_store(&app, &store)?;
    start_group(app.clone(), group_id.clone())
        .map_err(|error| format!("mesh group imported but failed to start: {error}"))?;
    Ok(MeshImportResult {
        group_id,
        mode: payload.mode,
        device_id: payload.device_id,
        device_name: payload.device_name,
        imported_nodes,
        message: "mesh group share code imported".to_string(),
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
        .ok_or_else(|| "未找到已连接设备".to_string())?;
    let allowed_scope = device_allowed_scope(device);
    device.trusted = trusted;
    device.revoked_at = if trusted { None } else { Some(now_string()) };
    device.auto_account_sync = auto_account_sync && allowed_scope.accounts;
    device.sync_scope = intersect_mesh_scopes(&sync_scope, &allowed_scope);
    device.allowed_sync_scope = Some(allowed_scope);
    push_event(&mut store, "info", "设备同步设置已保存");
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
        .filter(|device| device.trusted && device.revoked_at.is_none())
        .filter(|device| device.id != local_device_id())
        .filter(|device| {
            device.sync_scope.accounts
                || device.sync_scope.rules
                || device.sync_scope.routing
                || device.sync_scope.conversations
        })
        .filter(|device| device_id.as_deref().is_none_or(|id| id == device.id))
        .cloned()
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Err("没有可同步的受信任设备".to_string());
    }
    let _sync_guard = begin_mesh_data_sync()?;
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for device in targets {
        let allowed_scope = device_allowed_scope(&device);
        let selected_scope = intersect_mesh_scopes(&device.sync_scope, &allowed_scope);
        let scope = MeshSyncScope {
            accounts: selected_scope.accounts,
            rules: selected_scope.rules,
            routing: selected_scope.routing,
            conversations: selected_scope.conversations,
        };
        match sync_from_device_with_scope(&app, &key, MESH_GROUP_LEGACY_ID, &device, scope, false) {
            Ok(()) => succeeded += 1,
            Err(error) => {
                failed += 1;
                push_event(
                    &mut store,
                    "warn",
                    &format!("设备同步失败（{}）：{error}", device.name),
                );
            }
        }
    }
    // Each sync imports and saves the store itself. Reload it before writing
    // the aggregate event so the stale pre-sync snapshot cannot overwrite
    // imported profiles.
    store = load_store(&app)?;
    push_event(
        &mut store,
        if failed == 0 { "info" } else { "warn" },
        &format!("设备同步完成：成功 {succeeded} 台，失败 {failed} 台"),
    );
    save_store(&app, &store)?;
    status(app)
}

pub(crate) fn sync_group_now(
    app: AppHandle,
    group_id: String,
    device_id: Option<String>,
) -> Result<MeshGroupStatus, String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        sync_now(app.clone(), device_id)?;
        return group_status(app, group_id);
    }
    let store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let targets = find_group(&store, &group_id)?
        .authorized_devices
        .iter()
        .filter(|device| device.trusted && device.revoked_at.is_none())
        .filter(|device| device.id != local_device_id())
        .filter(|device| device_id.as_deref().is_none_or(|id| id == device.id))
        .cloned()
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Err("no trusted sync target in mesh group".to_string());
    }
    let _sync_guard = begin_mesh_data_sync()?;
    let mut failures = Vec::new();
    for device in targets {
        let scope = intersect_mesh_scopes(&device.sync_scope, &device_allowed_scope(&device));
        if let Err(error) =
            sync_from_device_with_scope(&app, &key, &group_id, &device, scope, false)
        {
            failures.push(format!("{}: {error}", device.name));
        }
    }
    if !failures.is_empty() {
        return Err(format!("mesh group sync failed: {}", failures.join("; ")));
    }
    group_status(app, group_id)
}

pub(crate) fn authorize_peer_and_sync(app: AppHandle, ip: String) -> Result<MeshStatus, String> {
    let ip = ip.trim().to_string();
    if ip.parse::<IpAddr>().is_err() {
        return Err("无效的设备虚拟 IP".to_string());
    }

    let store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let network_secret = decrypt_network_secret(&store, &key)?;
    let token = mesh_hello_token(&network_secret);
    let local_device_credential = store
        .settings
        .mesh
        .encrypted_local_device_credential
        .as_ref()
        .and_then(|envelope| decrypt_secret(envelope, &key).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let local_id = local_device_id();
    let local_name = local_device_name();
    let local_scope = local_mesh_scope(&store);
    let local_scope_json = serde_json::to_string(&local_scope).ok();
    let (local_routing_url, local_routing_key) = routing_share(&store, true, &key);
    let configured_port = store.settings.routing.port;
    let hello = request_mesh_hello(
        &ip,
        configured_port,
        MESH_GROUP_LEGACY_ID,
        &token,
        local_device_credential.as_deref(),
        &local_id,
        &local_name,
        Some(&local_mesh_ipv4()),
        local_routing_url.as_deref(),
        local_routing_key.as_deref(),
        local_scope_json.as_deref(),
    )
    .or_else(|| {
        (configured_port != 15_722)
            .then(|| {
                request_mesh_hello(
                    &ip,
                    15_722,
                    MESH_GROUP_LEGACY_ID,
                    &token,
                    local_device_credential.as_deref(),
                    &local_id,
                    &local_name,
                    Some(&local_mesh_ipv4()),
                    local_routing_url.as_deref(),
                    local_routing_key.as_deref(),
                    local_scope_json.as_deref(),
                )
            })
            .flatten()
    })
    .ok_or_else(|| "设备已连接，但无法完成授权握手".to_string())?;

    if hello.device_id == local_id {
        return Err("不能将本机添加为同步设备".to_string());
    }

    let address = hello.routing_base_url.clone().or_else(|| {
        hello
            .virtual_ipv4
            .as_ref()
            .map(|value| format!("http://{}:{}/v1", value, store.settings.routing.port))
    });
    let encrypted_routing_key = hello
        .routing_api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| encrypt_secret(value.as_bytes(), &key))
        .transpose()?;
    let mut store = load_store(&app)?;
    upsert_device(
        &mut store,
        MeshDevice {
            id: hello.device_id.clone(),
            name: hello.device_name.clone(),
            address,
            last_seen_at: Some(now_string()),
            trusted: true,
            sync_scope: hello.sync_scope.clone(),
            allowed_sync_scope: Some(hello.sync_scope.clone()),
            auto_account_sync: hello.sync_scope.accounts,
            encrypted_routing_api_key: encrypted_routing_key,
            encrypted_mesh_credential: hello
                .device_credential
                .as_deref()
                .map(|value| encrypt_secret(value.as_bytes(), &key))
                .transpose()?,
            credential_fingerprint: hello
                .device_credential
                .as_deref()
                .map(credential_fingerprint),
            revoked_at: None,
        },
    );
    push_event(
        &mut store,
        "info",
        &format!("已授权设备并开始同步：{}", hello.device_name),
    );
    save_store(&app, &store)?;

    sync_now(app, Some(hello.device_id))
}

fn run_auto_account_sync(app: &AppHandle) {
    if MESH_AUTO_ACCOUNT_SYNC_RUNNING.swap(true, AtomicOrdering::AcqRel) {
        return;
    }
    let result = catch_unwind(AssertUnwindSafe(|| run_auto_account_sync_inner(app)));
    MESH_AUTO_ACCOUNT_SYNC_RUNNING.store(false, AtomicOrdering::Release);
    if let Err(panic) = result {
        append_app_log(
            app,
            "error",
            &format!(
                "automatic mesh account sync panicked: {}",
                panic_payload_message(panic.as_ref())
            ),
        );
    }
}

fn run_auto_account_sync_inner(app: &AppHandle) {
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
        .filter(|device| {
            device.trusted
                && device.revoked_at.is_none()
                && device.auto_account_sync
                && device.sync_scope.accounts
        })
        .filter(|device| device.id != local_device_id())
        .cloned()
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return;
    }

    let Ok(_sync_guard) = begin_mesh_data_sync() else {
        append_app_log(
            app,
            "debug",
            "automatic mesh account sync skipped because another sync is active",
        );
        return;
    };

    let scope = MeshSyncScope {
        accounts: true,
        rules: false,
        routing: false,
        conversations: false,
    };
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for device in targets {
        match sync_from_device_with_scope(
            app,
            &key,
            MESH_GROUP_LEGACY_ID,
            &device,
            scope.clone(),
            true,
        ) {
            Ok(()) => succeeded += 1,
            Err(error) => {
                failed += 1;
                push_event(
                    &mut store,
                    "warn",
                    &format!("账号自动同步失败（{}）：{error}", device.name),
                );
            }
        }
    }
    if succeeded > 0 || failed > 0 {
        // The import path writes the latest store; do not save the snapshot
        // loaded before the automatic sync started.
        let Ok(latest_store) = load_store(app) else {
            append_app_log(
                app,
                "error",
                "automatic mesh account sync could not reload store",
            );
            return;
        };
        store = latest_store;
        push_event(
            &mut store,
            if failed == 0 { "info" } else { "warn" },
            &format!("账号自动同步完成：成功 {succeeded} 台，失败 {failed} 台"),
        );
        let _ = save_store(app, &store);
    }
    if failed > 0 {
        append_app_log(
            app,
            "warn",
            &format!("automatic mesh account sync failed for {failed} device(s)"),
        );
    }
}

fn run_group_auto_account_sync(app: &AppHandle, group_id: &str) {
    let Ok(key) = load_master_key(app) else {
        return;
    };
    let Ok(store) = load_store(app) else {
        return;
    };
    let Ok(group) = find_group(&store, group_id) else {
        return;
    };
    let targets = group
        .authorized_devices
        .iter()
        .filter(|device| {
            device.trusted
                && device.revoked_at.is_none()
                && device.auto_account_sync
                && device.sync_scope.accounts
                && device.id != local_device_id()
        })
        .cloned()
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return;
    }
    let Ok(_sync_guard) = begin_mesh_data_sync() else {
        return;
    };
    for device in targets {
        let scope = MeshSyncScope {
            accounts: true,
            rules: false,
            routing: false,
            conversations: false,
        };
        if let Err(error) = sync_from_device_with_scope(app, &key, group_id, &device, scope, true) {
            append_app_log(
                app,
                "warn",
                &format!("mesh group {group_id} auto sync failed: {error}"),
            );
        }
    }
}

fn sync_from_device_with_scope(
    app: &AppHandle,
    key: &[u8; 32],
    group_id: &str,
    device: &MeshDevice,
    scope: MeshSyncScope,
    only_valid_accounts: bool,
) -> Result<(), String> {
    let base_url = device
        .address
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "该设备没有可用连接地址，请重新导入分享码".to_string())?;
    let encrypted_key = device
        .encrypted_routing_api_key
        .as_ref()
        .ok_or_else(|| "该设备没有同步权限，请重新导入分享码".to_string())?;
    let routing_api_key =
        String::from_utf8(decrypt_secret(encrypted_key, key)?).map_err(display_err)?;
    let password = migration_password_from_group(app, group_id)?;
    let scope_header = serde_json::to_string(&scope).map_err(display_err)?;
    let local_device_id_value = local_device_id();
    let local_device_credential = {
        let mut local_store = load_store(app)?;
        let credential = ensure_local_group_credential(&mut local_store, group_id, key)?;
        save_store(app, &local_store)?;
        credential
    };
    let mut client_builder = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180));
    if should_use_mesh_socks5(base_url) {
        client_builder = client_builder.proxy(
            reqwest::Proxy::all(mesh_socks5_proxy_url_for_group(group_id)).map_err(display_err)?,
        );
    }
    let client = client_builder.build().map_err(display_err)?;
    let response = client
        .post(pull_endpoint(base_url)?)
        .bearer_auth(routing_api_key)
        .header(MESH_GROUP_ID_HEADER, group_id)
        .header(MESH_HELLO_DEVICE_ID_HEADER, local_device_id_value)
        .header(MESH_DEVICE_CREDENTIAL_HEADER, local_device_credential)
        .header("X-Codex-Mesh-Scope", scope_header)
        .header(
            "X-Codex-Mesh-Valid-Accounts",
            if only_valid_accounts { "true" } else { "false" },
        )
        .send()
        .map_err(display_err)?;
    if !response.status().is_success() {
        return Err(format!(
            "target returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let bytes = response.bytes().map_err(display_err)?;
    if bytes.len() > MAX_MESH_SYNC_BYTES {
        return Err("target returned an oversized sync bundle".to_string());
    }
    let temp_path = std::env::temp_dir().join(format!(
        "codex-switcher-mesh-incoming-{}.zip.enc",
        uuid::Uuid::new_v4().simple()
    ));
    fs::write(&temp_path, bytes)
        .map_err(|error| format!("无法写入同步临时文件 {}：{}", temp_path.display(), error))?;
    let result = crate::import_accounts_bundle_with_scope(
        (*app).clone(),
        temp_path.to_string_lossy().to_string(),
        password,
        scope.conversations,
        None,
        Some(scope),
    )
    .map(|_| ());
    let _ = fs::remove_file(temp_path);
    result
}

fn pull_endpoint(base_url: &str) -> Result<String, String> {
    let mut url = url::Url::parse(base_url).map_err(display_err)?;
    let path = url.path().trim_end_matches('/');
    let base_path = path.strip_suffix("/v1").unwrap_or(path);
    url.set_path(&format!("{base_path}/mesh/pull"));
    Ok(url.to_string())
}

pub(crate) fn handle_sync_request(app: AppHandle, mut request: Request) -> Result<(), String> {
    let group_id = request_group_id(&request);
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

    if let Err((status, error)) = authorize_device_request(&request, &store, &group_id, &key) {
        respond_sync_json(request, status, serde_json::json!({ "error": error }))?;
        return Ok(());
    }

    let requested_scope = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("X-Codex-Mesh-Scope"))
        .and_then(|header| serde_json::from_str::<MeshSyncScope>(header.value.as_str()).ok())
        .unwrap_or_default();
    let scope = intersect_mesh_scopes(
        &requested_scope,
        &local_mesh_scope_for_group(&store, &group_id)?,
    );
    let _sync_guard = begin_mesh_data_sync()?;
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
    fs::write(&temp_path, bytes)
        .map_err(|error| format!("无法写入同步临时文件 {}：{}", temp_path.display(), error))?;
    let password = migration_password_from_group(&app, &group_id)?;
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
                "restoredFiles": manifest.restored_files,
                "skippedExpiredProfiles": manifest.skipped_expired_profiles
            }),
        ),
        Err(error) => respond_sync_json(request, 422, serde_json::json!({ "error": error })),
    }
}

pub(crate) fn handle_pull_request(app: AppHandle, request: Request) -> Result<(), String> {
    let group_id = request_group_id(&request);
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

    if let Err((status, error)) = authorize_device_request(&request, &store, &group_id, &key) {
        respond_sync_json(request, status, serde_json::json!({ "error": error }))?;
        return Ok(());
    }

    let requested_scope = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("X-Codex-Mesh-Scope"))
        .and_then(|header| serde_json::from_str::<MeshSyncScope>(header.value.as_str()).ok())
        .unwrap_or_default();
    let scope = intersect_mesh_scopes(
        &requested_scope,
        &local_mesh_scope_for_group(&store, &group_id)?,
    );
    let only_valid_accounts = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("X-Codex-Mesh-Valid-Accounts"))
        .is_some_and(|header| header.value.as_str().eq_ignore_ascii_case("true"));
    let _sync_guard = begin_mesh_data_sync()?;
    let temp_path = std::env::temp_dir().join(format!(
        "codex-switcher-mesh-outgoing-{}.zip.enc",
        uuid::Uuid::new_v4().simple()
    ));
    let password = migration_password_from_group(&app, &group_id)?;
    let result = crate::export_mesh_sync_bundle_internal(
        app,
        temp_path.to_string_lossy().to_string(),
        password,
        scope.conversations,
        scope.accounts,
        only_valid_accounts,
    )
    .and_then(|_| fs::read(&temp_path).map_err(display_err));
    let _ = fs::remove_file(&temp_path);
    match result {
        Ok(bytes) => {
            if bytes.len() > MAX_MESH_SYNC_BYTES {
                respond_sync_json(
                    request,
                    413,
                    serde_json::json!({ "error": "bundle too large" }),
                )?;
            } else {
                let content_type = Header::from_bytes("Content-Type", "application/octet-stream")
                    .map_err(|_| "failed to build response header".to_string())?;
                request
                    .respond(
                        Response::from_data(bytes)
                            .with_status_code(StatusCode(200))
                            .with_header(content_type),
                    )
                    .map_err(display_err)?;
            }
        }
        Err(error) => {
            respond_sync_json(request, 422, serde_json::json!({ "error": error }))?;
        }
    }
    Ok(())
}

pub(crate) fn handle_hello_request(app: AppHandle, request: Request) -> Result<(), String> {
    let mut store = load_store(&app)?;
    let key = load_master_key(&app)?;
    let group_id = request_group_id(&request);
    let network_secret = group_network_secret(&store, &group_id, &key)?;
    let expected = mesh_hello_token(&network_secret);
    let provided = request
        .headers()
        .iter()
        .find(|header| header.field.equiv(MESH_HELLO_TOKEN_HEADER))
        .map(|header| header.value.as_str())
        .unwrap_or_default();
    if provided != expected {
        respond_sync_json(
            request,
            401,
            serde_json::json!({ "error": "invalid mesh token" }),
        )?;
        return Ok(());
    }

    // The imported side already knows the share-code creator, but the creator
    // does not know the new device's id. Accept the caller's signed-in mesh
    // hello metadata so both sides can populate their device lists even when
    // EasyTier's route-info API is unavailable on Windows.
    let incoming_id = request
        .headers()
        .iter()
        .find(|header| header.field.equiv(MESH_HELLO_DEVICE_ID_HEADER))
        .map(|header| header.value.as_str().trim().to_string())
        .filter(|value| !value.is_empty() && value != &local_device_id());
    let incoming_credential = request_header(&request, MESH_DEVICE_CREDENTIAL_HEADER);
    if let Some(incoming_id) = incoming_id {
        let existing = group_devices(&store, &group_id)?
            .iter()
            .find(|device| device.id == incoming_id)
            .cloned();
        if existing
            .as_ref()
            .is_some_and(|device| !device.trusted || device.revoked_at.is_some())
        {
            respond_sync_json(
                request,
                403,
                serde_json::json!({ "error": "device access has been revoked" }),
            )?;
            return Ok(());
        }
        let incoming_name = request
            .headers()
            .iter()
            .find(|header| header.field.equiv(MESH_HELLO_DEVICE_NAME_HEADER))
            .map(|header| header.value.as_str().trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| existing.as_ref().map(|device| device.name.clone()))
            .unwrap_or_else(|| incoming_id.clone());
        let incoming_ip = request
            .headers()
            .iter()
            .find(|header| header.field.equiv(MESH_HELLO_VIRTUAL_IPV4_HEADER))
            .map(|header| header.value.as_str().trim().to_string())
            .filter(|value| value.parse::<IpAddr>().is_ok());
        let incoming_scope = request
            .headers()
            .iter()
            .find(|header| header.field.equiv(MESH_HELLO_SYNC_SCOPE_HEADER))
            .and_then(|header| serde_json::from_str::<MeshSyncScope>(header.value.as_str()).ok())
            .or_else(|| existing.as_ref().map(|device| device.sync_scope.clone()))
            .unwrap_or_default();
        let selected_scope = existing
            .as_ref()
            .map(|device| intersect_mesh_scopes(&device.sync_scope, &incoming_scope))
            .unwrap_or_else(|| incoming_scope.clone());
        let incoming_routing_url = request
            .headers()
            .iter()
            .find(|header| header.field.equiv(MESH_HELLO_ROUTING_URL_HEADER))
            .map(|header| header.value.as_str().trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                incoming_ip
                    .as_ref()
                    .map(|ip| format!("http://{}:{}/v1", ip, store.settings.routing.port))
            })
            .or_else(|| existing.as_ref().and_then(|device| device.address.clone()));
        let incoming_routing_key = request
            .headers()
            .iter()
            .find(|header| header.field.equiv(MESH_HELLO_ROUTING_KEY_HEADER))
            .map(|header| header.value.as_str().trim().to_string())
            .filter(|value| !value.is_empty());
        let encrypted_routing_key = incoming_routing_key
            .as_deref()
            .map(|value| encrypt_secret(value.as_bytes(), &key))
            .transpose()?;
        let incoming_device = MeshDevice {
            id: incoming_id.clone(),
            name: incoming_name,
            address: incoming_routing_url,
            last_seen_at: Some(now_string()),
            trusted: existing
                .as_ref()
                .map(|device| device.trusted)
                .unwrap_or(true),
            sync_scope: selected_scope,
            allowed_sync_scope: Some(incoming_scope.clone()),
            auto_account_sync: existing
                .as_ref()
                .map(|device| device.auto_account_sync)
                .unwrap_or(incoming_scope.accounts),
            encrypted_routing_api_key: encrypted_routing_key.or_else(|| {
                existing
                    .as_ref()
                    .and_then(|device| device.encrypted_routing_api_key.clone())
            }),
            encrypted_mesh_credential: incoming_credential
                .as_deref()
                .map(|value| encrypt_secret(value.as_bytes(), &key))
                .transpose()?
                .or_else(|| {
                    existing
                        .as_ref()
                        .and_then(|device| device.encrypted_mesh_credential.clone())
                }),
            credential_fingerprint: incoming_credential
                .as_deref()
                .map(credential_fingerprint)
                .or_else(|| {
                    existing
                        .as_ref()
                        .and_then(|device| device.credential_fingerprint.clone())
                }),
            revoked_at: existing
                .as_ref()
                .and_then(|device| device.revoked_at.clone()),
        };
        let structural_changed = existing.as_ref().is_none_or(|device| {
            device.name != incoming_device.name
                || device.address != incoming_device.address
                || device_allowed_scope(device) != incoming_scope
                || device.encrypted_routing_api_key.is_some()
                    != incoming_device.encrypted_routing_api_key.is_some()
                || device.encrypted_mesh_credential.is_some()
                    != incoming_device.encrypted_mesh_credential.is_some()
        });
        let heartbeat_due = existing
            .as_ref()
            .map(|device| !mesh_seen_recently(device.last_seen_at.as_deref()))
            .unwrap_or(true);
        let changed = structural_changed || heartbeat_due;
        if changed {
            upsert_group_device(&mut store, &group_id, incoming_device.clone())?;
            if structural_changed {
                push_event(
                    &mut store,
                    "info",
                    &format!("发现多设备共享连接：{}", incoming_device.name),
                );
            }
            save_store(&app, &store)?;
        }
    }

    let runtime = group_runtime_snapshot(&group_id);
    let routing_base_url = advertised_routing_host(
        &store.settings.routing.listen_host,
        runtime.virtual_ipv4.as_deref(),
    )
    .map(|host| format!("http://{}:{}/v1", host, store.settings.routing.port));
    let routing_api_key = store
        .settings
        .routing
        .encrypted_access_key
        .as_ref()
        .and_then(|envelope| decrypt_secret(envelope, &key).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let sync_scope = local_mesh_scope_for_group(&store, &group_id)?;
    let local_device_credential = ensure_local_group_credential(&mut store, &group_id, &key)?;
    respond_sync_json(
        request,
        200,
        serde_json::to_value(MeshHello {
            format: MESH_SHARE_FORMAT.to_string(),
            version: 1,
            device_id: local_device_id(),
            device_name: local_device_name(),
            virtual_ipv4: runtime.virtual_ipv4,
            routing_base_url,
            routing_api_key,
            sync_scope,
            group_id: Some(group_id),
            device_credential: Some(local_device_credential),
        })
        .map_err(display_err)?,
    )
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

fn migration_password_from_group(app: &AppHandle, group_id: &str) -> Result<String, String> {
    let store = load_store(app)?;
    let key = load_master_key(app)?;
    let secret = group_network_secret(&store, group_id, &key)?;
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":codex-switcher-migration-share:v1");
    Ok(URL_SAFE_NO_PAD.encode(hasher.finalize()))
}

fn mesh_hello_token(network_secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(network_secret.as_bytes());
    hasher.update(MESH_HELLO_TOKEN_SUFFIX);
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn local_mesh_scope(store: &AppStore) -> MeshSyncScope {
    if store.settings.mesh.sync_scope_initialized {
        store.settings.mesh.sync_scope.clone()
    } else {
        MeshSyncScope::default()
    }
}

fn local_mesh_scope_for_group(store: &AppStore, group_id: &str) -> Result<MeshSyncScope, String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        Ok(local_mesh_scope(store))
    } else {
        Ok(find_group(store, group_id)?.sync_scope.clone())
    }
}

fn intersect_mesh_scopes(requested: &MeshSyncScope, allowed: &MeshSyncScope) -> MeshSyncScope {
    MeshSyncScope {
        accounts: requested.accounts && allowed.accounts,
        rules: requested.rules && allowed.rules,
        routing: requested.routing && allowed.routing,
        conversations: requested.conversations && allowed.conversations,
    }
}

fn device_allowed_scope(device: &MeshDevice) -> MeshSyncScope {
    device
        .allowed_sync_scope
        .clone()
        .unwrap_or_else(|| device.sync_scope.clone())
}

fn build_status(_app: &AppHandle, store: &AppStore) -> MeshStatus {
    let runtime = runtime_snapshot();
    let local_id = local_device_id();
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
            sync_scope: if store.settings.mesh.sync_scope_initialized {
                store.settings.mesh.sync_scope.clone()
            } else {
                MeshSyncScope::default()
            },
            last_node_refresh_at: store.settings.mesh.last_node_refresh_at.clone(),
            last_node_refresh_error: store.settings.mesh.last_node_refresh_error.clone(),
        },
        public_nodes: store.settings.mesh.cached_nodes.clone(),
        devices: store
            .settings
            .mesh
            .authorized_devices
            .iter()
            .map(|device| {
                let mut view = MeshDeviceView::from(device);
                let device_ip = device
                    .address
                    .as_deref()
                    .and_then(|address| url::Url::parse(address).ok())
                    .and_then(|address| address.host_str().map(str::to_string))
                    .or_else(|| Some(mesh_ipv4_for_device_id(&device.id)));
                if device.id == local_id {
                    view.online = true;
                    view.ip = runtime.virtual_ipv4.clone().or(device_ip);
                } else if let Some(peer) = runtime
                    .peers
                    .iter()
                    .find(|peer| peer.ip.is_some() && peer.ip == device_ip)
                {
                    view.online = true;
                    view.ip = peer.ip.clone();
                    view.latency_ms = peer.latency_ms;
                } else if runtime.running && mesh_seen_recently(device.last_seen_at.as_deref()) {
                    // The EasyTier SDK can fail to enumerate Windows network
                    // interfaces even while its no-TUN transport is alive.
                    // A recent authenticated hello is therefore also a valid
                    // online signal.
                    view.online = true;
                    view.ip = device_ip;
                } else {
                    view.ip = device_ip;
                }
                view
            })
            .collect(),
        peers: runtime.peers.clone(),
        share_ready: store.settings.mesh.encrypted_network_secret.is_some(),
        local_device_id: local_device_id(),
        local_device_name: local_device_name(),
        routing_base_url: routing_host
            .map(|host| format!("http://{}:{}/v1", host, store.settings.routing.port)),
        runtime_kind: runtime.runtime_kind,
        process_id: runtime.process_id,
        runtime_binary_path: runtime.executable_path,
        peer_count: runtime.peer_count,
        virtual_ipv4: runtime.virtual_ipv4,
        started_at: runtime.started_at,
        last_error: runtime.last_error,
        groups: mesh_group_views(store),
    }
}

fn mesh_group_views(store: &AppStore) -> Vec<MeshGroupView> {
    store
        .settings
        .mesh
        .groups
        .iter()
        .map(|group| {
            let runtime = group_runtime_snapshot(&group.group_id);
            let online_device_count = group
                .authorized_devices
                .iter()
                .filter(|device| device.id != local_device_id())
                .filter(|device| device.revoked_at.is_none())
                .filter(|device| group_device_view(device, &runtime, &group.group_id).online)
                .count();
            MeshGroupView {
                group_id: group.group_id.clone(),
                name: group.name.clone(),
                enabled: group.enabled,
                auto_start: group.auto_start,
                sync_scope: group.sync_scope.clone(),
                node_count: group.nodes.len(),
                device_count: group.authorized_devices.len(),
                online_device_count,
                virtual_cidr: group.virtual_cidr.clone(),
                listen_port: group.listen_port,
                socks5_port: group.socks5_port,
                runtime: MeshGroupRuntimeState {
                    running: runtime.running,
                    instance_id: runtime
                        .running
                        .then(|| runtime.runtime_kind.clone())
                        .flatten(),
                    virtual_ipv4: runtime.virtual_ipv4,
                    started_at: runtime.started_at,
                    last_error: runtime.last_error,
                },
            }
        })
        .collect()
}

fn build_group_status(store: &AppStore, group_id: &str) -> Result<MeshGroupStatus, String> {
    let group = store
        .settings
        .mesh
        .groups
        .iter()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())?;
    let runtime = group_runtime_snapshot(group_id);
    let view = mesh_group_views(store)
        .into_iter()
        .find(|view| view.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())?;
    let devices = group
        .authorized_devices
        .iter()
        .map(|device| group_device_view(device, &runtime, group_id))
        .collect();
    let routing_base_url = advertised_routing_host(
        &store.settings.routing.listen_host,
        runtime.virtual_ipv4.as_deref(),
    )
    .map(|host| format!("http://{}:{}/v1", host, store.settings.routing.port));
    Ok(MeshGroupStatus {
        group: view,
        devices,
        peers: runtime.peers,
        local_device_id: local_device_id(),
        local_device_name: local_device_name(),
        routing_base_url,
    })
}

fn group_device_view(
    device: &MeshDevice,
    runtime: &MeshRuntimeSnapshot,
    group_id: &str,
) -> MeshDeviceView {
    let mut view = MeshDeviceView::from(device);
    let device_ip = device
        .address
        .as_deref()
        .and_then(|address| url::Url::parse(address).ok())
        .and_then(|address| address.host_str().map(str::to_string))
        .or_else(|| Some(mesh_ipv4_for_group_device(group_id, &device.id)));
    if device.id == local_device_id() {
        view.online = true;
        view.ip = runtime.virtual_ipv4.clone().or(device_ip);
    } else if let Some(peer) = runtime
        .peers
        .iter()
        .find(|peer| peer.ip.is_some() && peer.ip == device_ip)
    {
        view.online = true;
        view.ip = peer.ip.clone();
        view.latency_ms = peer.latency_ms;
    } else {
        view.online = runtime.running && mesh_seen_recently(device.last_seen_at.as_deref());
        view.ip = device_ip;
    }
    view
}

fn start_embedded_runtime(
    app: &AppHandle,
    store: &AppStore,
    network_secret: &str,
    peers: Vec<String>,
) -> Result<MeshRuntime, String> {
    stop_runtime();
    MESH_RUNTIME_INFO_UNAVAILABLE.store(false, AtomicOrdering::Release);

    let config = build_easytier_config(store, network_secret, &peers)?;
    let virtual_ipv4 = local_mesh_ipv4();
    let manager = network_manager();
    let instance_id = manager
        .run_network_instance(config, false, ConfigFileControl::STATIC_CONFIG)
        .map_err(|error| {
            let message = format!("设备连接服务启动失败：{error}");
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
    let peer_discovery_thread = Some(spawn_mesh_peer_discovery_loop(
        app.clone(),
        MESH_GROUP_LEGACY_ID.to_string(),
        manager.clone(),
        instance_id,
        refresh_stop.clone(),
    ));
    let routing_thread = Some(spawn_mesh_routing_loop(
        app.clone(),
        manager.clone(),
        instance_id,
        refresh_stop.clone(),
    ));

    Ok(MeshRuntime {
        group_id: MESH_GROUP_LEGACY_ID.to_string(),
        manager,
        instance_id,
        runtime_kind: "embeddedSdk".to_string(),
        started_at: now_string(),
        virtual_ipv4,
        virtual_cidr: MESH_GROUP_DEFAULT_CIDR.to_string(),
        socks5_port: MESH_GROUP_SOCKS5_PORT_BASE,
        peers,
        refresh_stop,
        refresh_thread,
        account_sync_thread,
        peer_discovery_thread,
        routing_thread,
    })
}

fn start_embedded_group_runtime(
    app: &AppHandle,
    group: &MeshShareGroup,
    network_secret: &str,
    peers: Vec<String>,
) -> Result<MeshRuntime, String> {
    MESH_RUNTIME_INFO_UNAVAILABLE.store(false, AtomicOrdering::Release);
    let config = build_easytier_group_config(group, network_secret, &peers)?;
    let virtual_ipv4 = mesh_ipv4_in_cidr(&group.virtual_cidr, &local_device_id())?;
    let manager = network_manager();
    let instance_id = manager
        .run_network_instance(config, false, ConfigFileControl::STATIC_CONFIG)
        .map_err(|error| {
            let message = format!("mesh group {} failed to start: {error}", group.group_id);
            set_group_runtime_error(&group.group_id, Some(message.clone()));
            message
        })?;
    let refresh_stop = Arc::new(AtomicBool::new(false));
    let peer_discovery_thread = Some(spawn_mesh_peer_discovery_loop(
        app.clone(),
        group.group_id.clone(),
        manager.clone(),
        instance_id,
        refresh_stop.clone(),
    ));
    let account_sync_thread = Some(spawn_group_account_sync_loop(
        app.clone(),
        group.group_id.clone(),
        refresh_stop.clone(),
        120,
    ));
    let routing_thread = Some(spawn_mesh_routing_loop(
        app.clone(),
        manager.clone(),
        instance_id,
        refresh_stop.clone(),
    ));
    Ok(MeshRuntime {
        group_id: group.group_id.clone(),
        manager,
        instance_id,
        runtime_kind: "embeddedSdk".to_string(),
        started_at: now_string(),
        virtual_ipv4,
        virtual_cidr: group.virtual_cidr.clone(),
        socks5_port: group.socks5_port,
        peers,
        refresh_stop,
        refresh_thread: None,
        account_sync_thread,
        peer_discovery_thread,
        routing_thread,
    })
}

fn runtime_state_from_runtime(runtime: &MeshRuntime) -> MeshGroupRuntimeState {
    MeshGroupRuntimeState {
        running: true,
        instance_id: Some(runtime.instance_id.to_string()),
        virtual_ipv4: Some(runtime.virtual_ipv4.clone()),
        started_at: Some(runtime.started_at.clone()),
        last_error: None,
    }
}

fn spawn_mesh_peer_discovery_loop(
    app: AppHandle,
    group_id: String,
    manager: Arc<NetworkInstanceManager>,
    instance_id: uuid::Uuid,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut last_error: Option<String> = None;
        while !stop.load(AtomicOrdering::Relaxed) {
            let result = catch_unwind(AssertUnwindSafe(|| {
                discover_mesh_peers(&app, &group_id, &manager, instance_id)
            }));
            match result {
                Ok(Ok(())) => last_error = None,
                Ok(Err(error)) => {
                    if last_error.as_deref() != Some(error.as_str()) {
                        append_app_log(&app, "debug", &format!("mesh peer discovery: {error}"));
                        last_error = Some(error);
                    }
                }
                Err(panic) => {
                    let error = format!(
                        "mesh peer discovery panicked: {}",
                        panic_payload_message(panic.as_ref())
                    );
                    if last_error.as_deref() != Some(error.as_str()) {
                        append_app_log(&app, "error", &error);
                        last_error = Some(error);
                    }
                }
            }
            for _ in 0..3 {
                if stop.load(AtomicOrdering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_secs(1));
            }
        }
    })
}

fn discover_mesh_peers(
    app: &AppHandle,
    group_id: &str,
    manager: &NetworkInstanceManager,
    instance_id: uuid::Uuid,
) -> Result<(), String> {
    let store = load_store(app)?;
    let virtual_cidr = if group_id == MESH_GROUP_LEGACY_ID {
        MESH_GROUP_DEFAULT_CIDR.to_string()
    } else {
        find_group(&store, group_id)?.virtual_cidr.clone()
    };
    let devices = group_devices(&store, group_id)?.to_vec();
    let info = collect_network_infos_safe(manager)
        .ok()
        .and_then(|mut infos| infos.remove(&instance_id));
    let local_ip = info
        .as_ref()
        .and_then(|value| value.my_node_info.as_ref())
        .and_then(|node| node.virtual_ipv4.as_ref())
        .and_then(|value| value.to_string().split('/').next().map(str::to_string))
        .or_else(|| mesh_ipv4_in_cidr(&virtual_cidr, &local_device_id()).ok());
    let mut peer_ips = info
        .as_ref()
        .map(|value| {
            value
                .routes
                .iter()
                .filter_map(|route| {
                    route
                        .ipv4_addr
                        .as_ref()
                        .and_then(|inet| inet.address.as_ref())
                        .map(ToString::to_string)
                        .map(|value| value.split('/').next().unwrap_or(&value).to_string())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    // The stable address fallback lets the imported side find the device that
    // generated the share code even when EasyTier cannot enumerate Windows
    // interfaces for its status API.
    for device in &devices {
        if device.id != local_device_id() {
            if let Some(address) = device
                .address
                .as_deref()
                .and_then(|address| url::Url::parse(address).ok())
                .and_then(|address| address.host_str().map(str::to_string))
            {
                peer_ips.push(address);
            }
            peer_ips.push(mesh_ipv4_for_group_device(group_id, &device.id));
        }
    }
    peer_ips.sort();
    peer_ips.dedup();
    let peer_ips = peer_ips
        .into_iter()
        .filter(|ip| local_ip.as_deref() != Some(ip.as_str()))
        .filter(|ip| ip.parse::<IpAddr>().is_ok())
        .collect::<Vec<_>>();
    let peer_candidate_count = peer_ips.len();
    if peer_ips.is_empty() {
        return Ok(());
    }

    let key = load_master_key(app)?;
    let network_secret = group_network_secret(&store, group_id, &key)?;
    let token = mesh_hello_token(&network_secret);
    let configured_port = store.settings.routing.port;
    let local_id = local_device_id();
    let local_name = local_device_name();
    let local_scope = local_mesh_scope_for_group(&store, group_id)?;
    let local_scope_json = serde_json::to_string(&local_scope).ok();
    let local_device_credential = if group_id == MESH_GROUP_LEGACY_ID {
        store
            .settings
            .mesh
            .encrypted_local_device_credential
            .as_ref()
    } else {
        find_group(&store, group_id)?
            .encrypted_local_device_credential
            .as_ref()
    }
    .and_then(|envelope| decrypt_secret(envelope, &key).ok())
    .and_then(|bytes| String::from_utf8(bytes).ok());
    let (local_routing_url, local_routing_key) = if group_id == MESH_GROUP_LEGACY_ID {
        routing_share(&store, true, &key)
    } else {
        routing_share_for_group(&store, group_id, true, &key)
    };
    let mut hellos = Vec::new();
    for ip in peer_ips {
        let request = |port| {
            request_mesh_hello(
                &ip,
                port,
                group_id,
                &token,
                local_device_credential.as_deref(),
                &local_id,
                &local_name,
                local_ip.as_deref(),
                local_routing_url.as_deref(),
                local_routing_key.as_deref(),
                local_scope_json.as_deref(),
            )
        };
        if let Some(hello) = request(configured_port).or_else(|| {
            (configured_port != 15_722)
                .then(|| request(15_722))
                .flatten()
        }) {
            if hello.device_id != local_device_id()
                && !hellos
                    .iter()
                    .any(|item: &MeshHello| item.device_id == hello.device_id)
            {
                hellos.push(hello);
            }
        }
    }
    if hellos.is_empty() {
        return Err(format!(
            "mesh peer hello failed: {} candidate(s), runtime peer info unavailable or unreachable",
            peer_candidate_count
        ));
    }

    let mut store = load_store(app)?;
    let key = load_master_key(app)?;
    let mut changed = false;
    let mut sync_needed = false;
    for hello in hellos {
        let existing = group_devices(&store, group_id)?
            .iter()
            .find(|device| device.id == hello.device_id)
            .cloned();
        let trusted = existing
            .as_ref()
            .map(|device| device.trusted)
            .unwrap_or(true);
        let auto_account_sync = existing
            .as_ref()
            .map(|device| device.auto_account_sync)
            .unwrap_or(hello.sync_scope.accounts);
        let encrypted_key = hello
            .routing_api_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| encrypt_secret(value.as_bytes(), &key))
            .transpose()?;
        let key_changed = match (&existing, &hello.routing_api_key) {
            (Some(device), Some(remote_key)) => {
                device
                    .encrypted_routing_api_key
                    .as_ref()
                    .and_then(|envelope| decrypt_secret(envelope, &key).ok())
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .as_deref()
                    != Some(remote_key.as_str())
            }
            (Some(device), None) => device.encrypted_routing_api_key.is_some(),
            (None, _) => true,
        };
        let address = hello.routing_base_url.clone();
        let selected_scope = existing
            .as_ref()
            .map(|device| intersect_mesh_scopes(&device.sync_scope, &hello.sync_scope))
            .unwrap_or_else(|| hello.sync_scope.clone());
        let structural_changed = existing.as_ref().is_none_or(|device| {
            device.name != hello.device_name
                || device.address != address
                || device_allowed_scope(device) != hello.sync_scope
                || key_changed
        });
        let heartbeat_due = existing
            .as_ref()
            .map(|device| !mesh_seen_recently(device.last_seen_at.as_deref()))
            .unwrap_or(true);
        let changed_for_device = structural_changed || heartbeat_due;
        if changed_for_device {
            upsert_group_device(
                &mut store,
                group_id,
                MeshDevice {
                    id: hello.device_id.clone(),
                    name: hello.device_name.clone(),
                    address,
                    last_seen_at: Some(now_string()),
                    trusted,
                    sync_scope: selected_scope,
                    allowed_sync_scope: Some(hello.sync_scope.clone()),
                    auto_account_sync,
                    encrypted_routing_api_key: encrypted_key,
                    encrypted_mesh_credential: hello
                        .device_credential
                        .as_deref()
                        .map(|value| encrypt_secret(value.as_bytes(), &key))
                        .transpose()?
                        .or_else(|| {
                            existing
                                .as_ref()
                                .and_then(|device| device.encrypted_mesh_credential.clone())
                        }),
                    credential_fingerprint: hello
                        .device_credential
                        .as_deref()
                        .map(credential_fingerprint)
                        .or_else(|| {
                            existing
                                .as_ref()
                                .and_then(|device| device.credential_fingerprint.clone())
                        }),
                    revoked_at: existing
                        .as_ref()
                        .and_then(|device| device.revoked_at.clone()),
                },
            )?;
            changed = true;
            if structural_changed {
                sync_needed = true;
                push_event(
                    &mut store,
                    "info",
                    &format!("已自动发现共享设备：{}", hello.device_name),
                );
            }
        }
    }
    if changed {
        save_store(app, &store)?;
        // Account sharing is the default automatic action. Rules, routing
        // and conversations remain available through the explicit sync
        // action and are limited by the remote device's advertised scope.
        if sync_needed {
            let sync_app = app.clone();
            let sync_group_id = group_id.to_string();
            let _ = thread::Builder::new()
                .name("mesh-auto-account-sync".to_string())
                .spawn(move || run_group_auto_account_sync(&sync_app, &sync_group_id));
        }
    }
    Ok(())
}

fn request_mesh_hello(
    ip: &str,
    port: u16,
    group_id: &str,
    token: &str,
    device_credential: Option<&str>,
    device_id: &str,
    device_name: &str,
    virtual_ipv4: Option<&str>,
    routing_url: Option<&str>,
    routing_key: Option<&str>,
    sync_scope: Option<&str>,
) -> Option<MeshHello> {
    let base_url = format!("http://{ip}:{port}");
    let mut builder = Client::builder()
        .connect_timeout(MESH_HELLO_TIMEOUT)
        .timeout(MESH_HELLO_TIMEOUT);
    if should_use_mesh_socks5(&base_url) {
        builder =
            builder.proxy(reqwest::Proxy::all(mesh_socks5_proxy_url_for_group(group_id)).ok()?);
    }
    let client = builder.build().ok()?;
    let mut request = client
        .get(format!("{base_url}{MESH_HELLO_PATH}"))
        .header(MESH_GROUP_ID_HEADER, group_id)
        .header(MESH_HELLO_TOKEN_HEADER, token)
        .header(MESH_HELLO_DEVICE_ID_HEADER, device_id)
        .header(MESH_HELLO_DEVICE_NAME_HEADER, device_name);
    if let Some(value) = device_credential {
        request = request.header(MESH_DEVICE_CREDENTIAL_HEADER, value);
    }
    if let Some(value) = virtual_ipv4 {
        request = request.header(MESH_HELLO_VIRTUAL_IPV4_HEADER, value);
    }
    if let Some(value) = routing_url {
        request = request.header(MESH_HELLO_ROUTING_URL_HEADER, value);
    }
    if let Some(value) = routing_key {
        request = request.header(MESH_HELLO_ROUTING_KEY_HEADER, value);
    }
    if let Some(value) = sync_scope {
        request = request.header(MESH_HELLO_SYNC_SCOPE_HEADER, value);
    }
    let response = request.send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let hello = response.json::<MeshHello>().ok()?;
    (hello.format == MESH_SHARE_FORMAT && hello.version == 1).then_some(hello)
}

fn spawn_mesh_routing_loop(
    app: AppHandle,
    manager: Arc<NetworkInstanceManager>,
    instance_id: uuid::Uuid,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_error: Option<String> = None;
        while !stop.load(AtomicOrdering::Relaxed) {
            let result = catch_unwind(AssertUnwindSafe(|| {
                let has_virtual_ipv4 = collect_network_infos_safe(&manager)
                    .ok()
                    .and_then(|infos| infos.get(&instance_id).cloned())
                    .and_then(|info| info.my_node_info)
                    .and_then(|node| node.virtual_ipv4)
                    .is_some();

                if has_virtual_ipv4 {
                    if crate::routing::is_running() {
                        Ok(())
                    } else {
                        crate::routing::start_for_mesh_share(app.clone()).map(|_| ())
                    }
                } else {
                    Ok(())
                }
            }));
            match result {
                Ok(Ok(())) => last_error = None,
                Ok(Err(error)) => {
                    if last_error.as_deref() != Some(error.as_str()) {
                        record_mesh_routing_error(&app, &error);
                        last_error = Some(error);
                    }
                }
                Err(panic) => {
                    let error = format!(
                        "mesh routing startup panicked: {}",
                        panic_payload_message(panic.as_ref())
                    );
                    if last_error.as_deref() != Some(error.as_str()) {
                        record_mesh_routing_error(&app, &error);
                        last_error = Some(error);
                    }
                }
            }

            std::thread::sleep(Duration::from_secs(1));
        }
    })
}

fn record_mesh_routing_error(app: &AppHandle, error: &str) {
    if let Ok(mut store) = load_store(app) {
        push_event(
            &mut store,
            "warn",
            &format!("多设备共享 API 自动启动失败：{error}"),
        );
        let _ = save_store(app, &store);
    }
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

fn spawn_group_account_sync_loop(
    app: AppHandle,
    group_id: String,
    stop: Arc<AtomicBool>,
    sync_secs: u64,
) -> JoinHandle<()> {
    thread::spawn(move || {
        run_group_auto_account_sync(&app, &group_id);
        let interval = sync_secs.clamp(60, 86_400);
        loop {
            for _ in 0..interval {
                if stop.load(AtomicOrdering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_secs(1));
            }
            run_group_auto_account_sync(&app, &group_id);
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
        .filter_map(|peer| {
            url::Url::parse(peer).ok().map(|uri| PeerConfig {
                uri,
                peer_public_key: None,
            })
        })
        .collect::<Vec<_>>();
    config.set_peers(peer_configs);
    // Give every app instance a stable address in the private overlay. DHCP
    // can leave a no-TUN node without an address until another node joins,
    // which prevents the first share handshake from ever reaching us.
    let virtual_ipv4 = local_mesh_ipv4_cidr().parse().map_err(display_err)?;
    config.set_dhcp(false);
    config.set_ipv4(Some(virtual_ipv4));

    // Use EasyTier's no-TUN mode so the desktop app does not need Wintun,
    // administrator privileges, or a system virtual adapter. The virtual
    // address is still assigned by EasyTier; outbound HTTP to another device
    // is sent through the internal loopback SOCKS5 portal below.
    let mut flags = config.get_flags();
    flags.no_tun = true;
    // Keep EasyTier's in-process TCP/UDP stack active for the SOCKS5 portal.
    // This is the SDK equivalent of the CLI's no-TUN networking path.
    flags.use_smoltcp = true;
    config.set_flags(flags);
    config.set_socks5_portal(Some(
        format!("socks5://127.0.0.1:{MESH_GROUP_SOCKS5_PORT_BASE}")
            .parse()
            .map_err(display_err)?,
    ));
    Ok(config)
}

fn build_easytier_group_config(
    group: &MeshShareGroup,
    network_secret: &str,
    peers: &[String],
) -> Result<TomlConfigLoader, String> {
    let config = TomlConfigLoader::new_from_str("").map_err(display_err)?;
    config.set_network_identity(NetworkIdentity::new(
        group.name.clone(),
        network_secret.to_string(),
    ));
    config.set_inst_name(format!(
        "codex-switcher-{}-{}",
        group.group_id,
        local_device_id()
    ));
    config.set_hostname(Some(local_device_name()));
    config.set_listeners(
        [
            format!("tcp://0.0.0.0:{}", group.listen_port),
            format!("udp://0.0.0.0:{}", group.listen_port),
        ]
        .into_iter()
        .map(|value| value.parse().map_err(display_err))
        .collect::<Result<Vec<url::Url>, String>>()?,
    );
    config.set_peers(
        peers
            .iter()
            .filter_map(|peer| {
                url::Url::parse(peer).ok().map(|uri| PeerConfig {
                    uri,
                    peer_public_key: None,
                })
            })
            .collect(),
    );
    config.set_dhcp(false);
    config.set_ipv4(Some(
        format!(
            "{}/{}",
            mesh_ipv4_in_cidr(&group.virtual_cidr, &local_device_id())?,
            cidr_prefix(&group.virtual_cidr)?
        )
        .parse()
        .map_err(display_err)?,
    ));
    let mut flags = config.get_flags();
    flags.no_tun = true;
    flags.use_smoltcp = true;
    config.set_flags(flags);
    config.set_socks5_portal(Some(
        format!("socks5://127.0.0.1:{}", group.socks5_port)
            .parse()
            .map_err(display_err)?,
    ));
    Ok(config)
}

fn runtime_holder() -> &'static Mutex<BTreeMap<String, MeshRuntime>> {
    MESH_RUNTIMES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn network_manager() -> Arc<NetworkInstanceManager> {
    MESH_MANAGER
        .get_or_init(|| Arc::new(NetworkInstanceManager::new()))
        .clone()
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
        .to_string()
}

fn collect_network_infos_safe(
    manager: &NetworkInstanceManager,
) -> Result<BTreeMap<uuid::Uuid, NetworkInstanceRunningInfo>, String> {
    if MESH_RUNTIME_INFO_UNAVAILABLE.load(AtomicOrdering::Acquire) {
        return Err("mesh runtime information is temporarily unavailable".to_string());
    }

    match catch_unwind(AssertUnwindSafe(|| manager.collect_network_infos_sync())) {
        Ok(Ok(infos)) => Ok(infos),
        Ok(Err(error)) => Err(display_err(error)),
        Err(panic) => {
            MESH_RUNTIME_INFO_UNAVAILABLE.store(true, AtomicOrdering::Release);
            Err(format!(
                "mesh runtime information panicked: {}",
                panic_payload_message(panic.as_ref())
            ))
        }
    }
}

fn mesh_lifecycle_guard() -> std::sync::MutexGuard<'static, ()> {
    MESH_LIFECYCLE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn last_error_holder() -> &'static Mutex<BTreeMap<String, String>> {
    MESH_LAST_RUNTIME_ERRORS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn set_runtime(runtime: MeshRuntime) -> Result<(), String> {
    let mut holder = runtime_holder().lock().map_err(display_err)?;
    holder.insert(runtime.group_id.clone(), runtime);
    Ok(())
}

fn stop_runtime() {
    stop_group_runtime(MESH_GROUP_LEGACY_ID);
}

fn stop_group_runtime(group_id: &str) {
    let runtime = runtime_holder()
        .lock()
        .ok()
        .and_then(|mut holder| holder.remove(group_id));
    if let Some(mut runtime) = runtime {
        runtime.refresh_stop.store(true, AtomicOrdering::Relaxed);
        if let Some(thread) = runtime.refresh_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = runtime.account_sync_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = runtime.peer_discovery_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = runtime.routing_thread.take() {
            let _ = thread.join();
        }
        let _ = runtime
            .manager
            .delete_network_instance(vec![runtime.instance_id]);
    }
}

fn set_last_runtime_error(error: Option<String>) {
    set_group_runtime_error(MESH_GROUP_LEGACY_ID, error);
}

fn set_group_runtime_error(group_id: &str, error: Option<String>) {
    if let Ok(mut holder) = last_error_holder().lock() {
        if let Some(error) = error {
            holder.insert(group_id.to_string(), error);
        } else {
            holder.remove(group_id);
        }
    }
}

fn runtime_snapshot() -> MeshRuntimeSnapshot {
    group_runtime_snapshot(MESH_GROUP_LEGACY_ID)
}

fn group_runtime_snapshot(group_id: &str) -> MeshRuntimeSnapshot {
    let last_error = last_error_holder()
        .lock()
        .ok()
        .and_then(|errors| errors.get(group_id).cloned());
    let Ok(holder) = runtime_holder().lock() else {
        return MeshRuntimeSnapshot {
            last_error,
            ..MeshRuntimeSnapshot::default()
        };
    };
    let Some(runtime) = holder.get(group_id) else {
        return MeshRuntimeSnapshot {
            last_error,
            ..MeshRuntimeSnapshot::default()
        };
    };
    let manager = runtime.manager.clone();
    let instance_id = runtime.instance_id;
    let fallback_peers = runtime.peers.clone();
    let runtime_kind = runtime.runtime_kind.clone();
    let started_at = runtime.started_at.clone();
    let fallback_virtual_ipv4 = runtime.virtual_ipv4.clone();
    drop(holder);

    let info = collect_network_infos_safe(&manager)
        .ok()
        .and_then(|infos| infos.get(&instance_id).cloned());
    let runtime_error = info.as_ref().and_then(|value| value.error_msg.clone());
    let running = info.as_ref().map(|value| value.running).unwrap_or(true);
    let virtual_ipv4 = info
        .as_ref()
        .and_then(|value| value.my_node_info.as_ref())
        .and_then(|node| node.virtual_ipv4.as_ref())
        .map(ToString::to_string)
        .map(|value| value.split('/').next().unwrap_or(&value).to_string());
    let virtual_ipv4 = virtual_ipv4.or(Some(fallback_virtual_ipv4));
    let peer_count = info
        .as_ref()
        .map(|value| value.peers.len())
        .or_else(|| Some(fallback_peers.len()));
    let peers = info
        .as_ref()
        .map(|value| {
            value
                .routes
                .iter()
                .filter_map(|route| {
                    let ip = route
                        .ipv4_addr
                        .as_ref()
                        .and_then(|inet| inet.address.as_ref())
                        .map(ToString::to_string);
                    let name = if route.hostname.trim().is_empty() {
                        format!("Peer {}", route.peer_id)
                    } else {
                        route.hostname.clone()
                    };
                    let latency_ms = route
                        .path_latency_latency_first
                        .or_else(|| (route.path_latency > 0).then_some(route.path_latency))
                        .map(|value| value as f64);
                    if ip.is_none() {
                        None
                    } else {
                        Some(MeshPeerView {
                            peer_id: route.peer_id,
                            name,
                            ip,
                            latency_ms,
                        })
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(error) = runtime_error.clone() {
        set_group_runtime_error(group_id, Some(error));
    }
    MeshRuntimeSnapshot {
        running,
        runtime_kind: Some(runtime_kind),
        process_id: None,
        executable_path: None,
        peer_count,
        peers,
        virtual_ipv4,
        started_at: Some(started_at),
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
                if byte.is_ascii() {
                    value.push(byte as char);
                    continue;
                }
                let character = std::str::from_utf8(&self.input[start..])
                    .map_err(display_err)?
                    .chars()
                    .next()
                    .ok_or_else(|| "invalid JavaScript UTF-8 string".to_string())?;
                self.position = start + character.len_utf8();
                value.push(character);
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
        if store.settings.mesh.network_name == default_network_name() {
            store.settings.mesh.network_name = unique_network_name();
        }
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        store.settings.mesh.encrypted_network_secret = Some(encrypt_secret(
            URL_SAFE_NO_PAD.encode(bytes).as_bytes(),
            key,
        )?);
    }
    Ok(())
}

fn unique_network_name() -> String {
    let mut hasher = Sha256::new();
    hasher.update(local_device_id().as_bytes());
    format!(
        "codex-switcher-{}",
        URL_SAFE_NO_PAD
            .encode(&hasher.finalize()[..6])
            .to_ascii_lowercase()
    )
}

fn ensure_local_device_credential(store: &mut AppStore, key: &[u8; 32]) -> Result<String, String> {
    if let Some(envelope) = store
        .settings
        .mesh
        .encrypted_local_device_credential
        .as_ref()
    {
        return String::from_utf8(decrypt_secret(envelope, key)?).map_err(display_err);
    }
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let credential = URL_SAFE_NO_PAD.encode(bytes);
    store.settings.mesh.encrypted_local_device_credential =
        Some(encrypt_secret(credential.as_bytes(), key)?);
    Ok(credential)
}

fn credential_fingerprint(credential: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(credential.as_bytes());
    URL_SAFE_NO_PAD.encode(&hasher.finalize()[..12])
}

fn request_header(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.to_string().eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str().trim().to_string())
        .filter(|value| !value.is_empty())
}

fn request_group_id(request: &Request) -> String {
    request_header(request, MESH_GROUP_ID_HEADER)
        .unwrap_or_else(|| MESH_GROUP_LEGACY_ID.to_string())
}

fn authorize_device_request(
    request: &Request,
    store: &AppStore,
    group_id: &str,
    key: &[u8; 32],
) -> Result<(), (u16, &'static str)> {
    let Some(device_id) = request_header(request, MESH_HELLO_DEVICE_ID_HEADER) else {
        // Legacy clients only have the shared routing key. Keep this fallback
        // for old installations; new clients always send a per-device token.
        return Ok(());
    };
    let Some(device) = group_devices(store, group_id)
        .ok()
        .and_then(|devices| devices.iter().find(|device| device.id == device_id))
    else {
        return Err((403, "device is not authorized"));
    };
    if !device.trusted || device.revoked_at.is_some() {
        return Err((403, "device access has been revoked"));
    }
    let expected = device
        .encrypted_mesh_credential
        .as_ref()
        .and_then(|envelope| decrypt_secret(envelope, key).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let Some(expected) = expected else {
        // A device imported by an older build has no per-device credential
        // yet. Keep the shared routing-key fallback until its next hello
        // upgrades the record.
        return Ok(());
    };
    let Some(provided) = request_header(request, MESH_DEVICE_CREDENTIAL_HEADER) else {
        return Err((401, "device credential is required"));
    };
    if expected != provided {
        return Err((401, "invalid device credential"));
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

fn group_network_secret(
    store: &AppStore,
    group_id: &str,
    key: &[u8; 32],
) -> Result<String, String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        decrypt_network_secret(store, key)
    } else {
        decrypt_group_network_secret(find_group(store, group_id)?, key)
    }
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

fn merge_shared_peers_into_group(group: &mut MeshShareGroup, payload: &MeshSharePayload) {
    for peer in &payload.peers {
        let Some(peer) = normalize_peer_url(peer) else {
            continue;
        };
        if group
            .nodes
            .iter()
            .any(|node| normalize_peer_url(&node.address).as_ref() == Some(&peer))
        {
            continue;
        }
        group.nodes.push(MeshPublicNode {
            id: hash_id(&peer),
            name: peer.clone(),
            address: peer,
            group: Some(group.group_id.clone()),
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
    let Some(host) = advertised_routing_host(
        &store.settings.routing.listen_host,
        runtime.virtual_ipv4.as_deref(),
    ) else {
        return (None, None);
    };
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

fn routing_share_for_group(
    store: &AppStore,
    group_id: &str,
    include_key: bool,
    key: &[u8; 32],
) -> (Option<String>, Option<String>) {
    if !include_key {
        return (None, None);
    }
    let runtime = group_runtime_snapshot(group_id);
    let Some(host) = advertised_routing_host(
        &store.settings.routing.listen_host,
        runtime.virtual_ipv4.as_deref(),
    ) else {
        return (None, None);
    };
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

fn advertised_routing_host(listen_host: &str, virtual_ipv4: Option<&str>) -> Option<String> {
    if matches!(listen_host, "0.0.0.0" | "::") {
        virtual_ipv4.map(ToString::to_string)
    } else {
        Some(display_mesh_host(listen_host))
    }
}

fn mesh_socks5_proxy_url() -> String {
    format!("socks5h://127.0.0.1:{MESH_GROUP_SOCKS5_PORT_BASE}")
}

fn mesh_socks5_proxy_url_for_group(group_id: &str) -> String {
    if group_id == MESH_GROUP_LEGACY_ID {
        return mesh_socks5_proxy_url();
    }
    let port = runtime_holder()
        .lock()
        .ok()
        .and_then(|runtimes| runtimes.get(group_id).map(|runtime| runtime.socks5_port))
        .unwrap_or(MESH_GROUP_SOCKS5_PORT_BASE);
    format!("socks5h://127.0.0.1:{port}")
}

fn should_use_mesh_socks5(base_url: &str) -> bool {
    let Ok(url) = url::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    host.trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .map(|address| !address.is_loopback())
        .unwrap_or(true)
}

fn mesh_seen_recently(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let Some(seen_at) = parse_time(value) else {
        return false;
    };
    let age = chrono::Utc::now().signed_duration_since(seen_at);
    age >= chrono::Duration::zero() && age <= chrono::Duration::seconds(MESH_ONLINE_TTL_SECS)
}

fn local_device_id() -> String {
    let name = local_device_name();
    hash_id(&format!("{}:{}", std::env::consts::OS, name))
}

fn local_mesh_ipv4() -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"codex-switcher-mesh-ip:v1:");
    hasher.update(local_device_id().as_bytes());
    let digest = hasher.finalize();
    let host = 2 + (u16::from_be_bytes([digest[0], digest[1]]) % 253) as u8;
    format!("10.126.126.{host}")
}

pub(crate) fn local_mesh_source() -> (String, Option<String>) {
    let runtime_ip = runtime_snapshot().virtual_ipv4;
    (
        local_device_name(),
        runtime_ip.or_else(|| Some(local_mesh_ipv4())),
    )
}

fn local_mesh_ipv4_cidr() -> String {
    format!("{}/24", local_mesh_ipv4())
}

fn mesh_ipv4_for_device_id(device_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"codex-switcher-mesh-ip:v1:");
    hasher.update(device_id.as_bytes());
    let digest = hasher.finalize();
    let host = 2 + (u16::from_be_bytes([digest[0], digest[1]]) % 253) as u8;
    format!("10.126.126.{host}")
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

fn default_mesh_group_cidr() -> String {
    MESH_GROUP_DEFAULT_CIDR.to_string()
}

fn default_mesh_group_port() -> u16 {
    EASYTIER_DEFAULT_PORT
}

fn default_mesh_group_socks5_port() -> u16 {
    MESH_GROUP_SOCKS5_PORT_BASE
}

fn find_group<'a>(store: &'a AppStore, group_id: &str) -> Result<&'a MeshShareGroup, String> {
    store
        .settings
        .mesh
        .groups
        .iter()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())
}

fn group_devices_mut<'a>(
    store: &'a mut AppStore,
    group_id: &str,
) -> Result<&'a mut Vec<MeshDevice>, String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        return Ok(&mut store.settings.mesh.authorized_devices);
    }
    store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
        .map(|group| &mut group.authorized_devices)
        .ok_or_else(|| "mesh group not found".to_string())
}

fn group_devices<'a>(store: &'a AppStore, group_id: &str) -> Result<&'a [MeshDevice], String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        return Ok(&store.settings.mesh.authorized_devices);
    }
    Ok(&find_group(store, group_id)?.authorized_devices)
}

fn upsert_group_device(
    store: &mut AppStore,
    group_id: &str,
    device: MeshDevice,
) -> Result<(), String> {
    let devices = group_devices_mut(store, group_id)?;
    if let Some(existing) = devices.iter_mut().find(|existing| existing.id == device.id) {
        *existing = device;
    } else {
        devices.push(device);
    }
    Ok(())
}

fn random_encrypted_secret(key: &[u8; 32]) -> Result<SecretEnvelope, String> {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    encrypt_secret(URL_SAFE_NO_PAD.encode(bytes).as_bytes(), key)
}

fn decrypt_group_network_secret(group: &MeshShareGroup, key: &[u8; 32]) -> Result<String, String> {
    let envelope = group
        .encrypted_network_secret
        .as_ref()
        .ok_or_else(|| "mesh group network secret is missing".to_string())?;
    String::from_utf8(decrypt_secret(envelope, key)?).map_err(display_err)
}

fn ensure_group_local_device_credential(
    group: &mut MeshShareGroup,
    key: &[u8; 32],
) -> Result<String, String> {
    if let Some(envelope) = group.encrypted_local_device_credential.as_ref() {
        return String::from_utf8(decrypt_secret(envelope, key)?).map_err(display_err);
    }
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let credential = URL_SAFE_NO_PAD.encode(bytes);
    group.encrypted_local_device_credential = Some(encrypt_secret(credential.as_bytes(), key)?);
    Ok(credential)
}

fn ensure_local_group_credential(
    store: &mut AppStore,
    group_id: &str,
    key: &[u8; 32],
) -> Result<String, String> {
    if group_id == MESH_GROUP_LEGACY_ID {
        return ensure_local_device_credential(store, key);
    }
    let group = store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())?;
    ensure_group_local_device_credential(group, key)
}

fn parse_ipv4_cidr(cidr: &str) -> Result<(u32, u8), String> {
    let (address, prefix) = cidr
        .trim()
        .split_once('/')
        .ok_or_else(|| "virtualCidr must use IPv4 CIDR notation".to_string())?;
    let address = address
        .parse::<std::net::Ipv4Addr>()
        .map_err(|_| "virtualCidr must contain a valid IPv4 address".to_string())?;
    let prefix = prefix
        .parse::<u8>()
        .map_err(|_| "virtualCidr prefix is invalid".to_string())?;
    if !(16..=29).contains(&prefix) {
        return Err("virtualCidr prefix must be between /16 and /29".to_string());
    }
    let mask = u32::MAX << (32 - prefix);
    Ok((u32::from(address) & mask, prefix))
}

fn cidr_prefix(cidr: &str) -> Result<u8, String> {
    parse_ipv4_cidr(cidr).map(|(_, prefix)| prefix)
}

fn cidrs_overlap(left: &str, right: &str) -> Result<bool, String> {
    let (left_network, left_prefix) = parse_ipv4_cidr(left)?;
    let (right_network, right_prefix) = parse_ipv4_cidr(right)?;
    let prefix = left_prefix.min(right_prefix);
    let mask = u32::MAX << (32 - prefix);
    Ok((left_network & mask) == (right_network & mask))
}

fn validate_group_cidr(
    cidr: &str,
    groups: &[MeshShareGroup],
    except_group_id: Option<&str>,
) -> Result<(), String> {
    parse_ipv4_cidr(cidr)?;
    for group in groups {
        if except_group_id == Some(group.group_id.as_str()) {
            continue;
        }
        if cidrs_overlap(cidr, &group.virtual_cidr)? {
            return Err(format!(
                "virtualCidr overlaps mesh group {}",
                group.group_id
            ));
        }
    }
    Ok(())
}

fn allocate_group_cidr(groups: &[MeshShareGroup]) -> String {
    (1u16..=254)
        .map(|slot| format!("10.127.{slot}.0/24"))
        .find(|candidate| {
            groups
                .iter()
                .all(|group| !cidrs_overlap(candidate, &group.virtual_cidr).unwrap_or(true))
        })
        .unwrap_or_else(|| "172.30.254.0/24".to_string())
}

fn mesh_ipv4_in_cidr(cidr: &str, device_id: &str) -> Result<String, String> {
    let (network, prefix) = parse_ipv4_cidr(cidr)?;
    let host_bits = 32 - prefix;
    let host_capacity = (1u32 << host_bits) - 2;
    let mut hasher = Sha256::new();
    hasher.update(b"codex-switcher-mesh-group-ip:v1:");
    hasher.update(device_id.as_bytes());
    let digest = hasher.finalize();
    let host =
        1 + (u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % host_capacity);
    Ok(std::net::Ipv4Addr::from(network + host).to_string())
}

fn mesh_ipv4_for_group_device(group_id: &str, device_id: &str) -> String {
    runtime_holder()
        .lock()
        .ok()
        .and_then(|runtimes| {
            runtimes
                .get(group_id)
                .and_then(|runtime| mesh_ipv4_in_cidr(&runtime.virtual_cidr, device_id).ok())
        })
        .unwrap_or_else(|| mesh_ipv4_for_device_id(device_id))
}

fn allocate_group_listen_port(groups: &[MeshShareGroup]) -> u16 {
    (0u16..1000)
        .map(|offset| EASYTIER_DEFAULT_PORT.saturating_add(offset * MESH_GROUP_PORT_STEP))
        .find(|port| groups.iter().all(|group| group.listen_port != *port))
        .unwrap_or(EASYTIER_DEFAULT_PORT + MESH_GROUP_PORT_STEP)
}

fn allocate_group_socks5_port(groups: &[MeshShareGroup]) -> u16 {
    (0u16..1000)
        .map(|offset| MESH_GROUP_SOCKS5_PORT_BASE.saturating_add(offset))
        .find(|port| groups.iter().all(|group| group.socks5_port != *port))
        .unwrap_or(MESH_GROUP_SOCKS5_PORT_BASE + 1)
}

fn validate_group_ports(
    listen_port: u16,
    socks5_port: u16,
    groups: &[MeshShareGroup],
    except_group_id: Option<&str>,
) -> Result<(), String> {
    if listen_port == 0 || socks5_port == 0 || listen_port == socks5_port {
        return Err("listenPort and socks5Port must be distinct non-zero ports".to_string());
    }
    if groups.iter().any(|group| {
        except_group_id != Some(group.group_id.as_str())
            && (group.listen_port == listen_port
                || group.socks5_port == socks5_port
                || group.listen_port == socks5_port
                || group.socks5_port == listen_port)
    }) {
        return Err("mesh group port is already allocated".to_string());
    }
    Ok(())
}

/// Lazily migrates the pre-group mesh settings into one stable legacy group.
/// The old fields remain authoritative for existing commands and are mirrored
/// back into this group before persistence by `main.rs`.
pub(crate) fn migrate_settings(settings: &mut MeshSettings) -> bool {
    if !settings.groups.is_empty() {
        return false;
    }
    settings.groups.push(MeshShareGroup {
        group_id: MESH_GROUP_LEGACY_ID.to_string(),
        name: settings.network_name.clone(),
        encrypted_network_secret: settings.encrypted_network_secret.clone(),
        nodes: settings.cached_nodes.clone(),
        authorized_devices: settings.authorized_devices.clone(),
        enabled: settings.enabled,
        auto_start: settings.auto_start,
        sync_scope: if settings.sync_scope_initialized {
            settings.sync_scope.clone()
        } else {
            MeshSyncScope::default()
        },
        virtual_cidr: default_mesh_group_cidr(),
        listen_port: default_mesh_group_port(),
        socks5_port: default_mesh_group_socks5_port(),
        runtime: MeshGroupRuntimeState::default(),
        credential_grants: Vec::new(),
        encrypted_local_device_credential: settings.encrypted_local_device_credential.clone(),
        legacy_compat: true,
    });
    true
}

pub(crate) fn sync_legacy_settings(settings: &mut MeshSettings) {
    let name = settings.network_name.clone();
    let secret = settings.encrypted_network_secret.clone();
    let nodes = settings.cached_nodes.clone();
    let devices = settings.authorized_devices.clone();
    let enabled = settings.enabled;
    let auto_start = settings.auto_start;
    let scope = settings.sync_scope.clone();
    let Some(group) = settings
        .groups
        .iter_mut()
        .find(|group| group.group_id == MESH_GROUP_LEGACY_ID)
    else {
        return;
    };
    group.name = name;
    group.encrypted_network_secret = secret;
    group.nodes = nodes;
    group.authorized_devices = devices;
    group.enabled = enabled;
    group.auto_start = auto_start;
    group.sync_scope = scope;
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
            group_id: Some("group-a".to_string()),
            group_name: Some("Home devices".to_string()),
            device_credential: None,
            credential_fingerprint: None,
        };

        let encoded = encode_payload(&payload).unwrap();
        let decoded = decode_payload(&encoded).unwrap();

        assert_eq!(decoded.format, MESH_SHARE_FORMAT);
        assert_eq!(decoded.mode, MeshShareMode::ContinuousSync);
        assert_eq!(decoded.network_secret, "secret");
        assert_eq!(decoded.group_id.as_deref(), Some("group-a"));
    }

    #[test]
    fn allocates_isolated_network_resources_for_another_group() {
        let existing = MeshShareGroup {
            group_id: "group-a".to_string(),
            name: "group-a-network".to_string(),
            encrypted_network_secret: None,
            nodes: Vec::new(),
            authorized_devices: Vec::new(),
            enabled: true,
            auto_start: true,
            sync_scope: MeshSyncScope::default(),
            virtual_cidr: "10.127.1.0/24".to_string(),
            listen_port: EASYTIER_DEFAULT_PORT,
            socks5_port: MESH_GROUP_SOCKS5_PORT_BASE,
            runtime: MeshGroupRuntimeState::default(),
            credential_grants: Vec::new(),
            encrypted_local_device_credential: None,
            legacy_compat: false,
        };
        let groups = vec![existing];
        let cidr = allocate_group_cidr(&groups);
        let listen_port = allocate_group_listen_port(&groups);
        let socks5_port = allocate_group_socks5_port(&groups);

        assert!(!cidrs_overlap(&cidr, &groups[0].virtual_cidr).unwrap());
        assert_ne!(listen_port, groups[0].listen_port);
        assert_ne!(socks5_port, groups[0].socks5_port);
        assert!(validate_group_ports(listen_port, socks5_port, &groups, None).is_ok());
    }

    #[test]
    fn rejects_invalid_payload_text() {
        assert!(decode_payload("not-a-payload").is_err());
    }

    #[test]
    fn builds_mesh_pull_endpoint_from_routing_url() {
        assert_eq!(
            pull_endpoint("http://10.126.126.2:15722/v1").unwrap(),
            "http://10.126.126.2:15722/mesh/pull"
        );
    }

    #[test]
    fn routes_remote_mesh_addresses_through_internal_socks5() {
        assert!(should_use_mesh_socks5("http://10.126.126.2:15722/v1"));
        assert!(should_use_mesh_socks5("http://mesh-device.local:15722/v1"));
        assert!(!should_use_mesh_socks5("http://127.0.0.1:15722/v1"));
        assert!(!should_use_mesh_socks5("http://[::1]:15722/v1"));
        assert_eq!(mesh_socks5_proxy_url(), "socks5h://127.0.0.1:22333");
    }

    #[test]
    fn remote_scope_limits_requested_sync_scope() {
        let requested = MeshSyncScope {
            accounts: true,
            rules: true,
            routing: true,
            conversations: true,
        };
        let allowed = MeshSyncScope {
            accounts: true,
            rules: false,
            routing: false,
            conversations: false,
        };
        assert_eq!(
            intersect_mesh_scopes(&requested, &allowed),
            MeshSyncScope {
                accounts: true,
                rules: false,
                routing: false,
                conversations: false,
            }
        );
    }

    #[test]
    fn mesh_hello_token_is_stable_without_exposing_network_secret() {
        let first = mesh_hello_token("network-secret");
        assert_eq!(first, mesh_hello_token("network-secret"));
        assert_ne!(first, "network-secret");
        assert_ne!(first, mesh_hello_token("another-secret"));
    }

    #[test]
    fn mesh_ipv4_is_stable_and_inside_private_overlay_range() {
        let first = mesh_ipv4_for_device_id("device-a");
        assert_eq!(first, mesh_ipv4_for_device_id("device-a"));
        assert_ne!(first, mesh_ipv4_for_device_id("device-b"));
        let address = first.parse::<IpAddr>().unwrap();
        assert!(matches!(address, IpAddr::V4(value) if value.octets()[0..3] == [10, 126, 126]));
        assert!(matches!(address, IpAddr::V4(value) if (2..=254).contains(&value.octets()[3])));
        assert!(local_mesh_ipv4_cidr().ends_with("/24"));
    }

    #[test]
    fn does_not_advertise_loopback_routing_before_mesh_ip_exists() {
        assert_eq!(advertised_routing_host("0.0.0.0", None), None);
        assert_eq!(
            advertised_routing_host("0.0.0.0", Some("10.126.126.2")),
            Some("10.126.126.2".to_string())
        );
        assert_eq!(
            advertised_routing_host("127.0.0.1", None),
            Some("127.0.0.1".to_string())
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
