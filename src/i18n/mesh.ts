import { resolveLanguage } from "../i18n";
import type { LanguageSetting } from "../types";

export type MeshI18n = {
  pageKicker: string;
  groupsTitle: string;
  groupsHint: string;
  createGroup: string;
  joinGroup: string;
  selectGroupHint: string;
  startGroupHint: string;
  stopGroupHint: string;
  deleteGroupHint: string;
  activeGroup: string;
  running: string;
  stopped: string;
  runtimeError: string;
  onlineDevices: string;
  startGroup: string;
  stopGroup: string;
  deleteGroup: string;
  deleteGroupConfirm: string;
  processing: string;
  waitingForGroupCommands: string;
  actionFailed: string;
  defaultGroup: string;
  deviceListTitle: string;
  currentGroupSummary: string;
  noGroups: string;
  noGroupDevices: string;
  deviceStatusOnline: string;
  deviceStatusOffline: string;
  deviceStatusAuthorized: string;
  deviceStatusPending: string;
  deviceStatusRevoked: string;
  revocationPersisted: string;
  syncDevice: string;
  syncAllDevices: string;
  browseDeviceHint: string;
  syncDeviceHint: string;
  restoreDeviceHint: string;
  revokeDeviceHint: string;
  removeDeviceHint: string;
  removeDeviceConfirmHint: string;
  syncAllDevicesHint: string;
  allowedSyncContents: string;
  groupScopeHint: string;
  deviceScopeHint: string;
  fileAccessHint: string;
  autoSyncAccounts: string;
  autoSyncAccountsHint: string;
  revokeDevice: string;
  restoreDevice: string;
  removeDevice: string;
  removeDeviceConfirm: string;
  confirmRemoveDevice: string;
  sharingTitle: string;
  sharingHint: string;
  shareContents: string;
  defaultScopeHint: string;
  createShareCode: string;
  copyOriginalShareCode: string;
  copy: string;
  shareCodeLabel: string;
  shareCodePlaceholder: string;
  scopeAccounts: string;
  scopeRules: string;
  scopeRouting: string;
  scopeConversations: string;
  migrationTitle: string;
  migrationHint: string;
  groupCredentialPlaceholder: string;
  passwordPlaceholder: string;
  useGroupCredential: string;
  useGroupCredentialHint: string;
  includeConversations: string;
  includeConversationsHint: string;
  restoreConversations: string;
  restoreConversationsHint: string;
  migrationAccounts: string;
  selectAll: string;
  clear: string;
  unknownAccount: string;
  exportMigration: string;
  importMigration: string;
  createGroupTitle: string;
  createGroupHint: string;
  joinGroupTitle: string;
  joinGroupHint: string;
  groupNameLabel: string;
  groupNamePlaceholder: string;
  groupShareCodeLabel: string;
  groupShareCodePlaceholder: string;
  close: string;
  cancel: string;
  create: string;
  join: string;
  filesTitle: string;
  filesHint: string;
  addFiles: string;
  addFolder: string;
  addFilesHint: string;
  addFolderHint: string;
  browseShareHint: string;
  collapseFileListHint: string;
  collapse: string;
  openFolderHint: string;
  copyAiInstructionsHint: string;
  browseRemoteShareHint: string;
  remoteShareMenuHint: string;
  navigateFolderHint: string;
  removeShareHint: string;
  createShareCodeHint: string;
  copyShareCodeHint: string;
  selectAllProfilesHint: string;
  clearProfilesHint: string;
  exportMigrationHint: string;
  importMigrationHint: string;
  cancelDialogHint: string;
  removeShare: string;
  browseFiles: string;
  downloadFile: string;
  fileAccess: string;
  aiFileAccess: string;
  aiFileAccessHint: string;
  aiCredentials: string;
  aiCredentialWarning: string;
  aiAccessInstructions: string;
  fileAccessDenied: string;
  noFileShares: string;
  fileListFailed: string;
  fileSize: string;
};

