// Tuma's script sandbox: the Postman `pm.*` API (and the older `postman.*`, `tests[...]` style) on QuickJS.
//
// Rust defines `__native` (log, send, lib, sleep) and `__inputJson`, runs each script with `__run`, drives timers with
// `__nextTimer` / `__fireTimer`, and reads the results with `__output`. Nothing here can touch files or the network
// except through `__native.send`, which applies the same host rules as the request itself.
"use strict";

(function () {
  const input = JSON.parse(globalThis.__inputJson);
  delete globalThis.__inputJson;
  const native = globalThis.__native;
  delete globalThis.__native;

  const results = []; // Test results, in the order the tests were declared.
  const errors = []; // Errors that stopped a script, or a callback it started.
  const pending = []; // Promises to wait for: async tests and sendRequest callbacks.
  const output = { skipRequest: false, visualizer: null };
  let current = ""; // The script running now, such as "collection Users" or "request".

  const message = (e) => {
    if (e && typeof e === "object" && "message" in e) return e.name && e.name !== "Error" ? `${e.name}: ${e.message}` : e.message;
    return String(e);
  };
  const track = (promise) => {
    pending.push(
      promise.then(
        () => {},
        (e) => errors.push({ script: current, message: message(e) }),
      ),
    );
  };

  // Console

  const inspect = (value) => {
    if (typeof value === "string") return value;
    if (value === undefined) return "undefined";
    if (typeof value === "function") return `[Function ${value.name || "anonymous"}]`;
    if (value instanceof Error) return value.stack ? `${message(value)}\n${value.stack}` : message(value);
    try {
      const json = JSON.stringify(value, (_, v) => (typeof v === "bigint" ? `${v}n` : v), 2);
      return json === undefined ? String(value) : json;
    } catch {
      return String(value);
    }
  };
  const logger = (level) => (...args) => native.log(level, args.map(inspect).join(" "));
  globalThis.console = {
    log: logger("log"),
    info: logger("info"),
    warn: logger("warn"),
    error: logger("error"),
    debug: logger("debug"),
    trace: logger("debug"),
  };

  // Timers, run by Rust after each script until none are left (or the time budget runs out).

  const timers = new Map();
  let timerId = 0;
  globalThis.setTimeout = (fn, ms = 0, ...args) => {
    const id = ++timerId;
    timers.set(id, { at: Date.now() + Math.max(0, Number(ms) || 0), fn, args });
    return id;
  };
  globalThis.setInterval = (fn, ms = 0, ...args) => {
    const id = ++timerId;
    const every = Math.max(1, Number(ms) || 0);
    timers.set(id, { at: Date.now() + every, fn, args, every });
    return id;
  };
  globalThis.setImmediate = (fn, ...args) => globalThis.setTimeout(fn, 0, ...args);
  globalThis.clearTimeout = globalThis.clearInterval = globalThis.clearImmediate = (id) => timers.delete(id);
  const nextTimer = () => {
    let best = null;
    for (const [id, t] of timers) if (!best || t.at < best[1].at) best = [id, t];
    return best;
  };
  globalThis.__nextTimer = () => {
    const best = nextTimer();
    return best ? Math.max(0, best[1].at - Date.now()) : -1;
  };
  globalThis.__fireTimer = () => {
    const best = nextTimer();
    if (!best) return;
    const [id, t] = best;
    if (t.every) t.at = Date.now() + t.every;
    else timers.delete(id);
    try {
      t.fn(...t.args);
    } catch (e) {
      errors.push({ script: current, message: message(e) });
    }
  };

  // Base64, as in browsers.

  const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  globalThis.btoa = (text) => {
    const s = String(text);
    let out = "";
    for (let i = 0; i < s.length; i += 3) {
      const [a, b, c] = [s.charCodeAt(i), s.charCodeAt(i + 1), s.charCodeAt(i + 2)];
      if (a > 255 || b > 255 || c > 255) throw new Error("btoa: the string has characters outside Latin-1");
      const n = (a << 16) | ((b || 0) << 8) | (c || 0);
      out += B64[(n >> 18) & 63] + B64[(n >> 12) & 63] + (i + 1 < s.length ? B64[(n >> 6) & 63] : "=") + (i + 2 < s.length ? B64[n & 63] : "=");
    }
    return out;
  };
  globalThis.atob = (text) => {
    const s = String(text).replace(/[\s=]/g, "");
    let out = "";
    let bits = 0;
    let n = 0;
    for (const ch of s) {
      const v = B64.indexOf(ch === "-" ? "+" : ch === "_" ? "/" : ch);
      if (v < 0) throw new Error("atob: not valid base64");
      n = (n << 6) | v;
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        out += String.fromCharCode((n >> bits) & 255);
      }
    }
    return out;
  };

  // Libraries

  const uuid = () =>
    "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
      const r = (Math.random() * 16) | 0;
      return (c === "x" ? r : (r & 3) | 8).toString(16);
    });
  const modules = new Map([
    ["uuid", { v4: uuid }],
    ["atob", globalThis.atob],
    ["btoa", globalThis.btoa],
  ]);
  const AVAILABLE = "lodash, moment, crypto-js, chai, ajv, tv4, uuid, atob, btoa";
  const load = (name) => {
    if (modules.has(name)) return modules.get(name);
    const source = native.lib(name);
    if (typeof source !== "string") {
      throw new Error(`Cannot find module '${name}'. Scripts in Tuma can require: ${AVAILABLE}`);
    }
    const module = { exports: {} };
    // As in Node: a module required while it loads gets what it has exported so far. lodash reads globalThis._
    // (its own getter below) while it starts.
    modules.set(name, module.exports);
    // Each library sees itself as a CommonJS module, so its UMD wrapper exports instead of setting globals.
    new Function("module", "exports", "require", "define", "global", "window", "self", source)(
      module,
      module.exports,
      load,
      undefined,
      globalThis,
      undefined,
      undefined,
    );
    modules.set(name, module.exports);
    return module.exports;
  };
  globalThis.require = load;
  // Postman's sandbox has these as globals too. They load on first use.
  for (const [global, name] of [
    ["_", "lodash"],
    ["CryptoJS", "crypto-js"],
    ["tv4", "tv4"],
  ]) {
    Object.defineProperty(globalThis, global, { configurable: true, get: () => load(name) });
  }

  // Variables

  const stringify = (value) => (typeof value === "string" ? value : value === undefined ? undefined : typeof value === "object" ? JSON.stringify(value) : String(value));
  const DYNAMIC = {
    $guid: uuid,
    $randomUUID: uuid,
    $timestamp: () => String(Math.floor(Date.now() / 1000)),
    $isoTimestamp: () => new Date().toISOString(),
    $randomInt: () => String(Math.floor(Math.random() * 1001)),
  };
  const replaceWith = (text, lookup) =>
    String(text).replace(/\{\{\s*([^{}]+?)\s*\}\}/g, (whole, name) => {
      const value = DYNAMIC[name] ? DYNAMIC[name]() : lookup(name);
      return value === undefined ? whole : stringify(value);
    });

  class VariableScope {
    constructor(values, name) {
      this._values = values;
      if (name !== undefined) this.name = name;
    }
    get(key) {
      return Object.prototype.hasOwnProperty.call(this._values, key) ? this._values[key] : undefined;
    }
    has(key) {
      return Object.prototype.hasOwnProperty.call(this._values, key);
    }
    set(key, value) {
      this._values[key] = value;
    }
    unset(key) {
      delete this._values[key];
    }
    clear() {
      for (const key of Object.keys(this._values)) delete this._values[key];
    }
    toObject() {
      return { ...this._values };
    }
    toJSON() {
      return this.toObject();
    }
    replaceIn(text) {
      return replaceWith(text, (name) => this.get(name));
    }
  }

  const v = input.vars;
  const scopes = {
    local: new VariableScope({ ...v.local }),
    data: new VariableScope({ ...v.data }),
    environment: new VariableScope({ ...v.environment.values }, v.environment.name ?? undefined),
    folders: v.folders.map((f) => new VariableScope({ ...f })),
    collection: new VariableScope({ ...v.collection }),
    globals: new VariableScope({ ...v.globals }),
  };
  const lookupAll = (name) => {
    for (const scope of [scopes.local, scopes.data, scopes.environment, ...scopes.folders, scopes.collection, scopes.globals]) {
      if (scope.has(name)) return scope.get(name);
    }
    return undefined;
  };
  const variables = {
    get: lookupAll,
    has: (name) => lookupAll(name) !== undefined,
    set: (name, value) => scopes.local.set(name, value),
    unset: (name) => scopes.local.unset(name),
    toObject: () => ({
      ...scopes.globals.toObject(),
      ...scopes.collection.toObject(),
      ...Object.assign({}, ...scopes.folders.map((f) => f.toObject()).reverse()),
      ...scopes.environment.toObject(),
      ...scopes.data.toObject(),
      ...scopes.local.toObject(),
    }),
    replaceIn: (text) => replaceWith(text, lookupAll),
  };

  // Property lists: headers, query parameters, form fields

  const toPair = (item) => {
    if (typeof item === "string") {
      const at = item.indexOf(":");
      return at < 0 ? { key: item.trim(), value: "" } : { key: item.slice(0, at).trim(), value: item.slice(at + 1).trim() };
    }
    return { key: String(item.key ?? item.name ?? ""), value: item.value === undefined ? "" : stringify(item.value), ...(item.disabled ? { disabled: true } : {}), ...(item.type ? { type: item.type } : {}), ...(item.src !== undefined ? { src: item.src } : {}) };
  };

  class PropertyList {
    constructor(items, { ignoreCase = false, onChange = () => {} } = {}) {
      this._items = (items || []).map(toPair);
      this._same = ignoreCase ? (a, b) => String(a).toLowerCase() === String(b).toLowerCase() : (a, b) => a === b;
      this._changed = onChange;
    }
    _find(key) {
      return this._items.find((i) => !i.disabled && this._same(i.key, key));
    }
    get(key) {
      return this._find(key)?.value;
    }
    one(key) {
      return this._find(key);
    }
    has(key, value) {
      const found = this._find(key);
      return !!found && (value === undefined || found.value === value);
    }
    add(item) {
      this._items.push(toPair(item));
      this._changed();
    }
    append(item) {
      this.add(item);
    }
    upsert(item) {
      const pair = toPair(item);
      const found = this._items.find((i) => this._same(i.key, pair.key));
      if (found) Object.assign(found, pair);
      else this._items.push(pair);
      this._changed();
    }
    remove(predicate) {
      const match = typeof predicate === "function" ? predicate : (i) => this._same(i.key, typeof predicate === "object" ? predicate.key : predicate);
      this._items = this._items.filter((i) => !match(i));
      this._changed();
    }
    clear() {
      this._items = [];
      this._changed();
    }
    populate(items) {
      this._items = (items || []).map(toPair);
      this._changed();
    }
    each(fn) {
      this._items.forEach((i) => fn(i));
    }
    all() {
      return this._items.slice();
    }
    map(fn) {
      return this._items.map(fn);
    }
    filter(fn) {
      return this._items.filter(fn);
    }
    find(fn) {
      return this._items.find(fn);
    }
    idx(index) {
      return this._items[index];
    }
    count() {
      return this._items.length;
    }
    get members() {
      return this._items;
    }
    toObject() {
      const out = {};
      for (const i of this._items) if (!i.disabled) out[i.key] = i.value;
      return out;
    }
    toJSON() {
      return this._items;
    }
  }

  // URLs

  class Url {
    constructor(raw) {
      this._set(String(raw));
    }
    _set(raw) {
      this._raw = raw;
      this._dirty = false;
      const m = raw.match(/^(?:([a-zA-Z][\w+.-]*):\/\/)?([^/?#]*)([^?#]*)(?:\?([^#]*))?(?:#(.*))?$/) || [];
      this.protocol = m[1];
      const hostPort = m[2] || "";
      const colon = hostPort.lastIndexOf(":");
      const hasPort = colon > -1 && /^\d+$/.test(hostPort.slice(colon + 1));
      this._host = hasPort ? hostPort.slice(0, colon) : hostPort;
      this.port = hasPort ? hostPort.slice(colon + 1) : undefined;
      this._path = m[3] || "";
      this.hash = m[5];
      const query = (m[4] ?? "").split("&").filter(Boolean).map((part) => {
        const at = part.indexOf("=");
        return at < 0 ? { key: part, value: null } : { key: part.slice(0, at), value: part.slice(at + 1) };
      });
      this.query = new PropertyList(query, { onChange: () => (this._dirty = true) });
    }
    get host() {
      return this._host.split(".");
    }
    set host(value) {
      this._host = Array.isArray(value) ? value.join(".") : String(value);
      this._dirty = true;
    }
    get path() {
      return this._path.split("/").filter(Boolean);
    }
    set path(value) {
      this._path = Array.isArray(value) ? `/${value.join("/")}` : String(value);
      this._dirty = true;
    }
    getHost() {
      return this._host;
    }
    getPath() {
      return this._path || "/";
    }
    getQueryString() {
      return this.query.all().map((q) => (q.value === null || q.value === undefined ? q.key : `${q.key}=${q.value}`)).join("&");
    }
    getPathWithQuery() {
      const q = this.getQueryString();
      return this.getPath() + (q ? `?${q}` : "");
    }
    getRemote() {
      return this._host + (this.port ? `:${this.port}` : "");
    }
    addQueryParams(params) {
      for (const p of typeof params === "string" ? params.split("&").map((s) => ({ key: s.split("=")[0], value: s.split("=").slice(1).join("=") })) : [].concat(params)) {
        this.query.add(p);
      }
    }
    removeQueryParams(keys) {
      for (const key of [].concat(keys)) this.query.remove(typeof key === "object" ? key.key : key);
    }
    update(raw) {
      this._set(typeof raw === "string" ? raw : String(raw?.raw ?? raw));
      this._dirty = true;
    }
    toString() {
      if (!this._dirty) return this._raw;
      const q = this.getQueryString();
      return (this.protocol ? `${this.protocol}://` : "") + this.getRemote() + this._path + (q ? `?${q}` : "") + (this.hash ? `#${this.hash}` : "");
    }
    toJSON() {
      return this.toString();
    }
  }

  // Requests and responses

  const makeBody = (body) => {
    const out = {
      mode: body?.mode,
      raw: body?.raw,
      options: body?.options,
      file: body?.file,
      urlencoded: new PropertyList(body?.urlencoded),
      formdata: new PropertyList(body?.formdata),
      update(value) {
        if (typeof value === "string") {
          this.mode = "raw";
          this.raw = value;
        } else if (value && typeof value === "object") {
          this.mode = value.mode ?? this.mode;
          if ("raw" in value) this.raw = value.raw;
          if (value.urlencoded) this.urlencoded = new PropertyList(value.urlencoded);
          if (value.formdata) this.formdata = new PropertyList(value.formdata);
          if (value.options) this.options = value.options;
        }
      },
      isEmpty() {
        return !this.mode || (this.mode === "raw" && !this.raw);
      },
      toString() {
        return this.mode === "raw" ? this.raw ?? "" : this.mode === "urlencoded" ? this.urlencoded.map((f) => `${encodeURIComponent(f.key)}=${encodeURIComponent(f.value)}`).join("&") : "";
      },
      toJSON() {
        return { mode: this.mode, raw: this.raw, options: this.options, file: this.file, urlencoded: this.urlencoded.all(), formdata: this.formdata.all() };
      },
    };
    return out;
  };

  const makeRequest = (r) => ({
    id: r.id,
    name: r.name,
    method: r.method,
    url: new Url(r.url),
    headers: new PropertyList(r.headers, { ignoreCase: true }),
    body: makeBody(r.body),
    auth: r.auth,
    addHeader(item) {
      this.headers.add(item);
    },
    removeHeader(key) {
      this.headers.remove(key);
    },
    upsertHeader(item) {
      this.headers.upsert(item);
    },
    toJSON() {
      return { method: this.method, url: this.url.toString(), headers: this.headers.all(), body: this.body.toJSON() };
    },
  });

  const parseCookie = (header) => {
    const [pair, ...attrs] = header.split(";");
    const at = pair.indexOf("=");
    const cookie = { name: pair.slice(0, at).trim(), value: pair.slice(at + 1).trim() };
    for (const attr of attrs) {
      const [k, ...rest] = attr.split("=");
      const key = k.trim().toLowerCase();
      const value = rest.join("=").trim();
      if (key === "domain") cookie.domain = value;
      else if (key === "path") cookie.path = value;
      else if (key === "expires") cookie.expires = value;
      else if (key === "max-age") cookie.maxAge = Number(value);
      else if (key === "httponly") cookie.httpOnly = true;
      else if (key === "secure") cookie.secure = true;
      else if (key === "samesite") cookie.sameSite = value;
    }
    return cookie;
  };

  class CookieList {
    constructor(cookies) {
      this._cookies = cookies;
    }
    get(name) {
      return this._cookies.find((c) => c.name === name)?.value;
    }
    one(name) {
      return this._cookies.find((c) => c.name === name);
    }
    has(name) {
      return this._cookies.some((c) => c.name === name);
    }
    all() {
      return this._cookies.slice();
    }
    count() {
      return this._cookies.length;
    }
    each(fn) {
      this._cookies.forEach(fn);
    }
    toObject() {
      return Object.fromEntries(this._cookies.map((c) => [c.name, c.value]));
    }
    jar() {
      const unsupported = (cb) => cb && cb(new Error("pm.cookies.jar() is not supported in Tuma yet"));
      return { get: (_u, _n, cb) => unsupported(cb), getAll: (_u, cb) => unsupported(cb), set: (_u, _n, _v, cb) => unsupported(cb), unset: (_u, _n, cb) => unsupported(cb), clear: (_u, cb) => unsupported(cb) };
    }
  }

  const makeResponse = (r) => {
    const headers = new PropertyList(r.headers, { ignoreCase: true });
    const cookies = new CookieList(r.headers.filter((h) => h.key.toLowerCase() === "set-cookie").map((h) => parseCookie(h.value)));
    let parsed;
    const response = {
      __tumaResponse: true,
      code: r.code,
      status: r.status,
      headers,
      cookies,
      responseTime: r.responseTime,
      responseSize: r.responseSize,
      text: () => r.body,
      json() {
        if (parsed === undefined) {
          try {
            parsed = JSON.parse(r.body);
          } catch (e) {
            throw new SyntaxError(`The response body is not JSON: ${message(e)}`);
          }
        }
        return parsed;
      },
      reason: () => r.status,
      size: () => ({ body: r.responseSize, header: 0, total: r.responseSize }),
      toJSON: () => ({ code: r.code, status: r.status, headers: headers.all(), body: r.body, responseTime: r.responseTime }),
    };
    Object.defineProperty(response, "to", { get: () => chai().expect(response).to });
    return response;
  };

  // Assertions: Chai, with Postman's response assertions added (pm.response.to.have.status(200) and friends).

  let chaiInstance;
  const chai = () => {
    if (chaiInstance) return chaiInstance;
    chaiInstance = load("chai");
    chaiInstance.use((chai, utils) => {
      const A = chai.Assertion;
      const isResponse = (o) => o && o.__tumaResponse;
      const property = (name, check, describe) => {
        const fn = function () {
          const res = this._obj;
          this.assert(check(res), `expected response to ${describe(res)}`, `expected response to not ${describe(res)}`);
        };
        if (name in A.prototype) {
          A.overwriteProperty(name, (base) =>
            function () {
              if (isResponse(this._obj)) fn.call(this);
              else base.call(this);
            },
          );
        } else {
          A.addProperty(name, fn);
        }
      };
      const range = (lo, hi, label) => [(r) => r.code >= lo && r.code <= hi, (r) => `be ${label} but got ${r.code}`];
      for (const [name, lo, hi, label] of [
        ["info", 100, 199, "informational (1XX)"],
        ["success", 200, 299, "successful (2XX)"],
        ["redirection", 300, 399, "a redirection (3XX)"],
        ["clientError", 400, 499, "a client error (4XX)"],
        ["serverError", 500, 599, "a server error (5XX)"],
        ["error", 400, 599, "an error (4XX or 5XX)"],
      ]) {
        property(name, ...range(lo, hi, label));
      }
      property("ok", (r) => r.code === 200, (r) => `have status code 200 but got ${r.code}`);
      for (const [name, code] of [
        ["accepted", 202],
        ["withoutContent", 204],
        ["badRequest", 400],
        ["unauthorized", 401],
        ["unauthorised", 401],
        ["forbidden", 403],
        ["notFound", 404],
        ["notAcceptable", 406],
        ["rateLimited", 429],
      ]) {
        property(name, (r) => r.code === code, (r) => `have status code ${code} but got ${r.code}`);
      }
      const isJson = (r) => {
        try {
          r.json();
          return true;
        } catch {
          return false;
        }
      };
      property("json", isJson, () => "have a JSON body");
      property("withBody", (r) => r.text().length > 0, () => "have a body");

      A.addMethod("status", function (expected) {
        const r = this._obj;
        const actual = typeof expected === "number" ? r.code : r.status;
        this.assert(actual === expected, `expected response to have status ${typeof expected === "number" ? "code" : "reason"} #{exp} but got #{act}`, `expected response to not have status #{exp}`, expected, actual);
      });
      A.addMethod("header", function (key, value) {
        const r = this._obj;
        const actual = r.headers.get(key);
        if (value === undefined) {
          this.assert(actual !== undefined, `expected response to have header ${key}`, `expected response to not have header ${key}`);
        } else {
          this.assert(actual === value, `expected response header ${key} to be #{exp} but got #{act}`, `expected response header ${key} to not be #{exp}`, value, actual);
        }
      });
      A.addMethod("body", function (expected) {
        const text = this._obj.text();
        if (expected === undefined) this.assert(text.length > 0, "expected response to have a body", "expected response to not have a body");
        else if (expected instanceof RegExp) this.assert(expected.test(text), "expected response body to match #{exp}", "expected response body to not match #{exp}", expected);
        else if (typeof expected === "string") this.assert(text === expected, "expected response body to equal #{exp} but got #{act}", "expected response body to not equal #{exp}", expected, text);
        else new A(this._obj.json()).to.deep.equal(expected);
      });
      A.addMethod("jsonBody", function (path, value) {
        const r = this._obj;
        this.assert(isJson(r), "expected response to have a JSON body", "expected response to not have a JSON body");
        if (path === undefined) return;
        const json = r.json();
        if (typeof path === "object") {
          this.assert(load("lodash").isEqual(json, path), "expected response body to deeply equal #{exp}", "expected response body to not deeply equal #{exp}", path, json);
          return;
        }
        const has = load("lodash").has(json, path);
        if (arguments.length < 2) {
          this.assert(has, `expected response body to have property ${path}`, `expected response body to not have property ${path}`);
        } else {
          const actual = load("lodash").get(json, path);
          this.assert(has && load("lodash").isEqual(actual, value), `expected response body property ${path} to equal #{exp} but got #{act}`, `expected response body property ${path} to not equal #{exp}`, value, actual);
        }
      });
      A.addMethod("jsonSchema", function (schema, options) {
        const data = isResponse(this._obj) ? this._obj.json() : this._obj;
        const Ajv = load("ajv");
        const ajv = new Ajv({ allErrors: true, ...(options || {}) });
        const valid = ajv.validate(schema, data);
        const detail = valid ? "" : ajv.errorsText(ajv.errors, { dataVar: "data" });
        this.assert(valid, `expected data to satisfy the schema but found: ${detail}`, "expected data to not satisfy the schema");
      });
    });
    return chaiInstance;
  };

  // Tests

  const test = (name, fn) => {
    name = String(name);
    if (typeof fn !== "function") {
      results.push({ name, skipped: true });
      return pm;
    }
    const entry = { name, passed: true };
    results.push(entry);
    try {
      const result = fn();
      if (result && typeof result.then === "function") {
        entry.pending = true;
        pending.push(
          result.then(
            () => delete entry.pending,
            (e) => {
              delete entry.pending;
              entry.passed = false;
              entry.error = message(e);
            },
          ),
        );
      }
    } catch (e) {
      entry.passed = false;
      entry.error = message(e);
    }
    return pm;
  };
  test.skip = (name) => {
    results.push({ name: String(name), skipped: true });
    return pm;
  };

  // pm.sendRequest

  const sendRequest = (spec, callback) => {
    const req = typeof spec === "string" ? { url: spec } : spec || {};
    const headerList = Array.isArray(req.header) ? req.header.map(toPair) : req.header && typeof req.header === "object" ? Object.entries(req.header).map(([key, value]) => ({ key, value: stringify(value) })) : [];
    const body = req.body
      ? {
          mode: req.body.mode,
          raw: req.body.raw === undefined ? undefined : stringify(req.body.raw),
          urlencoded: (req.body.urlencoded || []).map(toPair),
          formdata: (req.body.formdata || []).map(toPair),
        }
      : undefined;
    const wire = {
      method: String(req.method || "GET").toUpperCase(),
      url: variables.replaceIn(req.url instanceof Url ? req.url.toString() : typeof req.url === "object" && req.url ? req.url.raw ?? "" : req.url),
      headers: headerList.map((h) => ({ ...h, value: variables.replaceIn(h.value) })),
      body,
      timeout: req.timeout,
    };
    const result = JSON.parse(native.send(JSON.stringify(wire)));
    const error = result.error ? new Error(result.error) : null;
    const response = error ? undefined : makeResponse(result.response);
    if (typeof callback === "function") {
      track(Promise.resolve().then(() => callback(error, response)));
      return undefined;
    }
    return error ? Promise.reject(error) : Promise.resolve(response);
  };

  // XML, as Postman's xml2Json does it: attributes under "$", text as a string, repeated elements as arrays.

  const xml2Json = (xml) => {
    const s = String(xml);
    let i = 0;
    const decode = (t) => t.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, "&");
    const skip = () => {
      for (;;) {
        while (i < s.length && /\s/.test(s[i])) i++;
        if (s.startsWith("<?", i)) i = s.indexOf("?>", i) + 2;
        else if (s.startsWith("<!--", i)) i = s.indexOf("-->", i) + 3;
        else if (s.startsWith("<!", i)) i = s.indexOf(">", i) + 1;
        else return;
      }
    };
    const element = () => {
      const open = s.slice(i).match(/^<([^\s/>]+)([^>]*?)(\/?)>/);
      if (!open) throw new Error(`xml2Json: unexpected text at ${i}`);
      i += open[0].length;
      const attrs = {};
      for (const a of open[2].matchAll(/([^\s=]+)\s*=\s*("([^"]*)"|'([^']*)')/g)) attrs[a[1]] = decode(a[3] ?? a[4]);
      const node = Object.keys(attrs).length ? { $: attrs } : {};
      if (open[3]) return [open[1], Object.keys(node).length ? node : ""];
      let text = "";
      for (;;) {
        if (s.startsWith("</", i)) {
          i = s.indexOf(">", i) + 1;
          break;
        }
        if (s.startsWith("<![CDATA[", i)) {
          const end = s.indexOf("]]>", i);
          text += s.slice(i + 9, end);
          i = end + 3;
        } else if (s.startsWith("<!--", i)) {
          i = s.indexOf("-->", i) + 3;
        } else if (s[i] === "<") {
          const [name, child] = element();
          node[name] = name in node ? [].concat(node[name], [child]) : child;
        } else {
          const end = s.indexOf("<", i);
          if (end < 0) throw new Error("xml2Json: unclosed element");
          text += decode(s.slice(i, end));
          i = end;
        }
      }
      text = text.trim();
      const keys = Object.keys(node);
      if (!keys.length) return [open[1], text];
      if (text) node._ = text;
      return [open[1], node];
    };
    skip();
    const [name, value] = element();
    return { [name]: value };
  };
  globalThis.xml2Json = xml2Json;

  // pm

  const info = input.info;
  const execution = {
    skipRequest() {
      if (info.eventName !== "prerequest") throw new Error("pm.execution.skipRequest() works only in pre-request scripts");
      output.skipRequest = true;
    },
    setNextRequest(name) {
      output.nextRequest = name === null || name === undefined ? null : String(name);
    },
    location: Object.assign([...(info.location || [])], { current: info.eventName }),
  };

  const pm = {
    info: { ...info },
    variables,
    environment: scopes.environment,
    collectionVariables: scopes.collection,
    globals: scopes.globals,
    iterationData: scopes.data,
    request: makeRequest(input.request),
    response: input.response ? makeResponse(input.response) : undefined,
    cookies: new CookieList(input.response ? input.response.headers.filter((h) => h.key.toLowerCase() === "set-cookie").map((h) => parseCookie(h.value)) : []),
    test,
    sendRequest,
    execution,
    visualizer: {
      set(template, data, options) {
        output.visualizer = { template: String(template), data: data === undefined ? {} : data, options: options || {} };
      },
      clear() {
        output.visualizer = null;
      },
    },
    vault: {
      // Tuma's vault is its secret variables, read-only here.
      get: (key) => Promise.resolve(input.vault[key]),
      set: () => Promise.reject(new Error("pm.vault.set is not supported: mark the variable as secret in Tuma instead")),
      unset: () => Promise.reject(new Error("pm.vault.unset is not supported in Tuma")),
    },
    require: (name) => {
      throw new Error(`pm.require('${name}'): Postman package libraries are not supported in Tuma. Use require() for ${AVAILABLE}`);
    },
  };
  Object.defineProperty(pm, "expect", { get: () => chai().expect });
  globalThis.pm = pm;

  // The older Postman API, which imported collections still use.

  const res = input.response;
  globalThis.tests = {};
  globalThis.iteration = info.iteration;
  globalThis.data = scopes.data.toObject();
  globalThis.environment = scopes.environment.toObject();
  globalThis.globals = scopes.globals.toObject();
  globalThis.request = { url: input.request.url, method: input.request.method, headers: pm.request.headers.toObject(), data: input.request.body?.raw ?? {} };
  if (res) {
    globalThis.responseBody = res.body;
    globalThis.responseCode = { code: res.code, name: res.status, detail: res.status };
    globalThis.responseHeaders = pm.response.headers.toObject();
    globalThis.responseTime = res.responseTime;
    globalThis.responseCookies = pm.cookies.all();
  }
  globalThis.postman = {
    setEnvironmentVariable: (k, val) => scopes.environment.set(k, val),
    getEnvironmentVariable: (k) => scopes.environment.get(k),
    clearEnvironmentVariable: (k) => scopes.environment.unset(k),
    clearEnvironmentVariables: () => scopes.environment.clear(),
    setGlobalVariable: (k, val) => scopes.globals.set(k, val),
    getGlobalVariable: (k) => scopes.globals.get(k),
    clearGlobalVariable: (k) => scopes.globals.unset(k),
    clearGlobalVariables: () => scopes.globals.clear(),
    getVariable: (k) => lookupAll(k),
    setNextRequest: execution.setNextRequest,
    getResponseHeader: (k) => pm.response?.headers.get(k),
    getResponseCookie: (k) => pm.response?.cookies.one(k),
  };

  // Running

  const AsyncFunction = (async () => {}).constructor;
  globalThis.__run = (label, source) => {
    current = label;
    globalThis.tests = {};
    let fn;
    try {
      fn = new AsyncFunction(source);
    } catch (e) {
      errors.push({ script: label, message: message(e) });
      return;
    }
    track(fn());
  };
  // The legacy tests object, read after each script.
  globalThis.__collectTests = () => {
    for (const [name, passed] of Object.entries(globalThis.tests || {})) results.push({ name, passed: !!passed, ...(passed ? {} : { error: "the test's value was false" }) });
    globalThis.tests = {};
  };

  const strings = (scope) => {
    const out = {};
    for (const [k, value] of Object.entries(scope.toObject())) {
      const s = stringify(value);
      if (s !== undefined) out[k] = s;
    }
    return out;
  };
  globalThis.__output = () =>
    JSON.stringify({
      tests: results.map((r) => (r.pending ? { name: r.name, passed: false, error: "did not finish in time" } : r)),
      errors,
      request: pm.request.toJSON(),
      vars: {
        local: strings(scopes.local),
        environment: strings(scopes.environment),
        collection: strings(scopes.collection),
        globals: strings(scopes.globals),
      },
      skipRequest: output.skipRequest,
      ...("nextRequest" in output ? { nextRequest: output.nextRequest } : {}),
      visualizer: output.visualizer,
    });
})();
