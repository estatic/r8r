// r8r expression prelude: runs once per VM after luxon.min.js and
// jmespath.js. Provides n8n's data proxy and extension methods, a minimal
// Intl for Luxon (QuickJS has none), and closes off host escapes last.
(function () {
  'use strict';
  const g = globalThis;

  // ---- Intl (English only) and IANA zones backed by the host -------------
  const pad2 = (n) => String(n).padStart(2, '0');
  function formatOffset(offset, format) {
    const hours = Math.trunc(Math.abs(offset / 60));
    const minutes = Math.trunc(Math.abs(offset % 60));
    const sign = offset >= 0 ? '+' : '-';
    switch (format) {
      case 'short': return `${sign}${pad2(hours)}:${pad2(minutes)}`;
      case 'narrow': return `${sign}${hours}${minutes > 0 ? `:${minutes}` : ''}`;
      case 'techie': return `${sign}${pad2(hours)}${pad2(minutes)}`;
      default: throw new RangeError(`Value format ${format} is out of range for property format`);
    }
  }
  class HostZone extends luxon.Zone {
    constructor(name) { super(); this.zoneName = name; }
    get type() { return 'iana'; }
    get name() { return this.zoneName; }
    get ianaName() { return this.zoneName; }
    get isUniversal() { return false; }
    offsetName(ts) { return __r8r_tz_abbr(this.zoneName, ts); }
    formatOffset(ts, format) { return formatOffset(this.offset(ts), format); }
    offset(ts) { return __r8r_tz_offset(this.zoneName, ts); }
    equals(other) { return !!other && other.type === 'iana' && other.name === this.zoneName; }
    get isValid() { return true; }
  }
  const zoneCache = {};
  luxon.IANAZone.create = function (name) {
    if (!__r8r_tz_valid(name)) return new luxon.InvalidZone(name);
    return zoneCache[name] || (zoneCache[name] = new HostZone(name));
  };
  luxon.IANAZone.isValidZone = (name) => __r8r_tz_valid(name);
  luxon.IANAZone.isValidSpecifier = (name) => __r8r_tz_valid(name);

  class DateTimeFormat {
    constructor(locale, opts) { this.opts = opts || {}; }
    resolvedOptions() {
      return { locale: 'en-US', calendar: 'gregory', numberingSystem: 'latn', timeZone: this.opts.timeZone || __r8r_timezone };
    }
    format(date) { return new Date(date === undefined ? Date.now() : date).toISOString(); }
    formatToParts(date) { return [{ type: 'literal', value: this.format(date) }]; }
  }
  function groupDigits(n, opts) {
    opts = opts || {};
    const max = opts.maximumFractionDigits !== undefined ? opts.maximumFractionDigits : 3;
    const min = opts.minimumFractionDigits || 0;
    let [int, frac = ''] = Math.abs(n).toFixed(max).split('.');
    frac = frac.replace(/0+$/, '');
    while (frac.length < min) frac += '0';
    if (opts.useGrouping !== false) int = int.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
    return (n < 0 ? '-' : '') + int + (frac ? '.' + frac : '');
  }
  class NumberFormat {
    constructor(locale, opts) { this.opts = opts || {}; }
    format(n) { return groupDigits(Number(n), this.opts); }
    resolvedOptions() { return { locale: 'en-US', numberingSystem: 'latn' }; }
  }
  g.Intl = g.Intl || {
    DateTimeFormat,
    NumberFormat,
    PluralRules: class { select(n) { return n === 1 ? 'one' : 'other'; } resolvedOptions() { return { locale: 'en-US' }; } },
    RelativeTimeFormat: class { format(v, unit) { return v < 0 ? `${-v} ${unit} ago` : `in ${v} ${unit}`; } },
    Locale: class { constructor(tag) { this.baseName = tag; } },
    getCanonicalLocales: (l) => (Array.isArray(l) ? l : [l || 'en-US']),
  };
  luxon.Settings.defaultLocale = 'en-US';
  luxon.Settings.defaultZone = __r8r_timezone;
  g.DateTime = luxon.DateTime;
  g.Duration = luxon.Duration;
  g.Interval = luxon.Interval;

  // ---- result encoding ------------------------------------------------------
  function encodeValue(v) {
    if (v === undefined || typeof v === 'function' || typeof v === 'symbol') return '{"u":1}';
    return JSON.stringify({ v: v });
  }
  function encodeError(e) {
    const message = e && e.message !== undefined ? String(e.message) : String(e);
    return JSON.stringify({ e: { message, name: (e && e.name) || 'Error', stack: (e && e.stack) || '' } });
  }
  g.__r8r_run = function (fn) {
    try { return encodeValue(fn()); } catch (e) { return encodeError(e); }
  };
  g.__r8r_run_async = function (fn) {
    g.__r8r_async_done = false;
    g.__r8r_async_result = '';
    let p;
    try { p = fn(); } catch (e) { g.__r8r_async_result = encodeError(e); g.__r8r_async_done = true; return '{"u":1}'; }
    Promise.resolve(p).then(
      (v) => { g.__r8r_async_result = encodeValue(v); g.__r8r_async_done = true; },
      (e) => { g.__r8r_async_result = encodeError(e); g.__r8r_async_done = true; }
    );
    return '{"u":1}';
  };

  // ---- data proxy -------------------------------------------------------------
  function deepFreeze(o) {
    if (o && typeof o === 'object' && !Object.isFrozen(o)) {
      Object.freeze(o);
      for (const k of Object.keys(o)) deepFreeze(o[k]);
    }
    return o;
  }
  class ExpressionError extends Error {
    constructor(message) { super(message); this.name = 'ExpressionError'; }
  }
  g.ExpressionError = ExpressionError;

  let D = { input: [], inputs: [[]], source: [], runIndex: 0, workflow: {}, execution: {}, vars: {}, env: null, node: {} };
  const custom = {};
  g.__r8r_custom = custom;
  g.__r8r_static = { global: {}, node: {} };
  let extra = {};

  g.__r8r_set_data = function (data) {
    D = __r8r_freeze ? deepFreeze(data) : data;
    if (data.staticData) {
      g.__r8r_static.global = data.staticData.global || {};
      g.__r8r_static.node = data.staticData.node || {};
    }
    if (data.customData) Object.assign(custom, data.customData);
    g.__r8r_set_item(0);
  };
  g.__r8r_set_extra = function (values) { extra = values || {}; for (const k of Object.keys(extra)) g[k] = extra[k]; };
  g.__r8r_set_item = function (i) {
    g.$itemIndex = i;
    const item = D.input[i];
    g.$json = item ? item.json : {};
    g.$binary = (item && item.binary) || {};
  };

  // Run data is fetched from the host per task, on first use.
  const tasks = new Map();
  function runCount(name) { return __r8r_task_count(String(name)); }
  function taskOf(name, run) {
    const r = run === undefined || run === null ? -1 : run;
    const key = name + '\u0000' + r;
    if (!tasks.has(key)) {
      const text = __r8r_task(String(name), r);
      const task = text === undefined || text === null ? null : JSON.parse(text);
      tasks.set(key, task && __r8r_freeze ? deepFreeze(task) : task);
    }
    return tasks.get(key);
  }
  function outputOf(name, output, run) {
    const task = taskOf(name, run);
    if (!task || !task.data || !task.data.main) return [];
    return task.data.main[output || 0] || [];
  }
  function requireExecuted(name) {
    if (runCount(name) < 0) {
      if (!D.nodeNames || D.nodeNames.indexOf(name) === -1) {
        throw new ExpressionError(`Referenced node doesn't exist: '${name}'`);
      }
      throw new ExpressionError(`An expression references the node '${name}', but it hasn’t been executed yet`);
    }
  }
  function pairedIndex(p) {
    if (p === undefined || p === null) return null;
    if (typeof p === 'number') return { item: p, input: 0 };
    if (Array.isArray(p)) return p.length ? pairedIndex(p[0]) : null;
    return { item: p.item || 0, input: p.input || 0 };
  }
  // Follows pairedItem links from the current input item back to `target`.
  function pairedItemOf(target, itemIndex) {
    const src = D.source && D.source[0];
    if (!src) throw new ExpressionError(`Can't get data for expression: no path back to node '${target}'`);
    let node = src.previousNode, output = src.previousNodeOutput || 0, run = src.previousNodeRun || 0;
    let idx = itemIndex;
    for (let hops = 0; hops < 1000; hops++) {
      const task = taskOf(node, run);
      const items = (task && task.data && task.data.main && task.data.main[output]) || [];
      const item = items[idx];
      if (node === target) {
        if (!item) throw new ExpressionError(`Can't get data for expression: item ${idx} of '${target}' does not exist`);
        return item;
      }
      if (!item) throw new ExpressionError(`Can't get data for expression: no paired item from '${node}' to '${target}'`);
      const p = pairedIndex(item.pairedItem);
      if (!p) throw new ExpressionError(`Paired item data for item from node '${node}' is unavailable`);
      const source = task.source && task.source[p.input];
      if (!source) throw new ExpressionError(`Can't get data for expression: '${target}' is not an ancestor of '${D.node.name}'`);
      node = source.previousNode; output = source.previousNodeOutput || 0; run = source.previousNodeRun || 0; idx = p.item;
    }
    throw new ExpressionError('Paired item chain is too long');
  }

  g.$input = {
    all(branch, run) { return branch ? (D.inputs[branch] || []) : D.input; },
    first(branch) { return this.all(branch)[0]; },
    last(branch) { const a = this.all(branch); return a[a.length - 1]; },
    get item() { return D.input[g.$itemIndex]; },
    get params() { return D.node.parameters || {}; },
    get context() { return {}; },
  };
  g.$ = function (name) {
    requireExecuted(name);
    return {
      all(branch, run) { return outputOf(name, branch, run); },
      first(branch, run) { return outputOf(name, branch, run)[0]; },
      last(branch, run) { const a = outputOf(name, branch, run); return a[a.length - 1]; },
      get item() { return pairedItemOf(name, g.$itemIndex); },
      pairedItem(i) { return pairedItemOf(name, i === undefined ? g.$itemIndex : i); },
      itemMatching(i) { return pairedItemOf(name, i); },
      get isExecuted() { return true; },
      get params() { return {}; },
      get context() { return {}; },
      get runIndex() { return Math.max(runCount(name), 0) - 1; },
    };
  };
  g.$node = new Proxy({}, {
    get(_, name) {
      if (typeof name !== 'string') return undefined;
      requireExecuted(name);
      const items = outputOf(name, 0);
      const item = items[g.$itemIndex] || items[0] || { json: {} };
      return { json: item.json, binary: item.binary || {}, parameter: {}, runIndex: Math.max(runCount(name), 0) - 1, context: {} };
    },
  });
  g.$items = function (name, output, run) {
    if (name === undefined) return D.input;
    requireExecuted(name);
    return outputOf(name, output, run);
  };
  Object.defineProperty(g, '$prevNode', {
    get() {
      const s = D.source && D.source[0];
      return s ? { name: s.previousNode, outputIndex: s.previousNodeOutput || 0, runIndex: s.previousNodeRun || 0 } : {};
    },
  });
  Object.defineProperty(g, '$runIndex', { get() { return D.runIndex || 0; } });
  Object.defineProperty(g, '$workflow', { get() { return D.workflow; } });
  Object.defineProperty(g, '$vars', { get() { return D.vars || {}; } });
  Object.defineProperty(g, '$execution', {
    get() {
      const e = D.execution || {};
      return {
        id: e.id,
        mode: e.mode === 'manual' ? 'test' : 'production',
        resumeUrl: e.resumeUrl,
        resumeFormUrl: e.resumeFormUrl,
        customData: {
          set(k, v) { custom[String(k)] = String(v); },
          get(k) { return custom[k]; },
          setAll(o) { for (const k of Object.keys(o)) custom[k] = String(o[k]); },
          getAll() { return Object.assign({}, custom); },
        },
      };
    },
  });
  Object.defineProperty(g, '$env', {
    get() {
      if (!D.env) throw new ExpressionError('access to env vars denied');
      return D.env;
    },
  });
  Object.defineProperty(g, '$now', { get() { return luxon.DateTime.now(); } });
  Object.defineProperty(g, '$today', { get() { return luxon.DateTime.now().startOf('day'); } });
  g.$jmespath = (data, expr) => jmespath.search(data, expr);
  g.$if = (c, a, b) => (c ? a : b);
  g.$ifEmpty = (v, d) => (v === undefined || v === null || v === '' || (Array.isArray(v) && !v.length) || (typeof v === 'object' && !Array.isArray(v) && !Object.keys(v).length) ? d : v);
  g.$max = (...a) => Math.max(...a);
  g.$min = (...a) => Math.min(...a);
  g.$getWorkflowStaticData = (type) => (type === 'node' ? g.__r8r_static.node : g.__r8r_static.global);

  // ---- base64 / utf8 / Buffer ------------------------------------------------
  const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  function utf8Encode(s) {
    const out = [];
    for (const ch of String(s)) {
      let c = ch.codePointAt(0);
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return out;
  }
  function utf8Decode(bytes) {
    let s = '';
    for (let i = 0; i < bytes.length;) {
      const b = bytes[i++];
      if (b < 0x80) s += String.fromCodePoint(b);
      else if (b < 0xe0) s += String.fromCodePoint(((b & 31) << 6) | (bytes[i++] & 63));
      else if (b < 0xf0) { const c = ((b & 15) << 12) | ((bytes[i++] & 63) << 6) | (bytes[i++] & 63); s += String.fromCodePoint(c); }
      else { const c = ((b & 7) << 18) | ((bytes[i++] & 63) << 12) | ((bytes[i++] & 63) << 6) | (bytes[i++] & 63); s += String.fromCodePoint(c); }
    }
    return s;
  }
  function b64Encode(bytes) {
    let out = '';
    for (let i = 0; i < bytes.length; i += 3) {
      const n = (bytes[i] << 16) | ((bytes[i + 1] || 0) << 8) | (bytes[i + 2] || 0);
      out += B64[(n >> 18) & 63] + B64[(n >> 12) & 63] + (i + 1 < bytes.length ? B64[(n >> 6) & 63] : '=') + (i + 2 < bytes.length ? B64[n & 63] : '=');
    }
    return out;
  }
  function b64Decode(s) {
    s = String(s).replace(/[^A-Za-z0-9+/]/g, '');
    const out = [];
    for (let i = 0; i < s.length; i += 4) {
      const n = (B64.indexOf(s[i]) << 18) | (B64.indexOf(s[i + 1]) << 12) | ((B64.indexOf(s[i + 2]) & 63) << 6) | (B64.indexOf(s[i + 3]) & 63);
      out.push((n >> 16) & 255);
      if (s[i + 2] !== undefined) out.push((n >> 8) & 255);
      if (s[i + 3] !== undefined) out.push(n & 255);
    }
    return out;
  }
  g.__r8r_b64 = { encode: (s) => b64Encode(utf8Encode(s)), decode: (s) => utf8Decode(b64Decode(s)) };
  class Buffer extends Uint8Array {
    static from(v, enc) {
      let bytes;
      if (typeof v === 'string') bytes = enc === 'base64' ? b64Decode(v) : enc === 'hex' ? (v.match(/../g) || []).map((h) => parseInt(h, 16)) : utf8Encode(v);
      else bytes = Array.from(v);
      const b = new Buffer(bytes.length);
      b.set(bytes);
      return b;
    }
    static isBuffer(v) { return v instanceof Buffer; }
    toString(enc) {
      const bytes = Array.from(this);
      if (enc === 'base64') return b64Encode(bytes);
      if (enc === 'hex') return bytes.map((b) => b.toString(16).padStart(2, '0')).join('');
      return utf8Decode(bytes);
    }
  }
  g.Buffer = Buffer;
  g.btoa = (s) => b64Encode(Array.from(String(s), (c) => c.charCodeAt(0) & 255));
  g.atob = (s) => String.fromCharCode(...b64Decode(s));

  // ---- extension methods (n8n expression language) --------------------------
  function ext(proto, name, fn) {
    Object.defineProperty(proto, name, { value: fn, writable: true, configurable: true, enumerable: false });
  }
  const words = (s) => String(s).replace(/([a-z0-9])([A-Z])/g, '$1 $2').split(/[^A-Za-z0-9]+/).filter(Boolean);
  const EMAIL = /[\w.+-]+@[\w-]+\.[\w.-]+/;
  const URL_RE = /(https?:\/\/[^\s"'<>]+)/;
  const S = String.prototype;
  ext(S, 'toSnakeCase', function () { return words(this).map((w) => w.toLowerCase()).join('_'); });
  ext(S, 'toKebabCase', function () { return words(this).map((w) => w.toLowerCase()).join('-'); });
  ext(S, 'toTitleCase', function () { return String(this).replace(/\b([a-z])/g, (m) => m.toUpperCase()); });
  ext(S, 'toSentenceCase', function () { const s = String(this).toLowerCase(); return s.charAt(0).toUpperCase() + s.slice(1); });
  ext(S, 'extractEmail', function () { const m = String(this).match(EMAIL); return m ? m[0] : undefined; });
  ext(S, 'extractUrl', function () { const m = String(this).match(URL_RE); return m ? m[1] : undefined; });
  ext(S, 'extractDomain', function () {
    const s = String(this);
    const email = s.match(/@([\w.-]+\.[a-z]{2,})/i);
    if (email && !/^https?:/i.test(s)) return email[1];
    const m = s.match(/^(?:[a-z][a-z0-9+.-]*:\/\/)?([^/?#:]+)/i);
    return m ? m[1] : undefined;
  });
  ext(S, 'extractUrlPath', function () { const m = String(this).match(/^[a-z]+:\/\/[^/]+(\/[^?#]*)/i); return m ? m[1] : undefined; });
  ext(S, 'removeTags', function () { return String(this).replace(/<[^>]*>/g, ''); });
  ext(S, 'removeMarkdown', function () { return String(this).replace(/[*_`#>~]+/g, '').replace(/\[([^\]]*)\]\([^)]*\)/g, '$1'); });
  ext(S, 'isEmpty', function () { return String(this).length === 0; });
  ext(S, 'isNotEmpty', function () { return String(this).length > 0; });
  ext(S, 'isEmail', function () { return new RegExp('^' + EMAIL.source + '$').test(String(this)); });
  ext(S, 'isUrl', function () { return /^https?:\/\/[^\s]+$/.test(String(this)); });
  ext(S, 'isDomain', function () { return /^([a-z0-9-]+\.)+[a-z]{2,}$/i.test(String(this)); });
  ext(S, 'isNumeric', function () { return String(this).trim() !== '' && !isNaN(Number(this)); });
  ext(S, 'toNumber', function () {
    const n = Number(String(this).trim());
    if (String(this).trim() === '' || isNaN(n)) throw new ExpressionError(`'${this}' can't be converted to a number`);
    return n;
  });
  ext(S, 'toInt', function () { return parseInt(String(this), 10); });
  ext(S, 'toFloat', function () { return parseFloat(String(this)); });
  ext(S, 'toBoolean', function () { return !['false', '0', 'no', 'n', 'off', ''].includes(String(this).trim().toLowerCase()); });
  ext(S, 'hash', function (alg) { return __r8r_hash(alg || 'md5', String(this)); });
  ext(S, 'base64Encode', function () { return g.__r8r_b64.encode(String(this)); });
  ext(S, 'base64Decode', function () { return g.__r8r_b64.decode(String(this)); });
  ext(S, 'urlEncode', function (all) { return all ? encodeURI(String(this)) : encodeURIComponent(String(this)); });
  ext(S, 'urlDecode', function (all) { return all ? decodeURI(String(this)) : decodeURIComponent(String(this)); });
  ext(S, 'quote', function (q) { q = q || '"'; return q + String(this).split(q).join('\\' + q) + q; });
  ext(S, 'parseJson', function () { return JSON.parse(String(this)); });
  ext(S, 'toJsonString', function () { return JSON.stringify(String(this)); });
  ext(S, 'replaceSpecialChars', function () { return String(this).normalize('NFD').replace(/[̀-ͯ]/g, ''); });
  ext(S, 'toDateTime', function () {
    const s = String(this);
    const tries = [luxon.DateTime.fromISO, luxon.DateTime.fromRFC2822, luxon.DateTime.fromHTTP, luxon.DateTime.fromSQL];
    for (const f of tries) { const d = f(s); if (d.isValid) return d; }
    const d = new Date(s);
    if (!isNaN(d.getTime())) return luxon.DateTime.fromJSDate(d);
    throw new ExpressionError(`'${s}' can't be converted to a date`);
  });

  const N = Number.prototype;
  ext(N, 'round', function (d) { d = d || 0; return Number(Math.round(Number(this + 'e' + d)) + 'e-' + d); });
  ext(N, 'ceil', function () { return Math.ceil(this); });
  ext(N, 'floor', function () { return Math.floor(this); });
  ext(N, 'abs', function () { return Math.abs(this); });
  ext(N, 'isEven', function () { return Number(this) % 2 === 0; });
  ext(N, 'isOdd', function () { return Math.abs(Number(this) % 2) === 1; });
  ext(N, 'isInteger', function () { return Number.isInteger(Number(this)); });
  ext(N, 'toBoolean', function () { return Number(this) !== 0; });
  ext(N, 'format', function (locale, opts) { return groupDigits(Number(this), opts); });
  ext(N, 'toDateTime', function (unit) { return unit === 's' ? luxon.DateTime.fromSeconds(Number(this)) : luxon.DateTime.fromMillis(Number(this)); });

  const A = Array.prototype;
  const pick = (o, f) => (o === null || o === undefined ? undefined : o[f]);
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  ext(A, 'unique', function (...fields) {
    const seen = [];
    return this.filter((v) => {
      const key = fields.length ? fields.map((f) => pick(v, f)) : v;
      if (seen.some((s) => same(s, key))) return false;
      seen.push(key);
      return true;
    });
  });
  ext(A, 'removeDuplicates', A.unique);
  ext(A, 'pluck', function (...fields) {
    if (!fields.length) return this.slice();
    return this.map((v) => (fields.length === 1 ? pick(v, fields[0]) : Object.fromEntries(fields.map((f) => [f, pick(v, f)]))));
  });
  ext(A, 'sum', function () { return this.reduce((s, v) => s + Number(v), 0); });
  ext(A, 'max', function () { return Math.max(...this); });
  ext(A, 'min', function () { return Math.min(...this); });
  ext(A, 'average', function () { return this.length ? this.sum() / this.length : 0; });
  ext(A, 'chunk', function (n) { const out = []; for (let i = 0; i < this.length; i += n) out.push(this.slice(i, i + n)); return out; });
  ext(A, 'first', function () { return this[0]; });
  ext(A, 'last', function () { return this[this.length - 1]; });
  ext(A, 'compact', function () { return this.filter((v) => v !== null && v !== undefined && v !== ''); });
  ext(A, 'isEmpty', function () { return this.length === 0; });
  ext(A, 'isNotEmpty', function () { return this.length > 0; });
  ext(A, 'difference', function (o) { return this.filter((v) => !o.some((w) => same(v, w))); });
  ext(A, 'intersection', function (o) { return this.filter((v) => o.some((w) => same(v, w))).unique(); });
  ext(A, 'union', function (o) { return this.concat(o).unique(); });
  ext(A, 'smartJoin', function (k, v) { const out = {}; for (const o of this) out[o[k]] = o[v]; return out; });
  ext(A, 'merge', function () { return Object.assign({}, ...this); });
  ext(A, 'randomItem', function () { return this[Math.floor(Math.random() * this.length)]; });
  ext(A, 'append', function (...v) { return this.concat(v); });
  ext(A, 'toJsonString', function () { return JSON.stringify(this); });

  const O = Object.prototype;
  ext(O, 'compact', function () { const out = {}; for (const [k, v] of Object.entries(this)) if (v !== null && v !== undefined && v !== '') out[k] = v; return out; });
  ext(O, 'keys', function () { return Object.keys(this); });
  ext(O, 'values', function () { return Object.values(this); });
  ext(O, 'hasField', function (k) { return Object.prototype.hasOwnProperty.call(this, k); });
  ext(O, 'removeField', function (k) { const out = Object.assign({}, this); delete out[k]; return out; });
  ext(O, 'keepFieldsContaining', function (s) {
    const out = {};
    for (const [k, v] of Object.entries(this)) if (typeof v === 'string' && v.includes(s)) out[k] = v;
    return out;
  });
  ext(O, 'removeFieldsContaining', function (s) {
    const out = {};
    for (const [k, v] of Object.entries(this)) if (!(typeof v === 'string' && v.includes(s))) out[k] = v;
    return out;
  });
  ext(O, 'isEmpty', function () { return Object.keys(this).length === 0; });
  ext(O, 'isNotEmpty', function () { return Object.keys(this).length > 0; });
  ext(O, 'toJsonString', function () { return JSON.stringify(this); });
  ext(O, 'urlEncode', function () { return Object.entries(this).map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`).join('&'); });

  const DT = luxon.DateTime.prototype;
  const plus = DT.plus, minus = DT.minus;
  ext(DT, 'plus', function (a, unit) { return plus.call(this, typeof a === 'number' ? { [unit || 'milliseconds']: a } : a); });
  ext(DT, 'minus', function (a, unit) { return minus.call(this, typeof a === 'number' ? { [unit || 'milliseconds']: a } : a); });
  ext(DT, 'format', function (f) { return this.toFormat(f); });
  ext(DT, 'isWeekend', function () { return this.weekday > 5; });
  ext(DT, 'beginningOf', function (u) { return this.startOf(u); });
  ext(DT, 'endOfMonth', function () { return this.endOf('month'); });
  ext(DT, 'extract', function (u) { return this.get(u || 'hour'); });
  ext(DT, 'toDateTime', function () { return this; });

  // ---- sandbox: no route from data or functions back to a code generator -----
  const blocked = function () { throw new Error('Code generation from strings is not allowed in expressions'); };
  for (const F of [
    Function,
    Object.getPrototypeOf(function* () {}).constructor,
    Object.getPrototypeOf(async function () {}).constructor,
    Object.getPrototypeOf(async function* () {}).constructor,
  ]) {
    Object.defineProperty(F.prototype, 'constructor', { value: blocked, writable: false, configurable: false });
  }
  g.Function = blocked;
  g.eval = blocked;

  // ---- pooling: expression VMs are reused between node runs -----------------
  // `__r8r_harden` freezes the built-ins and the prelude's globals so an
  // expression can't leave anything behind for the next run; `__r8r_reset`
  // then clears the per-run state before reuse.
  const MUTABLE = new Set(['$json', '$binary', '$itemIndex', '__r8r_timezone', '__r8r_async_done', '__r8r_async_result']);
  let baseline = null;
  const restorable = [];
  g.__r8r_harden = function () {
    // Built-ins and Luxon are snapshotted here and restored on reset.
    // (Freezing them instead breaks code that assigns properties shadowing
    // a frozen prototype's, e.g. Luxon setting `values` on its objects.)
    const shared = [Object, Array, String, Number, Boolean, Date, RegExp, Map, Set, WeakMap, WeakSet, Promise, Symbol, Function, Error,
      TypeError, RangeError, SyntaxError, luxon.DateTime, luxon.Duration, luxon.Interval, luxon.Info, luxon.Zone, luxon.FixedOffsetZone,
      luxon.IANAZone, luxon.Settings];
    for (const C of shared) {
      if (!C) continue;
      for (const o of [C, C.prototype]) {
        if (o) restorable.push([o, Object.getOwnPropertyDescriptors(o)]);
      }
    }
    for (const o of [Math, JSON, Reflect, jmespath, luxon]) restorable.push([o, Object.getOwnPropertyDescriptors(o)]);
    const names = Object.getOwnPropertyNames(g);
    for (const k of names) {
      if (MUTABLE.has(k)) continue;
      const d = Object.getOwnPropertyDescriptor(g, k);
      if (!d.configurable) continue;
      if ('value' in d) d.writable = false;
      d.configurable = false;
      Object.defineProperty(g, k, d);
    }
    baseline = new Set(names);
  };
  g.__r8r_reset = function (tz) {
    if (baseline) {
      for (const k of Object.getOwnPropertyNames(g)) if (!baseline.has(k)) delete g[k];
    }
    // A VM whose shared objects can't be put back (frozen, or given
    // non-configurable properties) throws here and is discarded.
    for (const [o, descs] of restorable) {
      if (!Object.isExtensible(o)) throw new Error('tainted VM');
      for (const k of Reflect.ownKeys(o)) {
        if (!(k in descs) && !delete o[k]) throw new Error('tainted VM');
      }
      for (const k of Reflect.ownKeys(descs)) Object.defineProperty(o, k, descs[k]);
    }
    g.__r8r_timezone = tz;
    luxon.Settings.defaultZone = tz;
    D = { input: [], inputs: [[]], source: [], runIndex: 0, workflow: {}, execution: {}, vars: {}, env: null, node: {} };
    tasks.clear();
    for (const k of Object.keys(custom)) delete custom[k];
    g.__r8r_static.global = {};
    g.__r8r_static.node = {};
    extra = {};
    g.$json = {};
    g.$binary = {};
    g.$itemIndex = 0;
  };
})();