const zhCN: MeshI18n = {
  pageKicker: "多设备共享",
  groupsTitle: "分享组",
  groupsHint: "选择一个分享组，查看运行状态和组内设备。",
  createGroup: "创建组",
  joinGroup: "粘贴分享码加入",
  selectGroupHint: "切换到“{name}”并查看其设备和分享设置。",
  startGroupHint: "连接此分享组网络，并允许组内设备通信。",
  stopGroupHint: "断开此分享组网络；保存的组和共享文件设置会保留。",
  deleteGroupHint: "停止并删除本机保存的此分组配置，不会删除磁盘上的共享文件。",
  activeGroup: "当前组",
  running: "运行中",
  stopped: "已停止",
  runtimeError: "运行异常",
  onlineDevices: "{count} 台在线",
  startGroup: "启动组",
  stopGroup: "停止组",
  deleteGroup: "删除组",
  deleteGroupConfirm: "确定删除“{name}”？这会断开该组并移除本机保存的组配置，不会删除共享目录或文件。",
  processing: "处理中",
  waitingForGroupCommands: "等待 App.tsx 接入分享组命令",
  actionFailed: "操作失败，请稍后重试。",
  defaultGroup: "当前分享组",
  deviceListTitle: "当前组设备",
  currentGroupSummary: "仅显示并管理“{name}”中的设备",
  noGroups: "尚未选择分享组",
  noGroupDevices: "当前组暂无设备。",
  deviceStatusOnline: "在线",
  deviceStatusOffline: "离线",
  deviceStatusAuthorized: "已授权同步",
  deviceStatusPending: "等待授权",
  deviceStatusRevoked: "已撤销",
  revocationPersisted: "同步授权已撤销，设备记录保留",
  syncDevice: "同步此设备",
  syncAllDevices: "同步当前组在线设备",
  browseDeviceHint: "浏览“{name}”设备授权共享的文件并下载。",
  syncDeviceHint: "从“{name}”拉取此设备和对端都允许的同步内容。",
  restoreDeviceHint: "恢复“{name}”设备的同步和文件访问授权。",
  revokeDeviceHint: "撤销“{name}”设备授权，并关闭其文件访问权限。",
  removeDeviceHint: "从当前组移除此设备记录并清除其文件访问授权。",
  removeDeviceConfirmHint: "再次点击确认移除“{name}”并清除相关授权。",
  syncAllDevicesHint: "从当前组所有在线且已授权设备拉取允许的同步内容。",
  allowedSyncContents: "允许同步内容",
  groupScopeHint: "这是本机共享上限；每台设备只能使用此处允许内容与对方设备上限的交集。",
  deviceScopeHint: "这是此设备可从本机拉取的内容，并受分享码权限上限限制。",
  fileAccessHint: "单独控制此设备能否浏览和下载本机添加的共享文件，不影响账号同步。",
  autoSyncAccounts: "账号自动同步",
  autoSyncAccountsHint: "开启后，账号变更会自动同步到此设备。",
  revokeDevice: "撤销同步",
  restoreDevice: "恢复授权",
  removeDevice: "移除设备",
  removeDeviceConfirm: "再次点击确认从当前分享组移除这台设备。",
  confirmRemoveDevice: "确认移除",
  sharingTitle: "当前组分享",
  sharingHint: "生成当前组的分享码，并设置允许同步的内容。",
  shareContents: "分享内容",
  defaultScopeHint: "选择后自动保存；默认仅分享账号，规则、API 和会话均不分享。",
  createShareCode: "重新生成分享码",
  copyOriginalShareCode: "复制原分享码",
  copy: "复制",
  shareCodeLabel: "当前组分享码",
  shareCodePlaceholder: "生成后在此显示",
  scopeAccounts: "账号",
  scopeRules: "规则",
  scopeRouting: "API",
  scopeConversations: "会话",
  migrationTitle: "一次性迁移包",
  migrationHint: "保留迁移包导入导出；默认选择账号，其他内容按需勾选。",
  groupCredentialPlaceholder: "使用当前组凭据保护迁移包",
  passwordPlaceholder: "迁移包密码，可留空",
  useGroupCredential: "使用当前组凭据保护",
  useGroupCredentialHint: "使用此分享组的网络凭据加密迁移包；加入组的设备可用对应凭据解密。",
  includeConversations: "导出会话记录",
  includeConversationsHint: "勾选后将会话记录写入迁移包；文件可能较大且包含私人内容。",
  restoreConversations: "导入时恢复会话",
  restoreConversationsHint: "仅对当前导入操作生效；勾选后从迁移包恢复会话记录。",
  migrationAccounts: "迁移账号",
  selectAll: "全选",
  clear: "清空",
  unknownAccount: "未知账号",
  exportMigration: "导出迁移包",
  importMigration: "导入迁移包",
  createGroupTitle: "创建分享组",
  createGroupHint: "创建后由后端返回并选中新组；不会在本地伪造成功状态。",
  joinGroupTitle: "加入分享组",
  joinGroupHint: "粘贴分享码加入，结果以服务端返回为准。",
  groupNameLabel: "组名",
  groupNamePlaceholder: "例如：家庭设备",
  groupShareCodeLabel: "分享码",
  groupShareCodePlaceholder: "粘贴分享码",
  close: "关闭",
  cancel: "取消",
  create: "创建",
  join: "加入",
  filesTitle: "共享文件",
  filesHint: "添加单个文件或目录；其他设备手动浏览并下载。",
  addFiles: "添加文件",
  addFolder: "添加目录",
  addFilesHint: "选择一个或多个本机文件，加入当前组的共享列表。",
  addFolderHint: "选择本机目录；组内获准设备可浏览并下载其中内容。",
  browseShareHint: "在此卡片下展开“{name}”的文件和目录。",
  collapseFileListHint: "收起此共享项的文件列表。",
  collapse: "收起",
  openFolderHint: "打开“{name}”目录。",
  copyAiInstructionsHint: "复制访问令牌、目标设备地址和 AI 使用说明。",
  browseRemoteShareHint: "打开“{name}”并列出其中可浏览的文件。",
  remoteShareMenuHint: "打开“{name}”的文件操作菜单。",
  navigateFolderHint: "返回共享目录“{name}”。",
  removeShareHint: "停止共享此路径；不会删除原文件或目录。",
  createShareCodeHint: "生成当前组加入码；新设备获得的权限由分享内容选项决定。",
  copyShareCodeHint: "复制当前已生成的分享码。",
  selectAllProfilesHint: "选中迁移包中的全部账号。",
  clearProfilesHint: "取消选择所有要导出的账号。",
  exportMigrationHint: "将选中的账号和勾选内容打包到本机文件。",
  importMigrationHint: "选择已有迁移包并导入账号及其中包含的内容。",
  cancelDialogHint: "关闭窗口并放弃未提交的输入。",
  removeShare: "移除分享",
  browseFiles: "浏览",
  downloadFile: "下载",
  fileAccess: "允许文件访问",
  aiFileAccess: "外部 AI 文件访问",
  aiFileAccessHint: "启用后生成独立令牌，可用于 MCP 和 HTTP，只能读取共享文件；需先启动当前组网才能连接。",
  aiCredentials: "AI 接口",
  aiCredentialWarning: "令牌只在启用时显示，请复制并妥善保管。",
  aiAccessInstructions: "请求携带 Authorization: Bearer 令牌。HTTP GET 路径：/mesh/files/shares?groupId=组ID、/mesh/files?groupId=组ID&shareId=共享项ID&path=相对路径、/mesh/files/download?groupId=组ID&shareId=共享项ID&path=相对路径；MCP 可浏览、搜索并读取文本文件。",
  fileAccessDenied: "未授权文件访问",
  noFileShares: "当前组还没有共享文件。",
  fileListFailed: "读取共享文件失败",
  fileSize: "字节",
};

