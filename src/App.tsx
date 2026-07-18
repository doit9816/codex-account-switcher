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
  HardDriveUpload,
  KeyRound,
  LayoutDashboard,
  Network,
  Pencil,
  Power,
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
  subscriptionActiveStart?: string;
  subscriptionActiveUntil?: string;
  subscriptionLastChecked?: string;
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
  availableResetCount?: number;
};

type RouteHealth = {
  consecutiveFailures: number;
  activeConnections: number;
  lastRouteAt?: string;
  lastStatus?: string;
  lastError?: string;
  cooldownReason?: string;
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
  note: string;
  enabled: boolean;
  priority: number;
  cooldownUntil?: string;
  quotaRule: QuotaRule;
  summary: AuthSummary;
  apiConfig?: {
    providerId: string;
    baseUrl: string;
    model: string;
    wireApi: string;
  };
  usage: UsageStats;
  routeHealth: RouteHealth;
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
    routing: RoutingSettings;
  };
  profiles: Profile[];
  events: AppEvent[];
};

type RoutingSettings = {
  listenHost: string;
  port: number;
  enabled: boolean;
  riskConfirmed: boolean;
  appliedToCodex: boolean;
  mode: "auto" | "fixed";
  fixedProfileId?: string;
  stickyTtlSecs: number;
};

type RoutingLogEntry = {
  ts: string;
  sessionHash?: string;
  profileId?: string;
  alias?: string;
  requestedModel?: string;
  actualModel?: string;
  status: string;
  httpStatus?: number;
  latencyMs: number;
  fallback?: string;
  error?: string;
};

type RoutingStatus = {
  running: boolean;
  baseUrl: string;
  accessKey?: string;
  activeConnections: number;
  settings: RoutingSettings;
  recentLogs: RoutingLogEntry[];
};

type CodexScan = {
  codexHome: string;
  exists: boolean;
  hasAuth: boolean;
  currentAuth?: AuthSummary;
  migratable: string[];
  excluded: string[];
};

type ConfigFileView = {
  path: string;
  exists: boolean;
  content: string;
};

type CodexConfigFiles = {
  codexHome: string;
  authJson: ConfigFileView;
  configToml: ConfigFileView;
};

type OAuthLoginSession = {
  loginId: string;
  authUrl: string;
  callbackUrl: string;
  expiresAt: string;
};

