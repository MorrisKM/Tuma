import type { KeyValue, Part } from "../types";

interface Props<T extends KeyValue> {
  rows: T[];
  onChange: (rows: T[]) => void;
  /** Builds a new row; multipart rows carry extra fields. */
  blank: () => T;
  namePlaceholder?: string;
  valuePlaceholder?: string;
  /** Renders the value cell instead of a text input, for file parts. */
  renderValue?: (row: T, set: (patch: Partial<T>) => void) => React.ReactNode | null;
  /** Extra controls after the value, such as the text/file switch for multipart. */
  renderExtra?: (row: T, set: (patch: Partial<T>) => void) => React.ReactNode;
}

/** An editable table of name/value rows. There is always an empty row at the end; typing in it adds a row. */
export function KeyValueEditor<T extends KeyValue>({
  rows,
  onChange,
  blank,
  namePlaceholder = "Name",
  valuePlaceholder = "Value",
  renderValue,
  renderExtra,
}: Props<T>) {
  const all = [...rows, blank()];

  const set = (index: number, patch: Partial<T>) => {
    const next = all.map((row, i) => (i === index ? { ...row, ...patch } : row));
    // Drop the trailing row again if it is still blank.
    const last = next[next.length - 1];
    onChange(JSON.stringify(last) === JSON.stringify(blank()) ? next.slice(0, -1) : next);
  };
  const remove = (index: number) => onChange(rows.filter((_, i) => i !== index));

  return (
    <div className="kv">
      {all.map((row, i) => {
        const isNew = i === rows.length;
        const setRow = (patch: Partial<T>) => set(i, patch);
        return (
          <div className={`kv-row${row.enabled ? "" : " disabled"}`} key={i}>
            <input
              type="checkbox"
              checked={row.enabled}
              disabled={isNew}
              onChange={(e) => setRow({ enabled: e.target.checked } as Partial<T>)}
              aria-label="Enabled"
            />
            <input
              className="kv-name"
              value={row.name}
              placeholder={namePlaceholder}
              spellCheck={false}
              onChange={(e) => setRow({ name: e.target.value } as Partial<T>)}
            />
            {renderValue?.(row, setRow) ?? (
              <input
                className="kv-value"
                value={row.value}
                placeholder={valuePlaceholder}
                spellCheck={false}
                onChange={(e) => setRow({ value: e.target.value } as Partial<T>)}
              />
            )}
            {renderExtra?.(row, setRow)}
            <button className="icon-button" onClick={() => remove(i)} disabled={isNew} aria-label="Remove">
              ×
            </button>
          </div>
        );
      })}
    </div>
  );
}

export const blankRow = (): KeyValue => ({ name: "", value: "", enabled: true });
export const blankPart = (): Part => ({ name: "", value: "", enabled: true, file: false, contentType: null });
