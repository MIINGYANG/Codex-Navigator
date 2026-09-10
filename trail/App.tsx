import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Check,
  ChevronRight,
  Copy,
  FileText,
  Focus,
  GitBranch,
  House,
  Info,
  Maximize2,
  Menu,
  MessageCircle,
  Minus,
  PanelLeftClose,
  Plus,
  RefreshCw,
  Route,
  Search,
  ShieldCheck,
  Sparkles,
  X,
} from "lucide-react";
import Canvas, { type CanvasHandle } from "./Canvas";
import ThemeSwitch from "./ThemeSwitch";
import ProjectPath from "./ProjectPath";
import SessionActionDialog, {
  SessionActions,
  type SessionAction,
  type SessionTarget,
} from "./SessionActions";
import type { QuestionGraph, QuestionNode } from "./graph";
import { api, subscribe } from "./api";
import {
  fullTime,
  groupResults,
  reconcileSelection,
  reconcileSessions,
  initialLoadTransition,
  relativeTime,
  titleOf,
  type SearchResult,
  type SessionSummary,
} from "./state";

type GraphResponse = QuestionGraph & {
  key: string;
  meta: { cwd: string | null; id?: string | null; title?: string | null };
  generation: number;
  revision: number;
  loading: boolean;
  error: string | null;
  watch: boolean;
  notices: string[];
  stats?: {
    malformed_records?: number;
    skipped_oversize_records?: number;
    unknown_records?: number;
    omitted_text_bytes?: number;
  };
};
type Activity = {
  index: number;
  type: string;
  text?: string;
  name?: string;
  summary?: string;
  path?: string;
  is_error?: boolean;
};
type Detail = {
  generation: number;
  revision: number;
  turn: {
    index: number;
    ordinal: number;
    revision: number;
    prompt: {
      text: string;
      preview: string;
      images_count: number;
      omitted_bytes: number;
    };
    started_at?: string;
  };
  items: Activity[];
  items_total: number;
  next_offset: number | null;
  final_answer: { text: string; phase: string | null } | null;
};
const emptyGraph: QuestionGraph = { nodes: [], edges: [] };

function useEscape(callback: () => void) {
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") callback();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [callback]);
}

