import dagre from "@dagrejs/dagre";

export interface SessionEvent {
  id: string;
  kind: "commit" | "compaction";
  turn_index: number | null;
  timestamp: string | null;
  source: string;
  hash?: string | null;
  repository?: string | null;
  branch?: string | null;
  version?: string | null;
  summary?: string;
  trigger?: "auto" | "manual" | "unknown";
}
export interface CanvasLayout {
  direction: "vertical" | "horizontal";
  density: "comfortable" | "compact";
}
export const DEFAULT_LAYOUT: CanvasLayout = {
  direction: "vertical",
  density: "comfortable",
};
export interface QuestionNode {
  id: string;
  turnIndex: number;
  ordinal: number;
  title: string;
  promptPreview: string;
  timestamp: string | null;
  parentId?: string | null;
  isLatest: boolean;
  favorite?: boolean;
  bookmarkId?: string;
  commits?: SessionEvent[];
}
export interface QuestionEdge {
  id: string;
  source: string;
  target: string;
  type: "sequence" | "branch";
}
export interface QuestionGraph {
  nodes: QuestionNode[];
  edges: QuestionEdge[];
}
export interface Position {
  x: number;
  y: number;
}
export interface ScreenRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** Largest unobstructed strip; desktop panels outside the canvas do not affect it. */
export function visibleCanvasCenter(
  canvas: ScreenRect,
  occlusion: ScreenRect | null,
) {
  const right = canvas.left + canvas.width;
  const bottom = canvas.top + canvas.height;
  // DOMRect exposes geometry through prototype getters, not enumerable own keys.
  // Copy fields explicitly so real browser measurements behave like test objects.
  const frame = {
    left: canvas.left,
    top: canvas.top,
    width: canvas.width,
    height: canvas.height,
  };
  let visible = frame;
  if (occlusion) {
    const left = Math.max(canvas.left, occlusion.left);
    const top = Math.max(canvas.top, occlusion.top);
    const overlapRight = Math.min(right, occlusion.left + occlusion.width);
    const overlapBottom = Math.min(bottom, occlusion.top + occlusion.height);
    if (overlapRight > left && overlapBottom > top) {
      const strips = [
        { ...frame, width: left - canvas.left },
        { ...frame, left: overlapRight, width: right - overlapRight },
        { ...frame, height: top - canvas.top },
        { ...frame, top: overlapBottom, height: bottom - overlapBottom },
      ];
      visible = strips.reduce((largest, strip) =>
        strip.width * strip.height > largest.width * largest.height
          ? strip
          : largest,
      );
    }
  }
  return {
    ...visible,
    x: visible.left + visible.width / 2,
    y: visible.top + visible.height / 2,
  };
}
export const CARD_WIDTH = 280;
export const CARD_HEIGHT = 108;

export function layoutMetrics(
  layout: CanvasLayout = DEFAULT_LAYOUT,
  availableWidth = 670,
) {
  const compact = layout.density === "compact";
  const width = compact ? 248 : CARD_WIDTH;
  const height = compact ? 96 : CARD_HEIGHT;
  const gapX = compact ? 54 : 110;
  const gapY = compact ? 56 : 96;
  const columns =
    layout.direction === "horizontal"
      ? Math.max(
          2,
          Math.min(
            4,
            Math.floor((Math.max(0, availableWidth) + gapX) / (width + gapX)),
          ),
        )
      : 2;
  return { width, height, gapX, gapY, columns, direction: layout.direction };
}
export function linearPosition(
  index: number,
  layout: CanvasLayout = DEFAULT_LAYOUT,
  availableWidth = 670,
): Position {
  const { width, height, gapX, gapY, columns } = layoutMetrics(
    layout,
    availableWidth,
  );
  const row = Math.floor(index / columns);
  const column =
    row % 2 === 0 ? index % columns : columns - 1 - (index % columns);
  return { x: column * (width + gapX), y: row * (height + gapY) };
}

/** A compaction belongs after its recorded turn; never invent a relationship. */
export function compactionEdges(
  graph: QuestionGraph,
  events: SessionEvent[],
): Map<string, SessionEvent[]> {
  const index = indexGraph(graph);
  const turns = new Map<number, QuestionNode[]>();
  for (const node of graph.nodes) {
    if (!turns.has(node.turnIndex)) turns.set(node.turnIndex, []);
    turns.get(node.turnIndex)!.push(node);
  }
  const result = new Map<string, SessionEvent[]>();
  for (const event of events) {
    if (event.kind !== "compaction" || event.turn_index === null) continue;
    const matches = turns.get(event.turn_index) ?? [];
    if (matches.length !== 1) continue;
    const node = matches[0];
    const candidates = (index.outgoing.get(node.id) ?? []).filter(
      (edge) =>
        edge.type === "sequence" &&
        index.nodes.get(edge.target)!.turnIndex > node.turnIndex,
    );
    if (candidates.length !== 1) continue;
    const edgeId = candidates[0].id;
    if (!result.has(edgeId)) result.set(edgeId, []);
    result.get(edgeId)!.push(event);
  }
  return result;
}

