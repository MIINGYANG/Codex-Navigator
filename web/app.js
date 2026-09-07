import {
  PAGE_SIZE,
  statusLabel,
  sessionTitle,
  filterSessions,
  pageFor,
  selectionAfterUpdate,
  navigation,
  RequestGate,
  inlineTokens,
  markdownBlocks,
  pendingTurns,
  sameTurnSnapshot,
  MAX_VISIBLE_ITEMS,
  activityWindow,
  navigationContextMatches,
  pausesFollow,
  chronologicalTarget,
} from "./state.mjs";
import { renderFlow } from "./flow.js";

const $ = (id) => document.getElementById(id);
const state = {
  info: null,
  sessions: [],
  sessionOffset: 0,
  sessionsLoading: false,
  lastScan: 0,
  key: null,
  summary: null,
  meta: null,
  selected: null,
  follow: true,
  turns: [],
  turnOffset: 0,
  turnTotal: 0,
  loadedTurnOffset: 0,
  loadedTurnQuery: "",
  loadedTurnOrder: "relevance",
  detail: null,
  items: [],
  itemStart: 0,
  nextItem: null,
  detailLoading: false,
  detailDirty: false,
  turnsDirty: false,
  activityLoading: false,
  connected: false,
  authFailed: false,
  seenCount: 0,
  presentation: "reading",
  flowInspected: "prompt",
  flowError: false,
};
const gates = Object.fromEntries(
  [
    "view",
    "sessions",
    "metadata",
    "turns",
    "detail",
    "items",
    "navigation",
  ].map((name) => [name, new RequestGate()]),
);
let token = "",
  flowSignature = "",
  pollTimer,
  searchTimer,
  toastTimer;

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function button(className, text, action, id) {
  const node = element("button", className, text);
  node.type = "button";
  if (id) node.id = id;
  node.addEventListener("click", action);
  return node;
}

function notify(message) {
  $("toast").textContent = message;
  $("toast").hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    $("toast").hidden = true;
  }, 3500);
}

function dateTime(value) {
  if (!value) return "时间未知";
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? "时间未知"
    : date.toLocaleString("zh-CN", {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      });
}

function projectName(path) {
  return path?.split(/[\\/]/u).filter(Boolean).at(-1) || "未记录项目";
}
function shortId(id) {
  return id ? id.slice(0, 12) : "ID 未知";
}
function size(bytes) {
  return bytes >= 1048576
    ? `${(bytes / 1048576).toFixed(1)} MiB`
    : `${Math.ceil(bytes / 1024)} KiB`;
}
function sessionPath(suffix = "") {
  return `/api/session/${encodeURIComponent(state.key)}${suffix}`;
}

function updateFlow() {
  if (state.presentation !== "flow" || !state.key) return;
  const pendingDirectory =
    state.loadedTurnOrder !== "chronological" ||
    state.loadedTurnOffset !== state.turnOffset ||
    state.loadedTurnQuery !== $("prompt-search").value;
  const data = {
    turns: pendingDirectory ? [] : state.turns,
    selected: state.selected,
    offset: state.turnOffset,
    total: state.turnTotal,
    pageSize: PAGE_SIZE,
    query: $("prompt-search").value,
    loading: state.turnsDirty && (pendingDirectory || !state.turns.length),
    detail: state.detail?.turn.index === state.selected ? state.detail : null,
    items: state.items,
    start: state.itemStart,
    next: state.nextItem,
    inspected: state.flowInspected,
    activityLoading: state.activityLoading,
    detailLoading: state.detailLoading,
    error: state.flowError,
  };
  const signature = JSON.stringify([
    state.key,
    state.meta?.generation,
    data.turns.map((t) => [t.index, t.revision]),
    data.selected,
    data.offset,
    data.total,
    data.query,
    data.loading,
    data.detail?.turn.revision,
    data.items.map((item) => item.index),
    data.start,
    data.next,
    data.inspected,
    data.activityLoading,
    data.error,
  ]);
  if (flowSignature === signature) return;
  flowSignature = signature;
  renderFlow($("flow-view"), data, {
    select: selectTurn,
    move: moveSelection,
    markdown,
    copy: copyText,
    page: async (direction) => {
      state.turnOffset = Math.max(0, state.turnOffset + direction * PAGE_SIZE);
      await loadTurns();
      $("question-path")?.scrollTo({ left: 0 });
    },
    locate: async () => {
      if (state.selected === null) return;
      $("prompt-search").value = "";
      gates.navigation.invalidate();
      clearTimeout(searchTimer);
      state.turnOffset = pageFor(state.selected);
      await loadTurns();
      focusTurn(state.selected, true);
    },
    more: loadMoreItems,
    inspect: (id) => {
      state.flowInspected = id;
      updateFlow();
      $("flow-inspector")?.focus({ preventScroll: true });
      $("flow-inspector")?.scrollIntoView({ block: "nearest" });
    },
    read: () => {
      const final = state.flowInspected === "final";
      setPresentation("reading");
      if (final) jumpFinal();
      else $("reading-area").focus({ preventScroll: true });
    },
  });
}