export default function App() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sessionsLoading, setSessionsLoading] = useState(true);
  const [sessionError, setSessionError] = useState("");
  const [key, setKey] = useState<string | null>(null);
  const [graph, setGraph] = useState<GraphResponse | null>(null);
  const [graphError, setGraphError] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [focusPath, setFocusPath] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [sessionQuery, setSessionQuery] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [zoom, setZoom] = useState(100);
  const [seen, setSeen] = useState(0);
  const [connected, setConnected] = useState(false);
  const [watch, setWatch] = useState(true);
  const [toast, setToast] = useState("");
  const [sessionAction, setSessionAction] = useState<{
    session: SessionTarget;
    action: SessionAction;
  } | null>(null);
  const renamedSessions = useRef(new Map<string, string>());
  const renamedGraphs = useRef(new Map<string, string>());
  const deletedSessions = useRef(new Set<string>());
  const canvas = useRef<CanvasHandle>(null);
  const graphRef = useRef<GraphResponse | null>(null);
  const requestedNode = useRef<{ key: string; id: string } | null>(null);
  const refreshGraph = useRef<() => void>(() => {});
  const refreshSessions = useRef<() => void>(() => {});
  const initialSelection = useRef(false);
  const currentSession = sessions.find((session) => session.key === key);
  const selected = graph?.nodes.find((node) => node.id === selectedId) || null;
  const pending = Math.max(0, (graph?.nodes.length || 0) - seen);

  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let busy = false;
    let rescanPending = false;
    let configured = false;
    let autoWatch = true;
    async function scan(rescan = false) {
      if (controller.signal.aborted) return;
      if (busy) {
        rescanPending ||= rescan;
        return;
      }
      busy = true;
      clearTimeout(timer);
      let delay = 15000;
      try {
        if (!configured) {
          const info = await api<{
            watch: boolean;
            initial_session: string | null;
          }>("/api/info", controller.signal);
          if (controller.signal.aborted) return;
          autoWatch = info.watch;
          setWatch(info.watch);
          if (info.initial_session) {
            initialSelection.current = true;
            setKey(info.initial_session);
          }
          configured = true;
        }
        const data = await api<{
          loading: boolean;
          error: string | null;
          sessions: SessionSummary[];
        }>(
          `/api/sessions?all=1${rescan ? "&refresh=1" : ""}`,
          controller.signal,
        );
        if (controller.signal.aborted) return;
        for (const session of data.sessions) {
          if (renamedSessions.current.get(session.key) === session.title)
            renamedSessions.current.delete(session.key);
        }
        const nextSessions = reconcileSessions(
          data.sessions,
          renamedSessions.current,
          deletedSessions.current,
        );
        setSessions(nextSessions);
        setSessionsLoading(data.loading);
        setSessionError(data.error || "");
        if (data.loading) delay = 500;
        if (!initialSelection.current && nextSessions.length) {
          initialSelection.current = true;
          setKey(nextSessions[0].key);
        }
      } catch (error) {
        if (!controller.signal.aborted) {
          setSessionError(String((error as Error).message));
          setSessionsLoading(false);
        }
        delay = 3000;
      } finally {
        busy = false;
        if (rescanPending && !controller.signal.aborted) {
          rescanPending = false;
          void scan(true);
          return;
        }
        if (!controller.signal.aborted)
          timer = setTimeout(
            () => void scan(autoWatch && delay === 15000),
            delay,
          );
      }
    }
    refreshSessions.current = () => void scan(true);
    void scan();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let busy = false;
    let again = false;
    let initializing = true;
    let manualPending = false;
    graphRef.current = null;
    setGraph(null);
    setGraphError("");
    setSelectedId(null);
    setFocusPath(false);
    setSeen(0);
    setConnected(false);
    if (!key) return;
    const sessionKey = key;
    async function refresh(manual = false) {
      if (controller.signal.aborted) return;
      manualPending ||= manual;
      if (busy) {
        again = true;
        return;
      }
      busy = true;
      clearTimeout(timer);
      try {
        const suffix = manualPending ? "?refresh=1" : "";
        manualPending = false;
        const next = await api<GraphResponse>(
          `/api/trail/session/${encodeURIComponent(sessionKey)}${suffix}`,
          controller.signal,
        );
        if (
          controller.signal.aborted ||
          deletedSessions.current.has(sessionKey)
        )
          return;
        const renamed = renamedGraphs.current.get(sessionKey);
        if (renamed !== undefined) {
          if (next.meta.title === renamed)
            renamedGraphs.current.delete(sessionKey);
          else next.meta = { ...next.meta, title: renamed };
        }
        const previous = graphRef.current;
        const changed =
          !previous ||
          previous.generation !== next.generation ||
          previous.revision !== next.revision ||
          previous.loading !== next.loading ||
          previous.error !== next.error ||
          previous.meta.title !== next.meta.title;
        if (changed) {
          graphRef.current = next;
          setGraph(next);
          setSelectedId((value) =>
            reconcileSelection(
              value,
              new Set(next.nodes.map((node) => node.id)),
              previous?.generation,
              next.generation,
            ),
          );
          const initial = initialLoadTransition(
            initializing,
            previous?.generation,
            next.generation,
            next.loading,
          );
          initializing = initial.initializing;
          if (initial.resetSeen) setSeen(next.nodes.length);
        }
        setGraphError("");
        if (requestedNode.current?.key === sessionKey) {
          const id = requestedNode.current.id;
          if (next.nodes.some((node) => node.id === id)) {
            setSelectedId(id);
            requestedNode.current = null;
            requestAnimationFrame(() => canvas.current?.focus(id));
          } else if (!next.loading) {
            requestedNode.current = null;
            setToast("该问题已不在当前会话快照中，请重新搜索。");
          }
        }
        // SSE carries revisions; polling is only a loading/error recovery safety net.
        timer = setTimeout(() => void refresh(), next.loading ? 350 : 10000);
      } catch (error) {
        if (!controller.signal.aborted) {
          setGraphError((error as Error).message);
          timer = setTimeout(() => void refresh(), 3000);
        }
      } finally {
        busy = false;
        if (again && !controller.signal.aborted) {
          again = false;
          void refresh();
        }
      }
    }
    refreshGraph.current = () => void refresh(true);
    void refresh();
    if (watch)
      void subscribe(
        key,
        controller.signal,
        () => void refresh(),
        setConnected,
      );
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [key, watch]);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(""), 3500);
    return () => clearTimeout(timer);
  }, [toast]);

  const selectNode = useCallback((id: string) => {
    setSelectedId(id);
    const node = graphRef.current?.nodes.find((item) => item.id === id);
    if (node?.isLatest) setSeen(graphRef.current!.nodes.length);
  }, []);
  const navigate = useCallback(
    (id: string) => {
      selectNode(id);
      canvas.current?.focus(id);
    },
    [selectNode],
  );
  const closeOverlays = useCallback(() => {
    if (sessionAction) return;
    if (searchOpen) setSearchOpen(false);
    else if (sidebarOpen) setSidebarOpen(false);
    else if (selectedId) {
      setSelectedId(null);
      setFocusPath(false);
    } else if (expanded) setExpanded(false);
  }, [searchOpen, sidebarOpen, selectedId, expanded, sessionAction]);
  useEscape(closeOverlays);

  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (sessionAction) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearchOpen(true);
        return;
      }
      if (
        searchOpen ||
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        (event.target as HTMLElement)?.closest(
          "input,textarea,[contenteditable=true]",
        )
      )
        return;
      if (event.key.toLowerCase() === "f") {
        event.preventDefault();
        setFocusPath(false);
        canvas.current?.fit();
      }
      if (
        selected &&
        graph &&
        ["ArrowUp", "ArrowDown"].includes(event.key) &&
        (event.target as HTMLElement)?.closest(".detail-panel")
      ) {
        event.preventDefault();
        const index = graph.nodes.findIndex((node) => node.id === selected.id);
        const next = graph.nodes[index + (event.key === "ArrowDown" ? 1 : -1)];
        if (next) navigate(next.id);
      }
    };
    window.addEventListener("keydown", handle);
    return () => window.removeEventListener("keydown", handle);
  }, [graph, selected, searchOpen, navigate, sessionAction]);

  function manage(session: SessionTarget, action: SessionAction) {
    setSearchOpen(false);
    setSessionAction({ session, action });
  }
  function renamed(sessionKey: string, name: string) {
    renamedSessions.current.set(sessionKey, name);
    renamedGraphs.current.set(sessionKey, name);
    setSessions((items) =>
      reconcileSessions(
        items,
        renamedSessions.current,
        deletedSessions.current,
      ),
    );
    if (graphRef.current?.key === sessionKey) {
      const next = {
        ...graphRef.current,
        meta: { ...graphRef.current.meta, title: name },
      };
      graphRef.current = next;
      setGraph(next);
    }
    refreshSessions.current();
    setToast("会话名称已同步到 Codex");
  }
  function trashed(sessionKey: string) {
    deletedSessions.current.add(sessionKey);
    renamedSessions.current.delete(sessionKey);
    renamedGraphs.current.delete(sessionKey);
    const remaining = sessions.filter((session) => session.key !== sessionKey);
    setSessions(remaining);
    setSearchOpen(false);
    if (key === sessionKey) {
      requestedNode.current = null;
      graphRef.current = null;
      setGraph(null);
      setSelectedId(null);
      setFocusPath(false);
      setKey(remaining[0]?.key || null);
    }
    refreshSessions.current();
    setToast("会话文件已移到系统回收站");
  }

  function chooseSession(next: string) {
    requestedNode.current = null;
    setKey(next);
    setSidebarOpen(false);
  }
  function chooseResult(result: SearchResult) {
    setSearchOpen(false);
    setSidebarOpen(false);
    if (
      key === result.sessionKey &&
      graph?.nodes.some((node) => node.id === result.nodeId)
    )
      navigate(result.nodeId);
    else {
      requestedNode.current = { key: result.sessionKey, id: result.nodeId };
      setKey(result.sessionKey);
      refreshGraph.current();
    }
  }
  const visibleSessions = sessions.filter((session) =>
    `${titleOf(session)} ${session.cwd || ""}`
      .toLocaleLowerCase()
      .includes(sessionQuery.trim().toLocaleLowerCase()),
  );
  const warnings = [...(graph?.notices || [])];
  const currentTitle = currentSession
    ? titleOf(currentSession)
    : graph?.meta.title?.trim() ||
      graph?.nodes[0]?.title ||
      graph?.meta.id ||
      "把问题串起来，看清来路。";
  if (graph?.stats?.malformed_records)
    warnings.push(`${graph.stats.malformed_records} 条损坏记录已跳过`);
  if (graph?.stats?.skipped_oversize_records)
    warnings.push(`${graph.stats.skipped_oversize_records} 条超大记录已跳过`);
  if (graph?.stats?.unknown_records)
    warnings.push(`${graph.stats.unknown_records} 条未知类型记录未展示`);
  if (graph?.stats?.omitted_text_bytes)
    warnings.push("部分长文本受内存预算限制，详情中标明省略");

  return (
    <div
      className={`app-shell ${sidebarCollapsed ? "sidebar-collapsed" : ""} ${sidebarOpen ? "sidebar-open" : ""} ${expanded ? "canvas-expanded" : ""}`}
    >
      {sidebarOpen && (
        <button
          className="sidebar-scrim"
          aria-label="关闭会话列表"
          onClick={() => setSidebarOpen(false)}
        />
      )}
      <aside className="session-sidebar" aria-label="会话列表">
        <div className="brand">
          <span className="brand-mark">
            <Route size={23} />
          </span>
          <div>
            <strong>Codex Navigator</strong>
            <span>每个问题，都有来路</span>
          </div>
          <button
            className="icon-button mobile-close"
            aria-label="关闭会话列表"
            onClick={() => setSidebarOpen(false)}
          >
            <X size={18} />
          </button>
        </div>
        <div className="sidebar-label">
          <span>
            我的会话 <small>{sessions.length}</small>
          </span>
          <button
            className="icon-button"
            title="重新扫描会话"
            aria-label="重新扫描会话"
            onClick={() => refreshSessions.current()}
          >
            <RefreshCw
              size={15}
              className={sessionsLoading ? "spinning" : ""}
            />
          </button>
        </div>
        <label className="session-filter">
          <Search size={15} />
          <input
            aria-label="筛选会话"
            placeholder="查找会话或项目…"
            value={sessionQuery}
            onChange={(event) => setSessionQuery(event.target.value)}
          />
          {sessionQuery && (
            <button
              aria-label="清除会话筛选"
              onClick={() => setSessionQuery("")}
            >
              <X size={14} />
            </button>
          )}
        </label>
        <nav className="session-list" aria-label="选择会话">
          {sessionsLoading && !sessions.length && (
            <div className="skeleton-list" aria-label="正在查找会话">
              {[0, 1, 2, 3].map((id) => (
                <div className="skeleton" key={id} />
              ))}
            </div>
          )}
          {visibleSessions.map((session) => (
            <div className="session-row" key={session.key}>
              <button
                className={`session-item ${session.key === key ? "active" : ""}`}
                aria-current={session.key === key ? "true" : undefined}
                onClick={() => chooseSession(session.key)}
                title={`${titleOf(session)}\n${session.cwd || "项目未记录"}`}
              >
                <MessageCircle size={17} />
                <span className="session-copy">
                  <strong>{titleOf(session)}</strong>
                  <span>
                    {session.key === key && graph
                      ? `${graph.nodes.length} 个问题`
                      : session.turn_count !== null
                        ? `${session.turn_count} 个问题`
                        : "等待索引"}
                    <i>·</i>
                    {relativeTime(session.updated_at)}
                  </span>
                  <small>{session.cwd || "项目路径未记录"}</small>
                </span>
                <ChevronRight size={14} />
              </button>
              <SessionActions
                session={{
                  key: session.key,
                  title: titleOf(session),
                  cwd: session.cwd,
                }}
                onAction={manage}
              />
            </div>
          ))}
          {!sessionsLoading && !visibleSessions.length && (
            <p className="quiet-empty">
              {sessionQuery ? "没有匹配的会话" : "还没有发现主会话"}
            </p>
          )}
        </nav>
        <div className="sidebar-bottom">
          <div>
            <ShieldCheck size={16} />
            <span>只在本机 · 零额外 Token</span>
          </div>
          <p>
            仅呈现问题与已记录的关系
            <br />
            不读取或重建隐藏推理
          </p>
          <button
            className="subtle-button collapse-control"
            onClick={() => setSidebarCollapsed(true)}
          >
            <PanelLeftClose size={15} />
            收起侧栏
          </button>
          <span className="version">Codex Navigator 3.0</span>
        </div>
      </aside>

      <div className="workspace">
        <header className="topbar">
          <button
            className="icon-button sidebar-toggle"
            aria-label="打开会话列表"
            onClick={() => {
              setSidebarCollapsed(false);
              setSidebarOpen(true);
            }}
          >
            <Menu size={20} />
          </button>
          <button
            className="global-search"
            onClick={() => setSearchOpen(true)}
            aria-label="搜索所有问题"
          >
            <Search size={19} />
            <span>搜索你的问题，找回每一步…</span>
            <kbd>⌘ / Ctrl K</kbd>
          </button>
          <div className="topbar-caption">
            <GitBranch size={17} />
            <span>问题脉络</span>
          </div>
          <ThemeSwitch />
          <div
            className={`local-status ${connected ? "connected" : ""}`}
            title={
              !watch
                ? "自动监控已关闭，点击刷新读取新增内容"
                : connected
                  ? "本机服务实时连接中"
                  : "正在连接本机服务；已加载内容仍可阅读"
            }
          >
            <span />
            {!watch ? "手动刷新" : connected ? "本地实时" : "连接中"}
          </div>
        </header>

        <div className="workspace-body">
          <main className="canvas-panel" aria-label="问题脉络画布">
            <div className="canvas-heading">
              <div className="heading-copy">
                <div className="breadcrumb">
                  我的会话 <ChevronRight size={12} />
                  <span>问题脉络</span>
                </div>
                <div className="session-heading-title">
                  <h1 title={currentTitle}>{currentTitle}</h1>
                  {key && (currentSession || graph) && (
                    <SessionActions
                      session={{
                        key,
                        title: currentTitle,
                        cwd: graph?.meta.cwd ?? currentSession?.cwd ?? null,
                      }}
                      onAction={manage}
                    />
                  )}
                </div>
                <p>
                  {graph ? `${graph.nodes.length} 个问题` : "本地 Codex 会话"}
                  <span>·</span>连线仅表示记录中的顺序与分支
                </p>
                {key && (
                  <ProjectPath
                    key={key}
                    cwd={
                      graph?.key === key ? graph.meta.cwd : currentSession?.cwd
                    }
                    notify={setToast}
                  />
                )}
              </div>
              <button
                className="icon-button"
                aria-label="刷新当前会话"
                title="刷新当前会话"
                onClick={() => {
                  refreshGraph.current();
                  refreshSessions.current();
                }}
              >
                <RefreshCw
                  size={17}
                  className={graph?.loading ? "spinning" : ""}
                />
              </button>
            </div>
            <div
              className="canvas-toolbar"
              role="toolbar"
              aria-label="画布操作"
            >
              <div className="toolbar-group">
                <button
                  onClick={() => {
                    setFocusPath(false);
                    canvas.current?.fit();
                  }}
                  disabled={!graph?.nodes.length}
                >
                  <House size={15} />
                  <span>回到主线</span>
                </button>
                <button
                  className={focusPath ? "active" : ""}
                  aria-pressed={focusPath}
                  disabled={!selected}
                  onClick={() => setFocusPath(!focusPath)}
                >
                  <Focus size={15} />
                  <span>聚焦路径</span>
                </button>
              </div>
              <div className="toolbar-group zoom-tools">
                <button
                  aria-label="缩小"
                  onClick={() => canvas.current?.zoomOut()}
                >
                  <Minus size={16} />
                </button>
                <output aria-label="缩放比例">{zoom}%</output>
                <button
                  aria-label="放大"
                  onClick={() => canvas.current?.zoomIn()}
                >
                  <Plus size={16} />
                </button>
                <span className="toolbar-divider" />
                <button
                  aria-label={expanded ? "退出画布全屏" : "画布全屏"}
                  title={expanded ? "退出画布全屏" : "画布全屏"}
                  aria-pressed={expanded}
                  onClick={() => setExpanded(!expanded)}
                >
                  <Maximize2 size={16} />
                </button>
              </div>
            </div>
            {(graphError ||
              graph?.error ||
              sessionError ||
              warnings.length > 0) && (
              <div className="data-notice" role="status">
                <Info size={15} />
                <span>
                  {graphError ||
                    graph?.error ||
                    sessionError ||
                    warnings.join("；")}
                </span>
                {(graphError || sessionError) && (
                  <button
                    onClick={() => {
                      refreshSessions.current();
                      refreshGraph.current();
                    }}
                  >
                    重试
                  </button>
                )}
              </div>
            )}
            <div className="canvas-stage">
              {graph?.nodes.length ? (
                <Canvas
                  ref={canvas}
                  graph={graph}
                  sessionKey={key!}
                  generation={graph.generation}
                  selectedId={selectedId}
                  onSelect={selectNode}
                  focusPath={focusPath}
                  onFocusPathChange={setFocusPath}
                  onZoomChange={setZoom}
                />
              ) : graph?.loading ||
                (key && !graph && !graphError) ||
                sessionsLoading ? (
                <div className="graph-loading" role="status">
                  <div className="skeleton ghost-card" />
                  <div className="skeleton ghost-card" />
                  <div className="skeleton ghost-card" />
                  <p>正在串起你的问题…</p>
                </div>
              ) : (
                <div className="canvas-empty">
                  <span>
                    <Route size={36} />
                  </span>
                  <h2>
                    {key
                      ? "这个会话还没有用户问题"
                      : "你的下一段思路，从一个问题开始"}
                  </h2>
                  <p>
                    {key
                      ? "系统记录与工具活动不会变成问题节点。\n新问题记录后会自动出现在这里。"
                      : "先在 Codex 中发起一次对话，然后回到这里。\n我们只读本机会话，不需要登录或 API Key。"}
                  </p>
                  <button
                    className="primary-button"
                    onClick={() => {
                      refreshSessions.current();
                      refreshGraph.current();
                    }}
                  >
                    <RefreshCw size={16} />
                    重新扫描
                  </button>
                  <code>codex-nav doctor</code>
                </div>
              )}
              {pending > 0 && (
                <button
                  className="new-questions"
                  onClick={() => {
                    const latest =
                      graph?.nodes.find((node) => node.isLatest) ||
                      graph?.nodes.at(-1);
                    if (latest) navigate(latest.id);
                    setSeen(graph?.nodes.length || 0);
                  }}
                >
                  <Sparkles size={16} />
                  {pending} 个新问题
                  <ArrowDown size={15} />
                </button>
              )}
            </div>
          </main>
          {!expanded && (
            <DetailPanel
              key={`${key}:${graph?.generation}:${selectedId}`}
              sessionKey={key}
              node={selected}
              graph={graph || emptyGraph}
              revision={graph?.revision || 0}
              onNavigate={navigate}
              onClose={() => {
                setSelectedId(null);
                setFocusPath(false);
              }}
              onFocus={() => {
                setFocusPath(true);
                if (selectedId) canvas.current?.focus(selectedId);
              }}
              notify={setToast}
            />
          )}
        </div>
      </div>
      {searchOpen && (
        <SearchDialog
          onClose={() => setSearchOpen(false)}
          onChoose={chooseResult}
        />
      )}
      {sessionAction && (
        <SessionActionDialog
          session={sessionAction.session}
          action={sessionAction.action}
          onClose={() => setSessionAction(null)}
          onRenamed={renamed}
          onTrashed={trashed}
        />
      )}
      {toast && (
        <div className="toast" role="status">
          <Check size={16} />
          {toast}
        </div>
      )}
    </div>
  );
}

