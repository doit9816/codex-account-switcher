import { invoke } from "@tauri-apps/api/core";
import { Button, Input, Switch, Select, Tabs, Alert, Modal, InputNumber, Collapse, Checkbox, Card } from "antd";

function isTauriRuntime() {
  return Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);
}
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { confirm as confirmDialog, open, save } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import {
  Archive,
  CheckCircle2,
  Copy,
  Download,
  FileSearch,
  FileText,
  FolderOpen,
  Gauge,
  Grid2X2,
  HardDriveUpload,
  KeyRound,
  LayoutDashboard,
  Network,
  Pencil,
  RefreshCcw,
  RotateCcw,
  Rows3,
  Share2,
  Settings,
  ShieldCheck,
  Trash2,
  Upload,
  Wifi,
  X,
  Zap
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { logAppError } from "./api/appLog";
import { RoutingPage } from "./components/routing/RoutingPage";
import { RoutingLogSettings } from "./components/routing/RoutingLogSettings";
import { MeshSharePage } from "./components/mesh/MeshSharePage";
import { AppNavigation } from "./components/AppNavigation";
import { OpenCodeZenPreset, OPENCODE_ZEN_BASE_URL, OPENCODE_ZEN_FREE_MODELS } from "./components/OpenCodeZenPreset";
import { ProfileRuleFields } from "./components/ProfileRuleFields";
import { StatusPill } from "./components/StatusPill";
import type {
  AuthSummary,
  BundleManifest,
  CodexConfigFiles,
  CodexScan,
  DetectedLimit,
  LanguageSetting,
  MeshDevice,
  MeshGroup,
  MeshGroupStatus,
  MeshImportResult,
  MeshShareMode,
  MeshStatus,
  MeshSyncScope,
  Notice,
  OAuthEvent,
  OAuthLoginSession,
  Profile,
  QuotaRule,
  RoutingLogEntry,
  RoutingProbeResult,
  RoutingStatus,
  StoreView
} from "./types";
import { emptyQuota, languageLabels, messages, resolveLanguage, type I18n } from "./i18n";
import {
  accountState,
  authSummariesMatch,
  formatDate,
  formatReset,
  formatResetDateTime,
  formatResetExpiry,
  formatSubscriptionValidity,
  formatUsage,
  isConversationFile,
  isCooling,
  limitRemainingPercent,
  localizedLimitLabel,
  localizeDetectedText,
  normalizeNumber,
  planBadge,
  profileNeedsReauthorization,
  profileUsageError,
  profileScore,
  quotaSummary,
  subscriptionExpiryState,
  tokenState
} from "./profileUtils";

const UI_BUSY_TIMEOUT_MS = 90_000;
const ACCOUNT_PAGE_SIZE = 12;
const MESH_SHARE_PAYLOADS_STORAGE_KEY = "codex-account-switcher-mesh-share-payloads";

function readSavedMeshSharePayloads(): Record<string, string> {
  try {
    const value = localStorage.getItem(MESH_SHARE_PAYLOADS_STORAGE_KEY);
    const parsed: unknown = value ? JSON.parse(value) : {};
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return Object.fromEntries(
        Object.entries(parsed).filter((entry): entry is [string, string] => typeof entry[1] === "string"),
      );
    }
  } catch {
    // Ignore unavailable or malformed browser storage; users can generate a new code.
  }
  return {};
}

function saveMeshSharePayload(groupId: string, payload: string) {
  try {
    const saved = readSavedMeshSharePayloads();
    saved[groupId] = payload;
    localStorage.setItem(MESH_SHARE_PAYLOADS_STORAGE_KEY, JSON.stringify(saved));
  } catch {
    // Keep the generated payload usable in memory when browser storage is unavailable.
  }
}

type AccountViewMode = "cards" | "rows";
type AccountExpiryFilter = "all" | "valid" | "expired";
type AccountStatusFilter = "all" | "available" | "relogin" | "disabled" | "cooling" | "expired" | "error";

function generatedApiProviderId(alias: string, model: string) {
  const source = `${alias || model || "api"}`
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 32);
  return `${source || "api"}-${Date.now().toString(36)}`;
}

function apiProtocolLabel(wireApi: string, t: I18n) {
  if (wireApi === "chat_completions") return t.apiProtocolChat;
  if (wireApi === "anthropic_messages") return t.apiProtocolAnthropic;
  return t.apiProtocolResponses;
}

function updaterProxyUrl(store: StoreView | null) {
  const proxy = store?.settings.probeProxy;
  const url = proxy?.url.trim();
  return proxy?.enabled && url ? url : undefined;
}

function isAccountExpired(profile: Profile, t: I18n) {
  return subscriptionExpiryState(profile).expired || tokenState(profile, t) === t.expired;
}

function isAccountUsable(profile: Profile, t: I18n) {
  return accountState(profile, t) === t.available && !isAccountExpired(profile, t);
}

function matchesAccountStatus(profile: Profile, statusFilter: AccountStatusFilter, t: I18n) {
  const state = accountState(profile, t);
  const token = tokenState(profile, t);
  if (statusFilter === "all") return true;
  if (statusFilter === "available") return state === t.available && token !== t.expired;
  if (statusFilter === "relogin") return profileNeedsReauthorization(profile) || token === t.reloginRequired || token === t.authInvalid;
  if (statusFilter === "disabled") return !profile.enabled;
  if (statusFilter === "cooling") return isCooling(profile);
  if (statusFilter === "expired") return isAccountExpired(profile, t);
  return !!profile.usage.lastError || token === t.keepaliveFailed || state === t.probeFailed;
}

