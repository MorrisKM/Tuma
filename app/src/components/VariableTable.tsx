import { useState } from "react";
import type { VariableValue } from "../types";
import { KeyValueEditor } from "./KeyValueEditor";

const blankVariable = (): VariableValue => ({ name: "", value: "", secret: false, enabled: true });

interface Props {
  variables: VariableValue[];
  onChange: (variables: VariableValue[]) => void;
  /** Environments and globals can hold secrets; folder variables cannot. */
  allowSecrets: boolean;
}

/** Variables with an optional "secret" switch. Secret values are hidden until shown. */
export function VariableTable({ variables, onChange, allowSecrets }: Props) {
  const [reveal, setReveal] = useState(false);
  return (
    <>
      <KeyValueEditor
        rows={variables}
        onChange={onChange}
        blank={blankVariable}
        namePlaceholder="Variable"
        renderValue={(row, set) =>
          row.secret ? (
            <input
              className="kv-value"
              type={reveal ? "text" : "password"}
              value={row.value}
              placeholder="Secret value"
              spellCheck={false}
              onChange={(e) => set({ value: e.target.value })}
            />
          ) : null
        }
        renderExtra={
          allowSecrets
            ? (row, set) => (
                <label className="secret-toggle" title="Keep the value in the OS keychain instead of the file">
                  <input type="checkbox" checked={row.secret} onChange={(e) => set({ secret: e.target.checked })} />
                  Secret
                </label>
              )
            : undefined
        }
      />
      {allowSecrets && (
        <div className="toolbar spaced">
          <p className="hint">Secret values are kept in your OS keychain, never in the workspace files.</p>
          {variables.some((v) => v.secret) && (
            <button className="link-button" onClick={() => setReveal(!reveal)}>
              {reveal ? "Hide secrets" : "Show secrets"}
            </button>
          )}
        </div>
      )}
    </>
  );
}
