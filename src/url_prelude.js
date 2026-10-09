// URL and URLSearchParams (WHATWG), over the __r8r_url natives.
(() => {
  const N = globalThis.__r8r_url;
  delete globalThis.__r8r_url;

  const formEncode = (s) =>
    encodeURIComponent(s)
      .replace(/%20/g, '+')
      .replace(/[!'()~]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase());
  const formDecode = (s) => {
    const t = s.replace(/\+/g, ' ');
    try {
      return decodeURIComponent(t);
    } catch {
      return t;
    }
  };
  const CHANGED = Symbol('changed');

  class URLSearchParams {
    constructor(init) {
      this._list = [];
      this[CHANGED] = null;
      if (init === undefined || init === null) return;
      if (init instanceof URLSearchParams) {
        this._list = init._list.map(([k, v]) => [k, v]);
      } else if (typeof init === 'object' && typeof init[Symbol.iterator] === 'function') {
        for (const pair of init) {
          const p = [...pair];
          if (p.length !== 2) throw new TypeError("Failed to construct 'URLSearchParams': each pair must have two values");
          this._list.push([String(p[0]), String(p[1])]);
        }
      } else if (typeof init === 'object') {
        for (const k of Object.keys(init)) this._list.push([k, String(init[k])]);
      } else {
        this._parse(String(init));
      }
    }
    _parse(s) {
      this._list = [];
      for (const part of s.replace(/^\?/, '').split('&')) {
        if (part === '') continue;
        const i = part.indexOf('=');
        this._list.push(i < 0 ? [formDecode(part), ''] : [formDecode(part.slice(0, i)), formDecode(part.slice(i + 1))]);
      }
    }
    _changed() {
      if (this[CHANGED]) this[CHANGED](this.toString());
    }
    get size() {
      return this._list.length;
    }
    append(k, v) {
      this._list.push([String(k), String(v)]);
      this._changed();
    }
    delete(k, v) {
      k = String(k);
      this._list = this._list.filter(([a, b]) => !(a === k && (v === undefined || b === String(v))));
      this._changed();
    }
    get(k) {
      const hit = this._list.find(([a]) => a === String(k));
      return hit ? hit[1] : null;
    }
    getAll(k) {
      return this._list.filter(([a]) => a === String(k)).map(([, b]) => b);
    }
    has(k, v) {
      return this._list.some(([a, b]) => a === String(k) && (v === undefined || b === String(v)));
    }
    set(k, v) {
      k = String(k);
      const i = this._list.findIndex(([a]) => a === k);
      if (i < 0) this._list.push([k, String(v)]);
      else {
        this._list[i] = [k, String(v)];
        this._list = this._list.filter(([a], j) => a !== k || j === i);
      }
      this._changed();
    }
    sort() {
      this._list.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
      this._changed();
    }
    forEach(fn, thisArg) {
      for (const [k, v] of this._list) fn.call(thisArg, v, k, this);
    }
    *entries() {
      for (const [k, v] of this._list) yield [k, v];
    }
    *keys() {
      for (const [k] of this._list) yield k;
    }
    *values() {
      for (const [, v] of this._list) yield v;
    }
    [Symbol.iterator]() {
      return this.entries();
    }
    toString() {
      return this._list.map(([k, v]) => formEncode(k) + '=' + formEncode(v)).join('&');
    }
    get [Symbol.toStringTag]() {
      return 'URLSearchParams';
    }
  }

  const parse = (input, base) => {
    const text = base === undefined ? N.parse(String(input)) : N.parse(String(input), String(base));
    if (!text) throw new TypeError(`Invalid URL: ${String(input).slice(0, 200)}`);
    return JSON.parse(text);
  };

  class URL {
    #p;
    #params;
    constructor(input, base) {
      this.#p = parse(input instanceof URL ? input.href : input, base instanceof URL ? base.href : base);
    }
    static canParse(input, base) {
      try {
        parse(input, base);
        return true;
      } catch {
        return false;
      }
    }
    static parse(input, base) {
      try {
        return new URL(input, base);
      } catch {
        return null;
      }
    }
    #set(part, value) {
      this.#p = JSON.parse(N.set(this.#p.href, part, String(value)));
      if (this.#params && part !== 'search-from-params') this.#params._parse(this.#p.search);
    }
    get href() { return this.#p.href; }
    set href(v) { this.#p = parse(v); if (this.#params) this.#params._parse(this.#p.search); }
    get origin() { return this.#p.origin; }
    get protocol() { return this.#p.protocol; }
    set protocol(v) { this.#set('protocol', v); }
    get username() { return this.#p.username; }
    set username(v) { this.#set('username', v); }
    get password() { return this.#p.password; }
    set password(v) { this.#set('password', v); }
    get host() { return this.#p.host; }
    set host(v) { this.#set('host', v); }
    get hostname() { return this.#p.hostname; }
    set hostname(v) { this.#set('hostname', v); }
    get port() { return this.#p.port; }
    set port(v) { this.#set('port', v); }
    get pathname() { return this.#p.pathname; }
    set pathname(v) { this.#set('pathname', v); }
    get search() { return this.#p.search; }
    set search(v) { this.#set('search', v); }
    get hash() { return this.#p.hash; }
    set hash(v) { this.#set('hash', v); }
    get searchParams() {
      if (!this.#params) {
        this.#params = new URLSearchParams(this.#p.search);
        this.#params[CHANGED] = (q) => {
          this.#p = JSON.parse(N.set(this.#p.href, 'search', q));
        };
      }
      return this.#params;
    }
    toString() { return this.#p.href; }
    toJSON() { return this.#p.href; }
    get [Symbol.toStringTag]() { return 'URL'; }
  }

  globalThis.URL = URL;
  globalThis.URLSearchParams = URLSearchParams;
})();