export default function App() {
  const [store, setStore] = useState<StoreView | null>(null);
  const [scan, setScan] = useState<CodexScan | null>(null);
  const [selectedId, setSelectedId] = useState<string>("");
  const [codexHome, setCodexHome] = useState("");
  const [alias, setAlias] = useState("");
  const [showAddAccountDialog, setShowAddAccountDialog] = useState(false);
  const [editingProfileId, setEditingProfileId] = useState<string | null>(null);
  const [addAccountTab, setAddAccountTab] = useState<"oauth" | "json" | "api" | "import">("oauth");
  const [authJsonInput, setAuthJsonInput] = useState("");
  const [oauthSession, setOauthSession] = useState<OAuthLoginSession | null>(null);
  const [oauthStatus, setOauthStatus] = useState<"idle" | "starting" | "waiting" | "exchanging" | "error" | "timeout">("idle");
  const [oauthError, setOauthError] = useState("");
  const [oauthCallbackInput, setOauthCallbackInput] = useState("");
  const [oauthRemainingSeconds, setOauthRemainingSeconds] = useState(0);
  const [oauthReauthProfileId, setOauthReauthProfileId] = useState<string | null>(null);
  const [accountFilter, setAccountFilter] = useState("");
  const [apiProviderName, setApiProviderName] = useState("");
  const [apiProviderId, setApiProviderId] = useState("");
  const [apiBaseUrl, setApiBaseUrl] = useState("");
  const [apiModel, setApiModel] = useState("");
  const [apiWireApi, setApiWireApi] = useState("responses");
  const [apiKey, setApiKey] = useState("");
  const [accountViewMode, setAccountViewMode] = useState<AccountViewMode>(() => {
    const saved = localStorage.getItem("codex-account-switcher-account-view");
    return saved === "rows" ? "rows" : "cards";
  });
  const [accountExpiryFilter, setAccountExpiryFilter] = useState<AccountExpiryFilter>("all");
  const [accountStatusFilter, setAccountStatusFilter] = useState<AccountStatusFilter>("all");
  const [accountPage, setAccountPage] = useState(1);
  const [codexConfig, setCodexConfig] = useState<CodexConfigFiles | null>(null);
  const [authJsonDraft, setAuthJsonDraft] = useState("");
  const [configTomlDraft, setConfigTomlDraft] = useState("");
  const [password, setPassword] = useState("");
  const [includeConversations, setIncludeConversations] = useState(false);
  const [exportProfileIds, setExportProfileIds] = useState<string[]>([]);
  const [restoreConversations, setRestoreConversations] = useState(false);
  const [forceSwitch, setForceSwitch] = useState(false);
  const [proxyEnabled, setProxyEnabled] = useState(false);
  const [proxyUrl, setProxyUrl] = useState("");
  const [backgroundTokenRefreshEnabled, setBackgroundTokenRefreshEnabled] = useState(false);
  const [backgroundTokenRefreshIntervalSecs, setBackgroundTokenRefreshIntervalSecs] = useState(3600);
  const [tokenRefreshThresholdSecs, setTokenRefreshThresholdSecs] = useState(0);
  const [autoProbeEnabled, setAutoProbeEnabled] = useState(true);
  const [autoProbeIntervalSecs, setAutoProbeIntervalSecs] = useState(60);
  const [autoProbeRunning, setAutoProbeRunning] = useState(false);
  const [routingStatus, setRoutingStatus] = useState<RoutingStatus | null>(null);
  const [routingHost, setRoutingHost] = useState("0.0.0.0");
  const [routingPort, setRoutingPort] = useState(15722);
  const [routingMode, setRoutingMode] = useState<"auto" | "fixed">("auto");
  const [routingFixedProfileId, setRoutingFixedProfileId] = useState("");
  const [routingStickyTtlSecs, setRoutingStickyTtlSecs] = useState(3600);
  const [routingLogRetentionDays, setRoutingLogRetentionDays] = useState(7);
  const [routingBusy, setRoutingBusy] = useState(false);
  const [meshStatus, setMeshStatus] = useState<MeshStatus | null>(null);
  const [meshSelectedGroupId, setMeshSelectedGroupId] = useState<string | null>(null);
  const [meshGroupStatus, setMeshGroupStatus] = useState<MeshGroupStatus | null>(null);
  const [meshShareMode, setMeshShareMode] = useState<MeshShareMode>("joinOnly");
  const [meshSharePayload, setMeshSharePayload] = useState("");
  const [meshImportPayload, setMeshImportPayload] = useState("");
  const [meshNetworkName, setMeshNetworkName] = useState("codex-switcher");
  const [meshNetworkSecret, setMeshNetworkSecret] = useState("");
  const [meshNodeSourceUrl, setMeshNodeSourceUrl] = useState("https://info.qtet.cn/uptime/easytier");
  const [meshNodeRefreshSecs, setMeshNodeRefreshSecs] = useState(120);
  const [meshAutoStart, setMeshAutoStart] = useState(false);
  const [meshSyncScope, setMeshSyncScope] = useState<MeshSyncScope>({
    accounts: true,
    rules: false,
    routing: false,
    conversations: false
  });
  const [meshMigrationUseSecret, setMeshMigrationUseSecret] = useState(true);
  const [editAliasDraft, setEditAliasDraft] = useState("");
  const [editNoteDraft, setEditNoteDraft] = useState("");
  const [editQuotaDraft, setEditQuotaDraft] = useState<QuotaRule>(emptyQuota);
  const [editPriorityDraft, setEditPriorityDraft] = useState(100);
  const [editEnabledDraft, setEditEnabledDraft] = useState(true);
  const [editProviderIdDraft, setEditProviderIdDraft] = useState("");
  const [editBaseUrlDraft, setEditBaseUrlDraft] = useState("");
  const [editModelDraft, setEditModelDraft] = useState("");
  const [editWireApiDraft, setEditWireApiDraft] = useState("responses");
  const [editApiKeyDraft, setEditApiKeyDraft] = useState("");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);
  const [initializing, setInitializing] = useState(true);
  const [activePage, setActivePage] = useState<"dashboard" | "mesh" | "routing" | "settings">("dashboard");
  const [appVersion, setAppVersion] = useState("");
  const [availableUpdate, setAvailableUpdate] = useState<Update | null>(null);
  const [showUpdateDialog, setShowUpdateDialog] = useState(false);
  const [updateChecked, setUpdateChecked] = useState(false);
  const [updateError, setUpdateError] = useState("");
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateInstalling, setUpdateInstalling] = useState(false);
  const [updateInstalled, setUpdateInstalled] = useState(false);
  const [updateDownloaded, setUpdateDownloaded] = useState(0);
  const [updateTotal, setUpdateTotal] = useState<number | undefined>(undefined);
  const [languageSetting, setLanguageSetting] = useState<LanguageSetting>(() => {
    const saved = localStorage.getItem("codex-account-switcher-language");
    return saved === "zh-CN" || saved === "en" || saved === "zh-TW" || saved === "system" ? saved : "system";
  });
  const language = useMemo(() => resolveLanguage(languageSetting), [languageSetting]);
  const t = messages[language];

  const selectedProfile = useMemo(
    () => store?.profiles.find((profile) => profile.id === selectedId),
    [store, selectedId]
  );
  const editingProfile = useMemo(
    () => store?.profiles.find((profile) => profile.id === editingProfileId),
    [store?.profiles, editingProfileId]
  );
  const currentGlobalProfileId = useMemo(() => {
    const configuredId = store?.settings.currentProfileId;
    const configuredProfile = store?.profiles.find((profile) => profile.id === configuredId);
    if (configuredProfile?.apiConfig) return configuredProfile.id;
    const currentAuth = scan?.currentAuth;
    if (currentAuth) {
      const matched = store?.profiles.find((profile) => authSummariesMatch(profile.summary, currentAuth));
      if (matched) return matched.id;
    }
    return store?.settings.currentProfileId;
  }, [scan?.currentAuth, store?.profiles, store?.settings.currentProfileId]);
  const updateProgressPercent = updateTotal
    ? Math.min(100, Math.round((updateDownloaded / updateTotal) * 100))
    : undefined;
  const filteredProfiles = useMemo(() => {
    const profiles = (store?.profiles || [])
      .map((profile, index) => ({ profile, index }))
      .sort((left, right) => {
        const usableDelta = Number(isAccountUsable(right.profile, t)) - Number(isAccountUsable(left.profile, t));
        if (usableDelta !== 0) return usableDelta;
        const expiryDelta = Number(isAccountExpired(left.profile, t)) - Number(isAccountExpired(right.profile, t));
        if (expiryDelta !== 0) return expiryDelta;
        return left.index - right.index;
      })
      .map((item) => item.profile);
    const query = accountFilter.trim().toLowerCase();
    return profiles.filter((profile) => {
      const expired = isAccountExpired(profile, t);
      if (accountExpiryFilter === "valid" && expired) return false;
      if (accountExpiryFilter === "expired" && !expired) return false;
      if (!matchesAccountStatus(profile, accountStatusFilter, t)) return false;
      if (!query) return true;
      const values = [
        profile.alias,
        profile.note,
        profile.summary.email,
        profile.summary.accountId,
        profile.summary.plan,
        profile.summary.authMode,
        accountState(profile, t),
        formatSubscriptionValidity(profile, t),
        quotaSummary(profile, t),
        tokenState(profile, t),
        currentGlobalProfileId === profile.id ? t.currentUsing : ""
      ];
      return values.some((value) => String(value || "").toLowerCase().includes(query));
    });
  }, [store?.profiles, accountFilter, accountExpiryFilter, accountStatusFilter, currentGlobalProfileId, t]);
  const totalAccountPages = Math.max(1, Math.ceil(filteredProfiles.length / ACCOUNT_PAGE_SIZE));
  const pagedProfiles = filteredProfiles.slice((accountPage - 1) * ACCOUNT_PAGE_SIZE, accountPage * ACCOUNT_PAGE_SIZE);
  const selectedExportProfiles = useMemo(() => {
    const selected = new Set(exportProfileIds);
    return (store?.profiles || []).filter((profile) => selected.has(profile.id));
  }, [store?.profiles, exportProfileIds]);
  const meshGroups = useMemo<MeshGroup[]>(() => (meshStatus?.groups || []).map((group) => {
    const selectedStatus = meshGroupStatus?.group.groupId === group.groupId ? meshGroupStatus : null;
    return {
      id: group.groupId,
      name: group.name,
      runtimeStatus: group.runtime.lastError
        ? "error"
        : group.runtime.running
          ? "running"
          : "stopped",
      onlineDeviceCount: selectedStatus?.devices.filter((device) => device.online && !device.revokedAt).length
        ?? group.onlineDeviceCount,
      deviceCount: group.deviceCount,
      selected: group.groupId === meshSelectedGroupId,
      lastError: group.runtime.lastError
    };
  }), [meshStatus?.groups, meshGroupStatus, meshSelectedGroupId]);
  const passwordTooShort = password.length > 0 && password.length < 8;
  const selectedIdRef = useRef("");
  const autoBusyRef = useRef(false);
  const backgroundTokenBusyRef = useRef(false);
  const aliasRef = useRef("");
  const oauthCompletingRef = useRef(false);
  const storeRef = useRef<StoreView | null>(null);
  const proxyDraftDirtyRef = useRef(false);
  const autoSettingsDirtyRef = useRef(false);
  const oauthReauthProfileIdRef = useRef<string | null>(null);

  useEffect(() => {
    void refresh().finally(() => setInitializing(false));
    void getVersion().then(setAppVersion).catch((error) => {
      setAppVersion("");
      void logAppError("get_app_version", error);
    });
    const updateTimer = window.setTimeout(() => {
      void checkForUpdate(false);
    }, 3500);
    return () => window.clearTimeout(updateTimer);
  }, []);

  useEffect(() => {
    selectedIdRef.current = selectedId;
  }, [selectedId]);

  useEffect(() => {
    aliasRef.current = alias;
  }, [alias]);

  useEffect(() => {
    storeRef.current = store;
  }, [store]);

  useEffect(() => {
    oauthReauthProfileIdRef.current = oauthReauthProfileId;
  }, [oauthReauthProfileId]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let unlistenCompleted: (() => void) | undefined;
    let unlistenTimeout: (() => void) | undefined;
    void listen<OAuthEvent>("codex-oauth-login-completed", (event) => {
      void completeNativeOAuth(event.payload.loginId);
    }).then((unlisten) => {
      unlistenCompleted = unlisten;
    });
    void listen<OAuthEvent>("codex-oauth-login-timeout", (event) => {
      setOauthSession((current) => current?.loginId === event.payload.loginId ? null : current);
      setOauthStatus("timeout");
      setOauthError(t.oauthTimedOut);
    }).then((unlisten) => {
      unlistenTimeout = unlisten;
    });
    return () => {
      unlistenCompleted?.();
      unlistenTimeout?.();
    };
  }, []);

  useEffect(() => {
    if (!oauthSession) {
      setOauthRemainingSeconds(0);
      return;
    }
    const updateRemaining = () => {
      const remaining = Math.max(0, Math.ceil((new Date(oauthSession.expiresAt).getTime() - Date.now()) / 1000));
      setOauthRemainingSeconds(remaining);
      if (remaining === 0 && oauthStatus === "waiting") {
        setOauthStatus("timeout");
        setOauthError(t.oauthTimedOut);
      }
    };
    updateRemaining();
    const id = window.setInterval(updateRemaining, 1000);
    return () => window.clearInterval(id);
  }, [oauthSession, oauthStatus, t.oauthTimedOut]);

  useEffect(() => {
    localStorage.setItem("codex-account-switcher-language", languageSetting);
  }, [languageSetting]);

  useEffect(() => {
    localStorage.setItem("codex-account-switcher-account-view", accountViewMode);
  }, [accountViewMode]);

  useEffect(() => {
    const ids = (meshStatus?.groups || []).map((group) => group.groupId);
    if (ids.length === 0) {
      setMeshSelectedGroupId(null);
      setMeshGroupStatus(null);
      return;
    }
    if (!meshSelectedGroupId || !ids.includes(meshSelectedGroupId)) {
      setMeshSelectedGroupId(ids[0]);
    }
  }, [meshStatus?.groups, meshSelectedGroupId]);

  useEffect(() => {
    const saved = readSavedMeshSharePayloads();
    setMeshSharePayload(saved[meshSelectedGroupId ?? "legacy"] ?? "");
  }, [meshSelectedGroupId]);

  useEffect(() => {
    if (!meshSelectedGroupId || activePage !== "mesh") return;
    void invoke<MeshGroupStatus>("mesh_group_status", { groupId: meshSelectedGroupId })
      .then((groupStatus) => {
        setMeshGroupStatus(groupStatus);
        setMeshSyncScope(groupStatus.group.syncScope);
      })
      .catch((error) => {
        setMeshGroupStatus(null);
        void logAppError("load_mesh_group_status", error);
      });
  }, [meshSelectedGroupId, activePage]);

  useEffect(() => {
    setAccountPage(1);
  }, [accountFilter, accountExpiryFilter, accountStatusFilter]);

  useEffect(() => {
    setAccountPage((page) => Math.min(page, totalAccountPages));
  }, [totalAccountPages]);

  useEffect(() => {
    if (!store) return;
    const current = store.settings.currentProfileId || store.profiles[0]?.id || "";
    setSelectedId((old) => old || current);
    setCodexHome(store.settings.codexHome || "");
    // Keep persisted settings in sync after startup/update, but never overwrite
    // a draft while the user is editing it.
    if (!proxyDraftDirtyRef.current) {
      setProxyEnabled(!!store.settings.probeProxy?.enabled);
      setProxyUrl(store.settings.probeProxy?.url || "");
    }
    if (!autoSettingsDirtyRef.current) {
      setBackgroundTokenRefreshEnabled(store.settings.backgroundTokenRefreshEnabled ?? false);
      setBackgroundTokenRefreshIntervalSecs(store.settings.backgroundTokenRefreshIntervalSecs || 3600);
      setTokenRefreshThresholdSecs(store.settings.tokenRefreshThresholdSecs ?? 0);
      setAutoProbeEnabled(store.settings.autoProbeEnabled ?? true);
      setAutoProbeIntervalSecs(store.settings.autoProbeIntervalSecs || 60);
    }
    const mesh = store.settings.mesh;
    if (mesh) {
      setMeshNetworkName(mesh.networkName || "codex-switcher");
      setMeshNodeSourceUrl(mesh.nodeSourceUrl || "https://info.qtet.cn/uptime/easytier");
      setMeshNodeRefreshSecs(mesh.nodeRefreshSecs || 120);
      setMeshAutoStart(!!mesh.autoStart);
      if (!meshSelectedGroupId || meshSelectedGroupId === "legacy") {
        setMeshSyncScope(
          mesh.syncScopeInitialized === false
            ? { accounts: true, rules: false, routing: false, conversations: false }
            : mesh.syncScope || { accounts: true, rules: false, routing: false, conversations: false }
        );
      }
    }
  }, [store, meshSelectedGroupId]);

  // Keep the routing form aligned with an actual backend status response.
  // The store is refreshed in the background every few seconds; syncing the
  // draft fields from it made an unsaved mode change jump back unexpectedly.
  useEffect(() => {
    const settings = routingStatus?.settings;
    if (!settings) return;
    setRoutingHost(settings.listenHost || "0.0.0.0");
    setRoutingPort(settings.port || 15722);
    setRoutingMode(settings.mode || "auto");
    setRoutingFixedProfileId(settings.fixedProfileId || "");
    setRoutingStickyTtlSecs(settings.stickyTtlSecs || 3600);
    setRoutingLogRetentionDays(settings.logRetentionDays || 7);
  }, [routingStatus]);

  useEffect(() => {
    const profileIds = (store?.profiles || []).map((profile) => profile.id);
    setExportProfileIds((current) => {
      const retained = current.filter((id) => profileIds.includes(id));
      if (retained.length > 0 || current.length > 0) return retained;
      return profileIds;
    });
  }, [store?.profiles]);

  useEffect(() => {
    if (!store?.settings.autoProbeEnabled) return;
    const intervalMs = Math.max(30, store.settings.autoProbeIntervalSecs || 60) * 1000;
    const id = window.setInterval(() => {
      void autoProbeTick();
    }, intervalMs);
    return () => window.clearInterval(id);
  }, [store?.settings.autoProbeEnabled, store?.settings.autoProbeIntervalSecs]);

  useEffect(() => {
    if (!store?.settings.backgroundTokenRefreshEnabled) return;
    const intervalMs = Math.max(3600, store.settings.backgroundTokenRefreshIntervalSecs || 3600) * 1000;
    const id = window.setInterval(() => {
      void autoBackgroundTokenRefreshTick();
    }, intervalMs);
    return () => window.clearInterval(id);
  }, [
    store?.settings.backgroundTokenRefreshEnabled,
    store?.settings.backgroundTokenRefreshIntervalSecs,
    store?.settings.tokenRefreshThresholdSecs
  ]);

  useEffect(() => {
    const refreshStore = () => {
      void invoke<StoreView>("get_store")
        .then(setStore)
        .catch((error) => {
          void logAppError("background_refresh_store", error);
          // Background sync may be writing the store; retry on the next tick.
        });
    };
    refreshStore();
    const timer = window.setInterval(() => {
      if (activePage === "mesh") {
        void invoke<MeshStatus>("mesh_status")
          .then(setMeshStatus)
          .catch((error) => {
            void logAppError("background_refresh_mesh", error);
            // A stopped or restarting mesh runtime is reflected on the next tick.
          });
        if (meshSelectedGroupId) {
          void invoke<MeshGroupStatus>("mesh_group_status", { groupId: meshSelectedGroupId })
            .then(setMeshGroupStatus)
            .catch((error) => {
              void logAppError("background_refresh_mesh_group", error);
              // A group may be restarting or have just been removed from persisted state.
            });
        }
      }
      refreshStore();
    }, 3000);
    return () => window.clearInterval(timer);
  }, [activePage, meshSelectedGroupId]);

  useEffect(() => {
    if (!notice) return;
    const timeout = window.setTimeout(() => {
      setNotice(null);
    }, notice.kind === "error" ? 8000 : 4500);
    return () => window.clearTimeout(timeout);
  }, [notice]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let unlisten: (() => void) | undefined;
    void listen<string>("tray-action", (event) => {
      if (event.payload === "settings") {
        setActivePage("settings");
        return;
      }
      if (event.payload === "refresh") {
        void refresh();
        return;
      }
      if (event.payload === "probe-current") {
        void probeSelected();
        return;
      }
      if (event.payload === "auto-switch") {
        void autoSwitch();
      }
    }).then((cleanup) => {
      unlisten = cleanup;
    });
    return () => {
      unlisten?.();
    };
  }, [selectedId, store, codexHome, forceSwitch]);

  async function run<T>(task: () => Promise<T>, okText?: string, operation = "ui_action") {
    setBusy(true);
    setNotice(null);
    let timeoutId: number | undefined;
    try {
      const taskPromise = task();
      taskPromise.catch(() => undefined);
      const timeoutPromise = new Promise<never>((_, reject) => {
        timeoutId = window.setTimeout(() => {
          reject(new Error("操作超时，请稍后刷新状态或重试"));
        }, UI_BUSY_TIMEOUT_MS);
      });
      const result = await Promise.race([taskPromise, timeoutPromise]);
      if (okText) setNotice({ kind: "ok", text: okText });
      return result;
    } catch (error) {
      void logAppError(operation, error);
      setNotice({ kind: "error", text: String(error) });
      return undefined;
    } finally {
      if (timeoutId !== undefined) window.clearTimeout(timeoutId);
      setBusy(false);
    }
  }

  async function refresh() {
    try {
      const view = await invoke<StoreView>("get_store");
      setStore(view);
      const routing = await invoke<RoutingStatus>("routing_status");
      setRoutingStatus(routing);
      const mesh = await invoke<MeshStatus>("mesh_status");
      setMeshStatus(mesh);
      if (view.settings.codexHome) {
        const currentScan = await invoke<CodexScan>("scan_codex_home", {
          codexHome: view.settings.codexHome
        });
        setScan(currentScan);
      }
    } catch (error) {
      void logAppError("refresh_application_state", error);
      setNotice({ kind: "error", text: `${t.failed}: ${String(error)}` });
    }
  }

  async function scanHome() {
    await run(async () => {
      const currentScan = await invoke<CodexScan>("scan_codex_home", {
        codexHome: codexHome || undefined
      });
      setScan(currentScan);
      setCodexHome(currentScan.codexHome);
      return currentScan;
    }, t.scannedCodex);
  }

  async function openCodexHome() {
    await run(async () => {
      await invoke("open_codex_home", {
        codexHome: codexHome || undefined
      });
    }, t.openedCodex);
  }

  async function openLogsDirectory() {
    await run(async () => {
      await invoke("open_logs_directory");
    }, "已打开日志目录");
  }

  async function importCurrentAuth() {
    const result = await run(async () => {
      const view = await invoke<StoreView>("import_current_auth_as_profile", {
        codexHome: codexHome || undefined,
        alias: alias || undefined
      });
      setStore(view);
      setSelectedId(view.profiles[0]?.id || "");
      setAlias("");
      return view;
    }, t.importedCurrent);
    if (result) setShowAddAccountDialog(false);
  }

  async function startCliOAuthLogin() {
    await run(() => invoke<string>("start_codex_oauth_login"));
  }

  async function beginNativeOAuth() {
    setOauthStatus("starting");
    setOauthError("");
    setOauthCallbackInput("");
    try {
      const session = await invoke<OAuthLoginSession>("codex_oauth_login_start");
      setOauthSession(session);
      setOauthStatus("waiting");
    } catch (error) {
      void logAppError("oauth_login_start", error);
      setOauthStatus("error");
      setOauthError(String(error));
    }
  }

  async function completeNativeOAuth(loginId: string) {
    if (oauthCompletingRef.current) return;
    oauthCompletingRef.current = true;
    setOauthStatus("exchanging");
    setOauthError("");
    try {
      const reauthProfileId = oauthReauthProfileIdRef.current;
      const previousReauthProfile = reauthProfileId
        ? storeRef.current?.profiles.find((profile) => profile.id === reauthProfileId)
        : undefined;
      const view = await invoke<StoreView>("codex_oauth_login_complete", {
        loginId,
        alias: aliasRef.current || undefined
      });
      const updatedReauthProfile = reauthProfileId
        ? view.profiles.find((profile) => profile.id === reauthProfileId)
        : undefined;
      setStore(view);
      setSelectedId(
        updatedReauthProfile && updatedReauthProfile.updatedAt !== previousReauthProfile?.updatedAt
          ? updatedReauthProfile.id
          : view.profiles[view.profiles.length - 1]?.id || ""
      );
      setAlias("");
      setOauthReauthProfileId(null);
      setOauthSession(null);
      setOauthStatus("idle");
      setOauthCallbackInput("");
      setShowAddAccountDialog(false);
      setNotice({ kind: "ok", text: t.accountAdded });
    } catch (error) {
      void logAppError("oauth_login_complete", error);
      setOauthStatus("error");
      setOauthError(String(error));
    } finally {
      oauthCompletingRef.current = false;
    }
  }

  async function submitOAuthCallback() {
    if (!oauthSession || !oauthCallbackInput.trim()) return;
    setOauthError("");
    try {
      await invoke("codex_oauth_submit_callback_url", {
        loginId: oauthSession.loginId,
        callbackUrl: oauthCallbackInput
      });
    } catch (error) {
      void logAppError("oauth_submit_callback", error);
      setOauthStatus("error");
      setOauthError(String(error));
    }
  }

  async function reopenOAuthUrl() {
    if (!oauthSession) return;
    try {
      await invoke("codex_oauth_open_auth_url", { loginId: oauthSession.loginId });
    } catch (error) {
      void logAppError("oauth_open_authorization_url", error);
      setOauthError(String(error));
    }
  }

  async function copyOAuthUrl() {
    if (!oauthSession) return;
    try {
      await navigator.clipboard.writeText(oauthSession.authUrl);
      setNotice({ kind: "ok", text: t.copied });
    } catch (error) {
      void logAppError("oauth_copy_authorization_url", error);
      setOauthError(String(error));
    }
  }

  async function cancelNativeOAuth(nextStatus: "idle" | "error" = "error") {
    const loginId = oauthSession?.loginId;
    setOauthSession(null);
    setOauthStatus(nextStatus);
    setOauthError("");
    setOauthReauthProfileId(null);
    setOauthCallbackInput("");
    if (loginId) {
      try {
        await invoke("codex_oauth_login_cancel", { loginId });
      } catch (error) {
        void logAppError("oauth_login_cancel", error);
        // Session may already be completed or timed out.
      }
    }
  }

  function closeAddAccountDialog() {
    if (addAccountTab === "oauth") void cancelNativeOAuth("idle");
    setShowAddAccountDialog(false);
  }

  function selectAddAccountTab(tab: "oauth" | "json" | "api" | "import") {
    if (addAccountTab === "oauth" && tab !== "oauth") void cancelNativeOAuth("idle");
    setAddAccountTab(tab);
  }

  async function reauthorizeProfile(profile: Profile) {
    setSelectedId(profile.id);
    if (oauthSession) {
      await cancelNativeOAuth("idle");
    } else {
      setOauthSession(null);
      setOauthStatus("idle");
      setOauthError("");
      setOauthCallbackInput("");
    }
    setAlias(profile.alias);
    setOauthReauthProfileId(profile.id);
    setAddAccountTab("oauth");
    setShowAddAccountDialog(true);
    await beginNativeOAuth();
  }

  async function addAuthJsonAccount() {
    const result = await run(async () => {
      const view = await invoke<StoreView>("add_auth_json_profile", {
        alias: alias || undefined,
        authJson: authJsonInput
      });
      setStore(view);
      setSelectedId(view.profiles[view.profiles.length - 1]?.id || "");
      setAlias("");
      setAuthJsonInput("");
      return view;
    }, t.accountAdded);
    if (result) setShowAddAccountDialog(false);
  }

  function openEditProfile(profile: Profile) {
    setEditingProfileId(profile.id);
    setEditAliasDraft(profile.alias);
    setEditNoteDraft(profile.note || "");
    setEditQuotaDraft(profile.quotaRule);
    setEditPriorityDraft(profile.priority);
    setEditEnabledDraft(profile.enabled);
    setEditProviderIdDraft(profile.apiConfig?.providerId || "");
    setEditBaseUrlDraft(profile.apiConfig?.baseUrl || "");
    setEditModelDraft(profile.apiConfig?.model || "");
    setEditWireApiDraft(profile.apiConfig?.wireApi || "responses");
    setEditApiKeyDraft("");
  }

  function closeEditProfile() {
    setEditingProfileId(null);
    setEditApiKeyDraft("");
  }

  async function copyProfileAccount(profile: Profile) {
    const text = profile.summary.email || profile.summary.accountId || profile.apiConfig?.baseUrl || "";
    if (!text) return;
    await navigator.clipboard.writeText(text);
    setNotice({ kind: "ok", text: `${t.accountCopied}: ${text}` });
  }

  async function saveProfileDetails() {
    if (!editingProfile) return;
    await run(async () => {
      const view = await invoke<StoreView>("update_profile_details", {
        profileId: editingProfile.id,
        alias: editAliasDraft.trim(),
        note: editNoteDraft,
        hourlyLimit: normalizeNumber(editQuotaDraft.hourlyLimit),
        dailyLimit: normalizeNumber(editQuotaDraft.dailyLimit),
        cooldownMinutes: editQuotaDraft.cooldownMinutes || 180,
        enabled: editEnabledDraft,
        priority: editPriorityDraft,
        providerId: editingProfile.apiConfig ? editProviderIdDraft : undefined,
        baseUrl: editingProfile.apiConfig ? editBaseUrlDraft : undefined,
        model: editingProfile.apiConfig ? editModelDraft : undefined,
        wireApi: editingProfile.apiConfig ? editWireApiDraft : undefined,
        apiKey: editingProfile.apiConfig ? editApiKeyDraft : undefined
      });
      setStore(view);
      setSelectedId(editingProfile.id);
      closeEditProfile();
      return view;
    }, t.savedProfile);
  }

  async function saveProxySettings() {
    await run(async () => {
      const view = await invoke<StoreView>("save_proxy_settings", {
        enabled: proxyEnabled,
        url: proxyUrl
      });
      proxyDraftDirtyRef.current = false;
      setStore(view);
      return view;
    }, proxyEnabled ? t.savedProxy : t.disabledProxy);
  }

  async function testProxySettings() {
    await run(async () => {
      const result = await invoke<{ status: string; httpStatus?: number; elapsedMs: number; proxyUrl?: string; message: string }>(
        "test_proxy_settings",
        {
          enabled: proxyEnabled,
          url: proxyUrl
        }
      );
      setNotice({
        kind: result.status === "ok" ? "ok" : "warn",
        text: `${t.proxyTestOk}: HTTP ${result.httpStatus ?? "-"}, ${result.elapsedMs}ms`
      });
      return result;
    });
  }

  async function saveAutoSettings() {
    await run(async () => {
      const view = await invoke<StoreView>("save_auto_settings", {
        autoTokenRefreshEnabled: false,
        autoRefreshIntervalSecs: 600,
        backgroundTokenRefreshEnabled,
        backgroundTokenRefreshIntervalSecs,
        tokenRefreshThresholdSecs,
        autoProbeEnabled,
        autoProbeIntervalSecs
      });
      setStore(view);
      autoSettingsDirtyRef.current = false;
      return view;
    }, t.savedAuto);
  }

  async function reloadRoutingStatus() {
    const routing = await invoke<RoutingStatus>("routing_status");
    setRoutingStatus(routing);
    return routing;
  }

  async function reloadMeshStatus() {
    const mesh = await invoke<MeshStatus>("mesh_status");
    setMeshStatus(mesh);
    return mesh;
  }

  async function reloadMeshGroupStatus(groupId = meshSelectedGroupId) {
    if (!groupId) {
      setMeshGroupStatus(null);
      return null;
    }
    const groupStatus = await invoke<MeshGroupStatus>("mesh_group_status", { groupId });
    setMeshGroupStatus(groupStatus);
    return groupStatus;
  }

  async function createMeshGroup(input: { name: string; syncScope: MeshSyncScope }) {
    return await run(async () => {
      const groupStatus = await invoke<MeshGroupStatus>("mesh_group_create", {
        input: {
          name: input.name,
          enabled: true,
          autoStart: true,
          syncScope: input.syncScope
        }
      });
      setMeshSelectedGroupId(groupStatus.group.groupId);
      setMeshGroupStatus(groupStatus);
      setMeshSyncScope(groupStatus.group.syncScope);
      await reloadMeshStatus();
      return groupStatus;
    }, "分享组已创建");
  }

  async function importMeshGroup(input: { shareCode: string }) {
    return await run(async () => {
      const result = await invoke<MeshImportResult>("mesh_group_import", {
        payloadText: input.shareCode
      });
      await reloadMeshStatus();
      if (result.groupId) {
        setMeshSelectedGroupId(result.groupId);
        await reloadMeshGroupStatus(result.groupId);
      }
      return result;
    }, "分享组已加入");
  }

  async function selectMeshGroup(input: { groupId: string }) {
    setMeshSelectedGroupId(input.groupId);
    const groupStatus = await reloadMeshGroupStatus(input.groupId);
    if (groupStatus) setMeshSyncScope(groupStatus.group.syncScope);
    return groupStatus;
  }

  async function setMeshGroupRunning(groupId: string, running: boolean) {
    return await run(async () => {
      const groupStatus = await invoke<MeshGroupStatus>(
        running ? "mesh_group_start" : "mesh_group_stop",
        { groupId }
      );
      setMeshGroupStatus(groupStatus);
      await reloadMeshStatus();
      return groupStatus;
    }, running ? "分享组连接已启动" : "分享组连接已停止");
  }

  async function deleteMeshGroup(groupId: string) {
    return await run(async () => {
      await invoke("mesh_group_delete", { groupId });
      const mesh = await invoke<MeshStatus>("mesh_status");
      setMeshStatus(mesh);
      const remaining = mesh.groups ?? [];
      const nextGroupId = remaining[0]?.groupId ?? null;
      setMeshSelectedGroupId(nextGroupId);
      if (nextGroupId) {
        await reloadMeshGroupStatus(nextGroupId);
      } else {
        setMeshGroupStatus(null);
      }
      return mesh;
    }, "分享组已删除");
  }

  async function revokeMeshGroupDevice(input: { groupId: string; deviceId: string; revoked: boolean }) {
    return await run(async () => {
      const groupStatus = await invoke<MeshGroupStatus>("mesh_group_revoke", input);
      setMeshGroupStatus(groupStatus);
      await reloadMeshStatus();
      return groupStatus;
    }, input.revoked ? "已禁止该设备使用此分享组" : "已恢复该设备使用此分享组");
  }

  async function removeMeshGroupDevice(input: { groupId: string; deviceId: string }) {
    return await run(async () => {
      if (input.groupId === "legacy") {
        const mesh = await invoke<MeshStatus>("mesh_remove_device", { deviceId: input.deviceId });
        setMeshStatus(mesh);
        setMeshGroupStatus(null);
        return mesh;
      }
      const groupStatus = await invoke<MeshGroupStatus>("mesh_group_remove_device", input);
      setMeshGroupStatus(groupStatus);
      await reloadMeshStatus();
      return groupStatus;
    }, "设备已从当前分享组移除");
  }

  async function saveMeshGroupDevice(input: {
    groupId: string;
    deviceId: string;
    autoAccountSync: boolean;
    syncScope: MeshSyncScope;
  }) {
    return await run(async () => {
      const groupStatus = await invoke<MeshGroupStatus>("mesh_group_save_device_sync", input);
      setMeshGroupStatus(groupStatus);
      return groupStatus;
    }, "设备同步设置已保存");
  }

  async function syncMeshGroup(input: { groupId: string; deviceId?: string }) {
    return await run(async () => {
      const groupStatus = await invoke<MeshGroupStatus>("mesh_group_sync_now", input);
      setMeshGroupStatus(groupStatus);
      const refreshedStore = await invoke<StoreView>("get_store");
      setStore(refreshedStore);
      return groupStatus;
    }, "设备同步完成");
  }

  async function createMeshGroupShare(input: { groupId: string; mode: MeshShareMode }) {
    return await run(async () => {
      const payload = await invoke<string>("mesh_group_create_share_payload", input);
      setMeshSharePayload(payload);
      saveMeshSharePayload(input.groupId, payload);
      await reloadMeshGroupStatus(input.groupId);
      return payload;
    }, "分享码已生成");
  }

  async function saveMeshGroupScope(groupId: string, syncScope: MeshSyncScope) {
    const group = meshStatus?.groups?.find((item) => item.groupId === groupId);
    if (!group) return;
    return await run(async () => {
      const groupStatus = await invoke<MeshGroupStatus>("mesh_group_create", {
        input: {
          groupId,
          name: group.name,
          enabled: group.enabled,
          autoStart: group.autoStart,
          syncScope,
          virtualCidr: group.virtualCidr,
          listenPort: group.listenPort,
          socks5Port: group.socks5Port
        }
      });
      setMeshGroupStatus(groupStatus);
      await reloadMeshStatus();
      return groupStatus;
    }, "分享内容已保存");
  }

  async function saveMeshSettings() {
    await run(async () => {
      const mesh = await invoke<MeshStatus>("mesh_save_settings", {
        input: {
          enabled: meshStatus?.settings.enabled ?? false,
          autoStart: meshAutoStart,
          networkName: meshNetworkName,
          networkSecret: meshNetworkSecret || undefined,
          nodeSourceUrl: meshNodeSourceUrl,
          nodeRefreshSecs: meshNodeRefreshSecs,
          syncScope: meshSyncScope
        }
      });
      setMeshStatus(mesh);
      setMeshNetworkSecret("");
      await refresh();
      return mesh;
    }, "多设备共享设置已保存");
  }

  async function toggleMeshService() {
    await run(async () => {
      const mesh = await invoke<MeshStatus>(meshStatus?.running ? "mesh_stop" : "mesh_start");
      setMeshStatus(mesh);
      await refresh();
      return mesh;
    }, meshStatus?.running ? "设备连接已断开" : "设备连接已建立");
  }

  async function refreshMeshNodes() {
    await run(async () => {
      const mesh = await invoke<MeshStatus>("mesh_refresh_public_nodes");
      setMeshStatus(mesh);
      return mesh;
    }, "公共节点已刷新");
  }

  async function createMeshSharePayload() {
    await run(async () => {
      const saved = await invoke<MeshStatus>("mesh_save_settings", {
        input: {
          enabled: meshStatus?.settings.enabled ?? false,
          autoStart: meshAutoStart,
          networkName: meshNetworkName,
          networkSecret: meshNetworkSecret || undefined,
          nodeSourceUrl: meshNodeSourceUrl,
          nodeRefreshSecs: meshNodeRefreshSecs,
          syncScope: meshSyncScope
        }
      });
      setMeshStatus(saved);
      const payload = await invoke<string>("mesh_create_share_payload", {
        mode: meshShareMode
      });
      setMeshSharePayload(payload);
      saveMeshSharePayload("legacy", payload);
      await reloadMeshStatus();
      return payload;
    }, "共享码已生成");
  }

  async function copyMeshSharePayload() {
    if (!meshSharePayload.trim()) return;
    await navigator.clipboard.writeText(meshSharePayload);
    setNotice({ kind: "ok", text: "共享码已复制" });
  }

  async function importMeshSharePayload(payloadText = meshImportPayload) {
    if (!payloadText.trim()) return;
    return await run(async () => {
      const result = await invoke<MeshImportResult>("mesh_import_share_payload", {
        payloadText
      });
      setMeshImportPayload("");
      await refresh();
      return result;
    }, "共享码已导入");
  }

  async function saveMeshDevice(device: MeshDevice) {
    await run(async () => {
      const mesh = await invoke<MeshStatus>("mesh_save_device_sync", {
        deviceId: device.id,
        trusted: device.trusted,
        autoAccountSync: device.autoAccountSync === true,
        syncScope: device.syncScope
      });
      setMeshStatus(mesh);
      return mesh;
    }, "设备同步设置已保存");
  }

  async function syncMeshNow(deviceId?: string) {
    await run(async () => {
      const mesh = await invoke<MeshStatus>("mesh_sync_now", {
        deviceId
      });
      setMeshStatus(mesh);
      const refreshedStore = await invoke<StoreView>("get_store");
      setStore(refreshedStore);
      return mesh;
    }, "已请求设备同步");
  }

  async function authorizeMeshPeer(ip: string) {
    await run(async () => {
      const mesh = await invoke<MeshStatus>("mesh_authorize_peer_sync", { ip });
      setMeshStatus(mesh);
      const refreshedStore = await invoke<StoreView>("get_store");
      setStore(refreshedStore);
      return mesh;
    }, "设备已授权并同步");
  }

  async function exportMeshMigrationShare() {
    if (!meshMigrationUseSecret && passwordTooShort) {
      setNotice({ kind: "warn", text: t.passwordTooShortExport });
      return;
    }
    const selectableProfileIds = (store?.profiles || []).map((profile) => profile.id);
    const profileIds = exportProfileIds.filter((id) => selectableProfileIds.includes(id));
    if (profileIds.length === 0) {
      setNotice({ kind: "warn", text: t.exportNoAccount });
      return;
    }
    const path = await save({
      title: "导出迁移分享包",
      defaultPath: meshMigrationUseSecret || password ? "codex-switcher.mesh.zip.enc" : "codex-switcher.mesh.zip",
      filters: [{ name: "Codex Switcher Mesh Bundle", extensions: ["zip", "enc"] }]
    });
    if (!path) return;
    await run(async () => {
      const manifest = await invoke<BundleManifest>("mesh_export_migration_share", {
        outputPath: path,
        password,
        useMeshSecret: meshMigrationUseSecret,
        includeConversations,
        profileIds
      });
      setNotice({
        kind: meshMigrationUseSecret || password ? "ok" : "warn",
        text: `迁移分享包已导出: ${manifest.profileCount} ${t.accountCount}, ${manifest.files.length} ${t.configFiles}`
      });
      return manifest;
    });
  }

  async function importMeshMigrationShare() {
    if (!meshMigrationUseSecret && passwordTooShort) {
      setNotice({ kind: "warn", text: t.passwordTooShortImport });
      return;
    }
    const path = await open({
      title: "导入迁移分享包",
      multiple: false,
      directory: false,
      filters: [{ name: "Codex Switcher Mesh Bundle", extensions: ["zip", "enc"] }]
    });
    if (!path || Array.isArray(path)) return;
    await run(async () => {
      const result = await invoke<{ importedProfiles: number; restoredFiles: number; message: string }>(
        "mesh_import_migration_share",
        {
          bundlePath: path,
          password,
          useMeshSecret: meshMigrationUseSecret,
          restoreConversations,
          codexHome: codexHome || undefined
        }
      );
      await refresh();
      return result;
    }, "迁移分享包已导入");
  }

  async function saveRoutingSettings(enabled = routingStatus?.settings.enabled ?? false) {
    await run(async () => {
      const routing = await invoke<RoutingStatus>("routing_save_settings", {
        input: {
          listenHost: routingHost,
          port: routingPort,
          enabled,
          mode: routingMode,
          fixedProfileId: routingMode === "fixed" ? routingFixedProfileId || undefined : undefined,
          stickyTtlSecs: routingStickyTtlSecs
        }
      });
      setRoutingStatus(routing);
      await refresh();
      return routing;
    }, "路由设置已保存");
  }

  async function toggleRoutingService() {
    const running = routingStatus?.running;
    if (running && meshStatus?.running) {
      const stopMeshToo = await confirmDialog(
        "检测到多设备共享正在运行。它会自动拉起路由 API。是否同时停止多设备共享和路由 API？",
        { title: "停止路由 API", kind: "warning" }
      );
      if (!stopMeshToo) return;
    }
    setRoutingBusy(true);
    try {
      await run(async () => {
      if (running && meshStatus?.running) {
        const mesh = await invoke<MeshStatus>("mesh_stop");
        setMeshStatus(mesh);
      }
      const routing = await invoke<RoutingStatus>("routing_save_settings", {
        input: {
          listenHost: routingHost,
          port: routingPort,
          enabled: !running,
          mode: routingMode,
          fixedProfileId: routingMode === "fixed" ? routingFixedProfileId || undefined : undefined,
          stickyTtlSecs: routingStickyTtlSecs
        }
      });
      setRoutingStatus(routing);
      await refresh();
      return routing;
    }, running ? "路由服务已停止" : "路由服务已启动");
    } finally {
      setRoutingBusy(false);
    }
  }

  async function regenerateRoutingKey() {
    await run(async () => {
      const routing = await invoke<RoutingStatus>("routing_regenerate_access_key");
      setRoutingStatus(routing);
      return routing;
    }, "路由 API Key 已重新生成");
  }

  async function copyRoutingConfig() {
    const text = `base_url = "${routingStatus?.baseUrl || `http://${routingHost}:${routingPort}/v1`}"\napi_key = "${routingStatus?.accessKey || ""}"`;
    await navigator.clipboard.writeText(text);
    setNotice({ kind: "ok", text: "路由配置已复制" });
  }

  async function applyRoutingCodexConfig() {
    const codexRunning = await invoke<boolean>("is_codex_process_running");
    if (codexRunning) {
      const confirmed = await confirmDialog(t.routingTakeoverRestartConfirm, {
        title: t.routingTakeoverRestartTitle,
        kind: "warning"
      });
      if (!confirmed) return;
    }
    await run(async () => {
      const routing = await invoke<RoutingStatus>("routing_apply_codex_config", {
        restartCodex: codexRunning
      });
      setRoutingStatus(routing);
      await refresh();
      return routing;
    }, codexRunning ? t.routingTakeoverRestarted : t.routingTakeoverApplied);
  }

  async function restoreRoutingCodexConfig() {
    await run(async () => {
      const routing = await invoke<RoutingStatus>("routing_restore_codex_config");
      setRoutingStatus(routing);
      await refresh();
      return routing;
    }, "已恢复接管前配置");
  }

  async function fixProfileToRouting(profileId: string) {
    setRoutingMode("fixed");
    setRoutingFixedProfileId(profileId);
    await run(async () => {
      const routing = await invoke<RoutingStatus>("routing_save_settings", {
        input: {
          listenHost: routingHost,
          port: routingPort,
          enabled: routingStatus?.settings.enabled ?? false,
          mode: "fixed",
          fixedProfileId: profileId,
          stickyTtlSecs: routingStickyTtlSecs
        }
      });
      setRoutingStatus(routing);
      await refresh();
      return routing;
    }, "已固定到路由");
  }

  async function saveRoutingProfilePriority(profile: Profile, priority: number) {
    const saved = await run(async () => {
      const view = await invoke<StoreView>("save_quota_rule", {
        profileId: profile.id,
        alias: profile.alias,
        hourlyLimit: normalizeNumber(profile.quotaRule.hourlyLimit),
        dailyLimit: normalizeNumber(profile.quotaRule.dailyLimit),
        cooldownMinutes: profile.quotaRule.cooldownMinutes || 180,
        enabled: profile.enabled,
        priority
      });
      setStore(view);
      return view;
    }, "优先级已保存");
    return !!saved;
  }

  async function checkForUpdate(manual = true) {
    setUpdateChecking(true);
    try {
      const proxy = updaterProxyUrl(storeRef.current);
      const update = await check({
        timeout: 15000,
        ...(proxy ? { proxy } : {})
      });
      setUpdateChecked(true);
      setUpdateError("");
      setAvailableUpdate(update);
      if (update) setShowUpdateDialog(true);
      setUpdateInstalled(false);
      setUpdateDownloaded(0);
      setUpdateTotal(undefined);
      if (manual) {
        setNotice({
          kind: update ? "ok" : "info",
          text: update ? `${t.updateAvailable}: ${update.version}` : t.upToDate
        });
      } else if (update) {
        setNotice({ kind: "info", text: `${t.updateAvailable}: ${update.version}` });
      }
      return update;
    } catch (error) {
      void logAppError("check_for_update", error);
      const message = String(error);
      setUpdateChecked(true);
      setAvailableUpdate(null);
      setUpdateError(message);
      if (manual) setNotice({ kind: "error", text: `${t.updateCheckFailed}: ${message}` });
      return null;
    } finally {
      setUpdateChecking(false);
    }
  }

  async function installAvailableUpdate() {
    if (!availableUpdate) return;
    setUpdateInstalling(true);
    setUpdateDownloaded(0);
    setUpdateTotal(undefined);
    try {
      let totalBytes: number | undefined;
      await availableUpdate.downloadAndInstall((event: DownloadEvent) => {
        if (event.event === "Started") {
          totalBytes = event.data.contentLength;
          setUpdateDownloaded(0);
          setUpdateTotal(totalBytes);
        } else if (event.event === "Progress") {
          setUpdateDownloaded((value) => value + event.data.chunkLength);
        } else if (event.event === "Finished") {
          setUpdateDownloaded((value) => totalBytes || value);
        }
      });
      setUpdateInstalled(true);
      setNotice({ kind: "ok", text: t.updateInstalled });
    } catch (error) {
      void logAppError("install_update", error);
      setNotice({ kind: "error", text: `${t.updateInstallFailed}: ${String(error)}` });
    } finally {
      setUpdateInstalling(false);
    }
  }

  async function autoProbeTick() {
    if (autoBusyRef.current) return;
    autoBusyRef.current = true;
    setAutoProbeRunning(true);
    try {
      const latest = await invoke<StoreView>("get_store");
      const profiles = latest.profiles.filter((profile) => (
        profile.enabled &&
        !profile.apiConfig &&
        !isCooling(profile) &&
        tokenState(profile, t) !== t.reloginRequired &&
        tokenState(profile, t) !== t.authInvalid
      ));
      for (const profile of profiles) {
        await invoke("probe_usage", { profileId: profile.id });
      }
      const view = await invoke<StoreView>("get_store");
      setStore(view);
      if (activePage === "routing") {
        await reloadRoutingStatus();
      }
    } catch (error) {
      void logAppError("automatic_usage_probe", error);
      console.warn("auto probe failed", error);
    } finally {
      autoBusyRef.current = false;
      setAutoProbeRunning(false);
    }
  }

  async function autoBackgroundTokenRefreshTick() {
    if (backgroundTokenBusyRef.current) return;
    backgroundTokenBusyRef.current = true;
    try {
      await invoke("refresh_all_profile_tokens", {
        includeCurrent: false,
        thresholdSecs: store?.settings.tokenRefreshThresholdSecs || tokenRefreshThresholdSecs
      });
      const view = await invoke<StoreView>("get_store");
      setStore(view);
    } catch (error) {
      void logAppError("background_token_refresh", error);
      console.warn("background token refresh failed", error);
    } finally {
      backgroundTokenBusyRef.current = false;
    }
  }

  async function refreshOtherProfileTokensNow() {
    await run(async () => {
      const result = await invoke<{ refreshed: number; skipped: number; failed: number; message: string }>(
        "refresh_all_profile_tokens",
        {
          includeCurrent: false,
          thresholdSecs: tokenRefreshThresholdSecs
        }
      );
      await refresh();
      setNotice({
        kind: result.failed > 0 ? "warn" : "ok",
        text: `${t.tokenKeepaliveDone}: ${t.refreshed} ${result.refreshed}, ${t.skipped} ${result.skipped}, ${t.failed} ${result.failed}`
      });
      return result;
    });
  }

  async function shouldForceSwitch(profile: Profile) {
    if (forceSwitch) return true;
    const codexRunning = await invoke<boolean>("is_codex_process_running");
    if (!codexRunning) return false;
    const confirmed = await confirmDialog(
      `检测到 Codex 正在运行。\n\n如果继续，将先安全关闭 Codex、保存当前账号最新凭证，再切换到 ${profile.alias}，最后按当前设备的启动方式重新打开。是否继续？`,
      { title: "强制切换账号", kind: "warning" }
    );
    return confirmed ? true : null;
  }

  async function switchProfile(profileId = selectedId) {
    const profile = store?.profiles.find((item) => item.id === profileId);
    if (!profile) return;
    setSelectedId(profile.id);
    const apiProtocol = profile.apiConfig?.wireApi;
    if (apiProtocol && apiProtocol !== "responses") {
      await routeApiProfile(profile);
      return;
    }
    const restoreRoutingFirst = !!store?.settings.routing.appliedToCodex;
    if (restoreRoutingFirst) {
      const confirmed = await confirmDialog(
        `当前本机 Codex 已接管到路由 API。\n\n如果继续切换全局账号，会先恢复接管前配置，再写入 ${profile.alias}。如果你只是想让路由使用这个账号，请点“固定到路由”。是否继续？`,
        { title: "切换全局账号", kind: "warning" }
      );
      if (!confirmed) return;
    }
    const force = await shouldForceSwitch(profile);
    if (force == null) return;
    await run(async () => {
      if (restoreRoutingFirst) {
        const routing = await invoke<RoutingStatus>("routing_restore_codex_config");
        setRoutingStatus(routing);
      }
      const result = await invoke<{ message: string; codexRunning: boolean }>("switch_profile", {
        profileId: profile.id,
        codexHome: codexHome || undefined,
        force
      });
      await refresh();
      setNotice({ kind: result.message.includes("失败") || (result.codexRunning && !force) ? "warn" : "ok", text: result.message });
      return result;
    });
  }

  async function routeApiProfile(profile: Profile) {
    const codexRunning = await invoke<boolean>("is_codex_process_running");
    let restartCodex = false;
    if (codexRunning) {
      const confirmed = await confirmDialog(
        `检测到 Codex 正在运行。\n\n接管路由后需要重启 Codex，才能让新会话使用 ${profile.alias}。是否继续？`,
        { title: "固定账号到路由", kind: "warning" }
      );
      if (!confirmed) return;
      restartCodex = true;
    }
    await run(async () => {
      const saved = await invoke<RoutingStatus>("routing_save_settings", {
        input: {
          listenHost: routingHost,
          port: routingPort,
          enabled: true,
          mode: "fixed",
          fixedProfileId: profile.id,
          stickyTtlSecs: routingStickyTtlSecs
        }
      });
      setRoutingStatus(saved);
      const applied = await invoke<RoutingStatus>("routing_apply_codex_config", {
        restartCodex
      });
      setRoutingStatus(applied);
      await refresh();
      return applied;
    }, restartCodex ? "路由已启动并固定账号，Codex 已重启" : "路由已启动并固定账号，请新建 Codex 会话");
  }

  async function autoSwitch() {
    if (!store) return;
    const candidate = store.profiles
      .filter((profile) => profile.enabled && !isCooling(profile))
      .sort((a, b) => profileScore(b) - profileScore(a))[0];
    if (!candidate) {
      setNotice({ kind: "warn", text: t.noAvailableAccount });
      return;
    }
    setSelectedId(candidate.id);
    const force = await shouldForceSwitch(candidate);
    if (force == null) return;
    await run(async () => {
      if (store.settings.routing.appliedToCodex) {
        const routing = await invoke<RoutingStatus>("routing_restore_codex_config");
        setRoutingStatus(routing);
      }
      const result = await invoke<{ message: string; codexRunning: boolean }>("switch_profile", {
        profileId: candidate.id,
        codexHome: codexHome || undefined,
        force
      });
      await refresh();
      setNotice({
        kind: result.message.includes("失败") || (result.codexRunning && !force) ? "warn" : "ok",
        text: `${t.autoSelected} ${candidate.alias}: ${result.message}`
      });
      return result;
    });
  }

  async function probeProfile(profileId = selectedId) {
    const profile = store?.profiles.find((item) => item.id === profileId);
    if (!profile) return;
    setSelectedId(profile.id);
    await run(async () => {
      const result = await invoke<{ message: string; status: string; httpStatus?: number }>("probe_usage", {
        profileId: profile.id
      });
      await refresh();
      setNotice({
        kind: result.status === "ok" ? "ok" : "warn",
        text: `${result.message}${result.httpStatus ? ` HTTP ${result.httpStatus}` : ""}`
      });
      return result;
    });
  }

  async function probeSelected() {
    await probeProfile();
  }

  async function addApiProvider() {
    const result = await run(async () => {
      const providerAlias = apiProviderName.trim() || alias.trim() || apiModel.trim() || "API Key";
      const view = await invoke<StoreView>("add_api_profile", {
        alias: providerAlias,
        providerId: apiProviderId.trim() || generatedApiProviderId(providerAlias, apiModel),
        baseUrl: apiBaseUrl,
        model: apiModel,
        wireApi: apiWireApi,
        apiKey
      });
      setStore(view);
      setSelectedId(view.profiles[view.profiles.length - 1]?.id || "");
      setApiProviderName("");
      setAlias("");
      setApiProviderId("");
      setApiBaseUrl("");
      setApiModel("");
      setApiWireApi("responses");
      setApiKey("");
      return view;
    }, t.apiProviderAdded);
    if (result) setShowAddAccountDialog(false);
  }

  async function loadCodexConfigFiles() {
    await run(async () => {
      const files = await invoke<CodexConfigFiles>("load_codex_config_files", {
        codexHome: codexHome || undefined
      });
      setCodexConfig(files);
      setCodexHome(files.codexHome);
      setAuthJsonDraft(files.authJson.content);
      setConfigTomlDraft(files.configToml.content);
      return files;
    }, t.configLoaded);
  }

  async function formatCodexConfig(fileName: "auth.json" | "config.toml") {
    const content = fileName === "auth.json" ? authJsonDraft : configTomlDraft;
    await run(async () => {
      const formatted = await invoke<string>("format_codex_config_file", { fileName, content });
      if (fileName === "auth.json") setAuthJsonDraft(formatted);
      else setConfigTomlDraft(formatted);
      return formatted;
    }, t.configFormatted);
  }

  async function saveCodexConfig(fileName: "auth.json" | "config.toml") {
    const content = fileName === "auth.json" ? authJsonDraft : configTomlDraft;
    await run(async () => {
      const files = await invoke<CodexConfigFiles>("save_codex_config_file", {
        codexHome: codexHome || undefined,
        fileName,
        content
      });
      setCodexConfig(files);
      setCodexHome(files.codexHome);
      setAuthJsonDraft(files.authJson.content);
      setConfigTomlDraft(files.configToml.content);
      await refresh();
      return files;
    }, t.configSaved);
  }

  async function consumeUsageReset(profileId = selectedId) {
    const profile = store?.profiles.find((item) => item.id === profileId);
    if (!profile || !(await confirmDialog(t.useResetConfirm, { title: t.useReset, kind: "warning" }))) return;
    await run(async () => {
      const result = await invoke<{ message: string; outcome: string; availableResetCount?: number }>(
        "consume_usage_reset",
        { profileId: profile.id }
      );
      await refresh();
      setNotice({
        kind: "ok",
        text: `${t.usageResetDone}${result.availableResetCount != null ? ` · ${t.usageResets}: ${result.availableResetCount}` : ""}`
      });
      return result;
    });
  }

  async function deleteProfile(profileId = selectedId) {
    const profile = store?.profiles.find((item) => item.id === profileId);
    if (!profile) return;
    const isCurrent = currentGlobalProfileId === profile.id;
    const confirmed = await confirmDialog(
      `${isCurrent ? t.deleteCurrentAccountConfirm : t.deleteAccountConfirm}\n\n${profile.alias}`,
      { title: t.deleteAccount, kind: "warning" },
    );
    if (!confirmed) return;
    await run(async () => {
      const view = await invoke<StoreView>("delete_profile", {
        profileId: profile.id
      });
      setStore(view);
      if (selectedId === profile.id) {
        setSelectedId(view.settings.currentProfileId || view.profiles[0]?.id || "");
      }
      return view;
    }, t.deletedAccount);
  }

  async function exportBundle() {
    if (passwordTooShort) {
      setNotice({ kind: "warn", text: t.passwordTooShortExport });
      return;
    }
    const selectableProfileIds = (store?.profiles || []).map((profile) => profile.id);
    const profileIds = exportProfileIds.filter((id) => selectableProfileIds.includes(id));
    if (profileIds.length === 0) {
      setNotice({ kind: "warn", text: t.exportNoAccount });
      return;
    }
    const path = await save({
      title: t.exportTitle,
      defaultPath: password ? "codex-switcher.zip.enc" : "codex-switcher.zip",
      filters: [{ name: "Codex Switcher Bundle", extensions: ["zip", "enc"] }]
    });
    if (!path) return;
    await run(async () => {
      const manifest = await invoke<BundleManifest>("export_all_accounts_bundle", {
        outputPath: path,
        password,
        includeConversations,
        profileIds
      });
      const conversationCount = manifest.files.filter((file) => isConversationFile(file.path)).length;
      setNotice({
        kind: password ? "ok" : "warn",
        text: `${password ? t.encryptedExported : t.plaintextExported}: ${manifest.profileCount} ${t.accountCount}, ${manifest.files.length} ${t.configFiles}, ${t.conversationFiles} ${conversationCount}`
      });
      return manifest;
    });
  }

  async function saveRoutingLogSettings() {
    await run(async () => {
      const routing = await invoke<RoutingStatus>("routing_save_log_settings", {
        retentionDays: Math.min(365, Math.max(1, routingLogRetentionDays))
      });
      setRoutingStatus(routing);
      await refresh();
      return routing;
    }, t.savedRoutingLogSettings);
  }

  async function testRoutingRequest(): Promise<RoutingLogEntry | null> {
    setNotice(null);
    try {
      const result = await invoke<RoutingProbeResult>("routing_test_request");
      const routing = await reloadRoutingStatus();
      const log = routing.recentLogs
        .slice()
        .reverse()
        .find((entry) => entry.requestId === result.requestId) || null;
      setNotice({
        kind: result.ok ? "ok" : "error",
        text: `${result.message} · HTTP ${result.httpStatus} · ${result.elapsedMs} ms`
      });
      return log;
    } catch (error) {
      void logAppError("routing_test_request", error);
      setNotice({ kind: "error", text: String(error) });
      return null;
    }
  }

  function toggleExportProfile(profileId: string) {
    setExportProfileIds((current) => (
      current.includes(profileId)
        ? current.filter((id) => id !== profileId)
        : [...current, profileId]
    ));
  }

  async function importBundle() {
    if (passwordTooShort) {
      setNotice({ kind: "warn", text: t.passwordTooShortImport });
      return;
    }
    const path = await open({
      title: t.importTitle,
      multiple: false,
      directory: false,
      filters: [{ name: "Codex Switcher Bundle", extensions: ["zip", "enc"] }]
    });
    if (!path || Array.isArray(path)) return;
    await run(async () => {
      const manifest = await invoke<BundleManifest>("preview_bundle", {
        bundlePath: path,
        password
      });
      const result = await invoke<{ importedProfiles: number; restoredFiles: number; message: string }>(
        "import_accounts_bundle",
        {
          bundlePath: path,
          password,
          restoreConversations,
          codexHome: codexHome || undefined
        }
      );
      await refresh();
      setNotice({
        kind: "ok",
        text: `${t.importedAccounts} ${result.importedProfiles} ${t.accountCount}, ${t.restoredFiles} ${result.restoredFiles}; ${t.bundleAccountCount} ${manifest.profileCount}`
      });
      return result;
    });
  }

  async function restoreBackup() {
    await run(async () => {
      const message = await invoke<string>("restore_backup", {
        codexHome: codexHome || undefined,
        backupPath: undefined
      });
      return message;
    }, t.restoredBackup);
  }

  function renderProfileActions(profile: Profile, needsReauthorization: boolean) {
    return (
      <>
        <Button
          className="mini-button icon-only"
          onClick={(event) => {
            event.stopPropagation();
            openEditProfile(profile);
          }}
          disabled={busy}
          title={t.editAccountHint}
          aria-label={t.editAccount}
        >
          <Pencil size={14} />
        </Button>
        {(profile.usage.availableResetCount || 0) > 0 && (
          <Button
            className="mini-button icon-only"
            onClick={(event) => {
              event.stopPropagation();
              void consumeUsageReset(profile.id);
            }}
            disabled={busy}
            title={t.useResetHint}
            aria-label={t.useReset}
          >
            <RotateCcw size={14} />
          </Button>
        )}
        {needsReauthorization && (
          <Button
            className="mini-button icon-only"
            onClick={(event) => {
              event.stopPropagation();
              void reauthorizeProfile(profile);
            }}
            disabled={busy || oauthStatus === "starting" || oauthStatus === "exchanging"}
            title={t.reauthorizeHint}
            aria-label={t.reauthorize}
          >
            <KeyRound size={14} />
          </Button>
        )}
        <Button
          className="mini-button icon-only"
          onClick={(event) => {
            event.stopPropagation();
            void probeProfile(profile.id);
          }}
          disabled={busy}
          title={t.probeQuotaHint}
          aria-label={t.probeQuota}
        >
          <RefreshCcw size={14} />
        </Button>
        <Button
          className="mini-button primary icon-only"
          onClick={(event) => {
            event.stopPropagation();
            void switchProfile(profile.id);
          }}
          disabled={busy}
          title={profile.apiConfig && profile.apiConfig.wireApi !== "responses" ? t.pinRoutingHint : t.switchAccountHint}
          aria-label={profile.apiConfig && profile.apiConfig.wireApi !== "responses" ? "固定到路由" : t.switch}
        >
          {profile.apiConfig && profile.apiConfig.wireApi !== "responses" ? <Network size={14} /> : <Zap size={14} />}
        </Button>
        {store?.settings.routing.appliedToCodex && (
          <Button
            className="mini-button icon-only"
            onClick={(event) => {
              event.stopPropagation();
              void fixProfileToRouting(profile.id);
            }}
            disabled={busy}
            title={t.pinRoutingHint}
            aria-label={t.pinRoutingHint}
          >
            <Network size={14} />
          </Button>
        )}
        <Button
          className="mini-button danger icon-only"
          onClick={(event) => {
            event.stopPropagation();
            void deleteProfile(profile.id);
          }}
          disabled={busy}
          title={t.deleteAccountHint}
          aria-label={t.deleteAccount}
        >
          <Trash2 size={15} />
        </Button>
      </>
    );
  }

  if (initializing && !store) {
    return (
      <main className="app-shell startup-shell">
        <div className="startup-loading" role="status" aria-live="polite">
          <div className="startup-loading-mark"><RefreshCcw size={20} /></div>
          <strong>CodexSwitcher</strong>
          <span>正在加载配置…</span>
        </div>
      </main>
    );
  }

  return (
    <main className="app-shell">
      <AppNavigation page={activePage} onPageChange={setActivePage} language={languageSetting} onLanguageChange={setLanguageSetting} labels={{ language: t.language, followSystem: t.followSystem, dashboard: t.dashboard, settings: t.settings, refresh: t.refresh, refreshHint: t.refreshHint, subtitle: t.appSubtitle, mesh: t.multiDeviceSharing, routing: t.routingPage, page: t.pageLabel }} languageLabels={languageLabels} busy={busy} onRefresh={() => void refresh()} />

      {notice && (
        <div className={`notice ${notice.kind}`} role="status" aria-live="polite">
          <span>{notice.text}</span>
          <Button className="notice-close" onClick={() => setNotice(null)} title={t.closeNotice}>
            <X size={15} />
          </Button>
        </div>
      )}

      {showUpdateDialog && availableUpdate && (
        <Modal
          open
          onCancel={() => { if (!updateInstalling) setShowUpdateDialog(false); }}
          title={`${t.updateAvailable}: ${availableUpdate.version}`}
          width={560}
          footer={null}
          closable={!updateInstalling}
          maskClosable={!updateInstalling}
          className="update-dialog"
        >
            <div className="update-dialog-versions">
              <div><span>{t.currentVersion}</span><strong>{appVersion || "-"}</strong></div>
              <div><span>{t.updateAvailable}</span><strong>{availableUpdate.version}</strong></div>
            </div>
            {availableUpdate.date && <small>{t.updateDate}: {formatDate(availableUpdate.date)}</small>}
            {availableUpdate.body && <div className="update-dialog-notes">{availableUpdate.body}</div>}
            {updateInstalling && (
              <div className="update-progress" aria-label={t.installingUpdate}>
                <div><span style={{ width: `${updateProgressPercent ?? 35}%` }} /></div>
                <strong>{updateProgressPercent !== undefined ? `${t.installingUpdate} ${updateProgressPercent}%` : t.installingUpdate}</strong>
              </div>
            )}
            <div className="update-dialog-actions">
              {!updateInstalled && (
                <Button className="icon-button" onClick={() => setShowUpdateDialog(false)} disabled={updateInstalling} title={t.cancelUpdateDialogHint}>
                  {t.closeNotice}
                </Button>
              )}
              {!updateInstalled ? (
                <Button className="icon-button primary" onClick={() => void installAvailableUpdate()} disabled={updateInstalling} title={t.installUpdateHint}>
                  <Download size={17} /> {t.downloadAndInstall}
                </Button>
              ) : (
                <Button className="icon-button primary" onClick={() => void relaunch()} title={t.restartUpdateHint}>
                  <RotateCcw size={17} /> {t.relaunchNow}
                </Button>
              )}
            </div>
        </Modal>
      )}

      {showAddAccountDialog && (
        <Modal title={t.addAccount} open={showAddAccountDialog} onCancel={closeAddAccountDialog} footer={null} width={760} closable={!busy && oauthStatus !== "exchanging"} maskClosable={!busy && oauthStatus !== "exchanging"}>
            <Tabs activeKey={addAccountTab} onChange={(key) => selectAddAccountTab(key as typeof addAccountTab)} items={([
              ["oauth", t.oauthLogin], ["json", t.tokenJson], ["api", t.apiKeyAccount], ["import", t.importAccount]
            ] as const).map(([key, label]) => ({ key, label, disabled: oauthStatus === "exchanging" }))} />
            {addAccountTab !== "api" && (
              <label className="add-account-field">
                {t.importAlias}
                <Input value={alias} onChange={(event) => setAlias(event.target.value)} placeholder={t.importAlias} />
              </label>
            )}
            {addAccountTab === "oauth" && (
              <div className="add-account-content">
                <p>{t.oauthLoginHint}</p>
                <div className={`oauth-session-status ${oauthStatus}`}>
                  <span>
                    {oauthStatus === "starting" ? t.oauthStarting
                      : oauthStatus === "exchanging" ? t.oauthExchanging
                      : oauthStatus === "timeout" ? t.oauthTimedOut
                      : oauthSession ? t.oauthWaiting
                      : t.startOAuthLogin}
                  </span>
                  {oauthSession && oauthRemainingSeconds > 0 && <strong>{oauthRemainingSeconds} {t.secondsRemaining}</strong>}
                </div>
                {oauthSession ? (
                  <>
                    <label className="add-account-field">
                      {t.authorizationUrl}
                      <div className="oauth-url-row">
                        <Input value={oauthSession.authUrl} readOnly />
                        <Button onClick={() => void copyOAuthUrl()} title={t.copyOAuthUrlHint}>{t.copyLink}</Button>
                        <Button onClick={() => void reopenOAuthUrl()} title={t.reopenOAuthUrlHint}>{t.openAgain}</Button>
                      </div>
                    </label>
                    <div className="oauth-callback-row">
                      <Input value={oauthCallbackInput} onChange={(event) => setOauthCallbackInput(event.target.value)} placeholder={t.manualCallback} disabled={oauthStatus === "exchanging"} />
                      <Button className="icon-button" onClick={() => void submitOAuthCallback()} disabled={!oauthCallbackInput.trim() || oauthStatus === "exchanging"} title={t.submitOAuthCallbackHint}>{t.submitCallback}</Button>
                    </div>
                    {oauthError && <div className="oauth-error">{oauthError}</div>}
                    <div className="oauth-action-row">
                      {oauthStatus === "error" && <Button className="icon-button primary" onClick={() => void completeNativeOAuth(oauthSession.loginId)} title={t.retryExchangeHint}>{t.retryExchange}</Button>}
                      <Button className="icon-button" onClick={() => void cancelNativeOAuth()} disabled={oauthStatus === "exchanging"} title={t.cancelOAuthHint}>{t.cancelOAuth}</Button>
                    </div>
                  </>
                ) : (
                  <>
                    {oauthError && <div className="oauth-error">{oauthError}</div>}
                    <Button className="icon-button primary wide-button" onClick={() => {
                      setOauthReauthProfileId(null);
                      void beginNativeOAuth();
                    }} disabled={oauthStatus === "starting" || oauthStatus === "exchanging"} title={t.startOAuthHint}>
                      <KeyRound size={17} /> {t.startOAuthLogin}
                    </Button>
                  </>
                )}
                <Collapse className="oauth-cli-fallback" items={[{ key: "cli", label: t.cliFallback, children: <>
                  <p>{t.cliFallbackHint}</p>
                  <Button className="icon-button wide-button" onClick={() => void startCliOAuthLogin()} disabled={busy} title={t.startCliLoginHint}>
                    <KeyRound size={17} /> {t.startCliLogin}
                  </Button>
                  <Button className="icon-button wide-button" onClick={() => void importCurrentAuth()} disabled={busy} title={t.oauthImportDoneHint}>
                    <CheckCircle2 size={17} /> {t.oauthImportDone}
                  </Button>
                </> }]} />
              </div>
            )}
            {addAccountTab === "json" && (
              <div className="add-account-content">
                <Input.TextArea className="auth-json-input" spellCheck={false} value={authJsonInput} onChange={(event) => setAuthJsonInput(event.target.value)} placeholder={t.authJsonPlaceholder} rows={9} />
                <Button className="icon-button primary wide-button" onClick={() => void addAuthJsonAccount()} disabled={busy || !authJsonInput.trim()} title={t.addJsonAccountHint}>
                  <FileText size={17} /> {t.addFromJson}
                </Button>
              </div>
            )}
            {addAccountTab === "api" && (
              <div className="api-provider-form add-account-content">
                <OpenCodeZenPreset
                  disabled={busy}
                  t={t}
                  onApply={(preset) => {
                    setApiProviderName(preset.providerName);
                    setApiProviderId("");
                    setApiBaseUrl(preset.baseUrl);
                    setApiModel(preset.model);
                    setApiWireApi(preset.wireApi);
                    setApiKey(preset.apiKey);
                  }}
                />
                <label>{t.apiModel}<Input value={apiModel} onChange={(event) => setApiModel(event.target.value)} placeholder="gpt-5.4" /></label>
                <label>{t.apiKey}<Input.Password value={apiKey} onChange={(event) => setApiKey(event.target.value)} /></label>
                <label>{t.importAlias}<Input value={apiProviderName} onChange={(event) => setApiProviderName(event.target.value)} placeholder="可空，默认使用模型名" /></label>
                <Collapse className="api-advanced-settings" items={[{ key: "advanced", label: "高级设置（可选）", children: <div className="api-advanced-grid">
                    <label>{t.providerId}<Input value={apiProviderId} onChange={(event) => setApiProviderId(event.target.value)} placeholder="自动生成" /></label>
                    <label>{t.apiBaseUrl}<Input value={apiBaseUrl} onChange={(event) => setApiBaseUrl(event.target.value)} placeholder="默认 https://api.openai.com/v1" /></label>
                    <label>
                      {t.apiProtocol}
                      <Select value={apiWireApi} onChange={setApiWireApi} options={[
                        { value: "responses", label: t.apiProtocolResponses }, { value: "chat_completions", label: t.apiProtocolChat }, { value: "anthropic_messages", label: t.apiProtocolAnthropic },
                      ]} />
                    </label>
                  </div>
                }]} />
                <Button className="icon-button primary wide-button" onClick={() => void addApiProvider()} disabled={busy || !apiModel.trim() || !apiKey.trim()} title={t.addApiProviderHint}>
                  <KeyRound size={17} /> {t.addApiProvider}
                </Button>
              </div>
            )}
            {addAccountTab === "import" && (
              <div className="add-account-content">
                <p>{t.codexConfigHint}</p>
                <Button className="icon-button primary wide-button" onClick={() => void importCurrentAuth()} disabled={busy} title={t.importCurrentHint}>
                  <Download size={17} /> {t.importCurrent}
                </Button>
              </div>
            )}
        </Modal>
      )}

      {editingProfile && (
        <Modal title={<>{t.editAccountInfo} <small>{editingProfile.apiConfig ? t.apiKeyAccount : t.oauthLogin}</small></>} open onCancel={closeEditProfile} footer={null} width={760} closable={!busy}>
            {editingProfile.apiConfig ? (
              <div className="api-provider-form edit-api-provider-form add-account-content">
                <label>
                  {t.accountAlias}
                  <Input value={editAliasDraft} onChange={(event) => setEditAliasDraft(event.target.value)} placeholder={t.accountAlias} />
                </label>
                <label>
                  {t.openCodeZenFreeModel}
                  <Select
                    value={OPENCODE_ZEN_FREE_MODELS.some((item) => item.id === editModelDraft) ? editModelDraft : ""}
                    onChange={(model) => {
                      if (!model) return;
                      setEditModelDraft(model);
                      setEditBaseUrlDraft(OPENCODE_ZEN_BASE_URL);
                      setEditWireApiDraft("chat_completions");
                    }}
                    options={[{ value: "", label: "手动输入模型…" }, ...OPENCODE_ZEN_FREE_MODELS.map((item) => ({ value: item.id, label: `${item.name} (${item.id})` }))]}
                  />
                </label>
                <label>
                  {t.apiModel}
                  <Input value={editModelDraft} onChange={(event) => setEditModelDraft(event.target.value)} placeholder="gpt-5.4" />
                </label>
                <label>
                  {t.apiKeyOptional}
                  <Input.Password value={editApiKeyDraft} onChange={(event) => setEditApiKeyDraft(event.target.value)} />
                </label>
                <Collapse className="api-advanced-settings form-span-all" items={[{ key: "advanced", label: "高级设置（可选）", children: <div className="api-advanced-grid">
                    <label>
                      {t.providerId}
                      <Input value={editProviderIdDraft} onChange={(event) => setEditProviderIdDraft(event.target.value)} placeholder="自动生成" />
                    </label>
                    <label>
                      {t.apiBaseUrl}
                      <Input value={editBaseUrlDraft} onChange={(event) => setEditBaseUrlDraft(event.target.value)} placeholder="默认 https://api.openai.com/v1" />
                    </label>
                    <label>
                      {t.apiProtocol}
                      <Select value={editWireApiDraft} onChange={setEditWireApiDraft} options={[
                        { value: "responses", label: t.apiProtocolResponses },
                        { value: "chat_completions", label: t.apiProtocolChat },
                        { value: "anthropic_messages", label: t.apiProtocolAnthropic },
                      ]} />
                    </label>
                  </div> }]} />
                <label className="form-span-all">
                  {t.accountNote}
                  <Input.TextArea
                    className="profile-note-input compact"
                    value={editNoteDraft}
                    onChange={(event) => setEditNoteDraft(event.target.value)}
                    placeholder={t.notePlaceholder}
                    rows={4}
                  />
                </label>
                <ProfileRuleFields
                  t={t}
                  quota={editQuotaDraft}
                  priority={editPriorityDraft}
                  enabled={editEnabledDraft}
                  onQuotaChange={setEditQuotaDraft}
                  onPriorityChange={setEditPriorityDraft}
                  onEnabledChange={setEditEnabledDraft}
                />
                <div className="edit-form-actions">
                  <Button onClick={closeEditProfile} disabled={busy} title={t.closeDialogHint}>{t.closeNotice}</Button>
                  <Button
                    className="icon-button primary"
                    onClick={() => void saveProfileDetails()}
                    disabled={busy || !editAliasDraft.trim() || !editModelDraft.trim()}
                    title={t.saveProfileHint}
                  >
                    <ShieldCheck size={17} /> {t.saveRules}
                  </Button>
                </div>
              </div>
            ) : (
              <div className="edit-oauth-form add-account-content">
                <label className="add-account-field">
                  {t.accountAlias}
                  <Input value={editAliasDraft} onChange={(event) => setEditAliasDraft(event.target.value)} />
                </label>
                <label className="add-account-field">
                  {t.accountNote}
                  <Input.TextArea
                    className="profile-note-input"
                    value={editNoteDraft}
                    onChange={(event) => setEditNoteDraft(event.target.value)}
                    placeholder={t.notePlaceholder}
                    rows={4}
                  />
                </label>
                <ProfileRuleFields
                  t={t}
                  quota={editQuotaDraft}
                  priority={editPriorityDraft}
                  enabled={editEnabledDraft}
                  onQuotaChange={setEditQuotaDraft}
                  onPriorityChange={setEditPriorityDraft}
                  onEnabledChange={setEditEnabledDraft}
                />
                <div className="edit-form-actions">
                  <Button onClick={closeEditProfile} disabled={busy} title={t.closeDialogHint}>{t.closeNotice}</Button>
                  <Button
                    className="icon-button primary"
                    onClick={() => void saveProfileDetails()}
                    disabled={busy || !editAliasDraft.trim()}
                    title={t.saveProfileHint}
                  >
                    <ShieldCheck size={17} /> {t.saveRules}
                  </Button>
                </div>
              </div>
            )}
        </Modal>
      )}

      {activePage === "routing" ? (
        <RoutingPage
          t={t}
          busy={busy}
          profiles={store?.profiles || []}
          appliedToCodex={!!store?.settings.routing.appliedToCodex}
          status={routingStatus}
          host={routingHost}
          port={routingPort}
          mode={routingMode}
          fixedProfileId={routingFixedProfileId}
          stickyTtlSecs={routingStickyTtlSecs}
          routingBusy={routingBusy}
          onHostChange={setRoutingHost}
          onPortChange={setRoutingPort}
          onModeChange={setRoutingMode}
          onFixedProfileIdChange={setRoutingFixedProfileId}
          onStickyTtlSecsChange={setRoutingStickyTtlSecs}
          onToggleService={toggleRoutingService}
          onSaveSettings={() => saveRoutingSettings()}
          onReloadStatus={reloadRoutingStatus}
          onCopyConfig={copyRoutingConfig}
          onRegenerateKey={regenerateRoutingKey}
          onApplyCodexConfig={applyRoutingCodexConfig}
          onRestoreCodexConfig={restoreRoutingCodexConfig}
          onFixProfile={fixProfileToRouting}
          onSaveProfilePriority={saveRoutingProfilePriority}
          onTestRequest={testRoutingRequest}
        />
      ) : activePage === "mesh" ? (
        <MeshSharePage
          status={meshStatus}
          groups={meshGroups}
          selectedGroupId={meshSelectedGroupId}
          groupStatus={meshGroupStatus}
          profiles={store?.profiles || []}
          busy={busy}
          sharePayload={meshSharePayload}
          importPayload={meshImportPayload}
          networkName={meshNetworkName}
          networkSecret={meshNetworkSecret}
          nodeSourceUrl={meshNodeSourceUrl}
          nodeRefreshSecs={meshNodeRefreshSecs}
          autoStart={meshAutoStart}
          syncScope={meshSyncScope}
          migrationPassword={password}
          migrationUseMeshSecret={meshMigrationUseSecret}
          includeConversations={includeConversations}
          restoreConversations={restoreConversations}
          exportProfileIds={exportProfileIds}
          onImportPayloadChange={setMeshImportPayload}
          onNetworkNameChange={setMeshNetworkName}
          onNetworkSecretChange={setMeshNetworkSecret}
          onNodeSourceUrlChange={setMeshNodeSourceUrl}
          onNodeRefreshSecsChange={setMeshNodeRefreshSecs}
          onAutoStartChange={setMeshAutoStart}
          onSyncScopeChange={setMeshSyncScope}
          onMigrationPasswordChange={setPassword}
          onMigrationUseMeshSecretChange={setMeshMigrationUseSecret}
          onIncludeConversationsChange={setIncludeConversations}
          onRestoreConversationsChange={setRestoreConversations}
          onToggleExportProfile={toggleExportProfile}
          onSelectAllProfiles={() => setExportProfileIds((store?.profiles || []).map((profile) => profile.id))}
          onClearProfiles={() => setExportProfileIds([])}
          onSaveSettings={saveMeshSettings}
          onToggleService={toggleMeshService}
          onRefreshNodes={refreshMeshNodes}
          onCreateShare={createMeshSharePayload}
          onCopyShare={copyMeshSharePayload}
          onImportShare={importMeshSharePayload}
          onSaveDevice={saveMeshDevice}
          onSyncNow={syncMeshNow}
          onAuthorizePeer={authorizeMeshPeer}
          onExportMigration={exportMeshMigrationShare}
          onImportMigration={importMeshMigrationShare}
          onCreateGroup={createMeshGroup}
          onImportGroup={importMeshGroup}
          onSelectGroup={selectMeshGroup}
          onStartGroup={({ groupId }) => setMeshGroupRunning(groupId, true)}
          onStopGroup={({ groupId }) => setMeshGroupRunning(groupId, false)}
          onDeleteGroup={({ groupId }) => deleteMeshGroup(groupId)}
          onRevokeDevice={revokeMeshGroupDevice}
          onRemoveDevice={removeMeshGroupDevice}
          onSaveGroupDevice={saveMeshGroupDevice}
          onSyncGroup={syncMeshGroup}
          onCreateGroupShare={createMeshGroupShare}
          onSaveGroupScope={saveMeshGroupScope}
          onGroupFilesChanged={setMeshGroupStatus}
        />
      ) : activePage === "settings" ? (
        <>
      <Card className="settings-band toolbar-band">
        <div className="path-control">
          <label>{t.codexDir}</label>
          <Input value={codexHome} onChange={(event) => setCodexHome(event.target.value)} />
          <Button icon={<FileSearch size={17} />} onClick={() => void scanHome()} disabled={busy} title={t.scanCodexHomeHint}>{t.scan}</Button>
          <Button icon={<FolderOpen size={17} />} onClick={() => void openCodexHome()} disabled={busy} title={t.openCodexHomeHint}>{t.openDir}</Button>
        </div>
        <div className="scan-state">
          <StatusPill ok={!!scan?.exists} text={scan?.exists ? t.dirExists : t.notScanned} />
          <StatusPill ok={!!scan?.hasAuth} text={scan?.hasAuth ? t.authFound : t.authMissing} />
        </div>
      </Card>

      <Card className="settings-band proxy-band">
        <label className="checkline" title={t.proxyEnabled}>
          <Switch
            checked={proxyEnabled}
            onChange={(checked) => {
              proxyDraftDirtyRef.current = true;
              setProxyEnabled(checked);
            }}
          />
          {t.proxyEnabled}
        </label>
        <Input
          className="proxy-input"
          placeholder={t.proxyPlaceholder}
          value={proxyUrl}
          onChange={(event) => {
            proxyDraftDirtyRef.current = true;
            setProxyUrl(event.target.value);
          }}
        />
        <Button icon={<ShieldCheck size={17} />} onClick={() => void saveProxySettings()} disabled={busy} title={t.saveProxyHint}>{t.saveProxy}</Button>
        <Button icon={<Wifi size={17} />} onClick={() => void testProxySettings()} disabled={busy} title={t.testProxyHint}>{t.testProxy}</Button>
        <span className="proxy-hint">{t.proxyHint}</span>
      </Card>

      <Card className="settings-band auto-band">
        <label className="checkline" title={t.backgroundTokenKeepalive}>
          <Switch
            checked={backgroundTokenRefreshEnabled}
            onChange={(checked) => {
              autoSettingsDirtyRef.current = true;
              setBackgroundTokenRefreshEnabled(checked);
            }}
          />
          {t.backgroundTokenKeepalive}
        </label>
        <label>
          {t.keepaliveInterval}
          <InputNumber
            min={3600}
            value={backgroundTokenRefreshIntervalSecs}
            onChange={(value) => {
              autoSettingsDirtyRef.current = true;
              setBackgroundTokenRefreshIntervalSecs(value || 3600);
            }}
            title={t.keepaliveInterval}
            style={{ width: 100 }}
          />
        </label>
        <label>
          {t.refreshThreshold}
          <InputNumber
            min={0}
            value={tokenRefreshThresholdSecs}
            onChange={(value) => {
              autoSettingsDirtyRef.current = true;
              setTokenRefreshThresholdSecs(value || 0);
            }}
            title={t.refreshThreshold}
            style={{ width: 100 }}
          />
        </label>
        <label className="checkline" title={t.autoProbeQuota}>
          <Switch
            checked={autoProbeEnabled}
            onChange={(checked) => {
              autoSettingsDirtyRef.current = true;
              setAutoProbeEnabled(checked);
            }}
          />
          {t.autoProbeQuota}
        </label>
        <label>
          {t.probeInterval}
          <InputNumber
            min={30}
            value={autoProbeIntervalSecs}
            onChange={(value) => {
              autoSettingsDirtyRef.current = true;
              setAutoProbeIntervalSecs(value || 60);
            }}
            title={t.probeInterval}
            style={{ width: 100 }}
          />
        </label>
        <Button icon={<RefreshCcw size={17} />} onClick={() => void saveAutoSettings()} disabled={busy || autoProbeRunning} title={t.saveAutoSettingsHint}>{autoProbeRunning ? "探测中..." : t.saveAutoRefresh}</Button>
        <Button icon={<KeyRound size={17} />} onClick={() => void refreshOtherProfileTokensNow()} disabled={busy} title={t.keepaliveNowHint}>{t.keepaliveNow}</Button>
        <span className="proxy-hint">{t.autoHint}</span>
      </Card>

      <RoutingLogSettings
        t={t}
        retentionDays={routingLogRetentionDays}
        busy={busy}
        onRetentionDaysChange={setRoutingLogRetentionDays}
        onSave={saveRoutingLogSettings}
        onOpenLogs={openLogsDirectory}
      />

      <Card className="settings-band update-band">
        <div className="update-copy">
          <h2>{t.softwareUpdate}</h2>
          <p>{t.updateHint}</p>
        </div>
        <div className="update-status">
          <span>{t.currentVersion}</span>
          <strong>{appVersion || "-"}</strong>
        </div>
        <div className="update-status">
          <span>{updateError ? t.updateCheckFailed : availableUpdate ? t.updateAvailable : updateChecked ? t.upToDate : t.updateNotChecked}</span>
          <strong>{availableUpdate?.version || "-"}</strong>
        </div>
        {updateError && <div className="update-error">{updateError}</div>}
        {availableUpdate?.date && (
          <div className="update-status">
            <span>{t.updateDate}</span>
            <strong>{formatDate(availableUpdate.date)}</strong>
          </div>
        )}
        {availableUpdate?.body && <div className="update-notes">{availableUpdate.body}</div>}
        {updateInstalling && (
          <div className="update-progress" aria-label={t.installingUpdate}>
            <div>
              <span style={{ width: `${updateProgressPercent ?? 35}%` }} />
            </div>
            <strong>
              {updateProgressPercent !== undefined
                ? `${t.installingUpdate} ${updateProgressPercent}%`
                : t.installingUpdate}
            </strong>
          </div>
        )}
        <div className="action-row">
          <Button className="icon-button" onClick={() => void checkForUpdate(true)} disabled={updateChecking || updateInstalling} title={t.checkUpdateHint}>
            <RefreshCcw size={17} />
            {updateChecking ? t.checkUpdate : t.checkUpdate}
          </Button>
          <Button
            className="icon-button primary"
            onClick={() => void installAvailableUpdate()}
            disabled={!availableUpdate || updateInstalling || updateInstalled}
            title={t.installUpdateHint}
          >
            <Download size={17} />
            {t.downloadAndInstall}
          </Button>
          <Button className="icon-button" onClick={() => void relaunch()} disabled={!updateInstalled} title={t.restartUpdateHint}>
            <RotateCcw size={17} />
            {t.relaunchNow}
          </Button>
        </div>
      </Card>
        </>
      ) : (
        <>

      <section className="main-grid account-only-grid">
        <Card className="panel account-panel">
          <div className="panel-header account-panel-header">
            <div>
              <h2>{t.accounts}</h2>
              <p>{filteredProfiles.length}/{store?.profiles.length || 0} {t.profiles}</p>
            </div>
            <div className="compact-actions account-toolbar">
              <label className="checkline compact-checkline" title={t.forceSwitch}>
                <Checkbox checked={forceSwitch} onChange={(event) => setForceSwitch(event.target.checked)} />
                {t.forceSwitch}
              </label>
              <Select value={accountExpiryFilter} onChange={(value) => setAccountExpiryFilter(value as AccountExpiryFilter)} title={t.expiryFilter} aria-label={t.expiryFilter} options={[
                { value: "all", label: t.allExpiry }, { value: "valid", label: t.validOnly }, { value: "expired", label: t.expiredOnly },
              ]} />
              <Select value={accountStatusFilter} onChange={(value) => setAccountStatusFilter(value as AccountStatusFilter)} title={t.loginStatusFilter} aria-label={t.loginStatusFilter} options={[
                { value: "all", label: t.allLoginStatuses }, { value: "available", label: t.available }, { value: "relogin", label: t.reloginRequired }, { value: "disabled", label: t.disabled }, { value: "cooling", label: t.cooling }, { value: "expired", label: t.expired }, { value: "error", label: t.probeFailed },
              ]} />
              <div className="view-toggle" role="group" aria-label={t.accountViewMode}>
                <Button
                  className={accountViewMode === "cards" ? "active" : ""}
                  onClick={() => setAccountViewMode("cards")}
                  title={t.cardView}
                  aria-label={t.cardView}
                >
                  <Grid2X2 size={15} /> <span className="view-toggle-label">{t.cardView}</span>
                </Button>
                <Button
                  className={accountViewMode === "rows" ? "active" : ""}
                  onClick={() => setAccountViewMode("rows")}
                  title={t.rowView}
                  aria-label={t.rowView}
                >
                  <Rows3 size={15} /> <span className="view-toggle-label">{t.rowView}</span>
                </Button>
              </div>
              <div className="account-search-row">
                <Input
                  placeholder={t.searchPlaceholder}
                  value={accountFilter}
                  onChange={(event) => setAccountFilter(event.target.value)}
                  title={t.searchPlaceholder}
                />
                <Button type="primary" icon={<KeyRound size={17} />} onClick={() => setShowAddAccountDialog(true)} disabled={busy} title={t.addAccountDialogHint}>{t.addAccount}</Button>
              </div>
            </div>
          </div>

          <Collapse className="codex-config-panel" items={[{ key: "config", label: t.codexConfig, children: <>
            <p>{t.codexConfigHint}</p>
            <div className="config-toolbar">
              <Button className="icon-button" onClick={() => void loadCodexConfigFiles()} disabled={busy} title={t.loadConfigHint}>
                <FileSearch size={16} /> {t.loadConfig}
              </Button>
              <span>{codexConfig?.codexHome || codexHome || "~/.codex"}</span>
            </div>
            <div className="config-editor-grid">
              <section className="config-editor-card">
                <div className="config-editor-head">
                  <div>
                    <strong>{t.authJsonConfig}</strong>
                    <small>{codexConfig?.authJson.exists === false ? t.configMissing : codexConfig?.authJson.path || "auth.json"}</small>
                  </div>
                  <div className="row-actions">
                    <Button size="small" onClick={() => void formatCodexConfig("auth.json")} disabled={busy || !authJsonDraft.trim()} title={t.formatAuthJsonHint}>
                      {t.formatConfig}
                    </Button>
                    <Button size="small" type="primary" onClick={() => void saveCodexConfig("auth.json")} disabled={busy || !authJsonDraft.trim()} title={t.saveAuthJsonHint}>
                      {t.saveConfig}
                    </Button>
                  </div>
                </div>
                <Input.TextArea
                  className="config-editor"
                  rows={16}
                  spellCheck={false}
                  value={authJsonDraft}
                  onChange={(event) => setAuthJsonDraft(event.target.value)}
                  placeholder={'{\n  "tokens": {}\n}'}
                />
              </section>
              <section className="config-editor-card">
                <div className="config-editor-head">
                  <div>
                    <strong>{t.configTomlConfig}</strong>
                    <small>{codexConfig?.configToml.exists === false ? t.configMissing : codexConfig?.configToml.path || "config.toml"}</small>
                  </div>
                  <div className="row-actions">
                    <Button size="small" onClick={() => void formatCodexConfig("config.toml")} disabled={busy} title={t.formatTomlHint}>
                      {t.formatConfig}
                    </Button>
                    <Button size="small" type="primary" onClick={() => void saveCodexConfig("config.toml")} disabled={busy} title={t.saveTomlHint}>
                      {t.saveConfig}
                    </Button>
                  </div>
                </div>
                <Input.TextArea
                  className="config-editor"
                  rows={16}
                  spellCheck={false}
                  value={configTomlDraft}
                  onChange={(event) => setConfigTomlDraft(event.target.value)}
                  placeholder={'model = "gpt-5"\nmodel_provider = "openai"'}
                />
              </section>
            </div>
          </> }]} />

          {accountViewMode === "cards" ? (
          <div className="account-card-grid">
            {pagedProfiles.map((profile) => {
              const isCurrent = currentGlobalProfileId === profile.id;
              const limits = profile.usage.detectedLimits || [];
              const accountText = profile.summary.email || profile.summary.accountId || profile.apiConfig?.baseUrl || t.unknownAccount;
              const canCopyAccount = accountText !== t.unknownAccount;
              const needsReauthorization = profileNeedsReauthorization(profile);
              const usageError = profileUsageError(profile);
              return (
                <article
                  key={profile.id}
                  className={`account-card ${selectedId === profile.id ? "selected" : ""} ${isCurrent ? "current" : ""}`}
                  onClick={() => setSelectedId(profile.id)}
                >
                  <div className="account-card-head">
                    <div className="account-card-title">
                      <strong>{profile.alias}</strong>
                      <div className="account-identity-row">
                        <small>{accountText}</small>
                        {canCopyAccount && (
                          <Button
                            className="mini-button icon-only account-copy-button"
                            onClick={(event) => {
                              event.stopPropagation();
                              void copyProfileAccount(profile);
                            }}
                            title={t.copyAccountHint}
                            aria-label={t.copyAccountHint}
                          >
                            <Copy size={13} />
                          </Button>
                        )}
                      </div>
                      {profile.note && <small className="account-note">{profile.note}</small>}
                    </div>
                    <div className="account-card-badges">
                      {isCurrent && <em className="current-badge">{t.currentUsing}</em>}
                      <span className="plan-badge">{planBadge(profile, t)}</span>
                    </div>
                  </div>

                  {needsReauthorization && (
                    <div className="account-relogin-alert">
                      <KeyRound size={14} />
                      <span>{usageError || t.reloginRequiredHint}</span>
                    </div>
                  )}
                  {usageError && !needsReauthorization && (
                    <div className="account-usage-error" title={usageError}>{usageError}</div>
                  )}

                  <div className="account-card-meta">
                    <StatusPill ok={profile.enabled && !isCooling(profile)} text={accountState(profile, t)} />
                    <span>{t.token}: {tokenState(profile, t)}</span>
                    {profile.usage.availableResetCount != null && (
                      <span title={formatResetExpiry(profile, t)}>
                        {t.usageResets}: {profile.usage.availableResetCount}
                        {(profile.usage.availableResetCount || 0) > 0 && ` · ${formatResetExpiry(profile, t)}`}
                      </span>
                    )}
                  </div>

                  {profile.apiConfig && (
                    <div className="api-provider-summary">
                      <strong>{profile.apiConfig.model}</strong>
                      <small>{profile.apiConfig.baseUrl}</small>
                      <span>{apiProtocolLabel(profile.apiConfig.wireApi, t)}</span>
                    </div>
                  )}

                  {!profile.apiConfig && !usageError && <div className="account-limit-list">
                    {limits.map((item, index) => {
                      const remainingPercent = limitRemainingPercent(item);
                      return (
                        <div className="account-limit" key={`${item.window}-${index}`}>
                          <div className="account-limit-head">
                            <span title={localizedLimitLabel(item.label || item.window, t)}>{localizedLimitLabel(item.label || item.window, t)}</span>
                            <strong>{remainingPercent != null ? `${remainingPercent}%` : formatUsage(item.used, item.limit, t)}</strong>
                          </div>
                          {remainingPercent != null && (
                            <div className="account-limit-track">
                              <i style={{ width: `${remainingPercent}%` }} />
                            </div>
                          )}
                          <small>{item.resetAt ? formatResetDateTime(item.resetAt) : t.notProbed}</small>
                        </div>
                      );
                    })}
                    {limits.length === 0 && <div className="account-limit-empty">{t.noParsedQuota}</div>}
                  </div>}

                  {!profile.apiConfig && <div className={`account-validity ${subscriptionExpiryState(profile).expired ? "expired" : ""}`}>
                    <span>{t.loginValidity}</span>
                    <strong>{formatSubscriptionValidity(profile, t)}</strong>
                    <small>{formatDate(profile.summary.subscriptionActiveUntil)}</small>
                  </div>}

                  <div className="account-card-foot">
                    <small>{profile.usage.lastProbeAt ? `${t.probe}: ${formatReset(profile.usage.lastProbeAt)}` : t.notProbed}</small>
                    <span className="row-actions">
                      {renderProfileActions(profile, needsReauthorization)}
                    </span>
                  </div>
                </article>
              );
            })}
            {filteredProfiles.length === 0 && (
              <div className="account-empty">{t.noMatchedAccounts}</div>
            )}
          </div>
          ) : (
          <div className="account-table account-row-table">
            <div className="account-row header">
              <span>{t.account}</span>
              <span>{t.loginValidity}</span>
              <span>{t.state}</span>
              <span>{t.token}</span>
              <span>{t.quota}</span>
              <span>{t.probe}</span>
              <span>{t.actions}</span>
            </div>
            {pagedProfiles.map((profile) => {
              const isCurrent = currentGlobalProfileId === profile.id;
              const accountText = profile.summary.email || profile.summary.accountId || profile.apiConfig?.baseUrl || t.unknownAccount;
              const needsReauthorization = profileNeedsReauthorization(profile);
              const usageError = profileUsageError(profile);
              return (
                <article
                  className={`account-row ${selectedId === profile.id ? "selected" : ""} ${isCurrent ? "current" : ""}`}
                  key={profile.id}
                  onClick={() => setSelectedId(profile.id)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      setSelectedId(profile.id);
                    }
                  }}
                  role="button"
                  tabIndex={0}
                >
                  <span>
                    <strong>
                      {profile.alias}
                      {isCurrent && <em className="current-badge">{t.currentUsing}</em>}
                    </strong>
                    <small>{accountText}</small>
                  </span>
                  <span className={isAccountExpired(profile, t) ? "expired-cell" : ""}>
                    {profile.apiConfig ? profile.apiConfig.model : formatSubscriptionValidity(profile, t)}
                  </span>
                  <span><StatusPill ok={profile.enabled && !isCooling(profile)} text={accountState(profile, t)} /></span>
                  <span>{tokenState(profile, t)}</span>
                  <span className={`quota-cell ${usageError ? "account-usage-error" : ""}`} title={usageError || undefined}>{usageError || quotaSummary(profile, t)}</span>
                  <span>{profile.usage.lastProbeAt ? formatReset(profile.usage.lastProbeAt) : t.notProbed}</span>
                  <span className="row-actions">{renderProfileActions(profile, needsReauthorization)}</span>
                </article>
              );
            })}
            {filteredProfiles.length === 0 && (
              <div className="account-empty">{t.noMatchedAccounts}</div>
            )}
          </div>
          )}

          {filteredProfiles.length > 0 && (
            <div className="account-pagination">
              <span>
                {t.pageSummary
                  .replace("{page}", String(accountPage))
                  .replace("{pages}", String(totalAccountPages))
                  .replace("{count}", String(filteredProfiles.length))}
              </span>
              <div className="row-actions">
                <Button
                  className="mini-button"
                  onClick={() => setAccountPage((page) => Math.max(1, page - 1))}
                  disabled={accountPage <= 1}
                  title={t.previousPageHint}
                  aria-label={t.previousPage}
                >
                  {t.previousPage}
                </Button>
                <Button
                  className="mini-button"
                  onClick={() => setAccountPage((page) => Math.min(totalAccountPages, page + 1))}
                  disabled={accountPage >= totalAccountPages}
                  title={t.nextPageHint}
                  aria-label={t.nextPage}
                >
                  {t.nextPage}
                </Button>
              </div>
            </div>
          )}

        </Card>
      </section>

      <Card className="settings-band migration-band">
        <div className="migration-copy">
          <h2>多设备共享与迁移</h2>
          <p>迁移包导出/导入、连接分享码和设备同步已统一放到多设备共享页。</p>
        </div>
        <div className="migration-controls">
          <Button className="icon-button primary" onClick={() => setActivePage("mesh")} disabled={busy} title={t.openMeshHint}>
            <Share2 size={17} />
            打开多设备共享
          </Button>
          <Button className="icon-button" onClick={() => void restoreBackup()} disabled={busy} title={t.restoreBackupHint}>
            <RotateCcw size={17} />
            {t.restoreBackup}
          </Button>
        </div>
      </Card>

      <section className="bottom-grid">
        <Card className="panel">
          <div className="panel-header">
            <div>
              <h2>{t.migrationList}</h2>
              <p>{t.machineBoundExcluded}</p>
            </div>
            <Archive size={22} />
          </div>
          <div className="list-columns">
            <div>
              <h3>{t.defaultMigration}</h3>
              {(scan?.migratable || ["config.toml", "rules", "memories"]).map((item) => (
                <span className="tag" key={item}>{item}</span>
              ))}
            </div>
            <div>
              <h3>{t.neverMigrate}</h3>
              {(scan?.excluded || ["installation_id", "cap_sid", ".sandbox"]).map((item) => (
                <span className="tag danger" key={item}>{item}</span>
              ))}
            </div>
          </div>
        </Card>

        <Card className="panel">
          <div className="panel-header">
            <div>
              <h2>{t.operationLog}</h2>
              <p>{t.latest100}</p>
            </div>
            <HardDriveUpload size={22} />
          </div>
          <div className="events">
            {(store?.events || []).map((event) => (
              <div className="event" key={`${event.ts}-${event.message}`}>
                <span>{formatDate(event.ts)}</span>
                <strong>{event.level}</strong>
                <p>{event.message}</p>
              </div>
            ))}
          </div>
        </Card>
      </section>
        </>
      )}
    </main>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
