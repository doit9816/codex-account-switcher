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
  Trash2,
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

type LanguageSetting = "system" | "zh-CN" | "en" | "zh-TW";
type ResolvedLanguage = Exclude<LanguageSetting, "system">;

const emptyQuota: QuotaRule = {
  hourlyLimit: undefined,
  dailyLimit: undefined,
  cooldownMinutes: 180
};

const languageLabels: Record<LanguageSetting, string> = {
  system: "跟随系统",
  "zh-CN": "简体中文",
  en: "English",
  "zh-TW": "繁體中文"
};

const messages = {
  "zh-CN": {
    appSubtitle: "多账号额度观察、全局切换和一键加密迁移",
    dashboard: "仪表板",
    settings: "设置",
    refresh: "刷新",
    closeNotice: "关闭提示",
    language: "语言",
    followSystem: "跟随系统",
    codexDir: "Codex 目录",
    scan: "扫描",
    openDir: "打开目录",
    dirExists: "目录存在",
    notScanned: "未扫描",
    authFound: "发现 auth.json",
    authMissing: "未发现 auth.json",
    proxyEnabled: "探测接口走代理",
    proxyPlaceholder: "http://127.0.0.1:7890 或 socks5://127.0.0.1:7890",
    saveProxy: "保存代理",
    proxyHint: "影响额度探测和 token 保活接口，不修改系统代理。",
    backgroundTokenKeepalive: "其他账号 token 保活",
    keepaliveInterval: "保活间隔秒",
    refreshThreshold: "提前刷新秒",
    autoProbeQuota: "自动刷新额度",
    probeInterval: "额度间隔秒",
    saveAutoRefresh: "保存自动刷新",
    keepaliveNow: "立即保活",
    autoHint: "默认到期后刷新；提前刷新可能被服务端拒绝。其他账号保活只更新各自加密 profile。",
    accounts: "账号",
    profiles: "个 profile",
    searchPlaceholder: "搜索账号/额度/状态",
    importAlias: "导入别名",
    importCurrent: "导入当前",
    account: "账号",
    plan: "计划",
    state: "状态",
    quota: "额度",
    priority: "优先级",
    accessExpires: "Access 过期",
    token: "Token",
    probe: "探测",
    actions: "操作",
    switch: "切换",
    deleteAccount: "删除账号",
    noMatchedAccounts: "没有匹配的账号",
    unknownAccount: "未知账号",
    notProbed: "未探测",
    selectedRules: "选中账号规则",
    selectAccount: "选择一个账号",
    currentGlobal: "当前全局账号",
    currentUsing: "当前",
    notWrittenGlobal: "未写入全局",
    deleteAccountConfirm: "只删除工具内保存的账号 profile，不会删除当前 ~/.codex/auth.json。确定删除？",
    deleteCurrentAccountConfirm: "这个账号正在写入全局使用。删除只会移除工具内保存的 profile，不会删除当前 ~/.codex/auth.json。确定删除？",
    deletedAccount: "已删除账号",
    probeSummary: "探测摘要",
    noParsedQuota: "暂无可解析额度数据",
    hourlyLimit: "每小时限额",
    dailyLimit: "每天限额",
    cooldownMinutes: "冷却分钟",
    enableAccount: "启用账号",
    forceSwitch: "强制切换",
    saveRules: "保存规则",
    probeQuota: "探测额度",
    autoSelect: "自动选择",
    oneClickMigration: "一键迁移",
    migrationIntro: "导出所有账号 profile、规则和可迁移配置；换电脑后导入这个加密包即可恢复账号列表。",
    bundlePassword: "迁移包口令（可留空明文导出/导入）",
    passwordWarning: "口令至少 8 位；留空则使用明文 zip",
    exportConversations: "导出对话记录",
    restoreConversations: "导入时恢复对话",
    exportAll: "导出全部账号",
    importBundle: "导入迁移包",
    restoreBackup: "恢复备份",
    migrationList: "迁移清单",
    machineBoundExcluded: "机器绑定文件会自动排除",
    defaultMigration: "默认迁移",
    neverMigrate: "永不迁移",
    operationLog: "操作记录",
    latest100: "最近 100 条",
    unlimited: "不限",
    disabled: "禁用",
    cooling: "冷却",
    probeFailed: "探测失败",
    available: "可用",
    authInvalid: "认证失效",
    reloginRequired: "需重登",
    keptAlive: "已保活",
    keepaliveFailed: "保活失败",
    expired: "已过期",
    normal: "正常",
    remaining: "剩余",
    used: "已用",
    scannedCodex: "已扫描 Codex 目录",
    openedCodex: "已打开 Codex 目录",
    importedCurrent: "已导入当前账号",
    savedRules: "已保存账号规则",
    savedProxy: "已保存探测代理设置",
    disabledProxy: "已关闭探测代理",
    savedAuto: "已保存自动刷新设置",
    noAvailableAccount: "没有可用账号：全部被禁用或仍在冷却中",
    autoSelected: "已自动选择",
    passwordTooShortExport: "迁移包口令至少 8 位；如需明文导出请清空口令",
    passwordTooShortImport: "迁移包口令至少 8 位；明文 zip 导入请清空口令",
    exportTitle: "导出全部 Codex 账号",
    importTitle: "导入 Codex 账号迁移包",
    encryptedExported: "已加密导出",
    plaintextExported: "已明文导出",
    accountCount: "个账号",
    configFiles: "个配置文件",
    conversationFiles: "对话文件",
    importedAccounts: "已导入",
    restoredFiles: "恢复",
    bundleAccountCount: "包内账号数",
    restoredBackup: "已恢复最近一次 auth.json 备份",
    tokenKeepaliveDone: "token 保活完成",
    refreshed: "刷新",
    skipped: "跳过",
    failed: "失败",
    fiveHours: "5小时",
    oneWeek: "1周",
    tokenInvalidatedHint: "认证已失效，需要重新登录该账号",
    refreshTokenReusedHint: "refresh token 已被其他会话使用，需要重新登录"
  },
  en: {
    appSubtitle: "Multi-account usage overview, global switching, and encrypted migration",
    dashboard: "Dashboard",
    settings: "Settings",
    refresh: "Refresh",
    closeNotice: "Close notice",
    language: "Language",
    followSystem: "System",
    codexDir: "Codex directory",
    scan: "Scan",
    openDir: "Open directory",
    dirExists: "Directory exists",
    notScanned: "Not scanned",
    authFound: "auth.json found",
    authMissing: "auth.json missing",
    proxyEnabled: "Proxy probe APIs",
    proxyPlaceholder: "http://127.0.0.1:7890 or socks5://127.0.0.1:7890",
    saveProxy: "Save proxy",
    proxyHint: "Affects usage probe and token keepalive requests only. System proxy is unchanged.",
    backgroundTokenKeepalive: "Other account token keepalive",
    keepaliveInterval: "Keepalive interval seconds",
    refreshThreshold: "Refresh threshold seconds",
    autoProbeQuota: "Auto refresh quota",
    probeInterval: "Quota interval seconds",
    saveAutoRefresh: "Save auto refresh",
    keepaliveNow: "Keepalive now",
    autoHint: "Default is refresh after expiry. Early refresh can be rejected. Other-account keepalive only updates encrypted profiles.",
    accounts: "Accounts",
    profiles: "profiles",
    searchPlaceholder: "Search account / quota / state",
    importAlias: "Import alias",
    importCurrent: "Import current",
    account: "Account",
    plan: "Plan",
    state: "State",
    quota: "Quota",
    priority: "Priority",
    accessExpires: "Access expires",
    token: "Token",
    probe: "Probe",
    actions: "Actions",
    switch: "Switch",
    deleteAccount: "Delete account",
    noMatchedAccounts: "No matching accounts",
    unknownAccount: "Unknown account",
    notProbed: "Not probed",
    selectedRules: "Selected account rules",
    selectAccount: "Select an account",
    currentGlobal: "Current global account",
    currentUsing: "Current",
    notWrittenGlobal: "Not written globally",
    deleteAccountConfirm: "This only deletes the saved profile in this tool. It will not delete the current ~/.codex/auth.json. Delete it?",
    deleteCurrentAccountConfirm: "This account is currently written globally. Deleting only removes the saved profile in this tool and will not delete ~/.codex/auth.json. Delete it?",
    deletedAccount: "Account deleted",
    probeSummary: "Probe summary",
    noParsedQuota: "No parsed quota data",
    hourlyLimit: "Hourly limit",
    dailyLimit: "Daily limit",
    cooldownMinutes: "Cooldown minutes",
    enableAccount: "Enable account",
    forceSwitch: "Force switch",
    saveRules: "Save rules",
    probeQuota: "Probe quota",
    autoSelect: "Auto select",
    oneClickMigration: "One-click migration",
    migrationIntro: "Export all account profiles, rules, and migratable settings. Import the bundle on a new computer to restore the account list.",
    bundlePassword: "Bundle password (empty for plain zip)",
    passwordWarning: "Password must be at least 8 characters; empty uses plain zip",
    exportConversations: "Export conversations",
    restoreConversations: "Restore conversations",
    exportAll: "Export all accounts",
    importBundle: "Import bundle",
    restoreBackup: "Restore backup",
    migrationList: "Migration list",
    machineBoundExcluded: "Machine-bound files are excluded automatically",
    defaultMigration: "Default migration",
    neverMigrate: "Never migrate",
    operationLog: "Operation log",
    latest100: "Latest 100",
    unlimited: "Unlimited",
    disabled: "Disabled",
    cooling: "Cooling",
    probeFailed: "Probe failed",
    available: "Available",
    authInvalid: "Auth invalid",
    reloginRequired: "Relogin required",
    keptAlive: "Kept alive",
    keepaliveFailed: "Keepalive failed",
    expired: "Expired",
    normal: "Normal",
    remaining: "Remaining",
    used: "Used",
    scannedCodex: "Codex directory scanned",
    openedCodex: "Codex directory opened",
    importedCurrent: "Current account imported",
    savedRules: "Account rules saved",
    savedProxy: "Probe proxy settings saved",
    disabledProxy: "Probe proxy disabled",
    savedAuto: "Auto refresh settings saved",
    noAvailableAccount: "No available accounts: all are disabled or cooling down",
    autoSelected: "Auto selected",
    passwordTooShortExport: "Bundle password must be at least 8 characters; clear it for plain export",
    passwordTooShortImport: "Bundle password must be at least 8 characters; clear it for plain zip import",
    exportTitle: "Export all Codex accounts",
    importTitle: "Import Codex account bundle",
    encryptedExported: "Encrypted export complete",
    plaintextExported: "Plain export complete",
    accountCount: "accounts",
    configFiles: "config files",
    conversationFiles: "conversation files",
    importedAccounts: "Imported",
    restoredFiles: "restored files",
    bundleAccountCount: "bundle accounts",
    restoredBackup: "Latest auth.json backup restored",
    tokenKeepaliveDone: "Token keepalive complete",
    refreshed: "refreshed",
    skipped: "skipped",
    failed: "failed",
    fiveHours: "5h",
    oneWeek: "1w",
    tokenInvalidatedHint: "Authentication is invalid. Sign in to this account again.",
    refreshTokenReusedHint: "Refresh token was used by another session. Sign in again."
  },
  "zh-TW": {
    appSubtitle: "多帳號額度觀察、全域切換和一鍵加密遷移",
    dashboard: "儀表板",
    settings: "設定",
    refresh: "重新整理",
    closeNotice: "關閉提示",
    language: "語言",
    followSystem: "跟隨系統",
    codexDir: "Codex 目錄",
    scan: "掃描",
    openDir: "開啟目錄",
    dirExists: "目錄存在",
    notScanned: "未掃描",
    authFound: "發現 auth.json",
    authMissing: "未發現 auth.json",
    proxyEnabled: "探測介面走代理",
    proxyPlaceholder: "http://127.0.0.1:7890 或 socks5://127.0.0.1:7890",
    saveProxy: "儲存代理",
    proxyHint: "影響額度探測和 token 保活介面，不修改系統代理。",
    backgroundTokenKeepalive: "其他帳號 token 保活",
    keepaliveInterval: "保活間隔秒",
    refreshThreshold: "提前刷新秒",
    autoProbeQuota: "自動刷新額度",
    probeInterval: "額度間隔秒",
    saveAutoRefresh: "儲存自動刷新",
    keepaliveNow: "立即保活",
    autoHint: "預設到期後刷新；提前刷新可能被服務端拒絕。其他帳號保活只更新各自加密 profile。",
    accounts: "帳號",
    profiles: "個 profile",
    searchPlaceholder: "搜尋帳號/額度/狀態",
    importAlias: "匯入別名",
    importCurrent: "匯入目前",
    account: "帳號",
    plan: "方案",
    state: "狀態",
    quota: "額度",
    priority: "優先級",
    accessExpires: "Access 過期",
    token: "Token",
    probe: "探測",
    actions: "操作",
    switch: "切換",
    deleteAccount: "刪除帳號",
    noMatchedAccounts: "沒有符合的帳號",
    unknownAccount: "未知帳號",
    notProbed: "未探測",
    selectedRules: "選中帳號規則",
    selectAccount: "選擇一個帳號",
    currentGlobal: "目前全域帳號",
    currentUsing: "目前",
    notWrittenGlobal: "未寫入全域",
    deleteAccountConfirm: "只刪除工具內儲存的帳號 profile，不會刪除目前 ~/.codex/auth.json。確定刪除？",
    deleteCurrentAccountConfirm: "這個帳號正在寫入全域使用。刪除只會移除工具內儲存的 profile，不會刪除目前 ~/.codex/auth.json。確定刪除？",
    deletedAccount: "已刪除帳號",
    probeSummary: "探測摘要",
    noParsedQuota: "暫無可解析額度資料",
    hourlyLimit: "每小時限額",
    dailyLimit: "每天限額",
    cooldownMinutes: "冷卻分鐘",
    enableAccount: "啟用帳號",
    forceSwitch: "強制切換",
    saveRules: "儲存規則",
    probeQuota: "探測額度",
    autoSelect: "自動選擇",
    oneClickMigration: "一鍵遷移",
    migrationIntro: "匯出所有帳號 profile、規則和可遷移設定；換電腦後匯入這個加密包即可恢復帳號列表。",
    bundlePassword: "遷移包口令（可留空明文匯出/匯入）",
    passwordWarning: "口令至少 8 位；留空則使用明文 zip",
    exportConversations: "匯出對話記錄",
    restoreConversations: "匯入時恢復對話",
    exportAll: "匯出全部帳號",
    importBundle: "匯入遷移包",
    restoreBackup: "恢復備份",
    migrationList: "遷移清單",
    machineBoundExcluded: "機器綁定檔案會自動排除",
    defaultMigration: "預設遷移",
    neverMigrate: "永不遷移",
    operationLog: "操作記錄",
    latest100: "最近 100 條",
    unlimited: "不限",
    disabled: "停用",
    cooling: "冷卻",
    probeFailed: "探測失敗",
    available: "可用",
    authInvalid: "認證失效",
    reloginRequired: "需重登",
    keptAlive: "已保活",
    keepaliveFailed: "保活失敗",
    expired: "已過期",
    normal: "正常",
    remaining: "剩餘",
    used: "已用",
    scannedCodex: "已掃描 Codex 目錄",
    openedCodex: "已開啟 Codex 目錄",
    importedCurrent: "已匯入目前帳號",
    savedRules: "已儲存帳號規則",
    savedProxy: "已儲存探測代理設定",
    disabledProxy: "已關閉探測代理",
    savedAuto: "已儲存自動刷新設定",
    noAvailableAccount: "沒有可用帳號：全部被停用或仍在冷卻中",
    autoSelected: "已自動選擇",
    passwordTooShortExport: "遷移包口令至少 8 位；如需明文匯出請清空口令",
    passwordTooShortImport: "遷移包口令至少 8 位；明文 zip 匯入請清空口令",
    exportTitle: "匯出全部 Codex 帳號",
    importTitle: "匯入 Codex 帳號遷移包",
    encryptedExported: "已加密匯出",
    plaintextExported: "已明文匯出",
    accountCount: "個帳號",
    configFiles: "個設定檔",
    conversationFiles: "對話檔案",
    importedAccounts: "已匯入",
    restoredFiles: "恢復",
    bundleAccountCount: "包內帳號數",
    restoredBackup: "已恢復最近一次 auth.json 備份",
    tokenKeepaliveDone: "token 保活完成",
    refreshed: "刷新",
    skipped: "跳過",
    failed: "失敗",
    fiveHours: "5小時",
    oneWeek: "1週",
    tokenInvalidatedHint: "認證已失效，需要重新登入該帳號",
    refreshTokenReusedHint: "refresh token 已被其他會話使用，需要重新登入"
  }
} as const;

