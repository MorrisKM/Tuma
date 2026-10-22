import { useRef } from "react";

interface Props {
  /** "x" drags left and right, between columns; "y" up and down, between rows. */
  axis: "x" | "y";
  /** The pointer's position along the axis while dragging, in client pixels. */
  onDrag: (position: number) => void;
  /** An arrow key moved the handle by this many pixels (negative is left or up). */
  onStep: (pixels: number) => void;
  onReset?: () => void;
  label: string;
}

/** A handle between two panes. Drag it, use the arrow keys when it has focus, or double-click it to reset. */
export function Splitter({ axis, onDrag, onStep, onReset, label }: Props) {
  const dragging = useRef(false);
  return (
    <div
      className={`splitter splitter-${axis}`}
      role="separator"
      aria-orientation={axis === "x" ? "vertical" : "horizontal"}
      aria-label={label}
      tabIndex={0}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        dragging.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
        document.body.classList.add(`resizing-${axis}`);
        e.preventDefault();
      }}
      onPointerMove={(e) => dragging.current && onDrag(axis === "x" ? e.clientX : e.clientY)}
      onPointerUp={(e) => {
        dragging.current = false;
        e.currentTarget.releasePointerCapture(e.pointerId);
        document.body.classList.remove(`resizing-${axis}`);
      }}
      onDoubleClick={onReset}
      onKeyDown={(e) => {
        const back = axis === "x" ? "ArrowLeft" : "ArrowUp";
        const forward = axis === "x" ? "ArrowRight" : "ArrowDown";
        const step = e.shiftKey ? 64 : 16;
        if (e.key === back) onStep(-step);
        else if (e.key === forward) onStep(step);
        else return;
        e.preventDefault();
      }}
    />
  );
}
