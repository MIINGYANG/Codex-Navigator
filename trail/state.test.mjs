import assert from "node:assert/strict";
import test from "node:test";
import {
  canvasPreferences,
  filterSessions,
  filterFavoriteQuestions,
  resolveFavoriteQuestion,
  titleOf,
  relativeTime,
  fullTime,
  groupResults,
  reconcileSelection,
  reconcileEventView,
  initialLoadTransition,
  reconcileSessions,
} from "./state.ts";

test("初次多批读取不报新增，后续大批实时追加不会吞掉新增提示", () => {
  assert.deepEqual(initialLoadTransition(true, undefined, 1, true), {
    resetSeen: true,
    initializing: true,
  });
  assert.deepEqual(initialLoadTransition(true, 1, 1, false), {
    resetSeen: true,
    initializing: false,
  });
  assert.deepEqual(initialLoadTransition(false, 1, 1, true), {
    resetSeen: false,
    initializing: false,
  });
  assert.deepEqual(initialLoadTransition(false, 1, 1, false), {
    resetSeen: false,
    initializing: false,
  });
  assert.deepEqual(initialLoadTransition(false, 1, 2, true), {
    resetSeen: true,
    initializing: true,
  });
});

test("会话优先显示 Codex 名称，缺失时回退原始问题和 ID", () => {
  assert.equal(
    titleOf({
      first_prompt: " \n 原文   中的词？\n下一行",
      title: "已保存标题",
    }),
    "已保存标题",
  );
  assert.equal(
    titleOf({ first_prompt: " ", title: "已保存标题" }),
    "已保存标题",
  );
  assert.equal(titleOf({ id: "session-id" }), "session-id");
  assert.equal(
    titleOf({ title: "  ", first_prompt: " \n 原文   中的词？\n下一行" }),
    "原文 中的词？",
  );
  assert.equal(titleOf(), "未命名会话");
});
test("异步旧扫描不能覆盖已改名称或恢复已删除会话", () => {
  const snapshot = [
    { key: "a", title: "旧名称" },
    { key: "b", title: "待删除" },
    { key: "c", title: "另一会话" },
  ];
  assert.deepEqual(
    reconcileSessions(snapshot, new Map([["a", "新名称"]]), new Set(["b"])),
    [
      { key: "a", title: "新名称" },
      { key: "c", title: "另一会话" },
    ],
  );
  assert.equal(snapshot[0].title, "旧名称");
  assert.deepEqual(
    reconcileSessions(snapshot, new Map(), new Set(["a", "b", "c"])),
    [],
  );
});
test("日期缺失和无效时不伪造时间", () => {
  assert.equal(relativeTime(null), "时间未记录");
  assert.equal(relativeTime("invalid"), "时间未记录");
  assert.equal(fullTime("invalid"), "时间未记录");
  const now = Date.parse("2026-09-08T10:00:00Z");
  assert.equal(relativeTime("2026-09-08T09:58:00Z", now), "2 分钟前");
  assert.equal(relativeTime("2026-09-08T11:00:00Z", now), "刚刚更新");
});
test("跨会话搜索分组保留稳定顺序，不合并同名会话", () => {
  const rows = [
    { sessionKey: "a", sessionTitle: "相同标题", nodeId: "q1" },
    { sessionKey: "b", sessionTitle: "相同标题", nodeId: "q1" },
    { sessionKey: "a", sessionTitle: "相同标题", nodeId: "q2" },
  ];
  const grouped = groupResults(rows);
  assert.deepEqual(
    grouped.map(([key]) => key),
    ["a", "b"],
  );
  assert.deepEqual(
    grouped[0][1].results.map((row) => row.nodeId),
    ["q1", "q2"],
  );
});
test("实时追加保留历史选择，文件代际变化或节点消失清除选择", () => {
  const ids = new Set(["q1", "q2", "q3"]);
  assert.equal(reconcileSelection("q1", ids, 1, 1), "q1");
  assert.equal(reconcileSelection("q1", ids, 1, 2), null);
  assert.equal(reconcileSelection("q4", ids, 1, 1), null);
  assert.equal(reconcileSelection(null, ids, undefined, 1), null);
});

