import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  Archive,
  CheckCircle2,
  Download,
  FileSearch,
  FolderOpen,
  Gauge,
  HardDriveUpload,
  KeyRound,
  LayoutDashboard,
  RefreshCcw,
  RotateCcw,
  Settings,
  ShieldCheck,
  Upload,
  X,
  Zap
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

type AuthSummary = {
  email?: string;
  plan?: string;
  accountId?: string;
  userId?: string;
  organizationId?: string;
  accessTokenExp?: number;
  idTokenExp?: number;
  authMode?: string;
};

type QuotaRule = {
  hourlyLimit?: number;
  dailyLimit?: number;
  cooldownMinutes: number;
};

type UsageStats = {
  hourlyUsed: number;
  dailyUsed: number;
  detectedLimits: DetectedLimit[];
  detectedSummary?: string;
  lastProbeAt?: string;
  lastProbeStatus?: string;
  lastError?: string;
  lastUsedAt?: string;
  estimatedResetAt?: string;
  lastTokenRefreshAt?: string;
  lastTokenRefreshStatus?: string;
  lastTokenRefreshError?: string;
};

type DetectedLimit = {
  window: string;
  used?: number;
  limit?: number;
  remaining?: number;
  usedPercent?: number;
  remainingPercent?: number;
  resetAt?: string;
  label?: string;
};

type Profile = {
  id: string;
  alias: string;
  enabled: boolean;
  priority: number;
  cooldownUntil?: string;
  quotaRule: QuotaRule;
  summary: AuthSummary;
  usage: UsageStats;
  createdAt: string;
  updatedAt: string;
};

type AppEvent = {
  ts: string;
  level: string;
  message: string;
};

type StoreView = {
  settings: {
    codexHome?: string;
    currentProfileId?: string;
    autoSwitchEnabled: boolean;
    probeProxy?: {
      enabled: boolean;
      url: string;
    };
    autoTokenRefreshEnabled: boolean;
    autoRefreshIntervalSecs: number;
    backgroundTokenRefreshEnabled: boolean;
    backgroundTokenRefreshIntervalSecs: number;
    tokenRefreshThresholdSecs: number;
    autoProbeEnabled: boolean;
    autoProbeIntervalSecs: number;
  };
  profiles: Profile[];
  events: AppEvent[];
};

type CodexScan = {
  codexHome: string;
  exists: boolean;
  hasAuth: boolean;
  currentAuth?: AuthSummary;
  migratable: string[];
  excluded: string[];
};

type BundleManifest = {
  exportedAt: string;
  platform: string;
  profileCount: number;
  includeConversations: boolean;
  files: Array<{ path: string; bytes: number; sha256: string }>;
};

type Notice = {
  kind: "ok" | "warn" | "error" | "info";
  text: string;
};

const emptyQuota: QuotaRule = {
  hourlyLimit: undefined,
  dailyLimit: undefined,
  cooldownMinutes: 180
};

