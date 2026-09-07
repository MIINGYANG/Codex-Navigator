import test from "node:test";
import assert from "node:assert/strict";
import {
  questionPath,
  processPath,
  PAGE_SIZE,
  MAX_VISIBLE_ITEMS,
} from "./state.mjs";

test("问题脉络空态与当前节点选择", () => {
  assert.deepEqual(questionPath([], null), { nodes: [], edges: [] });
  const turns = [
    { index: 4, preview: "修复 🌱" },
    { index: 5, preview: "加测试" },
  ];
  const result = questionPath(turns, 5);
  assert.deepEqual(result.nodes, [
    { ...turns[0], selected: false },
    { ...turns[1], selected: true },
  ]);
  assert.deepEqual(result.edges, [
    { from: 4, to: 5, skipped: 0, label: "下一轮 · 记录顺序" },
  ]);
});

test("筛选间隙明确表示未展示轮次，不推断因果", () => {
  const result = questionPath([{ index: 2 }, { index: 7 }, { index: 9 }], 8);
  assert.deepEqual(result.edges, [
    { from: 2, to: 7, skipped: 4, label: "中间 4 轮未展示" },
    { from: 7, to: 9, skipped: 1, label: "中间 1 轮未展示" },
  ]);
  assert.ok(result.nodes.every((node) => !node.selected));
});

test("相关性倒序输入防御性按当前窗口记录顺序排列，不修改原数组", () => {
  const turns = Object.freeze([
    Object.freeze({ index: 4, preview: "后来的问题" }),
    Object.freeze({ index: 1, preview: "更早的问题" }),
  ]);
  const result = questionPath(turns, 4);
  assert.deepEqual(
    result.nodes.map((node) => node.index),
    [1, 4],
  );
  assert.deepEqual(result.edges, [
    { from: 1, to: 4, skipped: 2, label: "中间 2 轮未展示" },
  ]);
  assert.deepEqual(
    turns.map((turn) => turn.index),
    [4, 1],
  );
});

test("问题节点数量有界且不改变来源对象", () => {
  const turns = Object.freeze(
    Array.from({ length: PAGE_SIZE + 9 }, (_, index) =>
      Object.freeze({ index, preview: `问题 ${index}` }),
    ),
  );
  const result = questionPath(turns, 0);
  assert.equal(result.nodes.length, PAGE_SIZE);
  assert.equal(result.edges.length, PAGE_SIZE - 1);
  assert.equal(turns[0].selected, undefined);
});

test("无 detail 不创建虚构过程；图片问题和空文本有明确缺省", () => {
  assert.deepEqual(processPath(null, [], 0, null), []);
  assert.deepEqual(processPath(undefined, [], 0, null), []);
  const [node] = processPath(
    {
      turn: { prompt: { text: "", preview: "", images_count: 2 } },
      items_total: 0,
    },
    [],
    0,
    null,
  );
  assert.equal(node.id, "prompt");
  assert.equal(node.label, "你的问题");
  assert.equal(node.imagesCount, 2);
  assert.equal(node.text, "（无文字内容）");
  assert.equal(
    processPath({ turn: {}, items_total: 0 }, [], 0, null)[0].imagesCount,
    0,
  );
});

test("Prompt 保留 Unicode 与省略提示，不把预览当全文", () => {
  const prompt = Object.freeze({
    text: "",
    preview: "问题👨‍💻：为什么？",
    omitted_bytes: 900,
    images_count: 0,
  });
  const detail = Object.freeze({
    turn: Object.freeze({ prompt }),
    items_total: 0,
  });
  const [node] = processPath(detail, [], 0, null);
  assert.equal(node.text, prompt.preview);
  assert.equal(node.omittedBytes, 900);
  assert.match(node.notice, /目录摘要/u);
  assert.match(node.notice, /不能恢复/u);
  assert.equal(
    processPath(
      { turn: { prompt: { ...prompt, text: "保留原文🧪" } } },
      [],
      0,
      null,
    )[0].text,
    "保留原文🧪",
  );
});

