import {
  BookOpen,
  Clipboard,
  Copy,
  Download,
  KeyRound,
  Link2,
  Network,
  PlugZap,
  RefreshCcw,
  Save,
  Send,
  ShieldCheck,
  Share2,
  Upload,
  Wifi,
} from "lucide-react";
import type { ReactNode } from "react";
import type { MeshDevice, MeshStatus, MeshSyncScope, Profile } from "../../types";
import { formatDate } from "../../profileUtils";
import { StatusPill } from "../StatusPill";

type MeshSharePageProps = {
  status: MeshStatus | null;
  profiles: Profile[];
  busy: boolean;
  sharePayload: string;
  importPayload: string;
  networkName: string;
  networkSecret: string;
  nodeSourceUrl: string;
  nodeRefreshSecs: number;
  autoStart: boolean;
  syncScope: MeshSyncScope;
  migrationPassword: string;
  migrationUseMeshSecret: boolean;
  includeConversations: boolean;
  restoreConversations: boolean;
  exportProfileIds: string[];
  onImportPayloadChange: (value: string) => void;
  onNetworkNameChange: (value: string) => void;
  onNetworkSecretChange: (value: string) => void;
  onNodeSourceUrlChange: (value: string) => void;
  onNodeRefreshSecsChange: (value: number) => void;
  onAutoStartChange: (value: boolean) => void;
  onSyncScopeChange: (scope: MeshSyncScope) => void;
  onMigrationPasswordChange: (value: string) => void;
  onMigrationUseMeshSecretChange: (value: boolean) => void;
  onIncludeConversationsChange: (value: boolean) => void;
  onRestoreConversationsChange: (value: boolean) => void;
  onToggleExportProfile: (profileId: string) => void;
  onSelectAllProfiles: () => void;
  onClearProfiles: () => void;
  onSaveSettings: () => void | Promise<unknown>;
  onToggleService: () => void | Promise<unknown>;
  onRefreshNodes: () => void | Promise<unknown>;
  onCreateShare: () => void | Promise<unknown>;
  onCopyShare: () => void | Promise<unknown>;
  onImportShare: () => void | Promise<unknown>;
  onSaveDevice: (device: MeshDevice) => void | Promise<unknown>;
  onSyncNow: (deviceId?: string) => void | Promise<unknown>;
  onExportMigration: () => void | Promise<unknown>;
  onImportMigration: () => void | Promise<unknown>;
};

