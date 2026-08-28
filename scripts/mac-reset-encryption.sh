#!/bin/bash
set -euo pipefail

APP_ID="local.codex.account-switcher"
APP_SUPPORT="$HOME/Library/Application Support"
CONFIG_DIR="$APP_SUPPORT/$APP_ID"
STAMP="$(date +%Y%m%d-%H%M%S)"
BACKUP_DIR="$APP_SUPPORT/${APP_ID}.backup-${STAMP}"

echo "CodexSwitcher Mac 加密配置修复"
echo "将备份并移走：$CONFIG_DIR"
echo "原数据不会删除。"
read -r -p "确认继续？输入 YES：" answer
if [[ "$answer" != "YES" ]]; then
  echo "已取消。"
  exit 0
fi

if [[ ! -d "$CONFIG_DIR" ]]; then
  echo "未找到配置目录，可能已经是全新配置：$CONFIG_DIR"
  exit 0
fi

mv "$CONFIG_DIR" "$BACKUP_DIR"
mkdir -p "$CONFIG_DIR"

echo
echo "处理完成。现在可以重新打开 CodexSwitcher。"
echo "备份位置：$BACKUP_DIR"
echo "请通过 OAuth 重新登录账号，并通过分享码/迁移包重新加入组网。"
