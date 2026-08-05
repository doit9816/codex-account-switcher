import { invoke } from "@tauri-apps/api/core";
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
import { RoutingPage } from "./components/routing/RoutingPage";
import { RoutingLogSettings } from "./components/routing/RoutingLogSettings";
import { MeshSharePage } from "./components/mesh/MeshSharePage";
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
  profileScore,
  quotaSummary,
  subscriptionExpiryState,
  tokenState
} from "./profileUtils";

const UI_BUSY_TIMEOUT_MS = 90_000;
const ACCOUNT_PAGE_SIZE = 12;

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
  const passwordTooShort = password.length > 0 && password.length < 8;
  const selectedIdRef = useRef("");
  const autoBusyRef = useRef(false);
  const backgroundTokenBusyRef = useRef(false);
  const aliasRef = useRef("");
  const oauthCompletingRef = useRef(false);
  const storeRef = useRef<StoreView | null>(null);
  const oauthReauthProfileIdRef = useRef<string | null>(null);

  useEffect(() => {
    void refresh();
    void getVersion().then(setAppVersion).catch(() => setAppVersion(""));
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
    setProxyEnabled(!!store.settings.probeProxy?.enabled);
    setProxyUrl(store.settings.probeProxy?.url || "");
    setBackgroundTokenRefreshEnabled(store.settings.backgroundTokenRefreshEnabled ?? false);
    setBackgroundTokenRefreshIntervalSecs(store.settings.backgroundTokenRefreshIntervalSecs || 3600);
    setTokenRefreshThresholdSecs(store.settings.tokenRefreshThresholdSecs ?? 0);
    setAutoProbeEnabled(store.settings.autoProbeEnabled ?? true);
    setAutoProbeIntervalSecs(store.settings.autoProbeIntervalSecs || 60);
    const routing = store.settings.routing;
    setRoutingHost(routing.listenHost || "0.0.0.0");
    setRoutingPort(routing.port || 15722);
    setRoutingMode(routing.mode || "auto");
    setRoutingFixedProfileId(routing.fixedProfileId || "");
    setRoutingStickyTtlSecs(routing.stickyTtlSecs || 3600);
    setRoutingLogRetentionDays(routing.logRetentionDays || 7);
    const mesh = store.settings.mesh;
    if (mesh) {
      setMeshNetworkName(mesh.networkName || "codex-switcher");
      setMeshNodeSourceUrl(mesh.nodeSourceUrl || "https://info.qtet.cn/uptime/easytier");
      setMeshNodeRefreshSecs(mesh.nodeRefreshSecs || 120);
      setMeshAutoStart(!!mesh.autoStart);
      setMeshSyncScope(
        mesh.syncScopeInitialized === false
          ? { accounts: true, rules: false, routing: false, conversations: false }
          : mesh.syncScope || { accounts: true, rules: false, routing: false, conversations: false }
      );
    }
  }, [store]);

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
    if (activePage !== "mesh") return;
    const timer = window.setInterval(() => {
      void invoke<MeshStatus>("mesh_status")
        .then(setMeshStatus)
        .catch(() => {
          // A stopped or restarting mesh runtime is reflected on the next tick.
        });
    }, 3000);
    return () => window.clearInterval(timer);
  }, [activePage]);

  useEffect(() => {
    if (!notice) return;
    const timeout = window.setTimeout(() => {
      setNotice(null);
    }, notice.kind === "error" ? 8000 : 4500);
    return () => window.clearTimeout(timeout);
  }, [notice]);

  useEffect(() => {
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

  async function run<T>(task: () => Promise<T>, okText?: string) {
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
      setOauthStatus("error");
      setOauthError(String(error));
    }
  }

  async function reopenOAuthUrl() {
    if (!oauthSession) return;
    try {
      await invoke("codex_oauth_open_auth_url", { loginId: oauthSession.loginId });
    } catch (error) {
      setOauthError(String(error));
    }
  }

  async function copyOAuthUrl() {
    if (!oauthSession) return;
    try {
      await navigator.clipboard.writeText(oauthSession.authUrl);
      setNotice({ kind: "ok", text: t.copied });
    } catch (error) {
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
      } catch {
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
      await reloadMeshStatus();
      return payload;
    }, "共享码已生成");
  }

  async function copyMeshSharePayload() {
    if (!meshSharePayload.trim()) return;
    await navigator.clipboard.writeText(meshSharePayload);
    setNotice({ kind: "ok", text: "共享码已复制" });
  }

  async function importMeshSharePayload() {
    if (!meshImportPayload.trim()) return;
    await run(async () => {
      const result = await invoke<MeshImportResult>("mesh_import_share_payload", {
        payloadText: meshImportPayload
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
      return mesh;
    }, "已请求设备同步");
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
    setRoutingBusy(true);
    try {
      await run(async () => {
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
    if (!profile || !window.confirm(t.useResetConfirm)) return;
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
    const confirmed = window.confirm(
      `${isCurrent ? t.deleteCurrentAccountConfirm : t.deleteAccountConfirm}\n\n${profile.alias}`
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
        <button
          className="mini-button icon-only"
          onClick={(event) => {
            event.stopPropagation();
            openEditProfile(profile);
          }}
          disabled={busy}
          title={t.editAccount}
          aria-label={t.editAccount}
        >
          <Pencil size={14} />
        </button>
        {(profile.usage.availableResetCount || 0) > 0 && (
          <button
            className="mini-button icon-only"
            onClick={(event) => {
              event.stopPropagation();
              void consumeUsageReset(profile.id);
            }}
            disabled={busy}
            title={t.useReset}
            aria-label={t.useReset}
          >
            <RotateCcw size={14} />
          </button>
        )}
        {needsReauthorization && (
          <button
            className="mini-button icon-only"
            onClick={(event) => {
              event.stopPropagation();
              void reauthorizeProfile(profile);
            }}
            disabled={busy || oauthStatus === "starting" || oauthStatus === "exchanging"}
            title={t.reauthorize}
            aria-label={t.reauthorize}
          >
            <KeyRound size={14} />
          </button>
        )}
        <button
          className="mini-button icon-only"
          onClick={(event) => {
            event.stopPropagation();
            void probeProfile(profile.id);
          }}
          disabled={busy}
          title={t.probeQuota}
          aria-label={t.probeQuota}
        >
          <RefreshCcw size={14} />
        </button>
        <button
          className="mini-button primary icon-only"
          onClick={(event) => {
            event.stopPropagation();
            void switchProfile(profile.id);
          }}
          disabled={busy}
          title={t.switch}
          aria-label={t.switch}
        >
          <Zap size={14} />
        </button>
        {store?.settings.routing.appliedToCodex && (
          <button
            className="mini-button icon-only"
            onClick={(event) => {
              event.stopPropagation();
              void fixProfileToRouting(profile.id);
            }}
            disabled={busy}
            title="固定到路由"
            aria-label="固定到路由"
          >
            <Network size={14} />
          </button>
        )}
        <button
          className="mini-button danger icon-only"
          onClick={(event) => {
            event.stopPropagation();
            void deleteProfile(profile.id);
          }}
          disabled={busy}
          title={t.deleteAccount}
          aria-label={t.deleteAccount}
        >
          <Trash2 size={15} />
        </button>
      </>
    );
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <h1>CodexSwitcher</h1>
          <p>{t.appSubtitle}</p>
        </div>
        <div className="topbar-actions">
          <label className="language-select">
            <span>{t.language}</span>
            <select
              value={languageSetting}
              onChange={(event) => setLanguageSetting(event.target.value as LanguageSetting)}
              title={t.language}
            >
              <option value="system">{t.followSystem}</option>
              <option value="zh-CN">{languageLabels["zh-CN"]}</option>
              <option value="en">{languageLabels.en}</option>
              <option value="zh-TW">{languageLabels["zh-TW"]}</option>
            </select>
          </label>
          <div className="page-tabs" role="tablist">
            <button
              className={`tab-button ${activePage === "dashboard" ? "active" : ""}`}
              onClick={() => setActivePage("dashboard")}
              role="tab"
              aria-selected={activePage === "dashboard"}
            >
              <LayoutDashboard size={17} />
              {t.dashboard}
            </button>
            <button
              className={`tab-button ${activePage === "settings" ? "active" : ""}`}
              onClick={() => setActivePage("settings")}
              role="tab"
              aria-selected={activePage === "settings"}
            >
              <Settings size={17} />
              {t.settings}
            </button>
            <button
              className={`tab-button ${activePage === "mesh" ? "active" : ""}`}
              onClick={() => setActivePage("mesh")}
              role="tab"
              aria-selected={activePage === "mesh"}
            >
              <Share2 size={17} />
              多设备共享
            </button>
            <button
              className={`tab-button ${activePage === "routing" ? "active" : ""}`}
              onClick={() => setActivePage("routing")}
              role="tab"
              aria-selected={activePage === "routing"}
            >
              <Network size={17} />
              路由
            </button>
          </div>
          <button className="icon-button primary" onClick={() => void refresh()} disabled={busy} title={t.refresh}>
            <RefreshCcw size={18} />
            {t.refresh}
          </button>
        </div>
      </header>

      {notice && (
        <div className={`notice ${notice.kind}`} role="status" aria-live="polite">
          <span>{notice.text}</span>
          <button className="notice-close" onClick={() => setNotice(null)} title={t.closeNotice}>
            <X size={15} />
          </button>
        </div>
      )}

      {showUpdateDialog && availableUpdate && (
        <div className="update-dialog-backdrop" role="presentation">
          <section
            className="update-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="update-dialog-title"
          >
            <div className="update-dialog-head">
              <div>
                <span>{t.softwareUpdate}</span>
                <h2 id="update-dialog-title">{t.updateAvailable}: {availableUpdate.version}</h2>
              </div>
              <button
                className="notice-close"
                onClick={() => setShowUpdateDialog(false)}
                disabled={updateInstalling}
                title={t.closeNotice}
              >
                <X size={18} />
              </button>
            </div>
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
                <button className="icon-button" onClick={() => setShowUpdateDialog(false)} disabled={updateInstalling}>
                  {t.closeNotice}
                </button>
              )}
              {!updateInstalled ? (
                <button className="icon-button primary" onClick={() => void installAvailableUpdate()} disabled={updateInstalling}>
                  <Download size={17} /> {t.downloadAndInstall}
                </button>
              ) : (
                <button className="icon-button primary" onClick={() => void relaunch()}>
                  <RotateCcw size={17} /> {t.relaunchNow}
                </button>
              )}
            </div>
          </section>
        </div>
      )}

      {showAddAccountDialog && (
        <div className="update-dialog-backdrop" role="presentation">
          <section className="add-account-dialog" role="dialog" aria-modal="true" aria-labelledby="add-account-title">
            <div className="update-dialog-head">
              <h2 id="add-account-title">{t.addAccount}</h2>
              <button className="notice-close" onClick={closeAddAccountDialog} disabled={busy || oauthStatus === "exchanging"} title={t.closeNotice}>
                <X size={18} />
              </button>
            </div>
            <div className="add-account-tabs" role="tablist">
              {([
                ["oauth", t.oauthLogin],
                ["json", t.tokenJson],
                ["api", t.apiKeyAccount],
                ["import", t.importAccount]
              ] as const).map(([id, label]) => (
                <button key={id} className={addAccountTab === id ? "active" : ""} onClick={() => selectAddAccountTab(id)} role="tab" aria-selected={addAccountTab === id} disabled={oauthStatus === "exchanging"}>
                  {label}
                </button>
              ))}
            </div>
            {addAccountTab !== "api" && (
              <label className="add-account-field">
                {t.importAlias}
                <input value={alias} onChange={(event) => setAlias(event.target.value)} placeholder={t.importAlias} />
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
                        <input value={oauthSession.authUrl} readOnly />
                        <button className="mini-button" onClick={() => void copyOAuthUrl()}>{t.copyLink}</button>
                        <button className="mini-button" onClick={() => void reopenOAuthUrl()}>{t.openAgain}</button>
                      </div>
                    </label>
                    <div className="oauth-callback-row">
                      <input value={oauthCallbackInput} onChange={(event) => setOauthCallbackInput(event.target.value)} placeholder={t.manualCallback} disabled={oauthStatus === "exchanging"} />
                      <button className="icon-button" onClick={() => void submitOAuthCallback()} disabled={!oauthCallbackInput.trim() || oauthStatus === "exchanging"}>{t.submitCallback}</button>
                    </div>
                    {oauthError && <div className="oauth-error">{oauthError}</div>}
                    <div className="oauth-action-row">
                      {oauthStatus === "error" && <button className="icon-button primary" onClick={() => void completeNativeOAuth(oauthSession.loginId)}>{t.retryExchange}</button>}
                      <button className="icon-button" onClick={() => void cancelNativeOAuth()} disabled={oauthStatus === "exchanging"}>{t.cancelOAuth}</button>
                    </div>
                  </>
                ) : (
                  <>
                    {oauthError && <div className="oauth-error">{oauthError}</div>}
                    <button className="icon-button primary wide-button" onClick={() => {
                      setOauthReauthProfileId(null);
                      void beginNativeOAuth();
                    }} disabled={oauthStatus === "starting" || oauthStatus === "exchanging"}>
                      <KeyRound size={17} /> {t.startOAuthLogin}
                    </button>
                  </>
                )}
                <details className="oauth-cli-fallback">
                  <summary>{t.cliFallback}</summary>
                  <p>{t.cliFallbackHint}</p>
                  <button className="icon-button wide-button" onClick={() => void startCliOAuthLogin()} disabled={busy}>
                    <KeyRound size={17} /> {t.startCliLogin}
                  </button>
                  <button className="icon-button wide-button" onClick={() => void importCurrentAuth()} disabled={busy}>
                    <CheckCircle2 size={17} /> {t.oauthImportDone}
                  </button>
                </details>
              </div>
            )}
            {addAccountTab === "json" && (
              <div className="add-account-content">
                <textarea className="auth-json-input" spellCheck={false} value={authJsonInput} onChange={(event) => setAuthJsonInput(event.target.value)} placeholder={t.authJsonPlaceholder} />
                <button className="icon-button primary wide-button" onClick={() => void addAuthJsonAccount()} disabled={busy || !authJsonInput.trim()}>
                  <FileText size={17} /> {t.addFromJson}
                </button>
              </div>
            )}
            {addAccountTab === "api" && (
              <div className="api-provider-form add-account-content">
                <label>{t.apiModel}<input value={apiModel} onChange={(event) => setApiModel(event.target.value)} placeholder="gpt-5.4" /></label>
                <label>{t.apiKey}<input type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} /></label>
                <label>{t.importAlias}<input value={apiProviderName} onChange={(event) => setApiProviderName(event.target.value)} placeholder="可空，默认使用模型名" /></label>
                <details className="api-advanced-settings">
                  <summary>高级设置（可选）</summary>
                  <div className="api-advanced-grid">
                    <label>{t.providerId}<input value={apiProviderId} onChange={(event) => setApiProviderId(event.target.value)} placeholder="自动生成" /></label>
                    <label>{t.apiBaseUrl}<input value={apiBaseUrl} onChange={(event) => setApiBaseUrl(event.target.value)} placeholder="默认 https://api.openai.com/v1" /></label>
                    <label>
                      {t.apiProtocol}
                      <select value={apiWireApi} onChange={(event) => setApiWireApi(event.target.value)}>
                        <option value="responses">{t.apiProtocolResponses}</option>
                        <option value="chat_completions">{t.apiProtocolChat}</option>
                        <option value="anthropic_messages">{t.apiProtocolAnthropic}</option>
                      </select>
                    </label>
                  </div>
                </details>
                <button className="icon-button primary wide-button" onClick={() => void addApiProvider()} disabled={busy || !apiModel.trim() || !apiKey.trim()}>
                  <KeyRound size={17} /> {t.addApiProvider}
                </button>
              </div>
            )}
            {addAccountTab === "import" && (
              <div className="add-account-content">
                <p>{t.codexConfigHint}</p>
                <button className="icon-button primary wide-button" onClick={() => void importCurrentAuth()} disabled={busy}>
                  <Download size={17} /> {t.importCurrent}
                </button>
              </div>
            )}
          </section>
        </div>
      )}

      {editingProfile && (
        <div className="update-dialog-backdrop" role="presentation">
          <section className="add-account-dialog edit-account-dialog" role="dialog" aria-modal="true" aria-labelledby="edit-account-title">
            <div className="update-dialog-head">
              <div>
                <h2 id="edit-account-title">{t.editAccountInfo}</h2>
                <span className="edit-account-type">
                  {editingProfile.apiConfig ? t.apiKeyAccount : t.oauthLogin}
                </span>
              </div>
              <button className="notice-close" onClick={closeEditProfile} disabled={busy} title={t.closeNotice}>
                <X size={18} />
              </button>
            </div>
            {editingProfile.apiConfig ? (
              <div className="api-provider-form edit-api-provider-form add-account-content">
                <label>
                  {t.accountAlias}
                  <input value={editAliasDraft} onChange={(event) => setEditAliasDraft(event.target.value)} placeholder={t.accountAlias} />
                </label>
                <label>
                  {t.apiModel}
                  <input value={editModelDraft} onChange={(event) => setEditModelDraft(event.target.value)} placeholder="gpt-5.4" />
                </label>
                <label>
                  {t.apiKeyOptional}
                  <input type="password" value={editApiKeyDraft} onChange={(event) => setEditApiKeyDraft(event.target.value)} />
                </label>
                <details className="api-advanced-settings form-span-all">
                  <summary>高级设置（可选）</summary>
                  <div className="api-advanced-grid">
                    <label>
                      {t.providerId}
                      <input value={editProviderIdDraft} onChange={(event) => setEditProviderIdDraft(event.target.value)} placeholder="自动生成" />
                    </label>
                    <label>
                      {t.apiBaseUrl}
                      <input value={editBaseUrlDraft} onChange={(event) => setEditBaseUrlDraft(event.target.value)} placeholder="默认 https://api.openai.com/v1" />
                    </label>
                    <label>
                      {t.apiProtocol}
                      <select value={editWireApiDraft} onChange={(event) => setEditWireApiDraft(event.target.value)}>
                        <option value="responses">{t.apiProtocolResponses}</option>
                        <option value="chat_completions">{t.apiProtocolChat}</option>
                        <option value="anthropic_messages">{t.apiProtocolAnthropic}</option>
                      </select>
                    </label>
                  </div>
                </details>
                <label className="form-span-all">
                  {t.accountNote}
                  <textarea
                    className="profile-note-input compact"
                    value={editNoteDraft}
                    onChange={(event) => setEditNoteDraft(event.target.value)}
                    placeholder={t.notePlaceholder}
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
                  <button className="icon-button" onClick={closeEditProfile} disabled={busy}>{t.closeNotice}</button>
                  <button
                    className="icon-button primary"
                    onClick={() => void saveProfileDetails()}
                    disabled={busy || !editAliasDraft.trim() || !editModelDraft.trim()}
                  >
                    <ShieldCheck size={17} /> {t.saveRules}
                  </button>
                </div>
              </div>
            ) : (
              <div className="edit-oauth-form add-account-content">
                <label className="add-account-field">
                  {t.accountAlias}
                  <input value={editAliasDraft} onChange={(event) => setEditAliasDraft(event.target.value)} />
                </label>
                <label className="add-account-field">
                  {t.accountNote}
                  <textarea
                    className="profile-note-input"
                    value={editNoteDraft}
                    onChange={(event) => setEditNoteDraft(event.target.value)}
                    placeholder={t.notePlaceholder}
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
                  <button className="icon-button" onClick={closeEditProfile} disabled={busy}>{t.closeNotice}</button>
                  <button
                    className="icon-button primary"
                    onClick={() => void saveProfileDetails()}
                    disabled={busy || !editAliasDraft.trim()}
                  >
                    <ShieldCheck size={17} /> {t.saveRules}
                  </button>
                </div>
              </div>
            )}
          </section>
        </div>
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
          onExportMigration={exportMeshMigrationShare}
          onImportMigration={importMeshMigrationShare}
        />
      ) : activePage === "settings" ? (
        <>
      <section className="toolbar-band">
        <div className="path-control">
          <label>{t.codexDir}</label>
          <input value={codexHome} onChange={(event) => setCodexHome(event.target.value)} />
          <button className="icon-button" onClick={() => void scanHome()} disabled={busy} title={t.scan}>
            <FileSearch size={17} />
            {t.scan}
          </button>
          <button className="icon-button" onClick={() => void openCodexHome()} disabled={busy} title={t.openDir}>
            <FolderOpen size={17} />
            {t.openDir}
          </button>
        </div>
        <div className="scan-state">
          <StatusPill ok={!!scan?.exists} text={scan?.exists ? t.dirExists : t.notScanned} />
          <StatusPill ok={!!scan?.hasAuth} text={scan?.hasAuth ? t.authFound : t.authMissing} />
        </div>
      </section>

      <section className="proxy-band">
        <label className="checkline" title={t.proxyEnabled}>
          <input
            type="checkbox"
            checked={proxyEnabled}
            onChange={(event) => setProxyEnabled(event.target.checked)}
          />
          {t.proxyEnabled}
        </label>
        <input
          className="proxy-input"
          placeholder={t.proxyPlaceholder}
          value={proxyUrl}
          onChange={(event) => setProxyUrl(event.target.value)}
        />
        <button className="icon-button" onClick={() => void saveProxySettings()} disabled={busy} title={t.saveProxy}>
          <ShieldCheck size={17} />
          {t.saveProxy}
        </button>
        <button className="icon-button" onClick={() => void testProxySettings()} disabled={busy} title={t.testProxy}>
          <Wifi size={17} />
          {t.testProxy}
        </button>
        <span className="proxy-hint">{t.proxyHint}</span>
      </section>

      <section className="auto-band">
        <label className="checkline" title={t.backgroundTokenKeepalive}>
          <input
            type="checkbox"
            checked={backgroundTokenRefreshEnabled}
            onChange={(event) => setBackgroundTokenRefreshEnabled(event.target.checked)}
          />
          {t.backgroundTokenKeepalive}
        </label>
        <label>
          {t.keepaliveInterval}
          <input
            className="small-number"
            type="number"
            min={3600}
            value={backgroundTokenRefreshIntervalSecs}
            onChange={(event) => setBackgroundTokenRefreshIntervalSecs(Number(event.target.value) || 3600)}
            title={t.keepaliveInterval}
          />
        </label>
        <label>
          {t.refreshThreshold}
          <input
            className="small-number"
            type="number"
            min={0}
            value={tokenRefreshThresholdSecs}
            onChange={(event) => setTokenRefreshThresholdSecs(Number(event.target.value) || 0)}
            title={t.refreshThreshold}
          />
        </label>
        <label className="checkline" title={t.autoProbeQuota}>
          <input
            type="checkbox"
            checked={autoProbeEnabled}
            onChange={(event) => setAutoProbeEnabled(event.target.checked)}
          />
          {t.autoProbeQuota}
        </label>
        <label>
          {t.probeInterval}
          <input
            className="small-number"
            type="number"
            min={30}
            value={autoProbeIntervalSecs}
            onChange={(event) => setAutoProbeIntervalSecs(Number(event.target.value) || 60)}
            title={t.probeInterval}
          />
        </label>
        <button className="icon-button" onClick={() => void saveAutoSettings()} disabled={busy || autoProbeRunning} title={t.saveAutoRefresh}>
          <RefreshCcw size={17} />
          {autoProbeRunning ? "探测中..." : t.saveAutoRefresh}
        </button>
        <button className="icon-button" onClick={() => void refreshOtherProfileTokensNow()} disabled={busy} title={t.keepaliveNow}>
          <KeyRound size={17} />
          {t.keepaliveNow}
        </button>
        <span className="proxy-hint">{t.autoHint}</span>
      </section>

      <RoutingLogSettings
        t={t}
        retentionDays={routingLogRetentionDays}
        busy={busy}
        onRetentionDaysChange={setRoutingLogRetentionDays}
        onSave={saveRoutingLogSettings}
        onOpenLogs={openLogsDirectory}
      />

      <section className="update-band">
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
          <button className="icon-button" onClick={() => void checkForUpdate(true)} disabled={updateChecking || updateInstalling} title={t.checkUpdate}>
            <RefreshCcw size={17} />
            {updateChecking ? t.checkUpdate : t.checkUpdate}
          </button>
          <button
            className="icon-button primary"
            onClick={() => void installAvailableUpdate()}
            disabled={!availableUpdate || updateInstalling || updateInstalled}
            title={t.downloadAndInstall}
          >
            <Download size={17} />
            {t.downloadAndInstall}
          </button>
          <button className="icon-button" onClick={() => void relaunch()} disabled={!updateInstalled} title={t.relaunchNow}>
            <RotateCcw size={17} />
            {t.relaunchNow}
          </button>
        </div>
      </section>
        </>
      ) : (
        <>

      <section className="main-grid account-only-grid">
        <div className="panel account-panel">
          <div className="panel-header">
            <div>
              <h2>{t.accounts}</h2>
              <p>{filteredProfiles.length}/{store?.profiles.length || 0} {t.profiles}</p>
            </div>
            <div className="compact-actions">
              <label className="checkline compact-checkline" title={t.forceSwitch}>
                <input type="checkbox" checked={forceSwitch} onChange={(event) => setForceSwitch(event.target.checked)} />
                {t.forceSwitch}
              </label>
              <select
                value={accountExpiryFilter}
                onChange={(event) => setAccountExpiryFilter(event.target.value as AccountExpiryFilter)}
                title={t.expiryFilter}
                aria-label={t.expiryFilter}
              >
                <option value="all">{t.allExpiry}</option>
                <option value="valid">{t.validOnly}</option>
                <option value="expired">{t.expiredOnly}</option>
              </select>
              <select
                value={accountStatusFilter}
                onChange={(event) => setAccountStatusFilter(event.target.value as AccountStatusFilter)}
                title={t.loginStatusFilter}
                aria-label={t.loginStatusFilter}
              >
                <option value="all">{t.allLoginStatuses}</option>
                <option value="available">{t.available}</option>
                <option value="relogin">{t.reloginRequired}</option>
                <option value="disabled">{t.disabled}</option>
                <option value="cooling">{t.cooling}</option>
                <option value="expired">{t.expired}</option>
                <option value="error">{t.probeFailed}</option>
              </select>
              <div className="view-toggle" role="group" aria-label={t.accountViewMode}>
                <button
                  className={accountViewMode === "cards" ? "active" : ""}
                  onClick={() => setAccountViewMode("cards")}
                  title={t.cardView}
                  aria-label={t.cardView}
                >
                  <Grid2X2 size={16} />
                </button>
                <button
                  className={accountViewMode === "rows" ? "active" : ""}
                  onClick={() => setAccountViewMode("rows")}
                  title={t.rowView}
                  aria-label={t.rowView}
                >
                  <Rows3 size={16} />
                </button>
              </div>
              <input
                className="alias-input"
                placeholder={t.searchPlaceholder}
                value={accountFilter}
                onChange={(event) => setAccountFilter(event.target.value)}
                title={t.searchPlaceholder}
              />
              <button className="icon-button primary" onClick={() => setShowAddAccountDialog(true)} disabled={busy} title={t.addAccount}>
                <KeyRound size={17} />
                {t.addAccount}
              </button>
            </div>
          </div>

          <details className="codex-config-panel">
            <summary>{t.codexConfig}</summary>
            <p>{t.codexConfigHint}</p>
            <div className="config-toolbar">
              <button className="icon-button" onClick={() => void loadCodexConfigFiles()} disabled={busy}>
                <FileSearch size={16} /> {t.loadConfig}
              </button>
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
                    <button className="mini-button" onClick={() => void formatCodexConfig("auth.json")} disabled={busy || !authJsonDraft.trim()}>
                      {t.formatConfig}
                    </button>
                    <button className="mini-button primary" onClick={() => void saveCodexConfig("auth.json")} disabled={busy || !authJsonDraft.trim()}>
                      {t.saveConfig}
                    </button>
                  </div>
                </div>
                <textarea
                  className="config-editor"
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
                    <button className="mini-button" onClick={() => void formatCodexConfig("config.toml")} disabled={busy}>
                      {t.formatConfig}
                    </button>
                    <button className="mini-button primary" onClick={() => void saveCodexConfig("config.toml")} disabled={busy}>
                      {t.saveConfig}
                    </button>
                  </div>
                </div>
                <textarea
                  className="config-editor"
                  spellCheck={false}
                  value={configTomlDraft}
                  onChange={(event) => setConfigTomlDraft(event.target.value)}
                  placeholder={'model = "gpt-5"\nmodel_provider = "openai"'}
                />
              </section>
            </div>
          </details>

          {accountViewMode === "cards" ? (
          <div className="account-card-grid">
            {pagedProfiles.map((profile) => {
              const isCurrent = currentGlobalProfileId === profile.id;
              const limits = profile.usage.detectedLimits || [];
              const accountText = profile.summary.email || profile.summary.accountId || profile.apiConfig?.baseUrl || t.unknownAccount;
              const canCopyAccount = accountText !== t.unknownAccount;
              const needsReauthorization = profileNeedsReauthorization(profile);
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
                          <button
                            className="mini-button icon-only account-copy-button"
                            onClick={(event) => {
                              event.stopPropagation();
                              void copyProfileAccount(profile);
                            }}
                            title={t.copyAccount}
                            aria-label={t.copyAccount}
                          >
                            <Copy size={13} />
                          </button>
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
                      <span>{t.reloginRequiredHint}</span>
                    </div>
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

                  {!profile.apiConfig && <div className="account-limit-list">
                    {limits.slice(0, 2).map((item, index) => {
                      const remainingPercent = limitRemainingPercent(item);
                      return (
                        <div className="account-limit" key={`${item.window}-${index}`}>
                          <div className="account-limit-head">
                            <span>{localizedLimitLabel(item.label || item.window, t)}</span>
                            <strong>{remainingPercent != null ? `${remainingPercent}%` : formatUsage(item.used, item.limit, t)}</strong>
                          </div>
                          <div className="account-limit-track">
                            <i style={{ width: `${remainingPercent ?? 0}%` }} />
                          </div>
                          <small>{item.resetAt ? formatReset(item.resetAt) : t.notProbed}</small>
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
                  <span className="quota-cell">{quotaSummary(profile, t)}</span>
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
                <button
                  className="mini-button"
                  onClick={() => setAccountPage((page) => Math.max(1, page - 1))}
                  disabled={accountPage <= 1}
                  title={t.previousPage}
                  aria-label={t.previousPage}
                >
                  {t.previousPage}
                </button>
                <button
                  className="mini-button"
                  onClick={() => setAccountPage((page) => Math.min(totalAccountPages, page + 1))}
                  disabled={accountPage >= totalAccountPages}
                  title={t.nextPage}
                  aria-label={t.nextPage}
                >
                  {t.nextPage}
                </button>
              </div>
            </div>
          )}

        </div>
      </section>

      <section className="migration-band">
        <div className="migration-copy">
          <h2>多设备共享与迁移</h2>
          <p>迁移包导出/导入、连接分享码和设备同步已统一放到多设备共享页。</p>
        </div>
        <div className="migration-controls">
          <button className="icon-button primary" onClick={() => setActivePage("mesh")} disabled={busy} title="打开多设备共享">
            <Share2 size={17} />
            打开多设备共享
          </button>
          <button className="icon-button" onClick={() => void restoreBackup()} disabled={busy} title={t.restoreBackup}>
            <RotateCcw size={17} />
            {t.restoreBackup}
          </button>
        </div>
      </section>

      <section className="bottom-grid">
        <div className="panel">
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
        </div>

        <div className="panel">
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
        </div>
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