function DetailPanel({
  sessionKey,
  node,
  graph,
  revision,
  onNavigate,
  onClose,
  onFocus,
  notify,
}: {
  sessionKey: string | null;
  node: QuestionNode | null;
  graph: QuestionGraph;
  revision: number;
  onNavigate: (id: string) => void;
  onClose: () => void;
  onFocus: () => void;
  notify: (message: string) => void;
}) {
  const [detail, setDetail] = useState<Detail | null>(null);
  const [error, setError] = useState("");
  const [raw, setRaw] = useState(false);
  const [offset, setOffset] = useState(0);
  const [retry, setRetry] = useState(0);
  const [copyFallback, setCopyFallback] = useState(false);
  const panel = useRef<HTMLElement>(null);
  const fallback = useRef<HTMLTextAreaElement>(null);
  const nodeIndex = graph.nodes.findIndex((item) => item.id === node?.id);
  const previous = graph.nodes[nodeIndex - 1];
  const next = graph.nodes[nodeIndex + 1];
  const branches = graph.edges
    .filter((edge) => edge.type === "branch" && edge.source === node?.id)
    .map((edge) => graph.nodes.find((item) => item.id === edge.target))
    .filter((item): item is QuestionNode => Boolean(item));

  useEffect(() => {
    if (!node || !sessionKey) return;
    const controller = new AbortController();
    void api<Detail>(
      `/api/session/${encodeURIComponent(sessionKey)}/turn/${node.turnIndex}?offset=${offset}&limit=8`,
      controller.signal,
    )
      .then((data) => {
        if (!controller.signal.aborted) {
          setDetail((previous) =>
            previous?.generation === data.generation &&
            previous.turn.revision === data.turn.revision &&
            previous.items[0]?.index === data.items[0]?.index
              ? previous
              : data,
          );
          setError("");
        }
      })
      .catch((error: Error) => {
        if (!controller.signal.aborted) setError(error.message);
      });
    return () => controller.abort();
  }, [sessionKey, node?.id, node?.turnIndex, revision, offset, retry]);
  useEffect(() => {
    if (node) panel.current?.focus({ preventScroll: true });
  }, [node?.id]);
  useEffect(() => {
    if (copyFallback) {
      fallback.current?.focus();
      fallback.current?.select();
    }
  }, [copyFallback]);

  async function copy() {
    if (!detail) return;
    try {
      await navigator.clipboard.writeText(
        detail.turn.prompt.text || detail.turn.prompt.preview,
      );
      notify(
        detail.turn.prompt.omitted_bytes
          ? "已复制已保留的问题内容（含省略）"
          : "已复制完整问题",
      );
    } catch {
      setCopyFallback(true);
    }
  }
  return (
    <aside
      ref={panel}
      tabIndex={-1}
      className={`detail-panel ${node ? "has-selection" : ""}`}
      aria-label="问题详情"
    >
      <header className="detail-header">
        <h2>问题详情</h2>
        {node && (
          <button
            className="icon-button"
            aria-label="关闭问题详情"
            onClick={onClose}
          >
            <X size={18} />
          </button>
        )}
      </header>
      {!node ? (
        <div className="detail-empty">
          <span>
            <MessageCircle size={29} />
          </span>
          <h3>从一个问题，回看整段探索</h3>
          <p>
            点击画布上的节点
            <br />
            查看完整问题、前后关系和原始对话
          </p>
          <div>
            <span>单击</span>查看问题<span>双击</span>聚焦相邻问题
          </div>
          <small>展示已记录的对话，而非隐藏思维链</small>
        </div>
      ) : (
        <>
          <div className="detail-tabs" role="tablist" aria-label="详情内容">
            <button
              role="tab"
              aria-selected={!raw}
              onClick={() => setRaw(false)}
            >
              问题详情
            </button>
            <button role="tab" aria-selected={raw} onClick={() => setRaw(true)}>
              原始对话
            </button>
          </div>
          <div className="detail-scroll">
            <div className="question-heading">
              <span className="question-icon">
                <MessageCircle size={20} />
              </span>
              <h3>
                {node.ordinal}. {node.title}
              </h3>
            </div>
            <div className="question-time">{fullTime(node.timestamp)}</div>
            {error && (
              <div className="detail-error" role="alert">
                {error}
                <button onClick={() => setRetry(retry + 1)}>重试</button>
              </div>
            )}
            {!detail && !error && (
              <div
                className="skeleton detail-skeleton"
                role="status"
                aria-label="正在读取问题原文"
              />
            )}
            <section className="detail-section">
              <h4>{raw ? "用户问题" : "完整问题"}</h4>
              <div className="prompt-text">
                {detail?.turn.prompt.text ||
                  detail?.turn.prompt.preview ||
                  node.promptPreview}
              </div>
              {Boolean(detail?.turn.prompt.images_count) && (
                <p className="content-notice">
                  包含 {detail!.turn.prompt.images_count} 张图片；不加载图片或
                  base64 内容。
                </p>
              )}
              {Boolean(detail?.turn.prompt.omitted_bytes) && (
                <p className="content-notice">
                  内存预算省略了{" "}
                  {detail!.turn.prompt.omitted_bytes.toLocaleString()}{" "}
                  字节。这里显示已保留文本，不代表完整原文。
                </p>
              )}
            </section>
            {raw ? (
              <>
                <section className="detail-section">
                  <h4>
                    最终回复{" "}
                    <small>
                      {detail?.final_answer?.phase === "final_answer"
                        ? "已明确标记"
                        : detail?.final_answer
                          ? "末条助手消息 · 未标记 final"
                          : ""}
                    </small>
                  </h4>
                  <div className="answer-text">
                    {detail?.final_answer?.text ||
                      "本地记录中尚无可定位的最终回复。"}
                  </div>
                </section>
                <section className="detail-section">
                  <details className="activity-disclosure">
                    <summary>
                      已记录的活动 <span>{detail?.items_total || 0} 条</span>
                    </summary>
                    <p className="content-notice">
                      仅展示已保留的公开消息和工具摘要，不包含隐藏推理。
                    </p>
                    {detail?.items.map((item) => (
                      <details className="activity-item" key={item.index}>
                        <summary>
                          {item.index + 1}.{" "}
                          {item.name ||
                            (
                              {
                                agent_message: "助手消息",
                                tool_call: "工具调用",
                                tool_output: "工具结果",
                                file_activity: "文件活动",
                                notice: "提示",
                                omitted: "省略记录",
                              } as Record<string, string>
                            )[item.type] ||
                            "其他记录"}
                          {item.is_error && <span>执行警告</span>}
                        </summary>
                        <pre>
                          {item.text ||
                            item.summary ||
                            item.path ||
                            "正文未保留"}
                        </pre>
                      </details>
                    ))}
                    <div className="activity-pagination">
                      <button
                        disabled={!offset}
                        onClick={() => setOffset(Math.max(0, offset - 8))}
                      >
                        <ArrowLeft size={14} />
                        上一组
                      </button>
                      <span>
                        {detail?.items_total
                          ? `${offset + 1}–${Math.min(offset + 8, detail.items_total)}`
                          : "0"}
                      </span>
                      <button
                        disabled={detail?.next_offset == null}
                        onClick={() => setOffset(detail!.next_offset!)}
                      >
                        下一组
                        <ArrowRight size={14} />
                      </button>
                    </div>
                  </details>
                </section>
              </>
            ) : (
              <section className="detail-section relations">
                <h4>问题关系</h4>
                {previous ? (
                  <button onClick={() => onNavigate(previous.id)}>
                    <ArrowUp size={16} />
                    <span>
                      <small>上一个问题 · 记录顺序</small>
                      <strong>{previous.title}</strong>
                    </span>
                    <ChevronRight size={14} />
                  </button>
                ) : (
                  <p className="relation-boundary">
                    <House size={15} />
                    这是会话的第一个问题
                  </p>
                )}
                {next ? (
                  <button onClick={() => onNavigate(next.id)}>
                    <ArrowDown size={16} />
                    <span>
                      <small>下一个问题 · 记录顺序</small>
                      <strong>{next.title}</strong>
                    </span>
                    <ChevronRight size={14} />
                  </button>
                ) : (
                  <p className="relation-boundary">
                    <Check size={15} />
                    已到最新记录
                  </p>
                )}
                {branches.length > 0 && (
                  <div className="branch-relations">
                    <h5>
                      <GitBranch size={15} />
                      直接分支（{branches.length}）
                    </h5>
                    {branches.map((branch) => (
                      <button
                        key={branch.id}
                        onClick={() => onNavigate(branch.id)}
                      >
                        <span>{branch.title}</span>
                        <ChevronRight size={14} />
                      </button>
                    ))}
                  </div>
                )}
                <p className="relationship-note">
                  关系来自记录顺序或明确的父问题字段，不推断主题、因果或结果正确性。
                </p>
              </section>
            )}
            {copyFallback && (
              <section className="copy-fallback">
                <label htmlFor="copy-question">剪贴板不可用，请手动复制</label>
                <textarea
                  id="copy-question"
                  ref={fallback}
                  readOnly
                  value={
                    detail?.turn.prompt.text ||
                    detail?.turn.prompt.preview ||
                    ""
                  }
                />
                <button onClick={() => setCopyFallback(false)}>收起</button>
              </section>
            )}
            <div className="detail-actions">
              <button className="blue-button" onClick={onFocus}>
                <Focus size={16} />
                聚焦此路径
              </button>
              <button disabled={!detail} onClick={() => void copy()}>
                <Copy size={15} />
                复制问题
              </button>
              <button className="raw-button" onClick={() => setRaw(!raw)}>
                <FileText size={16} />
                {raw ? "返回问题详情" : "查看原始 Turn"}
                <ChevronRight size={14} />
              </button>
            </div>
          </div>
          <footer className="detail-navigation">
            <button
              disabled={!previous}
              onClick={() => previous && onNavigate(previous.id)}
            >
              <ArrowLeft size={16} />
              上一个
            </button>
            <span>
              {nodeIndex + 1} / {graph.nodes.length}
            </span>
            <button
              disabled={!next}
              onClick={() => next && onNavigate(next.id)}
            >
              下一个
              <ArrowRight size={16} />
            </button>
          </footer>
        </>
      )}
    </aside>
  );
}

