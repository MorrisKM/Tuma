// Small pieces used across the app.

import { useEffect, useLayoutEffect, useRef } from "react";

/** The paper plane of the app icon. */
export function Logo({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
      <g transform="rotate(-12 16 16)">
        <path d="M4 16 L28 6 L22 27 L16.5 19.5 Z" fill="var(--accent)" />
        <path d="M16.5 19.5 L28 6 L18.5 21.5 Z" fill="var(--accent-deep)" />
      </g>
    </svg>
  );
}

/** A method, short enough for a narrow column: DELETE is DEL, OPTIONS is OPT. */
export function MethodTag({ method }: { method: string }) {
  const m = method.toUpperCase();
  const short = m === "DELETE" ? "DEL" : m === "OPTIONS" ? "OPT" : m;
  return <span className={`method-tag method-${m.toLowerCase()}`}>{short}</span>;
}

/** A small popup menu that closes on any click or Escape. */
export function Menu({
  children,
  onClose,
  className = "",
  at,
}: {
  children: React.ReactNode;
  onClose: () => void;
  className?: string;
  /** Opens at this point of the window, for a right-click, instead of below its parent. */
  at?: { x: number; y: number };
}) {
  useEffect(() => {
    const close = () => onClose();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    // Deferred, so the click that opened the menu does not close it.
    const timer = setTimeout(() => window.addEventListener("click", close));
    window.addEventListener("keydown", onKey);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);
  const ref = useRef<HTMLDivElement>(null);
  // Kept inside the window: moved left or up when it would run off the edge.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || !at) return;
    const r = el.getBoundingClientRect();
    el.style.left = `${Math.max(4, Math.min(at.x, window.innerWidth - r.width - 4))}px`;
    el.style.top = `${Math.max(4, Math.min(at.y, window.innerHeight - r.height - 4))}px`;
  }, [at]);
  return (
    <div
      ref={ref}
      className={`menu ${className}`}
      role="menu"
      style={at && { position: "fixed", left: at.x, top: at.y, right: "auto" }}
      onClick={(e) => e.stopPropagation()}
    >
      <div onClick={onClose}>{children}</div>
    </div>
  );
}
