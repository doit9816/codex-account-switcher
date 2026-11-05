import { Breadcrumb, Button, Card, Dropdown, Empty, Progress, Space, Tree, Typography, type TreeDataNode } from "antd";
import { Download, File, Folder, MoreHorizontal } from "lucide-react";
import type { MeshDevice, MeshFileEntry, MeshFileShareView } from "../../types";
import { getMeshI18n, meshText } from "../../i18n/mesh";

type TransferProgress = Record<string, { bytes: number; total?: number }>;
type FileTreeNode = MeshFileEntry & { children?: FileTreeNode[] };

type Props = {
  device: MeshDevice;
  groupId: string;
  shares: MeshFileShareView[];
  shareId: string;
  path: string;
  files: MeshFileEntry[];
  directoryNodes: Record<string, MeshFileEntry[]>;
  expandedKeys: string[];
  progress: TransferProgress;
  onBrowse: (shareId: string, path?: string) => void;
  onLoadDirectory: (shareId: string, path: string) => Promise<MeshFileEntry[]>;
  onDirectoryLoaded: (key: string, children: MeshFileEntry[]) => void;
  onExpandedKeysChange: (keys: string[]) => void;
  onDownload: (file: MeshFileEntry) => void;
  onShareMenu?: (share: MeshFileShareView) => void;
};

function fileSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

function flattenFiles(nodes: FileTreeNode[]): FileTreeNode[] {
  return nodes.flatMap((node) => [node, ...(node.children ? flattenFiles(node.children) : [])]);
}

function makeTreeNodes(
  files: MeshFileEntry[],
  directoryNodes: Record<string, MeshFileEntry[]>,
  shareId: string,
  groupId: string,
  deviceId: string,
  progress: TransferProgress,
  t: ReturnType<typeof getMeshI18n>,
  onBrowse: Props["onBrowse"],
  onDownload: Props["onDownload"],
): TreeDataNode[] {
  return files.map((file) => ({
    key: file.id,
    title: <Dropdown trigger={["contextMenu"]} menu={{ items: file.kind === "directory"
      ? [{ key: "browse", label: t.browseFiles, onClick: () => onBrowse(shareId, file.id) }]
      : [{ key: "download", label: t.downloadFile, icon: <Download size={14} />, onClick: () => onDownload(file) }] }}>
      <Space className="mesh-file-tree-title" size={8} title={file.kind === "directory" ? meshText(t.openFolderHint, { name: file.name }) : `${t.downloadFile}: ${file.name}`}>
        <Typography.Text ellipsis={{ tooltip: file.name }}>{file.name}</Typography.Text>
        {file.kind === "file" && <Typography.Text type="secondary">{fileSize(file.size)}</Typography.Text>}
        {file.kind === "file" && progress[`${groupId}:${deviceId}:${shareId}:${file.id}`] && <Progress className="mesh-file-tree-progress" percent={Math.round(100 * progress[`${groupId}:${deviceId}:${shareId}:${file.id}`].bytes / (progress[`${groupId}:${deviceId}:${shareId}:${file.id}`].total || file.size || 1))} size="small" />}
      </Space>
    </Dropdown>,
    isLeaf: file.kind !== "directory",
    icon: file.kind === "directory" ? <Folder size={16} /> : <File size={16} />,
    children: directoryNodes[file.id] ? makeTreeNodes(directoryNodes[file.id], directoryNodes, shareId, groupId, deviceId, progress, t, onBrowse, onDownload) : undefined,
  }));
}

export function RemoteFilesPanel({ device, groupId, shares, shareId, path, files, directoryNodes, expandedKeys, progress, onBrowse, onLoadDirectory, onDirectoryLoaded, onExpandedKeysChange, onDownload, onShareMenu }: Props) {
  const t = getMeshI18n();
  const segments = path.split("/").filter(Boolean);
  const rootName = shares.find((share) => share.id === shareId)?.name ?? t.filesTitle;
  const treeData: FileTreeNode[] = files.map((file) => ({ ...file, children: directoryNodes[file.id]?.map((child) => ({ ...child })) }));
  return <Card className="mesh-remote-files-panel" title={`${device.name} · ${t.filesTitle}`} size="small">
    <div className="mesh-remote-shares">
      {shares.map((share) => <div className={`mesh-remote-share ${share.id === shareId ? "active" : ""}`} key={share.id}>
        <Button type="text" title={meshText(t.browseRemoteShareHint, { name: share.name })} icon={share.kind === "directory" ? <Folder size={16} /> : <File size={16} />} onClick={() => onBrowse(share.id)}>{share.name}</Button>
        <Dropdown trigger={["contextMenu"]} menu={{ items: [
          { key: "browse", label: t.browseFiles, onClick: () => onBrowse(share.id) },
          ...(share.kind === "file" ? [{ key: "download", label: t.downloadFile, icon: <Download size={14} />, onClick: () => onDownload({ id: "", name: share.name, kind: "file", size: 0 }) }] : []),
          ...(onShareMenu ? [{ key: "manage", label: t.fileAccess, onClick: () => onShareMenu(share) }] : []),
        ] }}>
          <Button type="text" aria-label={meshText(t.remoteShareMenuHint, { name: share.name })} title={meshText(t.remoteShareMenuHint, { name: share.name })} icon={<MoreHorizontal size={16} />} />
        </Dropdown>
      </div>)}
    </div>
    {shareId && <>
      <Breadcrumb className="mesh-file-breadcrumb" items={[
        { title: <Button type="link" title={meshText(t.navigateFolderHint, { name: rootName })} onClick={() => onBrowse(shareId)}>{rootName}</Button> },
        ...segments.map((segment, index) => ({ title: <Button type="link" title={meshText(t.navigateFolderHint, { name: segment })} onClick={() => onBrowse(shareId, segments.slice(0, index + 1).join("/"))}>{segment}</Button> })),
      ]} />
      <div className="mesh-directory-tree">
        {files.length === 0 ? <Empty description={t.fileListFailed} /> : <Tree.DirectoryTree
          blockNode
          showLine={{ showLeafIcon: false }}
          showIcon
          autoExpandParent={false}
          expandedKeys={expandedKeys}
          onExpand={(keys) => onExpandedKeysChange(keys.map(String))}
          treeData={makeTreeNodes(treeData, directoryNodes, shareId, groupId, device.id, progress, t, onBrowse, onDownload)}
          loadData={async (node) => {
            const key = String(node.key);
            const children = await onLoadDirectory(shareId, key);
            onDirectoryLoaded(key, children);
          }}
        />}
      </div>
    </>}
  </Card>;
}