async function setPresentation(value) {
  state.presentation = value;
  gates.navigation.invalidate();
  $("article").hidden = value === "flow";
  $("flow-view").hidden = value !== "flow";
  $("view-reading").setAttribute("aria-pressed", String(value === "reading"));
  $("view-flow").setAttribute("aria-pressed", String(value === "flow"));
  if (value === "flow") {
    state.follow = false;
    renderMetadata();
    updateFlow();
    focusTurn(state.selected, true);
  } else $("reading-area").focus({ preventScroll: true });
  $("reading-area").scrollTop = 0;
  const key = state.key;
  if (key && state.meta) {
    await loadTurns();
    if (state.key === key && state.presentation === value && value === "flow")
      focusTurn(state.selected, true);
  }
}

function focusTurn(index, flow = false) {
  const node = flow
    ? $(`flow-question-${index}`)
    : [...$("turns").children].find(
        (item) => item.dataset.turn === String(index),
      );
  node?.focus({ preventScroll: true });
  node?.scrollIntoView({ block: "nearest", inline: "nearest" });
  if (flow && !node) $("question-path")?.focus({ preventScroll: true });
}
function validView(view, key) {
  return gates.view.current(view) && key === state.key;
}

function connected() {
  state.connected = true;
  $("error-banner").hidden = true;
  $("connection").textContent =
    state.info?.watch === false ? "手动刷新" : "本地已连接";
}

function failed(error) {
  state.connected = false;
  state.authFailed = error.status === 401 || error.status === 403;
  $("connection").textContent = state.authFailed ? "需要启动链接" : "连接中断";
  $("error-text").textContent = state.authFailed
    ? "访问凭据无效。请重新打开终端打印的完整启动链接。"
    : `暂时无法读取本地服务。请确认 codex-nav --web 仍在运行。${error.message ? `（${error.message}）` : ""}`;
  $("error-banner").hidden = false;
}

async function api(path, plain = false) {
  let response;
  try {
    response = await fetch(path, {
      headers: { "X-Codex-Nav-Token": token },
      cache: "no-store",
      credentials: "omit",
      signal: AbortSignal.timeout(15000),
    });
  } catch (error) {
    throw new Error(
      error.name === "TimeoutError" ? "读取超时，可重试" : "服务不可达",
    );
  }
  if (!response.ok) {
    const data = await response.json().catch(() => ({}));
    const error = new Error(data.error || `HTTP ${response.status}`);
    error.status = response.status;
    throw error;
  }
  return plain ? response.text() : response.json();
}

function readToken() {
  const fragment = new URLSearchParams(location.hash.slice(1));
  const incoming = fragment.get("token");
  if (incoming) {
    token = incoming;
    try {
      sessionStorage.setItem("codex-nav-token", token);
    } catch {
      /* Private mode can disable storage. */
    }
    history.replaceState(null, "", location.pathname + location.search);
  } else {
    try {
      token = sessionStorage.getItem("codex-nav-token") || "";
    } catch {
      token = "";
    }
  }
}

async function initialize() {
  state.authFailed = false;
  try {
    state.info = await api("/api/info");
    $("all-sessions").checked = state.info.default_all;
    $("watch-label").textContent = state.info.watch
      ? "实时读取 · 本地只读"
      : "自动更新已关闭 · r 手动刷新";
    connected();
    await loadSessions(false);
    if (state.info.initial_session && !state.key)
      await openSession(state.info.initial_session);
  } catch (error) {
    failed(error);
  }
  schedulePoll();
}

function renderSessions() {
  const filtered = filterSessions(state.sessions, $("session-search").value);
  if (state.sessionOffset >= filtered.length) state.sessionOffset = 0;
  const activeKey = document.activeElement?.dataset.session;
  const fragment = document.createDocumentFragment();
  for (const [offset, session] of filtered
    .slice(state.sessionOffset, state.sessionOffset + PAGE_SIZE)
    .entries()) {
    const row = button("session-row", "", () => openSession(session.key));
    row.dataset.session = session.key;
    row.title = session.title || session.first_prompt || session.id;
    row.append(
      element(
        "span",
        "session-number",
        String(state.sessionOffset + offset + 1).padStart(2, "0"),
      ),
    );
    const label = element("div");
    label.append(
      element("h2", "", sessionTitle(session)),
      element(
        "p",
        "",
        `${session.cwd || "未记录工作目录"} / ${shortId(session.id)}`,
      ),
    );
    const extra = element("div", "session-extra");
    extra.append(
      element("div", "", dateTime(session.updated_at)),
      element(
        "span",
        "",
        session.turn_count == null
          ? "主会话　↗"
          : `主会话 · ${session.turn_count} 轮　↗`,
      ),
    );
    row.append(label, extra);
    fragment.append(row);
  }
  if (!filtered.length)
    fragment.append(
      element(
        "p",
        "empty",
        state.sessionsLoading
          ? "正在读取会话目录…"
          : state.sessions.length
            ? "没有匹配的主会话。试试项目名或 ID，或清空搜索。"
            : "还没有找到主会话。可以包含更早会话，或在 Codex 中开始一段会话后刷新。",
      ),
    );
  $("sessions").replaceChildren(fragment);
  $("sessions").setAttribute("aria-busy", String(state.sessionsLoading));
  $("session-count").textContent =
    `${$("all-sessions").checked ? "全部" : "最近"}主会话 · ${filtered.length}${state.sessionsLoading ? " · 读取中" : ""}`;
  $("session-pages").hidden = filtered.length <= PAGE_SIZE;
  $("sessions-prev").disabled = state.sessionOffset === 0;
  $("sessions-next").disabled =
    state.sessionOffset + PAGE_SIZE >= filtered.length;
  $("sessions-page-label").textContent =
    `${state.sessionOffset + 1}–${Math.min(filtered.length, state.sessionOffset + PAGE_SIZE)} / ${filtered.length}`;
  if (activeKey)
    [...$("sessions").children]
      .find((node) => node.dataset.session === activeKey)
      ?.focus({ preventScroll: true });
}

