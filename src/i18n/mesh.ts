import { resolveLanguage } from "../i18n";
import type { LanguageSetting } from "../types";

export type MeshI18n = {
  pageKicker: string;
  groupsTitle: string;
  groupsHint: string;
  createGroup: string;
  joinGroup: string;
  activeGroup: string;
  running: string;
  stopped: string;
  runtimeError: string;
  onlineDevices: string;
  startGroup: string;
  stopGroup: string;
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
  autoSyncAccounts: string;
  revokeDevice: string;
  restoreDevice: string;
  removeDevice: string;
  removeDeviceConfirm: string;
  sharingTitle: string;
  sharingHint: string;
  shareContents: string;
  defaultScopeHint: string;
  saveShareContents: string;
  createShareCode: string;
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
  includeConversations: string;
  restoreConversations: string;
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
};

const zhCN: MeshI18n = {
  pageKicker: "多设备共享",
  groupsTitle: "分享组",
  groupsHint: "选择一个分享组，查看运行状态和组内设备。",
  createGroup: "创建组",
  joinGroup: "粘贴分享码加入",
  activeGroup: "当前组",
  running: "运行中",
  stopped: "已停止",
  runtimeError: "运行异常",
  onlineDevices: "{count} 台在线",
  startGroup: "启动组",
  stopGroup: "停止组",
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
  autoSyncAccounts: "自动同步账号",
  revokeDevice: "撤销同步",
  restoreDevice: "恢复授权",
  removeDevice: "移除设备",
  removeDeviceConfirm: "确定从当前分享组移除这台设备吗？移除后它不会再显示在本组设备列表中。",
  sharingTitle: "当前组分享",
  sharingHint: "生成当前组的分享码，并设置允许同步的内容。",
  shareContents: "分享内容",
  defaultScopeHint: "默认仅分享账号；规则、API 和会话均不分享。",
  saveShareContents: "保存分享内容",
  createShareCode: "生成分享码",
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
  includeConversations: "导出会话记录",
  restoreConversations: "导入时恢复会话",
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
};

const en: MeshI18n = {
  pageKicker: "Multi-device sharing",
  groupsTitle: "Share groups",
  groupsHint: "Choose a share group to see its runtime and devices.",
  createGroup: "Create group",
  joinGroup: "Join with code",
  activeGroup: "Current",
  running: "Running",
  stopped: "Stopped",
  runtimeError: "Runtime error",
  onlineDevices: "{count} online",
  startGroup: "Start group",
  stopGroup: "Stop group",
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
  autoSyncAccounts: "Auto-sync accounts",
  revokeDevice: "Revoke sync",
  restoreDevice: "Restore access",
  removeDevice: "Remove device",
  removeDeviceConfirm: "Remove this device from the current share group? It will no longer appear in this group.",
  sharingTitle: "Share current group",
  sharingHint: "Create a code for this group and choose what it may sync.",
  shareContents: "Shared content",
  defaultScopeHint: "Accounts only by default; rules, API settings, and conversations stay off.",
  saveShareContents: "Save shared content",
  createShareCode: "Create share code",
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
  includeConversations: "Export conversations",
  restoreConversations: "Restore conversations on import",
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
};

const zhTW: MeshI18n = {
  ...zhCN,
  pageKicker: "多裝置共享",
  groupsTitle: "分享組",
  groupsHint: "選擇一個分享組，查看執行狀態和組內裝置。",
  createGroup: "建立組",
  joinGroup: "貼上分享碼加入",
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
