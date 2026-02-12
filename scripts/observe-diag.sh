#!/bin/bash
# observe-diag.sh - 全链路事件追踪工具
#
# 用法:
#   ./scripts/observe-diag.sh          # 拉最近 30 条事件
#   ./scripts/observe-diag.sh 100      # 拉最近 100 条
#   ./scripts/observe-diag.sh --watch  # 持续监控（每 2 秒增量）
#   ./scripts/observe-diag.sh --hooks  # 只看 hook 事件（过滤消息）
#   ./scripts/observe-diag.sh --msgs   # 只看消息推送
#
# 数据源:
#   ETerm:  ~/.vimo/eterm/logs/debug-{date}.log
#   Server: dev-runner REST API (http://localhost:9274)
#   iOS:    dev-runner REST API (http://localhost:9274)

set -euo pipefail

DR_BASE="http://localhost:9274"
DR_LOGS="/api/v1/projects/logs"
DR_PROJECTS="/api/v1/projects"
ETERM_LOG="$HOME/.vimo/eterm/logs/debug-$(date +%Y-%m-%d).log"

# 参数
LIMIT=30
MODE="all"
WATCH=false

while [ $# -gt 0 ]; do
    case "$1" in
        --watch) WATCH=true; shift ;;
        --hooks) MODE="hooks"; shift ;;
        --msgs)  MODE="msgs"; shift ;;
        -h|--help)
            sed -n '2,/^$/p' "$0" | sed 's/^# \?//'
            exit 0
            ;;
        *)
            if [[ "$1" =~ ^[0-9]+$ ]]; then
                LIMIT="$1"
            fi
            shift
            ;;
    esac
done

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

    echo -e "${DIM}ETerm:  $ETERM_LOG${RESET}" >&2
    echo -e "${DIM}Server: ${SERVER_PATH:-N/A}${RESET}" >&2
    echo -e "${DIM}iOS:    ${IOS_PATH:-N/A}${RESET}" >&2
}

# ── 拉取日志 ──────────────────────────────────────────────

fetch_eterm() {
    local limit=$1
    if [ ! -f "$ETERM_LOG" ]; then return; fi
    grep '\[DIAG\]' "$ETERM_LOG" 2>/dev/null | tail -"$limit" | while IFS= read -r line; do
        local diag=$(echo "$line" | grep -oE '\[DIAG\].*')
        echo "ETERM|$diag"
    done
}

fetch_devrunner() {
    local path=$1 source=$2 limit=$3 since=${4:-0}
    if [ -z "$path" ]; then return; fi

    local encoded_path
    encoded_path=$(python3 -c "import urllib.parse; print(urllib.parse.quote('$path'))")

    if [ "$since" -eq 0 ]; then
        local probe
        probe=$(curl -sf "$DR_BASE$DR_LOGS?path=$encoded_path&limit=1&since=999999999&action=run" 2>/dev/null) || true
        local next_seq
        next_seq=$(echo "$probe" | python3 -c "import json,sys; print(json.load(sys.stdin).get('next_seq',0))" 2>/dev/null)
        next_seq=${next_seq:-0}
        local scan_start=$((next_seq > 5000 ? next_seq - 5000 : 0))
        since=$scan_start
    fi

    local url="$DR_BASE$DR_LOGS?path=$encoded_path&search=%5BDIAG%5D&limit=1000&action=run"
    if [ "$since" -gt 0 ]; then
        url="${url}&since=$since"
    fi

    local result
    result=$(curl -sf "$url" 2>/dev/null) || return

    echo "$result" | python3 -c "
import json, sys, re
data = json.load(sys.stdin)
for line in data.get('lines', []):
    text = line.get('text', '')
    seq = line.get('seq', 0)
    diag_match = re.search(r'(\[DIAG\].*)', text)
    if diag_match:
        print(f'$source|{diag_match.group(1)}')
" 2>/dev/null

    # next_seq → stderr
    echo "$result" | python3 -c "
import json, sys
data = json.load(sys.stdin)
print(data.get('next_seq', 0), file=sys.stderr)
" 2>/dev/null
}

# ── 对齐输出 ──────────────────────────────────────────────