async function loadSessions(refresh = false) {
  const request = gates.sessions.next();
  if (refresh) state.lastScan = Date.now();
  try {
    const result = await api(
      `/api/sessions?all=${$("all-sessions").checked ? 1 : 0}${refresh ? "&refresh=1" : ""}`,
    );
    if (!gates.sessions.current(request)) return;
    state.sessions = result.sessions;
    state.sessionsLoading = result.loading;
    connected();
    renderSessions();
    if (result.error) {
      $("sessions").prepend(
        element(
          "p",
          "omission-note",
          `会话目录读取提示：${result.error}。可按 r 重试。`,
        ),
      );
    }
  } catch (error) {
    if (gates.sessions.current(request)) failed(error);
  }
}

async function openSession(key) {
  flowSignature = "";
  gates.view.next();
  for (const name of ["metadata", "turns", "detail", "items"])
    gates[name].invalidate();
  state.key = key;
  state.summary = state.sessions.find((session) => session.key === key) || {
    id: key,
  };
  state.meta = null;
  state.selected = null;
  state.detail = null;
  state.turns = [];
  state.items = [];
  state.itemStart = 0;
  state.follow = true;
  state.turnOffset = 0;
  state.turnTotal = 0;
  state.loadedTurnOffset = 0;
  state.loadedTurnQuery = "";
  state.loadedTurnOrder = "relevance";
  state.detailDirty = true;
  state.turnsDirty = true;
  state.seenCount = 0;
  state.flowInspected = "prompt";
  state.flowError = false;
  if (state.presentation === "flow") state.follow = false;
  $("flow-view").replaceChildren(element("p", "empty", "正在读取问题脉络…"));
  $("prompt-search").value = "";
  $("picker").hidden = true;
  $("reader").hidden = false;
  $("session-title").textContent = sessionTitle(state.summary);
  $("project").textContent = state.summary.cwd || "正在读取会话";
  $("session-detail").textContent = state.summary.id;
  $("article").replaceChildren(element("p", "empty", "正在读取会话内容…"));
  $("breadcrumb").textContent = "正在打开";
  $("reading-area").scrollTop = 0;
  $("turns").replaceChildren();
  $("turn-pages").hidden = true;
  $("copy-turn").disabled = true;
  $("jump-final").disabled = true;
  $("latest").disabled = true;
  $("reading-area").focus({ preventScroll: true });
  await refreshMetadata();
}

function showPicker() {
  gates.view.invalidate();
  state.key = null;
  state.meta = null;
  clearTimeout(searchTimer);
  $("reader").hidden = true;
  $("picker").hidden = false;
  renderSessions();
  $("session-search").focus();
  loadSessions(false);
}

function renderMetadata() {
  const meta = state.meta;
  if (!meta) return;
  $("project").textContent =
    meta.meta.cwd || state.summary.cwd || "未记录工作目录";
  const kind =
    {
      MAIN: "主会话",
      SUBAGENT: "子代理会话（显式打开）",
      UNKNOWN: "来源未知（显式打开）",
    }[meta.meta.kind] || "会话";
  $("session-detail").textContent =
    `${kind} · ${meta.meta.id || state.summary.id}`;
  $("session-detail").title = meta.meta.id || "";
  const notes = [];
  if (meta.loading)
    notes.push(`正在读取 ${size(meta.offset)} / ${size(meta.total_bytes)}`);
  if (meta.error) notes.push(`读取提示：${meta.error} · 可按 r 重试`);
  if (meta.stats?.malformed_records)
    notes.push(`${meta.stats.malformed_records} 条格式异常记录已跳过`);
  if (meta.stats?.skipped_oversize_records)
    notes.push(`${meta.stats.skipped_oversize_records} 条超大记录已跳过`);
  if (meta.stats?.omitted_text_bytes)
    notes.push("部分历史文本因内存预算省略，当前轮会单独标注");
  $("load-status").textContent = notes.join(" · ");
  const latest = meta.latest_active ?? meta.turn_count - 1;
  const pending = pendingTurns(state.seenCount, meta.turn_count);
  $("latest").disabled = !meta.turn_count;
  $("latest").textContent = pending
    ? `${pending} 条新问题 · 回到最新 ↓`
    : state.selected != null && state.selected < latest
      ? `距最新 ${latest - state.selected} 轮 · 回到最新 ↓`
      : state.follow
        ? "正在跟随最新问题 ↓"
        : "回到最新问题 ↓";
}