const en: MeshI18n = {
  pageKicker: "Multi-device sharing",
  groupsTitle: "Share groups",
  groupsHint: "Choose a share group to see its runtime and devices.",
  createGroup: "Create group",
  joinGroup: "Join with code",
  selectGroupHint: "Switch to “{name}” to view its devices and sharing settings.",
  startGroupHint: "Connect this share group network so its devices can communicate.",
  stopGroupHint: "Disconnect this group. Its saved configuration and file shares remain.",
  deleteGroupHint: "Stop and remove this group configuration from this device. Shared files on disk remain.",
  activeGroup: "Current",
  running: "Running",
  stopped: "Stopped",
  runtimeError: "Runtime error",
  onlineDevices: "{count} online",
  startGroup: "Start group",
  stopGroup: "Stop group",
  deleteGroup: "Delete group",
  deleteGroupConfirm: "Delete “{name}”? This disconnects the group and removes its saved configuration from this device. Shared files and folders on disk are not deleted.",
  processing: "Working",
  waitingForGroupCommands: "Waiting for App.tsx to connect group commands",
  actionFailed: "The action failed. Try again.",
  defaultGroup: "Current share group",
  deviceListTitle: "Current group devices",
  currentGroupSummary: "Only devices in “{name}” are shown and managed",
  noGroups: "No share group selected",
  noGroupDevices: "No devices in this group yet.",
  deviceStatusOnline: "Online",
  deviceStatusOffline: "Offline",
  deviceStatusAuthorized: "Sync authorized",
  deviceStatusPending: "Authorization needed",
  deviceStatusRevoked: "Revoked",
  revocationPersisted: "Sync access revoked; device record retained",
  syncDevice: "Sync device",
  syncAllDevices: "Sync online devices in this group",
  browseDeviceHint: "Browse and download files shared by “{name}”.",
  syncDeviceHint: "Pull content allowed by both this device and “{name}”.",
  restoreDeviceHint: "Restore sync and file access for “{name}”.",
  revokeDeviceHint: "Revoke “{name}” and disable its file access.",
  removeDeviceHint: "Remove this device record and clear its file access grant.",
  removeDeviceConfirmHint: "Click again to remove “{name}” and clear its grants.",
  syncAllDevicesHint: "Pull allowed content from every authorized device currently online.",
  allowedSyncContents: "Allowed sync content",
  groupScopeHint: "This sets the content limit shared by this device. Each peer can use only the intersection with the limit advertised by the other device.",
  deviceScopeHint: "Controls what this device may pull from this device, bounded by the share-code permissions.",
  fileAccessHint: "Separately allows this device to browse and download files shared from this device. It does not affect account sync.",
  autoSyncAccounts: "Auto-sync accounts",
  autoSyncAccountsHint: "When enabled, account changes are sent to this device automatically.",
  revokeDevice: "Revoke sync",
  restoreDevice: "Restore access",
  removeDevice: "Remove device",
  removeDeviceConfirm: "Click again to remove this device from the current share group.",
  confirmRemoveDevice: "Confirm removal",
  sharingTitle: "Share current group",
  sharingHint: "Create a code for this group and choose what it may sync.",
  shareContents: "Shared content",
  defaultScopeHint: "Selections save automatically; accounts are shared by default, while rules, API settings, and conversations stay off.",
  createShareCode: "Regenerate share code",
  copyOriginalShareCode: "Copy existing share code",
  copy: "Copy",
  shareCodeLabel: "Current group share code",
  shareCodePlaceholder: "The generated code appears here",
  scopeAccounts: "Accounts",
  scopeRules: "Rules",
  scopeRouting: "API",
  scopeConversations: "Conversations",
  migrationTitle: "One-time migration bundle",
  migrationHint: "Migration import and export remain available; accounts are the default selection.",
  groupCredentialPlaceholder: "Protected with the current group credential",
  passwordPlaceholder: "Bundle password, optional",
  useGroupCredential: "Protect with current group credential",
  useGroupCredentialHint: "Encrypts the migration bundle with this group’s network credential. Devices with the matching credential can decrypt it.",
  includeConversations: "Export conversations",
  includeConversationsHint: "Includes conversation history in the bundle. It may be large and contains private data.",
  restoreConversations: "Restore conversations on import",
  restoreConversationsHint: "Applies only to this import. When selected, conversation history is restored from the bundle.",
  migrationAccounts: "Accounts to migrate",
  selectAll: "Select all",
  clear: "Clear",
  unknownAccount: "Unknown account",
  exportMigration: "Export bundle",
  importMigration: "Import bundle",
  createGroupTitle: "Create share group",
  createGroupHint: "The backend returns and selects the new group; the UI does not fake success locally.",
  joinGroupTitle: "Join share group",
  joinGroupHint: "Paste a share code. The result comes from the backend.",
  groupNameLabel: "Group name",
  groupNamePlaceholder: "For example: Home devices",
  groupShareCodeLabel: "Share code",
  groupShareCodePlaceholder: "Paste share code",
  close: "Close",
  cancel: "Cancel",
  create: "Create",
  join: "Join",
  filesTitle: "Shared files",
  filesHint: "Add files or folders. Other devices can browse and download manually.",
  addFiles: "Add files",
  addFolder: "Add folder",
  addFilesHint: "Choose one or more local files to share with this group.",
  addFolderHint: "Choose a local folder for authorized group devices to browse and download.",
  browseShareHint: "Expand the files and folders in “{name}” below this item.",
  collapseFileListHint: "Collapse this shared item's file list.",
  collapse: "Collapse",
  openFolderHint: "Open the “{name}” folder.",
  copyAiInstructionsHint: "Copy the access token, target device address, and AI usage instructions.",
  browseRemoteShareHint: "Open “{name}” and list the files available to browse.",
  remoteShareMenuHint: "Open file actions for “{name}”.",
  navigateFolderHint: "Return to the shared folder “{name}”.",
  removeShareHint: "Stop sharing this path. The original file or folder will not be deleted.",
  createShareCodeHint: "Create a join code. New device permissions follow the Shared content options.",
  copyShareCodeHint: "Copy the currently generated share code.",
  selectAllProfilesHint: "Select every account for the migration bundle.",
  clearProfilesHint: "Deselect all accounts for export.",
  exportMigrationHint: "Package the selected accounts and enabled content into a local file.",
  importMigrationHint: "Choose a migration bundle and import its accounts and included content.",
  cancelDialogHint: "Close the dialog and discard unsubmitted input.",
  removeShare: "Remove share",
  browseFiles: "Browse",
  downloadFile: "Download",
  fileAccess: "Allow file access",
  aiFileAccess: "External AI file access",
  aiFileAccessHint: "Creates a separate read-only token for MCP and HTTP. Start this share group before connecting.",
  aiCredentials: "AI endpoints",
  aiCredentialWarning: "The token is shown only when enabled. Copy and store it securely.",
  aiAccessInstructions: "Send Authorization: Bearer <token>. HTTP GET paths: /mesh/files/shares?groupId=<id>, /mesh/files?groupId=<id>&shareId=<id>&path=<relative-path>, and /mesh/files/download?groupId=<id>&shareId=<id>&path=<relative-path>. MCP can browse, search, and read text files.",
  fileAccessDenied: "File access is not authorized",
  noFileShares: "No files have been shared in this group.",
  fileListFailed: "Could not load shared files",
  fileSize: "bytes",
};