align_output() {
    local mode=$1
    python3 -c "
import sys, re
from collections import defaultdict
from datetime import datetime

# 解析所有 DIAG 行
entries = []

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    parts = line.split('|', 1)
    if len(parts) < 2:
        continue
    source, diag = parts

    # Hook 事件: [DIAG] hook-xxx ts=HH:mm:ss.SSS event=Type sid=xxx [tool=xxx] [toolUseId=xxx]
    m = re.search(r'\[DIAG\]\s+(hook-\w+)\s+ts=(\d{2}:\d{2}:\d{2}\.\d{3})\s+event=(\w+)\s+sid=(\w+)(.*)', diag)
    if m:
        tag, ts, event, sid, rest = m.groups()
        tool = ''
        tool_m = re.search(r'tool=(\S+)', rest)
        if tool_m:
            tool = tool_m.group(1)
        tool_use_id = ''
        tui_m = re.search(r'toolUseId=(\S+)', rest)
        if tui_m:
            tool_use_id = tui_m.group(1)
        subscribers = ''
        sub_m = re.search(r'subscribers=(\d+)', rest)
        if sub_m:
            subscribers = sub_m.group(1)
        entries.append({
            'type': 'hook',
            'source': source,
            'tag': tag,
            'ts': ts,
            'event': event,
            'sid': sid,
            'tool': tool,
            'toolUseId': tool_use_id,
            'subscribers': subscribers,
        })
        continue

    # 消息事件: [DIAG] push/relay/recv ts=HH:mm:ss.SSS sid=xxx uuid=xxx role=xxx [subscribers=N]
    m = re.search(r'\[DIAG\]\s+(push|relay|recv)\s+ts=(\d{2}:\d{2}:\d{2}\.\d{3})\s+sid=(\w+)\s+uuid=(\w+)\s+role=(\w+)(.*)', diag)
    if m:
        tag, ts, sid, uuid, role, rest = m.groups()
        subscribers = ''
        sub_m = re.search(r'subscribers=(\d+)', rest)
        if sub_m:
            subscribers = sub_m.group(1)
        entries.append({
            'type': 'msg',
            'source': source,
            'tag': tag,
            'ts': ts,
            'sid': sid,
            'uuid': uuid,
            'role': role,
            'subscribers': subscribers,
        })
        continue

mode = '$mode'

# 按 mode 过滤
if mode == 'hooks':
    entries = [e for e in entries if e['type'] == 'hook']
elif mode == 'msgs':
    entries = [e for e in entries if e['type'] == 'msg']

if not entries:
    print('  (no [DIAG] entries found)')
    sys.exit(0)

def ts_to_ms(ts_str):
    try:
        parts = ts_str.split(':')
        h, m = int(parts[0]), int(parts[1])
        sec_parts = parts[2].split('.')
        s = int(sec_parts[0])
        ms = int(sec_parts[1]) if len(sec_parts) > 1 else 0
        return ((h * 60 + m) * 60 + s) * 1000 + ms
    except:
        return 0

# ── Hook 事件对齐 ──
# 同一个事件的 hook-out / hook-in / hook-relay / hook-recv 关联
# 关联 key: (event, sid, tool, ts 在 2s 窗口内)

hook_groups = []  # [{event, sid, tool, hook-out_ts, hook-in_ts, hook-relay_ts, hook-recv_ts, toolUseId, subscribers}]
hook_index = {}   # (event, sid, tool, ts_bucket) → group，用于快速查找

def find_hook_group(event, sid, tool, ts_ms):
    # 在 ±2s 的 bucket 范围内查找匹配的 group
    bucket = ts_ms // 1000
    for b in range(bucket - 2, bucket + 3):
        key = (event, sid, tool, b)
        if key in hook_index:
            g = hook_index[key]
            if abs(ts_ms - g['first_ts_ms']) <= 2000:
                return g
    return None

def register_hook_group(g, ts_ms):
    bucket = ts_ms // 1000
    for b in range(bucket - 2, bucket + 3):
        key = (g['event'], g['sid'], g.get('tool',''), b)
        if key not in hook_index:
            hook_index[key] = g

for e in entries:
    if e['type'] != 'hook':
        continue
    tag = e['tag']
    ts_ms = ts_to_ms(e['ts'])

    g = find_hook_group(e['event'], e['sid'], e.get('tool',''), ts_ms)
    if g:
        g[tag] = e['ts']
        if e.get('toolUseId') and not g.get('toolUseId'):
            g['toolUseId'] = e['toolUseId']
        if e.get('subscribers'):
            g['subscribers'] = e['subscribers']
    else:
        g = {
            'event': e['event'],
            'sid': e['sid'],
            'tool': e.get('tool', ''),
            'toolUseId': e.get('toolUseId', ''),
            'subscribers': e.get('subscribers', ''),
            tag: e['ts'],
            'first_ts_ms': ts_ms,
        }
        hook_groups.append(g)
        register_hook_group(g, ts_ms)

# ── 消息事件对齐 ──
msg_groups = {}  # uuid → {push_ts, relay_ts, recv_ts, sid, role, subscribers}
msg_order = []

for e in entries:
    if e['type'] != 'msg':
        continue
    uuid = e['uuid']
    if uuid not in msg_groups:
        msg_groups[uuid] = {'sid': e['sid'], 'role': e['role'], 'uuid': uuid}
        msg_order.append(uuid)
    tag_map = {'push': 'push_ts', 'relay': 'relay_ts', 'recv': 'recv_ts'}
    if e['tag'] in tag_map:
        msg_groups[uuid][tag_map[e['tag']]] = e['ts']
    if e.get('subscribers'):
        msg_groups[uuid]['subscribers'] = e['subscribers']

# ── 统一排序 + 输出 ──
all_events = []

for g in hook_groups:
    # 取最早的时间戳作为排序 key
    ts = g.get('hook-out') or g.get('hook-in') or g.get('hook-relay') or g.get('hook-recv') or '?'
    detail = g.get('tool', '')
    if g.get('toolUseId'):
        detail += f\" tuId={g['toolUseId'][:8]}\"

    out_ts = g.get('hook-out', '')
    in_ts = g.get('hook-in', '')
    relay_ts = g.get('hook-relay', '')
    recv_ts = g.get('hook-recv', '')

    # 计算端到端延迟
    latency = ''
    first_ms = ts_to_ms(out_ts) if out_ts else (ts_to_ms(in_ts) if in_ts else 0)
    last_ms = ts_to_ms(recv_ts) if recv_ts else (ts_to_ms(relay_ts) if relay_ts else (ts_to_ms(in_ts) if in_ts else 0))
    if first_ms and last_ms and last_ms >= first_ms:
        diff = last_ms - first_ms
        latency = f'{diff}ms'

    all_events.append({
        'sort_ts': ts,
        'ts': ts[:8] if len(ts) >= 8 else ts,
        'kind': g['event'],
        'sid': g['sid'],
        'detail': detail,
        'eterm': '\033[0;32m✅\033[0m ' + out_ts[-6:] if out_ts else '\033[2m·\033[0m',
        'server': '\033[0;32m✅\033[0m ' + in_ts[-6:] if in_ts else '\033[2m·\033[0m',
        'relay': '\033[0;32m✅\033[0m ' + relay_ts[-6:] if relay_ts else ('\033[2m·\033[0m' if not recv_ts else '\033[0;31m❌\033[0m'),
        'ios': '\033[0;32m✅\033[0m ' + recv_ts[-6:] if recv_ts else '\033[2m·\033[0m',
        'latency': latency,
        'subscribers': g.get('subscribers', ''),
    })

for uuid in msg_order:
    g = msg_groups[uuid]
    push_ts = g.get('push_ts', '')
    relay_ts = g.get('relay_ts', '')
    recv_ts = g.get('recv_ts', '')
    ts = push_ts or relay_ts or recv_ts or '?'

    latency = ''
    first_ms = ts_to_ms(push_ts) if push_ts else 0
    last_ms = ts_to_ms(recv_ts) if recv_ts else 0
    if first_ms and last_ms and last_ms >= first_ms:
        diff = last_ms - first_ms
        latency = f'{diff}ms'

    all_events.append({
        'sort_ts': ts,
        'ts': ts[:8] if len(ts) >= 8 else ts,
        'kind': 'msg',
        'sid': g['sid'],
        'detail': f\"{g['role']} {g['uuid'][:6]}\",
        'eterm': '\033[0;32m✅\033[0m ' + push_ts[-6:] if push_ts else '\033[0;31m❌\033[0m',
        'server': '\033[0;32m✅\033[0m ' + relay_ts[-6:] if relay_ts else '\033[0;31m❌\033[0m',
        'relay': '',
        'ios': '\033[0;32m✅\033[0m ' + recv_ts[-6:] if recv_ts else '\033[0;31m❌\033[0m',
        'latency': latency,
        'subscribers': g.get('subscribers', ''),
    })

# 按时间排序
all_events.sort(key=lambda e: e['sort_ts'])

# 只保留最后 LIMIT 条
all_events = all_events[-$LIMIT:]

# 输出表头
print()
if mode in ('all', 'hooks'):
    print(f\"  {'时间':8s}  {'事件':18s}  {'Session':8s}  {'详情':20s}  {'ETerm':12s}  {'Server':12s}  {'→iOS':12s}  {'iOS':12s}  {'延迟':>6s}\")
    print(f\"  {'─'*8}  {'─'*18}  {'─'*8}  {'─'*20}  {'─'*12}  {'─'*12}  {'─'*12}  {'─'*12}  {'─'*6}\")
else:
    print(f\"  {'时间':8s}  {'Session':8s}  {'详情':20s}  {'ETerm':12s}  {'Server':12s}  {'iOS':12s}  {'延迟':>6s}\")
    print(f\"  {'─'*8}  {'─'*8}  {'─'*20}  {'─'*12}  {'─'*12}  {'─'*12}  {'─'*6}\")

for e in all_events:
    if mode in ('all', 'hooks') and e['kind'] != 'msg':
        # Hook 事件有 4 列: ETerm, Server, →iOS(relay), iOS(recv)
        print(f\"  {e['ts']:8s}  {e['kind']:18s}  {e['sid']:8s}  {e['detail']:20s}  {e['eterm']}  {e['server']}  {e['relay']}  {e['ios']}  {e['latency']:>6s}\")
    elif mode in ('all', 'msgs') and e['kind'] == 'msg':
        if mode == 'all':
            print(f\"  {e['ts']:8s}  {'msg':18s}  {e['sid']:8s}  {e['detail']:20s}  {e['eterm']}  {e['server']}          {e['ios']}  {e['latency']:>6s}\")
        else:
            print(f\"  {e['ts']:8s}  {e['sid']:8s}  {e['detail']:20s}  {e['eterm']}  {e['server']}  {e['ios']}  {e['latency']:>6s}\")

print()
print(f\"  \033[2mTotal: {len(all_events)} events\033[0m\")
"
}

# ── 主流程 ──────────────────────────────────────────────

if [ "$WATCH" = "true" ]; then
    discover_projects
    echo ""
    echo -e "${BOLD}── 全链路事件追踪 (持续监控) ──${RESET}"
    echo -e "${DIM}Ctrl+C 退出${RESET}"
    echo ""

    SVR_SEQ=0
    IOS_SEQ=0

    while true; do
        eterm_lines=$(fetch_eterm 20 2>/dev/null || true)
        svr_out=$(fetch_devrunner "$SERVER_PATH" "SERVER" 50 "$SVR_SEQ" 2>/tmp/dr_svr_seq || true)
        SVR_SEQ=$(cat /tmp/dr_svr_seq 2>/dev/null || echo "$SVR_SEQ")
        ios_out=$(fetch_devrunner "$IOS_PATH" "IOS" 50 "$IOS_SEQ" 2>/tmp/dr_ios_seq || true)
        IOS_SEQ=$(cat /tmp/dr_ios_seq 2>/dev/null || echo "$IOS_SEQ")

        combined=$(printf '%s\n%s\n%s' "$eterm_lines" "$svr_out" "$ios_out" | grep -v '^$' || true)

        if [ -n "$combined" ]; then
            clear
            echo -e "${BOLD}── 全链路事件追踪 ──${RESET}  ${DIM}$(date +%H:%M:%S) | mode=$MODE${RESET}"
            echo "$combined" | align_output "$MODE"
        fi

        sleep 2
    done
else
    discover_projects

    echo ""
    echo -e "${BOLD}── 全链路事件追踪 (最近 $LIMIT 条) ──${RESET}"

    eterm_lines=$(fetch_eterm "$LIMIT" 2>/dev/null || true)
    svr_lines=$(fetch_devrunner "$SERVER_PATH" "SERVER" "$LIMIT" 0 2>/dev/null || true)
    ios_lines=$(fetch_devrunner "$IOS_PATH" "IOS" "$LIMIT" 0 2>/dev/null || true)

    printf '%s\n%s\n%s' "$eterm_lines" "$svr_lines" "$ios_lines" | align_output "$MODE"
fi
