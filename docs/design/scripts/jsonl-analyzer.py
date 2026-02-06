#!/usr/bin/env python3
"""
Claude Code JSONL 事件模型分析工具

用途: 全量分析 Claude Code 会话 JSONL 数据结构，生成统计报告。
配套文档: docs/design/jsonl-event-model.md

使用方式:
    python3 docs/design/scripts/jsonl-analyzer.py [options]

    # 全量分析（默认）
    python3 docs/design/scripts/jsonl-analyzer.py

    # 指定项目目录
    python3 docs/design/scripts/jsonl-analyzer.py --project /path/to/project

    # 分析特定 session
    python3 docs/design/scripts/jsonl-analyzer.py --session a6c8415b

    # 只跑某个分析模块
    python3 docs/design/scripts/jsonl-analyzer.py --module block_types
    python3 docs/design/scripts/jsonl-analyzer.py --module transition_matrix
    python3 docs/design/scripts/jsonl-analyzer.py --module turn_structure

    # 输出 JSON 格式（便于程序消费）
    python3 docs/design/scripts/jsonl-analyzer.py --format json --output report.json
"""

import json
import os
import sys
import glob
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone, timedelta

BJT = timezone(timedelta(hours=8))


# ============================================================
# JSONL 解析
# ============================================================

def parse_jsonl(path):
    """解析 JSONL 文件，返回所有有效行"""
    messages = []
    with open(path, encoding="utf-8") as f:
        for i, line in enumerate(f):
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
                obj["_line"] = i
                obj["_size"] = len(line)
                messages.append(obj)
            except json.JSONDecodeError:
                pass
    return messages


def find_jsonl_files(base_dir):
    """找到所有 JSONL 文件"""
    pattern = os.path.join(base_dir, "**", "*.jsonl")
    files = glob.glob(pattern, recursive=True)
    result = []
    for f in files:
        size = os.path.getsize(f)
        if size == 0:
            continue
        result.append({"path": f, "size": size, "name": os.path.basename(f)})
    result.sort(key=lambda x: -x["size"])
    return result


def find_project_dirs(base=None):
    """找到所有项目的 JSONL 目录"""
    if base is None:
        base = os.path.expanduser("~/.claude/projects")
    dirs = []
    for entry in os.listdir(base):
        full = os.path.join(base, entry)
        if os.path.isdir(full):
            jsonl_count = len(glob.glob(os.path.join(full, "*.jsonl")))
            if jsonl_count > 0:
                dirs.append({"path": full, "name": entry, "sessions": jsonl_count})
    dirs.sort(key=lambda x: -x["sessions"])
    return dirs


# ============================================================
# 分析模块
# ============================================================

def analyze_line_types(messages):
    """分析行类型分布"""
    type_counter = Counter()
    for msg in messages:
        type_counter[msg.get("type", "unknown")] += 1
    return dict(type_counter.most_common())


def analyze_block_types(messages):
    """分析 assistant 消息的 content block type 模式"""
    block_counter = Counter()
    pattern_counter = Counter()
    mixed_examples = []

    for msg in messages:
        if msg.get("type") != "assistant":
            continue
        content = msg.get("message", {}).get("content", [])
        if not isinstance(content, list):
            continue

        types = []
        for b in content:
            if isinstance(b, dict):
                bt = b.get("type", "unknown")
                block_counter[bt] += 1
                types.append(bt)

        combo = tuple(sorted(set(types)))
        if combo:
            pattern_counter[combo] += 1

        if len(set(types)) > 1:
            blocks_desc = []
            for b in content:
                if isinstance(b, dict):
                    bt = b.get("type", "?")
                    if bt == "thinking":
                        blocks_desc.append(f"thinking({len(b.get('thinking', ''))}ch)")
                    elif bt == "tool_use":
                        blocks_desc.append(f"tool_use({b.get('name', '')})")
                    elif bt == "text":
                        blocks_desc.append(f"text({len(b.get('text', ''))}ch)")
                    else:
                        blocks_desc.append(bt)
            mixed_examples.append({
                "line": msg.get("_line"),
                "blocks": blocks_desc,
            })

    return {
        "block_counts": dict(block_counter.most_common()),
        "patterns": {str(k): v for k, v in pattern_counter.most_common()},
        "total_assistant": sum(pattern_counter.values()),
        "mixed_count": len(mixed_examples),
        "mixed_examples": mixed_examples,
    }