function SearchDialog({
  onClose,
  onChoose,
}: {
  onClose: () => void;
  onChoose: (result: SearchResult) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const [error, setError] = useState("");
  const [active, setActive] = useState(0);
  const ordered = groupResults(results).flatMap(([, group]) => group.results);
  useEffect(() => {
    dialog.current?.showModal();
    input.current?.focus();
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    setResults([]);
    setActive(0);
    setError("");
    setTruncated(false);
    if (!query.trim()) {
      setLoading(false);
      return () => controller.abort();
    }
    setLoading(true);
    async function search() {
      try {
        const data = await api<{
          results: SearchResult[];
          loading: boolean;
          truncated?: boolean;
        }>(
          `/api/trail/search?q=${encodeURIComponent(query.trim())}`,
          controller.signal,
        );
        if (controller.signal.aborted) return;
        setResults(data.results);
        setLoading(data.loading);
        setTruncated(Boolean(data.truncated));
        if (data.loading) timer = setTimeout(() => void search(), 600);
      } catch (error) {
        if (!controller.signal.aborted) {
          setError((error as Error).message);
          setLoading(false);
        }
      }
    }
    timer = setTimeout(() => void search(), 180);
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [query]);
  useEffect(() => {
    dialog.current
      ?.querySelector(`[data-result-index="${active}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [active]);
  return (
    <dialog
      ref={dialog}
      className="search-dialog"
      aria-label="搜索所有会话中的问题"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (event.target === dialog.current) onClose();
      }}
    >
      <div
        className="search-dialog-inner"
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            setActive((index) =>
              Math.max(
                0,
                Math.min(
                  ordered.length - 1,
                  index + (event.key === "ArrowDown" ? 1 : -1),
                ),
              ),
            );
          }
          if (
            event.key === "Enter" &&
            event.target === input.current &&
            ordered[active]
          ) {
            event.preventDefault();
            onChoose(ordered[active]);
          }
        }}
      >
        <div className="search-dialog-input">
          <Search size={21} />
          <input
            ref={input}
            aria-label="搜索问题原文"
            placeholder="搜索所有会话中的问题…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            autoComplete="off"
          />
          <button
            className="icon-button"
            aria-label={query ? "清除搜索" : "关闭搜索"}
            onClick={() => (query ? setQuery("") : onClose())}
          >
            <X size={18} />
          </button>
        </div>
        <div
          className="search-results"
          role="region"
          aria-live="polite"
          aria-busy={loading}
        >
          {!query.trim() ? (
            <div className="search-empty">
              <Search size={29} />
              <h3>那个问题，你不必重新问</h3>
              <p>
                输入原文关键词，跨会话定位。
                <br />
                只搜索本地记录，不调用 AI。
              </p>
            </div>
          ) : (
            <>
              {loading && <p className="search-state">正在搜索本地问题索引…</p>}
              {error && (
                <p className="search-state" role="alert">
                  {error}
                </p>
              )}
              {!loading && !error && !results.length && (
                <div className="search-empty">
                  <h3>没有找到匹配的问题</h3>
                  <p>换一个原文关键词试试。</p>
                </div>
              )}
              {groupResults(results).map(([sessionKey, group]) => (
                <section className="result-group" key={sessionKey}>
                  <h3>
                    <MessageCircle size={14} />
                    {group.title}
                  </h3>
                  {group.results.map((result) => {
                    const index = ordered.indexOf(result);
                    return (
                      <button
                        data-result-index={index}
                        className={`search-result ${active === index ? "active" : ""}`}
                        key={`${result.sessionKey}:${result.nodeId}`}
                        onMouseMove={() => setActive(index)}
                        onFocus={() => setActive(index)}
                        onClick={() => onChoose(result)}
                      >
                        <span className="result-ordinal">
                          Q{result.ordinal}
                        </span>
                        <span>
                          <strong>{result.title}</strong>
                          <small>{result.preview}</small>
                        </span>
                        <ArrowRight size={16} />
                      </button>
                    );
                  })}
                </section>
              ))}
              {truncated && (
                <p className="search-state">
                  仅展示部分匹配结果；缩小关键词范围继续查找。
                </p>
              )}
            </>
          )}
        </div>
        <footer>
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd>选择 <kbd>Enter</kbd>打开
          </span>
          <button onClick={onClose}>
            <kbd>Esc</kbd>关闭
          </button>
        </footer>
      </div>
    </dialog>
  );
}