test("排列偏好只接受合法值，损坏或旧存储回退原布局", () => {
  for (const value of [
    null,
    false,
    [],
    "horizontal",
    { direction: "diagonal" },
  ]) {
    assert.deepEqual(canvasPreferences(value), {
      direction: "vertical",
      density: "comfortable",
    });
  }
  assert.deepEqual(
    canvasPreferences({ direction: "horizontal", density: "compact" }),
    { direction: "horizontal", density: "compact" },
  );
});

test("会话筛选按收藏及可靠提交计数工作，收藏优先保持组内顺序且不改源列表", () => {
  const sessions = [
    { key: "a", title: "第一条", commit_count: null },
    {
      key: "b",
      title: "第二条",
      favorite: true,
      commit_count: 1,
      last_commit: { repository: "/work/demo", branch: "feature/layout" },
    },
    { key: "c", title: "第三条", favorite: true, commit_count: 0 },
    { key: "d", title: "第四条", commit_count: 2 },
  ];
  assert.deepEqual(
    filterSessions(sessions, "", "all", true).map((s) => s.key),
    ["b", "c", "a", "d"],
  );
  assert.deepEqual(
    filterSessions(sessions, "", "favorites", false).map((s) => s.key),
    ["b", "c"],
  );
  assert.deepEqual(
    filterSessions(sessions, "", "commits", false).map((s) => s.key),
    ["b", "d"],
  );
  assert.deepEqual(
    filterSessions(sessions, "feature/layout", "all", false).map((s) => s.key),
    ["b"],
  );
  assert.deepEqual(
    filterSessions(sessions, "/work/demo", "all", false).map((s) => s.key),
    ["b"],
  );
  assert.deepEqual(
    sessions.map((s) => s.key),
    ["a", "b", "c", "d"],
  );
});

test("压缩分组弹窗保留同代追加，切换会话、重置或关联边消失清除范围", () => {
  const view = { key: "s1", generation: 1, edgeId: "e2", label: "Q2 → Q3" };
  const graph = {
    key: "s1",
    generation: 1,
    edges: [{ id: "e2" }, { id: "e3" }],
  };
  assert.equal(reconcileEventView(view, graph), view);
  assert.equal(
    reconcileEventView(view, {
      ...graph,
      edges: [...graph.edges, { id: "e4" }],
    }),
    view,
  );
  assert.equal(reconcileEventView(view, { ...graph, key: "s2" }), null);
  assert.equal(reconcileEventView(view, { ...graph, generation: 2 }), null);
  assert.equal(
    reconcileEventView(view, { ...graph, edges: [{ id: "e3" }] }),
    null,
  );
  assert.equal(reconcileEventView("all", graph), "all");
  assert.equal(reconcileEventView(null, graph), null);
});

test("收藏的问题可独立于父会话收藏检索，支持来源名称和项目路径", () => {
  const rows = [
    {
      favoriteId: "turn:a",
      sessionKey: "s1",
      sessionTitle: "机器人控制",
      cwd: "/work/Robot",
      promptPreview: "如何选择增益？",
    },
    {
      favoriteId: "turn:b",
      sessionKey: "s2",
      sessionTitle: "论文结构",
      cwd: "/work/paper",
      promptPreview: "调整摘要",
    },
  ];
  assert.deepEqual(filterFavoriteQuestions(rows, ""), rows);
  assert.deepEqual(filterFavoriteQuestions(rows, " 增益 "), [rows[0]]);
  assert.deepEqual(filterFavoriteQuestions(rows, "论文结构"), [rows[1]]);
  assert.deepEqual(filterFavoriteQuestions(rows, "/WORK/robot"), [rows[0]]);
  assert.deepEqual(filterFavoriteQuestions(rows, "没有这个词"), []);
  assert.equal(rows.length, 2);
});

test("收藏跳转只匹配完整快照中的唯一稳定身份，绝不复用旧问题编号", () => {
  const nodes = [
    { id: "q1", favorite_id: "turn:replacement" },
    { id: "q4", favorite_id: "turn:saved" },
  ];
  assert.equal(resolveFavoriteQuestion("turn:saved", nodes, false), "q4");
  assert.equal(resolveFavoriteQuestion("turn:saved", nodes, true), null);
  assert.equal(resolveFavoriteQuestion("turn:removed", nodes, false), null);
  assert.equal(resolveFavoriteQuestion("", [{ id: "q1" }], false), null);
  assert.equal(
    resolveFavoriteQuestion(
      "turn:saved",
      [...nodes, { id: "q5", favorite_id: "turn:saved" }],
      false,
    ),
    null,
  );
});
