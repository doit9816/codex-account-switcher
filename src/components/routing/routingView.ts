import type { I18n } from "../../i18n";
import {
  accountState,
  isCooling,
  profileNeedsReauthorization,
  subscriptionExpiryState,
  tokenState
} from "../../profileUtils";
import type { Profile, RoutingLogEntry } from "../../types";

export type RoutingPoolSort = "route" | "priority" | "expiry" | "connections" | "status";
export type RoutingLogTone = "ok" | "warning" | "error";

export function routingProfileRank(profile: Profile, t: I18n) {
  if (!profile.enabled) return 5;
  if (isCooling(profile)) return 4;
  if (subscriptionExpiryState(profile).expired || tokenState(profile, t) === t.expired) return 3;
  if (
    profileNeedsReauthorization(profile) ||
    tokenState(profile, t) === t.reloginRequired ||
    tokenState(profile, t) === t.authInvalid
  ) return 2;
  if (accountState(profile, t) !== t.available) return 1;
  return 0;
}

export function isRoutingProfileAvailable(profile: Profile, t: I18n) {
  return routingProfileRank(profile, t) === 0;
}

export function sortRoutingProfiles(profiles: Profile[], sort: RoutingPoolSort, t: I18n) {
  const expiryValue = (profile: Profile) => {
    const expiresAt = subscriptionExpiryState(profile).expiresAt;
    return expiresAt == null ? Number.MAX_SAFE_INTEGER : expiresAt;
  };
  return profiles.slice().sort((left, right) => {
    const availabilityOrder =
      Number(!isRoutingProfileAvailable(left, t)) - Number(!isRoutingProfileAvailable(right, t));
    if (availabilityOrder !== 0) return availabilityOrder;
    if (sort === "priority") {
      return right.priority - left.priority || left.alias.localeCompare(right.alias);
    }
    if (sort === "expiry") {
      return (
        expiryValue(left) - expiryValue(right) ||
        right.priority - left.priority ||
        left.alias.localeCompare(right.alias)
      );
    }
    if (sort === "connections") {
      return (
        (right.routeHealth?.activeConnections || 0) -
          (left.routeHealth?.activeConnections || 0) ||
        right.priority - left.priority ||
        left.alias.localeCompare(right.alias)
      );
    }
    if (sort === "status") {
      return (
        routingProfileRank(left, t) - routingProfileRank(right, t) ||
        right.priority - left.priority ||
        left.alias.localeCompare(right.alias)
      );
    }
    return (
      routingProfileRank(left, t) - routingProfileRank(right, t) ||
      expiryValue(left) - expiryValue(right) ||
      right.priority - left.priority ||
      (right.routeHealth?.activeConnections || 0) -
        (left.routeHealth?.activeConnections || 0) ||
      left.alias.localeCompare(right.alias)
    );
  });
}

export function routingLogTone(log: RoutingLogEntry): RoutingLogTone {
  if ((log.httpStatus || 0) >= 400 || log.status === "stream_error" || !!log.error) return "error";
  if (log.status === "fallback_ok" || !!log.fallback) return "warning";
  return "ok";
}

export function routingLogResult(log: RoutingLogEntry) {
  if (log.status === "stream_error") return "流内错误";
  if (log.status === "http_error" || (log.httpStatus || 0) >= 400) return "HTTP 错误";
  if (log.status === "fallback_ok" || log.fallback) return "回退成功";
  return "成功";
}