async function refreshMetadata(refresh = false) {
  if (!state.key) return;
  const request = gates.metadata.next(),
    view = gates.view.capture(),
    key = state.key;
  try {
    const next = await api(sessionPath(refresh ? "?refresh=1" : ""));
    if (!gates.metadata.current(request) || !validView(view, key)) return;
    connected();
    const previous = state.meta;
    const selected = selectionAfterUpdate(
      state.selected,
      previous,
      next,
      state.follow,
      Boolean($("prompt-search").value.trim()),
    );
    const reset = previous && previous.generation !== next.generation;
    const changed = !previous || previous.revision !== next.revision || reset;
    const selectionChanged = selected !== state.selected;
    state.meta = next;
    state.selected = selected;
    if (
      !previous ||
      reset ||
      (state.follow && selected === (next.latest_active ?? next.turn_count - 1))
    )
      state.seenCount = next.turn_count;
    if (reset) {
      state.detail = null;
      state.items = [];
      state.flowInspected = "prompt";
      notify("会话文件已重新加载");
    }
    renderMetadata();
    if (
      changed ||
      selectionChanged ||
      refresh ||
      state.detailDirty ||
      state.turnsDirty ||
      !state.detail ||
      state.detail.turn.index !== selected
    ) {
      if (
        selectionChanged &&
        !$("prompt-search").value.trim() &&
        selected !== null
      )
        state.turnOffset = pageFor(selected);
      await Promise.all([
        loadTurns(),
        selected === null
          ? Promise.resolve()
          : loadDetail(!selectionChanged && !reset),
      ]);
    }
    if (!gates.metadata.current(request) || !validView(view, key)) return;
    if (selected === null) {
      $("article").replaceChildren(
        element(
          "p",
          "empty",
          next.loading
            ? "正在解析问题轮次…"
            : "这段会话还没有可显示的问题。保持页面打开，或按 r 重新读取。",
        ),
      );
      $("copy-turn").disabled = true;
      $("jump-final").disabled = true;
      updateFlow();
    }
  } catch (error) {
    if (gates.metadata.current(request) && validView(view, key)) failed(error);
  }
}

function renderTurns() {
  const activeIndex = document.activeElement?.dataset.turn;
  const fragment = document.createDocumentFragment();
  for (const turn of state.turns) {
    const node = button("turn-btn", "", () => selectTurn(turn.index));
    node.dataset.turn = turn.index;
    node.setAttribute("aria-current", String(turn.index === state.selected));
    node.title = turn.preview;
    node.append(
      element("span", "turn-n", String(turn.ordinal).padStart(2, "0")),
    );
    const name = element("span", "turn-name");
    name.append(
      element("span", "turn-preview", turn.preview || "（未记录文字 Prompt）"),
    );
    const info = element("span", "turn-info", statusLabel(turn.status));
    if (turn.errors)
      info.append(element("span", "warning", ` · !${turn.errors}`));
    if (turn.has_final) info.append(document.createTextNode(" · 回复"));
    name.append(info);
    node.append(name);
    fragment.append(node);
  }
  if (!state.turns.length)
    fragment.append(
      element(
        "p",
        "empty",
        $("prompt-search").value
          ? "没有匹配的 Prompt。清空搜索后查看全部轮次。"
          : "暂时没有问题轮次。",
      ),
    );
  $("turns").replaceChildren(fragment);
  $("turn-count").textContent =
    `${state.turnTotal} 轮${$("prompt-search").value ? "匹配" : ""}`;
  $("turn-pages").hidden = state.turnTotal <= PAGE_SIZE;
  $("turns-prev").disabled = state.turnOffset === 0;
  $("turns-next").disabled = state.turnOffset + PAGE_SIZE >= state.turnTotal;
  $("turn-page-label").textContent =
    `${state.turnOffset + 1}–${Math.min(state.turnTotal, state.turnOffset + PAGE_SIZE)} / ${state.turnTotal}`;
  if (activeIndex !== undefined)
    [...$("turns").children]
      .find((node) => node.dataset.turn === activeIndex)
      ?.focus({ preventScroll: true });
  updateFlow();
}

async function loadTurns() {
  if (!state.key) return;
  const request = gates.turns.next(),
    key = state.key;
  state.turnsDirty = true;
  updateFlow();
  const query = new URLSearchParams({
    q: $("prompt-search").value,
    offset: state.turnOffset,
    limit: PAGE_SIZE,
    order: state.presentation === "flow" ? "chronological" : "relevance",
  });
  try {
    const result = await api(sessionPath(`/turns?${query}`));
    if (!gates.turns.current(request) || key !== state.key) return;
    if (result.generation !== state.meta?.generation) return;
    state.turns = result.turns;
    state.turnTotal = result.total;
    state.turnOffset = result.offset;
    state.loadedTurnOffset = result.offset;
    state.loadedTurnQuery = query.get("q");
    state.loadedTurnOrder = query.get("order");
    state.turnsDirty = false;
    renderTurns();
  } catch (error) {
    if (gates.turns.current(request) && key === state.key) failed(error);
  }
}

async function selectTurn(index, fromNavigation = false) {
  if (!fromNavigation) gates.navigation.invalidate();
  const changed = state.selected !== index;
  if (changed) state.flowInspected = "prompt";
  state.selected = index;
  state.follow =
    index === (state.meta?.latest_active ?? state.meta?.turn_count - 1) &&
    !$("prompt-search").value &&
    state.presentation !== "flow";
  if (state.follow) state.seenCount = state.meta?.turn_count || 0;
  renderTurns();
  renderMetadata();
  if (changed || !state.detail) await loadDetail(false);
}

function appendInline(node, text) {
  for (const token of inlineTokens(text)) {
    if (token.type === "text") node.append(document.createTextNode(token.text));
    else if (token.type === "link") {
      const link = element("a", "", token.text);
      link.href = token.href;
      link.target = "_blank";
      link.rel = "noopener noreferrer";
      node.append(link);
    } else node.append(element(token.type, "", token.text));
  }
}

