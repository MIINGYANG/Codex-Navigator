import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import {
  Background,
  BackgroundVariant,
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  Handle,
  MiniMap,
  MarkerType,
  Position as HandlePosition,
  ReactFlow,
  ReactFlowProvider,
  applyNodeChanges,
  useNodesInitialized,
  useReactFlow,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeProps,
  type Viewport,
} from "@xyflow/react";
import {
  Circle,
  GitBranch,
  GitCommitHorizontal,
  Home,
  Minimize2,
  Sparkles,
  Star,
} from "lucide-react";
import {
  DEFAULT_LAYOUT,
  compactionEdges,
  layoutMetrics,
  edgePorts,
  highlightedPath,
  initialVisibleIds,
  indexGraph,
  layoutGraph,
  neighborhood,
  visibleCanvasCenter,
  type CanvasLayout,
  type SessionEvent,
  type Position,
  type QuestionGraph,
  type QuestionNode,
} from "./graph";
import "@xyflow/react/dist/style.css";
import "./canvas.css";
import { useTheme } from "./ThemeSwitch";

export interface CanvasHandle {
  fit(): void;
  focus(id: string, neighbors?: boolean): void;
  zoomIn(): void;
  zoomOut(): void;
}
export interface CanvasProps {
  graph: QuestionGraph;
  sessionKey: string;
  generation: number;
  selectedId: string | null;
  onSelect(id: string): void;
  focusPath: boolean;
  onFocusPathChange(value: boolean): void;
  onZoomChange?(percent: number): void;
  layout?: CanvasLayout;
  favoriteOnly?: boolean;
  events?: SessionEvent[];
  onToggleFavorite?(node: QuestionNode): void;
  onEventSelect?(event: SessionEvent): void;
}
type CardData = QuestionNode &
  Record<string, unknown> & {
    root: boolean;
    branching: boolean;
    fresh: boolean;
    delay: number;
    ports: string[];
    compact: boolean;
    onToggleFavorite?: (node: QuestionNode) => void;
    onEventSelect?: (event: SessionEvent) => void;
  };
type CardNode = Node<CardData, "question">;
type TrailEdge = Edge<
  {
    branch: boolean;
    active: boolean;
    label: string;
    events: SessionEvent[];
    onEventSelect?: (event: SessionEvent) => void;
  },
  "trail"
>;

