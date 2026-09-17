import assert from "node:assert/strict";
import test from "node:test";
import {
  CARD_HEIGHT,
  CARD_WIDTH,
  compactionEdges,
  compactionMarkerPosition,
  layoutMetrics,
  edgePorts,
  highlightedPath,
  initialVisibleIds,
  indexGraph,
  layoutGraph,
  neighborhood,
  validEdges,
  visibleCanvasCenter,
} from "./graph.ts";

function fixture(count = 6) {
  const nodes = Array.from({ length: count }, (_, index) => ({
    id: `q${index + 1}`,
    turnIndex: index,
    ordinal: index + 1,
    title: `问题 ${index + 1}`,
    promptPreview: `用户原文 ${index + 1}`,
    timestamp: null,
    isLatest: index === count - 1,
  }));
  return {
    nodes,
    edges: nodes.slice(1).map((node, index) => ({
      id: `e${index + 1}`,
      source: nodes[index].id,
      target: node.id,
      type: "sequence",
    })),
  };
}
function noOverlap(positions) {
  const values = [...positions.values()];
  values.forEach((a, index) =>
    values
      .slice(index + 1)
      .forEach((b) =>
        assert.ok(
          Math.abs(a.x - b.x) >= CARD_WIDTH ||
            Math.abs(a.y - b.y) >= CARD_HEIGHT,
        ),
      ),
  );
}

test("linear layout is deterministic, chronological and non-overlapping", () => {
  const graph = fixture();
  const before = JSON.stringify(graph);
  const first = layoutGraph(graph);
  assert.deepEqual(first, layoutGraph(graph));
  noOverlap(first);
  assert.ok(first.get("q6").x > first.get("q1").x);
  assert.ok(first.get("q6").y > first.get("q1").y);
  assert.equal(JSON.stringify(graph), before);
});
test("a persisted branch receives distinct dagre positions without invented links", () => {
  const graph = fixture();
  graph.edges = [
    ["q1", "q2"],
    ["q1", "q3"],
    ["q2", "q4"],
    ["q2", "q5"],
    ["q3", "q6"],
  ].map(([source, target], index) => ({
    id: `e${index}`,
    source,
    target,
    type: index === 1 || index === 3 ? "branch" : "sequence",
  }));
  const positions = layoutGraph(graph);
  noOverlap(positions);
  assert.equal(validEdges(graph).length, 5);
  assert.equal(positions.get("q2").y, positions.get("q3").y);
  assert.notEqual(positions.get("q2").x, positions.get("q3").x);
  assert.ok(positions.get("q2").x < positions.get("q3").x);
  assert.ok(positions.get("q4").x < positions.get("q5").x);
  assert.ok(positions.get("q5").x < positions.get("q6").x);
});
test("path highlights ancestors and direct children, never grandchildren or siblings", () => {
  const graph = fixture();
  graph.edges = [
    ["q1", "q2"],
    ["q1", "q3"],
    ["q2", "q4"],
    ["q4", "q5"],
    ["q3", "q6"],
  ].map(([source, target], index) => ({
    id: `e${index}`,
    source,
    target,
    type: "sequence",
  }));
  const path = highlightedPath(graph, "q2");
  assert.deepEqual([...path.nodes].sort(), ["q1", "q2", "q4"]);
  assert.deepEqual([...path.edges].sort(), ["e0", "e2"]);
  assert.deepEqual([...neighborhood(graph, "q2")].sort(), ["q1", "q2", "q4"]);
});
test("invalid links are discarded, missing selection and cycles remain safe", () => {
  const graph = fixture(2);
  graph.edges.push(
    { id: "missing", source: "q2", target: "absent", type: "branch" },
    { id: "self", source: "q1", target: "q1", type: "branch" },
  );
  assert.equal(validEdges(graph).length, 1);
  assert.equal(highlightedPath(graph, "absent").nodes.size, 0);
  graph.edges.push({ id: "cycle", source: "q2", target: "q1", type: "branch" });
  assert.equal(highlightedPath(graph, "q2").nodes.size, 2);
});
test("live append preserves all existing and user-dragged positions", () => {
  const previous = layoutGraph(fixture(5));
  previous.set("q3", { x: 700, y: 410 });
  const current = layoutGraph(fixture(6), previous);
  for (const [id, point] of previous) assert.deepEqual(current.get(id), point);
  assert.ok(current.has("q6"));
  assert.equal(previous.size, 5);
});
test("new branch is placed without covering an existing sibling", () => {
  const graph = fixture(4);
  graph.edges = [
    { id: "a", source: "q1", target: "q2", type: "sequence" },
    { id: "b", source: "q1", target: "q3", type: "branch" },
  ];
  const previous = layoutGraph({
    nodes: graph.nodes.slice(0, 3),
    edges: graph.edges,
  });
  graph.edges.push({ id: "c", source: "q1", target: "q4", type: "branch" });
  noOverlap(layoutGraph(graph, previous));
});
test("1000-turn linear and branched sessions do not require recursive layout", () => {
  const graph = fixture(1000);
  const start = performance.now();
  const positions = layoutGraph(graph);
  assert.equal(positions.size, 1000);
  assert.ok(performance.now() - start < 1000);
  graph.edges.push({
    id: "branch",
    source: "q1",
    target: "q1000",
    type: "branch",
  });
  assert.equal(layoutGraph(graph).size, 1000);
});
test("first viewport caps long sessions but includes a short complete trail", () => {
  assert.deepEqual(initialVisibleIds(fixture(6)), [
    "q1",
    "q2",
    "q3",
    "q4",
    "q5",
    "q6",
  ]);
  assert.equal(initialVisibleIds(fixture(1000)).length, 6);
  assert.deepEqual(layoutGraph({ nodes: [], edges: [] }), new Map());
});

