#!/bin/bash
#
# list-recordings.sh — 查看 hook 录制概览
#
# 用法：
#   list-recordings.sh              列出所有录制的 session
#   list-recordings.sh <session_id> 显示指定 session 的事件时间线
#

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

if [ $# -eq 0 ]; then
    exec "$SCRIPT_DIR/replay-hooks.sh" --list-sessions
else
    exec "$SCRIPT_DIR/replay-hooks.sh" "$1" --list
fi
