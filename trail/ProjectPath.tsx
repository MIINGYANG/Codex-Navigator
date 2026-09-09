import { useRef } from "react";
import { Copy, FolderOpen } from "lucide-react";

export default function ProjectPath({
  cwd,
  notify,
}: {
  cwd: string | null | undefined;
  notify: (message: string) => void;
}) {
  const text = useRef<HTMLElement>(null);
  async function copy() {
    if (!cwd) return;
    try {
      await navigator.clipboard.writeText(cwd);
      notify("项目路径已复制");
    } catch {
      if (text.current) {
        const range = document.createRange();
        range.selectNodeContents(text.current);
        const selection = window.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(range);
      }
      notify("剪贴板不可用，请手动复制项目路径");
    }
  }
  return (
    <div className="project-path" aria-label="项目路径">
      <FolderOpen size={14} aria-hidden="true" />
      <span>项目路径</span>
      <code ref={text}>{cwd || "项目路径未记录"}</code>
      {cwd && (
        <button
          className="icon-button"
          aria-label="复制项目路径"
          title="复制项目路径"
          onClick={() => void copy()}
        >
          <Copy size={14} />
        </button>
      )}
    </div>
  );
}