test("各类过程记录直接投影，工具错误不改变轮次或最终回复", () => {
  const items = [
    { index: 0, type: "agent_message", text: "公开进展", phase: "commentary" },
    {
      index: 1,
      type: "tool_call",
      name: "exec_command",
      summary: "cargo test",
    },
    { index: 2, type: "tool_output", summary: "failed", is_error: true },
    { index: 3, type: "file_activity", path: "/demo/测试.rs", kind: "read" },
    { index: 4, type: "notice", text: "提示" },
    { index: 5, type: "future_type", summary: "未来记录" },
    {
      index: 6,
      type: "agent_message",
      text: "最终回复",
      phase: "final_answer",
    },
    { index: 7, type: "agent_message", text: "无 phase 的晚到消息" },
  ];
  const snapshot = JSON.stringify(items);
  const nodes = processPath(
    { turn: { status: "completed", prompt: {} }, items_total: items.length },
    items,
    0,
    null,
  ).slice(1);
  assert.equal(nodes.length, items.length);
  assert.deepEqual(
    nodes.map((node) => node.kind),
    items.map((item) => item.type),
  );
  assert.equal(nodes[0].isFinal, false);
  assert.equal(nodes[1].name, "exec_command");
  assert.equal(nodes[2].isError, true);
  assert.equal(nodes[3].text, "/demo/测试.rs");
  assert.equal(nodes[5].label, "其他记录");
  assert.equal(nodes[6].label, "最终回复（已标记）");
  assert.equal(nodes[6].isFinal, true);
  assert.equal(nodes[6].isError, false);
  assert.equal(nodes[7].isFinal, false);
  assert.equal(JSON.stringify(items), snapshot);
});

test("仅公开明确 phase 可标记最终回复，不采纳非消息上的标记", () => {
  const nodes = processPath(
    { turn: { prompt: {} }, final_answer: { index: 1 }, items_total: 3 },
    [
      { index: 0, type: "tool_output", phase: "final_answer" },
      { index: 1, type: "agent_message", phase: "commentary" },
      {
        index: 2,
        type: "agent_message",
        phase: "analysis",
        text: "已记录消息",
      },
    ],
    0,
    null,
  );
  assert.ok(nodes.slice(1).every((node) => !node.isFinal));
  assert.equal(nodes[3].label, "助手消息");
});

test("活动省略记录明确不能恢复，不用相邻节点填补", () => {
  const nodes = processPath(
    { turn: { prompt: {} }, items_total: 1 },
    [{ index: 0, type: "omitted" }],
    0,
    null,
  );
  assert.equal(nodes[1].omitted, true);
  assert.match(nodes[1].text, /内存预算/u);
  assert.match(nodes[1].text, /不能恢复/u);
});

test("未知类型与原型属性重名也按其他记录展示", () => {
  const nodes = processPath({ turn: { prompt: {} }, items_total: 2 }, [
    { index: 0, type: "__proto__" },
    { index: 1, type: "constructor" },
  ]);
  assert.equal(nodes[1].label, "其他记录");
  assert.equal(nodes[2].label, "其他记录");
});

test("分页前后缺口根据实际展示尾部计算，最多展示 64 条真实记录", () => {
  const items = Array.from({ length: 80 }, (_, offset) => ({
    index: offset + 64,
    type: "notice",
    text: `${offset}`,
  }));
  const nodes = processPath(
    { turn: { prompt: {} }, items_total: 200 },
    items,
    64,
    144,
  );
  assert.equal(nodes[1].id, "before");
  assert.match(nodes[1].label, /64 条前序记录未展示/u);
  const records = nodes.filter((node) => node.itemIndex !== undefined);
  assert.equal(records.length, MAX_VISIBLE_ITEMS);
  assert.equal(records.at(-1).id, "item-127");
  assert.equal(nodes.at(-1).id, "after");
  assert.match(nodes.at(-1).label, /72 条后续记录未展示/u);
});

test("最后一页没有后续缺口，未加载的空页仍提示后续记录", () => {
  const detail = { turn: { prompt: {} }, items_total: 66 };
  const nodes = processPath(
    detail,
    [
      { index: 64, type: "notice" },
      { index: 65, type: "notice" },
    ],
    64,
    null,
  );
  assert.equal(nodes.at(-1).id, "item-65");
  const emptyPage = processPath(detail, [], 64, 64);
  assert.equal(emptyPage.at(-1).id, "after");
  assert.match(emptyPage.at(-1).label, /2 条后续记录未展示/u);
});