export function MeshSharePage({
  status,
  profiles,
  busy,
  sharePayload,
  importPayload,
  networkName,
  networkSecret,
  nodeSourceUrl,
  nodeRefreshSecs,
  autoStart,
  syncScope,
  migrationPassword,
  migrationUseMeshSecret,
  includeConversations,
  restoreConversations,
  exportProfileIds,
  onImportPayloadChange,
  onNetworkNameChange,
  onNetworkSecretChange,
  onNodeSourceUrlChange,
  onNodeRefreshSecsChange,
  onAutoStartChange,
  onSyncScopeChange,
  onMigrationPasswordChange,
  onMigrationUseMeshSecretChange,
  onIncludeConversationsChange,
  onRestoreConversationsChange,
  onToggleExportProfile,
  onSelectAllProfiles,
  onClearProfiles,
  onSaveSettings,
  onToggleService,
  onRefreshNodes,
  onCreateShare,
  onCopyShare,
  onImportShare,
  onSaveDevice,
  onSyncNow,
  onExportMigration,
  onImportMigration,
}: MeshSharePageProps) {
  const upNodes = status?.publicNodes.filter((node) => node.status === "up").length || 0;
  const selectedProfiles = profiles.filter((profile) => exportProfileIds.includes(profile.id));
  const onlineDevices = (status?.devices || []).filter(
    (device) => device.online === true && device.id !== status?.localDeviceId
  );
  const discoveredPeers = (status?.peers || []).filter(
    (peer) =>
      peer.ip &&
      peer.ip !== status?.virtualIpv4 &&
      !onlineDevices.some((device) => device.ip === peer.ip)
  );

  return (
    <section className="mesh-page">
      <div className="mesh-hero panel">
        <div>
          <span className="section-kicker">Multi-device sharing</span>
          <h2>多设备共享与迁移</h2>
          <p>复制一台设备生成的分享码，建立连接并同步指定设备信息。</p>
        </div>
        <div className="mesh-hero-status">
          <StatusPill ok={!!status?.running} text={status?.running ? "运行中" : "已停止"} />
          <strong>{status?.localDeviceName || "-"}</strong>
          <small>{status?.localDeviceId || "等待初始化"}</small>
          {status?.runtimeKind && <small>{status.runtimeKind}</small>}
          {status?.peerCount != null && <small>{status.peerCount} 台在线设备</small>}
          {status?.virtualIpv4 && <small>{status.virtualIpv4}</small>}
          {status?.routingBaseUrl && <small>Routing API: {status.routingBaseUrl}</small>}
          {status?.startedAt && <small>{formatDate(status.startedAt)}</small>}
        </div>
      </div>
      {status?.lastError && <div className="mesh-warning">{status.lastError}</div>}

      <div className="mesh-columns">
        <div className="mesh-column">
          <div className="mesh-share-flow">
          <section className="panel mesh-settings-panel">
            <div className="panel-header">
              <div>
                <h2>多设备共享设置</h2>
                <p>选择分享内容，生成分享码后在其他设备导入即可。</p>
              </div>
              <Network size={22} />
            </div>
            <div className="mesh-form-grid">
              <div className="mesh-internal-settings" aria-hidden="true">
              <label>
                Network name
                <input value={networkName} onChange={(event) => onNetworkNameChange(event.target.value)} />
              </label>
              <label>
                Network secret
                <input
                  value={networkSecret}
                  onChange={(event) => onNetworkSecretChange(event.target.value)}
                  placeholder={status?.shareReady ? "留空保留现有密钥" : "留空自动生成"}
                />
              </label>
              <label className="form-span-all">
                节点源
                <input value={nodeSourceUrl} onChange={(event) => onNodeSourceUrlChange(event.target.value)} />
              </label>
              <label>
                刷新间隔
                <input
                  type="number"
                  min={60}
                  value={nodeRefreshSecs}
                  onChange={(event) => onNodeRefreshSecsChange(Number(event.target.value) || 120)}
                />
              </label>
              </div>
              <label className="checkline mesh-checkline">
                <input type="checkbox" checked={autoStart} onChange={(event) => onAutoStartChange(event.target.checked)} />
                启动应用时自动建立连接
              </label>
            </div>
            <div className="mesh-share-scope-hint">
              <strong>分享内容</strong>
              <span>由分享者决定，接收设备会按此设置建立连接。</span>
            </div>
            <ScopeEditor value={syncScope} onChange={onSyncScopeChange} />
            <div className="action-row">
              <button className="icon-button primary" onClick={() => void onSaveSettings()} disabled={busy}>
                <Save size={17} /> 保存设置
              </button>
              <button className="icon-button" onClick={() => void onToggleService()} disabled={busy}>
                <PlugZap size={17} /> {status?.running ? "断开连接" : "建立连接"}
              </button>
            </div>
          </section>

          <section className="panel mesh-share-panel">
            <div className="panel-header">
              <div>
                <h2>连接分享码</h2>
                <p>在其他设备粘贴导入，自动建立多设备共享连接。</p>
              </div>
              <Share2 size={22} />
            </div>
            <div className="action-row mesh-share-actions">
              <button className="icon-button primary" onClick={() => void onCreateShare()} disabled={busy}>
                <KeyRound size={17} /> 生成分享码
              </button>
              <button className="icon-button" onClick={() => void onCopyShare()} disabled={busy || !sharePayload.trim()}>
                <Copy size={17} /> 复制
              </button>
            </div>
            <div className="mesh-payload-group">
              <div className="mesh-payload-label">
                <strong>已生成的分享码</strong>
                <span>复制此分享码到其他设备导入</span>
              </div>
              <div className="mesh-payload-editor">
                <textarea
                  className="mesh-payload-box"
                  value={sharePayload}
                  readOnly
                  placeholder="点击“生成分享码”后显示"
                  spellCheck={false}
                  aria-label="已生成的分享码"
                />
                <button
                  className="mesh-payload-copy"
                  onClick={() => void onCopyShare()}
                  disabled={busy || !sharePayload.trim()}
                  title="复制分享码"
                  aria-label="复制分享码"
                >
                  <Copy size={17} />
                </button>
              </div>
            </div>
            <div className="mesh-payload-group mesh-import-group">
              <div className="mesh-payload-label">
                <strong>导入其他设备的分享码</strong>
                <span>粘贴后建立连接</span>
              </div>
              <textarea
                className="mesh-payload-box"
                value={importPayload}
                onChange={(event) => onImportPayloadChange(event.target.value)}
                placeholder="粘贴另一台设备的分享码"
                spellCheck={false}
                aria-label="导入其他设备的分享码"
              />
              <button className="icon-button mesh-import-button" onClick={() => void onImportShare()} disabled={busy || !importPayload.trim()}>
                <Clipboard size={17} /> 粘贴导入
              </button>
            </div>
          </section>

          </div>

          <section className="panel mesh-migration-panel">
            <div className="panel-header">
              <div>
                <h2>一次性迁移</h2>
                <p>复用现有迁移包，可选账号和会话；默认不包含会话。</p>
              </div>
              <Download size={22} />
            </div>
            <div className="mesh-migration-options">
              <input
                type="password"
                value={migrationPassword}
                onChange={(event) => onMigrationPasswordChange(event.target.value)}
                placeholder={migrationUseMeshSecret ? "使用共享密钥派生密码" : "迁移包密码，可留空明文导出"}
                disabled={migrationUseMeshSecret}
              />
              <label className="checkline">
                <input
                  type="checkbox"
                  checked={migrationUseMeshSecret}
                  onChange={(event) => onMigrationUseMeshSecretChange(event.target.checked)}
                />
                使用共享密钥加密迁移包
              </label>
              <label className="checkline">
                <input
                  type="checkbox"
                  checked={includeConversations}
                  onChange={(event) => onIncludeConversationsChange(event.target.checked)}
                />
                导出会话记录
              </label>
              <label className="checkline">
                <input
                  type="checkbox"
                  checked={restoreConversations}
                  onChange={(event) => onRestoreConversationsChange(event.target.checked)}
                />
                导入时恢复会话
              </label>
            </div>
            <details className="export-selector mesh-export-selector">
              <summary>
                <span>迁移账号</span>
                <strong>{selectedProfiles.length}/{profiles.length}</strong>
              </summary>
              <div className="export-selector-actions">
                <button type="button" className="mini-button" onClick={onSelectAllProfiles}>全选</button>
                <button type="button" className="mini-button" onClick={onClearProfiles}>清空</button>
              </div>
              <div className="export-account-list">
                {profiles.map((profile) => (
                  <label className="export-account-row" key={profile.id}>
                    <input
                      type="checkbox"
                      checked={exportProfileIds.includes(profile.id)}
                      onChange={() => onToggleExportProfile(profile.id)}
                    />
                    <span>
                      <strong>{profile.alias}</strong>
                      <small>{profile.summary.email || profile.summary.accountId || profile.apiConfig?.baseUrl || "未知账号"}</small>
                    </span>
                  </label>
                ))}
              </div>
            </details>
            <div className="action-row">
              <button className="icon-button primary" onClick={() => void onExportMigration()} disabled={busy || selectedProfiles.length === 0}>
                <Download size={17} /> 导出迁移分享包
              </button>
              <button className="icon-button" onClick={() => void onImportMigration()} disabled={busy}>
                <Upload size={17} /> 导入迁移分享包
              </button>
            </div>
          </section>
        </div>

        <div className="mesh-column">
          <section className="panel mesh-node-panel">
            <div className="panel-header">
              <div>
                <h2>公共节点</h2>
                <p>{upNodes}/{status?.publicNodes.length || 0} 在线 · {status?.settings.lastNodeRefreshAt ? formatDate(status.settings.lastNodeRefreshAt) : "未刷新"}</p>
              </div>
              <Wifi size={22} />
            </div>
            {status?.settings.lastNodeRefreshError && (
              <div className="mesh-warning">{status.settings.lastNodeRefreshError}</div>
            )}
            <div className="mesh-node-list">
              {(status?.publicNodes || []).slice(0, 80).map((node) => (
                <article className="mesh-node-row" key={node.id}>
                  <StatusPill ok={node.status === "up"} text={node.status === "up" ? "up" : "down"} />
                  <span>
                    <strong>{node.name}</strong>
                    <small>{node.address}</small>
                  </span>
                  <em>{node.pingMs != null ? `${Math.round(node.pingMs)} ms` : "-"}</em>
                </article>
              ))}
              {(!status || status.publicNodes.length === 0) && <div className="account-empty">暂无节点，点击刷新节点。</div>}
            </div>
          </section>

          <section className="panel mesh-device-panel">
            <div className="panel-header">
              <div>
                <h2>在线设备</h2>
                <p>设备断开后会从这里隐藏，重新连接后自动出现。</p>
              </div>
              <Send size={22} />
            </div>
            <div className="mesh-device-list">
              {onlineDevices.map((device) => (
                <article className="mesh-device-row" key={device.id}>
                  <div>
                    <div className="mesh-device-title">
                      <StatusPill ok text="在线" />
                      <strong>{device.name}</strong>
                    </div>
                    <small className="mesh-device-meta">
                      {device.ip || "虚拟 IP 获取中"}
                      {device.latencyMs != null ? ` · ${Math.round(device.latencyMs)} ms` : ""}
                    </small>
                    {device.autoAccountSync && <small>账号自动同步</small>}
                  </div>
                  <div className="mesh-device-actions">
                  <label className="checkline">
                    <input
                      type="checkbox"
                      checked={device.trusted}
                      onChange={(event) => void onSaveDevice({ ...device, trusted: event.target.checked })}
                    />
                    信任
                  </label>
                  <label className="checkline" title="关闭后，这台设备不会在后台自动接收账号">
                    <input
                      type="checkbox"
                      checked={device.autoAccountSync === true}
                      onChange={(event) => void onSaveDevice({ ...device, autoAccountSync: event.target.checked })}
                    />
                    账号自动同步
                  </label>
                  </div>
                  <ScopeEditor
                    value={device.syncScope}
                    compact
                    onChange={(scope) => void onSaveDevice({ ...device, syncScope: scope })}
                  />
                  <button className="mini-button" title="同步该设备的账号及已选择的规则、路由和会话" onClick={() => void onSyncNow(device.id)} disabled={busy || !device.trusted}>
                    同步此设备
                  </button>
                </article>
              ))}
              {discoveredPeers.map((peer) => (
                <article className="mesh-device-row mesh-peer-discovered" key={`peer-${peer.peerId}`}>
                  <div>
                    <div className="mesh-device-title">
                      <StatusPill ok text="在线" />
                      <strong>{peer.name}</strong>
                    </div>
                    <small className="mesh-device-meta">
                      {peer.ip}
                      {peer.latencyMs != null ? ` · ${Math.round(peer.latencyMs)} ms` : ""}
                    </small>
                  </div>
                  <span className="mesh-peer-note">已连接，未授权同步</span>
                </article>
              ))}
              {onlineDevices.length === 0 && discoveredPeers.length === 0 && (
                <div className="account-empty">暂无在线设备，设备连接后会自动出现。</div>
              )}
            </div>
            <button className="icon-button" onClick={() => void onSyncNow()} disabled={busy || !onlineDevices.some((device) => device.trusted)}>
              <RefreshCcw size={17} /> 同步全部已连接设备
            </button>
          </section>

          <section className="panel mesh-guide-panel">
            <div className="panel-header">
              <div>
                <h2>使用说明</h2>
                <p>按需分享，连接后即可管理其他设备。</p>
              </div>
              <BookOpen size={22} />
            </div>
            <div className="mesh-guide-list">
              <GuideItem icon={<Share2 size={18} />} title="多设备共享" text="在主设备生成分享码，其他设备导入即可建立连接。" />
              <GuideItem icon={<Link2 size={18} />} title="在线设备" text="实时查看连接状态，点击设备即可同步。" />
              <GuideItem icon={<Download size={18} />} title="一次性迁移" text="需要换电脑时，可单独导出并恢复迁移包。" />
              <GuideItem icon={<ShieldCheck size={18} />} title="安全可靠" text="分享内容和账号数据会按选择加密传输。" />
            </div>
          </section>
        </div>
      </div>
    </section>
  );
}

function ScopeEditor({
  value,
  onChange,
  compact = false,
}: {
  value: MeshSyncScope;
  onChange: (value: MeshSyncScope) => void;
  compact?: boolean;
}) {
  const options: Array<{ key: keyof MeshSyncScope; label: string }> = [
    { key: "accounts", label: "账号" },
    { key: "rules", label: "规则" },
    { key: "routing", label: "路由" },
    { key: "conversations", label: "会话" },
  ];
  return (
    <div className={`mesh-scope-editor ${compact ? "compact" : ""}`}>
      {options.map((option) => (
        <label className="checkline" key={option.key}>
          <input
            type="checkbox"
            checked={value[option.key]}
            disabled={compact && option.key === "accounts"}
            onChange={(event) => onChange({ ...value, [option.key]: event.target.checked })}
          />
          {option.label}
        </label>
      ))}
    </div>
  );
}

function GuideItem({
  icon,
  title,
  text,
}: {
  icon: ReactNode;
  title: string;
  text: string;
}) {
  return (
    <div className="mesh-guide-item">
      <span className="mesh-guide-icon">{icon}</span>
      <span>
        <strong>{title}</strong>
        <small>{text}</small>
      </span>
    </div>
  );
}