def analyze_text_content(messages):
    """分析 text 块的内容分布"""
    categories = Counter()
    empty_positions = Counter()  # 空 text 出现在什么位置
    prev_type = None

    for msg in messages:
        t = msg.get("type")

        if t == "assistant":
            content = msg.get("message", {}).get("content", [])
            if isinstance(content, list):
                for b in content:
                    if isinstance(b, dict) and b.get("type") == "text":
                        text = b.get("text", "")
                        if not text.strip():
                            categories["empty_whitespace"] += 1
                            if prev_type == "user":
                                empty_positions["after_user"] += 1
                            else:
                                empty_positions["after_other"] += 1
                        elif len(text.strip()) < 5:
                            categories["very_short_lt5"] += 1
                        elif len(text.strip()) < 20:
                            categories["short_lt20"] += 1
                        else:
                            categories["substantial"] += 1

        if t in ("user", "assistant"):
            prev_type = t

    return {
        "categories": dict(categories.most_common()),
        "empty_positions": dict(empty_positions),
    }


def analyze_transition_matrix(messages):
    """分析事件转移矩阵"""
    transitions = Counter()
    prev_event = None

    for msg in messages:
        t = msg.get("type")
        event = None

        if t == "assistant":
            content = msg.get("message", {}).get("content", [])
            if isinstance(content, list):
                for b in content:
                    if isinstance(b, dict):
                        bt = b.get("type", "?")
                        if bt == "text":
                            txt = b.get("text", "")
                            event = "text_empty" if not txt.strip() else "text"
                        else:
                            event = bt

        elif t == "user":
            content = msg.get("message", {}).get("content", "")
            if isinstance(content, str) and content.strip() and not content.strip().startswith("<"):
                event = "user_question"
            elif isinstance(content, list):
                for b in content:
                    if isinstance(b, dict) and b.get("type") == "tool_result":
                        event = "tool_result"
                        break

        if event:
            if prev_event:
                transitions[(prev_event, event)] += 1
            prev_event = event

    result = []
    for (a, b), count in sorted(transitions.items(), key=lambda x: -x[1]):
        result.append({"from": a, "to": b, "count": count})
    return result


def analyze_turn_structure(messages):
    """分析 Turn 结构"""
    turns = []
    current_turn = []

    for msg in messages:
        t = msg.get("type")

        if t == "user":
            content = msg.get("message", {}).get("content", "")
            if isinstance(content, str) and content.strip() and not content.strip().startswith("<"):
                if current_turn:
                    turns.append(current_turn)
                current_turn = [{"line": msg.get("_line"), "event": "user_question",
                                 "detail": content.strip()[:80]}]
                continue
            elif isinstance(content, list):
                for b in content:
                    if isinstance(b, dict) and b.get("type") == "tool_result":
                        is_err = b.get("is_error", False)
                        current_turn.append({
                            "line": msg.get("_line"),
                            "event": "tool_result",
                            "detail": f"is_error={is_err}",
                        })
                        break
                continue

        if not current_turn:
            continue

        if t == "assistant":
            content = msg.get("message", {}).get("content", [])
            if isinstance(content, list):
                for b in content:
                    if isinstance(b, dict):
                        bt = b.get("type", "?")
                        if bt == "text":
                            txt = b.get("text", "")
                            if not txt.strip():
                                current_turn.append({
                                    "line": msg.get("_line"),
                                    "event": "empty_text",
                                    "detail": repr(txt[:10]),
                                })
                            else:
                                current_turn.append({
                                    "line": msg.get("_line"),
                                    "event": "text",
                                    "detail": txt[:60],
                                })
                        elif bt == "thinking":
                            current_turn.append({
                                "line": msg.get("_line"),
                                "event": "thinking",
                                "detail": f"{len(b.get('thinking', ''))}ch",
                            })
                        elif bt == "tool_use":
                            current_turn.append({
                                "line": msg.get("_line"),
                                "event": "tool_use",
                                "detail": b.get("name", ""),
                            })

        elif t == "progress":
            data = msg.get("data", {})
            if isinstance(data, dict) and data.get("type") == "mcp_progress":
                current_turn.append({
                    "line": msg.get("_line"),
                    "event": f"mcp_{data.get('status', '')}",
                    "detail": data.get("toolName", "")[:30],
                })

    if current_turn:
        turns.append(current_turn)

    # 统计
    turn_lengths = [len(t) for t in turns]
    event_types = Counter()
    for turn in turns:
        for item in turn:
            event_types[item["event"]] += 1

    return {
        "turn_count": len(turns),
        "turn_lengths": {
            "min": min(turn_lengths) if turn_lengths else 0,
            "max": max(turn_lengths) if turn_lengths else 0,
            "avg": round(sum(turn_lengths) / len(turn_lengths), 1) if turn_lengths else 0,
        },
        "event_distribution": dict(event_types.most_common()),
        "turns": turns,  # 完整 turn 数据
    }


