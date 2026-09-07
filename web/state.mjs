// Pure state helpers: shared by the browser and the dependency-free Node tests.
export const PAGE_SIZE = 100;
export const MAX_VISIBLE_ITEMS = 64;

// Connections express recorded order only, never inferred causes or model thoughts.
export function questionPath(turns, selected) {
  const nodes = turns
    .slice(0, PAGE_SIZE)
    .sort((a, b) => a.index - b.index)
    .map((turn) => ({
      ...turn,
      selected: turn.index === selected,
    }));
  const edges = nodes.slice(1).map((node, index) => {
    const from = nodes[index].index;
    const skipped = Math.max(0, node.index - from - 1);
    return {
      from,
      to: node.index,
      skipped,
      label: skipped ? `中间 ${skipped} 轮未展示` : "下一轮 · 记录顺序",
    };
  });
  return { nodes, edges };
}

export function processPath(detail, items, start = 0, next = null) {
  if (!detail) return [];
  const prompt = detail.turn?.prompt || {};
  const nodes = [
    {
      id: "prompt",
      kind: "prompt",
      label: "你的问题",
      text: prompt.text || prompt.preview || "（无文字内容）",
      imagesCount: prompt.images_count || 0,
      omittedBytes: prompt.omitted_bytes || 0,
      notice: prompt.omitted_bytes
        ? `${prompt.omitted_bytes} 字节原文因内存预算省略；${prompt.text ? "以上为已保留内容" : "以上仅为目录摘要"}，此视图不能恢复省略内容。`
        : "",
    },
  ];
  if (start > 0)
    nodes.push({
      id: "before",
      kind: "gap",
      label: `${start} 条前序记录未展示`,
      text: "可加载上一组记录；当前连线不表示内容完整或因果关系。",
    });
  const visible = items.slice(0, MAX_VISIBLE_ITEMS);
  const labels = {
    agent_message: "助手消息",
    tool_call: "工具调用",
    tool_output: "工具结果",
    file_activity: "文件活动",
    notice: "提示记录",
    omitted: "省略记录",
  };
  for (const item of visible) {
    const isFinal =
      item.type === "agent_message" && item.phase === "final_answer";
    const omitted = item.type === "omitted";
    nodes.push({
      id: `item-${item.index}`,
      kind: item.type,
      label: isFinal
        ? "最终回复（已标记）"
        : Object.hasOwn(labels, item.type)
          ? labels[item.type]
          : "其他记录",
      text: omitted
        ? "此条活动正文因内存预算省略；此视图不能恢复省略内容。"
        : item.text || item.summary || item.path || "（无文字内容）",
      itemIndex: item.index,
      isError: Boolean(item.is_error),
      isFinal,
      omitted,
      phase: item.phase || null,
      name: item.name || "",
    });
  }
  const remaining = Math.max(
    0,
    (detail.items_total || 0) - start - visible.length,
  );
  if (remaining > 0 || next !== null)
    nodes.push({
      id: "after",
      kind: "gap",
      label:
        remaining > 0 ? `${remaining} 条后续记录未展示` : "后续记录尚未展示",
      text: "可继续加载记录；未展示不代表没有后续活动。",
    });
  return nodes;
}

export function activityWindow(start, length, requestedOffset) {
  return {
    replace: requestedOffset < start || length >= MAX_VISIBLE_ITEMS,
    start:
      requestedOffset < start || length >= MAX_VISIBLE_ITEMS
        ? requestedOffset
        : start,
  };
}

export function statusLabel(status) {
  return (
    {
      in_progress: "… 未结束",
      completed: "✓ 正常结束",
      failed: "✕ 执行错误",
      interrupted: "⊘ 已中断",
      unknown: "? 状态未知",
      rolled_back: "↶ 已回滚",
    }[status] || "? 状态未知"
  );
}

export function sessionTitle(session) {
  const source = [session.title, session.first_prompt, session.id].find(
    (value) => typeof value === "string" && value.trim(),
  );
  if (!source) return "未命名会话";
  const line = source
    .split(/\r?\n/u)
    .find((value) => value.trim())
    .trim();
  const characters = Array.from(line);
  return characters.length > 90 ? characters.slice(0, 90).join("") + "…" : line;
}

export function filterSessions(sessions, query) {
  const needle = query.trim().toLocaleLowerCase();
  return sessions.filter((session) =>
    [
      sessionTitle(session),
      session.title,
      session.first_prompt,
      session.cwd,
      session.id,
    ]
      .join(" ")
      .toLocaleLowerCase()
      .includes(needle),
  );
}

export function navigationContextMatches(previous, next) {
  return (
    previous.view === next.view &&
    previous.key === next.key &&
    previous.query === next.query
  );
}

export function pausesFollow(key) {
  return [
    "PageDown",
    "PageUp",
    "ArrowDown",
    "ArrowUp",
    "Home",
    "End",
    " ",
  ].includes(key);
}

export function pageFor(index, size = PAGE_SIZE) {
  return Math.floor(Math.max(0, index) / size) * size;
}

