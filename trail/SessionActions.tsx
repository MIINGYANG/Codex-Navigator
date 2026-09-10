import { useEffect, useRef, useState } from "react";
import { Pencil, Trash2, X } from "lucide-react";
import { manageSession } from "./api";

export type SessionTarget = {
  key: string;
  title: string;
  cwd: string | null;
};
export type SessionAction = "rename" | "trash";

export function SessionActions({
  session,
  onAction,
}: {
  session: SessionTarget;
  onAction: (session: SessionTarget, action: SessionAction) => void;
}) {
  return (
    <span className="session-actions">
      <button
        className="icon-button"
        aria-label={`重命名会话：${session.title}`}
        title="重命名，同步到 Codex"
        onClick={() => onAction(session, "rename")}
      >
        <Pencil size={14} />
      </button>
      <button
        className="icon-button"
        aria-label={`删除会话：${session.title}`}
        title="移到系统回收站"
        onClick={() => onAction(session, "trash")}
      >
        <Trash2 size={14} />
      </button>
    </span>
  );
}

export default function SessionActionDialog({
  session,
  action,
  onClose,
  onRenamed,
  onTrashed,
}: {
  session: SessionTarget;
  action: SessionAction;
  onClose: () => void;
  onRenamed: (key: string, name: string) => void;
  onTrashed: (key: string) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [name, setName] = useState(session.title);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const trimmed = name.trim();
  const valid = [...trimmed].length >= 1 && [...trimmed].length <= 100;
  const trash = action === "trash";
  useEffect(() => {
    dialog.current?.showModal();
  }, []);
  async function submit() {
    if (busy || (!trash && !valid)) return;
    setBusy(true);
    setError("");
    try {
      if (trash) {
        await manageSession(session.key, "trash", { confirm: true });
        onTrashed(session.key);
      } else {
        const result = await manageSession<{ name: string }>(
          session.key,
          "rename",
          { name: trimmed },
        );
        onRenamed(session.key, result.name);
      }
      onClose();
    } catch (reason) {
      setError((reason as Error).message);
      setBusy(false);
    }
  }
  return (
    <dialog
      ref={dialog}
      className="session-action-dialog"
      aria-labelledby="session-action-title"
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onClose();
      }}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <header>
          <h2 id="session-action-title">{trash ? "删除会话" : "重命名会话"}</h2>
          <button
            type="button"
            className="icon-button"
            aria-label="关闭会话管理"
            disabled={busy}
            onClick={onClose}
          >
            <X size={18} />
          </button>
        </header>
        <p className="session-action-context">
          <strong>{session.title}</strong>
          <code>{session.cwd || "项目路径未记录"}</code>
        </p>
        {trash ? (
          <p>会话文件将移到系统回收站，可恢复。请先结束正在运行的该会话。</p>
        ) : (
          <label className="session-name-field">
            会话名称
            <input
              autoFocus
              value={name}
              disabled={busy}
              onChange={(event) => setName(event.target.value)}
              aria-describedby="session-name-hint"
            />
            <span id="session-name-hint">
              1–100 个字符，保存后同步到 Codex。
            </span>
          </label>
        )}
        {error && (
          <p className="session-action-error" role="alert">
            {error}
          </p>
        )}
        <footer>
          <button
            type="button"
            disabled={busy}
            onClick={onClose}
            autoFocus={trash}
          >
            取消
          </button>
          <button
            type="submit"
            className={trash ? "danger-button" : "save-button"}
            disabled={busy || (!trash && !valid)}
          >
            {busy ? "正在处理…" : trash ? "移到回收站" : "保存名称"}
          </button>
        </footer>
      </form>
    </dialog>
  );
}
