#!/bin/bash
# observe-diag.sh - 三端 [DIAG] 日志对齐观察工具
#
# 用法:
#   ./scripts/observe-diag.sh          # 拉最近 20 条
#   ./scripts/observe-diag.sh 50       # 拉最近 50 条
#   ./scripts/observe-diag.sh --watch  # 持续监控模式（每 2 秒轮询增量）
#
# 数据源:
#   ETerm:  ~/.vimo/eterm/logs/debug-{date}.log (文件)
#   Server: dev-runner REST API (http://localhost:9274)
#   iOS:    dev-runner REST API (http://localhost:9274)

set -euo pipefail

DR_BASE="http://localhost:9274"
DR_LOGS="/api/v1/projects/logs"
DR_PROJECTS="/api/v1/projects"
ETERM_LOG="$HOME/.vimo/eterm/logs/debug-$(date +%Y-%m-%d).log"
LIMIT="${1:-20}"

# 颜色
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

# ── 动态发现项目路径 ──────────────────────────────────────

discover_projects() {
    local projects
    projects=$(curl -sf "$DR_BASE$DR_PROJECTS" 2>/dev/null) || {
        echo -e "${RED}dev-runner 未运行 ($DR_BASE)${RESET}" >&2
        exit 1
    }

    # 找 running 的 server 和 iOS（status 可能是 running 或 success）
    SERVER_PATH=$(echo "$projects" | python3 -c "
import json, sys
data = json.load(sys.stdin)
for p in data.get('projects', []):
    if p.get('type') == 'node' and p.get('status') == 'running' and 'server' in p.get('name',''):
        print(p['path']); break
" 2>/dev/null)

    IOS_PATH=$(echo "$projects" | python3 -c "
import json, sys
data = json.load(sys.stdin)
for p in data.get('projects', []):
    if p.get('type') == 'xcode' and p.get('status') in ('running', 'success') and p.get('name') == 'Vlaude' and 'packages/Vlaude' in p.get('path',''):
        print(p['path']); break
" 2>/dev/null)

    if [ -z "$SERVER_PATH" ]; then
        echo -e "${YELLOW}Server 未运行${RESET}" >&2
    fi
    if [ -z "$IOS_PATH" ]; then
        echo -e "${YELLOW}iOS 未运行${RESET}" >&2
    fi
}

# ── 拉取日志 ──────────────────────────────────────────────

fetch_eterm() {
    local limit=$1
    if [ ! -f "$ETERM_LOG" ]; then
        echo -e "${YELLOW}ETerm 日志不存在: $ETERM_LOG${RESET}" >&2
        return
    fi
    grep '\[DIAG\]' "$ETERM_LOG" | tail -"$limit" | while IFS= read -r line; do
        # 提取时间戳（只取 HH:MM:SS.mmm）和 DIAG 内容
        local ts=$(echo "$line" | grep -oE '[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}')
        local diag=$(echo "$line" | grep -oE '\[DIAG\].*')
        echo "ETERM|$ts|$diag"
    done
}

fetch_devrunner() {
    local path=$1 source=$2 limit=$3 since=${4:-0}
    if [ -z "$path" ]; then return; fi

    local encoded_path
    encoded_path=$(python3 -c "import urllib.parse; print(urllib.parse.quote('$path'))")

    # 如果没有指定 since，从日志末尾往回取最后 N 条 DIAG
    if [ "$since" -eq 0 ]; then
        # 先获取 next_seq（日志末尾位置）
        local probe
        probe=$(curl -sf "$DR_BASE$DR_LOGS?path=$encoded_path&limit=1&since=999999999&action=run" 2>/dev/null) || true
        local next_seq
        next_seq=$(echo "$probe" | python3 -c "import json,sys; print(json.load(sys.stdin).get('next_seq',0))" 2>/dev/null)
        next_seq=${next_seq:-0}
        # 从末尾往前扫描足够范围（DIAG 行在 run 日志中稀疏，扫 2000 行）
        local scan_start=$((next_seq > 2000 ? next_seq - 2000 : 0))
        since=$scan_start
    fi

    local url="$DR_BASE$DR_LOGS?path=$encoded_path&search=%5BDIAG%5D&limit=500&action=run"
    if [ "$since" -gt 0 ]; then
        url="${url}&since=$since"
    fi

    local result
    result=$(curl -sf "$url" 2>/dev/null) || return

    echo "$result" | python3 -c "
import json, sys, re
data = json.load(sys.stdin)
entries = []
for line in data.get('lines', []):
    text = line.get('text', '')
    seq = line.get('seq', 0)
    diag_match = re.search(r'(\[DIAG\].*)', text)
    if not diag_match:
        continue
    diag = diag_match.group(1)
    # 优先从 DIAG 内容提取 ts= 字段（精确到毫秒）
    ts_inline = re.search(r'ts=(\d{2}:\d{2}:\d{2}\.\d{3})', diag)
    if ts_inline:
        ts = ts_inline.group(1)
    else:
        # 回退：提取 NestJS 外层时间戳
        ts_nest = re.search(r'(\d{1,2}/\d{1,2}/\d{4}, \d{1,2}:\d{2}:\d{2} [AP]M)', text)
        if ts_nest:
            from datetime import datetime
            try:
                dt = datetime.strptime(ts_nest.group(1), '%m/%d/%Y, %I:%M:%S %p')
                ts = dt.strftime('%H:%M:%S')
            except:
                ts = '?'
        else:
            ts = '?'
    entries.append(f'$source|{ts}|{diag}|seq={seq}')
# 只输出最后 limit 条
for e in entries[-$limit:]:
    print(e)
" 2>/dev/null

    # 输出 next_seq 到 stderr（用于增量轮询）
    echo "$result" | python3 -c "
import json, sys
data = json.load(sys.stdin)
print(data.get('next_seq', 0), file=sys.stderr)
" 2>/dev/null
}

# ── 对齐输出 ──────────────────────────────────────────────

align_output() {
    # 收集所有日志行，按 uuid 分组
    python3 -c "
import sys, re
from collections import defaultdict

entries = []  # (source, ts, uuid, role, raw)

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    parts = line.split('|', 3)
    if len(parts) < 3:
        continue
    source, ts, diag = parts[0], parts[1], parts[2]

    # 解析 [DIAG] action [ts=xxx] sid=xxx uuid=xxx role=xxx
    m = re.search(r'\[DIAG\]\s+(\w+)\s+(?:ts=\S+\s+)?sid=(\w+)\s+uuid=(\w+)\s+role=(\w+)', diag)
    if m:
        action, sid, uuid, role = m.groups()
        entries.append((source, ts, sid, uuid, role, action))

# 按 uuid 分组
by_uuid = defaultdict(dict)
order = []
for source, ts, sid, uuid, role, action in entries:
    if uuid not in by_uuid:
        order.append(uuid)
    by_uuid[uuid][source] = ts
    by_uuid[uuid]['role'] = role
    by_uuid[uuid]['sid'] = sid

if not order:
    print('  (no [DIAG] entries found)')
    sys.exit(0)

# 输出表头
print(f'  {\"uuid\":8s} | {\"role\":9s} | {\"ETerm push\":12s} | {\"Server relay\":12s} | {\"iOS recv\":12s}')
print(f'  {\"-\"*8}-+-{\"-\"*9}-+-{\"-\"*12}-+-{\"-\"*12}-+-{\"-\"*12}')

for uuid in order:
    d = by_uuid[uuid]
    role = d.get('role', '?')
    eterm = d.get('ETERM', '-')
    server = d.get('SERVER', '-')
    ios = d.get('IOS', '-')
    print(f'  {uuid:8s} | {role:9s} | {eterm:12s} | {server:12s} | {ios:12s}')
"
}

# ── 主流程 ──────────────────────────────────────────────

if [ "$LIMIT" = "--watch" ]; then
    # 持续监控模式
    discover_projects
    echo -e "${BOLD}── 三端 DIAG 持续监控 ──${RESET}"
    echo -e "${DIM}ETerm: $ETERM_LOG${RESET}"
    echo -e "${DIM}Server: ${SERVER_PATH:-N/A}${RESET}"
    echo -e "${DIM}iOS: ${IOS_PATH:-N/A}${RESET}"
    echo -e "${DIM}Ctrl+C 退出${RESET}"
    echo ""

    SVR_SEQ=0
    IOS_SEQ=0

    while true; do
        output=""

        # ETerm: 只取最新 5 条（增量近似）
        eterm_lines=$(fetch_eterm 5 2>/dev/null || true)

        # Server 增量
        svr_out=$(fetch_devrunner "$SERVER_PATH" "SERVER" 20 "$SVR_SEQ" 2>/tmp/dr_svr_seq || true)
        SVR_SEQ=$(cat /tmp/dr_svr_seq 2>/dev/null || echo "$SVR_SEQ")

        # iOS 增量
        ios_out=$(fetch_devrunner "$IOS_PATH" "IOS" 20 "$IOS_SEQ" 2>/tmp/dr_ios_seq || true)
        IOS_SEQ=$(cat /tmp/dr_ios_seq 2>/dev/null || echo "$IOS_SEQ")

        combined=$(printf '%s\n%s\n%s' "$eterm_lines" "$svr_out" "$ios_out" | grep -v '^$' || true)

        if [ -n "$combined" ]; then
            echo -e "${DIM}$(date +%H:%M:%S)${RESET}"
            echo "$combined" | align_output
            echo ""
        fi

        sleep 2
    done
else
    # 单次快照模式
    discover_projects

    echo -e "${BOLD}── 三端 DIAG 日志对齐 (最近 $LIMIT 条) ──${RESET}"
    echo ""

    # 并行拉取
    eterm_lines=$(fetch_eterm "$LIMIT" 2>/dev/null || true)
    svr_lines=$(fetch_devrunner "$SERVER_PATH" "SERVER" "$LIMIT" 0 2>/dev/null || true)
    ios_lines=$(fetch_devrunner "$IOS_PATH" "IOS" "$LIMIT" 0 2>/dev/null || true)

    printf '%s\n%s\n%s' "$eterm_lines" "$svr_lines" "$ios_lines" | align_output
fi