function timestamp(value: string | null): string {
  if (!value) return "时间未记录";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date
    .toLocaleString("zh-CN", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    })
    .replaceAll("/", "-");
}
const QuestionCard = memo(function QuestionCard({
  data,
  selected,
}: NodeProps<CardNode>) {
  const Icon = data.root
    ? Home
    : data.branching
      ? GitBranch
      : data.isLatest
        ? Sparkles
        : Circle;
  return (
    <div
      className={`qt-card${selected ? " is-selected" : ""}${data.fresh ? " is-new" : ""}${data.favorite ? " is-favorite" : ""}${data.compact ? " is-compact" : ""}${data.commits?.length ? " has-commits" : ""}`}
      style={{ animationDelay: `${data.delay}ms` }}
      data-question-id={data.id}
    >
      {(["top", "bottom", "left", "right"] as const).flatMap((side) =>
        (["source", "target"] as const).map((type) => {
          const id = `${type}-${side}`;
          return (
            <Handle
              key={id}
              id={id}
              type={type}
              position={side as HandlePosition}
              isConnectable={false}
              style={{ opacity: data.ports.includes(id) ? 1 : 0 }}
            />
          );
        }),
      )}
      {data.onToggleFavorite && (
        <button
          className="qt-card-favorite nodrag nopan"
          aria-label={`${data.favorite ? "取消收藏" : "收藏"}问题 ${data.ordinal}`}
          aria-pressed={Boolean(data.favorite)}
          title={data.favorite ? "取消收藏" : "收藏问题"}
          onClick={(event) => {
            event.stopPropagation();
            data.onToggleFavorite?.(data);
          }}
          onDoubleClick={(event) => event.stopPropagation()}
        >
          <Star size={15} fill={data.favorite ? "currentColor" : "none"} />
        </button>
      )}
      <span
        className={`qt-card-icon${data.isLatest && !data.root ? " is-latest" : ""}`}
      >
        <Icon size={23} strokeWidth={1.8} />
      </span>
      <div className="qt-card-copy">
        <div className="qt-card-title">
          {data.ordinal}. {data.title || "非文本问题"}
        </div>
        <div className="qt-card-meta">
          <time dateTime={data.timestamp ?? undefined}>
            {timestamp(data.timestamp)}
          </time>
          {data.commits?.length ? (
            <button
              className="qt-card-commit nodrag nopan"
              aria-label={`查看问题 ${data.ordinal} 的 ${data.commits.length} 条提交`}
              title="查看提交的仓库、分支与版本"
              onClick={(event) => {
                event.stopPropagation();
                data.onEventSelect?.(data.commits![0]);
              }}
              onDoubleClick={(event) => event.stopPropagation()}
            >
              <GitCommitHorizontal size={12} />
              {data.commits.length} 提交
            </button>
          ) : (
            <span>
              {data.isLatest ? (
                <>
                  <i />
                  最新
                </>
              ) : (
                `Q${data.ordinal}`
              )}
            </span>
          )}
        </div>
      </div>
    </div>
  );
});
const TrailConnection = memo(function TrailConnection(
  props: EdgeProps<TrailEdge>,
) {
  const [path, labelX, labelY] = getBezierPath(props);
  return (
    <>
      <BaseEdge
        id={props.id}
        path={path}
        style={props.style}
        interactionWidth={20}
        markerEnd={props.markerEnd}
        className={`qt-connection${props.data?.branch ? " is-branch" : " is-sequence"}`}
      />
      {!!props.data?.events.length && (
        <EdgeLabelRenderer>
          <div
            className="qt-compaction-group nodrag nopan"
            style={{
              transform: `translate(-50%, -50%) translate(${labelX}px,${labelY}px)`,
            }}
          >
            {props.data.events.map((event) => (
              <button
                key={event.id}
                className="qt-compaction-marker"
                title={`${event.trigger === "auto" ? "自动压缩" : event.trigger === "manual" ? "手动压缩" : "压缩 · 触发方式未知"} · ${timestamp(event.timestamp)}`}
                aria-label={`查看${event.trigger === "auto" ? "自动" : event.trigger === "manual" ? "手动" : "未知来源"}压缩详情`}
                onClick={(click) => {
                  click.stopPropagation();
                  props.data?.onEventSelect?.(event);
                }}
                onDoubleClick={(click) => click.stopPropagation()}
              >
                <Minimize2 size={12} />
                {event.trigger === "auto"
                  ? "自动压缩"
                  : event.trigger === "manual"
                    ? "手动压缩"
                    : "压缩 · 未知"}
              </button>
            ))}
          </div>
        </EdgeLabelRenderer>
      )}
      {props.data?.active && !props.data.events.length && (
        <EdgeLabelRenderer>
          <div
            className="qt-edge-label"
            style={{
              transform: `translate(-50%, -50%) translate(${labelX}px,${labelY}px)`,
            }}
          >
            {props.data.label}
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
});
const nodeTypes = { question: QuestionCard };
const edgeTypes = { trail: TrailConnection };

const CanvasInner = forwardRef<CanvasHandle, CanvasProps>(function CanvasInner(
  {
    graph,
    sessionKey,
    generation,
    selectedId,
    onSelect,
    focusPath,
    onFocusPathChange,
    onZoomChange,
    layout = DEFAULT_LAYOUT,
    favoriteOnly = false,
    events = [],
    onToggleFavorite,
    onEventSelect,
  },
  ref,
) {
  const { resolved: theme } = useTheme();
  const flow = useReactFlow<CardNode, TrailEdge>();
  const favoriteCallback = useRef(onToggleFavorite);
  const eventCallback = useRef(onEventSelect);
  favoriteCallback.current = onToggleFavorite;
  eventCallback.current = onEventSelect;
  const favoriteEnabled = Boolean(onToggleFavorite);
  const toggleFavorite = useCallback(
    (node: QuestionNode) => favoriteCallback.current?.(node),
    [],
  );
  const selectEvent = useCallback(
    (event: SessionEvent) => eventCallback.current?.(event),
    [],
  );

  const container = useRef<HTMLDivElement>(null);
  const [availableWidth, setAvailableWidth] = useState(670);
  const metrics = layoutMetrics(layout, availableWidth);
  const layoutKey = `${layout.direction}:${layout.density}:${metrics.columns}`;
  const layoutRef = useRef("");
  useLayoutEffect(() => {
    const element = container.current;
    if (!element) return;
    const measure = () =>
      setAvailableWidth(Math.max(0, element.clientWidth - 48));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const focusFrame = useRef<number | undefined>(undefined);
  const initialized = useNodesInitialized();
  const [nodes, setNodes] = useState<CardNode[]>([]);
  const [nodesIdentity, setNodesIdentity] = useState("");
  const graphIndex = useMemo(() => indexGraph(graph), [graph]);
  const [hovered, setHovered] = useState<string | null>(null);
  const [hoverEdge, setHoverEdge] = useState<string | null>(null);
  const [tooltip, setTooltip] = useState<{
    id: string;
    left: number;
    top: number;
  } | null>(null);
  const [localFocus, setLocalFocus] = useState(false);
  const positions = useRef(new Map<string, Position>());
  const identity = `${sessionKey}:${generation}`;
  const identityRef = useRef("");
  const fitPending = useRef(false);
  const tooltipTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const reducedMotion = useRef(false);
  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => {
      reducedMotion.current = query.matches;
    };
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);
  const duration = () => (reducedMotion.current ? 0 : 280);
  const clearHover = useCallback(() => {
    clearTimeout(tooltipTimer.current);
    setHovered(null);
    setTooltip(null);
  }, []);
  useEffect(() => () => clearTimeout(tooltipTimer.current), []);

  useEffect(() => {
    const changed = identityRef.current !== identity;
    const layoutChanged = layoutRef.current !== layoutKey;
    if (changed || layoutChanged) {
      layoutRef.current = layoutKey;
      positions.current = new Map();
      identityRef.current = identity;
      fitPending.current = true;
      setLocalFocus(false);
      clearHover();
    }
    const prior = positions.current;
    positions.current = layoutGraph(graph, prior, layout, availableWidth);
    const usedPorts = new Map<string, Set<string>>();
    for (const edge of graphIndex.edges.values()) {
      const ports = edgePorts(
        positions.current.get(edge.source)!,
        positions.current.get(edge.target)!,
        metrics,
      );
      if (!usedPorts.has(edge.source)) usedPorts.set(edge.source, new Set());
      if (!usedPorts.has(edge.target)) usedPorts.set(edge.target, new Set());
      usedPorts.get(edge.source)!.add(`source-${ports.source}`);
      usedPorts.get(edge.target)!.add(`target-${ports.target}`);
    }
    setNodesIdentity(identity);
    setNodes((current) => {
      const existing = new Map(current.map((node) => [node.id, node]));
      return graph.nodes.map((node, index) => ({
        id: node.id,
        type: "question",
        position: positions.current.get(node.id)!,
        width: metrics.width,
        height: metrics.height,
        measured:
          changed || layoutChanged
            ? undefined
            : existing.get(node.id)?.measured,
        data: {
          ...node,
          compact: layout.density === "compact",
          onToggleFavorite: favoriteEnabled ? toggleFavorite : undefined,
          onEventSelect: selectEvent,
          root: !graphIndex.incoming.has(node.id),
          branching: (graphIndex.outgoing.get(node.id)?.length ?? 0) > 1,
          fresh: !prior.has(node.id),
          ports: [...(usedPorts.get(node.id) ?? [])],
          delay:
            changed && graph.nodes.length <= 15 ? Math.min(index * 25, 100) : 0,
        },
        ariaLabel: `问题 ${node.ordinal}：${node.title}`,
        focusable: true,
      }));
    });
  }, [
    graph,
    graphIndex,
    identity,
    clearHover,
    layoutKey,
    favoriteEnabled,
    toggleFavorite,
    selectEvent,
  ]);

  useEffect(() => {
    if (
      nodesIdentity !== identity ||
      !initialized ||
      !nodes.length ||
      !fitPending.current
    )
      return;
    fitPending.current = false;
    if (selectedId) {
      focusLatest.current(selectedId);
      return;
    }
    void flow.fitView({
      nodes: initialVisibleIds(graph).map((id) => ({ id })),
      padding: 0.12,
      minZoom: 0.35,
      maxZoom: 1,
      duration: 0,
    });
  }, [initialized, nodes, nodesIdentity, identity, graph, flow, selectedId]);

  const fit = useCallback(() => {
    setLocalFocus(false);
    onFocusPathChange(false);
    void flow.fitView({
      padding: 0.16,
      duration: reducedMotion.current ? 0 : 280,
      minZoom: 0.001,
      maxZoom: 1,
    });
  }, [flow, onFocusPathChange]);
  const focus = useCallback(
    (id: string, neighbors = false) => {
      if (focusFrame.current !== undefined)
        cancelAnimationFrame(focusFrame.current);
      // The drawer's DOM must reflect the newly selected question before measuring.
      focusFrame.current = requestAnimationFrame(() => {
        const node = flow.getNode(id);
        const canvasRect = container.current?.getBoundingClientRect();
        if (!node || !canvasRect) return;
        clearHover();
        const drawer = document.querySelector<HTMLElement>(
          ".detail-panel.has-selection",
        );
        const drawerRect =
          drawer && drawer.getClientRects().length
            ? drawer.getBoundingClientRect()
            : null;
        const visible = visibleCanvasCenter(canvasRect, drawerRect);
        const bounds = neighbors
          ? flow.getNodesBounds([...neighborhood(graph, id, graphIndex)])
          : {
              ...node.position,
              width: node.width ?? metrics.width,
              height: node.height ?? metrics.height,
            };
        if (neighbors) setLocalFocus(true);
        const preferredZoom = neighbors
          ? 1
          : Math.max(0.65, Math.min(flow.getZoom(), 1.15));
        const zoom = Math.max(
          0.001,
          Math.min(
            preferredZoom,
            Math.max(20, visible.width - 32) / bounds.width,
            Math.max(20, visible.height - 28) / bounds.height,
          ),
        );
        void flow.setViewport(
          {
            x:
              visible.x -
              canvasRect.left -
              (bounds.x + bounds.width / 2) * zoom,
            y:
              visible.y -
              canvasRect.top -
              (bounds.y + bounds.height / 2) * zoom,
            zoom,
          },
          { duration: reducedMotion.current ? 0 : 280 },
        );
      });
    },
    [flow, graph, graphIndex, clearHover],
  );
  const focusLatest = useRef(focus);
  focusLatest.current = focus;
  useLayoutEffect(() => {
    if (!selectedId) return;
    focusLatest.current(selectedId);
    const element = container.current;
    if (!element) return;
    const geometry = () => {
      const rect = element.getBoundingClientRect();
      const drawer = document.querySelector<HTMLElement>(
        ".detail-panel.has-selection",
      );
      const area = visibleCanvasCenter(
        rect,
        drawer?.getBoundingClientRect() ?? null,
      );
      return [
        rect.left,
        rect.top,
        rect.width,
        rect.height,
        area.left,
        area.top,
        area.width,
        area.height,
      ].join(":");
    };
    let previous = geometry();
    const resize = () => {
      const next = geometry();
      if (next !== previous) {
        previous = next;
        focusLatest.current(selectedId);
      }
    };
    const observer = new ResizeObserver(resize);
    observer.observe(element);
    const drawer = document.querySelector<HTMLElement>(
      ".detail-panel.has-selection",
    );
    if (drawer) observer.observe(drawer);
    window.addEventListener("resize", resize);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", resize);
      if (focusFrame.current !== undefined)
        cancelAnimationFrame(focusFrame.current);
    };
    // Graph revisions intentionally do not re-center a reader's historical viewport.
  }, [selectedId, identity, layoutKey]);
  useEffect(
    () => () => {
      if (focusFrame.current !== undefined)
        cancelAnimationFrame(focusFrame.current);
    },
    [],
  );
  useImperativeHandle(
    ref,
    () => ({
      fit,
      focus,
      zoomIn: () => {
        void flow.zoomIn({ duration: duration() });
      },
      zoomOut: () => {
        void flow.zoomOut({ duration: duration() });
      },
    }),
    [fit, focus, flow],
  );

  const path = useMemo(
    () => highlightedPath(graph, selectedId, graphIndex),
    [graph, graphIndex, selectedId],
  );
  const nearby = useMemo(
    () => (hovered ? neighborhood(graph, hovered, graphIndex) : null),
    [graph, graphIndex, hovered],
  );
  const edgeEndpoints = useMemo(() => {
    const edge = hoverEdge ? graphIndex.edges.get(hoverEdge) : null;
    return edge ? new Set([edge.source, edge.target]) : null;
  }, [graphIndex, hoverEdge]);
  const visibleNodes = useMemo(
    () =>
      nodes.map((node) => ({
        ...node,
        selected: node.id === selectedId,
        style: {
          opacity:
            favoriteOnly && !node.data.favorite && node.id !== selectedId
              ? 0.2
              : focusPath && selectedId && !path.nodes.has(node.id)
                ? 0.22
                : (nearby && !nearby.has(node.id)) ||
                    (edgeEndpoints && !edgeEndpoints.has(node.id))
                  ? 0.55
                  : 1,
        },
      })),
    [nodes, selectedId, focusPath, path, nearby, edgeEndpoints, favoriteOnly],
  );
  const edgeEvents = useMemo(
    () => compactionEdges(graph, events),
    [graph, events],
  );
  const edges = useMemo<TrailEdge[]>(() => {
    return [...graphIndex.edges.values()].map((edge) => {
      const active = edge.id === hoverEdge;
      const adjacent =
        active || edge.source === hovered || edge.target === hovered;
      const emphasized = adjacent || path.edges.has(edge.id);
      const ports = edgePorts(
        positions.current.get(edge.source) ?? { x: 0, y: 0 },
        positions.current.get(edge.target) ?? { x: 0, y: 0 },
        metrics,
      );
      const stroke = emphasized
        ? "var(--qt-accent-strong)"
        : edge.type === "sequence"
          ? "var(--qt-accent)"
          : "var(--qt-branch)";
      return {
        ...edge,
        type: "trail",
        sourceHandle: `source-${ports.source}`,
        targetHandle: `target-${ports.target}`,
        markerEnd:
          edge.type === "sequence"
            ? {
                type: MarkerType.ArrowClosed,
                width: 11,
                height: 11,
                color: stroke,
              }
            : undefined,
        focusable: false,
        selectable: false,
        data: {
          events: edgeEvents.get(edge.id) ?? [],
          onEventSelect: selectEvent,
          branch: edge.type === "branch",
          active,
          label: `Q${graphIndex.nodes.get(edge.source)?.ordinal} → Q${graphIndex.nodes.get(edge.target)?.ordinal}`,
        },
        style: {
          stroke,
          strokeWidth: adjacent ? 2.4 : edge.type === "sequence" ? 1.8 : 1.4,
          opacity:
            focusPath && selectedId && !path.edges.has(edge.id)
              ? 0.22
              : emphasized
                ? 1
                : 0.65,
        },
      };
    });
  }, [
    graphIndex,
    nodes,
    hoverEdge,
    hovered,
    path,
    focusPath,
    selectedId,
    edgeEvents,
    selectEvent,
  ]);
  const tooltipNode = tooltip ? graphIndex.nodes.get(tooltip.id) : null;
  const parents = tooltipNode
    ? (graphIndex.incoming.get(tooltipNode.id) ?? [])
        .map((edge) => graphIndex.nodes.get(edge.source)?.ordinal)
        .filter(Boolean)
    : [];

  return (
    <div
      className="qt-canvas"
      ref={container}
      data-testid="question-canvas"
      onKeyDownCapture={(event) => {
        if (
          event.key !== "Enter" ||
          (event.target as HTMLElement).closest(
            "button, input, textarea, select, a",
          )
        )
          return;
        const target = (event.target as HTMLElement).closest<HTMLElement>(
          ".react-flow__node-question",
        );
        const id = target?.dataset.id;
        if (id) {
          event.preventDefault();
          event.stopPropagation();
          onSelect(id);
          focus(id);
        }
      }}
    >
      <ReactFlow<CardNode, TrailEdge>
        colorMode={theme}
        nodes={visibleNodes}
        edges={edges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodesChange={(changes) =>
          setNodes((current) => {
            for (const change of changes) {
              if (change.type === "position" && change.position)
                positions.current.set(change.id, { ...change.position });
            }
            return applyNodeChanges(changes, current);
          })
        }
        onNodeDragStop={(_event, node) => {
          positions.current.set(node.id, { ...node.position });
        }}
        onNodeDragStart={clearHover}
        onNodeClick={(event, node) => {
          if ((event.target as HTMLElement).closest("button")) return;
          onSelect(node.id);
          focus(node.id);
        }}
        onNodeDoubleClick={(event, node) => {
          if ((event.target as HTMLElement).closest("button")) return;
          onSelect(node.id);
          focus(node.id, true);
        }}
        onNodeMouseEnter={(event, node) => {
          clearTimeout(tooltipTimer.current);
          setHovered(node.id);
          const rect = event.currentTarget.getBoundingClientRect();
          const width = 260;
          const rightFits = rect.right + width + 18 < window.innerWidth;
          const leftFits = rect.left - width - 18 > 0;
          const left = rightFits
            ? rect.right + 14
            : leftFits
              ? rect.left - width - 14
              : Math.max(
                  10,
                  Math.min(window.innerWidth - width - 10, rect.left),
                );
          const top =
            rightFits || leftFits
              ? Math.max(12, Math.min(window.innerHeight - 180, rect.top))
              : Math.max(12, rect.top - 180);
          tooltipTimer.current = setTimeout(
            () => setTooltip({ id: node.id, left, top }),
            300,
          );
        }}
        onNodeMouseLeave={clearHover}
        onMoveStart={clearHover}
        onEdgeMouseEnter={(_event, edge) => setHoverEdge(edge.id)}
        onEdgeMouseLeave={() => setHoverEdge(null)}
        onMove={(_event, viewport: Viewport) =>
          onZoomChange?.(Math.round(viewport.zoom * 100))
        }
        nodesConnectable={false}
        edgesReconnectable={false}
        elementsSelectable
        nodesDraggable
        nodeDragThreshold={5}
        minZoom={0.001}
        maxZoom={1.75}
        zoomOnDoubleClick={false}
        onlyRenderVisibleElements={graph.nodes.length > 300}
        deleteKeyCode={null}
        selectionKeyCode={null}
        multiSelectionKeyCode={null}
        panActivationKeyCode="Space"
        proOptions={{ hideAttribution: true }}
        ariaLabelConfig={{
          "node.a11yDescription.default":
            "按 Enter 查看问题详情。拖动只改变画布位置。",
          "minimap.ariaLabel": "问题轨迹缩略图",
        }}
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={20}
          size={0.8}
          color="var(--qt-grid)"
        />
        <MiniMap
          style={{ width: 160, height: 92 }}
          pannable
          zoomable
          onClick={(_event, point) => {
            clearHover();
            void flow.setCenter(point.x, point.y, {
              zoom: flow.getZoom(),
              duration: reducedMotion.current ? 0 : 280,
            });
          }}
          className="qt-minimap"
          bgColor="var(--qt-panel)"
          nodeColor={(node) =>
            node.id === selectedId
              ? "var(--qt-accent)"
              : node.data.favorite
                ? "var(--qt-favorite, #c18a18)"
                : Array.isArray(node.data.commits) && node.data.commits.length
                  ? "var(--qt-success)"
                  : "var(--qt-minimap-node)"
          }
          maskColor="var(--qt-minimap-mask)"
          nodeStrokeWidth={0}
          nodeBorderRadius={4}
        />
      </ReactFlow>
      {localFocus && (
        <button className="qt-return-map" onClick={fit}>
          回到全图
        </button>
      )}
      <div className="qt-legend" aria-label="关系图例">
        <span>
          <i className="qt-legend-sequence" />
          主线
        </span>
        <span>
          <i className="qt-legend-branch" />
          分支
        </span>
        <span>
          <i className="qt-legend-selected" />
          当前问题
        </span>
      </div>
      {tooltip &&
        tooltipNode &&
        createPortal(
          <div
            role="tooltip"
            className="qt-tooltip"
            style={{ left: tooltip.left, top: tooltip.top }}
          >
            <p>
              {Array.from(tooltipNode.promptPreview || tooltipNode.title)
                .slice(0, 160)
                .join("")}
            </p>
            <time>{timestamp(tooltipNode.timestamp)}</time>
            <span>
              上游：
              {parents.length
                ? parents.map((ordinal) => `Q${ordinal}`).join("、")
                : "起点"}{" "}
              · 下游：
              {graphIndex.outgoing.get(tooltipNode.id)?.length ?? 0} 个问题
            </span>
            <small>点击查看详情</small>
          </div>,
          document.body,
        )}
    </div>
  );
});

export const Canvas = forwardRef<CanvasHandle, CanvasProps>(
  function Canvas(props, ref) {
    return (
      <ReactFlowProvider>
        <CanvasInner {...props} ref={ref} />
      </ReactFlowProvider>
    );
  },
);
export default Canvas;
