import type { AuthSummary, DetectedLimit, Profile } from "./types";
import { messages, type I18n } from "./i18n";

export function formatUsage(used?: number, limit?: number, t: I18n = messages["zh-CN"]) {
  const shownUsed = used ?? 0;
  const shownLimit = limit && limit > 0 ? String(limit) : t.unlimited;
  return `${shownUsed}/${shownLimit}`;
}

export function formatLimitChip(item: DetectedLimit, t: I18n) {
  const label = localizedLimitLabel(item.label || item.window, t);
  if (item.remainingPercent !== undefined) {
    return `${label}: ${t.remaining} ${item.remainingPercent}%${item.resetAt ? ` ${formatReset(item.resetAt)}` : ""}`;
  }
  if (item.usedPercent !== undefined) {
    return `${label}: ${t.used} ${item.usedPercent}%${item.resetAt ? ` ${formatReset(item.resetAt)}` : ""}`;
  }
  return `${label}: ${formatUsage(item.used, item.limit, t)}`;
}

export function limitRemainingPercent(item: DetectedLimit) {
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

export function planBadge(profile: Profile, t: I18n) {
  const plan = profile.summary.plan?.trim();
  return plan ? plan.toUpperCase().replace(/_/g, " ") : t.pendingPlan;
}

export function subscriptionExpiryState(profile: Profile) {
  const expiresAt = profile.summary.subscriptionActiveUntil
    ? Math.floor(new Date(profile.summary.subscriptionActiveUntil).getTime() / 1000)
    : undefined;
  const validExpiresAt = expiresAt != null && Number.isFinite(expiresAt) ? expiresAt : undefined;
  const remainingSeconds = validExpiresAt == null ? undefined : validExpiresAt - Math.floor(Date.now() / 1000);
  return { expiresAt: validExpiresAt, remainingSeconds, expired: remainingSeconds != null && remainingSeconds <= 0 };
}

export function formatSubscriptionValidity(profile: Profile, t: I18n) {
  const { remainingSeconds, expired } = subscriptionExpiryState(profile);
  if (remainingSeconds == null) return "-";
  if (expired) return t.validityExpired;
  const days = Math.floor(remainingSeconds / 86400);
  const hours = Math.floor((remainingSeconds % 86400) / 3600);
  if (days > 0) return `${days}d ${hours}h`;
  const minutes = Math.max(0, Math.floor((remainingSeconds % 3600) / 60));
  return `${hours}h ${minutes}m`;
}

export function quotaSummary(profile: Profile, t: I18n) {
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

export function isConversationFile(path: string) {
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

export function parseOptionalNumber(value: string) {
  if (value === "") return undefined;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : undefined;
}

export function normalizeNumber(value?: number) {
  return value && value > 0 ? value : undefined;
}

export function isCooling(profile: Profile) {
  if (!profile.cooldownUntil) return false;
  return new Date(profile.cooldownUntil).getTime() > Date.now();
}

export function accountState(profile: Profile, t: I18n) {
  if (!profile.enabled) return t.disabled;
  if (isCooling(profile)) return t.cooling;
  if (profile.usage.lastError) return t.probeFailed;
  return t.available;
}

export function tokenState(profile: Profile, t: I18n) {
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

export function profileNeedsReauthorization(profile: Profile) {
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

export function authSummariesMatch(left: AuthSummary, right: AuthSummary) {
  if (left.accountId && right.accountId && left.accountId === right.accountId) return true;
  if (left.userId && right.userId && left.userId === right.userId) return true;
  if (left.email && right.email && left.email.toLowerCase() === right.email.toLowerCase()) return true;
  return false;
}

export function friendlyProbeSummary(profile: Profile | undefined, t: I18n) {
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

export function localizedLimitLabel(label: string, t: I18n) {
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

export function localizeDetectedText(text: string, t: I18n) {
  return text
    .replace(/5小时|5小時|5h/gi, t.fiveHours)
    .replace(/1周|1週|1w/gi, t.oneWeek)
    .replace(/剩余|剩餘|remaining/gi, t.remaining)
    .replace(/已用|used/gi, t.used);
}

export function profileScore(profile: Profile) {
  const hourlyRemaining = profile.quotaRule.hourlyLimit
    ? profile.quotaRule.hourlyLimit - profile.usage.hourlyUsed
    : 10000;
  const dailyRemaining = profile.quotaRule.dailyLimit
    ? profile.quotaRule.dailyLimit - profile.usage.dailyUsed
    : 10000;
  const lastUsedPenalty = profile.usage.lastUsedAt ? new Date(profile.usage.lastUsedAt).getTime() / 1000000000 : 0;
  return Math.min(hourlyRemaining, dailyRemaining) * 10 + profile.priority - lastUsedPenalty;
}

export function formatDate(value?: string) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

export function formatReset(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  return sameDay
    ? date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString([], { month: "numeric", day: "numeric" });
}
