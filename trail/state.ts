import type { CanvasLayout, QuestionNode, SessionEvent } from "./graph";

export type SessionSummary = {
  key: string;
  id: string;
  title: string | null;
  cwd: string | null;
  updated_at: string | null;
  turn_count: number | null;
  first_prompt: string | null;
  favorite?: boolean;
  events_loading?: boolean;
  commit_count?: number | null;
  compaction_count?: number | null;
  last_commit?: SessionEvent | null;
};

export function canvasPreferences(value: unknown): CanvasLayout {
  const saved =
    value && typeof value === "object"
      ? (value as Record<string, unknown>)
      : {};
  return {
    direction: saved.direction === "horizontal" ? "horizontal" : "vertical",
    density: saved.density === "compact" ? "compact" : "comfortable",
  };
}

export function filterSessions(
  sessions: SessionSummary[],
  query: string,
  filter: "all" | "favorites" | "commits",
  favoriteFirst: boolean,
) {
  const term = query.trim().toLocaleLowerCase();
  return sessions
    .filter(
      (session) =>
        (filter !== "favorites" || session.favorite) &&
        (filter !== "commits" || (session.commit_count ?? 0) > 0) &&
        `${titleOf(session)} ${session.cwd || ""} ${session.last_commit?.repository || ""} ${session.last_commit?.branch || ""}`
          .toLocaleLowerCase()
          .includes(term),
    )
    .sort((a, b) =>
      favoriteFirst
        ? Number(Boolean(b.favorite)) - Number(Boolean(a.favorite))
        : 0,
    );
}
export type SearchResult = {
  sessionKey: string;
  sessionTitle: string;
  nodeId: string;
  turnIndex: number;
  ordinal: number;
  title: string;
  preview: string;
};

export function titleOf(session?: SessionSummary) {
  return (
    (
      [session?.title, session?.first_prompt, session?.id].find((value) =>
        value?.trim(),
      ) || "未命名会话"
    )
      .trim()
      .split(/\r?\n/)
      .find((line) => line.trim())
      ?.replace(/\s+/g, " ") || "未命名会话"
  );
}

export function reconcileSessions(
  sessions: SessionSummary[],
  renamed: ReadonlyMap<string, string>,
  deleted: ReadonlySet<string>,
) {
  return sessions
    .filter((session) => !deleted.has(session.key))
    .map((session) =>
      renamed.has(session.key)
        ? { ...session, title: renamed.get(session.key)! }
        : session,
    );
}

export function relativeTime(
  value: string | null | undefined,
  now = Date.now(),
) {
  if (!value) return "时间未记录";
  const date = Date.parse(value);
  if (!Number.isFinite(date)) return "时间未记录";
  const minutes = Math.max(0, Math.floor((now - date) / 60000));
  if (minutes < 1) return "刚刚更新";
  if (minutes < 60) return `${minutes} 分钟前`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} 小时前`;
  if (minutes < 10080) return `${Math.floor(minutes / 1440)} 天前`;
  return new Date(date).toLocaleDateString("zh-CN");
}

export function fullTime(value?: string | null) {
  if (!value || !Number.isFinite(Date.parse(value))) return "时间未记录";
  return new Date(value).toLocaleString("zh-CN", { hour12: false });
}

export function groupResults(results: SearchResult[]) {
  const groups = new Map<string, { title: string; results: SearchResult[] }>();
  for (const result of results) {
    if (!groups.has(result.sessionKey))
      groups.set(result.sessionKey, {
        title: result.sessionTitle,
        results: [],
      });
    groups.get(result.sessionKey)!.results.push(result);
  }
  return [...groups.entries()];
}

export function reconcileSelection(
  selected: string | null,
  ids: Set<string>,
  previousGeneration: number | undefined,
  generation: number,
) {
  return previousGeneration !== undefined && previousGeneration !== generation
    ? null
    : selected && ids.has(selected)
      ? selected
      : null;
}

// Loading can recur during a large live append; it must not reset the unread baseline.
export function initialLoadTransition(
  initializing: boolean,
  previousGeneration: number | undefined,
  generation: number,
  loading: boolean,
) {
  const resetSeen =
    initializing ||
    previousGeneration === undefined ||
    previousGeneration !== generation;
  return { resetSeen, initializing: resetSeen && loading };
}

/** A scoped dialog belongs to one session generation and one recorded edge. */
export type EventView =
  | "all"
  | {
      key: string;
      generation: number;
      edgeId: string;
      label: string;
    }
  | null;

export function reconcileEventView(
  view: EventView,
  next: { key: string; generation: number; edges: { id: string }[] },
): EventView {
  if (!view || view === "all") return view;
  return view.key === next.key &&
    view.generation === next.generation &&
    next.edges.some((edge) => edge.id === view.edgeId)
    ? view
    : null;
}

export type FavoriteQuestion = {
  favoriteId: string;
  sessionKey: string;
  nodeId: string;
  sessionTitle: string;
  cwd: string | null;
  promptPreview: string;
  timestamp: string | null;
  ordinal: number;
};

export type FavoriteCatalog = {
  loading: boolean;
  truncated: boolean;
  error: string | null;
  results: FavoriteQuestion[];
  unavailable: { favoriteId: string; reason: string }[];
};

export function filterFavoriteQuestions(
  questions: FavoriteQuestion[],
  query: string,
) {
  const term = query.trim().toLocaleLowerCase();
  return questions.filter((question) =>
    `${question.promptPreview} ${question.sessionTitle} ${question.cwd || ""}`
      .toLocaleLowerCase()
      .includes(term),
  );
}

// 只有完整快照中的唯一稳定身份才允许跳转，qN 会在会话重建时复用。
export function resolveFavoriteQuestion(
  favoriteId: string,
  nodes: QuestionNode[],
  loading: boolean,
): string | null {
  if (loading || !favoriteId) return null;
  const matches = nodes.filter((node) => node.favorite_id === favoriteId);
  return matches.length === 1 ? matches[0].id : null;
}