type OAuthEvent = {
  loginId: string;
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
    softwareUpdate: "软件更新",
    currentVersion: "当前版本",
    checkUpdate: "检查更新",
    updateAvailable: "发现新版本",
    updateDate: "发布时间",
    upToDate: "已是最新版本",
    updateNotChecked: "尚未检查更新",
    downloadAndInstall: "下载并安装",
    installingUpdate: "正在下载并安装",
    updateInstalled: "更新已安装，重启后生效",
    relaunchNow: "立即重启",
    updateCheckFailed: "检查更新失败",
    updateInstallFailed: "安装更新失败",
    updateHint: "使用 GitHub Release stable 通道。发现新版本后需要手动确认安装。",
    accounts: "账号",
    profiles: "个 profile",
    searchPlaceholder: "搜索账号/额度/状态",
    importAlias: "导入别名",
    importCurrent: "导入当前",
    addAccount: "添加账号",
    oauthLogin: "OAuth 授权",
    tokenJson: "Token / JSON",
    apiKeyAccount: "API Key",
    importAccount: "导入当前",
    oauthLoginHint: "使用应用内原生 OAuth，在浏览器完成 ChatGPT 授权后自动添加账号。",
    startOAuthLogin: "开始 OAuth 授权",
    oauthImportDone: "我已完成授权，导入当前账号",
    oauthStarting: "正在创建安全授权会话",
    oauthWaiting: "等待浏览器授权",
    oauthExchanging: "正在交换并加密保存 Token",
    oauthTimedOut: "授权会话已超时，请重新开始",
    authorizationUrl: "授权链接",
    copyLink: "复制链接",
    openAgain: "重新打开",
    manualCallback: "手动粘贴回调地址",
    submitCallback: "提交回调",
    retryExchange: "重试 Token 交换",
    cancelOAuth: "取消授权",
    cliFallback: "Codex CLI 备用登录",
    cliFallbackHint: "原生 OAuth 无法使用时，启动官方 codex login，完成后再导入当前账号。",
    startCliLogin: "启动 codex login",
    secondsRemaining: "秒后超时",
    copied: "已复制",
    authJsonPlaceholder: "粘贴完整的 Codex auth.json 内容",
    addFromJson: "添加 Token / JSON 账号",
    accountAdded: "账号已添加",
    apiProvider: "API Provider",
    apiProviderName: "Provider 名称",
    providerId: "Provider ID",
    apiBaseUrl: "API Base URL（可空）",
    apiModel: "模型",
    apiKey: "API Key",
    addApiProvider: "添加 API Provider",
    apiResponsesHint: "Base URL 不填默认使用官方 https://api.openai.com/v1；当前支持 Codex Responses API 兼容接口，Chat Completions 接口需要协议路由。",
    apiProviderAdded: "API Provider 已添加",
    codexConfig: "Codex 配置",
    codexConfigHint: "查看和编辑当前 Codex Home 下的 auth.json 与 config.toml；保存前会校验并自动备份旧文件。",
    loadConfig: "加载配置",
    formatConfig: "格式化",
    saveConfig: "保存配置",
    authJsonConfig: "auth.json（JSON）",
    configTomlConfig: "config.toml（TOML）",
    configMissing: "文件不存在，保存后会创建",
    configLoaded: "Codex 配置已加载",
    configSaved: "Codex 配置已保存",
    configFormatted: "配置格式化完成",
    account: "账号",
    plan: "计划",
    state: "状态",
    quota: "额度",
    priority: "优先级",
    accessExpires: "Access 过期",
    loginValidity: "订阅有效期",
    validityExpired: "已过期",
    pendingPlan: "待探测",
    token: "Token",
    probe: "探测",
    actions: "操作",
    switch: "切换",
    deleteAccount: "删除账号",
    accountNote: "账号备注",
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
    refreshTokenReusedHint: "refresh token 已被其他会话使用，需要重新登录",
    reauthorize: "重新授权",
    editAccount: "编辑账号",
    editAccountInfo: "编辑账号信息",
    accountAlias: "账号别名",
    notePlaceholder: "备注、设备、用途等",
    apiKeyOptional: "API Key（留空不修改）",
    savedProfile: "账号信息已保存",
    usageResets: "可用重置",
    useReset: "使用重置",
    useResetConfirm: "确定使用一次重置吗？这会重置当前适用的 Codex 用量窗口。",
    usageResetDone: "用量已重置"
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
    softwareUpdate: "Software update",
    currentVersion: "Current version",
    checkUpdate: "Check for updates",
    updateAvailable: "Update available",
    updateDate: "Release date",
    upToDate: "You are up to date",
    updateNotChecked: "Not checked yet",
    downloadAndInstall: "Download and install",
    installingUpdate: "Downloading and installing",
    updateInstalled: "Update installed. Relaunch to apply it.",
    relaunchNow: "Relaunch now",
    updateCheckFailed: "Update check failed",
    updateInstallFailed: "Update install failed",
    updateHint: "Uses the GitHub Release stable channel. New versions require manual confirmation before install.",
    accounts: "Accounts",
    profiles: "profiles",
    searchPlaceholder: "Search account / quota / state",
    importAlias: "Import alias",
    importCurrent: "Import current",
    addAccount: "Add account",
    oauthLogin: "OAuth",
    tokenJson: "Token / JSON",
    apiKeyAccount: "API Key",
    importAccount: "Import current",
    oauthLoginHint: "Use native OAuth and automatically add the account after ChatGPT authorization in your browser.",
    startOAuthLogin: "Start OAuth",
    oauthImportDone: "Authorization complete, import account",
    oauthStarting: "Creating a secure authorization session",
    oauthWaiting: "Waiting for browser authorization",
    oauthExchanging: "Exchanging and encrypting tokens",
    oauthTimedOut: "Authorization timed out. Start again.",
    authorizationUrl: "Authorization URL",
    copyLink: "Copy URL",
    openAgain: "Open again",
    manualCallback: "Paste callback URL manually",
    submitCallback: "Submit callback",
    retryExchange: "Retry token exchange",
    cancelOAuth: "Cancel authorization",
    cliFallback: "Codex CLI fallback",
    cliFallbackHint: "If native OAuth is unavailable, run official codex login and then import the current account.",
    startCliLogin: "Run codex login",
    secondsRemaining: "seconds remaining",
    copied: "Copied",
    authJsonPlaceholder: "Paste the complete Codex auth.json content",
    addFromJson: "Add Token / JSON account",
    accountAdded: "Account added",
    apiProvider: "API Provider",
    apiProviderName: "Provider name",
    providerId: "Provider ID",
    apiBaseUrl: "API Base URL (optional)",
    apiModel: "Model",
    apiKey: "API Key",
    addApiProvider: "Add API Provider",
    apiResponsesHint: "Leave Base URL empty to use official https://api.openai.com/v1. Supports Codex Responses API-compatible endpoints; Chat Completions endpoints require protocol routing.",
    apiProviderAdded: "API Provider added",
    codexConfig: "Codex config",
    codexConfigHint: "View and edit auth.json and config.toml under the current Codex Home. Saves validate content and back up old files first.",
    loadConfig: "Load config",
    formatConfig: "Format",
    saveConfig: "Save config",
    authJsonConfig: "auth.json (JSON)",
    configTomlConfig: "config.toml (TOML)",
    configMissing: "File is missing; saving will create it",
    configLoaded: "Codex config loaded",
    configSaved: "Codex config saved",
    configFormatted: "Config formatted",
    account: "Account",
    plan: "Plan",
    state: "State",
    quota: "Quota",
    priority: "Priority",
    accessExpires: "Access expires",
    loginValidity: "Subscription validity",
    validityExpired: "Expired",
    pendingPlan: "Pending",
    token: "Token",
    probe: "Probe",
    actions: "Actions",
    switch: "Switch",
    deleteAccount: "Delete account",
    accountNote: "Account note",
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
    refreshTokenReusedHint: "Refresh token was used by another session. Sign in again.",
    reauthorize: "Reauthorize",
    editAccount: "Edit account",
    editAccountInfo: "Edit account info",
    accountAlias: "Account alias",
    notePlaceholder: "Notes, device, purpose, etc.",
    apiKeyOptional: "API Key (leave blank to keep current)",
    savedProfile: "Account info saved",
    usageResets: "Resets available",
    useReset: "Use reset",
    useResetConfirm: "Use one reset now? This resets the currently eligible Codex usage windows.",
    usageResetDone: "Usage reset"
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
    softwareUpdate: "軟體更新",
    currentVersion: "目前版本",
    checkUpdate: "檢查更新",
    updateAvailable: "發現新版本",
    updateDate: "發布時間",
    upToDate: "已是最新版本",
    updateNotChecked: "尚未檢查更新",
    downloadAndInstall: "下載並安裝",
    installingUpdate: "正在下載並安裝",
    updateInstalled: "更新已安裝，重新啟動後生效",
    relaunchNow: "立即重新啟動",
    updateCheckFailed: "檢查更新失敗",
    updateInstallFailed: "安裝更新失敗",
    updateHint: "使用 GitHub Release stable 通道。發現新版本後需要手動確認安裝。",
    accounts: "帳號",
    profiles: "個 profile",
    searchPlaceholder: "搜尋帳號/額度/狀態",
    importAlias: "匯入別名",
    importCurrent: "匯入目前",
    addAccount: "新增帳號",
    oauthLogin: "OAuth 授權",
    tokenJson: "Token / JSON",
    apiKeyAccount: "API Key",
    importAccount: "匯入目前",
    oauthLoginHint: "使用應用程式內原生 OAuth，在瀏覽器完成 ChatGPT 授權後自動新增帳號。",
    startOAuthLogin: "開始 OAuth 授權",
    oauthImportDone: "我已完成授權，匯入目前帳號",
    oauthStarting: "正在建立安全授權工作階段",
    oauthWaiting: "等待瀏覽器授權",
    oauthExchanging: "正在交換並加密儲存 Token",
    oauthTimedOut: "授權工作階段已逾時，請重新開始",
    authorizationUrl: "授權連結",
    copyLink: "複製連結",
    openAgain: "重新開啟",
    manualCallback: "手動貼上回呼地址",
    submitCallback: "提交回呼",
    retryExchange: "重試 Token 交換",
    cancelOAuth: "取消授權",
    cliFallback: "Codex CLI 備用登入",
    cliFallbackHint: "原生 OAuth 無法使用時，啟動官方 codex login，完成後再匯入目前帳號。",
    startCliLogin: "啟動 codex login",
    secondsRemaining: "秒後逾時",
    copied: "已複製",
    authJsonPlaceholder: "貼上完整的 Codex auth.json 內容",
    addFromJson: "新增 Token / JSON 帳號",
    accountAdded: "帳號已新增",
    apiProvider: "API Provider",
    apiProviderName: "Provider 名稱",
    providerId: "Provider ID",
    apiBaseUrl: "API Base URL（可空）",
    apiModel: "模型",
    apiKey: "API Key",
    addApiProvider: "新增 API Provider",
    apiResponsesHint: "Base URL 不填預設使用官方 https://api.openai.com/v1；目前支援 Codex Responses API 相容端點，Chat Completions 端點需要協議路由。",
    apiProviderAdded: "API Provider 已新增",
    codexConfig: "Codex 配置",
    codexConfigHint: "查看和編輯目前 Codex Home 下的 auth.json 與 config.toml；保存前會校驗並自動備份舊文件。",
    loadConfig: "載入配置",
    formatConfig: "格式化",
    saveConfig: "保存配置",
    authJsonConfig: "auth.json（JSON）",
    configTomlConfig: "config.toml（TOML）",
    configMissing: "文件不存在，保存後會建立",
    configLoaded: "Codex 配置已載入",
    configSaved: "Codex 配置已保存",
    configFormatted: "配置格式化完成",
    account: "帳號",
    plan: "方案",
    state: "狀態",
    quota: "額度",
    priority: "優先級",
    accessExpires: "Access 過期",
    loginValidity: "訂閱有效期",
    validityExpired: "已過期",
    pendingPlan: "待探測",
    token: "Token",
    probe: "探測",
    actions: "操作",
    switch: "切換",
    deleteAccount: "刪除帳號",
    accountNote: "帳號備註",
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
    refreshTokenReusedHint: "refresh token 已被其他會話使用，需要重新登入",
    reauthorize: "重新授權",
    editAccount: "編輯帳號",
    editAccountInfo: "編輯帳號資訊",
    accountAlias: "帳號別名",
    notePlaceholder: "備註、裝置、用途等",
    apiKeyOptional: "API Key（留空不修改）",
    savedProfile: "帳號資訊已儲存",
    usageResets: "可用重置",
    useReset: "使用重置",
    useResetConfirm: "確定使用一次重置嗎？這會重置目前適用的 Codex 用量視窗。",
    usageResetDone: "用量已重置"
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
  const [apiKey, setApiKey] = useState("");
  const [codexConfig, setCodexConfig] = useState<CodexConfigFiles | null>(null);
  const [authJsonDraft, setAuthJsonDraft] = useState("");
  const [configTomlDraft, setConfigTomlDraft] = useState("");
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
  const [autoProbeRunning, setAutoProbeRunning] = useState(false);
  const [routingStatus, setRoutingStatus] = useState<RoutingStatus | null>(null);
  const [routingHost, setRoutingHost] = useState("0.0.0.0");
  const [routingPort, setRoutingPort] = useState(15722);
  const [routingRiskConfirmed, setRoutingRiskConfirmed] = useState(false);
  const [routingMode, setRoutingMode] = useState<"auto" | "fixed">("auto");
  const [routingFixedProfileId, setRoutingFixedProfileId] = useState("");
  const [routingStickyTtlSecs, setRoutingStickyTtlSecs] = useState(3600);
  const [quotaDraft, setQuotaDraft] = useState<QuotaRule>(emptyQuota);
  const [aliasDraft, setAliasDraft] = useState("");
  const [editAliasDraft, setEditAliasDraft] = useState("");
  const [editNoteDraft, setEditNoteDraft] = useState("");
  const [editProviderIdDraft, setEditProviderIdDraft] = useState("");
  const [editBaseUrlDraft, setEditBaseUrlDraft] = useState("");
  const [editModelDraft, setEditModelDraft] = useState("");
  const [editWireApiDraft, setEditWireApiDraft] = useState("responses");
  const [editApiKeyDraft, setEditApiKeyDraft] = useState("");
  const [priorityDraft, setPriorityDraft] = useState(100);
  const [enabledDraft, setEnabledDraft] = useState(true);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);
  const [activePage, setActivePage] = useState<"dashboard" | "routing" | "settings">("dashboard");
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
    const profiles = store?.profiles || [];
    const query = accountFilter.trim().toLowerCase();
    if (!query) return profiles;
    return profiles.filter((profile) => {
      const values = [
        profile.alias,
        profile.note,
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
    setRoutingRiskConfirmed(!!routing.riskConfirmed);
    setRoutingMode(routing.mode || "auto");
    setRoutingFixedProfileId(routing.fixedProfileId || "");
    setRoutingStickyTtlSecs(routing.stickyTtlSecs || 3600);
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
    setAliasDraft(selectedProfile.alias);
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
    const routing = await invoke<RoutingStatus>("routing_status");
    setRoutingStatus(routing);
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

  async function saveQuota() {
    if (!selectedProfile) return;
    await run(async () => {
      const view = await invoke<StoreView>("save_quota_rule", {
        profileId: selectedProfile.id,
        alias: aliasDraft.trim(),
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

  function openEditProfile(profile: Profile) {
    setEditingProfileId(profile.id);
    setEditAliasDraft(profile.alias);
    setEditNoteDraft(profile.note || "");
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

  async function saveProfileDetails() {
    if (!editingProfile) return;
    await run(async () => {
      const view = await invoke<StoreView>("update_profile_details", {
        profileId: editingProfile.id,
        alias: editAliasDraft.trim(),
        note: editNoteDraft,
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

  async function saveRoutingSettings(enabled = routingStatus?.settings.enabled ?? false) {
    await run(async () => {
      const routing = await invoke<RoutingStatus>("routing_save_settings", {
        input: {
          listenHost: routingHost,
          port: routingPort,
          enabled,
          riskConfirmed: routingRiskConfirmed,
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
    await run(async () => {
      const routing = await invoke<RoutingStatus>("routing_save_settings", {
        input: {
          listenHost: routingHost,
          port: routingPort,
          enabled: !running,
          riskConfirmed: routingRiskConfirmed,
          mode: routingMode,
          fixedProfileId: routingMode === "fixed" ? routingFixedProfileId || undefined : undefined,
          stickyTtlSecs: routingStickyTtlSecs
        }
      });
      setRoutingStatus(routing);
      await refresh();
      return routing;
    }, running ? "路由服务已停止" : "路由服务已启动");
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
    await run(async () => {
      const routing = await invoke<RoutingStatus>("routing_apply_codex_config");
      setRoutingStatus(routing);
      await refresh();
      return routing;
    }, "已接管本机 Codex 配置");
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
          riskConfirmed: routingRiskConfirmed,
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

  async function checkForUpdate(manual = true) {
    setUpdateChecking(true);
    try {
      const update = await check({ timeout: 15000 });
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
    const force = await shouldForceSwitch(profile);
    if (force == null) return;
    await run(async () => {
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
      const view = await invoke<StoreView>("add_api_profile", {
        alias: apiProviderName || alias,
        providerId: apiProviderId,
        baseUrl: apiBaseUrl,
        model: apiModel,
        apiKey
      });
      setStore(view);
      setSelectedId(view.profiles[view.profiles.length - 1]?.id || "");
      setApiProviderName("");
      setAlias("");
      setApiProviderId("");
      setApiBaseUrl("");
      setApiModel("");
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
            <label className="add-account-field">
              {t.importAlias}
              <input value={alias} onChange={(event) => setAlias(event.target.value)} placeholder={t.importAlias} />
            </label>
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
                <label>{t.apiProviderName}<input value={apiProviderName} onChange={(event) => setApiProviderName(event.target.value)} placeholder="LongCat / OpenAI Compatible" /></label>
                <label>{t.providerId}<input value={apiProviderId} onChange={(event) => setApiProviderId(event.target.value)} placeholder="openai-compatible" /></label>
                <label>{t.apiBaseUrl}<input value={apiBaseUrl} onChange={(event) => setApiBaseUrl(event.target.value)} placeholder="https://api.openai.com/v1" /></label>
                <label>{t.apiModel}<input value={apiModel} onChange={(event) => setApiModel(event.target.value)} placeholder="gpt-5.4" /></label>
                <label>{t.apiKey}<input type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} /></label>
                <button className="icon-button primary wide-button" onClick={() => void addApiProvider()} disabled={busy || !apiProviderId.trim() || !apiModel.trim() || !apiKey.trim()}>
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
          <section className="add-account-dialog" role="dialog" aria-modal="true" aria-labelledby="edit-account-title">
            <div className="update-dialog-head">
              <h2 id="edit-account-title">{t.editAccountInfo}</h2>
              <button className="notice-close" onClick={closeEditProfile} disabled={busy} title={t.closeNotice}>
                <X size={18} />
              </button>
            </div>
            <div className="api-provider-form add-account-content">
              <label>
                {t.accountAlias}
                <input value={editAliasDraft} onChange={(event) => setEditAliasDraft(event.target.value)} />
              </label>
              <label>
                {t.accountNote}
                <textarea
                  className="profile-note-input"
                  value={editNoteDraft}
                  onChange={(event) => setEditNoteDraft(event.target.value)}
                  placeholder={t.notePlaceholder}
                />
              </label>
              {editingProfile.apiConfig && (
                <>
                  <label>{t.providerId}<input value={editProviderIdDraft} onChange={(event) => setEditProviderIdDraft(event.target.value)} /></label>
                  <label>{t.apiBaseUrl}<input value={editBaseUrlDraft} onChange={(event) => setEditBaseUrlDraft(event.target.value)} placeholder="https://api.openai.com/v1" /></label>
                  <label>{t.apiModel}<input value={editModelDraft} onChange={(event) => setEditModelDraft(event.target.value)} /></label>
                  <label>
                    Wire API
                    <select value={editWireApiDraft} onChange={(event) => setEditWireApiDraft(event.target.value)}>
                      <option value="responses">Responses API</option>
                    </select>
                  </label>
                  <label>{t.apiKeyOptional}<input type="password" value={editApiKeyDraft} onChange={(event) => setEditApiKeyDraft(event.target.value)} /></label>
                </>
              )}
              <div className="oauth-action-row">
                <button className="icon-button" onClick={closeEditProfile} disabled={busy}>{t.closeNotice}</button>
                <button
                  className="icon-button primary"
                  onClick={() => void saveProfileDetails()}
                  disabled={busy || !editAliasDraft.trim() || !!(editingProfile.apiConfig && (!editProviderIdDraft.trim() || !editModelDraft.trim()))}
                >
                  <ShieldCheck size={17} /> {t.saveRules}
                </button>
              </div>
            </div>
          </section>
        </div>
      )}

      {activePage === "routing" ? (
        <section className="routing-page">
          <div className="routing-hero panel">
            <div>
              <h2>路由 API</h2>
              <p>单一转发 API，按规则在本地账号池中选择上游账号。</p>
            </div>
            <StatusPill ok={!!routingStatus?.running} text={routingStatus?.running ? "运行中" : "已停止"} />
          </div>

          <section className="routing-grid">
            <div className="panel routing-settings-panel">
              <div className="panel-header">
                <div>
                  <h2>服务设置</h2>
                  <p>{routingStatus?.baseUrl || `http://${routingHost}:${routingPort}/v1`}</p>
                </div>
                <button className="icon-button primary" onClick={() => void toggleRoutingService()} disabled={busy}>
                  <Power size={17} />
                  {routingStatus?.running ? "停止" : "启动"}
                </button>
              </div>

              <div className="form-grid">
                <label>
                  监听地址
                  <input value={routingHost} onChange={(event) => setRoutingHost(event.target.value)} />
                </label>
                <label>
                  端口
                  <input type="number" min={1} max={65535} value={routingPort} onChange={(event) => setRoutingPort(Number(event.target.value) || 15722)} />
                </label>
                <label>
                  粘性 TTL 秒
                  <input type="number" min={60} value={routingStickyTtlSecs} onChange={(event) => setRoutingStickyTtlSecs(Number(event.target.value) || 3600)} />
                </label>
                <label>
                  路由模式
                  <select value={routingMode} onChange={(event) => setRoutingMode(event.target.value as "auto" | "fixed")}>
                    <option value="auto">自动会话粘性</option>
                    <option value="fixed">固定账号并兜底</option>
                  </select>
                </label>
                {routingMode === "fixed" && (
                  <label>
                    固定账号
                    <select value={routingFixedProfileId} onChange={(event) => setRoutingFixedProfileId(event.target.value)}>
                      <option value="">未指定</option>
                      {(store?.profiles || []).map((profile) => (
                        <option value={profile.id} key={profile.id}>{profile.alias}</option>
                      ))}
                    </select>
                  </label>
                )}
              </div>

              <label className="checkline routing-risk">
                <input
                  type="checkbox"
                  checked={routingRiskConfirmed}
                  onChange={(event) => setRoutingRiskConfirmed(event.target.checked)}
                />
                我确认 OAuth 订阅账号反代可能带来账号限制风险，仅在可信环境使用。
              </label>

              <div className="action-row">
                <button className="icon-button" onClick={() => void saveRoutingSettings()} disabled={busy}>
                  <ShieldCheck size={17} /> 保存设置
                </button>
                <button className="icon-button" onClick={() => void reloadRoutingStatus()} disabled={busy}>
                  <RefreshCcw size={17} /> 刷新状态
                </button>
              </div>
            </div>

            <div className="panel routing-key-panel">
              <div className="panel-header">
                <div>
                  <h2>客户端配置</h2>
                  <p>用于 Codex 自定义 provider 或其他兼容客户端。</p>
                </div>
                <KeyRound size={22} />
              </div>
              <div className="routing-secret">
                <span>Base URL</span>
                <strong>{routingStatus?.baseUrl || "-"}</strong>
              </div>
              <div className="routing-secret">
                <span>API Key</span>
                <strong>{routingStatus?.accessKey || "未生成"}</strong>
              </div>
              <div className="action-row">
                <button className="icon-button" onClick={() => void copyRoutingConfig()} disabled={!routingStatus?.accessKey}>
                  <Copy size={17} /> 复制配置
                </button>
                <button className="icon-button" onClick={() => void regenerateRoutingKey()} disabled={busy}>
                  <KeyRound size={17} /> 重生成 Key
                </button>
              </div>
              <div className="action-row">
                <button className="icon-button primary" onClick={() => void applyRoutingCodexConfig()} disabled={busy || !routingStatus?.accessKey}>
                  <Zap size={17} /> 一键接管 Codex
                </button>
                <button className="icon-button" onClick={() => void restoreRoutingCodexConfig()} disabled={busy || !store?.settings.routing.appliedToCodex}>
                  <RotateCcw size={17} /> 恢复配置
                </button>
              </div>
              <StatusPill ok={!!store?.settings.routing.appliedToCodex} text={store?.settings.routing.appliedToCodex ? "已接管本机 Codex" : "未接管"} />
            </div>
          </section>

          <section className="routing-grid">
            <div className="panel">
              <div className="panel-header">
                <div>
                  <h2>账号池</h2>
                  <p>{routingStatus?.activeConnections || 0} active connections</p>
                </div>
                <Gauge size={22} />
              </div>
              <div className="routing-account-list">
                {(store?.profiles || []).map((profile) => (
                  <article className="routing-account-row" key={profile.id}>
                    <div>
                      <strong>{profile.alias}</strong>
                      <small>{profile.summary.email || profile.apiConfig?.baseUrl || profile.summary.accountId || t.unknownAccount}</small>
                    </div>
                    <StatusPill ok={profile.enabled && !isCooling(profile)} text={accountState(profile, t)} />
                    <span>{profile.apiConfig ? profile.apiConfig.model : formatSubscriptionValidity(profile, t)}</span>
                    <span>连接 {profile.routeHealth?.activeConnections || 0}</span>
                    <span>{profile.routeHealth?.lastStatus || "-"}</span>
                    <button
                      className="mini-button primary"
                      onClick={() => {
                        setRoutingMode("fixed");
                        setRoutingFixedProfileId(profile.id);
                        setActivePage("routing");
                      }}
                    >
                      固定
                    </button>
                  </article>
                ))}
              </div>
            </div>

            <div className="panel">
              <div className="panel-header">
                <div>
                  <h2>最近请求</h2>
                  <p>仅记录路由元数据，不记录提示词或响应正文。</p>
                </div>
                <FileText size={22} />
              </div>
              <div className="routing-log-list">
                {(routingStatus?.recentLogs || []).slice().reverse().map((log, index) => (
                  <div className="routing-log-row" key={`${log.ts}-${index}`}>
                    <div>
                      <strong>{log.alias || log.profileId || "-"}</strong>
                      <small>{formatDate(log.ts)}</small>
                    </div>
                    <span>{log.httpStatus || log.status}</span>
                    <span>{log.actualModel || log.requestedModel || "-"}</span>
                    <span>{log.latencyMs} ms</span>
                    <small>{log.fallback || log.error || log.sessionHash || "-"}</small>
                  </div>
                ))}
                {(routingStatus?.recentLogs || []).length === 0 && <div className="account-empty">暂无请求日志</div>}
              </div>
            </div>
          </section>
        </section>
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

          <details className="api-provider-panel">
            <summary>{t.apiProvider}</summary>
            <p>{t.apiResponsesHint}</p>
            <div className="api-provider-form">
              <label>
                {t.apiProviderName}
                <input value={apiProviderName} onChange={(event) => setApiProviderName(event.target.value)} />
              </label>
              <label>
                {t.providerId}
                <input
                  value={apiProviderId}
                  onChange={(event) => setApiProviderId(event.target.value)}
                  placeholder="openai-compatible"
                />
              </label>
              <label>
                {t.apiBaseUrl}
                <input
                  value={apiBaseUrl}
                  onChange={(event) => setApiBaseUrl(event.target.value)}
                  placeholder="默认 https://api.openai.com/v1"
                />
              </label>
              <label>
                {t.apiModel}
                <input value={apiModel} onChange={(event) => setApiModel(event.target.value)} placeholder="gpt-5.4" />
              </label>
              <label>
                {t.apiKey}
                <input type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} />
              </label>
              <button
                className="icon-button primary"
                onClick={() => void addApiProvider()}
                disabled={busy || !apiProviderId.trim() || !apiModel.trim() || !apiKey.trim()}
              >
                <KeyRound size={16} /> {t.addApiProvider}
              </button>
            </div>
          </details>

          <div className="account-card-grid">
            {filteredProfiles.map((profile) => {
              const isCurrent = currentGlobalProfileId === profile.id;
              const limits = profile.usage.detectedLimits || [];
              return (
                <article
                  key={profile.id}
                  className={`account-card ${selectedId === profile.id ? "selected" : ""} ${isCurrent ? "current" : ""}`}
                  onClick={() => setSelectedId(profile.id)}
                >
                  <div className="account-card-head">
                    <div className="account-card-title">
                      <strong>{profile.alias}</strong>
                      <small>{profile.summary.email || profile.summary.accountId || t.unknownAccount}</small>
                      {profile.note && <small className="account-note">{profile.note}</small>}
                    </div>
                    <div className="account-card-badges">
                      {isCurrent && <em className="current-badge">{t.currentUsing}</em>}
                      <span className="plan-badge">{planBadge(profile, t)}</span>
                    </div>
                  </div>

                  <div className="account-card-meta">
                    <StatusPill ok={profile.enabled && !isCooling(profile)} text={accountState(profile, t)} />
                    <span>{t.token}: {tokenState(profile, t)}</span>
                    {profile.usage.availableResetCount != null && (
                      <span>{t.usageResets}: {profile.usage.availableResetCount}</span>
                    )}
                  </div>

                  {profile.apiConfig && (
                    <div className="api-provider-summary">
                      <strong>{profile.apiConfig.model}</strong>
                      <small>{profile.apiConfig.baseUrl}</small>
                      <span>Responses API</span>
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
                      {profileNeedsReauthorization(profile) && (
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
                      if (store?.settings.routing.appliedToCodex) void fixProfileToRouting(profile.id);
                      else void switchProfile(profile.id);
                    }}
                    disabled={busy}
                    title={store?.settings.routing.appliedToCodex ? "固定到路由" : t.switch}
                    aria-label={store?.settings.routing.appliedToCodex ? "固定到路由" : t.switch}
                  >
                    <Zap size={14} />
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
                </article>
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
                {selectedProfile?.usage.availableResetCount != null && (
                  <span className="limit-chip">
                    {t.usageResets}: {selectedProfile.usage.availableResetCount}
                  </span>
                )}
                {(selectedProfile?.usage.availableResetCount || 0) > 0 && (
                  <button
                    className="mini-button primary"
                    onClick={() => void consumeUsageReset(selectedProfile?.id)}
                    disabled={busy}
                    title={t.useReset}
                  >
                    {t.useReset}
                  </button>
                )}
              </div>
            </div>
            <div className="form-grid">
            <label>
              {t.accountNote}
              <input
                value={aliasDraft}
                onChange={(event) => setAliasDraft(event.target.value)}
                title={t.accountNote}
              />
            </label>
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
            <button
              className="icon-button primary"
              onClick={() => selectedProfile && store?.settings.routing.appliedToCodex ? void fixProfileToRouting(selectedProfile.id) : void switchSelected()}
              disabled={!selectedProfile || busy}
              title={store?.settings.routing.appliedToCodex ? "固定到路由" : t.switch}
            >
              <Zap size={17} />
              {store?.settings.routing.appliedToCodex ? "固定到路由" : t.switch}
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

function limitRemainingPercent(item: DetectedLimit) {
  if (item.remainingPercent != null) return Math.max(0, Math.min(100, item.remainingPercent));
  if (item.usedPercent != null) return Math.max(0, Math.min(100, 100 - item.usedPercent));
  if (item.remaining != null && item.limit && item.limit > 0) {
    return Math.max(0, Math.min(100, Math.round((item.remaining / item.limit) * 100)));
  }
  if (item.used != null && item.limit && item.limit > 0) {
    return Math.max(0, Math.min(100, Math.round(((item.limit - item.used) / item.limit) * 100)));
  }
  return undefined;
}

function planBadge(profile: Profile, t: I18n) {
  const plan = profile.summary.plan?.trim();
  return plan ? plan.toUpperCase().replace(/_/g, " ") : t.pendingPlan;
}

function subscriptionExpiryState(profile: Profile) {
  const expiresAt = profile.summary.subscriptionActiveUntil
    ? Math.floor(new Date(profile.summary.subscriptionActiveUntil).getTime() / 1000)
    : undefined;
  const validExpiresAt = expiresAt != null && Number.isFinite(expiresAt) ? expiresAt : undefined;
  const remainingSeconds = validExpiresAt == null ? undefined : validExpiresAt - Math.floor(Date.now() / 1000);
  return { expiresAt: validExpiresAt, remainingSeconds, expired: remainingSeconds != null && remainingSeconds <= 0 };
}

function formatSubscriptionValidity(profile: Profile, t: I18n) {
  const { remainingSeconds, expired } = subscriptionExpiryState(profile);
  if (remainingSeconds == null) return "-";
  if (expired) return t.validityExpired;
  const days = Math.floor(remainingSeconds / 86400);
  const hours = Math.floor((remainingSeconds % 86400) / 3600);
  if (days > 0) return `${days}d ${hours}h`;
  const minutes = Math.max(0, Math.floor((remainingSeconds % 3600) / 60));
  return `${hours}h ${minutes}m`;
}

function quotaSummary(profile: Profile, t: I18n) {
  const items = profile.usage.detectedLimits || [];
  if (items.length > 0) {
    const limits = items
      .slice(0, 2)
      .map((item) => {
        const label = localizedLimitLabel(item.label || item.window, t);
        if (item.remainingPercent !== undefined) return `${label} ${item.remainingPercent}%`;
        if (item.usedPercent !== undefined) return `${label} ${t.used}${item.usedPercent}%`;
        return `${label} ${formatUsage(item.used, item.limit, t)}`;
      })
      .join(" / ");
    return profile.usage.availableResetCount != null
      ? `${limits} · ${t.usageResets} ${profile.usage.availableResetCount}`
      : limits;
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
  if (
    profile.usage.lastTokenRefreshStatus === "relogin_required" ||
    error.includes("refresh_token_reused") ||
    error.includes("invalid_grant")
  ) return t.reloginRequired;
  if (profile.usage.lastTokenRefreshStatus === "ok") return t.keptAlive;
  if (profile.usage.lastTokenRefreshStatus === "error") return t.keepaliveFailed;
  if (profile.summary.accessTokenExp && profile.summary.accessTokenExp * 1000 <= Date.now()) return t.expired;
  return t.normal;
}

function profileNeedsReauthorization(profile: Profile) {
  if (profile.apiConfig) return false;
  const error = profile.usage.lastTokenRefreshError || profile.usage.lastError || "";
  return (
    profile.usage.lastTokenRefreshStatus === "error" ||
    profile.usage.lastTokenRefreshStatus === "relogin_required" ||
    error.includes("token_invalidated") ||
    error.includes("refresh_token_reused") ||
    error.includes("invalid_grant")
  );
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
  if (
    profile.usage.lastTokenRefreshStatus === "relogin_required" ||
    summary.includes("refresh_token_reused") ||
    error.includes("refresh_token_reused") ||
    summary.includes("invalid_grant") ||
    error.includes("invalid_grant")
  ) {
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