function markdown(text) {
  const container = element("div", "markdown");
  for (const block of markdownBlocks(text)) {
    const node = element(block.type);
    if (block.type === "pre") node.append(element("code", "", block.text));
    else if (block.items) {
      for (const value of block.items) {
        const li = element("li");
        appendInline(li, value);
        node.append(li);
      }
    } else if (block.text) appendInline(node, block.text);
    container.append(node);
  }
  return container;
}

function renderArticle(preserve) {
  const detail = state.detail,
    turn = detail.turn,
    area = $("reading-area");
  const oldScroll = area.scrollTop;
  const oldOpen = Boolean($("activity")?.open);
  const focusId =
    preserve && $("article").contains(document.activeElement)
      ? document.activeElement.id
      : null;
  const article = document.createDocumentFragment();
  const meta = element("div", "article-meta");
  meta.append(
    element("span", "", `TURN ${String(turn.ordinal).padStart(2, "0")}`),
    element("span", "", dateTime(turn.started_at)),
    element("span", "status", statusLabel(turn.status)),
  );
  if (turn.activity.errors)
    meta.append(
      element("span", "warning", `!${turn.activity.errors} 过程告警`),
    );
  article.append(
    meta,
    element("h1", "", turn.prompt.preview || "这一轮的问题"),
  );
  const promptHeading = element("div", "section-heading");
  promptHeading.append(
    element("p", "section-label", "YOUR PROMPT"),
    button(
      "copy",
      "复制问题",
      () => copyText(turn.prompt.text || turn.prompt.preview, "问题"),
      "copy-prompt",
    ),
  );
  article.append(
    promptHeading,
    element(
      "div",
      "prompt-box",
      turn.prompt.text || turn.prompt.preview || "（仅有图片或未记录文字）",
    ),
  );
  if (turn.prompt.images_count)
    article.append(
      element(
        "div",
        "omission-note",
        `此问题包含 ${turn.prompt.images_count} 张图片。只读阅读器不加载图片附件。`,
      ),
    );
  if (turn.prompt.omitted_bytes)
    article.append(
      element(
        "div",
        "omission-note",
        `此问题有 ${size(turn.prompt.omitted_bytes)} 原文因内存预算省略。${turn.prompt.text ? "以上为已保留内容。" : "以上仅为目录摘要。"}`,
      ),
    );
  const activity = element("details", "activity");
  activity.id = "activity";
  activity.open = preserve ? oldOpen : !detail.final_answer;
  const summary = element("summary");
  summary.id = "activity-summary";
  summary.append(
    element(
      "span",
      "activity-title",
      `执行过程 · ${detail.items_total} 条记录`,
    ),
  );
  if (turn.activity.errors)
    summary.append(
      element("span", "warning", `!${turn.activity.errors} 活动错误`),
    );
  activity.append(summary);
  const items = element("div", "activity-items");
  items.id = "activity-items";
  activity.append(items);
  article.append(activity);
  const final = element("section", "final");
  final.id = "final-answer";
  final.tabIndex = -1;
  final.setAttribute("aria-label", "最终回复");
  const header = element("div", "final-heading");
  const label = element("span", "final-label");
  label.append(
    element("span", "mini-mark"),
    document.createTextNode("FINAL ANSWER"),
  );
  header.append(label);
  if (detail.final_answer) {
    header.append(
      button(
        "copy",
        "复制回复",
        () => copyText(detail.final_answer.text, "最终回复"),
        "copy-final",
      ),
    );
    final.append(header, markdown(detail.final_answer.text));
  } else {
    final.append(
      header,
      element(
        "p",
        "empty",
        "尚无明确标记的最终回复。可以展开执行过程阅读已记录的消息；不会把普通消息猜成最终答案。",
      ),
    );
  }
  final.append(
    element(
      "div",
      "result-note",
      `${statusLabel(turn.status)}${turn.activity.errors ? ` · 过程中有 ${turn.activity.errors} 次活动错误` : ""}。这只是执行记录，不判断答案正确与否。`,
    ),
    element(
      "div",
      "end-mark",
      detail.final_answer ? "读到这里，结果由你判断" : "等待明确的最终回复标记",
    ),
  );
  article.append(final);
  $("article").replaceChildren(article);
  renderItems();
  $("breadcrumb").textContent =
    `${projectName(state.meta?.meta.cwd || state.summary.cwd)} / 问题 ${String(turn.ordinal).padStart(2, "0")}`;
  $("copy-turn").disabled = false;
  $("jump-final").disabled = false;
  area.scrollTop = preserve ? oldScroll : 0;
  if (focusId) $(focusId)?.focus({ preventScroll: true });
}