def analyze_special_fields(messages):
    """分析特殊字段"""
    common_keys = {
        "parentUuid", "isSidechain", "userType", "cwd", "sessionId",
        "version", "gitBranch", "type", "message", "uuid", "timestamp",
        "tool", "toolUseId", "snapshot", "messageId", "isSnapshotUpdate",
        "_line", "_size",
    }

    special_fields = Counter()
    models = Counter()
    stop_reasons = Counter()
    sidechain_count = 0

    for msg in messages:
        for k in msg:
            if k not in common_keys:
                special_fields[k] += 1

        if msg.get("isSidechain"):
            sidechain_count += 1

        m = msg.get("message", {})
        if isinstance(m, dict):
            model = m.get("model", "")
            if model:
                models[model] += 1

        sr = msg.get("stopReason") or (m.get("stop_reason") if isinstance(m, dict) else None)
        if sr:
            stop_reasons[sr] += 1

    return {
        "special_fields": dict(special_fields.most_common()),
        "models": dict(models),
        "stop_reasons": dict(stop_reasons),
        "sidechain_count": sidechain_count,
    }


def analyze_tool_result(messages):
    """分析 tool_result"""
    ok_count = 0
    err_count = 0
    err_examples = []

    for msg in messages:
        if msg.get("type") != "user":
            continue
        content = msg.get("message", {}).get("content", [])
        if not isinstance(content, list):
            continue
        for b in content:
            if isinstance(b, dict) and b.get("type") == "tool_result":
                if b.get("is_error"):
                    err_count += 1
                    if len(err_examples) < 10:
                        rc = b.get("content", "")
                        if isinstance(rc, list):
                            for rb in rc:
                                if isinstance(rb, dict) and rb.get("type") == "text":
                                    err_examples.append(rb.get("text", "")[:150])
                                    break
                        elif isinstance(rc, str):
                            err_examples.append(rc[:150])
                else:
                    ok_count += 1

    return {
        "ok_count": ok_count,
        "error_count": err_count,
        "error_rate": f"{err_count / (ok_count + err_count) * 100:.1f}%" if (ok_count + err_count) > 0 else "N/A",
        "error_examples": err_examples,
    }


def analyze_tool_names(messages):
    """分析 tool_use name 分布"""
    names = Counter()
    for msg in messages:
        if msg.get("type") != "assistant":
            continue
        content = msg.get("message", {}).get("content", [])
        if isinstance(content, list):
            for b in content:
                if isinstance(b, dict) and b.get("type") == "tool_use":
                    names[b.get("name", "unknown")] += 1
    return dict(names.most_common())


def analyze_parallel_tools(messages):
    """分析并行 tool_use (same parent)"""
    parent_groups = defaultdict(list)
    for msg in messages:
        if msg.get("type") != "assistant":
            continue
        content = msg.get("message", {}).get("content", [])
        if isinstance(content, list):
            for b in content:
                if isinstance(b, dict) and b.get("type") == "tool_use":
                    parent = msg.get("parentUuid", "")
                    parent_groups[parent].append({
                        "line": msg.get("_line"),
                        "name": b.get("name", ""),
                        "uuid": msg.get("uuid", ""),
                    })

    parallel = []
    for p, items in parent_groups.items():
        if len(items) > 1:
            parallel.append({"parent": p[:20], "tools": items})

    return {
        "parallel_groups": len(parallel),
        "examples": parallel[:5],
    }


def analyze_progress(messages):
    """分析 progress 消息"""
    progress_types = Counter()
    for msg in messages:
        if msg.get("type") != "progress":
            continue
        data = msg.get("data", {})
        if isinstance(data, dict):
            progress_types[data.get("type", "unknown")] += 1
        else:
            progress_types["no_data"] += 1
    return dict(progress_types.most_common())


# ============================================================
# 报告生成
# ============================================================

