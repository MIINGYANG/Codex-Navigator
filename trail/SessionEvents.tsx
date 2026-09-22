import { useEffect, useRef, useState } from "react";
import {
  ArrowRight,
  ChevronsDownUp,
  Clock,
  GitCommitHorizontal,
  X,
} from "lucide-react";
import type { SessionEvent } from "./graph";
import { fullTime } from "./state";

export function eventTitle(event: SessionEvent) {
  if (event.kind === "commit") return "代码已提交";
  return event.trigger === "auto"
    ? "自动压缩"
    : event.trigger === "manual"
      ? "手动压缩"
      : "压缩 · 触发方式未记录";
}

export function EventDetail({
  event,
  onClose,
  onLocate,
}: {
  event: SessionEvent;
  onClose(): void;
  onLocate(index: number): void;
}) {
  const panel = useRef<HTMLElement>(null);
  useEffect(() => {
    panel.current?.focus({ preventScroll: true });
  }, [event.id]);
  return (
    <aside
      id="detail-panel"
      className="detail-panel has-selection event-detail"
      aria-label="会话事件详情"
      ref={panel}
      tabIndex={-1}
    >
      <header className="detail-header">
        <h2>{event.kind === "commit" ? "代码提交" : "上下文压缩"}</h2>
        <button
          className="icon-button"
          aria-label="关闭事件详情"
          onClick={onClose}
        >
          <X size={18} />
        </button>
      </header>
      <div className="detail-scroll">
        <div className={`event-title ${event.kind}`}>
          {event.kind === "commit" ? (
            <GitCommitHorizontal size={21} />
          ) : (
            <ChevronsDownUp size={21} />
          )}
          <h3>{eventTitle(event)}</h3>
        </div>
        <p className="question-time">{fullTime(event.timestamp)}</p>
        {event.summary && <p className="event-summary">{event.summary}</p>}
        <dl className="event-facts">
          <dt>关联问题</dt>
          <dd>
            {event.turn_index === null
              ? "关联位置未记录"
              : `Q${event.turn_index + 1}`}
          </dd>
          {event.kind === "commit" ? (
            <>
              <dt>仓库 / 路径</dt>
              <dd>{event.repository || "仓库未记录"}</dd>
              <dt>提交时分支</dt>
              <dd>{event.branch || "分支未记录"}</dd>
              <dt>版本标签</dt>
              <dd>{event.version || "未标记版本"}</dd>
              <dt>提交哈希</dt>
              <dd>{event.hash || "未记录"}</dd>
            </>
          ) : (
            <>
              <dt>触发方式</dt>
              <dd>
                {event.trigger === "auto"
                  ? "自动"
                  : event.trigger === "manual"
                    ? "手动"
                    : "未记录"}
              </dd>
            </>
          )}
        </dl>
        <p className="relationship-note">
          {event.kind === "commit"
            ? "根据会话内的成功提交记录标记。仓库、分支和版本仅显示记录中的信息。"
            : "压缩是上下文整理事件，不增加问题编号。旧记录可能没有写明触发方式。"}
        </p>
        <details className="event-evidence">
          <summary>记录来源</summary>
          <p>{event.source}</p>
        </details>
        {event.turn_index !== null && (
          <button
            className="event-locate blue-button"
            onClick={() => onLocate(event.turn_index!)}
          >
            查看关联问题 Q{event.turn_index + 1}
            <ArrowRight size={15} />
          </button>
        )}
      </div>
    </aside>
  );
}

export function EventsDialog({
  events,
  onChoose,
  onClose,
  loading,
  scopeLabel,
}: {
  scopeLabel?: string;
  events: SessionEvent[];
  onChoose(event: SessionEvent): void;
  onClose(): void;
  loading: boolean;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [filter, setFilter] = useState<"all" | "commit" | "compaction">("all");
  const [limit, setLimit] = useState(100);
  const visible = events.filter(
    (event) => filter === "all" || event.kind === filter,
  );
  useEffect(() => {
    dialog.current?.showModal();
  }, []);
  return (
    <dialog
      ref={dialog}
      className="search-dialog session-events-dialog"
      aria-label={scopeLabel ? "压缩记录" : "会话事件"}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (event.target === dialog.current) onClose();
      }}
    >
      <div className="search-dialog-inner">
        <header className="events-dialog-header">
          <Clock size={18} />
          <h2>{scopeLabel ? "压缩记录" : "会话事件"}</h2>
          <button
            className="icon-button"
            aria-label="关闭会话事件"
            onClick={onClose}
          >
            <X size={18} />
          </button>
        </header>
        {scopeLabel ? (
          <div className="events-scope-summary">
            <strong>
              {scopeLabel} · {events.length} 条记录
            </strong>
            <p>
              逐条查看时间与来源。此处统计事件记录，不据此推断实际压缩次数。
            </p>
          </div>
        ) : (
          <div className="events-filters" role="group" aria-label="事件类型">
            {(
              [
                ["all", "全部"],
                ["commit", "提交"],
                ["compaction", "压缩"],
              ] as const
            ).map(([value, label]) => (
              <button
                key={value}
                aria-pressed={filter === value}
                onClick={() => {
                  setFilter(value);
                  setLimit(100);
                }}
              >
                {label}{" "}
                {value === "all"
                  ? events.length
                  : events.filter((e) => e.kind === value).length}
              </button>
            ))}
          </div>
        )}
        <div className="events-list">
          {visible.slice(0, limit).map((event) => (
            <button
              className={`session-event-row ${event.kind}`}
              key={event.id}
              onClick={() => onChoose(event)}
            >
              {event.kind === "commit" ? (
                <GitCommitHorizontal size={18} />
              ) : (
                <ChevronsDownUp size={18} />
              )}
              <span>
                <strong>{eventTitle(event)}</strong>
                <small>
                  {event.turn_index === null
                    ? "关联位置未记录"
                    : `Q${event.turn_index + 1}`}{" "}
                  · {fullTime(event.timestamp)}
                </small>
                {event.kind === "commit" && (
                  <small>
                    {event.repository || "仓库未记录"} ·{" "}
                    {event.version || event.hash}
                  </small>
                )}
              </span>
              <ArrowRight size={14} />
            </button>
          ))}
          {!visible.length && (
            <p className="quiet-empty">
              {loading ? "正在读取会话事件…" : "没有已识别的这类事件记录"}
            </p>
          )}
          {visible.length > limit && (
            <button
              className="subtle-button"
              onClick={() => setLimit(limit + 100)}
            >
              继续显示（还有 {visible.length - limit} 条）
            </button>
          )}
        </div>
      </div>
    </dialog>
  );
}
