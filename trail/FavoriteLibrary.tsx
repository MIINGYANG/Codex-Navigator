import { useEffect, useState, type ReactNode } from "react";
import { ArrowUpRight, MessageCircle, RefreshCw, Star } from "lucide-react";
import { api } from "./api";
import {
  filterFavoriteQuestions,
  filterSessions,
  fullTime,
  relativeTime,
  type FavoriteCatalog,
  type FavoriteQuestion,
  type SessionSummary,
} from "./state";

export default function FavoriteLibrary({
  sessions,
  query,
  revision,
  selectedKey,
  selectedFavoriteId,
  renderSession,
  onQuestion,
}: {
  sessions: SessionSummary[];
  query: string;
  revision: number;
  selectedKey: string | null;
  selectedFavoriteId: string | null;
  renderSession: (session: SessionSummary) => ReactNode;
  onQuestion: (question: FavoriteQuestion) => void;
}) {
  const [category, setCategory] = useState<"all" | "sessions" | "questions">(
    "all",
  );
  const [catalog, setCatalog] = useState<FavoriteCatalog | null>(null);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function load() {
      setRefreshing(true);
      try {
        const next = await api<FavoriteCatalog>(
          "/api/trail/favorites",
          controller.signal,
        );
        if (controller.signal.aborted) return;
        setCatalog(next);
        setError(next.error || "");
        timer = setTimeout(() => void load(), next.loading ? 700 : 15000);
      } catch (cause) {
        if (!controller.signal.aborted) setError((cause as Error).message);
      } finally {
        if (!controller.signal.aborted) setRefreshing(false);
      }
    }
    void load();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [revision, retry]);

  const favoriteSessions = filterSessions(sessions, query, "favorites", false);
  const questions = filterFavoriteQuestions(catalog?.results || [], query);
  const loading = (!catalog && !error) || Boolean(catalog?.loading);
  const unavailable =
    catalog?.unavailable.filter((item) => item.reason !== "indexing") || [];
  const showQuestions = category !== "sessions";
  const showSessions = category !== "questions";
  const count =
    (showQuestions ? questions.length : 0) +
    (showSessions ? favoriteSessions.length : 0);
  const categories = [
    ["all", "全部", questions.length + favoriteSessions.length],
    ["questions", "问题", questions.length],
    ["sessions", "会话", favoriteSessions.length],
  ] as const;

  return (
    <section className="favorite-library" aria-label="我的收藏">
      <div className="favorite-categories" role="group" aria-label="收藏分类">
        {categories.map(([value, label, total]) => (
          <button
            key={value}
            aria-pressed={category === value}
            onClick={() => setCategory(value)}
          >
            {label}
            <small>{total}</small>
          </button>
        ))}
      </div>
      <nav className="session-list favorite-list" aria-label="选择收藏">
        {showQuestions && questions.length > 0 && (
          <>
            <div className="favorite-section-label">
              <Star size={12} />
              收藏的问题 <span>{questions.length}</span>
            </div>
            {questions.map((question) => {
              const active =
                selectedKey === question.sessionKey &&
                selectedFavoriteId === question.favoriteId;
              return (
                <button
                  key={`${question.sessionKey}:${question.favoriteId}`}
                  className={`favorite-question ${active ? "active" : ""}`}
                  aria-current={active ? "true" : undefined}
                  onClick={() => onQuestion(question)}
                  title={`${question.promptPreview}\n来自：${question.sessionTitle}\n${question.cwd || "项目路径未记录"}\n${fullTime(question.timestamp)}`}
                >
                  <span className="favorite-question-caption">
                    <span>问题 {question.ordinal}</span>
                    <ArrowUpRight size={13} />
                  </span>
                  <strong>{question.promptPreview || "问题正文未记录"}</strong>
                  <span className="favorite-question-source">
                    <MessageCircle size={12} />
                    <span>{question.sessionTitle}</span>
                  </span>
                  <small className="favorite-question-path">
                    {question.cwd || "项目路径未记录"}
                  </small>
                  <time title={fullTime(question.timestamp)}>
                    {relativeTime(question.timestamp)}
                  </time>
                </button>
              );
            })}
          </>
        )}
        {showSessions && favoriteSessions.length > 0 && (
          <>
            <div className="favorite-section-label">
              <MessageCircle size={12} />
              收藏的会话 <span>{favoriteSessions.length}</span>
            </div>
            {favoriteSessions.map(renderSession)}
          </>
        )}
        {loading && (
          <p className="favorite-status" role="status">
            <RefreshCw size={13} className="spinning" />
            正在查找收藏的问题…
          </p>
        )}
        {error && (
          <div className="favorite-status favorite-error" role="alert">
            <p>{error}</p>
            <button
              onClick={() => setRetry((value) => value + 1)}
              disabled={refreshing}
            >
              重试
            </button>
          </div>
        )}
        {!loading && !error && count === 0 && (
          <div className="favorite-empty">
            <Star size={23} />
            <strong>
              {query.trim()
                ? "没有匹配的收藏"
                : category === "sessions"
                  ? "还没有收藏会话"
                  : category === "questions"
                    ? "还没有收藏问题"
                    : "把值得回看的内容留在这里"}
            </strong>
            <p>
              {query.trim()
                ? "试试问题中的词、会话名称或项目路径。"
                : "点击问题或会话旁的星标即可收藏。收藏问题后，无需再收藏整个会话。"}
            </p>
          </div>
        )}
        {Boolean(unavailable.length) && (
          <p className="favorite-status">
            {catalog!.unavailable.length}{" "}
            个收藏的问题暂时无法定位。原会话可能已移走、回滚或不在当前数据目录；收藏记录仍保留。
          </p>
        )}
        {catalog?.truncated && (
          <p className="favorite-status">
            会话索引已达到读取上限，部分收藏可能尚未显示。
          </p>
        )}
      </nav>
    </section>
  );
}
