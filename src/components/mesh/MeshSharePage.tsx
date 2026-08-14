import {
  CheckCircle2,
  Copy,
  Download,
  KeyRound,
  LoaderCircle,
  Link2,
  Plus,
  Play,
  RefreshCcw,
  Save,
  ShieldOff,
  Square,
  Trash2,
  Upload,
  Users,
  X,
} from "lucide-react";
import { useMemo, useState } from "react";
import {
  DEFAULT_MESH_SYNC_SCOPE,
  type MeshDevice,
  type MeshGroup,
  type MeshGroupStatus,
  type MeshSyncScope,
  type MeshStatus,
  type Profile,
} from "../../types";
import { getMeshI18n, meshText } from "../../i18n/mesh";
import { StatusPill } from "../StatusPill";
import {
  buildMeshGroupRevokeDeviceInput,
  buildMeshRevokeDeviceInput,
  type MeshGroupCreateInput,
  type MeshGroupDeviceSyncInput,
  type MeshGroupImportInput,
  type MeshGroupRevokeDeviceInput,
  type MeshGroupRemoveDeviceInput,
  type MeshGroupShareInput,
  type MeshGroupStartInput,
  type MeshGroupStopInput,
  type MeshGroupSyncInput,
  type MeshGroupSelectInput,
} from "./meshCommands";

export type MeshSharePageProps = {
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
  onImportShare: (payload?: string) => void | Promise<unknown>;
  onSaveDevice: (device: MeshDevice) => void | Promise<unknown>;
  onSyncNow: (deviceId?: string) => void | Promise<unknown>;
  onAuthorizePeer: (ip: string) => void | Promise<unknown>;
  onExportMigration: () => void | Promise<unknown>;
  onImportMigration: () => void | Promise<unknown>;

  /** Group-aware contract for App.tsx. Legacy props above remain during migration. */
  groups?: MeshGroup[];
  groupStatus?: MeshGroupStatus | null;
  selectedGroupId?: string | null;
  onCreateGroup?: (input: MeshGroupCreateInput) => void | Promise<unknown>;
  onImportGroup?: (input: MeshGroupImportInput) => void | Promise<unknown>;
  onSelectGroup?: (input: MeshGroupSelectInput) => void | Promise<unknown>;
  onStartGroup?: (input: MeshGroupStartInput) => void | Promise<unknown>;
  onStopGroup?: (input: MeshGroupStopInput) => void | Promise<unknown>;
  onRevokeDevice?: (input: MeshGroupRevokeDeviceInput) => void | Promise<unknown>;
  onRemoveDevice?: (input: MeshGroupRemoveDeviceInput) => void | Promise<unknown>;
  onSaveGroupDevice?: (input: MeshGroupDeviceSyncInput) => void | Promise<unknown>;
  onSyncGroup?: (input: MeshGroupSyncInput) => void | Promise<unknown>;
  onCreateGroupShare?: (input: MeshGroupShareInput) => void | Promise<unknown>;
  onSaveGroupScope?: (groupId: string, scope: MeshSyncScope) => void | Promise<unknown>;
};

const LEGACY_GROUP_ID = "legacy";

