import test from "node:test";
import assert from "node:assert/strict";
import {
  statusLabel,
  sessionTitle,
  filterSessions,
  pageFor,
  selectionAfterUpdate,
  navigation,
  RequestGate,
  safeHref,
  inlineTokens,
  markdownBlocks,
  pendingTurns,
  sameTurnSnapshot,
  activityWindow,
  navigationContextMatches,
  pausesFollow,
  chronologicalTarget,
} from "./state.mjs";

test("无筛选目录 G 使用最新元数据，不受尚未返回的旧分页影响", () => {
  assert.equal(
    chronologicalTarget("end", 0, { turn_count: 106, latest_active: 105 }),
    105,
  );
  assert.equal(
    chronologicalTarget("end", 0, { turn_count: 106, latest_active: 103 }),
    103,
  );
  assert.equal(chronologicalTarget("start", 105, { turn_count: 106 }), 0);
  assert.equal(chronologicalTarget("previous", 0, { turn_count: 106 }), 0);
  assert.equal(chronologicalTarget("next", 105, { turn_count: 106 }), 105);
  assert.equal(chronologicalTarget("end", null, { turn_count: 0 }), null);
});

test("生命周期与过程告警独立，不判断答案正确性", () => {
  assert.equal(statusLabel("completed"), "✓ 正常结束");
  assert.equal(statusLabel("failed"), "✕ 执行错误");
  assert.equal(statusLabel("unexpected"), "? 状态未知");
});
test("会话标题和筛选有稳定的缺省值", () => {
  const sessions = [
    { id: "AA", title: null, first_prompt: "修复问题", cwd: "/work/Navigator" },
    { id: "BB" },
  ];
  assert.equal(sessionTitle(sessions[0]), "修复问题");
  assert.equal(filterSessions(sessions, " navigator ")[0].id, "AA");
  assert.equal(filterSessions(sessions, "bb")[0].id, "BB");
  assert.equal(filterSessions(sessions, "none").length, 0);
});
test("长 Prompt 不会成为无限长的会话标题，保留首行与 Unicode 字符", () => {
  assert.equal(sessionTitle({ first_prompt: "\n\n第一行\n第二行" }), "第一行");
  assert.equal(
    Array.from(sessionTitle({ title: "🪶".repeat(200) })).length,
    91,
  );
  assert.equal(sessionTitle({ title: "  ", id: "fallback" }), "fallback");
});
test("分页边界不会越界", () => {
  assert.equal(pageFor(-1), 0);
  assert.equal(pageFor(99), 0);
  assert.equal(pageFor(100), 100);
  assert.equal(pageFor(99999), 99900);
});
test("活动最多保留 64 条 DOM 记录，前后组均可访问", () => {
  assert.deepEqual(activityWindow(0, 8, 8), { replace: false, start: 0 });
  assert.deepEqual(activityWindow(0, 64, 64), { replace: true, start: 64 });
  assert.deepEqual(activityWindow(64, 8, 0), { replace: true, start: 0 });
});
test("实时更新保留历史轮次，跟随模式才跳到最新", () => {
  const previous = { generation: 2, turn_count: 8, latest_active: 7 };
  const next = { generation: 2, turn_count: 9, latest_active: 8 };
  assert.equal(selectionAfterUpdate(3, previous, next, false, false), 3);
  assert.equal(selectionAfterUpdate(7, previous, next, true, false), 8);
  assert.equal(selectionAfterUpdate(7, previous, next, true, true), 7);
  assert.equal(selectionAfterUpdate(null, previous, next, false, false), 8);
});
test("会话重载和回滚收缩修正已失效选择", () => {
  assert.equal(
    selectionAfterUpdate(
      8,
      { generation: 1 },
      { generation: 2, turn_count: 3, latest_active: 1 },
      false,
      false,
    ),
    1,
  );
  assert.equal(
    selectionAfterUpdate(
      8,
      { generation: 2 },
      { generation: 2, turn_count: 3, latest_active: 2 },
      false,
      false,
    ),
    2,
  );
  assert.equal(
    selectionAfterUpdate(0, null, { turn_count: 0 }, true, false),
    null,
  );
});
test("宽窄布局不影响 j/k/g/G 按焦点分发", () => {
  for (const focus of ["picker", "directory"]) {
    assert.deepEqual(navigation("G", focus), {
      kind: "select",
      direction: "end",
    });
    assert.deepEqual(navigation("j", focus), {
      kind: "select",
      direction: "next",
    });
  }
  assert.deepEqual(navigation("G", "reader"), {
    kind: "scroll",
    direction: "end",
  });
  assert.deepEqual(navigation("k", "reader"), {
    kind: "scroll",
    direction: "up",
  });
  assert.equal(navigation("f", "reader"), null);
});
test("过期请求不再有权覆盖当前会话或搜索", () => {
  const gate = new RequestGate(),
    first = gate.next(),
    second = gate.next();
  assert.equal(gate.current(first), false);
  assert.equal(gate.current(second), true);
  gate.invalidate();
  assert.equal(gate.current(second), false);
  const snapshot = gate.capture();
  assert.equal(gate.capture(), snapshot);
  assert.equal(gate.current(snapshot), true);
});
test("跨页导航在切换视图、会话或搜索后失效", () => {
  const before = { view: 3, key: "a", query: "prompt" };
  assert.equal(navigationContextMatches(before, { ...before }), true);
  for (const after of [
    { ...before, view: 4 },
    { ...before, key: "b" },
    { ...before, query: "other" },
  ]) {
    assert.equal(navigationContextMatches(before, after), false);
  }
});
test("原生正文滚动按键也会暂停跟随，不劫持原生行为", () => {
  for (const key of [
    "PageDown",
    "PageUp",
    "ArrowDown",
    "ArrowUp",
    "Home",
    "End",
    " ",
  ])
    assert.equal(pausesFollow(key), true);
  assert.equal(pausesFollow("Tab"), false);
});
test("历史距离不是新消息计数，回滚不会产生负数", () => {
  assert.equal(pendingTurns(100, 100), 0);
  assert.equal(pendingTurns(100, 103), 3);
  assert.equal(pendingTurns(100, 97), 0);
});
test("其他轮更新不使历史 DOM 或已加载分页失效", () => {
  const previous = {
    generation: 1,
    revision: 100,
    turn: { index: 4, revision: 8 },
  };
  const next = {
    generation: 1,
    revision: 105,
    turn: { index: 4, revision: 8 },
  };
  assert.equal(sameTurnSnapshot(previous, next), true);
  assert.equal(
    sameTurnSnapshot(previous, { ...next, turn: { index: 4, revision: 9 } }),
    false,
  );
  assert.equal(sameTurnSnapshot(previous, { ...next, generation: 2 }), false);
  assert.equal(sameTurnSnapshot(null, next), false);
});
test("链接拒绝脚本、文件、相对地址和控制字符", () => {
  for (const href of [
    "javascript:alert(1)",
    "data:text/html,x",
    "file:///etc/passwd",
    "/api/info",
    "//evil.test",
    "https://a.test/\n",
  ]) {
    assert.equal(safeHref(href), null, href);
  }
  assert.equal(safeHref("https://example.org/x"), "https://example.org/x");
  assert.equal(safeHref("mailto:a@example.org"), "mailto:a@example.org");
});
test("Markdown HTML 始终为文本，图片不生成图片节点", () => {
  const blocks = markdownBlocks(
    "<script>window.pwned=true</script>\n\n![x](https://evil.test/x)",
  );
  assert.equal(blocks[0].type, "p");
  assert.equal(blocks[0].text, "<script>window.pwned=true</script>");
  assert.ok(!blocks.some((block) => block.type === "img"));
  assert.equal(inlineTokens("[x](javascript:evil)")[0].type, "text");
});
test("Markdown 支持标题、列表、代码块且未闭合围栏仍显示", () => {
  const blocks = markdownBlocks(
    "# 标题\n\n- 第一\n- 第二\n\n```rs\nlet x = 1;\n```\n\n> 引用\n\n~~~\nunfinished",
  );
  assert.deepEqual(
    blocks.map((block) => block.type),
    ["h2", "ul", "pre", "blockquote", "pre"],
  );
  assert.deepEqual(blocks[1].items, ["第一", "第二"]);
  assert.equal(blocks[4].text, "unfinished");
});
test("内联代码、强调、链接只产生明确安全 token", () => {
  assert.deepEqual(inlineTokens("a `b` **c** [d](https://example.org)"), [
    { type: "text", text: "a " },
    { type: "code", text: "b" },
    { type: "text", text: " " },
    { type: "strong", text: "c" },
    { type: "text", text: " " },
    { type: "link", text: "d", href: "https://example.org/" },
  ]);
});
