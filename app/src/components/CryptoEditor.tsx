import { Lock } from "lucide-react";
import { useState } from "react";
import { useStore } from "../store";
import type { CryptoAlgorithm, CryptoEncoding, CryptoSettings, HybridFormat } from "../types";

const ALGORITHMS: { id: CryptoAlgorithm; label: string }[] = [
  { id: "aes-256-gcm", label: "AES-256-GCM (recommended)" },
  { id: "aes-128-gcm", label: "AES-128-GCM" },
  { id: "aes-256-cbc", label: "AES-256-CBC" },
  { id: "aes-128-cbc", label: "AES-128-CBC" },
  { id: "rsa-oaep-sha1", label: "RSA-OAEP, SHA-1 (as node-forge)" },
  { id: "rsa-oaep-sha256", label: "RSA-OAEP, SHA-256" },
];

const HYBRID_DEFAULT: HybridFormat = { separator: ":", wrapped: true, keysBase64: true, hash: "sha1" };

/** Paths, one per line, as the text area edits them. */
const lines = (text: string) =>
  text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);

/** A key written into the request file itself rather than referred to as a {{variable}}. */
const literal = (value?: string) => !!value?.trim() && !value.includes("{{");

/** Which body fields are encrypted before sending, which response fields are decrypted, and with what keys. */
export function CryptoEditor() {
  const crypto = useStore((s) => s.draft.crypto);
  const set = (patch: Partial<CryptoSettings>) =>
    useStore.getState().update({ crypto: { algorithm: "aes-256-gcm", ...crypto, ...patch } });
  const c = crypto ?? {};
  const algorithm = c.algorithm ?? "aes-256-gcm";
  const rsa = algorithm.startsWith("rsa");
  const hybrid = c.hybrid ?? null;
  const needsPrivate = rsa || !!hybrid;

  return (
    <div className="crypto">
      <p className="hint crypto-intro">
        <Lock size={13} /> For APIs that exchange encrypted fields. Chosen body fields are encrypted just before the
        request is sent, and chosen response fields are decrypted when it arrives, so checks and test scripts see the
        plain values. Keep keys in secret variables, which stay in this computer's keychain.
      </p>
      <h4 className="crypto-heading">Request</h4>
      <div className="form crypto-form">
        <label>
          <span>Algorithm</span>
          <select value={algorithm} onChange={(e) => set({ algorithm: e.target.value as CryptoAlgorithm })}>
            {ALGORITHMS.map((a) => (
              <option key={a.id} value={a.id}>
                {a.label}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>{rsa ? "Public key (PEM)" : "Key"}</span>
          <span className="crypto-pair">
            <input
              className="mono"
              value={c.key ?? ""}
              placeholder={rsa ? "{{rsaPublicKey}}" : "{{encryptionKey}}"}
              spellCheck={false}
              onChange={(e) => set({ key: e.target.value })}
            />
            {!rsa && (
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
            )}
          </span>
          {literal(c.key) && !rsa && (
            <small className="warning-text">
              The key is written in the request file. Use a secret variable instead, such as {"{{encryptionKey}}"}, so it
              stays in the keychain.
            </small>
          )}
          {rsa && <small className="muted">A PEM public key; one line, as in a .env file, works too.</small>}
        </label>
        {!rsa && (
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
        )}
        <label>
          <span>Ciphertext as</span>
          <select value={c.output === "hex" ? "hex" : "base64"} onChange={(e) => set({ output: e.target.value as CryptoEncoding })}>
            <option value="base64">base64</option>
            <option value="hex">hex</option>
          </select>
        </label>
      </div>
      <h4 className="crypto-heading">Response</h4>
      <div className="form crypto-form">
        <label>
          <span>Encrypted as</span>
          <select
            value={hybrid ? "hybrid" : "same"}
            onChange={(e) => set({ hybrid: e.target.value === "hybrid" ? HYBRID_DEFAULT : null })}
          >
            <option value="same">{rsa ? "RSA-OAEP, as above" : "With the key above"}</option>
            <option value="hybrid">Hybrid: an RSA-encrypted AES key and IV with the data</option>
          </select>
          {hybrid && (
            <small className="muted">
              base64 of <code>RSA(key){hybrid.separator}RSA(iv){hybrid.separator}AES-CBC(data)</code>; the private key
              unwraps the AES key and IV.
            </small>
          )}
        </label>
        {hybrid && (
          <>
            <label>
              <span>Format</span>
              <span className="crypto-options">
                <span className="example-field">
                  Separator
                  <input
                    className="mono crypto-separator"
                    value={hybrid.separator}
                    onChange={(e) => set({ hybrid: { ...hybrid, separator: e.target.value } })}
                  />
                </span>
                <select
                  value={hybrid.hash}
                  onChange={(e) => set({ hybrid: { ...hybrid, hash: e.target.value as "sha1" | "sha256" } })}
                  aria-label="RSA-OAEP hash"
                >
                  <option value="sha1">OAEP SHA-1</option>
                  <option value="sha256">OAEP SHA-256</option>
                </select>
              </span>
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={hybrid.wrapped}
                onChange={(e) => set({ hybrid: { ...hybrid, wrapped: e.target.checked } })}
              />
              <span>The joined parts are base64 again</span>
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={hybrid.keysBase64}
                onChange={(e) => set({ hybrid: { ...hybrid, keysBase64: e.target.checked } })}
              />
              <span>The decrypted AES key and IV are base64 text</span>
            </label>
          </>
        )}
        {needsPrivate && (
          <label>
            <span>Private key (PEM)</span>
            <input
              className="mono"
              value={c.privateKey ?? ""}
              placeholder="{{rsaPrivateKey}}"
              spellCheck={false}
              onChange={(e) => set({ privateKey: e.target.value })}
            />
            {literal(c.privateKey) ? (
              <small className="danger">
                A private key is written into the request file, which goes into git with the workspace. Make it a secret
                variable such as {"{{rsaPrivateKey}}"}, and treat a key that was shared as exposed.
              </small>
            ) : (
              <small className="muted">For decrypting. Put the PEM in a secret variable; it never leaves the keychain.</small>
            )}
          </label>
        )}
      </div>
      <div className="crypto-paths">
        <label>
          <span>Encrypt in the request body</span>
          <PathList
            paths={c.encrypt ?? []}
            onChange={(encrypt) => set({ encrypt })}
            placeholder={rsa ? "$.pin\n($ for the whole body)" : "$.card.number\n$.items[*].ssn\n($ for the whole body)"}
          />
        </label>
        <label>
          <span>Decrypt in the response</span>
          <PathList
            paths={c.decrypt ?? []}
            onChange={(decrypt) => set({ decrypt })}
            placeholder={"$ for the whole body\n$.data"}
          />
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