function renderItems() {
  const container = $("activity-items");
  if (!container || !state.detail) return;
  const fragment = document.createDocumentFragment();
  if (state.itemStart > 0) {
    const previous = button(
      "more-button",
      "← 上一组活动",
      () => loadMoreItems(Math.max(0, state.itemStart - MAX_VISIBLE_ITEMS)),
      "activity-prev",
    );
    previous.disabled = state.activityLoading;
    fragment.append(previous);
  }
  for (const item of state.items) {
    if (item.index === state.detail.final_answer?.index) continue;
    const node = element("div", "activity-item");
    node.dataset.item = item.index;
    const names = {
      agent_message: "ASSISTANT",
      tool_call: "TOOL CALL",
      tool_output: "TOOL OUTPUT",
      file_activity: "FILE",
      notice: "NOTICE",
      omitted: "OMITTED",
    };
    const label = element(
      "div",
      item.is_error ? "item-label warning" : "item-label",
      `${names[item.type] || "RECORD"}${item.name ? ` · ${item.name}` : ""}${item.is_error ? " · 活动错误" : ""}`,
    );
    node.append(label);
    if (item.type === "agent_message") node.append(markdown(item.text || ""));
    else if (item.type === "omitted")
      node.append(
        element("div", "omission-note", "部分历史活动正文因内存预算省略。"),
      );
    else
      node.append(
        element(
          "pre",
          "",
          item.text ||
            item.summary ||
            [item.kind, item.path].filter(Boolean).join(" · ") ||
            "（无文字内容）",
        ),
      );
    fragment.append(node);
  }
  if (!fragment.children.length)
    fragment.append(
      element(
        "p",
        "empty",
        state.detail.items_total
          ? "当前已加载记录中的最终回复已单独展示在下方。"
          : "没有记录工具活动。",
      ),
    );
  if (state.nextItem !== null) {
    const more = button(
      "more-button",
      state.activityLoading
        ? "正在读取…"
        : `${state.items.length >= MAX_VISIBLE_ITEMS ? "下一组活动" : "继续加载活动"}（${state.itemStart + 1}–${state.itemStart + state.items.length} / ${state.detail.items_total}）`,
      () => loadMoreItems(),
      "activity-more",
    );
    more.disabled = state.activityLoading;
    fragment.append(more);
  }
  container.replaceChildren(fragment);
  updateFlow();
}

async function loadDetail(preserve) {
  if (!state.key || state.selected === null) return;
  const request = gates.detail.next(),
    key = state.key,
    index = state.selected;
  state.detailLoading = true;
  state.detailDirty = true;
  state.flowError = false;
  if (!preserve) {
    gates.items.invalidate();
    state.activityLoading = false;
    $("article").replaceChildren(element("p", "empty", "正在读取这一轮…"));
    $("copy-turn").disabled = true;
    $("jump-final").disabled = true;
    state.detail = null;
    updateFlow();
  }
  try {
    const itemStart = preserve ? state.itemStart : 0;
    const detail = await api(
      sessionPath(`/turn/${index}?offset=${itemStart}&limit=8`),
    );
    if (
      !gates.detail.current(request) ||
      key !== state.key ||
      index !== state.selected
    )
      return;
    if (detail.generation !== state.meta?.generation) return;
    if (preserve && sameTurnSnapshot(state.detail, detail)) {
      state.detailDirty = false;
      return;
    }
    gates.items.invalidate();
    state.activityLoading = false;
    // Refresh only the bounded visible activity window, and commit a consistent turn snapshot.
    const visibleCount = preserve
      ? Math.min(state.items.length, MAX_VISIBLE_ITEMS)
      : 8;
    const offsets = [];
    for (
      let offset = itemStart + 8;
      offset < Math.min(detail.items_total, itemStart + visibleCount);
      offset += 8
    )
      offsets.push(offset);
    const pages = [];
    for (const offset of offsets) {
      if (
        !gates.detail.current(request) ||
        key !== state.key ||
        index !== state.selected
      )
        return;
      pages.push(
        await api(sessionPath(`/turn/${index}?offset=${offset}&limit=8`)),
      );
    }
    if (
      !gates.detail.current(request) ||
      key !== state.key ||
      index !== state.selected
    )
      return;
    if (pages.some((page) => !sameTurnSnapshot(detail, page))) return;
    state.detail = detail;
    state.detailDirty = false;
    state.itemStart = itemStart;
    state.items = [detail, ...pages].flatMap((page) => page.items);
    state.nextItem =
      pages.at(-1)?.next_offset ?? (pages.length ? null : detail.next_offset);
    renderArticle(preserve);
  } catch (error) {
    if (
      gates.detail.current(request) &&
      key === state.key &&
      index === state.selected
    ) {
      state.flowError = true;
      if (!preserve)
        $("article").replaceChildren(
          element(
            "p",
            "empty",
            "这一轮暂时无法读取。按 r 重试；如果会话刚被重载，可重新选择轮次。",
          ),
        );
      failed(error);
      updateFlow();
    }
  } finally {
    if (gates.detail.current(request)) {
      state.detailLoading = false;
      updateFlow();
    }
  }
}

async function loadMoreItems(requestedOffset = state.nextItem) {
  if (
    requestedOffset === null ||
    state.activityLoading ||
    !state.detail ||
    state.detailLoading
  )
    return;
  const request = gates.items.next(),
    key = state.key,
    index = state.selected,
    offset = requestedOffset;
  const generation = state.detail.generation,
    revision = state.detail.turn.revision;
  state.activityLoading = true;
  renderItems();
  try {
    const result = await api(
      sessionPath(`/turn/${index}?offset=${offset}&limit=8`),
    );
    if (
      !gates.items.current(request) ||
      key !== state.key ||
      index !== state.selected
    )
      return;
    if (result.generation !== generation || result.turn.revision !== revision) {
      await refreshMetadata();
      return;
    }
    const window = activityWindow(state.itemStart, state.items.length, offset);
    state.itemStart = window.start;
    state.items = window.replace
      ? result.items
      : [...state.items, ...result.items];
    state.nextItem = result.next_offset;
    if (window.replace)
      (state.presentation === "flow"
        ? $("process-path")
        : $("activity-summary")
      )?.scrollIntoView({ block: "start" });
  } catch (error) {
    if (gates.items.current(request) && key === state.key) failed(error);
  } finally {
    if (gates.items.current(request)) {
      state.activityLoading = false;
      renderItems();
      (state.presentation === "flow"
        ? $("flow-after") || $("flow-before")
        : $("activity-more")
      )?.focus({ preventScroll: true });
    }
  }
}

