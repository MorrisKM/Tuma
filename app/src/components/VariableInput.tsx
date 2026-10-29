import { type InputHTMLAttributes, type TextareaHTMLAttributes, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Choice } from "../headers";
import { useStore } from "../store";
import {
  describeVariable,
  matching,
  NO_ENVIRONMENT,
  noEnvironment,
  type Suggestion,
  suggestions,
  typedVariable,
} from "../variables";

type Props = Omit<InputHTMLAttributes<HTMLInputElement>, "value" | "onChange"> & {
  value: string;
  onChange: (value: string) => void;
  /** Values to offer in a list when the input is focused or typed in, such as header names; one replaces the text. */
  choices?: Choice[];
};

/** How tall a focused field grows to show all of a long value before it scrolls instead, in pixels. */
const GROW_LIMIT = 168;

type Field = HTMLInputElement | HTMLTextAreaElement;

/** A whole value that is one {{variable}}, possibly not closed yet. */
const REFERENCE = /^\{\{\s*[\w$.-]*\s*(\}\}?)?$/;

/** A {{variable}} anywhere in the text. */
const VARIABLE = /\{\{\s*([^{}]+?)\s*\}\}/g;

/** The {{variable}} under the pointer, if any, and where it is drawn. */
interface Hover {
  name: string;
  left: number;
  top: number;
}

/** The style a copy of a field needs to lay its text out the same way. */
const LAYOUT = [
  "boxSizing",
  "width",
  "paddingTop",
  "paddingRight",
  "paddingBottom",
  "paddingLeft",
  "borderTopWidth",
  "borderRightWidth",
  "borderBottomWidth",
  "borderLeftWidth",
  "borderStyle",
  "fontFamily",
  "fontSize",
  "fontWeight",
  "fontStyle",
  "letterSpacing",
  "lineHeight",
  "tabSize",
  "textTransform",
  "whiteSpace",
  "overflowWrap",
  "wordBreak",
] as const;

/**
 * Finds the {{variable}} at a point in a field. A field can't say which character is where, so its text is laid out
 * again in a hidden copy over it, with each variable in a span that can be measured.
 */
function variableAt(el: Field, x: number, y: number): Hover | null {
  const text = el.value;
  // A masked password's dots aren't its characters.
  if (!text.includes("{{") || (el instanceof HTMLInputElement && el.type === "password")) return null;
  const box = el.getBoundingClientRect();
  const style = getComputedStyle(el);
  const copy = document.createElement("div");
  for (const key of LAYOUT) copy.style[key] = style[key];
  // An <input> is always one line.
  if (el instanceof HTMLInputElement) copy.style.whiteSpace = "pre";
  Object.assign(copy.style, {
    position: "fixed",
    left: `${box.left - el.scrollLeft}px`,
    top: `${box.top - el.scrollTop}px`,
    borderColor: "transparent",
    visibility: "hidden",
    pointerEvents: "none",
    overflow: "visible",
  });
  let at = 0;
  const spans: { name: string; span: HTMLSpanElement }[] = [];
  for (const m of text.matchAll(VARIABLE)) {
    copy.append(text.slice(at, m.index));
    const span = document.createElement("span");
    span.textContent = m[0];
    copy.append(span);
    spans.push({ name: m[1], span });
    at = m.index! + m[0].length;
  }
  copy.append(text.slice(at));
  document.body.append(copy);
  try {
    for (const { name, span } of spans) {
      for (const r of span.getClientRects()) {
        // An <input> centers its line, which the copy may not: there only the horizontal position counts.
        const inLine = el instanceof HTMLInputElement || (y >= r.top && y <= r.bottom);
        if (x >= r.left && x <= r.right && inLine) {
          return { name, left: Math.max(r.left, box.left), top: Math.max(r.top, box.top) };
        }
      }
    }
    return null;
  } finally {
    copy.remove();
  }
}

interface Open {
  /** A {{variable}} to complete, or a choice to replace the whole text. */
  kind: "variable" | "choice";
  /** Where the typed name starts and the caret is. */
  from: number;
  to: number;
  options: Suggestion[];
  /** -1 when none is picked: Enter and Tab then do what they would without the list. */
  active: number;
  /** Whether to say that no environment is selected. */
  note: boolean;
  rect: DOMRect;
}

