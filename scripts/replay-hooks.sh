#!/bin/bash
#
# replay-hooks.sh — Hook 事件回放工具
#
# 从录制文件中回放事件到指定节点，用于测试 ETerm/Server/iOS 链路
#
# 用法：
#   replay-hooks.sh <session_id> [选项]
#   replay-hooks.sh --list-sessions          列出所有录制
#
# 选项：
#   --target hook|eterm|server   回放注入点（默认 hook）
#   --event <类型>               只回放指定事件类型（可多次使用）
#   --from <序号>                从第 N 条开始（1-based）
#   --count <数量>               回放 N 条后停止
#   --speed <倍率>               回放速度（默认 0=立即，1=实时，2=2倍速）
#   --list                       只列出事件不回放
#   --dry-run                    显示要发送的内容，不实际发送
#

set -euo pipefail

REC_DIR="$HOME/.vimo/recordings"
AGENT_SOCK="$HOME/.vimo/agent.sock"
ETERM_SOCK_DIR="$HOME/.vimo/eterm/run/sockets"

# 颜色
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
DIM='\033[2m'
BOLD='\033[1m'
NC='\033[0m'

# 事件颜色映射
event_color() {
    case "$1" in
        SessionStart)       echo "$GREEN" ;;
        SessionEnd)         echo "$RED" ;;
        UserPromptSubmit)   echo "$CYAN" ;;
        PermissionRequest)  echo "$YELLOW" ;;
        Stop)               echo "$RED" ;;
        PreToolUse)         echo "$DIM" ;;
        PostToolUse)        echo "$DIM" ;;
        Notification)       echo "$BLUE" ;;
        *)                  echo "$NC" ;;
    esac
}

# 事件摘要提取
event_summary() {
    local event="$1"
    local raw="$2"

    case "$event" in
        UserPromptSubmit)
            local prompt_len=$(echo "$raw" | jq -r '.prompt // "" | length')
            echo "(${prompt_len} chars)"
            ;;
        PermissionRequest)
            local tool=$(echo "$raw" | jq -r '.tool_name // "?"')
            local input_preview=$(echo "$raw" | jq -r '.tool_input // {} | to_entries | map(.key + "=" + (.value | tostring | .[0:30])) | join(", ") | .[0:60]')
            echo "$tool ${input_preview}"
            ;;
        PreToolUse|PostToolUse)
            local tool=$(echo "$raw" | jq -r '.tool_name // "?"')
            echo "$tool"
            ;;
        Notification)
            local ntype=$(echo "$raw" | jq -r '.notification_type // "?"')
            echo "type=$ntype"
            ;;
        *)
            echo ""
            ;;
    esac
}