async function copyText(text, name) {
  try {
    if (!navigator.clipboard) throw new Error("Clipboard unavailable");
    await navigator.clipboard.writeText(text);
    notify(`已复制${name}`);
  } catch {
    $("copy-text").value = text;
    if (!$("copy-fallback").open) $("copy-fallback").showModal();
    $("copy-text").focus();
    $("copy-text").select();
  }
}

async function copyTurn() {
  if (!state.key || state.selected === null || state.detailLoading) return;
  const key = state.key,
    index = state.selected;
  $("copy-turn").disabled = true;
  try {
    const text = await api(sessionPath(`/turn/${index}/text`), true);
    if (key === state.key && index === state.selected)
      await copyText(text, "整轮已保留内容");
  } catch (error) {
    if (key === state.key) failed(error);
  } finally {
    $("copy-turn").disabled =
      !state.key || state.selected === null || state.detailLoading;
  }
}

function jumpFinal() {
  if (state.presentation === "flow" && !$("reader").hidden) {
    state.flowInspected = "final";
    updateFlow();
    $("flow-inspector")?.focus({ preventScroll: true });
    $("flow-inspector")?.scrollIntoView({ block: "nearest" });
    return;
  }
  const final = $("final-answer");
  if (!final || $("reader").hidden) return;
  final.scrollIntoView({ block: "start", behavior: "instant" });
  final.focus({ preventScroll: true });
  if (!state.detail?.final_answer) notify("当前轮尚无明确标记的最终回复");
}

async function jumpLatest() {
  if (!state.meta?.turn_count) return;
  $("prompt-search").value = "";
  const index = state.meta.latest_active ?? state.meta.turn_count - 1;
  state.turnOffset = pageFor(index);
  state.follow = true;
  await Promise.all([loadTurns(), selectTurn(index)]);
  $("reading-area").focus({ preventScroll: true });
}

async function refresh() {
  if (!state.info) return initialize();
  state.authFailed = false;
  if (state.key) await refreshMetadata(true);
  else await loadSessions(true);
}

function schedulePoll() {
  clearTimeout(pollTimer);
  pollTimer = setTimeout(
    async () => {
      try {
        if (!state.authFailed && state.info && !document.hidden) {
          if (state.key) {
            // Metadata polling only drains the worker cache; --no-watch never requests a file refresh here.
            await refreshMetadata();
          } else if (
            state.sessionsLoading ||
            state.info.watch ||
            !state.connected
          ) {
            const rescan =
              state.info.watch &&
              !state.sessionsLoading &&
              Date.now() - state.lastScan > 5000;
            await loadSessions(rescan);
          }
        }
      } finally {
        schedulePoll();
      }
    },
    document.hidden ? 3000 : state.info?.refresh_ms || 750,
  );
}

async function moveSelection(direction) {
  const flowFocus = Boolean(document.activeElement?.closest("#question-path"));
  if (
    flowFocus &&
    (state.loadedTurnOrder !== "chronological" ||
      state.loadedTurnOffset !== state.turnOffset ||
      state.loadedTurnQuery !== $("prompt-search").value)
  )
    return;
  const focusedTurn = document.activeElement?.dataset.flowTurn;
  const anchor =
    flowFocus && focusedTurn !== undefined
      ? Number(focusedTurn)
      : state.selected;
  const request = gates.navigation.next();
  const context = {
    view: gates.view.capture(),
    key: state.key,
    query: $("prompt-search").value,
  };
  const current = () =>
    gates.navigation.current(request) &&
    navigationContextMatches(context, {
      view: gates.view.capture(),
      key: state.key,
      query: $("prompt-search").value,
    });
  if ($("reader").hidden) {
    const filtered = filterSessions(state.sessions, $("session-search").value);
    if (!filtered.length) return;
    let index = filtered.findIndex(
      (session) => session.key === document.activeElement?.dataset.session,
    );
    index =
      direction === "start"
        ? 0
        : direction === "end"
          ? filtered.length - 1
          : Math.max(
              0,
              Math.min(
                filtered.length - 1,
                index + (direction === "next" ? 1 : -1),
              ),
            );
    state.sessionOffset = pageFor(index);
    renderSessions();
    [...$("sessions").children]
      .find((node) => node.dataset.session === filtered[index].key)
      ?.focus();
    return;
  }
  if (!context.query.trim()) {
    const target = chronologicalTarget(direction, anchor, state.meta);
    if (target === null) return;
    state.turnOffset = pageFor(target);
    await Promise.all([loadTurns(), selectTurn(target, true)]);
    if (!current()) return;
    focusTurn(target, flowFocus);
    return;
  }
  if (!state.turnTotal) return;
  let position = state.turns.findIndex((turn) => turn.index === anchor);
  let absolute = state.turnOffset + Math.max(0, position);
  absolute =
    direction === "start"
      ? 0
      : direction === "end"
        ? state.turnTotal - 1
        : Math.max(
            0,
            Math.min(
              state.turnTotal - 1,
              absolute + (position < 0 ? 0 : direction === "next" ? 1 : -1),
            ),
          );
  if (pageFor(absolute) !== state.turnOffset) {
    state.turnOffset = pageFor(absolute);
    await loadTurns();
  }
  if (!current()) return;
  const turn = state.turns[absolute - state.turnOffset];
  if (!turn) return;
  await selectTurn(turn.index, true);
  if (!current()) return;
  focusTurn(turn.index, flowFocus);
}

