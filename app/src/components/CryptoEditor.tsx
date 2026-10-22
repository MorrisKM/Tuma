import { Lock } from "lucide-react";
import { useState } from "react";
import { useStore } from "../store";
import type { CryptoAlgorithm, CryptoEncoding, CryptoSettings } from "../types";

const ALGORITHMS: { id: CryptoAlgorithm; label: string }[] = [
  { id: "aes-256-gcm", label: "AES-256-GCM (recommended)" },
  { id: "aes-128-gcm", label: "AES-128-GCM" },
  { id: "aes-256-cbc", label: "AES-256-CBC" },
  { id: "aes-128-cbc", label: "AES-128-CBC" },
];

/** Paths, one per line, as the text area edits them. */
const lines = (text: string) =>
  text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);

/** Which body fields are encrypted before sending, which response fields are decrypted, and with what key. */
export function CryptoEditor() {
  const crypto = useStore((s) => s.draft.crypto);
  const set = (patch: Partial<CryptoSettings>) =>
    useStore.getState().update({ crypto: { algorithm: "aes-256-gcm", ...crypto, ...patch } });
  const c = crypto ?? {};
  const keyIsLiteral = !!c.key && !c.key.includes("{{");

  return (
    <div className="crypto">
      <p className="hint crypto-intro">
        <Lock size={13} /> For APIs that exchange encrypted fields. Chosen body fields are encrypted just before the
        request is sent, and chosen response fields are decrypted when it arrives, so checks and test scripts see the
        plain values. Each value gets a random IV, written before the ciphertext, unless you set a fixed one.
      </p>
      <div className="form crypto-form">
        <label>
          <span>Algorithm</span>
          <select value={c.algorithm ?? "aes-256-gcm"} onChange={(e) => set({ algorithm: e.target.value as CryptoAlgorithm })}>
            {ALGORITHMS.map((a) => (
              <option key={a.id} value={a.id}>
                {a.label}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Key</span>
          <span className="crypto-pair">
            <input
              className="mono"
              value={c.key ?? ""}
              placeholder="{{encryptionKey}}"
              spellCheck={false}
              onChange={(e) => set({ key: e.target.value })}
            />
            <select
              value={c.keyEncoding ?? "base64"}
              onChange={(e) => set({ keyEncoding: e.target.value as CryptoEncoding })}
              aria-label="Key encoding"
              title="How the key and IV are written"
            >
              <option value="base64">base64</option>
              <option value="hex">hex</option>
              <option value="utf8">text</option>
            </select>
          </span>
          {keyIsLiteral && (
            <small className="warning-text">
              The key is written in the request file. Use a secret variable instead, such as {"{{encryptionKey}}"}, so it
              stays in the keychain.
            </small>
          )}
        </label>
        <label>
          <span>Fixed IV</span>
          <input
            className="mono"
            value={c.iv ?? ""}
            placeholder="Empty: a random IV for every value"
            spellCheck={false}
            onChange={(e) => set({ iv: e.target.value })}
          />
        </label>
        <label>
          <span>Ciphertext as</span>
          <select value={c.output === "hex" ? "hex" : "base64"} onChange={(e) => set({ output: e.target.value as CryptoEncoding })}>
            <option value="base64">base64</option>
            <option value="hex">hex</option>
          </select>
        </label>
      </div>
      <div className="crypto-paths">
        <label>
          <span>Encrypt in the request body</span>
          <PathList paths={c.encrypt ?? []} onChange={(encrypt) => set({ encrypt })} placeholder={"$.card.number\n$.card.cvv\n$.items[*].ssn\n($ for the whole body)"} />
        </label>
        <label>
          <span>Decrypt in the response</span>
          <PathList paths={c.decrypt ?? []} onChange={(decrypt) => set({ decrypt })} placeholder={"$.data.secret\n($ for the whole body)"} />
        </label>
      </div>
      <p className="hint">One JSON path per line. Leave both empty to turn encryption off for this request.</p>
    </div>
  );
}

/** JSON paths, one per line; the text is kept as typed, and only the paths are stored. */
function PathList({ paths, onChange, placeholder }: { paths: string[]; onChange: (paths: string[]) => void; placeholder: string }) {
  const [text, setText] = useState(paths.join("\n"));
  return (
    <textarea
      className="mono"
      rows={4}
      spellCheck={false}
      placeholder={placeholder}
      value={text}
      onChange={(e) => {
        setText(e.target.value);
        onChange(lines(e.target.value));
      }}
    />
  );
}