test("shared graph index resolves neighborhood and tooltip metadata without repeated scans", () => {
  const graph = fixture(1000);
  const index = indexGraph(graph);
  assert.equal(index.nodes.get("q750").ordinal, 750);
  assert.deepEqual([...neighborhood(graph, "q750", index)].sort(), [
    "q749",
    "q750",
    "q751",
  ]);
  assert.equal(highlightedPath(graph, "q750", index).nodes.size, 751);
  assert.equal(index.incoming.get("q750")[0].source, "q749");
  assert.equal(index.outgoing.get("q750")[0].target, "q751");
});

test("bulk live append uses bounded spatial lookups and preserves old coordinates", () => {
  const previous = layoutGraph(fixture(1));
  const start = performance.now();
  const next = layoutGraph(fixture(1000), previous);
  assert.equal(next.size, 1000);
  assert.deepEqual(next.get("q1"), previous.get("q1"));
  assert.ok(performance.now() - start < 1000);
  assert.ok(next.get("q1000").y > next.get("q998").y);
});

test("spatial collision lookups handle negative user-dragged positions", () => {
  const graph = fixture(3);
  const previous = new Map([
    ["q1", { x: -300, y: -400 }],
    ["q2", { x: -390, y: -604 }],
  ]);
  const next = layoutGraph(graph, previous);
  noOverlap(next);
  assert.deepEqual(next.get("q1"), previous.get("q1"));
});

test("mobile bottom sheet centers a question in the unobstructed upper canvas", () => {
  const center = visibleCanvasCenter(
    { left: 0, top: 190, width: 390, height: 630 },
    { left: 0, top: 320, width: 390, height: 500 },
  );
  assert.equal(center.x, 195);
  assert.equal(center.y, 255);
  assert.equal(center.height, 130);
});

test("medium right drawer centers a question in the remaining left canvas", () => {
  const center = visibleCanvasCenter(
    { left: 240, top: 180, width: 840, height: 620 },
    { left: 720, top: 80, width: 360, height: 720 },
  );
  assert.equal(center.x, 480);
  assert.equal(center.y, 490);
  assert.equal(center.width, 480);
});

test("desktop non-overlapping detail panel and fullscreen preserve the canvas center", () => {
  const rect = { left: 250, top: 170, width: 1060, height: 680 };
  const desktop = visibleCanvasCenter(rect, {
    left: 1310,
    top: 80,
    width: 360,
    height: 770,
  });
  assert.equal(desktop.x, 780);
  assert.equal(desktop.y, 510);
  assert.equal(desktop.width, 1060);
  const fullscreen = visibleCanvasCenter(
    { left: 0, top: 90, width: 1672, height: 851 },
    null,
  );
  assert.equal(fullscreen.x, 836);
  assert.equal(fullscreen.y, 515.5);
});