export type PortSide = "top" | "bottom" | "left" | "right";
export function edgePorts(
  source: Position,
  target: Position,
  dimensions: {
    width: number;
    height: number;
    direction?: CanvasLayout["direction"];
  } = { width: CARD_WIDTH, height: CARD_HEIGHT },
): { source: PortSide; target: PortSide } {
  if (
    (dimensions.direction === "horizontal" ||
      Math.abs(target.y - source.y) < dimensions.height / 2) &&
    Math.abs(target.x - source.x) >= dimensions.width
  ) {
    return target.x > source.x
      ? { source: "right", target: "left" }
      : { source: "left", target: "right" };
  }
  return target.y >= source.y
    ? { source: "bottom", target: "top" }
    : { source: "top", target: "bottom" };
}

export interface GraphIndex {
  nodes: Map<string, QuestionNode>;
  edges: Map<string, QuestionEdge>;
  incoming: Map<string, QuestionEdge[]>;
  outgoing: Map<string, QuestionEdge[]>;
}

export function indexGraph(graph: QuestionGraph): GraphIndex {
  const nodes = new Map(graph.nodes.map((node) => [node.id, node]));
  const edges = new Map<string, QuestionEdge>();
  const incoming = new Map<string, QuestionEdge[]>();
  const outgoing = new Map<string, QuestionEdge[]>();
  for (const edge of graph.edges) {
    if (
      edge.source === edge.target ||
      !nodes.has(edge.source) ||
      !nodes.has(edge.target)
    )
      continue;
    edges.set(edge.id, edge);
    if (!incoming.has(edge.target)) incoming.set(edge.target, []);
    if (!outgoing.has(edge.source)) outgoing.set(edge.source, []);
    incoming.get(edge.target)!.push(edge);
    outgoing.get(edge.source)!.push(edge);
  }
  return { nodes, edges, incoming, outgoing };
}

/** Only consume persisted edges. Layout never creates or changes a relationship. */
export function validEdges(graph: QuestionGraph): QuestionEdge[] {
  const ids = new Set(graph.nodes.map((node) => node.id));
  return graph.edges.filter(
    (edge) =>
      edge.source !== edge.target &&
      ids.has(edge.source) &&
      ids.has(edge.target),
  );
}

export function neighborhood(
  graph: QuestionGraph,
  id: string,
  index = indexGraph(graph),
): Set<string> {
  const result = new Set([id]);
  for (const edge of index.outgoing.get(id) ?? []) result.add(edge.target);
  for (const edge of index.incoming.get(id) ?? []) result.add(edge.source);
  return result;
}

export function highlightedPath(
  graph: QuestionGraph,
  id: string | null,
  index = indexGraph(graph),
): { nodes: Set<string>; edges: Set<string> } {
  const nodes = new Set<string>();
  const edges = new Set<string>();
  if (!id || !index.nodes.has(id)) return { nodes, edges };
  const pending = [id];
  while (pending.length) {
    const current = pending.pop()!;
    if (nodes.has(current)) continue;
    nodes.add(current);
    for (const edge of index.incoming.get(current) ?? []) {
      edges.add(edge.id);
      pending.push(edge.source);
    }
  }
  for (const edge of index.outgoing.get(id) ?? []) {
    nodes.add(edge.target);
    edges.add(edge.id);
  }
  return { nodes, edges };
}

function overlaps(
  a: Position,
  b: Position,
  width: number,
  height: number,
): boolean {
  return Math.abs(a.x - b.x) < width + 32 && Math.abs(a.y - b.y) < height + 32;
}