// Metadata may already announce a new turn while its directory page is still in flight.
export function chronologicalTarget(direction, selected, meta) {
  if (!meta?.turn_count) return null;
  if (direction === "start") return 0;
  if (direction === "end") return meta.latest_active ?? meta.turn_count - 1;
  return Math.max(
    0,
    Math.min(
      meta.turn_count - 1,
      (selected ?? 0) + (direction === "next" ? 1 : -1),
    ),
  );
}

export function selectionAfterUpdate(
  selected,
  previous,
  next,
  follow,
  searching,
) {
  if (!next.turn_count) return null;
  const latest = next.latest_active ?? next.turn_count - 1;
  if (selected === null || previous?.generation !== next.generation)
    return latest;
  if (selected >= next.turn_count) return latest;
  return follow && !searching ? latest : selected;
}

export function navigation(key, focus) {
  if (!["j", "k", "g", "G"].includes(key)) return null;
  if (focus === "reader") {
    return {
      kind: "scroll",
      direction: { j: "down", k: "up", g: "start", G: "end" }[key],
    };
  }
  return {
    kind: "select",
    direction: { j: "next", k: "previous", g: "start", G: "end" }[key],
  };
}

export class RequestGate {
  #revision = 0;
  next() {
    this.#revision += 1;
    return this.#revision;
  }
  capture() {
    return this.#revision;
  }
  current(revision) {
    return revision === this.#revision;
  }
  invalidate() {
    this.next();
  }
}

export function pendingTurns(seenCount, total) {
  return Math.max(0, total - seenCount);
}

export function sameTurnSnapshot(previous, next) {
  return Boolean(
    previous &&
    previous.generation === next.generation &&
    previous.turn.index === next.turn.index &&
    previous.turn.revision === next.turn.revision,
  );
}

export function safeHref(href) {
  // Raw HTML, filesystem links and executable/custom schemes stay plain text.
  if (/[\u0000-\u0020\u007f]/u.test(href)) return null;
  try {
    const url = new URL(href);
    return ["https:", "http:", "mailto:"].includes(url.protocol)
      ? url.href
      : null;
  } catch {
    return null;
  }
}

export function inlineTokens(text) {
  const tokens = [];
  const pattern = /(`[^`\n]+`|\*\*[^*\n]+\*\*|\[[^\]\n]+\]\([^\s)]+\))/gu;
  let offset = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index > offset)
      tokens.push({ type: "text", text: text.slice(offset, match.index) });
    const value = match[0];
    if (value.startsWith("`"))
      tokens.push({ type: "code", text: value.slice(1, -1) });
    else if (value.startsWith("**"))
      tokens.push({ type: "strong", text: value.slice(2, -2) });
    else {
      const boundary = value.indexOf("](");
      const href = safeHref(value.slice(boundary + 2, -1));
      tokens.push(
        href
          ? { type: "link", text: value.slice(1, boundary), href }
          : { type: "text", text: value },
      );
    }
    offset = match.index + value.length;
  }
  if (offset < text.length)
    tokens.push({ type: "text", text: text.slice(offset) });
  return tokens;
}

export function markdownBlocks(text) {
  const blocks = [];
  let paragraph = [],
    list = null,
    fence = null,
    code = [];
  const flush = () => {
    if (paragraph.length)
      blocks.push({ type: "p", text: paragraph.join("\n") });
    paragraph = [];
    if (list) blocks.push(list);
    list = null;
  };
  for (const line of String(text).replace(/\r\n?/gu, "\n").split("\n")) {
    const marker = line.match(/^\s{0,3}(`{3,}|~{3,})(.*)$/u);
    if (fence) {
      if (
        marker &&
        marker[1][0] === fence[0] &&
        marker[1].length >= fence.length &&
        !marker[2].trim()
      ) {
        blocks.push({ type: "pre", text: code.join("\n") });
        fence = null;
        code = [];
      } else code.push(line);
      continue;
    }
    if (marker) {
      flush();
      fence = marker[1];
      continue;
    }
    if (!line.trim()) {
      flush();
      continue;
    }
    const heading = line.match(/^(#{1,6})\s+(.+)$/u);
    if (heading) {
      flush();
      blocks.push({
        type: `h${Math.min(6, heading[1].length + 1)}`,
        text: heading[2],
      });
      continue;
    }
    const bullet = line.match(/^\s{0,3}(?:([-+*])|\d+[.)])\s+(.+)$/u);
    if (bullet) {
      const type = bullet[1] ? "ul" : "ol";
      if (!list || list.type !== type) {
        flush();
        list = { type, items: [] };
      }
      list.items.push(bullet[2]);
      continue;
    }
    if (/^\s{0,3}>\s?/u.test(line)) {
      flush();
      blocks.push({
        type: "blockquote",
        text: line.replace(/^\s{0,3}>\s?/u, ""),
      });
      continue;
    }
    if (/^\s{0,3}(?:-{3,}|\*{3,}|_{3,})\s*$/u.test(line)) {
      flush();
      blocks.push({ type: "hr" });
      continue;
    }
    if (list) flush();
    paragraph.push(line);
  }
  flush();
  if (fence) blocks.push({ type: "pre", text: code.join("\n") });
  return blocks;
}
