import {
  Copy,
  Eye,
  EyeOff,
  FileText,
  Gauge,
  KeyRound,
  LoaderCircle,
  Power,
  RefreshCcw,
  RotateCcw,
  ShieldCheck,
  Zap
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { I18n } from "../../i18n";
import { accountState, formatDate, formatSubscriptionValidity } from "../../profileUtils";
import type { Profile, RoutingLogEntry, RoutingStatus } from "../../types";
import { InfoTip } from "../InfoTip";
import { StatusPill } from "../StatusPill";
import { RoutingLogDialog } from "./RoutingLogDialog";
import {
  isRoutingProfileAvailable,
  routingLogResult,
  routingLogTone,
  sortRoutingProfiles,
  type RoutingPoolSort
} from "./routingView";

const ROUTING_LOG_PAGE_SIZE = 10;

type RoutingPageProps = {
  t: I18n;
  busy: boolean;
  profiles: Profile[];
  appliedToCodex: boolean;
  status: RoutingStatus | null;
  host: string;
  port: number;
  mode: "auto" | "fixed";
  fixedProfileId: string;
  stickyTtlSecs: number;
  routingBusy: boolean;
  onHostChange: (value: string) => void;
  onPortChange: (value: number) => void;
  onModeChange: (value: "auto" | "fixed") => void;
  onFixedProfileIdChange: (value: string) => void;
  onStickyTtlSecsChange: (value: number) => void;
  onToggleService: () => void | Promise<unknown>;
  onSaveSettings: () => void | Promise<unknown>;
  onReloadStatus: () => void | Promise<unknown>;
  onCopyConfig: () => void | Promise<unknown>;
  onRegenerateKey: () => void | Promise<unknown>;
  onApplyCodexConfig: () => void | Promise<unknown>;
  onRestoreCodexConfig: () => void | Promise<unknown>;
  onFixProfile: (profileId: string) => void | Promise<unknown>;
  onSaveProfilePriority: (profile: Profile, priority: number) => Promise<boolean>;
  onTestRequest: () => Promise<RoutingLogEntry | null>;
};

export function RoutingPage({
  t,
  busy,
  profiles,
  appliedToCodex,
  status,
  host,
  port,
  mode,
  fixedProfileId,
  stickyTtlSecs,
  routingBusy,
  onHostChange,
  onPortChange,
  onModeChange,
  onFixedProfileIdChange,
  onStickyTtlSecsChange,
  onToggleService,
  onSaveSettings,
  onReloadStatus,
  onCopyConfig,
  onRegenerateKey,
  onApplyCodexConfig,
  onRestoreCodexConfig,
  onFixProfile,
  onSaveProfilePriority,
  onTestRequest
}: RoutingPageProps) {
  const [poolSort, setPoolSort] = useState<RoutingPoolSort>("route");
  const [priorityDrafts, setPriorityDrafts] = useState<Record<string, string>>({});
  const [probeBusy, setProbeBusy] = useState(false);
  const [selectedLog, setSelectedLog] = useState<RoutingLogEntry | null>(null);
  const [logPage, setLogPage] = useState(1);
  const [showAccessKey, setShowAccessKey] = useState(false);
  const sortedProfiles = useMemo(
    () => sortRoutingProfiles(profiles, poolSort, t),
    [poolSort, profiles, t]
  );
  const previewProfile = mode === "fixed"
    ? profiles.find((profile) => profile.id === fixedProfileId)
    : sortedProfiles.find((profile) => isRoutingProfileAvailable(profile, t));
  const latestLog = status?.recentLogs.length
    ? status.recentLogs[status.recentLogs.length - 1]
    : undefined;
  const recentLogs = useMemo(
    () => (status?.recentLogs || []).slice().reverse(),
    [status?.recentLogs]
  );
  const logPageCount = Math.max(1, Math.ceil(recentLogs.length / ROUTING_LOG_PAGE_SIZE));
  const pagedLogs = recentLogs.slice(
    (logPage - 1) * ROUTING_LOG_PAGE_SIZE,
    logPage * ROUTING_LOG_PAGE_SIZE
  );
  const codexCheck = status?.codexCheck;
  const takeoverConfigured = !!codexCheck
    && codexCheck.providerPresent
    && codexCheck.baseUrlMatches
    && codexCheck.tokenPresent
    && codexCheck.authModeMatches;
  const takeoverExpected = appliedToCodex;
  const codexCheckOk = takeoverConfigured
    && !!codexCheck
    && codexCheck.serviceRunning
    && codexCheck.healthOk;
  const fixedModeIncomplete = mode === "fixed" && !fixedProfileId;
  const modeLabel = mode === "fixed" ? "固定账号" : "自动会话粘性";
  const previewText = mode === "fixed"
    ? previewProfile
      ? `固定使用：${previewProfile.alias}`
      : "固定账号未选择"
    : previewProfile
      ? `新会话优先：${previewProfile.alias}`
      : "暂无可用账号";
  const latestText = latestLog
    ? `${latestLog.alias || latestLog.profileId || "未知账号"} · ${latestLog.httpStatus || latestLog.status}`
    : "暂无请求";
  const codexCheckText = codexCheckOk
    ? "配置与服务正常"
    : !takeoverExpected && !takeoverConfigured
      ? "尚未接管"
      : codexCheck
        ? "需要检查"
        : "等待检查";

  useEffect(() => {
    setLogPage((current) => Math.min(current, logPageCount));
  }, [logPageCount]);

  useEffect(() => {
    setShowAccessKey(false);
  }, [status?.accessKey]);

  async function savePriority(profile: Profile) {
    const draft = Number(priorityDrafts[profile.id] ?? profile.priority);
    const priority = Number.isFinite(draft) ? draft : profile.priority;
    if (await onSaveProfilePriority(profile, priority)) {
      setPriorityDrafts((current) => {
        const next = { ...current };
        delete next[profile.id];
        return next;
      });
    }
  }

  async function testRequest() {
    setProbeBusy(true);
    try {
      const log = await onTestRequest();
      if (log) {
        setLogPage(1);
        setSelectedLog(log);
      }
    } finally {
      setProbeBusy(false);
    }
  }

  return (
    <>
      {selectedLog && <RoutingLogDialog log={selectedLog} onClose={() => setSelectedLog(null)} />}
      <section className="routing-page">
        <div className="routing-hero panel">
          <div>
            <h2>路由 API</h2>
            <p>单一转发 API，按规则在本地账号池中选择上游账号。</p>
          </div>
          <StatusPill ok={!!status?.running} text={routingBusy ? "处理中..." : status?.running ? "运行中" : "已停止"} />
        </div>

        <section className="routing-columns">
          <div className="routing-column">
            <div className="panel routing-settings-panel">
            <div className="panel-header">
              <div>
                <h2>服务设置</h2>
                <p>{status?.baseUrl || `http://${host}:${port}/v1`}</p>
              </div>
              <button
                className="icon-button primary"
                onClick={() => void onToggleService()}
                disabled={busy || routingBusy || fixedModeIncomplete}
                title={t.routingServiceToggleHint}
              >
                <Power size={17} />
                {routingBusy ? "处理中..." : status?.running ? "停止" : "启动"}
              </button>
            </div>

            <div className="form-grid">
              <label>
                <span className="field-label-with-tip">
                  监听地址
                  <InfoTip text={t.routingListenHostHint} />
                </span>
                <input value={host} onChange={(event) => onHostChange(event.target.value)} />
              </label>
              <label>
                <span className="field-label-with-tip">
                  端口
                  <InfoTip text={t.routingPortHint} />
                </span>
                <input type="number" min={1} max={65535} value={port} onChange={(event) => onPortChange(Number(event.target.value) || 15722)} />
              </label>
              <label>
                <span className="field-label-with-tip">
                  粘性 TTL 秒
                  <InfoTip text={t.routingStickyTtlHint} />
                </span>
                <input type="number" min={60} value={stickyTtlSecs} onChange={(event) => onStickyTtlSecsChange(Number(event.target.value) || 3600)} />
              </label>
              <label className="routing-mode-field">
                <span className="field-label-with-tip">
                  路由模式
                  <InfoTip text={t.routingModeHint} />
                </span>
                <select value={mode} onChange={(event) => onModeChange(event.target.value as "auto" | "fixed")}>
                  <option value="auto">自动会话粘性</option>
                  <option value="fixed">固定账号并兜底</option>
                </select>
              </label>
              {mode === "fixed" && (
                <label>
                  <span className="field-label-with-tip">
                    固定账号
                    <InfoTip text={t.routingFixedAccountHint} />
                  </span>
                  <select value={fixedProfileId} onChange={(event) => onFixedProfileIdChange(event.target.value)}>
                    <option value="">未指定</option>
                    {profiles.map((profile) => (
                      <option value={profile.id} key={profile.id}>{profile.alias}</option>
                    ))}
                  </select>
                </label>
              )}
            </div>
            {fixedModeIncomplete && (
              <p className="routing-inline-warning">固定模式未选择账号时不会保存、启动、测试或接管；请选择账号，或在账号池里直接点“固定”。</p>
            )}

            <div className="action-row">
              <button
                className="icon-button"
                onClick={() => void onSaveSettings()}
                disabled={busy || fixedModeIncomplete}
                title={t.routingSaveSettingsHint}
              >
                <ShieldCheck size={17} /> 保存设置
              </button>
              <button
                className="icon-button"
                onClick={() => void onReloadStatus()}
                disabled={busy}
                title={t.routingRefreshStatusHint}
              >
                <RefreshCcw size={17} /> 刷新状态
              </button>
            </div>
            </div>

            <div className="panel">
              <div className="panel-header">
                <div>
                  <h2>账号池</h2>
                  <p>{status?.activeConnections || 0} active connections</p>
                </div>
                <div className="routing-pool-tools">
                  <Gauge size={22} />
                  <select value={poolSort} onChange={(event) => setPoolSort(event.target.value as RoutingPoolSort)}>
                    <option value="route">路由顺序</option>
                    <option value="priority">优先级</option>
                    <option value="expiry">到期时间</option>
                    <option value="connections">连接数</option>
                    <option value="status">状态</option>
                  </select>
                </div>
              </div>
              <div className="routing-account-list">
                {sortedProfiles.map((profile) => (
                  <article className="routing-account-row" key={profile.id}>
                    <div className="routing-account-main">
                      <strong>{profile.alias}</strong>
                      <small>{profile.summary.email || profile.apiConfig?.baseUrl || profile.summary.accountId || t.unknownAccount}</small>
                    </div>
                    <div className="routing-account-meta">
                      <StatusPill ok={isRoutingProfileAvailable(profile, t)} text={accountState(profile, t)} />
                      <span>{profile.apiConfig ? profile.apiConfig.model : formatSubscriptionValidity(profile, t)}</span>
                      <span>连接 {profile.routeHealth?.activeConnections || 0}</span>
                      <span>{profile.routeHealth?.lastStatus || "-"}</span>
                    </div>
                    <label className="routing-priority-control">
                      <span>优先级</span>
                      <input
                        type="number"
                        value={priorityDrafts[profile.id] ?? String(profile.priority)}
                        onChange={(event) => setPriorityDrafts((current) => ({ ...current, [profile.id]: event.target.value }))}
                      />
                      <button
                        className="mini-button"
                        onClick={() => void savePriority(profile)}
                        disabled={busy || Number(priorityDrafts[profile.id] ?? profile.priority) === profile.priority}
                      >
                        保存
                      </button>
                    </label>
                    <button
                      className="mini-button primary"
                      onClick={() => void onFixProfile(profile.id)}
                      disabled={busy}
                      title={t.routingFixAccountHint}
                    >
                      固定
                    </button>
                  </article>
                ))}
              </div>
            </div>
          </div>

          <div className="routing-column">
            <div className="panel routing-key-panel">
            <div className="panel-header">
              <div>
                <h2>客户端配置</h2>
                <p>用于 Codex 自定义 provider 或其他兼容客户端。</p>
              </div>
              <KeyRound size={22} />
            </div>
            <div className={`routing-takeover-card ${takeoverConfigured ? "active" : ""}`}>
              <div>
                <span>Codex 接管状态</span>
                <strong>{takeoverConfigured ? "已接管到本路由" : takeoverExpected ? "接管配置已失效" : "未接管"}</strong>
                <p>{takeoverConfigured ? "Provider 与 API Key 认证已切换到本路由；请新建会话使用接管配置。" : takeoverExpected ? "本机 Codex 配置被其他工具修改，请点击重新接管。" : "点击一键接管会自动切换配置，并在确认后重启 Codex。"}</p>
              </div>
              <StatusPill ok={takeoverConfigured} text={takeoverConfigured ? "接管中" : takeoverExpected ? "已失效" : "未接管"} />
            </div>
            <div className="routing-status-grid">
              <div>
                <span>当前策略</span>
                <strong>{modeLabel}</strong>
              </div>
              <div>
                <span>{mode === "fixed" ? "使用账号" : "预计下一跳"}</span>
                <strong>{previewText}</strong>
              </div>
              <div>
                <span>最近实际命中</span>
                <strong>{latestText}</strong>
              </div>
              <div>
                <span>接管自检</span>
                <strong>{codexCheckText}</strong>
              </div>
            </div>
            {codexCheck && (takeoverExpected || takeoverConfigured) && (
              <div className={`routing-check-card ${codexCheckOk ? "ok" : "warn"}`}>
                <div>
                  <strong>{codexCheckOk ? "接管链路自检通过" : "接管链路还有异常"}</strong>
                  <p>{codexCheck.diagnostics[0] || "等待刷新状态"}</p>
                </div>
                <div className="routing-check-flags">
                  <StatusPill ok={codexCheck.providerPresent} text="Provider" />
                  <StatusPill ok={codexCheck.baseUrlMatches} text="Base URL" />
                  <StatusPill ok={codexCheck.tokenPresent} text="Key" />
                  <StatusPill ok={codexCheck.authModeMatches} text="认证" />
                  <StatusPill ok={codexCheck.healthOk} text="服务" />
                </div>
              </div>
            )}
            <div className="routing-secret">
              <span>Base URL</span>
              <strong>{status?.baseUrl || "-"}</strong>
            </div>
            <div className="routing-secret">
              <div className="routing-secret-head">
                <span>API Key</span>
                <button
                  className="mini-button icon-only"
                  type="button"
                  onClick={() => setShowAccessKey((current) => !current)}
                  disabled={!status?.accessKey}
                  title={showAccessKey ? t.hideApiKey : t.showApiKey}
                  aria-label={showAccessKey ? t.hideApiKey : t.showApiKey}
                >
                  {showAccessKey ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              </div>
              <strong>{status?.accessKey ? (showAccessKey ? status.accessKey : "••••••••••••••••••••••••") : "未生成"}</strong>
            </div>
            <div className="action-row">
              <button
                className="icon-button"
                onClick={() => void onCopyConfig()}
                disabled={!status?.accessKey}
                title={t.routingCopyConfigHint}
              >
                <Copy size={17} /> 复制配置
              </button>
              <button
                className="icon-button"
                onClick={() => void onRegenerateKey()}
                disabled={busy}
                title={t.routingRegenerateKeyHint}
              >
                <KeyRound size={17} /> 重生成 Key
              </button>
              <button
                className="icon-button"
                onClick={() => void onReloadStatus()}
                disabled={busy}
                title={t.routingSelfCheckHint}
              >
                <RefreshCcw size={17} /> 自检接管
              </button>
            </div>
            <div className="action-row">
              <button
                className="icon-button primary"
                onClick={() => void onApplyCodexConfig()}
                disabled={busy || !status?.accessKey || fixedModeIncomplete}
                title={t.routingTakeoverHint}
              >
                <Zap size={17} /> {takeoverExpected && !takeoverConfigured ? "重新接管 Codex" : "一键接管 Codex"}
              </button>
              <button
                className="icon-button"
                onClick={() => void onRestoreCodexConfig()}
                disabled={busy || (!takeoverExpected && !takeoverConfigured)}
                title={t.routingRestoreConfigHint}
              >
                <RotateCcw size={17} /> 恢复配置
              </button>
            </div>
            <p className="routing-help-text">自动模式会按账号池排序选择新会话；同一会话在 TTL 内保持粘性。实际用了谁，看“最近实际命中”和下方请求日志。</p>
            </div>

            <div className="panel">
            <div className="panel-header">
              <div>
                <h2>最近请求</h2>
                <p>仅记录路由元数据，不记录提示词或响应正文。</p>
              </div>
              <div className="panel-header-actions">
                <button
                  className="mini-button primary"
                  onClick={() => void testRequest()}
                  disabled={probeBusy || routingBusy || fixedModeIncomplete}
                  title={t.routingTestRequestHint}
                >
                  {probeBusy ? <LoaderCircle className="button-spinner" size={15} /> : <Zap size={15} />}
                  {probeBusy ? "测试中..." : "测试请求"}
                </button>
                <FileText size={22} />
              </div>
            </div>
            <div className="routing-log-list">
              {pagedLogs.map((log, index) => (
                <button
                  type="button"
                  className="routing-log-row"
                  key={`${log.ts}-${index}`}
                  onClick={() => setSelectedLog(log)}
                  title="查看请求详情"
                >
                  <div className="routing-log-main">
                    <strong>{log.alias || log.profileId || "-"}</strong>
                    <small>{formatDate(log.ts)}</small>
                  </div>
                  <span className={`routing-log-status ${routingLogTone(log)}`}>
                    {routingLogResult(log)} · {log.httpStatus || "-"}
                  </span>
                  <span className="routing-log-latency">{log.latencyMs} ms</span>
                  <div className="routing-log-meta">
                    <span>{log.actualModel || log.requestedModel || "未知模型"}</span>
                    {log.wireProtocol && <span>{log.wireProtocol}</span>}
                  </div>
                  <small className="routing-log-summary">{log.error || log.fallback || log.requestId || log.sessionHash || "点击查看详情"}</small>
                </button>
              ))}
              {recentLogs.length === 0 && (
                <div className="routing-log-empty">
                  <strong>暂无请求日志</strong>
                  <p>接管后需要让 Codex 发起一次新请求，这里才会出现实际命中的账号、状态和耗时。</p>
                  <small>如果自检通过但一直为空，请完全退出并重开 Codex，或新建线程后再发送一条消息；老会话通常不会重新读取刚写入的 provider。</small>
                </div>
              )}
            </div>
            {recentLogs.length > 0 && (
              <div className="routing-log-pagination">
                <span>共 {recentLogs.length} 条 · 第 {logPage} / {logPageCount} 页</span>
                <div>
                  <button
                    className="mini-button"
                    type="button"
                    disabled={logPage <= 1}
                    onClick={() => setLogPage((current) => Math.max(1, current - 1))}
                  >
                    上一页
                  </button>
                  <button
                    className="mini-button"
                    type="button"
                    disabled={logPage >= logPageCount}
                    onClick={() => setLogPage((current) => Math.min(logPageCount, current + 1))}
                  >
                    下一页
                  </button>
                </div>
              </div>
            )}
            </div>
          </div>
        </section>
      </section>
    </>
  );
}