/** The choices that contain the typed text, those that start with it first; none once it is one of them. */
function matchingChoices(choices: Choice[], typed: string): Suggestion[] {
  const t = typed.trim().toLowerCase();
  if (choices.some((c) => c.value.toLowerCase() === t)) return [];
  const found = choices.filter((c) => c.value.toLowerCase().includes(t));
  const first = found.filter((c) => c.value.toLowerCase().startsWith(t));
  return [...first, ...found.filter((c) => !first.includes(c))].map((c) => ({
    name: c.value,
    detail: c.detail,
    info: "",
    dynamic: false,
  }));
}

/**
 * A text input that suggests the variables in scope after {{, as the URL bar does, and with `choices` offers values
 * for the whole text. A text field shows a value too long for it on one line while it isn't focused; focused, the
 * value wraps and the field grows to show it, up to a limit past which it scrolls.
 */
export function VariableInput({ value, onChange, onKeyDown, onBlur, onFocus, choices, ...rest }: Props) {
  const input = useRef<Field>(null);
  const [open, setOpen] = useState<Open | null>(null);
  const [focused, setFocused] = useState(false);
  const [hover, setHover] = useState<Hover | null>(null);
  // Passwords stay in an <input>, which masks them.
  const grows = !rest.type || rest.type === "text";

  // Focused, as tall as the wrapped value, within the limit; otherwise back to one line, scrolled to its start.
  useLayoutEffect(() => {
    const el = input.current;
    if (!grows || !(el instanceof HTMLTextAreaElement)) return;
    if (!focused) {
      el.style.height = "";
      el.style.overflowY = "";
      el.scrollTop = 0;
      el.scrollLeft = 0;
      return;
    }
    el.style.height = "";
    const natural = el.offsetHeight;
    const needed = el.scrollHeight + (el.offsetHeight - el.clientHeight);
    if (needed > natural) {
      el.style.height = `${Math.min(needed, GROW_LIMIT)}px`;
      el.style.overflowY = needed > GROW_LIMIT ? "auto" : "hidden";
    }
  }, [value, focused, grows]);

  const refresh = (el: Field) => {
    const caret = el.selectionStart ?? el.value.length;
    const found = el.selectionStart === el.selectionEnd ? typedVariable(el.value.slice(0, caret)) : null;
    const options = found ? matching(suggestions(useStore.getState().scope), found.typed) : [];
    const note = noEnvironment();
    if (found) {
      setOpen(
        options.length || note
          ? { kind: "variable", from: found.from, to: caret, options, active: 0, note, rect: el.getBoundingClientRect() }
          : null,
      );
      return;
    }
    const offered = choices ? matchingChoices(choices, el.value) : [];
    setOpen(
      offered.length
        ? {
            kind: "choice",
            from: 0,
            to: el.value.length,
            options: offered,
            // With nothing typed, the list is a menu: Tab still moves on.
            active: el.value.trim() ? 0 : -1,
            note: false,
            rect: el.getBoundingClientRect(),
          }
        : null,
    );
  };

  const apply = (s: Suggestion) => {
    if (!open) return;
    if (open.kind === "choice") {
      onChange(s.name);
      setOpen(null);
      requestAnimationFrame(() => input.current?.setSelectionRange(s.name.length, s.name.length));
      return;
    }
    const after = value.slice(open.to).replace(/^[\w$.-]*/, "");
    const close = after.startsWith("}}") ? "" : "}}";
    const caret = open.from + s.name.length + 2;
    onChange(value.slice(0, open.from) + s.name + close + after);
    setOpen(null);
    requestAnimationFrame(() => input.current?.setSelectionRange(caret, caret));
  };

  // A password field that holds only a {{variable}} (or one being typed) shows it, so the reference can be read;
  // a real password stays masked.
  const type = rest.type === "password" && REFERENCE.test(value) ? "text" : rest.type;

  const { type: _, ...textProps } = rest;
  const handlers = {
    onChange: (e: React.ChangeEvent<Field>) => {
      // One line, as in an <input>: pasted line breaks are dropped.
      const text = e.target.value.replace(/\r?\n/g, "");
      onChange(text);
      setHover(null);
      refresh(e.target);
    },
    onKeyDown: (e: React.KeyboardEvent<Field>) => {
      if (open?.options.length) {
        const n = open.options.length;
        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          const from = open.active < 0 ? (e.key === "ArrowDown" ? n - 1 : 0) : open.active;
          setOpen({ ...open, active: (from + (e.key === "ArrowDown" ? 1 : n - 1)) % n });
          return;
        }
        if ((e.key === "Enter" || e.key === "Tab") && open.active >= 0) {
          e.preventDefault();
          apply(open.options[open.active]);
          return;
        }
      }
      if (open && e.key === "Escape") {
        e.preventDefault();
        setOpen(null);
        return;
      }
      onKeyDown?.(e as React.KeyboardEvent<HTMLInputElement>);
      // A wrapped field is still one line: Enter adds no line break.
      if (e.key === "Enter") e.preventDefault();
    },
    onFocus: (e: React.FocusEvent<Field>) => {
      setFocused(true);
      if (choices) refresh(e.target);
      onFocus?.(e as React.FocusEvent<HTMLInputElement>);
    },
    // Over a {{variable}}, its value, as in the URL bar.
    onMouseMove: (e: React.MouseEvent<Field>) => {
      const found = variableAt(e.currentTarget, e.clientX, e.clientY);
      setHover((h) => (found?.name === h?.name && found?.left === h?.left && found?.top === h?.top ? h : found));
    },
    onMouseLeave: () => setHover(null),
    onClick: (e: React.MouseEvent<Field>) => {
      if (choices && !open) refresh(e.currentTarget);
    },
    onBlur: (e: React.FocusEvent<Field>) => {
      setFocused(false);
      setOpen(null);
      onBlur?.(e as React.FocusEvent<HTMLInputElement>);
    },
  };

  return (
    <>
      {grows ? (
        <textarea
          {...(textProps as TextareaHTMLAttributes<HTMLTextAreaElement>)}
          className={`field-grow${rest.className ? ` ${rest.className}` : ""}`}
          ref={input as React.RefObject<HTMLTextAreaElement>}
          rows={1}
          wrap={focused ? "soft" : "off"}
          value={value}
          {...handlers}
        />
      ) : (
        <input {...rest} type={type} ref={input as React.RefObject<HTMLInputElement>} value={value} {...handlers} />
      )}
      {open &&
        createPortal(
          <ul
            className="var-suggest"
            role="listbox"
            style={{ left: open.rect.left, top: open.rect.bottom + 4, minWidth: Math.min(open.rect.width, 320) }}
          >
            {open.options.map((s, i) => (
              <li
                key={s.name}
                role="option"
                aria-selected={i === open.active}
                // Keep the focus in the input, which closes the list on blur.
                onMouseDown={(e) => {
                  e.preventDefault();
                  apply(s);
                }}
              >
                <span className="name">{s.name}</span>
                {s.detail && <span className="detail">{s.detail}</span>}
              </li>
            ))}
            {open.note && (
              <li className="note" aria-disabled="true" onMouseDown={(e) => e.preventDefault()}>
                {NO_ENVIRONMENT}: select one to use its variables
              </li>
            )}
          </ul>,
          document.body,
        )}
      {hover && !open && <VariableHover {...hover} />}
    </>
  );
}

/** The value of the {{variable}} under the pointer, above it. */
function VariableHover({ name, left, top }: Hover) {
  const info = describeVariable(name, useStore.getState().scope);
  return createPortal(
    <div className="cm-tooltip var-hover" style={{ left, top }}>
      <div className="var-tooltip">
        <strong>{name}</strong>
        <div className={info.missing ? "missing" : undefined}>{info.detail}</div>
        {info.source && <div className="source">{info.source}</div>}
      </div>
    </div>,
    document.body,
  );
}
