import { type InputHTMLAttributes, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useStore } from "../store";
import { matching, NO_ENVIRONMENT, noEnvironment, type Suggestion, suggestions, typedVariable } from "../variables";

type Props = Omit<InputHTMLAttributes<HTMLInputElement>, "value" | "onChange"> & {
  value: string;
  onChange: (value: string) => void;
};

/** A whole value that is one {{variable}}, possibly not closed yet. */
const REFERENCE = /^\{\{\s*[\w$.-]*\s*(\}\}?)?$/;

interface Open {
  /** Where the typed name starts and the caret is. */
  from: number;
  to: number;
  options: Suggestion[];
  active: number;
  /** Whether to say that no environment is selected. */
  note: boolean;
  rect: DOMRect;
}

/** A text input that suggests the variables in scope after {{, as the URL bar does. */
export function VariableInput({ value, onChange, onKeyDown, onBlur, ...rest }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState<Open | null>(null);

  const refresh = (el: HTMLInputElement) => {
    const caret = el.selectionStart ?? el.value.length;
    const found = el.selectionStart === el.selectionEnd ? typedVariable(el.value.slice(0, caret)) : null;
    const options = found ? matching(suggestions(useStore.getState().scope), found.typed) : [];
    const note = noEnvironment();
    setOpen(
      found && (options.length || note)
        ? { from: found.from, to: caret, options, active: 0, note, rect: el.getBoundingClientRect() }
        : null,
    );
  };

  const apply = (s: Suggestion) => {
    if (!open) return;
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

  return (
    <>
      <input
        {...rest}
        type={type}
        ref={input}
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          refresh(e.target);
        }}
        onKeyDown={(e) => {
          if (open?.options.length) {
            const n = open.options.length;
            if (e.key === "ArrowDown" || e.key === "ArrowUp") {
              e.preventDefault();
              setOpen({ ...open, active: (open.active + (e.key === "ArrowDown" ? 1 : n - 1)) % n });
              return;
            }
            if (e.key === "Enter" || e.key === "Tab") {
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
          onKeyDown?.(e);
        }}
        onBlur={(e) => {
          setOpen(null);
          onBlur?.(e);
        }}
      />
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