# 列出所有录制
list_sessions() {
    if [ ! -d "$REC_DIR" ] || [ -z "$(ls -A "$REC_DIR" 2>/dev/null)" ]; then
        echo "No recordings found in $REC_DIR"
        exit 0
    fi

    printf "${BOLD}%-40s %6s  %-20s  %-20s${NC}\n" "SESSION" "EVENTS" "FIRST" "LAST"
    for f in "$REC_DIR"/*.jsonl; do
        [ -f "$f" ] || continue
        local sid=$(basename "$f" .jsonl)
        local count=$(wc -l < "$f" | tr -d ' ')
        local first=$(head -1 "$f" | jq -r '"\(.event) \(.ts | split("T")[1] | split(".")[0])"' 2>/dev/null || echo "?")
        local last=$(tail -1 "$f" | jq -r '"\(.event) \(.ts | split("T")[1] | split(".")[0])"' 2>/dev/null || echo "?")
        printf "%-40s %6s  %-20s  %-20s\n" "$sid" "$count" "$first" "$last"
    done
}

# 查找录制文件（支持前缀匹配）
find_recording() {
    local query="$1"
    local exact="$REC_DIR/${query}.jsonl"
    if [ -f "$exact" ]; then
        echo "$exact"
        return 0
    fi

    # 前缀匹配
    local matches=("$REC_DIR"/${query}*.jsonl)
    if [ ${#matches[@]} -eq 1 ] && [ -f "${matches[0]}" ]; then
        echo "${matches[0]}"
        return 0
    elif [ ${#matches[@]} -gt 1 ]; then
        echo "Multiple matches for '$query':" >&2
        for m in "${matches[@]}"; do
            echo "  $(basename "$m" .jsonl)" >&2
        done
        return 1
    fi

    echo "No recording found for '$query'" >&2
    return 1
}

# 回放单条事件
replay_event() {
    local raw_json="$1"
    local target="$2"
    local dry_run="$3"

    case "$target" in
        hook)
            # 模拟 Claude Code hook：同时发送到 agent.sock 和 claude.sock
            if [ "$dry_run" = "true" ]; then
                echo "  → agent.sock + claude.sock"
                return
            fi
            if [ -S "$AGENT_SOCK" ]; then
                # 构造 agent HookEvent（和 claude_hook.sh 一致）
                local event_type=$(echo "$raw_json" | jq -r '.hook_event_name // "Unknown"')
                local session_id=$(echo "$raw_json" | jq -r '.session_id')
                local agent_json=$(echo "$raw_json" | jq -c '{type:"HookEvent",event_type:.hook_event_name,session_id:.session_id,transcript_path:.transcript_path,cwd:.cwd} + (if .tool_name then {tool_name:.tool_name,tool_input:.tool_input} else {} end)')
                (echo "$agent_json" | nc -w 1 -U "$AGENT_SOCK") &
                echo -e "  ${GREEN}→ agent.sock${NC}"
            else
                echo -e "  ${RED}✗ agent.sock not found${NC}"
            fi
            # 发送到 ETerm
            local eterm_sock=$(find "$ETERM_SOCK_DIR" -name "claude.sock" 2>/dev/null | head -1)
            if [ -n "$eterm_sock" ] && [ -S "$eterm_sock" ]; then
                local eterm_json=$(echo "$raw_json" | jq -c '{
                    event_type: (.hook_event_name | ascii_downcase | gsub("(?<a>[A-Z])"; "_" + .a | ascii_downcase) | ltrimstr("_")),
                    session_id: .session_id,
                    terminal_id: 0,
                    transcript_path: .transcript_path,
                    cwd: .cwd
                } + (if .tool_name then {tool_name:.tool_name,tool_input:.tool_input} else {} end)')
                (echo "$eterm_json" | nc -w 2 -U "$eterm_sock") &
                echo -e "  ${GREEN}→ claude.sock${NC}"
            else
                echo -e "  ${RED}✗ claude.sock not found${NC}"
            fi
            ;;

        eterm)
            # 只发送到 ETerm claude.sock
            local eterm_sock=$(find "$ETERM_SOCK_DIR" -name "claude.sock" 2>/dev/null | head -1)
            if [ "$dry_run" = "true" ]; then
                echo "  → claude.sock"
                return
            fi
            if [ -n "$eterm_sock" ] && [ -S "$eterm_sock" ]; then
                local eterm_json=$(echo "$raw_json" | jq -c '{
                    event_type: (.hook_event_name | ascii_downcase | gsub("(?<a>[A-Z])"; "_" + .a | ascii_downcase) | ltrimstr("_")),
                    session_id: .session_id,
                    terminal_id: 0,
                    transcript_path: .transcript_path,
                    cwd: .cwd
                } + (if .tool_name then {tool_name:.tool_name,tool_input:.tool_input} else {} end)')
                (echo "$eterm_json" | nc -w 2 -U "$eterm_sock") &
                echo -e "  ${GREEN}→ claude.sock${NC}"
            else
                echo -e "  ${RED}✗ claude.sock not found${NC}"
            fi
            ;;

        server)
            echo -e "  ${YELLOW}⚠ server target not yet implemented (needs Socket.IO client)${NC}"
            ;;
    esac
}

# ========================================
# 参数解析
# ========================================
SESSION_ID=""
TARGET="hook"
EVENTS=()
FROM=1
COUNT=0
SPEED=0
LIST_ONLY=false
DRY_RUN=false
LIST_SESSIONS=false

while [ $# -gt 0 ]; do
    case "$1" in
        --list-sessions) LIST_SESSIONS=true; shift ;;
        --target)  TARGET="$2"; shift 2 ;;
        --event)   EVENTS+=("$2"); shift 2 ;;
        --from)    FROM="$2"; shift 2 ;;
        --count)   COUNT="$2"; shift 2 ;;
        --speed)   SPEED="$2"; shift 2 ;;
        --list)    LIST_ONLY=true; shift ;;
        --dry-run) DRY_RUN=true; shift ;;
        -h|--help)
            sed -n '2,/^$/p' "$0" | sed 's/^# \?//'
            exit 0
            ;;
        *)
            if [ -z "$SESSION_ID" ]; then
                SESSION_ID="$1"
            fi
            shift
            ;;
    esac
done

# 列出所有录制
if [ "$LIST_SESSIONS" = "true" ]; then
    list_sessions
    exit 0
fi

# 必须指定 session
if [ -z "$SESSION_ID" ]; then
    echo "Usage: $0 <session_id> [--list] [--target hook|eterm|server] [--event <type>]"
    echo "       $0 --list-sessions"
    exit 1
fi

# 查找录制文件
REC_FILE=$(find_recording "$SESSION_ID") || exit 1
TOTAL=$(wc -l < "$REC_FILE" | tr -d ' ')
echo -e "${BOLD}Recording: $(basename "$REC_FILE" .jsonl)${NC} ($TOTAL events)"
echo ""

# ========================================
# 读取并处理事件
# ========================================
line_num=0
played=0
prev_ts=""

while IFS= read -r line; do
    line_num=$((line_num + 1))

    # --from 跳过
    [ $line_num -lt $FROM ] && continue

    # 解析事件
    event=$(echo "$line" | jq -r '.event // "?"')
    ts=$(echo "$line" | jq -r '.ts // "?"')
    raw=$(echo "$line" | jq -c '.raw // {}')
    ts_short=$(echo "$ts" | sed 's/.*T//;s/\..*//')

    # --event 过滤
    if [ ${#EVENTS[@]} -gt 0 ]; then
        match=false
        for e in "${EVENTS[@]}"; do
            [ "$event" = "$e" ] && match=true
        done
        [ "$match" = "false" ] && continue
    fi

    # 事件颜色和摘要
    color=$(event_color "$event")
    summary=$(event_summary "$event" "$raw")

    if [ "$LIST_ONLY" = "true" ]; then
        # 只列出
        printf "${DIM}%3d${NC} ${DIM}%s${NC} ${color}%-20s${NC} %s\n" "$line_num" "$ts_short" "$event" "$summary"
    else
        # 回放
        # 速度控制
        if [ "$SPEED" != "0" ] && [ -n "$prev_ts" ]; then
            # 简化：用秒级差值做延迟
            prev_sec=$(date -j -f "%H:%M:%S" "$prev_ts" +%s 2>/dev/null || echo "0")
            curr_sec=$(date -j -f "%H:%M:%S" "$ts_short" +%s 2>/dev/null || echo "0")
            if [ "$prev_sec" != "0" ] && [ "$curr_sec" != "0" ]; then
                delay=$(( (curr_sec - prev_sec) / SPEED ))
                [ $delay -gt 0 ] && [ $delay -lt 30 ] && sleep $delay
            fi
        fi
        prev_ts="$ts_short"

        printf "${DIM}%3d${NC} ${DIM}%s${NC} ${color}%-20s${NC} %s\n" "$line_num" "$ts_short" "$event" "$summary"
        replay_event "$raw" "$TARGET" "$DRY_RUN"

        played=$((played + 1))
    fi

    # --count 限制
    [ $COUNT -gt 0 ] && [ $played -ge $COUNT ] && break

done < "$REC_FILE"

echo ""
if [ "$LIST_ONLY" = "true" ]; then
    echo -e "${DIM}Total: $line_num events${NC}"
else
    echo -e "${DIM}Replayed: $played events (target=$TARGET)${NC}"
fi