const zhTW: MeshI18n = {
  ...zhCN,
  addFilesHint: "選擇一個或多個本機檔案，加入目前組的共享清單。",
  addFolderHint: "選擇本機目錄，讓組內獲准裝置瀏覽和下載。",
  browseShareHint: "在此項目下展開「{name}」的檔案和目錄。",
  collapseFileListHint: "收起此共享項目的檔案清單。",
  collapse: "收起",
  openFolderHint: "開啟「{name}」目錄。",
  copyAiInstructionsHint: "複製存取令牌、目標裝置地址和 AI 使用說明。",
  browseRemoteShareHint: "開啟「{name}」並列出可瀏覽的檔案。",
  remoteShareMenuHint: "開啟「{name}」的檔案操作選單。",
  navigateFolderHint: "返回共享目錄「{name}」。",
  removeShareHint: "停止共享此路徑；不會刪除原始檔案或目錄。",
  createShareCodeHint: "產生目前組加入碼；新裝置權限由分享內容選項決定。",
  copyShareCodeHint: "複製目前已產生的分享碼。",
  selectAllProfilesHint: "選取遷移包中的所有帳號。",
  clearProfilesHint: "取消選取所有要匯出的帳號。",
  exportMigrationHint: "將所選帳號和勾選內容打包成此裝置上的檔案。",
  importMigrationHint: "選取遷移包並匯入其中包含的帳號和內容。",
  cancelDialogHint: "關閉視窗並放棄尚未提交的輸入。",
  pageKicker: "多裝置共享",
  groupsTitle: "分享組",
  groupsHint: "選擇一個分享組，查看執行狀態和組內裝置。",
  createGroup: "建立組",
  joinGroup: "貼上分享碼加入",
  selectGroupHint: "切換至「{name}」並查看其裝置和分享設定。",
  startGroupHint: "連線此分享組網路，讓組內裝置互相通訊。",
  stopGroupHint: "中斷此分享組網路；已儲存的組態和共享檔案設定會保留。",
  deleteGroupHint: "停止並移除此裝置儲存的分組設定，不會刪除磁碟上的共享檔案。",
  activeGroup: "目前組",
  running: "執行中",
  stopped: "已停止",
  onlineDevices: "{count} 台在線",
  startGroup: "啟動組",
  stopGroup: "停止組",
  deviceListTitle: "目前組裝置",
  currentGroupSummary: "僅顯示並管理「{name}」中的裝置",
  noGroupDevices: "目前組暫無裝置。",
  syncDevice: "同步此裝置",
  syncAllDevices: "同步目前組在線裝置",
  browseDeviceHint: "瀏覽「{name}」裝置授權共享的檔案並下載。",
  syncDeviceHint: "從「{name}」取得兩端都允許同步的內容。",
  restoreDeviceHint: "恢復「{name}」裝置的同步和檔案存取授權。",
  revokeDeviceHint: "撤銷「{name}」裝置授權並關閉其檔案存取權限。",
  removeDeviceHint: "從目前組移除此裝置記錄並清除檔案存取授權。",
  removeDeviceConfirmHint: "再次點擊以移除「{name}」並清除相關授權。",
  syncAllDevicesHint: "從目前組所有在線且已授權的裝置取得允許同步的內容。",
  sharingTitle: "目前組分享",
  createGroupTitle: "建立分享組",
  joinGroupTitle: "加入分享組",
  groupNameLabel: "組名",
  groupNamePlaceholder: "例如：家庭裝置",
  cancel: "取消",
  create: "建立",
  join: "加入",
};

const messages: Record<"zh-CN" | "en" | "zh-TW", MeshI18n> = {
  "zh-CN": zhCN,
  en,
  "zh-TW": zhTW,
};

function readLanguageSetting(): LanguageSetting {
  const saved = localStorage.getItem("codex-account-switcher-language");
  return saved === "zh-CN" || saved === "en" || saved === "zh-TW" || saved === "system" ? saved : "system";
}

export function getMeshI18n(): MeshI18n {
  return messages[resolveLanguage(readLanguageSetting())];
}

export function meshText(template: string, values: Record<string, string>): string {
  return Object.entries(values).reduce(
    (text, [key, value]) => text.replace(`{${key}}`, value),
    template,
  );
}