export function MeshSharePage(props: MeshSharePageProps) {
  const t = getMeshI18n();
  const [dialog, setDialog] = useState<"create" | "join" | null>(null);
  const [groupName, setGroupName] = useState("");
  const [shareCode, setShareCode] = useState("");
  const [actionError, setActionError] = useState("");
  const [syncingDeviceId, setSyncingDeviceId] = useState<string | null>(null);
  const [removeConfirmDeviceId, setRemoveConfirmDeviceId] = useState<string | null>(null);

  const statusDevices = props.groupStatus?.devices ?? props.status?.devices ?? [];
  const localDeviceId = props.groupStatus?.localDeviceId ?? props.status?.localDeviceId;
  const remoteDevices = useMemo(
    () => statusDevices.filter((device) => device.id !== localDeviceId),
    [statusDevices, localDeviceId],
  );
  const backendGroups = props.groups ?? [];
  const usingLegacyGroup = backendGroups.length === 0;
  const effectiveGroups: MeshGroup[] = usingLegacyGroup
    ? [{
        id: LEGACY_GROUP_ID,
        name: t.defaultGroup,
        runtimeStatus: props.status?.running ? "running" : "stopped",
        onlineDeviceCount: remoteDevices.filter((device) => device.online).length,
        deviceCount: remoteDevices.length,
        selected: true,
        lastError: props.status?.lastError,
      }]
    : backendGroups;
  const activeGroupId = usingLegacyGroup
    ? LEGACY_GROUP_ID
    : props.selectedGroupId
      ?? effectiveGroups.find((group) => group.selected)?.id
      ?? effectiveGroups[0]?.id;
  const activeGroup = effectiveGroups.find((group) => group.id === activeGroupId) ?? effectiveGroups[0];
  const groupDevices = remoteDevices;
  const selectedProfiles = props.profiles.filter((profile) => props.exportProfileIds.includes(profile.id));
  const activeIsLegacy = usingLegacyGroup || activeGroup?.id === LEGACY_GROUP_ID;
  const canUseGroupCommands = !activeIsLegacy;

  async function runAction(action: () => void | Promise<unknown>, onSuccess?: () => void) {
    setActionError("");
    try {
      await action();
      onSuccess?.();
    } catch (error) {
      setActionError(error instanceof Error ? error.message : t.actionFailed);
    }
  }

  function selectGroup(group: MeshGroup) {
    if (group.id === activeGroup?.id || !props.onSelectGroup) return;
    void runAction(() => props.onSelectGroup!({ groupId: group.id }));
  }

  function toggleGroupRuntime() {
    if (!activeGroup) return;
    const running = activeGroup.runtimeStatus === "running" || activeGroup.runtimeStatus === "starting";
    if (canUseGroupCommands) {
      const callback = running ? props.onStopGroup : props.onStartGroup;
      if (callback) {
        void runAction(() => callback({ groupId: activeGroup.id }));
      }
      return;
    }
    void runAction(props.onToggleService);
  }

  async function syncDevice(deviceId?: string) {
    if (!activeGroup) return;
    const syncKey = deviceId ?? "__all__";
    setSyncingDeviceId(syncKey);
    try {
      if (canUseGroupCommands) {
        if (props.onSyncGroup) {
          await runAction(() => props.onSyncGroup!({ groupId: activeGroup.id, deviceId }));
        }
        return;
      }
      await runAction(() => props.onSyncNow(deviceId));
    } finally {
      setSyncingDeviceId(null);
    }
  }

  function toggleDeviceRevocation(device: MeshDevice) {
    if (!activeGroup) return;
    const revoked = deviceIsRevoked(device);
    if (canUseGroupCommands) {
      if (props.onRevokeDevice) {
        const input = buildMeshGroupRevokeDeviceInput(activeGroup.id, device, !revoked);
        void runAction(() => props.onRevokeDevice!(input));
      }
      return;
    }
    if (revoked) {
      void runAction(() => props.onSaveDevice({ ...device, trusted: true }));
    } else {
      const input = buildMeshRevokeDeviceInput(device);
      void runAction(() => props.onSaveDevice({ ...device, ...input }));
    }
  }

  function removeDevice(device: MeshDevice) {
    if (!activeGroup || !props.onRemoveDevice) return;
    if (removeConfirmDeviceId !== device.id) {
      setRemoveConfirmDeviceId(device.id);
      return;
    }
    void runAction(
      () => props.onRemoveDevice!({ groupId: activeGroup.id, deviceId: device.id }),
      () => setRemoveConfirmDeviceId(null),
    );
  }

  function saveDeviceSelection(device: MeshDevice, syncScope: MeshSyncScope, autoAccountSync = device.autoAccountSync === true) {
    if (!activeGroup) return;
    if (canUseGroupCommands && props.onSaveGroupDevice) {
      void runAction(() => props.onSaveGroupDevice!({
        groupId: activeGroup.id,
        deviceId: device.id,
        autoAccountSync,
        syncScope,
      }));
      return;
    }
    void runAction(() => props.onSaveDevice({ ...device, syncScope, autoAccountSync }));
  }

  function submitCreateGroup() {
    if (!props.onCreateGroup || !groupName.trim()) return;
    void runAction(
      () => props.onCreateGroup!({ name: groupName.trim(), syncScope: DEFAULT_MESH_SYNC_SCOPE }),
      () => {
        setGroupName("");
        setDialog(null);
      },
    );
  }

  function submitJoinGroup() {
    if (!shareCode.trim()) return;
    const action = props.onImportGroup
      ? () => props.onImportGroup!({ shareCode: shareCode.trim() })
      : () => props.onImportShare(shareCode.trim());
    void runAction(action, () => {
      setShareCode("");
      setGroupName("");
      setDialog(null);
    });
  }

  const running = activeGroup?.runtimeStatus === "running";
  const runtimePending = activeGroup?.runtimeStatus === "starting" || activeGroup?.runtimeStatus === "stopping";
  const groupActionAvailable = activeIsLegacy
    || (running ? Boolean(props.onStopGroup) : Boolean(props.onStartGroup));
  const syncAvailable = activeIsLegacy || Boolean(props.onSyncGroup);
  const revokeAvailable = activeIsLegacy || Boolean(props.onRevokeDevice);
  const removeAvailable = Boolean(props.onRemoveDevice);

  function deviceIsRevoked(device: MeshDevice) {
    return Boolean(device.revokedAt) || (activeIsLegacy && !device.trusted);
  }

  return (
    <section className="mesh-page mesh-groups-page">
      <section className="panel mesh-groups-panel">
        <div className="panel-header mesh-groups-header">
          <div>
            <span className="section-kicker">{t.pageKicker}</span>
            <h2>{t.groupsTitle}</h2>
            <p>{t.groupsHint}</p>
          </div>
          <div className="action-row mesh-group-header-actions">
            <button
              className="icon-button"
              onClick={() => { setActionError(""); setGroupName(""); setDialog("create"); }}
              disabled={props.busy || !props.onCreateGroup}
              title={!props.onCreateGroup ? t.waitingForGroupCommands : undefined}
            >
              <Plus size={17} /> {t.createGroup}
            </button>
            <button
              className="icon-button primary"
              onClick={() => { setActionError(""); setGroupName(""); setShareCode(""); setDialog("join"); }}
              disabled={props.busy}
            >
              <Link2 size={17} /> {t.joinGroup}
            </button>
          </div>
        </div>

        <div className="mesh-group-list" aria-label={t.groupsTitle}>
          {effectiveGroups.map((group) => {
            const selected = group.id === activeGroup?.id;
            const groupRunning = group.runtimeStatus === "running";
            return (
              <div
                className={`mesh-group-card ${selected ? "active" : ""}`}
                key={group.id}
              >
                <button
                  type="button"
                  className="mesh-group-card-select"
                  onClick={() => selectGroup(group)}
                  disabled={props.busy || (!selected && !props.onSelectGroup)}
                >
                  <span className="mesh-group-card-icon"><Users size={19} /></span>
                  <span className="mesh-group-card-main">
                  <span className="mesh-group-card-title">
                    <strong>{group.name}</strong>
                    {selected && <span className="mesh-group-selected">{t.activeGroup}</span>}
                  </span>
                  <span className="mesh-group-card-meta">
                    <span>{groupRunning ? t.running : group.runtimeStatus === "error" ? t.runtimeError : t.stopped}</span>
                    <span>{meshText(t.onlineDevices, { count: String(group.onlineDeviceCount) })}</span>
                  </span>
                  </span>
                  {selected && <CheckCircle2 className="mesh-group-card-check" size={18} />}
                </button>
                {selected && <button className={`icon-button ${groupRunning ? "" : "primary"}`} onClick={toggleGroupRuntime} disabled={props.busy || runtimePending || !groupActionAvailable}>
                  {groupRunning ? <Square size={16} /> : <Play size={16} />}
                  {runtimePending ? t.processing : groupRunning ? t.stopGroup : t.startGroup}
                </button>}
              </div>
            );
          })}
        </div>

      </section>

      {(actionError || activeGroup?.lastError || props.status?.lastError) && (
        <div className="mesh-warning">{actionError || activeGroup?.lastError || props.status?.lastError}</div>
      )}

      <div className="mesh-columns mesh-group-content">
        <div className="mesh-column">
          <section className="panel mesh-device-panel">
            <div className="panel-header">
              <div>
                <h2>{t.deviceListTitle}</h2>
                <p>{activeGroup ? meshText(t.currentGroupSummary, { name: activeGroup.name }) : t.noGroups}</p>
              </div>
              <Users size={22} />
            </div>
            <div className="mesh-device-list">
              {groupDevices.map((device) => {
                const revoked = deviceIsRevoked(device);
                const removing = removeConfirmDeviceId === device.id;
                return (
                  <article className={`mesh-device-row ${revoked ? "revoked" : ""}`} key={device.id}>
                    <div>
                      <div className="mesh-device-title">
                        <StatusPill
                          ok={!revoked && device.online === true}
                          text={revoked ? t.deviceStatusRevoked : device.online ? t.deviceStatusOnline : t.deviceStatusOffline}
                        />
                        <strong>{device.name}</strong>
                      </div>
                      <small className="mesh-device-meta">
                        {revoked ? t.revocationPersisted : device.trusted ? t.deviceStatusAuthorized : t.deviceStatusPending}
                        {!revoked && device.ip ? ` · ${device.ip}` : ""}
                        {!revoked && device.latencyMs != null ? ` · ${Math.round(device.latencyMs)} ms` : ""}
                      </small>
                    </div>
                    <div className="mesh-device-sync-options">
                      <div className="mesh-device-sync-heading">{t.allowedSyncContents}</div>
                      <ScopeEditor
                        compact
                        value={device.syncScope}
                        allowed={device.allowedSyncScope}
                        disabled={revoked || !device.trusted}
                        onChange={(scope) => saveDeviceSelection(device, scope)}
                      />
                      {device.allowedSyncScope.accounts && (
                        <label className="checkline mesh-device-auto-sync">
                          <input
                            type="checkbox"
                            checked={device.autoAccountSync === true}
                            disabled={revoked || !device.trusted}
                            onChange={(event) => saveDeviceSelection(device, device.syncScope, event.target.checked)}
                          />
                          <span>
                            <strong>{t.autoSyncAccounts}</strong>
                            <small>{t.autoSyncAccountsHint}</small>
                          </span>
                        </label>
                      )}
                    </div>
                    <div className="mesh-device-actions">
                      <button
                        type="button"
                        className="mini-button"
                        onClick={() => syncDevice(device.id)}
                        disabled={props.busy || syncingDeviceId !== null || !device.online || revoked || !device.trusted || !syncAvailable}
                        title={!device.online ? t.deviceStatusOffline : undefined}
                      >
                        {syncingDeviceId === device.id ? <LoaderCircle className="button-spinner" size={14} /> : <RefreshCcw size={14} />} {syncingDeviceId === device.id ? t.processing : t.syncDevice}
                      </button>
                      <button
                        type="button"
                        className={`mini-button ${revoked ? "" : "danger"}`}
                        onClick={() => toggleDeviceRevocation(device)}
                        disabled={props.busy || !revokeAvailable}
                      >
                        {revoked ? <CheckCircle2 size={14} /> : <ShieldOff size={14} />}
                        {revoked ? t.restoreDevice : t.revokeDevice}
                      </button>
                      {removeAvailable && (
                        <>
                          {removing && (
                            <button
                              type="button"
                              className="mini-button"
                              onClick={() => setRemoveConfirmDeviceId(null)}
                              disabled={props.busy}
                            >
                              <X size={14} /> {t.cancel}
                            </button>
                          )}
                          <button
                            type="button"
                            className="mini-button danger"
                            onClick={() => removeDevice(device)}
                            disabled={props.busy}
                            title={removing ? t.removeDeviceConfirm : t.removeDevice}
                          >
                            <Trash2 size={14} /> {removing ? t.confirmRemoveDevice : t.removeDevice}
                          </button>
                        </>
                      )}
                    </div>
                  </article>
                );
              })}
              {groupDevices.length === 0 && <div className="account-empty">{t.noGroupDevices}</div>}
            </div>
            <button
              className="icon-button"
              onClick={() => syncDevice()}
              disabled={props.busy || syncingDeviceId !== null || !syncAvailable || !groupDevices.some((device) => device.online && device.trusted && !deviceIsRevoked(device))}
            >
              {syncingDeviceId === "__all__" ? <LoaderCircle className="button-spinner" size={17} /> : <RefreshCcw size={17} />} {syncingDeviceId === "__all__" ? t.processing : t.syncAllDevices}
            </button>
          </section>
        </div>

        <div className="mesh-column">
          <section className="panel mesh-share-panel">
            <div className="panel-header">
              <div>
                <h2>{t.sharingTitle}</h2>
                <p>{t.sharingHint}</p>
              </div>
              <KeyRound size={22} />
            </div>
            <div className="mesh-share-scope-hint">
              <strong>{t.shareContents}</strong>
              <span>{t.defaultScopeHint}</span>
            </div>
            <ScopeEditor value={props.syncScope} onChange={props.onSyncScopeChange} />
            <div className="action-row">
              <button
                className="icon-button"
                onClick={() => {
                  if (activeGroup && canUseGroupCommands && props.onSaveGroupScope) {
                    void runAction(() => props.onSaveGroupScope!(activeGroup.id, props.syncScope));
                  } else if (activeIsLegacy) {
                    void runAction(props.onSaveSettings);
                  }
                }}
                disabled={props.busy || (canUseGroupCommands ? !props.onSaveGroupScope : false)}
              >
                <Save size={17} /> {t.saveShareContents}
              </button>
              <button
                className="icon-button primary"
                onClick={() => {
                  if (activeGroup && canUseGroupCommands && props.onCreateGroupShare) {
                    void runAction(() => props.onCreateGroupShare!({ groupId: activeGroup.id, mode: "joinOnly" }));
                  } else if (activeIsLegacy) {
                    void runAction(props.onCreateShare);
                  }
                }}
                disabled={props.busy || (canUseGroupCommands ? !props.onCreateGroupShare : false)}
              >
                <KeyRound size={17} /> {t.createShareCode}
              </button>
              <button className="icon-button" onClick={() => void props.onCopyShare()} disabled={props.busy || !props.sharePayload.trim()}>
                <Copy size={17} /> {t.copy}
              </button>
            </div>
            <textarea
              className="mesh-payload-box"
              value={props.sharePayload}
              readOnly
              placeholder={t.shareCodePlaceholder}
              aria-label={t.shareCodeLabel}
              spellCheck={false}
            />
          </section>

          <section className="panel mesh-migration-panel">
            <div className="panel-header">
              <div>
                <h2>{t.migrationTitle}</h2>
                <p>{t.migrationHint}</p>
              </div>
              <Download size={22} />
            </div>
            <div className="mesh-migration-options">
              <input
                type="password"
                value={props.migrationPassword}
                onChange={(event) => props.onMigrationPasswordChange(event.target.value)}
                placeholder={props.migrationUseMeshSecret ? t.groupCredentialPlaceholder : t.passwordPlaceholder}
                disabled={props.migrationUseMeshSecret}
              />
              <label className="checkline">
                <input
                  type="checkbox"
                  checked={props.migrationUseMeshSecret}
                  onChange={(event) => props.onMigrationUseMeshSecretChange(event.target.checked)}
                />
                {t.useGroupCredential}
              </label>
              <label className="checkline">
                <input
                  type="checkbox"
                  checked={props.includeConversations}
                  onChange={(event) => props.onIncludeConversationsChange(event.target.checked)}
                />
                {t.includeConversations}
              </label>
              <label className="checkline">
                <input
                  type="checkbox"
                  checked={props.restoreConversations}
                  onChange={(event) => props.onRestoreConversationsChange(event.target.checked)}
                />
                {t.restoreConversations}
              </label>
            </div>
            <details className="export-selector mesh-export-selector">
              <summary><span>{t.migrationAccounts}</span><strong>{selectedProfiles.length}/{props.profiles.length}</strong></summary>
              <div className="export-selector-actions">
                <button type="button" className="mini-button" onClick={props.onSelectAllProfiles}>{t.selectAll}</button>
                <button type="button" className="mini-button" onClick={props.onClearProfiles}>{t.clear}</button>
              </div>
              <div className="export-account-list">
                {props.profiles.map((profile) => (
                  <label className="export-account-row" key={profile.id}>
                    <input
                      type="checkbox"
                      checked={props.exportProfileIds.includes(profile.id)}
                      onChange={() => props.onToggleExportProfile(profile.id)}
                    />
                    <span>
                      <strong>{profile.alias}</strong>
                      <small>{profile.summary.email || profile.summary.accountId || profile.apiConfig?.baseUrl || t.unknownAccount}</small>
                    </span>
                  </label>
                ))}
              </div>
            </details>
            <div className="action-row">
              <button className="icon-button primary" onClick={() => void props.onExportMigration()} disabled={props.busy || selectedProfiles.length === 0}>
                <Download size={17} /> {t.exportMigration}
              </button>
              <button className="icon-button" onClick={() => void props.onImportMigration()} disabled={props.busy}>
                <Upload size={17} /> {t.importMigration}
              </button>
            </div>
          </section>
        </div>
      </div>

      {dialog && (
        <div className="update-dialog-backdrop" role="presentation" onMouseDown={() => setDialog(null)}>
          <section className="mesh-add-device-dialog mesh-join-group-dialog" role="dialog" aria-modal="true" onMouseDown={(event) => event.stopPropagation()}>
            <div className="update-dialog-head">
              <div><span>{t.groupsTitle}</span><h2>{dialog === "create" ? t.createGroupTitle : t.joinGroupTitle}</h2></div>
              <button className="notice-close" onClick={() => setDialog(null)} title={t.close}><X size={18} /></button>
            </div>
            <p className="mesh-add-device-hint">{dialog === "create" ? t.createGroupHint : t.joinGroupHint}</p>
            {dialog === "create" && (
              <label className="mesh-group-dialog-field">
                <span>{t.groupNameLabel}</span>
                <input value={groupName} onChange={(event) => setGroupName(event.target.value)} placeholder={t.groupNamePlaceholder} autoFocus />
              </label>
            )}
            {dialog === "join" && (
              <label className="mesh-group-dialog-field">
                <span>{t.groupShareCodeLabel}</span>
                <textarea className="mesh-payload-box" value={shareCode} onChange={(event) => setShareCode(event.target.value)} placeholder={t.groupShareCodePlaceholder} spellCheck={false} autoFocus />
              </label>
            )}
            {actionError && <div className="mesh-warning">{actionError}</div>}
            <div className="update-dialog-actions">
              <button className="icon-button" onClick={() => setDialog(null)}>{t.cancel}</button>
              <button
                className="icon-button primary"
                onClick={dialog === "create" ? submitCreateGroup : submitJoinGroup}
                disabled={props.busy || (dialog === "create" ? !groupName.trim() : !shareCode.trim())}
              >
                {dialog === "create" ? <Plus size={17} /> : <Link2 size={17} />}
                {dialog === "create" ? t.create : t.join}
              </button>
            </div>
          </section>
        </div>
      )}
    </section>
  );
}

function ScopeEditor({
  value,
  allowed,
  onChange,
  compact = false,
  disabled = false,
}: {
  value: MeshSyncScope;
  allowed?: MeshSyncScope;
  onChange: (value: MeshSyncScope) => void;
  compact?: boolean;
  disabled?: boolean;
}) {
  const t = getMeshI18n();
  const options: Array<{ key: keyof MeshSyncScope; label: string }> = [
    { key: "accounts", label: t.scopeAccounts },
    { key: "rules", label: t.scopeRules },
    { key: "routing", label: t.scopeRouting },
    { key: "conversations", label: t.scopeConversations },
  ];
  return (
    <div className={`mesh-scope-editor ${compact ? "compact" : ""}`}>
      {options.map((option) => (
        <label className="checkline" key={option.key}>
          <input
            type="checkbox"
            checked={value[option.key]}
            disabled={disabled || (allowed != null && !allowed[option.key])}
            onChange={(event) => onChange({ ...value, [option.key]: event.target.checked })}
          />
          {option.label}
        </label>
      ))}
    </div>
  );
}
