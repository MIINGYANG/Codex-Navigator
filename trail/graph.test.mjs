import assert from "node:assert/strict";
import test from "node:test";
import {
  CARD_HEIGHT,
  CARD_WIDTH,
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