def run_full_analysis(messages, file_info=None):
    """运行全部分析模块"""
    report = {}

    if file_info:
        report["file_info"] = file_info

    report["line_types"] = analyze_line_types(messages)
    report["block_types"] = analyze_block_types(messages)
    report["text_content"] = analyze_text_content(messages)
    report["transition_matrix"] = analyze_transition_matrix(messages)
    report["special_fields"] = analyze_special_fields(messages)
    report["tool_result"] = analyze_tool_result(messages)
    report["tool_names"] = analyze_tool_names(messages)
    report["parallel_tools"] = analyze_parallel_tools(messages)
    report["progress_types"] = analyze_progress(messages)

    # turn 结构放最后（数据量大）
    turn_data = analyze_turn_structure(messages)
    report["turn_structure"] = {
        "turn_count": turn_data["turn_count"],
        "turn_lengths": turn_data["turn_lengths"],
        "event_distribution": turn_data["event_distribution"],
    }

    return report


def print_report(report):
    """打印可读报告"""
    print("=" * 60)
    print("  Claude Code JSONL 事件模型分析报告")
    print("=" * 60)

    if "file_info" in report:
        fi = report["file_info"]
        print(f"\n文件数: {fi.get('file_count', '?')}")
        print(f"总行数: {fi.get('total_lines', '?')}")
        print(f"总大小: {fi.get('total_size_mb', '?')} MB")

    print(f"\n--- 行类型分布 ---")
    for k, v in report.get("line_types", {}).items():
        print(f"  {k:30s}: {v}")

    bt = report.get("block_types", {})
    print(f"\n--- Assistant Block Type ---")
    print(f"  总 assistant 消息: {bt.get('total_assistant', 0)}")
    print(f"  混合 block type: {bt.get('mixed_count', 0)} 条")
    print(f"  Block 计数:")
    for k, v in bt.get("block_counts", {}).items():
        print(f"    {k}: {v}")
    print(f"  模式:")
    for k, v in bt.get("patterns", {}).items():
        print(f"    {k}: {v}")
    if bt.get("mixed_examples"):
        print(f"  混合示例:")
        for ex in bt["mixed_examples"]:
            print(f"    L{ex['line']}: {' + '.join(ex['blocks'])}")

    tc = report.get("text_content", {})
    print(f"\n--- Text 内容分布 ---")
    for k, v in tc.get("categories", {}).items():
        print(f"  {k}: {v}")
    print(f"  空 text 位置: {tc.get('empty_positions', {})}")

    print(f"\n--- 事件转移矩阵 (top 15) ---")
    for item in report.get("transition_matrix", [])[:15]:
        print(f"  {item['from']:15s} → {item['to']:15s}: {item['count']}")

    sf = report.get("special_fields", {})
    print(f"\n--- 特殊字段 ---")
    print(f"  models: {sf.get('models', {})}")
    print(f"  stopReason: {sf.get('stop_reasons', {}) or '(无)'}")
    print(f"  sidechain: {sf.get('sidechain_count', 0)}")

    tr = report.get("tool_result", {})
    print(f"\n--- Tool Result ---")
    print(f"  正常: {tr.get('ok_count', 0)}, 错误: {tr.get('error_count', 0)} ({tr.get('error_rate', 'N/A')})")

    print(f"\n--- Tool 分布 (top 15) ---")
    for i, (k, v) in enumerate(report.get("tool_names", {}).items()):
        if i >= 15:
            break
        print(f"  {v:5d} {k}")

    pt = report.get("parallel_tools", {})
    print(f"\n--- 并行 Tool ---")
    print(f"  并行组数: {pt.get('parallel_groups', 0)}")

    print(f"\n--- Progress 类型 ---")
    for k, v in report.get("progress_types", {}).items():
        print(f"  {k}: {v}")

    ts = report.get("turn_structure", {})
    print(f"\n--- Turn 结构 ---")
    print(f"  Turn 数: {ts.get('turn_count', 0)}")
    print(f"  Turn 长度: {ts.get('turn_lengths', {})}")
    print(f"  事件分布:")
    for k, v in ts.get("event_distribution", {}).items():
        print(f"    {k}: {v}")


# ============================================================
# 多文件聚合分析
# ============================================================

