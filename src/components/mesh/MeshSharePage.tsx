import {
  CheckCircle2,
  Copy,
  Download,
  MoreHorizontal,
  File,
  Folder,
  KeyRound,
  LoaderCircle,
  Link2,
  Plus,
  Play,
  RefreshCcw,
  ShieldOff,
  Square,
  Trash2,
  Upload,
  Users,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Button, Card, Checkbox, Input, Switch, Collapse, Dropdown, Tree, Breadcrumb, Modal, Alert, Select, type TreeDataNode } from "antd";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { logAppError } from "../../api/appLog";
import {
  DEFAULT_MESH_SYNC_SCOPE,
  type MeshDevice,
  type MeshFileEntry,
  type MeshFileShareView,
  type MeshGroup,
  type MeshGroupStatus,
  type MeshSyncScope,
  type MeshStatus,
  type Profile,
} from "../../types";
import { getMeshI18n, meshText } from "../../i18n/mesh";

function isTauriRuntime() {
  return Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);
}
import { StatusPill } from "../StatusPill";
import { RemoteFilesPanel } from "./RemoteFilesPanel";
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
  onDeleteGroup?: (input: MeshGroupSelectInput) => void | Promise<unknown>;
  onRevokeDevice?: (input: MeshGroupRevokeDeviceInput) => void | Promise<unknown>;
  onRemoveDevice?: (input: MeshGroupRemoveDeviceInput) => void | Promise<unknown>;
  onSaveGroupDevice?: (input: MeshGroupDeviceSyncInput) => void | Promise<unknown>;
  onSyncGroup?: (input: MeshGroupSyncInput) => void | Promise<unknown>;
  onCreateGroupShare?: (input: MeshGroupShareInput) => void | Promise<unknown>;
  onSaveGroupScope?: (groupId: string, scope: MeshSyncScope) => void | Promise<unknown>;
  onGroupFilesChanged?: (status: MeshGroupStatus) => void;
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
  const [selectedShareId, setSelectedShareId] = useState<string>("");
  const [aiGuideCopied, setAiGuideCopied] = useState(false);
  const [aiTargetDeviceId, setAiTargetDeviceId] = useState("");
  const [expandedLocalKeys, setExpandedLocalKeys] = useState<string[]>([]);
  const [localFiles, setLocalFiles] = useState<MeshFileEntry[]>([]);
  const [remoteFiles, setRemoteFiles] = useState<MeshFileEntry[]>([]);
  const [filePath, setFilePath] = useState("");
  const [remoteDevice, setRemoteDevice] = useState<MeshDevice | null>(null);
  const [remoteShares, setRemoteShares] = useState<MeshFileShareView[]>([]);
  const [remoteShareId, setRemoteShareId] = useState("");
  const [remoteFilePath, setRemoteFilePath] = useState("");
  const [downloadProgress, setDownloadProgress] = useState<Record<string, { bytes: number; total?: number }>>({});
  const [localDirectoryNodes, setLocalDirectoryNodes] = useState<Record<string, MeshFileEntry[]>>({});
  const [remoteDirectoryNodes, setRemoteDirectoryNodes] = useState<Record<string, MeshFileEntry[]>>({});
  const [expandedRemoteKeys, setExpandedRemoteKeys] = useState<string[]>([]);
  const createLocalTreeNodes = (files: MeshFileEntry[]): TreeDataNode[] => files.map((file) => ({
    key: file.id,
    title: <Dropdown trigger={["click", "contextMenu"]} menu={{ items: file.kind === "directory"
      ? [{ key: "browse", label: t.browseFiles, onClick: () => void browseShare(selectedShareId, file.id) }]
      : [{ key: "download", label: t.downloadFile, icon: <Download size={14} />, onClick: () => void downloadSharedFile(file) }] }}>
      <span className="mesh-file-tree-title">{file.name}{file.kind === "file" && <small>{formatFileSize(file.size)}</small>}</span>
    </Dropdown>,
    isLeaf: file.kind !== "directory",
    icon: file.kind === "directory" ? <Folder size={16} /> : <File size={16} />,
    children: localDirectoryNodes[file.id] ? createLocalTreeNodes(localDirectoryNodes[file.id]) : undefined,
  }));
  function formatFileSize(bytes: number) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
  }

  const localDirectoryTree = useMemo(() => createLocalTreeNodes(localFiles), [localFiles, localDirectoryNodes, selectedShareId, t]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let unlisten: (() => void) | undefined;
    void listen<{ transferId: string; bytes: number; total?: number }>("mesh-file-download-progress", (event) => {
      setDownloadProgress((current) => ({ ...current, [event.payload.transferId]: event.payload }));
    }).then((stop) => { unlisten = stop; });
    return () => unlisten?.();
  }, []);

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
  const aiDevices = props.groupStatus?.fileAccess?.aiDevices ?? [];
  const selectedProfiles = props.profiles.filter((profile) => props.exportProfileIds.includes(profile.id));
  const activeIsLegacy = usingLegacyGroup || activeGroup?.id === LEGACY_GROUP_ID;

  useEffect(() => {
    if (!aiDevices.some((device) => device.id === aiTargetDeviceId)) {
      setAiTargetDeviceId(aiDevices[0]?.id ?? "");
    }
  }, [aiDevices.map((device) => device.id).join("|"), aiTargetDeviceId]);
  const canUseGroupCommands = !activeIsLegacy;

  useEffect(() => {
    setSelectedShareId("");
    setFilePath("");
    setLocalFiles([]);
    setRemoteFiles([]);
    setRemoteDevice(null);
    setRemoteShares([]);
    setRemoteShareId("");
    setRemoteFilePath("");
  }, [activeGroupId]);

  async function runAction(action: () => void | Promise<unknown>, onSuccess?: () => void) {
    setActionError("");
    try {
      await action();
      onSuccess?.();
    } catch (error) {
      void logAppError("mesh_share_action", error);
      setActionError(error instanceof Error ? error.message : typeof error === "string" ? error : t.actionFailed);
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

  function changeShareScope(syncScope: MeshSyncScope) {
    props.onSyncScopeChange(syncScope);
    if (activeGroup && canUseGroupCommands && props.onSaveGroupScope) {
      void runAction(() => props.onSaveGroupScope!(activeGroup.id, syncScope));
    } else if (activeIsLegacy) {
      void runAction(props.onSaveSettings);
    }
  }

  async function addFileShare(directory: boolean) {
    if (!activeGroup) return;
    await runAction(async () => {
      const selected = await openDialog({ multiple: !directory, directory, title: directory ? t.addFolder : t.addFiles });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      for (const path of paths) {
        const kind = directory ? "directory" : "file";
        const status = await invoke<MeshGroupStatus>("mesh_group_add_file_share", { groupId: activeGroup.id, path, kind });
        props.onGroupFilesChanged?.(status);
      }
    });
  }

  async function browseShare(shareId: string, relative = "") {
    if (!activeGroup) return;
    if (!relative && selectedShareId === shareId) {
      setSelectedShareId("");
      setFilePath("");
      setLocalFiles([]);
      setLocalDirectoryNodes({});
      setExpandedLocalKeys([]);
      return;
    }
    setSelectedShareId(shareId);
    setFilePath(relative);
    setLocalDirectoryNodes({});
    setExpandedLocalKeys([]);
    await runAction(async () => {
      const files = await invoke<MeshFileEntry[]>("mesh_group_list_files", { groupId: activeGroup.id, shareId, path: relative });
      setLocalFiles(files);
    });
  }

  async function loadLocalDirectory(shareId: string, directoryId: string) {
    if (localDirectoryNodes[directoryId] || !activeGroup) return;
    const files = await invoke<MeshFileEntry[]>("mesh_group_list_files", {
      groupId: activeGroup.id,
      shareId,
      path: directoryId,
    });
    setLocalDirectoryNodes((current) => ({ ...current, [directoryId]: files }));
  }

  async function downloadSharedFile(file: MeshFileEntry) {
    if (!activeGroup || !selectedShareId) return;
    const destination = await saveDialog({ defaultPath: file.name, title: t.downloadFile });
    if (!destination) return;
    await runAction(() => invoke("mesh_group_download_file", {
      groupId: activeGroup.id,
      shareId: selectedShareId,
      path: file.id,
      destination,
    }));
  }

  async function browseDeviceShares(device: MeshDevice) {
    if (!activeGroup) return;
    setRemoteDevice(device);
    setRemoteShareId("");
    setRemoteFiles([]);
    await runAction(async () => {
      const shares = await invoke<MeshFileShareView[]>("mesh_group_remote_file_shares", { groupId: activeGroup.id, deviceId: device.id });
      setRemoteShares(shares);
    });
  }

  async function browseRemoteFiles(shareId: string, path = "") {
    if (!activeGroup || !remoteDevice) return;
    setRemoteShareId(shareId);
    setRemoteFilePath(path);
    setRemoteDirectoryNodes({});
    setExpandedRemoteKeys([]);
    await runAction(async () => {
      const files = await invoke<MeshFileEntry[]>("mesh_group_remote_files", { groupId: activeGroup.id, deviceId: remoteDevice.id, shareId, path });
      setRemoteFiles(files);
    });
  }

  async function loadRemoteDirectory(shareId: string, path: string) {
    if (!activeGroup || !remoteDevice) return [];
    return invoke<MeshFileEntry[]>("mesh_group_remote_files", {
      groupId: activeGroup.id,
      deviceId: remoteDevice.id,
      shareId,
      path,
    });
  }

  async function downloadRemoteFile(file: MeshFileEntry) {
    if (!activeGroup || !remoteDevice || !remoteShareId) return;
    const destination = await saveDialog({ defaultPath: file.name, title: t.downloadFile });
    if (!destination) return;
    const transferId = `${activeGroup.id}:${remoteDevice.id}:${remoteShareId}:${file.id}`;
    setDownloadProgress((current) => ({ ...current, [transferId]: { bytes: 0, total: file.size } }));
    await runAction(() => invoke("mesh_group_download_remote_file", {
      groupId: activeGroup.id, deviceId: remoteDevice.id, shareId: remoteShareId, path: file.id, destination,
    }));
    setDownloadProgress((current) => { const next = { ...current }; delete next[transferId]; return next; });
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
      <Card className="mesh-groups-panel" size="small">
        <div className="panel-header mesh-groups-header">
          <div>
            <span className="section-kicker">{t.pageKicker}</span>
            <h2>{t.groupsTitle}</h2>
            <p>{t.groupsHint}</p>
          </div>
          <div className="action-row mesh-group-header-actions">
            <Button
              icon={<Plus size={17} />}
              onClick={() => { setActionError(""); setGroupName(""); setDialog("create"); }}
              disabled={props.busy || !props.onCreateGroup}
              title={props.onCreateGroup ? t.createGroupHint : t.waitingForGroupCommands}
            >
              {t.createGroup}
            </Button>
            <Button
              type="primary"
              icon={<Link2 size={17} />}
              onClick={() => { setActionError(""); setGroupName(""); setShareCode(""); setDialog("join"); }}
              disabled={props.busy}
              title={t.joinGroupHint}
            >
              {t.joinGroup}
            </Button>
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
                <Button
                  htmlType="button"
                  className="mesh-group-card-select"
                  onClick={() => selectGroup(group)}
                  disabled={props.busy || (!selected && !props.onSelectGroup)}
                  title={selected ? t.activeGroup : meshText(t.selectGroupHint, { name: group.name })}
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
                </Button>
                <div className="mesh-group-card-actions">
                  {selected && <Button className={`icon-button ${groupRunning ? "" : "primary"}`} onClick={toggleGroupRuntime} disabled={props.busy || runtimePending || !groupActionAvailable} title={groupRunning ? t.stopGroupHint : t.startGroupHint}>
                    {groupRunning ? <Square size={16} /> : <Play size={16} />}
                    {runtimePending ? t.processing : groupRunning ? t.stopGroup : t.startGroup}
                  </Button>}
                  {group.id !== LEGACY_GROUP_ID && props.onDeleteGroup && <Button
                    danger
                    icon={<Trash2 size={15} />}
                    disabled={props.busy || group.runtimeStatus === "starting" || group.runtimeStatus === "stopping"}
                    title={t.deleteGroupHint}
                    onClick={() => Modal.confirm({
                      title: t.deleteGroup,
                      content: meshText(t.deleteGroupConfirm, { name: group.name }),
                      okText: t.deleteGroup,
                      okButtonProps: { danger: true },
                      cancelText: t.cancel,
                      onOk: () => runAction(() => props.onDeleteGroup!({ groupId: group.id })),
                    })}
                  >{t.deleteGroup}</Button>}
                </div>
              </div>
            );
          })}
        </div>

      </Card>

      {(actionError || activeGroup?.lastError || props.status?.lastError) && (
        <div className="mesh-warning">{actionError || activeGroup?.lastError || props.status?.lastError}</div>
      )}

      {activeGroup && (
        <Card className="panel mesh-share-panel mesh-files-panel">
          <div className="panel-header">
            <div><h2>{t.filesTitle}</h2><p>{t.filesHint}</p></div>
            <div className="mesh-file-header-actions">
              <Button icon={<Plus size={16} />} onClick={() => void addFileShare(false)} disabled={props.busy} title={t.addFilesHint}>{t.addFiles}</Button>
              <Button type="primary" icon={<Plus size={16} />} onClick={() => void addFileShare(true)} disabled={props.busy} title={t.addFolderHint}>{t.addFolder}</Button>
            </div>
          </div>
          {(props.groupStatus?.fileAccess?.shares ?? []).map((share) => (
            <div className="mesh-share-entry" key={share.id}>
              <div className="mesh-shared-file-row">
                <span className="mesh-shared-file-info"><span className="mesh-share-name"><span className="mesh-file-type-icon">{share.kind === "directory" ? <Folder size={17} /> : <File size={17} />}</span><strong title={share.name}>{share.name}</strong></span></span>
                <div className="mesh-shared-file-actions"><Dropdown trigger={["click", "contextMenu"]} menu={{ items: [
                  { key: "browse", label: t.browseFiles, title: meshText(t.browseShareHint, { name: share.name }), onClick: () => void browseShare(share.id) },
                  { key: "remove", label: t.removeShare, title: t.removeShareHint, danger: true, onClick: () => void runAction(async () => {
                    const status = await invoke<MeshGroupStatus>("mesh_group_remove_file_share", { groupId: activeGroup.id, shareId: share.id });
                    props.onGroupFilesChanged?.(status);
                    if (selectedShareId === share.id) {
                      setSelectedShareId("");
                      setFilePath("");
                      setLocalFiles([]);
                      setLocalDirectoryNodes({});
                    }
                  }) },
                ] }}><Button aria-label={meshText(t.remoteShareMenuHint, { name: share.name })} title={meshText(t.remoteShareMenuHint, { name: share.name })} icon={<MoreHorizontal size={16} />} /></Dropdown>
                <Button className="mini-button" title={selectedShareId === share.id ? t.collapseFileListHint : meshText(t.browseShareHint, { name: share.name })} onClick={() => void browseShare(share.id)}>{selectedShareId === share.id ? t.collapse : t.browseFiles}</Button></div>
              </div>
              {selectedShareId === share.id && <div className="mesh-shared-file-list">
                {filePath && <Breadcrumb className="mesh-file-breadcrumb" items={filePath.split("/").filter(Boolean).map((segment, index, segments) => ({ title: <Button type="link" title={meshText(t.openFolderHint, { name: segment })} onClick={() => void browseShare(share.id, segments.slice(0, index + 1).join("/"))}>{segment}</Button> }))} />}
                <Tree.DirectoryTree
                  className="mesh-local-directory-tree"
                  showLine={{ showLeafIcon: false }}
                  showIcon
                  autoExpandParent={false}
                  expandedKeys={expandedLocalKeys}
                  onExpand={(keys) => setExpandedLocalKeys(keys.map(String))}
                  treeData={localDirectoryTree}
                  loadData={(node) => loadLocalDirectory(share.id, String(node.key))}
                />
              </div>}
            </div>
          ))}
          {(props.groupStatus?.fileAccess?.shares.length ?? 0) === 0 && <div className="account-empty">{t.noFileShares}</div>}
              <div className="mesh-file-settings"><label className="checkline mesh-device-auto-sync">
            <Switch checked={props.groupStatus?.fileAccess?.aiAccessEnabled === true} onChange={(checked) => void runAction(async () => {
              const status = await invoke<MeshGroupStatus>("mesh_group_set_ai_file_access", { groupId: activeGroup.id, enabled: checked });
              props.onGroupFilesChanged?.(status);
            })} />
            <span><strong>{t.aiFileAccess}</strong><small>{t.aiFileAccessHint}</small></span>
          </label>
          {props.groupStatus?.fileAccess?.aiAccessEnabled && <div className="mesh-ai-endpoints">
            {props.groupStatus.fileAccess.aiToken && <>
              <Select
                size="large"
                aria-label="外部 AI 访问设备"
                placeholder="选择 AI 访问的设备"
                value={aiTargetDeviceId || undefined}
                onChange={setAiTargetDeviceId}
                options={(props.groupStatus.fileAccess.aiDevices ?? []).map((device) => ({
                  value: device.id,
                  label: `${device.name}${device.online ? " · 在线" : " · 离线"}`,
                }))}
                style={{ width: "100%" }}
              />
              <div className="mesh-ai-guide-header">
                <strong>{t.aiCredentials}</strong>
              <Button size="small" title={t.copyAiInstructionsHint} aria-label={t.copyAiInstructionsHint} icon={<Copy size={14} />} onClick={async () => {
                  const fileAccess = props.groupStatus?.fileAccess;
                  if (!fileAccess?.aiToken) return;
                  if (!aiTargetDeviceId) return;
                  const httpUrl = `${fileAccess.aiHttpUrl}?groupId=${encodeURIComponent(activeGroup.id)}&deviceId=${encodeURIComponent(aiTargetDeviceId)}`;
                  const mcpUrl = `${fileAccess.aiMcpUrl}&deviceId=${encodeURIComponent(aiTargetDeviceId)}`;
                  const guide = [
                    "CodexSwitcher 外部 AI 文件访问",
                    `Authorization: Bearer ${fileAccess.aiToken}`,
                    `HTTP：${httpUrl}`,
                    `MCP：${mcpUrl}`,
                    `使用说明：${t.aiAccessInstructions}`,
                  ].join("\n");
                  await navigator.clipboard.writeText(guide);
                  setAiGuideCopied(true);
                  window.setTimeout(() => setAiGuideCopied(false), 1800);
                }} disabled={!aiTargetDeviceId}>{aiGuideCopied ? "已复制" : "复制给 AI"}</Button>
              </div>
              <Input.TextArea className="mesh-payload-box" readOnly value={[
                `Authorization: Bearer ${props.groupStatus.fileAccess.aiToken}`,
                `HTTP：${props.groupStatus.fileAccess.aiHttpUrl}?groupId=${encodeURIComponent(activeGroup.id)}&deviceId=${encodeURIComponent(aiTargetDeviceId || "<设备ID>")}`,
                `MCP：${props.groupStatus.fileAccess.aiMcpUrl}&deviceId=${encodeURIComponent(aiTargetDeviceId || "<设备ID>")}`,
                `使用说明：${t.aiAccessInstructions}`,
              ].join("\n")} aria-label={t.aiCredentials} autoSize={{ minRows: 5, maxRows: 10 }} />
            </>}
          </div>}</div>
        </Card>
      )}

      <div className="mesh-columns mesh-group-content">
        <div className="mesh-column">
          <Card className="panel mesh-device-panel">
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
              <div className="mesh-device-sync-heading" title={t.deviceScopeHint}>{t.allowedSyncContents}<span className="mesh-help-indicator" aria-hidden="true">ⓘ</span></div>
                      <ScopeEditor
                        compact
                        value={device.syncScope}
                        allowed={device.allowedSyncScope}
                        disabled={revoked || !device.trusted}
                        onChange={(scope) => saveDeviceSelection(device, scope)}
                      />
                      {device.allowedSyncScope.accounts && (
                        <label className="checkline mesh-device-auto-sync">
                          <Checkbox
                            checked={device.autoAccountSync === true}
                            disabled={revoked || !device.trusted}
                            onChange={(event) => saveDeviceSelection(device, device.syncScope, event.target.checked)}
                          />
                          <span>
                          <strong title={t.autoSyncAccountsHint}>{t.autoSyncAccounts}<span className="mesh-help-indicator" aria-hidden="true">ⓘ</span></strong>
                            <small>{t.autoSyncAccountsHint}</small>
                          </span>
                        </label>
                      )}
                      <label className="checkline mesh-device-auto-sync">
                        <Checkbox
                          checked={props.groupStatus?.fileAccessDeviceIds?.includes(device.id) === true}
                          disabled={revoked || !device.trusted}
                          onChange={(event) => void runAction(async () => {
                            const status = await invoke<MeshGroupStatus>("mesh_group_set_file_device_access", { groupId: activeGroup!.id, deviceId: device.id, allowed: event.target.checked });
                            props.onGroupFilesChanged?.(status);
                          })}
                        />
                        <span><strong title={t.fileAccessHint}>{t.fileAccess}<span className="mesh-help-indicator" aria-hidden="true">ⓘ</span></strong></span>
                      </label>
                    </div>
                    <div className="mesh-device-actions">
                      <Button
                        htmlType="button"
                        className="mini-button"
                        onClick={() => void browseDeviceShares(device)}
                        disabled={props.busy || !device.online || revoked || !device.trusted}
                        title={!device.online ? t.deviceStatusOffline : meshText(t.browseDeviceHint, { name: device.name })}
                      >
                        <Download size={14} /> {t.browseFiles}
                      </Button>
                      <Button
                        htmlType="button"
                        className="mini-button"
                        onClick={() => syncDevice(device.id)}
                        disabled={props.busy || syncingDeviceId !== null || !device.online || revoked || !device.trusted || !syncAvailable}
                        title={!device.online ? t.deviceStatusOffline : meshText(t.syncDeviceHint, { name: device.name })}
                      >
                        {syncingDeviceId === device.id ? <LoaderCircle className="button-spinner" size={14} /> : <RefreshCcw size={14} />} {syncingDeviceId === device.id ? t.processing : t.syncDevice}
                      </Button>
                      <Button
                        htmlType="button"
                        className={`mini-button ${revoked ? "" : "danger"}`}
                        onClick={() => toggleDeviceRevocation(device)}
                        disabled={props.busy || !revokeAvailable}
                        title={revoked ? meshText(t.restoreDeviceHint, { name: device.name }) : meshText(t.revokeDeviceHint, { name: device.name })}
                      >
                        {revoked ? <CheckCircle2 size={14} /> : <ShieldOff size={14} />}
                        {revoked ? t.restoreDevice : t.revokeDevice}
                      </Button>
                      {removeAvailable && (
                        <>
                          {removing && (
                            <Button
                              htmlType="button"
                              className="mini-button"
                              onClick={() => setRemoveConfirmDeviceId(null)}
                              disabled={props.busy}
                              title={t.cancelDialogHint}
                            >
                              <X size={14} /> {t.cancel}
                            </Button>
                          )}
                          <Button
                            htmlType="button"
                            className="mini-button danger"
                            onClick={() => removeDevice(device)}
                            disabled={props.busy}
                            title={removing ? meshText(t.removeDeviceConfirmHint, { name: device.name }) : meshText(t.removeDeviceHint, { name: device.name })}
                          >
                            <Trash2 size={14} /> {removing ? t.confirmRemoveDevice : t.removeDevice}
                          </Button>
                        </>
                      )}
                    </div>
                  </article>
                );
              })}
              {groupDevices.length === 0 && <div className="account-empty">{t.noGroupDevices}</div>}
            </div>
            <Button
              className="icon-button"
              onClick={() => syncDevice()}
              disabled={props.busy || syncingDeviceId !== null || !syncAvailable || !groupDevices.some((device) => device.online && device.trusted && !deviceIsRevoked(device))}
              title={t.syncAllDevicesHint}
            >
              {syncingDeviceId === "__all__" ? <LoaderCircle className="button-spinner" size={17} /> : <RefreshCcw size={17} />} {syncingDeviceId === "__all__" ? t.processing : t.syncAllDevices}
            </Button>
          </Card>
          {remoteDevice && activeGroup && <RemoteFilesPanel device={remoteDevice} groupId={activeGroup.id} shares={remoteShares} shareId={remoteShareId} path={remoteFilePath} files={remoteFiles} directoryNodes={remoteDirectoryNodes} expandedKeys={expandedRemoteKeys} progress={downloadProgress} onBrowse={(shareId, path) => void browseRemoteFiles(shareId, path)} onLoadDirectory={loadRemoteDirectory} onDirectoryLoaded={(key, children) => setRemoteDirectoryNodes((current) => ({ ...current, [key]: children }))} onExpandedKeysChange={setExpandedRemoteKeys} onDownload={(file) => void downloadRemoteFile(file)} />}
        </div>

        <div className="mesh-column">
          <Card className="panel mesh-share-panel mesh-code-panel">
            <div className="panel-header">
              <div>
                <h2>{t.sharingTitle}</h2>
                <p>{t.sharingHint}</p>
              </div>
              <KeyRound size={22} />
            </div>
            <div className="mesh-share-scope-hint" title={t.groupScopeHint}>
              <strong>{t.shareContents}</strong>
              <span>{t.defaultScopeHint}</span><span className="mesh-help-indicator" aria-hidden="true">ⓘ</span>
            </div>
            <ScopeEditor value={props.syncScope} onChange={changeShareScope} title={t.groupScopeHint} />
            <div className="action-row">
              <Button
                className="icon-button primary"
                onClick={() => {
                  if (activeGroup && canUseGroupCommands && props.onCreateGroupShare) {
                    void runAction(() => props.onCreateGroupShare!({ groupId: activeGroup.id, mode: "joinOnly" }));
                  } else if (activeIsLegacy) {
                    void runAction(props.onCreateShare);
                  }
                }}
                disabled={props.busy || (canUseGroupCommands ? !props.onCreateGroupShare : false)}
                title={t.createShareCodeHint}
              >
                <KeyRound size={17} /> {t.createShareCode}
              </Button>
              <Button className="icon-button" onClick={() => void props.onCopyShare()} disabled={props.busy || !props.sharePayload.trim()} title={t.copyShareCodeHint}>
                <Copy size={17} /> {t.copyOriginalShareCode}
              </Button>
            </div>
            <Input.TextArea
              className="mesh-payload-box"
              autoSize={{ minRows: props.sharePayload ? 3 : 1, maxRows: 6 }}
              value={props.sharePayload}
              readOnly
              placeholder={t.shareCodePlaceholder}
              aria-label={t.shareCodeLabel}
              spellCheck={false}
            />
          </Card>

          <Card className="panel mesh-migration-panel">
            <div className="panel-header">
              <div>
                <h2>{t.migrationTitle}</h2>
                <p>{t.migrationHint}</p>
              </div>
              <Download size={22} />
            </div>
            <div className="mesh-migration-options">
              <Input.Password
                value={props.migrationPassword}
                onChange={(event) => props.onMigrationPasswordChange(event.target.value)}
                placeholder={props.migrationUseMeshSecret ? t.groupCredentialPlaceholder : t.passwordPlaceholder}
                disabled={props.migrationUseMeshSecret}
              />
              <label className="checkline">
                <Checkbox
                  checked={props.migrationUseMeshSecret}
                  onChange={(event) => props.onMigrationUseMeshSecretChange(event.target.checked)}
                />
                <span title={t.useGroupCredentialHint}>{t.useGroupCredential}<span className="mesh-help-indicator" aria-hidden="true">ⓘ</span></span>
              </label>
              <label className="checkline">
                <Checkbox
                  checked={props.includeConversations}
                  onChange={(event) => props.onIncludeConversationsChange(event.target.checked)}
                />
                <span title={t.includeConversationsHint}>{t.includeConversations}<span className="mesh-help-indicator" aria-hidden="true">ⓘ</span></span>
              </label>
              <label className="checkline">
                <Checkbox
                  checked={props.restoreConversations}
                  onChange={(event) => props.onRestoreConversationsChange(event.target.checked)}
                />
                <span title={t.restoreConversationsHint}>{t.restoreConversations}<span className="mesh-help-indicator" aria-hidden="true">ⓘ</span></span>
              </label>
            </div>
            <Collapse className="export-selector mesh-export-selector" items={[{ key: "accounts", label: <><span>{t.migrationAccounts}</span><strong>{selectedProfiles.length}/{props.profiles.length}</strong></>, children: <>
              <div className="export-selector-actions">
                <Button size="small" onClick={props.onSelectAllProfiles} title={t.selectAllProfilesHint}>{t.selectAll}</Button>
                <Button size="small" onClick={props.onClearProfiles} title={t.clearProfilesHint}>{t.clear}</Button>
              </div>
              <div className="export-account-list">
                {props.profiles.map((profile) => (
                  <label className="export-account-row" key={profile.id}>
                    <Checkbox
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
            </> }]} />
            <div className="action-row">
              <Button className="icon-button primary" onClick={() => void props.onExportMigration()} disabled={props.busy || selectedProfiles.length === 0} title={t.exportMigrationHint}>
                <Download size={17} /> {t.exportMigration}
              </Button>
              <Button className="icon-button" onClick={() => void props.onImportMigration()} disabled={props.busy} title={t.importMigrationHint}>
                <Upload size={17} /> {t.importMigration}
              </Button>
            </div>
          </Card>
        </div>
      </div>

      {dialog && (
        <Modal
          open
          onCancel={() => setDialog(null)}
          title={dialog === "create" ? t.createGroupTitle : t.joinGroupTitle}
          width={560}
          footer={null}
        >
            <p className="mesh-add-device-hint">{dialog === "create" ? t.createGroupHint : t.joinGroupHint}</p>
            {dialog === "create" && (
              <label className="mesh-group-dialog-field">
                <span>{t.groupNameLabel}</span>
                <Input value={groupName} onChange={(event) => setGroupName(event.target.value)} placeholder={t.groupNamePlaceholder} autoFocus />
              </label>
            )}
            {dialog === "join" && (
              <label className="mesh-group-dialog-field">
                <span>{t.groupShareCodeLabel}</span>
                <Input.TextArea className="mesh-payload-box" value={shareCode} onChange={(event) => setShareCode(event.target.value)} placeholder={t.groupShareCodePlaceholder} spellCheck={false} autoFocus rows={5} />
              </label>
            )}
            {actionError && <div className="mesh-warning">{actionError}</div>}
            <div className="update-dialog-actions">
              <Button className="icon-button" onClick={() => setDialog(null)} title={t.cancelDialogHint}>{t.cancel}</Button>
              <Button
                className="icon-button primary"
                onClick={dialog === "create" ? submitCreateGroup : submitJoinGroup}
                disabled={props.busy || (dialog === "create" ? !groupName.trim() : !shareCode.trim())}
                title={dialog === "create" ? t.createGroupHint : t.joinGroupHint}
              >
                {dialog === "create" ? <Plus size={17} /> : <Link2 size={17} />}
                {dialog === "create" ? t.create : t.join}
              </Button>
            </div>
        </Modal>
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
  title,
}: {
  value: MeshSyncScope;
  allowed?: MeshSyncScope;
  onChange: (value: MeshSyncScope) => void;
  compact?: boolean;
  disabled?: boolean;
  title?: string;
}) {
  const t = getMeshI18n();
  const options: Array<{ key: keyof MeshSyncScope; label: string }> = [
    { key: "accounts", label: t.scopeAccounts },
    { key: "rules", label: t.scopeRules },
    { key: "routing", label: t.scopeRouting },
    { key: "conversations", label: t.scopeConversations },
  ];
  return (
    <div className={`mesh-scope-editor ${compact ? "compact" : ""}`} title={title}>
      {options.map((option) => (
        <label className="checkline" key={option.key}>
          <Checkbox
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