/** Stable append keeps both the viewport and previously dragged nodes undisturbed. */
export function layoutGraph(
  graph: QuestionGraph,
  previous: Map<string, Position> = new Map(),
  layout: CanvasLayout = DEFAULT_LAYOUT,
  availableWidth = 670,
): Map<string, Position> {
  const { width, height, gapX, gapY } = layoutMetrics(layout, availableWidth);
  const horizontal = layout.direction === "horizontal";
  const positions = new Map<string, Position>();
  if (!graph.nodes.length) return positions;
  const links = validEdges(graph);
  const ordered = [...graph.nodes].sort(
    (a, b) => a.ordinal - b.ordinal || a.id.localeCompare(b.id),
  );
  const incoming = new Map(links.map((edge) => [edge.target, edge.source]));
  const outgoing = new Map<string, number>();
  for (const edge of links)
    outgoing.set(edge.source, (outgoing.get(edge.source) ?? 0) + 1);
  const branched = [...outgoing.values()].some((count) => count > 1);
  if (previous.size) {
    const cells = new Map<string, Position[]>();
    const cellWidth = width + 32;
    const cellHeight = height + 32;
    const cell = (position: Position) => [
      Math.floor(position.x / cellWidth),
      Math.floor(position.y / cellHeight),
    ];
    const remember = (position: Position) => {
      const key = cell(position).join(":");
      if (!cells.has(key)) cells.set(key, []);
      cells.get(key)!.push(position);
    };
    const occupied = (position: Position) => {
      const [x, y] = cell(position);
      for (let dx = -1; dx <= 1; dx++)
        for (let dy = -1; dy <= 1; dy++) {
          if (
            (cells.get(`${x + dx}:${y + dy}`) ?? []).some((existing) =>
              overlaps(position, existing, width, height),
            )
          )
            return true;
        }
      return false;
    };
    for (const node of ordered) {
      const stored = previous.get(node.id);
      if (stored && Number.isFinite(stored.x) && Number.isFinite(stored.y)) {
        positions.set(node.id, { ...stored });
        remember(stored);
      }
    }
    for (const [index, node] of ordered.entries()) {
      if (positions.has(node.id)) continue;
      const parent = positions.get(incoming.get(node.id) ?? "");
      const pos = !branched
        ? linearPosition(index, layout, availableWidth)
        : parent
          ? horizontal
            ? { x: parent.x + width + gapX, y: parent.y + 30 }
            : { x: parent.x + 90, y: parent.y + height + gapY }
          : { x: 0, y: positions.size * (height + gapY) };
      while (occupied(pos)) {
        if (horizontal && branched) pos.y += height + gapY;
        else pos.x += width + gapX;
      }
      positions.set(node.id, pos);
      remember(pos);
    }
    return positions;
  }
  if (!branched) {
    // A long linear session needs no recursive graph algorithm (1000+ turns).
    ordered.forEach((node, index) =>
      positions.set(node.id, linearPosition(index, layout, availableWidth)),
    );
    return positions;
  }
  if (ordered.length <= 300) {
    const model = new dagre.graphlib.Graph()
      .setGraph({
        rankdir: horizontal ? "LR" : "TB",
        nodesep: horizontal ? gapY : gapX,
        ranksep: horizontal ? gapX : gapY,
        marginx: 30,
        marginy: 30,
      })
      .setDefaultEdgeLabel(() => ({}));
    ordered.forEach((node) => model.setNode(node.id, { width, height }));
    links.forEach((edge) => model.setEdge(edge.source, edge.target));
    dagre.layout(model);
    ordered.forEach((node) => {
      const point = model.node(node.id);
      positions.set(node.id, {
        x: point.x - width / 2,
        y: point.y - height / 2,
      });
    });
    // Dagre's equally valid horizontal mirror can put earlier questions on the
    // right. Anchor orientation to the earliest root fork, keeping every edge.
    const nodeById = new Map(ordered.map((node) => [node.id, node]));
    const fork =
      ordered.find(
        (node) => !incoming.has(node.id) && (outgoing.get(node.id) ?? 0) > 1,
      ) ?? ordered.find((node) => (outgoing.get(node.id) ?? 0) > 1);
    if (fork) {
      const axis = horizontal ? "y" : "x";
      const children = links
        .filter((edge) => edge.source === fork.id)
        .map((edge) => nodeById.get(edge.target)!)
        .sort((a, b) => a.ordinal - b.ordinal || a.id.localeCompare(b.id));
      const first = positions.get(children[0].id)!;
      const next = children
        .slice(1)
        .map((node) => positions.get(node.id)!)
        .find((point) => point[axis] !== first[axis]);
      if (next && first[axis] > next[axis]) {
        const coordinates = [...positions.values()].map((point) => point[axis]);
        const extent = Math.min(...coordinates) + Math.max(...coordinates);
        for (const point of positions.values())
          point[axis] = extent - point[axis];
      }
    }
    return positions;
  }
  // Iterative topological ranks avoid recursion limits for very deep branch graphs.
  const indegree = new Map(ordered.map((node) => [node.id, 0]));
  const children = new Map<string, string[]>();
  links.forEach((edge) => {
    indegree.set(edge.target, (indegree.get(edge.target) ?? 0) + 1);
    if (!children.has(edge.source)) children.set(edge.source, []);
    children.get(edge.source)!.push(edge.target);
  });
  const rank = new Map<string, number>();
  const queue = ordered
    .filter((node) => !indegree.get(node.id))
    .map((node) => node.id);
  for (let cursor = 0; cursor < queue.length; cursor++) {
    const id = queue[cursor];
    for (const child of children.get(id) ?? []) {
      rank.set(child, Math.max(rank.get(child) ?? 0, (rank.get(id) ?? 0) + 1));
      indegree.set(child, indegree.get(child)! - 1);
      if (!indegree.get(child)) queue.push(child);
    }
  }
  const slots = new Map<number, number>();
  for (const node of ordered) {
    const level = rank.get(node.id) ?? 0;
    const slot = slots.get(level) ?? 0;
    slots.set(level, slot + 1);
    positions.set(
      node.id,
      horizontal
        ? {
            x: level * (width + gapX),
            y: slot * (height + gapY) + level * 30,
          }
        : {
            x: slot * (width + gapX) + level * 30,
            y: level * (height + gapY),
          },
    );
  }
  return positions;
}

export function initialVisibleIds(graph: QuestionGraph): string[] {
  return [...graph.nodes]
    .sort((a, b) => a.ordinal - b.ordinal)
    .slice(0, 6)
    .map((node) => node.id);
}
