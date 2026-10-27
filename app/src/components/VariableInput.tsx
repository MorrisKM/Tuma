import { type InputHTMLAttributes, type TextareaHTMLAttributes, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Choice } from "../headers";
import { useStore } from "../store";
import { matching, NO_ENVIRONMENT, noEnvironment, type Suggestion, suggestions, typedVariable } from "../variables";

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
    </>
  );
}