test("browser DOMRect prototype accessors are copied explicitly, preventing NaN viewport coordinates", () => {
  const prototype = {};
  for (const [key, value] of Object.entries({
    left: 250,
    top: 170,
    width: 1060,
    height: 680,
  }))
    Object.defineProperty(prototype, key, { get: () => value });
  const rect = Object.create(prototype);
  assert.deepEqual(Object.keys(rect), []);
  assert.deepEqual(visibleCanvasCenter(rect, null), {
    left: 250,
    top: 170,
    width: 1060,
    height: 680,
    x: 780,
    y: 510,
  });
  const mobile = Object.create(
    Object.defineProperties(
      {},
      {
        left: { get: () => 0 },
        top: { get: () => 190 },
        width: { get: () => 390 },
        height: { get: () => 630 },
      },
    ),
  );
  assert.equal(
    visibleCanvasCenter(mobile, { left: 0, top: 320, width: 390, height: 500 })
      .y,
    255,
  );
});

test("six linear questions occupy a readable 670 by 516 two-column trail without invented branches", () => {
  const graph = fixture(6);
  const positions = layoutGraph(graph);
  const xs = [...positions.values()].map((point) => point.x);
  const ys = [...positions.values()].map((point) => point.y);
  assert.equal(Math.max(...xs) - Math.min(...xs) + CARD_WIDTH, 670);
  assert.equal(Math.max(...ys) - Math.min(...ys) + CARD_HEIGHT, 516);
  assert.deepEqual(
    [...positions.values()],
    [
      { x: 0, y: 0 },
      { x: 390, y: 0 },
      { x: 390, y: 204 },
      { x: 0, y: 204 },
      { x: 0, y: 408 },
      { x: 390, y: 408 },
    ],
  );
  assert.equal(graph.edges.length, 5);
  assert.ok(graph.edges.every((edge) => edge.type === "sequence"));
  noOverlap(positions);
});

test("every adjacent snake edge follows the gap without crossing another question card", () => {
  const graph = fixture(12);
  const positions = layoutGraph(graph);
  const pointAtPort = (position, side) => ({
    x:
      position.x +
      (side === "left" ? 0 : side === "right" ? CARD_WIDTH : CARD_WIDTH / 2),
    y:
      position.y +
      (side === "top" ? 0 : side === "bottom" ? CARD_HEIGHT : CARD_HEIGHT / 2),
  });
  for (const edge of graph.edges) {
    const source = positions.get(edge.source);
    const target = positions.get(edge.target);
    const ports = edgePorts(source, target);
    const start = pointAtPort(source, ports.source);
    const end = pointAtPort(target, ports.target);
    assert.ok(start.x === end.x || start.y === end.y);
    for (let step = 1; step < 20; step++) {
      const point = {
        x: start.x + ((end.x - start.x) * step) / 20,
        y: start.y + ((end.y - start.y) * step) / 20,
      };
      for (const [id, rect] of positions) {
        if (id === edge.source || id === edge.target) continue;
        assert.ok(
          !(
            point.x > rect.x &&
            point.x < rect.x + CARD_WIDTH &&
            point.y > rect.y &&
            point.y < rect.y + CARD_HEIGHT
          ),
          `${edge.id} crosses ${id}`,
        );
      }
    }
  }
  assert.deepEqual(edgePorts(positions.get("q3"), positions.get("q4")), {
    source: "left",
    target: "right",
  });
});

test("incremental snake append matches fresh layout and persisted branches never reflow previous cards", () => {
  const previous = layoutGraph(fixture(5));
  assert.deepEqual(layoutGraph(fixture(6), previous), layoutGraph(fixture(6)));
  const graph = fixture(7);
  graph.edges[5] = {
    id: "branch-new",
    source: "q2",
    target: "q7",
    type: "branch",
  };
  const existing = layoutGraph(fixture(6));
  const next = layoutGraph(graph, existing);
  for (const [id, point] of existing) assert.deepEqual(next.get(id), point);
  noOverlap(next);
});

