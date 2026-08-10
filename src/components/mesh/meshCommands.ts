import type { MeshDevice, MeshShareMode, MeshSyncScope } from "../../types";

/**
 * Mesh command names are kept in one place so the UI can adopt group-aware
 * commands later without scattering string literals through components.
 */
export const meshCommands = {
  status: "mesh_status",
  createSharePayload: "mesh_create_share_payload",
  importSharePayload: "mesh_import_share_payload",
  saveDeviceSync: "mesh_save_device_sync",
  syncNow: "mesh_sync_now",
  authorizePeerSync: "mesh_authorize_peer_sync",
  groupList: "mesh_group_list",
  groupStatus: "mesh_group_status",
  groupCreate: "mesh_group_create",
  groupImport: "mesh_group_import",
  groupStart: "mesh_group_start",
  groupStop: "mesh_group_stop",
  groupRevoke: "mesh_group_revoke",
  groupSaveDeviceSync: "mesh_group_save_device_sync",
  groupSyncNow: "mesh_group_sync_now",
  groupCreateSharePayload: "mesh_group_create_share_payload",
} as const;

export type MeshGroupStatusInput = { groupId: string };
export type MeshGroupSelectInput = { groupId: string };
export type MeshGroupStartInput = { groupId: string };
export type MeshGroupStopInput = { groupId: string };

export type MeshGroupCreateInput = {
  name: string;
  syncScope: MeshSyncScope;
};

export type MeshGroupImportInput = {
  shareCode: string;
};

export type MeshGroupRevokeDeviceInput = {
  groupId: string;
  deviceId: string;
  revoked: boolean;
};

export type MeshGroupSyncInput = {
  groupId: string;
  deviceId?: string;
};

export type MeshGroupDeviceSyncInput = {
  groupId: string;
  deviceId: string;
  autoAccountSync: boolean;
  syncScope: MeshSyncScope;
};

export type MeshGroupShareInput = {
  groupId: string;
  mode: MeshShareMode;
};

export type MeshDeviceSyncInput = {
  deviceId: string;
  trusted: boolean;
  autoAccountSync: boolean;
  syncScope: MeshSyncScope;
};

export function buildMeshGroupRevokeDeviceInput(
  groupId: string,
  device: MeshDevice,
  revoked: boolean,
): MeshGroupRevokeDeviceInput {
  return { groupId, deviceId: device.id, revoked };
}

/**
 * The current backend has no delete/revoke command. Saving a device as
 * untrusted and disabling automatic sync is its supported equivalent.
 */
export function buildMeshRevokeDeviceInput(device: MeshDevice): MeshDeviceSyncInput {
  return {
    deviceId: device.id,
    trusted: false,
    autoAccountSync: false,
    syncScope: {
      accounts: false,
      rules: false,
      routing: false,
      conversations: false,
    },
  };
}

export function buildMeshRestoreDeviceInput(device: MeshDevice): MeshDeviceSyncInput {
  return {
    deviceId: device.id,
    trusted: true,
    autoAccountSync: device.autoAccountSync === true,
    syncScope: device.syncScope,
  };
}