function keyboard(event) {
  if (
    event.defaultPrevented ||
    event.ctrlKey ||
    event.metaKey ||
    event.altKey ||
    event.isComposing
  )
    return;
  if ($("help").open || $("copy-fallback").open) return;
  const active = document.activeElement;
  const typing = active?.matches(
    "input, textarea, select, [contenteditable=true]",
  );
  if (event.key === "Escape") {
    if (typing && active.matches("input[type=search]")) {
      event.preventDefault();
      if (active.value) {
        active.value = "";
        active.dispatchEvent(new Event("input"));
      } else {
        active.blur();
        ($("reader").hidden
          ? $("sessions").querySelector("button")
          : $("turns")
        )?.focus();
      }
    }
    return;
  }
  if (typing) return;
  if (event.key === "v" && !$("reader").hidden) {
    event.preventDefault();
    setPresentation(state.presentation === "flow" ? "reading" : "flow");
    return;
  }
  if ($("reading-area").contains(active) && pausesFollow(event.key)) {
    state.follow = false;
    renderMetadata();
    return;
  }
  if (event.key === "?") {
    event.preventDefault();
    $("help").showModal();
    return;
  }
  if (event.key === "/") {
    event.preventDefault();
    ($("reader").hidden ? $("session-search") : $("prompt-search")).focus();
    return;
  }
  if (event.key === "s") {
    event.preventDefault();
    showPicker();
    return;
  }
  if (event.key === "r") {
    event.preventDefault();
    refresh();
    return;
  }
  if (event.key === "f" && !$("reader").hidden) {
    event.preventDefault();
    jumpFinal();
    return;
  }
  if (event.key === "c" && !$("reader").hidden && !state.detailLoading) {
    event.preventDefault();
    $("copy-prompt")?.click();
    return;
  }
  if (event.key === "C" && !$("reader").hidden) {
    event.preventDefault();
    copyTurn();
    return;
  }
  if ((event.key === "[" || event.key === "]") && !$("reader").hidden) {
    event.preventDefault();
    moveSelection(event.key === "]" ? "next" : "previous");
    return;
  }
  const focus = $("reader").hidden
    ? "picker"
    : $("question-path")?.contains(active)
      ? "directory"
      : $("reading-area").contains(active)
        ? "reader"
        : "directory";
  const action = navigation(event.key, focus);
  if (!action) return;
  event.preventDefault();
  if (action.kind === "select") {
    moveSelection(action.direction);
    return;
  }
  state.follow = false;
  renderMetadata();
  const area =
    active?.closest(".flow-inspector, .process-path") || $("reading-area");
  area.scrollTo({
    top:
      action.direction === "start"
        ? 0
        : action.direction === "end"
          ? area.scrollHeight
          : area.scrollTop + (action.direction === "down" ? 88 : -88),
    behavior: "instant",
  });
}

$("session-search").addEventListener("input", () => {
  state.sessionOffset = 0;
  renderSessions();
});
$("all-sessions").addEventListener("change", () => {
  state.sessionOffset = 0;
  state.sessions = [];
  state.sessionsLoading = true;
  renderSessions();
  loadSessions(true);
});
$("prompt-search").addEventListener("input", () => {
  state.follow = false;
  state.turnOffset = 0;
  gates.turns.invalidate();
  gates.navigation.invalidate();
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => loadTurns(), 120);
  renderMetadata();
});
for (const [id, change] of [
  ["sessions-prev", -PAGE_SIZE],
  ["sessions-next", PAGE_SIZE],
]) {
  $(id).addEventListener("click", () => {
    state.sessionOffset = Math.max(0, state.sessionOffset + change);
    renderSessions();
  });
}
for (const [id, change] of [
  ["turns-prev", -PAGE_SIZE],
  ["turns-next", PAGE_SIZE],
]) {
  $(id).addEventListener("click", () => {
    state.turnOffset = Math.max(0, state.turnOffset + change);
    loadTurns();
  });
}
$("back").addEventListener("click", showPicker);
$("view-reading").addEventListener("click", () => setPresentation("reading"));
$("view-flow").addEventListener("click", () => setPresentation("flow"));
$("brand-home").addEventListener("click", showPicker);
$("jump-final").addEventListener("click", jumpFinal);
$("latest").addEventListener("click", jumpLatest);
$("copy-turn").addEventListener("click", copyTurn);
$("refresh").addEventListener("click", refresh);
$("retry").addEventListener("click", refresh);
$("help-button").addEventListener("click", () => $("help").showModal());
$("close-help").addEventListener("click", () => $("help").close());
$("close-copy").addEventListener("click", () => $("copy-fallback").close());
$("copy-fallback").addEventListener("close", () => {
  $("copy-text").value = "";
});
$("reading-area").addEventListener(
  "wheel",
  () => {
    state.follow = false;
    renderMetadata();
  },
  { passive: true },
);
$("reading-area").addEventListener(
  "touchstart",
  () => {
    state.follow = false;
    renderMetadata();
  },
  { passive: true },
);
document.addEventListener("keydown", keyboard);
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) {
    schedulePoll();
    if (state.info?.watch) refresh();
  }
});
readToken();
initialize();