test("横向排列依据有效画布宽度取两到四列，紧凑密度独立减少空白", () => {
  const comfortable = { direction: "horizontal", density: "comfortable" };
  const compact = { direction: "horizontal", density: "compact" };
  assert.equal(layoutMetrics(comfortable, 390).columns, 2);
  assert.equal(layoutMetrics(comfortable, 1060).columns, 3);
  assert.equal(layoutMetrics(comfortable, 1600).columns, 4);
  assert.equal(layoutMetrics(comfortable, 5000).columns, 4);
  assert.equal(
    layoutMetrics({ ...compact, direction: "vertical" }, 1600).columns,
    2,
  );
  const graph = fixture(12);
  const loose = layoutGraph(graph, new Map(), comfortable, 1600);
  const dense = layoutGraph(graph, new Map(), compact, 1600);
  assert.ok(dense.get("q9").y < loose.get("q9").y);
  assert.ok(dense.get("q4").x < loose.get("q4").x);
});

test("两至四列蛇形遵循原顺序，转折边不穿过卡片", () => {
  for (const density of ["comfortable", "compact"]) {
    for (const availableWidth of [600, 1060, 1600]) {
      const layout = { direction: "horizontal", density };
      const metrics = layoutMetrics(layout, availableWidth);
      const graph = fixture(24);
      const before = JSON.stringify(graph);
      const positions = layoutGraph(graph, new Map(), layout, availableWidth);
      for (const edge of graph.edges) {
        const source = positions.get(edge.source);
        const target = positions.get(edge.target);
        const ports = edgePorts(source, target, metrics);
        if (source.y === target.y) {
          assert.equal(
            Math.abs(target.x - source.x),
            metrics.width + metrics.gapX,
          );
          assert.equal(ports.source, target.x > source.x ? "right" : "left");
        } else {
          assert.equal(source.x, target.x);
          assert.equal(target.y - source.y, metrics.height + metrics.gapY);
          assert.deepEqual(ports, { source: "bottom", target: "top" });
        }
      }
      assert.equal(JSON.stringify(graph), before);
    }
  }
});

test("真实分支使用横向层级，布局与密度切换不创造关系", () => {
  const graph = fixture(6);
  graph.edges[1] = { id: "fork", source: "q1", target: "q3", type: "branch" };
  const before = JSON.stringify(graph.edges);
  const positions = layoutGraph(
    graph,
    new Map(),
    { direction: "horizontal", density: "compact" },
    1200,
  );
  assert.ok(positions.get("q2").x > positions.get("q1").x);
  assert.ok(positions.get("q3").x > positions.get("q1").x);
  assert.notEqual(positions.get("q2").y, positions.get("q3").y);
  assert.ok(positions.get("q2").y < positions.get("q3").y);
  assert.deepEqual(
    edgePorts(
      positions.get("q1"),
      positions.get("q3"),
      layoutMetrics({ direction: "horizontal", density: "compact" }, 1200),
    ),
    { source: "right", target: "left" },
  );
  assert.equal(JSON.stringify(graph.edges), before);
});

test("紧凑横向布局追加及收藏变化保留已有拖拽坐标", () => {
  const options = { direction: "horizontal", density: "compact" };
  const previous = layoutGraph(fixture(8), new Map(), options, 1200);
  previous.set("q4", { x: -123, y: 777 });
  const graph = fixture(12);
  graph.nodes[3].favorite = true;
  const current = layoutGraph(graph, previous, options, 1200);
  for (const [id, point] of previous) assert.deepEqual(current.get(id), point);
  assert.equal(current.size, 12);
});

test("压缩事件只落在记录轮次之后的唯一真实主线边，多次压缩不丢失", () => {
  const graph = fixture(4);
  const event = {
    id: "c1",
    kind: "compaction",
    turn_index: 1,
    timestamp: null,
    source: "compacted",
    trigger: "unknown",
  };
  const events = [
    event,
    { ...event, id: "c2", trigger: "manual" },
    { ...event, id: "commit", kind: "commit" },
  ];
  const result = compactionEdges(graph, events);
  assert.deepEqual([...result.keys()], ["e2"]);
  assert.deepEqual(
    result.get("e2").map((item) => item.id),
    ["c1", "c2"],
  );
  assert.equal(result.get("e2")[0].trigger, "unknown");
});

