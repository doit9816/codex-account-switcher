export type AuthSummary = {
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

export type QuotaRule = {
  hourlyLimit?: number;
  dailyLimit?: number;
  cooldownMinutes: number;
};

export type UsageStats = {
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
  availableResetExpiresAt?: string;
};

export type RouteHealth = {
  consecutiveFailures: number;
  activeConnections: number;
  lastRouteAt?: string;
  lastStatus?: string;
  lastError?: string;
  cooldownReason?: string;
};

export type DetectedLimit = {
  window: string;
  used?: number;
  limit?: number;
  remaining?: number;
  usedPercent?: number;
  remainingPercent?: number;
  resetAt?: string;
  label?: string;
};

export type Profile = {
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

export type AppEvent = {
  ts: string;
  level: string;
  message: string;
};

export type StoreView = {
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

export type RoutingSettings = {
  listenHost: string;
  port: number;
  enabled: boolean;
  riskConfirmed: boolean;
  appliedToCodex: boolean;
  mode: "auto" | "fixed";
  fixedProfileId?: string;
  stickyTtlSecs: number;
  logRetentionDays: number;
};

export type RoutingLogEntry = {
  ts: string;
  requestId?: string;
  method?: string;
  path?: string;
  wireProtocol?: string;
  upstreamUrl?: string;
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

export type RoutingProbeResult = {
  ok: boolean;
  requestId: string;
  httpStatus: number;
  elapsedMs: number;
  profileId?: string;
  actualModel?: string;
  responseStatus?: string;
  outputItems: number;
  message: string;
};

export type RoutingStatus = {
  running: boolean;
  baseUrl: string;
  accessKey?: string;
  activeConnections: number;
  settings: RoutingSettings;
  recentLogs: RoutingLogEntry[];
  codexCheck: RoutingCodexCheck;
};

export type RoutingCodexCheck = {
  configPath: string;
  authPath: string;
  selectedProvider?: string;
  providerPresent: boolean;
  baseUrlMatches: boolean;
  tokenPresent: boolean;
  authModeMatches: boolean;
  serviceRunning: boolean;
  healthOk: boolean;
  diagnostics: string[];
};

export type CodexScan = {
  codexHome: string;
  exists: boolean;
  hasAuth: boolean;
  currentAuth?: AuthSummary;
  migratable: string[];
  excluded: string[];
};

export type ConfigFileView = {
  path: string;
  exists: boolean;
  content: string;
};

export type CodexConfigFiles = {
  codexHome: string;
  authJson: ConfigFileView;
  configToml: ConfigFileView;
};

export type OAuthLoginSession = {
  loginId: string;
  authUrl: string;
  callbackUrl: string;
  expiresAt: string;
};

export type OAuthEvent = {
  loginId: string;
};

export type BundleManifest = {
  exportedAt: string;
  platform: string;
  profileCount: number;
  includeConversations: boolean;
  files: Array<{ path: string; bytes: number; sha256: string }>;
};

export type Notice = {
  kind: "ok" | "warn" | "error" | "info";
  text: string;
};

export type LanguageSetting = "system" | "zh-CN" | "en" | "zh-TW";
export type ResolvedLanguage = Exclude<LanguageSetting, "system">;
