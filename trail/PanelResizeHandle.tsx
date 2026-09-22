import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";
import type { PanelSide } from "./panelWidths";

interface PanelResizeHandleProps {
  side: PanelSide;
  value: number;
  min: number;
  max: number;
  controls: string;
  onChange(value: number): void;
  onReset(): void;
  onCancel?(): void;
  onDraggingChange?(dragging: boolean): void;
}

export function PanelResizeHandle(props: PanelResizeHandleProps) {
  const { side, value, min, max, controls } = props;
  const latest = useRef(props);
  latest.current = props;
  const drag = useRef<{
    pointerId: number;
    startX: number;
    startValue: number;
    target: HTMLDivElement;
  } | null>(null);
  const [dragging, setDragging] = useState(false);
  const direction = side === "sidebar" ? 1 : -1;
  const label = side === "sidebar" ? "调整会话栏宽度" : "调整问题详情宽度";

  function finish(cancel = false) {
    const active = drag.current;
    if (!active) return;
    drag.current = null;
    if (cancel) {
      if (latest.current.onCancel) latest.current.onCancel();
      else latest.current.onChange(active.startValue);
    }
    if (active.target.hasPointerCapture(active.pointerId))
      active.target.releasePointerCapture(active.pointerId);
    setDragging(false);
    latest.current.onDraggingChange?.(false);
  }

  useEffect(() => {
    const blur = () => finish(true);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("blur", blur);
      const active = drag.current;
      drag.current = null;
      if (active?.target.hasPointerCapture(active.pointerId))
        active.target.releasePointerCapture(active.pointerId);
      if (active) {
        latest.current.onCancel?.();
        latest.current.onDraggingChange?.(false);
      }
    };
  }, []);

  function change(next: number) {
    latest.current.onChange(
      Math.round(
        Math.min(latest.current.max, Math.max(latest.current.min, next)),
      ),
    );
  }

  function pointerDown(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0 || !event.isPrimary || drag.current) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.focus();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startValue: value,
      target: event.currentTarget,
    };
    setDragging(true);
    latest.current.onDraggingChange?.(true);
  }

  function keyDown(event: KeyboardEvent<HTMLDivElement>) {
    // Keep graph navigation shortcuts from acting while its separator has focus.
    event.stopPropagation();
    if (event.key === "Escape" && drag.current) {
      event.preventDefault();
      finish(true);
      return;
    }
    if (
      !["ArrowLeft", "ArrowRight", "Home", "End", "Enter"].includes(event.key)
    )
      return;
    event.preventDefault();
    if (event.key === "Enter") latest.current.onReset();
    else if (event.key === "Home") change(min);
    else if (event.key === "End") change(max);
    else
      change(
        value +
          (event.key === "ArrowRight" ? 1 : -1) *
            direction *
            (event.shiftKey ? 50 : 10),
      );
  }

  return (
    <div
      className={`panel-resize-handle panel-resize-${side}${dragging ? " is-dragging" : ""}`}
      role="separator"
      tabIndex={0}
      aria-label={label}
      aria-orientation="vertical"
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuetext={`${Math.round(value)} 像素`}
      aria-controls={controls}
      title="拖动调整宽度 · 双击恢复默认 · 方向键微调"
      onPointerDown={pointerDown}
      onPointerMove={(event) => {
        const active = drag.current;
        if (!active || event.pointerId !== active.pointerId) return;
        event.preventDefault();
        change(active.startValue + (event.clientX - active.startX) * direction);
      }}
      onPointerUp={(event) => {
        if (event.pointerId === drag.current?.pointerId) finish();
      }}
      onPointerCancel={(event) => {
        if (event.pointerId === drag.current?.pointerId) finish(true);
      }}
      onLostPointerCapture={() => finish(true)}
      onBlur={() => finish(true)}
      onDoubleClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        finish();
        latest.current.onReset();
      }}
      onKeyDown={keyDown}
    >
      <span className="panel-resize-indicator" />
      <output className="panel-resize-value" aria-hidden="true">
        {Math.round(value)} px
      </output>
    </div>
  );
}
