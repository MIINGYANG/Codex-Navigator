import assert from "node:assert/strict";
import test from "node:test";
import {
  titleOf,
  relativeTime,
  fullTime,
  groupResults,
  reconcileSelection,
  initialLoadTransition,
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

test("会话使用第一个非空原文，不推测改写语义", () => {
  assert.equal(
    titleOf({
      first_prompt: " \n 原文   中的词？\n下一行",
      title: "不要采用这个标题",
    }),
    "原文 中的词？",
  );
  assert.equal(
    titleOf({ first_prompt: " ", title: "已保存标题" }),
    "已保存标题",
  );
  assert.equal(titleOf({ id: "session-id" }), "session-id");
  assert.equal(titleOf(), "未命名会话");
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