type I18n = Record<keyof typeof messages["zh-CN"], string>;

function resolveSystemLanguage(): ResolvedLanguage {
  const language = navigator.language.toLowerCase();
  if (language.includes("tw") || language.includes("hk") || language.includes("hant")) return "zh-TW";
  if (language.startsWith("zh")) return "zh-CN";
  return "en";
}

function resolveLanguage(setting: LanguageSetting): ResolvedLanguage {
  return setting === "system" ? resolveSystemLanguage() : setting;
}

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
  const currentGlobalProfileId = useMemo(() => {
    const currentAuth = scan?.currentAuth;
    if (currentAuth) {
      const matched = store?.profiles.find((profile) => authSummariesMatch(profile.summary, currentAuth));
      if (matched) return matched.id;
    }
    return store?.settings.currentProfileId;
  }, [scan?.currentAuth, store?.profiles, store?.settings.currentProfileId]);
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
        accountState(profile, t),
        quotaSummary(profile, t),
        tokenState(profile, t),
        currentGlobalProfileId === profile.id ? t.currentUsing : ""
      ];
      return values.some((value) => String(value || "").toLowerCase().includes(query));
    });
  }, [store?.profiles, accountFilter, currentGlobalProfileId, t]);
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
    localStorage.setItem("codex-account-switcher-language", languageSetting);
  }, [languageSetting]);

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
    }, t.scannedCodex);
  }

  async function openCodexHome() {
    await run(async () => {
      await invoke("open_codex_home", {
        codexHome: codexHome || undefined
      });
    }, t.openedCodex);
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
    }, t.importedCurrent);
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
    }, t.savedRules);
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
        text: `${t.tokenKeepaliveDone}: ${t.refreshed} ${result.refreshed}, ${t.skipped} ${result.skipped}, ${t.failed} ${result.failed}`
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
      setNotice({ kind: "warn", text: t.noAvailableAccount });
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
      setNotice({ kind: result.codexRunning ? "warn" : "ok", text: `${t.autoSelected} ${candidate.alias}: ${result.message}` });
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
        includeConversations
      });
      const conversationCount = manifest.files.filter((file) => isConversationFile(file.path)).length;
      setNotice({
        kind: password ? "ok" : "warn",
        text: `${password ? t.encryptedExported : t.plaintextExported}: ${manifest.profileCount} ${t.accountCount}, ${manifest.files.length} ${t.configFiles}, ${t.conversationFiles} ${conversationCount}`
      });
      return manifest;
    });
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

      {activePage === "settings" ? (
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
        <button className="icon-button" onClick={() => void saveAutoSettings()} disabled={busy} title={t.saveAutoRefresh}>
          <RefreshCcw size={17} />
          {t.saveAutoRefresh}
        </button>
        <button className="icon-button" onClick={() => void refreshOtherProfileTokensNow()} disabled={busy} title={t.keepaliveNow}>
          <KeyRound size={17} />
          {t.keepaliveNow}
        </button>
        <span className="proxy-hint">{t.autoHint}</span>
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
              <input
                className="alias-input"
                placeholder={t.searchPlaceholder}
                value={accountFilter}
                onChange={(event) => setAccountFilter(event.target.value)}
                title={t.searchPlaceholder}
              />
              <input
                className="alias-input"
                placeholder={t.importAlias}
                value={alias}
                onChange={(event) => setAlias(event.target.value)}
                title={t.importAlias}
              />
              <button className="icon-button" onClick={() => void importCurrentAuth()} disabled={busy} title={t.importCurrent}>
                <KeyRound size={17} />
                {t.importCurrent}
              </button>
            </div>
          </div>

          <div className="account-table" role="table">
            <div className="account-row header" role="row">
              <span>{t.account}</span>
              <span>{t.plan}</span>
              <span>{t.state}</span>
              <span>{t.quota}</span>
              <span>{t.priority}</span>
              <span>{t.accessExpires}</span>
              <span>{t.token}</span>
              <span>{t.probe}</span>
              <span>{t.actions}</span>
            </div>
            {filteredProfiles.map((profile) => {
              const isCurrent = currentGlobalProfileId === profile.id;
              return (
              <div
                key={profile.id}
                className={`account-row ${selectedId === profile.id ? "selected" : ""} ${isCurrent ? "current" : ""}`}
                onClick={() => setSelectedId(profile.id)}
                role="row"
              >
                <span>
                  <strong>
                    {profile.alias}
                    {isCurrent && <em className="current-badge">{t.currentUsing}</em>}
                  </strong>
                  <small>{profile.summary.email || profile.summary.accountId || t.unknownAccount}</small>
                </span>
                <span>{profile.summary.plan || profile.summary.authMode || "-"}</span>
                <span>
                  <StatusPill ok={profile.enabled && !isCooling(profile)} text={accountState(profile, t)} />
                </span>
                <span className="quota-cell">{quotaSummary(profile, t)}</span>
                <span>{profile.priority}</span>
                <span>{formatUnix(profile.summary.accessTokenExp)}</span>
                <span>{tokenState(profile, t)}</span>
                <span>{profile.usage.lastProbeStatus || t.notProbed}</span>
                <span className="row-actions">
                  <button
                    className="mini-button"
                    onClick={(event) => {
                      event.stopPropagation();
                      void probeProfile(profile.id);
                    }}
                    disabled={busy}
                    title={t.probeQuota}
                  >
                    {t.probe}
                  </button>
                  <button
                    className="mini-button primary"
                    onClick={(event) => {
                      event.stopPropagation();
                      void switchProfile(profile.id);
                    }}
                    disabled={busy}
                    title={t.switch}
                  >
                    {t.switch}
                  </button>
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
                </span>
              </div>
              );
            })}
            {filteredProfiles.length === 0 && (
              <div className="account-empty">{t.noMatchedAccounts}</div>
            )}
          </div>

          <div className="inline-detail">
            <div className="inline-detail-head">
              <div>
                <h3>{t.selectedRules}</h3>
                <p>{selectedProfile?.summary.email || selectedProfile?.alias || t.selectAccount}</p>
              </div>
              <StatusPill
                ok={currentGlobalProfileId === selectedProfile?.id}
                text={currentGlobalProfileId === selectedProfile?.id ? t.currentGlobal : t.notWrittenGlobal}
              />
            </div>
            <div className="probe-box compact-probe">
              <div>
                <span>{t.probeSummary}</span>
                <strong>{friendlyProbeSummary(selectedProfile, t)}</strong>
              </div>
              <div className="detected-limits">
                {(selectedProfile?.usage.detectedLimits || []).map((item, index) => (
                  <span className="limit-chip" key={`${item.window}-${item.label || ""}-${index}`}>
                    {formatLimitChip(item, t)}
                    {item.remaining != null ? ` ${t.remaining} ${item.remaining}` : ""}
                  </span>
                ))}
              </div>
            </div>
            <div className="form-grid">
            <label>
              {t.hourlyLimit}
              <input
                type="number"
                min={0}
                value={quotaDraft.hourlyLimit ?? ""}
                onChange={(event) => setQuotaDraft({ ...quotaDraft, hourlyLimit: parseOptionalNumber(event.target.value) })}
                title={t.hourlyLimit}
              />
            </label>
            <label>
              {t.dailyLimit}
              <input
                type="number"
                min={0}
                value={quotaDraft.dailyLimit ?? ""}
                onChange={(event) => setQuotaDraft({ ...quotaDraft, dailyLimit: parseOptionalNumber(event.target.value) })}
                title={t.dailyLimit}
              />
            </label>
            <label>
              {t.cooldownMinutes}
              <input
                type="number"
                min={1}
                value={quotaDraft.cooldownMinutes}
                onChange={(event) => setQuotaDraft({ ...quotaDraft, cooldownMinutes: Number(event.target.value) || 180 })}
                title={t.cooldownMinutes}
              />
            </label>
            <label>
              {t.priority}
              <input
                type="number"
                value={priorityDraft}
                onChange={(event) => setPriorityDraft(Number(event.target.value) || 0)}
                title={t.priority}
              />
            </label>
          </div>

          <div className="switches">
            <label className="checkline" title={t.enableAccount}>
              <input type="checkbox" checked={enabledDraft} onChange={(event) => setEnabledDraft(event.target.checked)} />
              {t.enableAccount}
            </label>
            <label className="checkline" title={t.forceSwitch}>
              <input type="checkbox" checked={forceSwitch} onChange={(event) => setForceSwitch(event.target.checked)} />
              {t.forceSwitch}
            </label>
          </div>

          <div className="action-row">
            <button className="icon-button" onClick={() => void saveQuota()} disabled={!selectedProfile || busy} title={t.saveRules}>
              <ShieldCheck size={17} />
              {t.saveRules}
            </button>
            <button className="icon-button" onClick={() => void probeSelected()} disabled={!selectedProfile || busy} title={t.probeQuota}>
              <Gauge size={17} />
              {t.probeQuota}
            </button>
            <button className="icon-button primary" onClick={() => void switchSelected()} disabled={!selectedProfile || busy} title={t.switch}>
              <Zap size={17} />
              {t.switch}
            </button>
            <button className="icon-button" onClick={() => void autoSwitch()} disabled={!store?.profiles.length || busy} title={t.autoSelect}>
              <CheckCircle2 size={17} />
              {t.autoSelect}
            </button>
          </div>
          </div>
        </div>
      </section>

      <section className="migration-band">
        <div className="migration-copy">
          <h2>{t.oneClickMigration}</h2>
          <p>{t.migrationIntro}</p>
        </div>
        <div className="migration-controls">
          <input
            type="password"
            placeholder={t.bundlePassword}
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            title={t.bundlePassword}
          />
          {passwordTooShort && (
            <span className="field-warning">{t.passwordWarning}</span>
          )}
          <label className="checkline" title={t.exportConversations}>
            <input
              type="checkbox"
              checked={includeConversations}
              onChange={(event) => setIncludeConversations(event.target.checked)}
            />
            {t.exportConversations}
          </label>
          <label className="checkline" title={t.restoreConversations}>
            <input
              type="checkbox"
              checked={restoreConversations}
              onChange={(event) => setRestoreConversations(event.target.checked)}
            />
            {t.restoreConversations}
          </label>
          <button className="icon-button primary" onClick={() => void exportBundle()} disabled={busy || passwordTooShort} title={t.exportAll}>
            <Download size={17} />
            {t.exportAll}
          </button>
          <button className="icon-button" onClick={() => void importBundle()} disabled={busy || passwordTooShort} title={t.importBundle}>
            <Upload size={17} />
            {t.importBundle}
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

function formatUsage(used?: number, limit?: number, t: I18n = messages["zh-CN"]) {
  const shownUsed = used ?? 0;
  const shownLimit = limit && limit > 0 ? String(limit) : t.unlimited;
  return `${shownUsed}/${shownLimit}`;
}

function formatLimitChip(item: DetectedLimit, t: I18n) {
  const label = localizedLimitLabel(item.label || item.window, t);
  if (item.remainingPercent !== undefined) {
    return `${label}: ${t.remaining} ${item.remainingPercent}%${item.resetAt ? ` ${formatReset(item.resetAt)}` : ""}`;
  }
  if (item.usedPercent !== undefined) {
    return `${label}: ${t.used} ${item.usedPercent}%${item.resetAt ? ` ${formatReset(item.resetAt)}` : ""}`;
  }
  return `${label}: ${formatUsage(item.used, item.limit, t)}`;
}

function quotaSummary(profile: Profile, t: I18n) {
  const items = profile.usage.detectedLimits || [];
  if (items.length > 0) {
    return items
      .slice(0, 2)
      .map((item) => {
        const label = localizedLimitLabel(item.label || item.window, t);
        if (item.remainingPercent !== undefined) return `${label} ${item.remainingPercent}%`;
        if (item.usedPercent !== undefined) return `${label} ${t.used}${item.usedPercent}%`;
        return `${label} ${formatUsage(item.used, item.limit, t)}`;
      })
      .join(" / ");
  }
  if (profile.usage.detectedSummary) return localizeDetectedText(profile.usage.detectedSummary.replace(/^unparsed:\s*/, ""), t).slice(0, 36);
  return `${formatUsage(profile.usage.hourlyUsed, profile.quotaRule.hourlyLimit, t)} / ${formatUsage(profile.usage.dailyUsed, profile.quotaRule.dailyLimit, t)}`;
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

function accountState(profile: Profile, t: I18n) {
  if (!profile.enabled) return t.disabled;
  if (isCooling(profile)) return t.cooling;
  if (profile.usage.lastError) return t.probeFailed;
  return t.available;
}

function tokenState(profile: Profile, t: I18n) {
  const error = profile.usage.lastTokenRefreshError || profile.usage.lastError || "";
  if (error.includes("token_invalidated")) return t.authInvalid;
  if (error.includes("refresh_token_reused")) return t.reloginRequired;
  if (profile.usage.lastTokenRefreshStatus === "ok") return t.keptAlive;
  if (profile.usage.lastTokenRefreshStatus === "error") return t.keepaliveFailed;
  if (profile.summary.accessTokenExp && profile.summary.accessTokenExp * 1000 <= Date.now()) return t.expired;
  return t.normal;
}

function authSummariesMatch(left: AuthSummary, right: AuthSummary) {
  if (left.accountId && right.accountId && left.accountId === right.accountId) return true;
  if (left.userId && right.userId && left.userId === right.userId) return true;
  if (left.email && right.email && left.email.toLowerCase() === right.email.toLowerCase()) return true;
  return false;
}

function friendlyProbeSummary(profile: Profile | undefined, t: I18n) {
  if (!profile) return t.noParsedQuota;
  if (profile.usage.detectedLimits?.length) {
    return profile.usage.detectedLimits.slice(0, 2).map((item) => formatLimitChip(item, t)).join("; ");
  }
  const summary = localizeDetectedText(profile.usage.detectedSummary || "", t);
  const error = profile.usage.lastError || profile.usage.lastTokenRefreshError || "";
  if (summary.includes("token_invalidated") || error.includes("token_invalidated")) {
    return t.tokenInvalidatedHint;
  }
  if (summary.includes("refresh_token_reused") || error.includes("refresh_token_reused")) {
    return t.refreshTokenReusedHint;
  }
  return summary || t.noParsedQuota;
}

function localizedLimitLabel(label: string, t: I18n) {
  const normalized = label.toLowerCase().replace(/\s+/g, "");
  if (
    normalized.includes("5小时") ||
    normalized.includes("5小時") ||
    normalized.includes("5hour") ||
    normalized.includes("5-hour") ||
    normalized === "5h"
  ) {
    return t.fiveHours;
  }
  if (
    normalized.includes("1周") ||
    normalized.includes("1週") ||
    normalized.includes("week") ||
    normalized.includes("7day") ||
    normalized === "1w"
  ) {
    return t.oneWeek;
  }
  return label;
}

function localizeDetectedText(text: string, t: I18n) {
  return text
    .replace(/5小时|5小時|5h/gi, t.fiveHours)
    .replace(/1周|1週|1w/gi, t.oneWeek)
    .replace(/剩余|剩餘|remaining/gi, t.remaining)
    .replace(/已用|used/gi, t.used);
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