export default function App() {
  const [store, setStore] = useState<StoreView | null>(null);
  const [scan, setScan] = useState<CodexScan | null>(null);
  const [selectedId, setSelectedId] = useState<string>("");
  const [codexHome, setCodexHome] = useState("");
  const [alias, setAlias] = useState("");
  const [accountFilter, setAccountFilter] = useState("");
  const [password, setPassword] = useState("");
  const [includeConversations, setIncludeConversations] = useState(false);
  const [restoreConversations, setRestoreConversations] = useState(false);
  const [forceSwitch, setForceSwitch] = useState(false);
  const [proxyEnabled, setProxyEnabled] = useState(false);
  const [proxyUrl, setProxyUrl] = useState("");
  const [backgroundTokenRefreshEnabled, setBackgroundTokenRefreshEnabled] = useState(false);
  const [backgroundTokenRefreshIntervalSecs, setBackgroundTokenRefreshIntervalSecs] = useState(3600);
  const [tokenRefreshThresholdSecs, setTokenRefreshThresholdSecs] = useState(0);
  const [autoProbeEnabled, setAutoProbeEnabled] = useState(true);
  const [autoProbeIntervalSecs, setAutoProbeIntervalSecs] = useState(60);
  const [quotaDraft, setQuotaDraft] = useState<QuotaRule>(emptyQuota);
  const [priorityDraft, setPriorityDraft] = useState(100);
  const [enabledDraft, setEnabledDraft] = useState(true);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);
  const [activePage, setActivePage] = useState<"dashboard" | "settings">("dashboard");

  const selectedProfile = useMemo(
    () => store?.profiles.find((profile) => profile.id === selectedId),
    [store, selectedId]
  );
  const filteredProfiles = useMemo(() => {
    const profiles = store?.profiles || [];
    const query = accountFilter.trim().toLowerCase();
    if (!query) return profiles;
    return profiles.filter((profile) => {
      const values = [
        profile.alias,
        profile.summary.email,
        profile.summary.accountId,
        profile.summary.plan,
        profile.summary.authMode,
        accountState(profile),
        quotaSummary(profile),
        tokenState(profile)
      ];
      return values.some((value) => String(value || "").toLowerCase().includes(query));
    });
  }, [store?.profiles, accountFilter]);
  const passwordTooShort = password.length > 0 && password.length < 8;
  const selectedIdRef = useRef("");
  const autoBusyRef = useRef(false);
  const backgroundTokenBusyRef = useRef(false);

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    selectedIdRef.current = selectedId;
  }, [selectedId]);

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
  }, [store]);

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
    if (!selectedProfile) return;
    setQuotaDraft(selectedProfile.quotaRule);
    setPriorityDraft(selectedProfile.priority);
    setEnabledDraft(selectedProfile.enabled);
  }, [selectedProfile]);

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
    try {
      const result = await task();
      if (okText) setNotice({ kind: "ok", text: okText });
      return result;
    } catch (error) {
      setNotice({ kind: "error", text: String(error) });
      return undefined;
    } finally {
      setBusy(false);
    }
  }

  async function refresh() {
    const view = await invoke<StoreView>("get_store");
    setStore(view);
    if (view.settings.codexHome) {
      const currentScan = await invoke<CodexScan>("scan_codex_home", {
        codexHome: view.settings.codexHome
      });
      setScan(currentScan);
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
    }, "已扫描 Codex 目录");
  }

  async function openCodexHome() {
    await run(async () => {
      await invoke("open_codex_home", {
        codexHome: codexHome || undefined
      });
    }, "已打开 Codex 目录");
  }

  async function importCurrentAuth() {
    await run(async () => {
      const view = await invoke<StoreView>("import_current_auth_as_profile", {
        codexHome: codexHome || undefined,
        alias: alias || undefined
      });
      setStore(view);
      setSelectedId(view.profiles[0]?.id || "");
      setAlias("");
      return view;
    }, "已导入当前账号");
  }

  async function saveQuota() {
    if (!selectedProfile) return;
    await run(async () => {
      const view = await invoke<StoreView>("save_quota_rule", {
        profileId: selectedProfile.id,
        hourlyLimit: normalizeNumber(quotaDraft.hourlyLimit),
        dailyLimit: normalizeNumber(quotaDraft.dailyLimit),
        cooldownMinutes: quotaDraft.cooldownMinutes || 180,
        enabled: enabledDraft,
        priority: priorityDraft
      });
      setStore(view);
      return view;
    }, "已保存账号规则");
  }

  async function saveProxySettings() {
    await run(async () => {
      const view = await invoke<StoreView>("save_proxy_settings", {
        enabled: proxyEnabled,
        url: proxyUrl
      });
      setStore(view);
      return view;
    }, proxyEnabled ? "已保存探测代理设置" : "已关闭探测代理");
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
    }, "已保存自动刷新设置");
  }

  async function autoProbeTick() {
    const profileId = selectedIdRef.current;
    if (!profileId || autoBusyRef.current) return;
    autoBusyRef.current = true;
    try {
      await invoke("probe_usage", { profileId });
      const view = await invoke<StoreView>("get_store");
      setStore(view);
    } catch (error) {
      console.warn("auto probe failed", error);
    } finally {
      autoBusyRef.current = false;
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
        text: `token 保活完成：刷新 ${result.refreshed} 个，跳过 ${result.skipped} 个，失败 ${result.failed} 个`
      });
      return result;
    });
  }

  async function switchProfile(profileId = selectedId) {
    const profile = store?.profiles.find((item) => item.id === profileId);
    if (!profile) return;
    setSelectedId(profile.id);
    await run(async () => {
      const result = await invoke<{ message: string; codexRunning: boolean }>("switch_profile", {
        profileId: profile.id,
        codexHome: codexHome || undefined,
        force: forceSwitch
      });
      await refresh();
      setNotice({ kind: result.codexRunning ? "warn" : "ok", text: result.message });
      return result;
    });
  }

  async function switchSelected() {
    await switchProfile();
  }

  async function autoSwitch() {
    if (!store) return;
    const candidate = store.profiles
      .filter((profile) => profile.enabled && !isCooling(profile))
      .sort((a, b) => profileScore(b) - profileScore(a))[0];
    if (!candidate) {
      setNotice({ kind: "warn", text: "没有可用账号：全部被禁用或仍在冷却中" });
      return;
    }
    setSelectedId(candidate.id);
    await run(async () => {
      const result = await invoke<{ message: string; codexRunning: boolean }>("switch_profile", {
        profileId: candidate.id,
        codexHome: codexHome || undefined,
        force: false
      });
      await refresh();
      setNotice({ kind: result.codexRunning ? "warn" : "ok", text: `已自动选择 ${candidate.alias}：${result.message}` });
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

  async function exportBundle() {
    if (passwordTooShort) {
      setNotice({ kind: "warn", text: "迁移包口令至少 8 位；如需明文导出请清空口令" });
      return;
    }
    const path = await save({
      title: "导出全部 Codex 账号",
      defaultPath: password ? "codex-switcher.zip.enc" : "codex-switcher.zip",
      filters: [{ name: "Codex Switcher Bundle", extensions: ["zip", "enc"] }]
    });
    if (!path) return;
    await run(async () => {
      const manifest = await invoke<BundleManifest>("export_all_accounts_bundle", {
        outputPath: path,
        password,
        includeConversations
      });
      const conversationCount = manifest.files.filter((file) => isConversationFile(file.path)).length;
      setNotice({
        kind: password ? "ok" : "warn",
        text: `${password ? "已加密导出" : "已明文导出"} ${manifest.profileCount} 个账号，${manifest.files.length} 个配置文件，对话文件 ${conversationCount} 个`
      });
      return manifest;
    });
  }

  async function importBundle() {
    if (passwordTooShort) {
      setNotice({ kind: "warn", text: "迁移包口令至少 8 位；明文 zip 导入请清空口令" });
      return;
    }
    const path = await open({
      title: "导入 Codex 账号迁移包",
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
        text: `已导入 ${result.importedProfiles} 个账号，恢复 ${result.restoredFiles} 个文件；包内账号数 ${manifest.profileCount}`
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
    }, "已恢复最近一次 auth.json 备份");
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <h1>Codex Account Switcher</h1>
          <p>多账号额度观察、全局切换和一键加密迁移</p>
        </div>
        <div className="topbar-actions">
          <div className="page-tabs" role="tablist">
            <button
              className={`tab-button ${activePage === "dashboard" ? "active" : ""}`}
              onClick={() => setActivePage("dashboard")}
              role="tab"
              aria-selected={activePage === "dashboard"}
            >
              <LayoutDashboard size={17} />
              仪表板
            </button>
            <button
              className={`tab-button ${activePage === "settings" ? "active" : ""}`}
              onClick={() => setActivePage("settings")}
              role="tab"
              aria-selected={activePage === "settings"}
            >
              <Settings size={17} />
              设置
            </button>
          </div>
          <button className="icon-button primary" onClick={() => void refresh()} disabled={busy} title="重新读取本工具保存的账号、设置和操作记录">
            <RefreshCcw size={18} />
            刷新
          </button>
        </div>
      </header>

      {notice && (
        <div className={`notice ${notice.kind}`} role="status" aria-live="polite">
          <span>{notice.text}</span>
          <button className="notice-close" onClick={() => setNotice(null)} title="关闭提示">
            <X size={15} />
          </button>
        </div>
      )}

      {activePage === "settings" ? (
        <>
      <section className="toolbar-band">
        <div className="path-control">
          <label>Codex 目录</label>
          <input value={codexHome} onChange={(event) => setCodexHome(event.target.value)} />
          <button className="icon-button" onClick={() => void scanHome()} disabled={busy} title="检查这个 Codex 目录是否存在，以及里面是否有 auth.json">
            <FileSearch size={17} />
            扫描
          </button>
          <button className="icon-button" onClick={() => void openCodexHome()} disabled={busy} title="用系统文件管理器打开当前 Codex 目录">
            <FolderOpen size={17} />
            打开目录
          </button>
        </div>
        <div className="scan-state">
          <StatusPill ok={!!scan?.exists} text={scan?.exists ? "目录存在" : "未扫描"} />
          <StatusPill ok={!!scan?.hasAuth} text={scan?.hasAuth ? "发现 auth.json" : "未发现 auth.json"} />
        </div>
      </section>

      <section className="proxy-band">
        <label className="checkline" title="开启后，额度探测和 token 保活请求会走下面填写的代理地址">
          <input
            type="checkbox"
            checked={proxyEnabled}
            onChange={(event) => setProxyEnabled(event.target.checked)}
          />
          探测接口走代理
        </label>
        <input
          className="proxy-input"
          placeholder="http://127.0.0.1:7890 或 socks5://127.0.0.1:7890"
          value={proxyUrl}
          onChange={(event) => setProxyUrl(event.target.value)}
        />
        <button className="icon-button" onClick={() => void saveProxySettings()} disabled={busy} title="保存代理开关和代理地址，仅影响本工具的探测/保活请求">
          <ShieldCheck size={17} />
          保存代理
        </button>
        <span className="proxy-hint">影响额度探测和 token 保活接口，不修改系统代理。</span>
      </section>

      <section className="auto-band">
        <label className="checkline" title="定期检查非当前账号 token；默认关闭，避免多设备 refresh_token 被轮换顶掉">
          <input
            type="checkbox"
            checked={backgroundTokenRefreshEnabled}
            onChange={(event) => setBackgroundTokenRefreshEnabled(event.target.checked)}
          />
          其他账号 token 保活
        </label>
        <label>
          保活间隔秒
          <input
            className="small-number"
            type="number"
            min={3600}
            value={backgroundTokenRefreshIntervalSecs}
            onChange={(event) => setBackgroundTokenRefreshIntervalSecs(Number(event.target.value) || 3600)}
            title="其他账号 token 检查间隔，最少 3600 秒"
          />
        </label>
        <label>
          提前刷新秒
          <input
            className="small-number"
            type="number"
            min={0}
            value={tokenRefreshThresholdSecs}
            onChange={(event) => setTokenRefreshThresholdSecs(Number(event.target.value) || 0)}
            title="距离过期多少秒内尝试刷新；0 表示到期后再刷新，最安全"
          />
        </label>
        <label className="checkline" title="定期探测当前选中账号的 5 小时/1 周额度">
          <input
            type="checkbox"
            checked={autoProbeEnabled}
            onChange={(event) => setAutoProbeEnabled(event.target.checked)}
          />
          自动刷新额度
        </label>
        <label>
          额度间隔秒
          <input
            className="small-number"
            type="number"
            min={30}
            value={autoProbeIntervalSecs}
            onChange={(event) => setAutoProbeIntervalSecs(Number(event.target.value) || 60)}
            title="额度自动探测间隔，默认 60 秒"
          />
        </label>
        <button className="icon-button" onClick={() => void saveAutoSettings()} disabled={busy} title="保存 token 同步、其他账号保活、额度自动刷新的间隔和开关">
          <RefreshCcw size={17} />
          保存自动刷新
        </button>
        <button className="icon-button" onClick={() => void refreshOtherProfileTokensNow()} disabled={busy} title="立即检查其他账号 token；如果 Codex 正在运行会暂停，避免顶掉当前会话">
          <KeyRound size={17} />
          立即保活
        </button>
        <span className="proxy-hint">默认到期后刷新；提前刷新可能被服务端拒绝。其他账号保活只更新各自加密 profile。</span>
      </section>
        </>
      ) : (
        <>

      <section className="main-grid account-only-grid">
        <div className="panel account-panel">
          <div className="panel-header">
            <div>
              <h2>账号</h2>
              <p>{filteredProfiles.length}/{store?.profiles.length || 0} 个 profile</p>
            </div>
            <div className="compact-actions">
              <input
                className="alias-input"
                placeholder="搜索账号/额度/状态"
                value={accountFilter}
                onChange={(event) => setAccountFilter(event.target.value)}
                title="模糊搜索账号邮箱、别名、状态、额度、token 状态"
              />
              <input
                className="alias-input"
                placeholder="导入别名"
                value={alias}
                onChange={(event) => setAlias(event.target.value)}
                title="导入当前 auth.json 时使用的账号别名，不用于搜索"
              />
              <button className="icon-button" onClick={() => void importCurrentAuth()} disabled={busy} title="读取当前 Codex 目录的 auth.json，并保存成一个加密账号 profile">
                <KeyRound size={17} />
                导入当前
              </button>
            </div>
          </div>

          <div className="account-table" role="table">
            <div className="account-row header" role="row">
              <span>账号</span>
              <span>计划</span>
              <span>状态</span>
              <span>额度</span>
              <span>优先级</span>
              <span>Access 过期</span>
              <span>Token</span>
              <span>探测</span>
              <span>操作</span>
            </div>
            {filteredProfiles.map((profile) => (
              <div
                key={profile.id}
                className={`account-row ${selectedId === profile.id ? "selected" : ""}`}
                onClick={() => setSelectedId(profile.id)}
                role="row"
              >
                <span>
                  <strong>{profile.alias}</strong>
                  <small>{profile.summary.email || profile.summary.accountId || "未知账号"}</small>
                </span>
                <span>{profile.summary.plan || profile.summary.authMode || "-"}</span>
                <span>
                  <StatusPill ok={profile.enabled && !isCooling(profile)} text={accountState(profile)} />
                </span>
                <span className="quota-cell">{quotaSummary(profile)}</span>
                <span>{profile.priority}</span>
                <span>{formatUnix(profile.summary.accessTokenExp)}</span>
                <span>{tokenState(profile)}</span>
                <span>{profile.usage.lastProbeStatus || "未探测"}</span>
                <span className="row-actions">
                  <button
                    className="mini-button"
                    onClick={(event) => {
                      event.stopPropagation();
                      void probeProfile(profile.id);
                    }}
                    disabled={busy}
                    title="用这个账号 token 探测剩余额度，不会切换全局账号"
                  >
                    探测
                  </button>
                  <button
                    className="mini-button primary"
                    onClick={(event) => {
                      event.stopPropagation();
                      void switchProfile(profile.id);
                    }}
                    disabled={busy}
                    title="把这个账号写入全局 auth.json，新的 Codex 会话会使用它"
                  >
                    切换
                  </button>
                </span>
              </div>
            ))}
            {filteredProfiles.length === 0 && (
              <div className="account-empty">没有匹配的账号</div>
            )}
          </div>

          <div className="inline-detail">
            <div className="inline-detail-head">
              <div>
                <h3>选中账号规则</h3>
                <p>{selectedProfile?.summary.email || selectedProfile?.alias || "选择一个账号"}</p>
              </div>
              <StatusPill
                ok={store?.settings.currentProfileId === selectedProfile?.id}
                text={store?.settings.currentProfileId === selectedProfile?.id ? "当前全局账号" : "未写入全局"}
              />
            </div>
            <div className="probe-box compact-probe">
              <div>
                <span>探测摘要</span>
                <strong>{friendlyProbeSummary(selectedProfile)}</strong>
              </div>
              <div className="detected-limits">
                {(selectedProfile?.usage.detectedLimits || []).map((item, index) => (
                  <span className="limit-chip" key={`${item.window}-${item.label || ""}-${index}`}>
                    {formatLimitChip(item)}
                    {item.remaining != null ? ` 剩 ${item.remaining}` : ""}
                  </span>
                ))}
              </div>
            </div>
            <div className="form-grid">
            <label>
              每小时限额
              <input
                type="number"
                min={0}
                value={quotaDraft.hourlyLimit ?? ""}
                onChange={(event) => setQuotaDraft({ ...quotaDraft, hourlyLimit: parseOptionalNumber(event.target.value) })}
                title="本地估算用的每小时额度；留空表示不限"
              />
            </label>
            <label>
              每天限额
              <input
                type="number"
                min={0}
                value={quotaDraft.dailyLimit ?? ""}
                onChange={(event) => setQuotaDraft({ ...quotaDraft, dailyLimit: parseOptionalNumber(event.target.value) })}
                title="本地估算用的每天额度；留空表示不限"
              />
            </label>
            <label>
              冷却分钟
              <input
                type="number"
                min={1}
                value={quotaDraft.cooldownMinutes}
                onChange={(event) => setQuotaDraft({ ...quotaDraft, cooldownMinutes: Number(event.target.value) || 180 })}
                title="额度耗尽或 429 后冷却多久再参与自动选择"
              />
            </label>
            <label>
              优先级
              <input
                type="number"
                value={priorityDraft}
                onChange={(event) => setPriorityDraft(Number(event.target.value) || 0)}
                title="自动选择账号时的权重；数字越大，在额度接近时越优先"
              />
            </label>
          </div>

          <div className="switches">
            <label className="checkline" title="关闭后这个账号不会参与自动选择，也不能普通切换">
              <input type="checkbox" checked={enabledDraft} onChange={(event) => setEnabledDraft(event.target.checked)} />
              启用账号
            </label>
            <label className="checkline" title="忽略冷却/禁用/Codex 正在运行等保护，强制写入全局 auth.json">
              <input type="checkbox" checked={forceSwitch} onChange={(event) => setForceSwitch(event.target.checked)} />
              强制切换
            </label>
          </div>

          <div className="action-row">
            <button className="icon-button" onClick={() => void saveQuota()} disabled={!selectedProfile || busy} title="保存选中账号的限额、冷却分钟、优先级和启用状态">
              <ShieldCheck size={17} />
              保存规则
            </button>
            <button className="icon-button" onClick={() => void probeSelected()} disabled={!selectedProfile || busy} title="用选中账号 token 探测 5 小时/1 周剩余额度">
              <Gauge size={17} />
              探测额度
            </button>
            <button className="icon-button primary" onClick={() => void switchSelected()} disabled={!selectedProfile || busy} title="把选中账号写入全局 auth.json；如果 Codex 正在运行，普通切换会被保护拦截">
              <Zap size={17} />
              切换
            </button>
            <button className="icon-button" onClick={() => void autoSwitch()} disabled={!store?.profiles.length || busy} title="按可用额度、优先级、最近使用时间自动挑一个账号并切换">
              <CheckCircle2 size={17} />
              自动选择
            </button>
          </div>
          </div>
        </div>
      </section>

      <section className="migration-band">
        <div className="migration-copy">
          <h2>一键迁移</h2>
          <p>导出所有账号 profile、规则和可迁移配置；换电脑后导入这个加密包即可恢复账号列表。</p>
        </div>
        <div className="migration-controls">
          <input
            type="password"
            placeholder="迁移包口令（可留空明文导出/导入）"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            title="留空会导出/导入明文 zip；填写口令会使用加密迁移包，口令至少 8 位"
          />
          {passwordTooShort && (
            <span className="field-warning">口令至少 8 位；留空则使用明文 zip</span>
          )}
          <label className="checkline" title="导出迁移包时包含 sessions、对话索引和本地日志，包会更大">
            <input
              type="checkbox"
              checked={includeConversations}
              onChange={(event) => setIncludeConversations(event.target.checked)}
            />
            导出对话记录
          </label>
          <label className="checkline" title="导入迁移包时同时恢复对话记录；默认只恢复账号和配置">
            <input
              type="checkbox"
              checked={restoreConversations}
              onChange={(event) => setRestoreConversations(event.target.checked)}
            />
            导入时恢复对话
          </label>
          <button className="icon-button primary" onClick={() => void exportBundle()} disabled={busy || passwordTooShort} title="把所有账号 profile、规则和可迁移配置导出；口令为空时是明文 zip，有口令时是加密包">
            <Download size={17} />
            导出全部账号
          </button>
          <button className="icon-button" onClick={() => void importBundle()} disabled={busy || passwordTooShort} title="选择明文 zip 或加密迁移包；加密包需要输入口令，明文 zip 可留空">
            <Upload size={17} />
            导入迁移包
          </button>
          <button className="icon-button" onClick={() => void restoreBackup()} disabled={busy} title="恢复最近一次切换账号前自动备份的 auth.json">
            <RotateCcw size={17} />
            恢复备份
          </button>
        </div>
      </section>

      <section className="bottom-grid">
        <div className="panel">
          <div className="panel-header">
            <div>
              <h2>迁移清单</h2>
              <p>机器绑定文件会自动排除</p>
            </div>
            <Archive size={22} />
          </div>
          <div className="list-columns">
            <div>
              <h3>默认迁移</h3>
              {(scan?.migratable || ["config.toml", "rules", "memories"]).map((item) => (
                <span className="tag" key={item}>{item}</span>
              ))}
            </div>
            <div>
              <h3>永不迁移</h3>
              {(scan?.excluded || ["installation_id", "cap_sid", ".sandbox"]).map((item) => (
                <span className="tag danger" key={item}>{item}</span>
              ))}
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-header">
            <div>
              <h2>操作记录</h2>
              <p>最近 100 条</p>
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

function StatusPill({ ok, text }: { ok: boolean; text: string }) {
  return <span className={`status-pill ${ok ? "ok" : "muted"}`}>{text}</span>;
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function formatUsage(used?: number, limit?: number) {
  const shownUsed = used ?? 0;
  const shownLimit = limit && limit > 0 ? String(limit) : "不限";
  return `${shownUsed}/${shownLimit}`;
}

function formatLimitChip(item: DetectedLimit) {
  const label = item.label || item.window;
  if (item.remainingPercent !== undefined) {
    return `${label}: 剩余 ${item.remainingPercent}%${item.resetAt ? ` ${formatReset(item.resetAt)}` : ""}`;
  }
  if (item.usedPercent !== undefined) {
    return `${label}: 已用 ${item.usedPercent}%${item.resetAt ? ` ${formatReset(item.resetAt)}` : ""}`;
  }
  return `${label}: ${formatUsage(item.used, item.limit)}`;
}

function quotaSummary(profile: Profile) {
  const items = profile.usage.detectedLimits || [];
  if (items.length > 0) {
    return items
      .slice(0, 2)
      .map((item) => {
        const label = item.label || item.window;
        if (item.remainingPercent !== undefined) return `${label} ${item.remainingPercent}%`;
        if (item.usedPercent !== undefined) return `${label} 已用${item.usedPercent}%`;
        return `${label} ${formatUsage(item.used, item.limit)}`;
      })
      .join(" / ");
  }
  if (profile.usage.detectedSummary) return profile.usage.detectedSummary.replace(/^unparsed:\s*/, "").slice(0, 36);
  return `${formatUsage(profile.usage.hourlyUsed, profile.quotaRule.hourlyLimit)} / ${formatUsage(profile.usage.dailyUsed, profile.quotaRule.dailyLimit)}`;
}

function isConversationFile(path: string) {
  const first = path.split("/")[0];
  return [
    "sessions",
    "session_index.jsonl",
    "logs_2.sqlite",
    "logs_2.sqlite-shm",
    "logs_2.sqlite-wal",
    "state_5.sqlite",
    "state_5.sqlite-shm",
    "state_5.sqlite-wal"
  ].includes(first);
}

function parseOptionalNumber(value: string) {
  if (value === "") return undefined;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : undefined;
}

function normalizeNumber(value?: number) {
  return value && value > 0 ? value : undefined;
}

function isCooling(profile: Profile) {
  if (!profile.cooldownUntil) return false;
  return new Date(profile.cooldownUntil).getTime() > Date.now();
}

function accountState(profile: Profile) {
  if (!profile.enabled) return "禁用";
  if (isCooling(profile)) return "冷却";
  if (profile.usage.lastError) return "探测失败";
  return "可用";
}

function tokenState(profile: Profile) {
  const error = profile.usage.lastTokenRefreshError || profile.usage.lastError || "";
  if (error.includes("token_invalidated")) return "认证失效";
  if (error.includes("refresh_token_reused")) return "需重登";
  if (profile.usage.lastTokenRefreshStatus === "ok") return "已保活";
  if (profile.usage.lastTokenRefreshStatus === "error") return "保活失败";
  if (profile.summary.accessTokenExp && profile.summary.accessTokenExp * 1000 <= Date.now()) return "已过期";
  return "正常";
}

function friendlyProbeSummary(profile?: Profile) {
  if (!profile) return "暂无可解析额度数据";
  const summary = profile.usage.detectedSummary || "";
  const error = profile.usage.lastError || profile.usage.lastTokenRefreshError || "";
  if (summary.includes("token_invalidated") || error.includes("token_invalidated")) {
    return "认证已失效，需要重新登录该账号";
  }
  if (summary.includes("refresh_token_reused") || error.includes("refresh_token_reused")) {
    return "refresh token 已被其他会话使用，需要重新登录";
  }
  return summary || "暂无可解析额度数据";
}

function profileScore(profile: Profile) {
  const hourlyRemaining = profile.quotaRule.hourlyLimit
    ? profile.quotaRule.hourlyLimit - profile.usage.hourlyUsed
    : 10000;
  const dailyRemaining = profile.quotaRule.dailyLimit
    ? profile.quotaRule.dailyLimit - profile.usage.dailyUsed
    : 10000;
  const lastUsedPenalty = profile.usage.lastUsedAt ? new Date(profile.usage.lastUsedAt).getTime() / 1000000000 : 0;
  return Math.min(hourlyRemaining, dailyRemaining) * 10 + profile.priority - lastUsedPenalty;
}

function formatDate(value?: string) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function formatReset(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  return sameDay
    ? date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString([], { month: "numeric", day: "numeric" });
}

function formatUnix(value?: number) {
  if (!value) return "-";
  return new Date(value * 1000).toLocaleString();
}
