import {
  Clipboard,
  Copy,
  Download,
  KeyRound,
  Network,
  PlugZap,
  RefreshCcw,
  Save,
  Send,
  Share2,
  Upload,
  Wifi,
} from "lucide-react";
import type { MeshDevice, MeshShareMode, MeshStatus, MeshSyncScope, Profile } from "../../types";
import { formatDate } from "../../profileUtils";
import { StatusPill } from "../StatusPill";

const shareModes: Array<{ id: MeshShareMode; title: string; text: string }> = [
  { id: "joinOnly", title: "仅加入组网", text: "只共享 EasyTier 网络信息，不同步账号。" },
  { id: "migrationBundle", title: "一次性迁移", text: "复用加密迁移包，适合换电脑。" },
  { id: "continuousSync", title: "持续同步", text: "按设备勾选账号、规则、路由和会话。" },
  { id: "routingApiShare", title: "路由 API 共享", text: "共享本机路由入口，仍需要 API Key。" },
];

type MeshSharePageProps = {
  status: MeshStatus | null;
  profiles: Profile[];
  busy: boolean;
  shareMode: MeshShareMode;
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
  onShareModeChange: (mode: MeshShareMode) => void;
  onSharePayloadChange: (value: string) => void;
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
  shareMode,
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
  onShareModeChange,
  onSharePayloadChange,
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

  return (
    <section className="mesh-page">
      <div className="mesh-hero panel">
        <div>
          <span className="section-kicker">EasyTier Mesh</span>
          <h2>组网分享与迁移</h2>
          <p>统一管理共享密钥、设备同步、一次性迁移包和路由 API 入口。</p>
        </div>
        <div className="mesh-hero-status">
          <StatusPill ok={!!status?.running} text={status?.running ? "运行中" : "已停止"} />
          <strong>{status?.localDeviceName || "-"}</strong>
          <small>{status?.localDeviceId || "等待初始化"}</small>
          {status?.runtimeKind && <small>{status.runtimeKind}</small>}
          {status?.peerCount != null && <small>{status.peerCount} 个 EasyTier peer 已注入</small>}
          {status?.virtualIpv4 && <small>{status.virtualIpv4}</small>}
          {status?.routingBaseUrl && <small>Routing API: {status.routingBaseUrl}</small>}
          {status?.startedAt && <small>{formatDate(status.startedAt)}</small>}
        </div>
      </div>
      {status?.lastError && <div className="mesh-warning">{status.lastError}</div>}

      <div className="mesh-mode-grid">
        {shareModes.filter((mode) => mode.id !== "routingApiShare").map((mode) => (
          <button
            key={mode.id}
            className={`mesh-mode-card ${shareMode === mode.id ? "active" : ""}`}
            onClick={() => onShareModeChange(mode.id)}
            type="button"
            disabled={busy}
          >
            <strong>{mode.title}</strong>
            <span>{mode.text}</span>
          </button>
        ))}
      </div>

      <div className="mesh-columns">
        <div className="mesh-column">
          <section className="panel mesh-settings-panel">
            <div className="panel-header">
              <div>
                <h2>网络设置</h2>
                <p>公共节点只负责连通，账号数据仍由应用层加密。</p>
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
                启动应用时自动组网
              </label>
            </div>
            <ScopeEditor value={syncScope} onChange={onSyncScopeChange} />
            <div className="action-row">
              <button className="icon-button primary" onClick={() => void onSaveSettings()} disabled={busy}>
                <Save size={17} /> 保存设置
              </button>
              <button className="icon-button" onClick={() => void onToggleService()} disabled={busy}>
                <PlugZap size={17} /> {status?.running ? "停止组网" : "启动组网"}
              </button>
              <button className="icon-button" onClick={() => void onRefreshNodes()} disabled={busy}>
                <RefreshCcw size={17} /> 刷新节点
              </button>
            </div>
          </section>

          <section className="panel mesh-share-panel">
            <div className="panel-header">
              <div>
                <h2>共享密钥</h2>
                <p>复制给另一台设备，或粘贴其他设备发来的共享码。</p>
              </div>
              <Share2 size={22} />
            </div>
            <div className="action-row">
              <button className="icon-button primary" onClick={() => void onCreateShare()} disabled={busy}>
                <KeyRound size={17} /> 生成当前模式共享码
              </button>
              <button className="icon-button" onClick={() => void onCopyShare()} disabled={busy || !sharePayload.trim()}>
                <Copy size={17} /> 复制
              </button>
            </div>
            <textarea
              className="mesh-payload-box"
              value={sharePayload}
              onChange={(event) => onSharePayloadChange(event.target.value)}
              placeholder="生成后的 codex-switcher-mesh:... 会显示在这里"
              spellCheck={false}
            />
            <textarea
              className="mesh-payload-box"
              value={importPayload}
              onChange={(event) => onImportPayloadChange(event.target.value)}
              placeholder="粘贴另一台设备的共享码"
              spellCheck={false}
            />
            <button className="icon-button" onClick={() => void onImportShare()} disabled={busy || !importPayload.trim()}>
              <Clipboard size={17} /> 粘贴导入
            </button>
          </section>

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
                placeholder={migrationUseMeshSecret ? "使用组网密钥派生密码" : "迁移包密码，可留空明文导出"}
                disabled={migrationUseMeshSecret}
              />
              <label className="checkline">
                <input
                  type="checkbox"
                  checked={migrationUseMeshSecret}
                  onChange={(event) => onMigrationUseMeshSecretChange(event.target.checked)}
                />
                使用组网密钥加密迁移包
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
                <h2>授权设备</h2>
                <p>每台设备可独立选择是否同步账号、规则、路由和会话。</p>
              </div>
              <Send size={22} />
            </div>
            <div className="mesh-device-list">
              {(status?.devices || []).map((device) => (
                <article className="mesh-device-row" key={device.id}>
                  <div>
                    <strong>{device.name}</strong>
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
                  <button className="mini-button" title="仅手动同步规则、路由和会话；账号由组网后台自动同步" onClick={() => void onSyncNow(device.id)} disabled={busy || !device.trusted}>
                    立即同步
                  </button>
                </article>
              ))}
              {(!status || status.devices.length === 0) && <div className="account-empty">还没有授权设备。导入共享码后会出现在这里。</div>}
            </div>
            <button className="icon-button" onClick={() => void onSyncNow()} disabled={busy || !status?.devices.some((device) => device.trusted)}>
              <RefreshCcw size={17} /> 同步全部信任设备
            </button>
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
