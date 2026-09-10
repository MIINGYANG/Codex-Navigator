export type SessionSummary = {
  key: string;
  id: string;
  title: string | null;
  cwd: string | null;
  updated_at: string | null;
  turn_count: number | null;
  first_prompt: string | null;
};
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