def aggregate_analysis(base_dir, session_filter=None, limit=None):
    """聚合分析多个 JSONL 文件"""
    files = find_jsonl_files(base_dir)

    if session_filter:
        files = [f for f in files if session_filter in f["name"]]

    if limit:
        files = files[:limit]

    all_messages = []
    total_size = 0

    for fi in files:
        msgs = parse_jsonl(fi["path"])
        # 标记来源
        for m in msgs:
            m["_source"] = fi["name"]
        all_messages.extend(msgs)
        total_size += fi["size"]

    file_info = {
        "file_count": len(files),
        "total_lines": len(all_messages),
        "total_size_mb": round(total_size / 1024 / 1024, 1),
    }

    return run_full_analysis(all_messages, file_info)


# ============================================================
# CLI
# ============================================================

def main():
    parser = argparse.ArgumentParser(description="Claude Code JSONL 事件模型分析")
    parser.add_argument("--project", type=str, default=None,
                        help="项目 JSONL 目录路径（默认: ~/.claude/projects 下的 ETerm）")
    parser.add_argument("--session", type=str, default=None,
                        help="分析特定 session（UUID 前缀匹配）")
    parser.add_argument("--limit", type=int, default=None,
                        help="限制分析的文件数量（按大小排序取 top N）")
    parser.add_argument("--all-projects", action="store_true",
                        help="分析所有项目（默认只分析 ETerm）")
    parser.add_argument("--module", type=str, default=None,
                        choices=["line_types", "block_types", "text_content",
                                 "transition_matrix", "turn_structure",
                                 "special_fields", "tool_result", "tool_names",
                                 "parallel_tools", "progress"],
                        help="只运行指定分析模块")
    parser.add_argument("--format", type=str, default="text",
                        choices=["text", "json"],
                        help="输出格式")
    parser.add_argument("--output", type=str, default=None,
                        help="输出文件路径（默认输出到 stdout）")
    parser.add_argument("--list-projects", action="store_true",
                        help="列出所有项目")

    args = parser.parse_args()

    # 列出项目
    if args.list_projects:
        dirs = find_project_dirs()
        print(f"共 {len(dirs)} 个项目:\n")
        for d in dirs:
            print(f"  {d['sessions']:5d} sessions  {d['name']}")
        return

    # 确定分析目录
    if args.project:
        base_dir = args.project
    elif args.all_projects:
        base_dir = os.path.expanduser("~/.claude/projects")
    else:
        # 默认 ETerm
        base = os.path.expanduser("~/.claude/projects")
        candidates = [d for d in os.listdir(base) if "ETerm" in d]
        if candidates:
            base_dir = os.path.join(base, candidates[0])
        else:
            print("未找到 ETerm 项目，请用 --project 指定路径")
            sys.exit(1)

    if not os.path.isdir(base_dir):
        print(f"目录不存在: {base_dir}")
        sys.exit(1)

    # 单模块分析
    if args.module:
        files = find_jsonl_files(base_dir)
        if args.session:
            files = [f for f in files if args.session in f["name"]]
        if args.limit:
            files = files[:args.limit]

        all_messages = []
        for fi in files:
            all_messages.extend(parse_jsonl(fi["path"]))

        module_map = {
            "line_types": analyze_line_types,
            "block_types": analyze_block_types,
            "text_content": analyze_text_content,
            "transition_matrix": analyze_transition_matrix,
            "turn_structure": analyze_turn_structure,
            "special_fields": analyze_special_fields,
            "tool_result": analyze_tool_result,
            "tool_names": analyze_tool_names,
            "parallel_tools": analyze_parallel_tools,
            "progress": analyze_progress,
        }

        result = module_map[args.module](all_messages)
        if args.format == "json":
            output = json.dumps(result, ensure_ascii=False, indent=2, default=str)
        else:
            output = json.dumps(result, ensure_ascii=False, indent=2, default=str)

        if args.output:
            with open(args.output, "w") as f:
                f.write(output)
            print(f"写入: {args.output}")
        else:
            print(output)
        return

    # 全量分析
    report = aggregate_analysis(base_dir, session_filter=args.session, limit=args.limit)

    if args.format == "json":
        output = json.dumps(report, ensure_ascii=False, indent=2, default=str)
        if args.output:
            with open(args.output, "w") as f:
                f.write(output)
            print(f"写入: {args.output}")
        else:
            print(output)
    else:
        print_report(report)
        if args.output:
            # text 模式也支持写文件
            import io
            buf = io.StringIO()
            old_stdout = sys.stdout
            sys.stdout = buf
            print_report(report)
            sys.stdout = old_stdout
            with open(args.output, "w") as f:
                f.write(buf.getvalue())
            print(f"\n写入: {args.output}")


if __name__ == "__main__":
    main()