test("无轮次、末尾压缩、缺边与多主线歧义不虚构压缩定位", () => {
  const graph = fixture(4);
  const event = {
    id: "c1",
    kind: "compaction",
    turn_index: null,
    timestamp: null,
    source: "compacted",
  };
  assert.equal(compactionEdges(graph, [event]).size, 0);
  assert.equal(compactionEdges(graph, [{ ...event, turn_index: 3 }]).size, 0);
  graph.edges = graph.edges.filter((edge) => edge.source !== "q2");
  assert.equal(compactionEdges(graph, [{ ...event, turn_index: 1 }]).size, 0);
  graph.edges.push(
    { id: "a", source: "q2", target: "q3", type: "sequence" },
    { id: "b", source: "q2", target: "q4", type: "sequence" },
  );
  assert.equal(compactionEdges(graph, [{ ...event, turn_index: 1 }]).size, 0);
});

test("大规模横向分支也沿正确方向前进且不递归溢出", () => {
  const graph = fixture(1000);
  graph.edges.push({
    id: "fork",
    source: "q1",
    target: "q1000",
    type: "branch",
  });
  const positions = layoutGraph(
    graph,
    new Map(),
    { direction: "horizontal", density: "compact" },
    1400,
  );
  assert.equal(positions.size, 1000);
  assert.ok(positions.get("q900").x > positions.get("q800").x);
});

test("同边四条压缩聚合为一组，其他边、提交与末尾事件不混入，追加后按原序更新", () => {
  const graph = fixture(5);
  const events = Array.from({ length: 4 }, (_, i) => ({
    id: `c${i}`,
    kind: "compaction",
    turn_index: 1,
    timestamp: `2026-09-17T10:0${i}:00Z`,
    source: "compacted",
    trigger: i === 0 ? "auto" : i === 1 ? "manual" : "unknown",
  }));
  events.splice(1, 0, { ...events[0], id: "commit", kind: "commit" });
  events.push(
    { ...events[0], id: "other-edge", turn_index: 2 },
    { ...events[0], id: "last-turn", turn_index: 4 },
  );
  const groups = compactionEdges(graph, events);
  assert.equal(groups.size, 2);
  assert.deepEqual(
    groups.get("e2").map((event) => event.id),
    ["c0", "c1", "c2", "c3"],
  );
  assert.deepEqual(
    groups.get("e3").map((event) => event.id),
    ["other-edge"],
  );
  const appended = compactionEdges(graph, [
    ...events,
    { ...events[0], id: "later" },
  ]);
  assert.equal(groups.get("e2").length, 4);
  assert.deepEqual(
    appended.get("e2").map((event) => event.id),
    ["c0", "c1", "c2", "c3", "later"],
  );
});

test("左右端口的压缩徽标始终在卡片上方，紧凑和拖拽后不遮挡卡片", () => {
  for (const height of [96, 108]) {
    for (const [sourceY, targetY] of [
      [200, 200],
      [200, 260],
      [260, 200],
    ]) {
      for (const [sourcePosition, targetPosition] of [
        ["right", "left"],
        ["left", "right"],
      ]) {
        const edge = { sourceY, targetY, sourcePosition, targetPosition };
        const center = { x: 300, y: (sourceY + targetY) / 2 };
        const marker = compactionMarkerPosition(edge, center, height);
        assert.equal(marker.x, center.x);
        assert.equal(marker.y, Math.min(sourceY, targetY) - height / 2 - 18);
        // The marker is 26px tall; its bottom remains 5px above either card.
        assert.ok(marker.y + 13 < sourceY - height / 2);
        assert.ok(marker.y + 13 < targetY - height / 2);
        assert.deepEqual(center, { x: 300, y: (sourceY + targetY) / 2 });
      }
    }
  }
});

test("上下端口的压缩徽标保留原边中点，不改变蛇形转折位置", () => {
  const center = { x: 380, y: 320 };
  for (const [sourcePosition, targetPosition] of [
    ["bottom", "top"],
    ["top", "bottom"],
  ]) {
    assert.equal(
      compactionMarkerPosition(
        { sourceY: 280, targetY: 360, sourcePosition, targetPosition },
        center,
        96,
      ),
      center,
    );
  }
});
