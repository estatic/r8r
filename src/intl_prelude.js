// Intl for r8r's QuickJS scripts, over the native helpers in globalThis.__r8r_intl
// (see src/intl.rs). Evaluated once per script context.
(() => {
  const native = globalThis.__r8r_intl;
  Object.defineProperty(globalThis, '__r8r_intl', { enumerable: false });

  const DEFAULT_LOCALE = 'en-US';

  // Locale arguments as JavaScript takes them: a tag, a list of tags, or nothing.
  function canonicalList(locales) {
    if (locales === undefined) return [];
    const list = typeof locales === 'string' ? [locales] : Array.from(locales);
    const out = [];
    for (const tag of list) {
      if (typeof tag !== 'string') throw new TypeError('Locale must be a string');
      const canonical = native.canonicalLocale(tag);
      if (!canonical) throw new RangeError(`Incorrect locale information provided: ${tag}`);
      if (!out.includes(canonical)) out.push(canonical);
    }
    return out;
  }
  function resolveLocale(locales) {
    return canonicalList(locales)[0] ?? DEFAULT_LOCALE;
  }
  function option(options, name, allowed, fallback) {
    const value = options?.[name];
    if (value === undefined) return fallback;
    const text = String(value);
    if (!allowed.includes(text)) throw new RangeError(`Value ${text} out of range for Intl options property ${name}`);
    return text;
  }
  function supportedLocalesOf(locales) {
    return canonicalList(locales);
  }

  class Segmenter {
    #locale;
    #granularity;
    constructor(locales, options = undefined) {
      this.#locale = resolveLocale(locales);
      this.#granularity = option(options, 'granularity', ['grapheme', 'word', 'sentence'], 'grapheme');
    }
    segment(input) {
      const text = String(input);
      const granularity = this.#granularity;
      const segments = JSON.parse(native.segment(text, granularity)).map(([start, end, wordLike]) => {
        const s = { segment: text.slice(start, end), index: start, input: text };
        if (granularity === 'word') s.isWordLike = wordLike;
        return s;
      });
      return {
        [Symbol.iterator]() {
          return segments[Symbol.iterator]();
        },
        containing(index = 0) {
          const i = Number(index) || 0;
          return segments.find((s) => i >= s.index && i < s.index + s.segment.length);
        },
      };
    }
    resolvedOptions() {
      return { locale: this.#locale, granularity: this.#granularity };
    }
    static supportedLocalesOf(locales) {
      return supportedLocalesOf(locales);
    }
  }

  // Digits after the decimal point per ISO 4217 (2 unless listed).
  const CURRENCY_DIGITS = { JPY: 0, KRW: 0, VND: 0, CLP: 0, ISK: 0, UGX: 0, PYG: 0, BHD: 3, KWD: 3, OMR: 3, JOD: 3, TND: 3, IQD: 3, LYD: 3 };

  function digits(options, name, fallback) {
    const value = options?.[name];
    if (value === undefined) return fallback;
    const n = Number(value);
    if (!Number.isInteger(n) || n < 0 || n > 100) throw new RangeError(`${name} value is out of range.`);
    return n;
  }
  function unsupported(options, names) {
    for (const name of names) {
      if (options?.[name] !== undefined) throw new RangeError(`Intl.NumberFormat option "${name}" is not supported in r8r yet`);
    }
  }

  class NumberFormat {
    #locale;
    #options;
    constructor(locales, options = undefined) {
      this.#locale = resolveLocale(locales);
      unsupported(options, ['minimumSignificantDigits', 'maximumSignificantDigits', 'roundingPriority', 'roundingIncrement']);
      if (options?.notation !== undefined && options.notation !== 'standard') {
        throw new RangeError('Intl.NumberFormat option "notation" is not supported in r8r yet');
      }
      const style = option(options, 'style', ['decimal', 'percent', 'currency', 'unit'], 'decimal');
      if (style === 'unit') throw new RangeError('Intl.NumberFormat style "unit" is not supported in r8r yet');
      let currency;
      if (style === 'currency') {
        if (options?.currency === undefined) throw new TypeError('Currency code is required with currency style.');
        currency = String(options.currency).toUpperCase();
        if (!/^[A-Z]{3}$/.test(currency)) throw new RangeError(`Invalid currency code : ${options.currency}`);
      }
      const currencyDisplay = option(options, 'currencyDisplay', ['symbol', 'narrowSymbol', 'code', 'name'], 'symbol');
      if (currencyDisplay === 'name') throw new RangeError('Intl.NumberFormat currencyDisplay "name" is not supported in r8r yet');
      const [defaultMin, defaultMax] =
        style === 'currency' ? [CURRENCY_DIGITS[currency] ?? 2, CURRENCY_DIGITS[currency] ?? 2] : style === 'percent' ? [0, 0] : [0, 3];
      const minimumFractionDigits = digits(options, 'minimumFractionDigits', defaultMin);
      let maximumFractionDigits = digits(options, 'maximumFractionDigits', Math.max(defaultMax, minimumFractionDigits));
      if (maximumFractionDigits < minimumFractionDigits) throw new RangeError('maximumFractionDigits value is out of range.');
      const grouping = options?.useGrouping;
      const useGrouping = grouping === undefined ? true : grouping !== false && grouping !== 'false';
      this.#options = { style, currency, currencyDisplay, minimumFractionDigits, maximumFractionDigits, useGrouping };
    }
    get format() {
      return (value) => {
        const n = Number(value);
        if (Number.isNaN(n)) return 'NaN';
        if (!Number.isFinite(n)) return (n < 0 ? '-' : '') + '∞' + (this.#options.style === 'percent' ? '%' : '');
        const o = this.#options;
        if (o.style === 'currency' && o.currencyDisplay === 'code') {
          const number = native.formatNumber(n, this.#locale, JSON.stringify({ ...o, style: 'decimal' }));
          return `${o.currency}\u00a0${number}`;
        }
        return native.formatNumber(n, this.#locale, JSON.stringify(o));
      };
    }
    resolvedOptions() {
      const { currency, currencyDisplay, ...rest } = this.#options;
      return {
        locale: this.#locale,
        numberingSystem: 'latn',
        ...rest,
        ...(rest.style === 'currency' ? { currency, currencyDisplay } : {}),
      };
    }
    static supportedLocalesOf(locales) {
      return supportedLocalesOf(locales);
    }
  }

  class PluralRules {
    #locale;
    #type;
    constructor(locales, options = undefined) {
      this.#locale = resolveLocale(locales);
      this.#type = option(options, 'type', ['cardinal', 'ordinal'], 'cardinal');
    }
    select(value) {
      const n = Number(value);
      if (!Number.isFinite(n)) return 'other';
      return native.pluralSelect(n, this.#locale, this.#type === 'ordinal');
    }
    resolvedOptions() {
      return { locale: this.#locale, type: this.#type };
    }
    static supportedLocalesOf(locales) {
      return supportedLocalesOf(locales);
    }
  }

  class Collator {
    #locale;
    #sensitivity;
    #numeric;
    #usage;
    constructor(locales, options = undefined) {
      this.#locale = resolveLocale(locales);
      this.#usage = option(options, 'usage', ['sort', 'search'], 'sort');
      this.#sensitivity = option(options, 'sensitivity', ['base', 'accent', 'case', 'variant'], 'variant');
      this.#numeric = options?.numeric === true || options?.numeric === 'true';
    }
    get compare() {
      return (a, b) => native.collate(String(a), String(b), this.#locale, this.#sensitivity, this.#numeric);
    }
    resolvedOptions() {
      return { locale: this.#locale, usage: this.#usage, sensitivity: this.#sensitivity, ignorePunctuation: false, collation: 'default', numeric: this.#numeric, caseFirst: 'false' };
    }
    static supportedLocalesOf(locales) {
      return supportedLocalesOf(locales);
    }
  }

  const DATE_FIELDS = ['weekday', 'era', 'year', 'month', 'day'];
  const TIME_FIELDS = ['hour', 'minute', 'second'];
  const FIELD_VALUES = {
    weekday: ['long', 'short', 'narrow'],
    era: ['long', 'short', 'narrow'],
    year: ['numeric', '2-digit'],
    month: ['numeric', '2-digit', 'long', 'short', 'narrow'],
    day: ['numeric', '2-digit'],
    hour: ['numeric', '2-digit'],
    minute: ['numeric', '2-digit'],
    second: ['numeric', '2-digit'],
  };
  const STYLES = ['full', 'long', 'medium', 'short'];

  class DateTimeFormat {
    #locale;
    #options;
    // `required`/`defaults` as the spec's ToDateTimeOptions: which fields
    // a bare call shows (Intl.DateTimeFormat and toLocaleDateString: the
    // date; toLocaleTimeString: the time; toLocaleString: both).
    constructor(locales, options = undefined, required = 'date', defaults = 'date') {
      this.#locale = resolveLocale(locales);
      for (const name of ['timeZoneName', 'fractionalSecondDigits', 'dayPeriod']) {
        if (options?.[name] !== undefined) throw new RangeError(`Intl.DateTimeFormat option "${name}" is not supported in r8r yet`);
      }
      const o = {};
      for (const field of [...DATE_FIELDS, ...TIME_FIELDS]) o[field] = option(options, field, FIELD_VALUES[field], undefined);
      o.dateStyle = option(options, 'dateStyle', STYLES, undefined);
      o.timeStyle = option(options, 'timeStyle', STYLES, undefined);
      const hasField = [...DATE_FIELDS, ...TIME_FIELDS].some((f) => o[f] !== undefined);
      if ((o.dateStyle || o.timeStyle) && hasField) {
        throw new TypeError(`Can't set option ${[...DATE_FIELDS, ...TIME_FIELDS].find((f) => o[f] !== undefined)} when ${o.dateStyle ? 'dateStyle' : 'timeStyle'} is used`);
      }
      if (!o.dateStyle && !o.timeStyle) {
        const needDate = required !== 'time' && !DATE_FIELDS.some((f) => o[f] !== undefined);
        const needTime = required !== 'date' && !TIME_FIELDS.some((f) => o[f] !== undefined);
        const none = (required === 'any' ? needDate && needTime : required === 'date' ? needDate : needTime) && !hasField;
        if (none || (required === 'any' && !hasField)) {
          if (defaults === 'date' || defaults === 'all') Object.assign(o, { year: 'numeric', month: 'numeric', day: 'numeric' });
          if (defaults === 'time' || defaults === 'all') Object.assign(o, { hour: 'numeric', minute: 'numeric', second: 'numeric' });
        }
      }
      let hourCycle = option(options, 'hourCycle', ['h11', 'h12', 'h23', 'h24'], undefined);
      if (options?.hour12 !== undefined) hourCycle = options.hour12 ? 'h12' : 'h23';
      o.hourCycle = hourCycle;
      if (options?.timeZone !== undefined) {
        const zone = String(options.timeZone);
        const canonical = zone.toUpperCase() === 'UTC' ? 'UTC' : zone;
        if (!native.isTimeZone(canonical)) throw new RangeError(`Invalid time zone specified: ${zone}`);
        o.timeZone = canonical;
      }
      this.#options = o;
    }
    get format() {
      return (date = Date.now()) => {
        const ms = date instanceof Date ? date.getTime() : Number(date);
        if (!Number.isFinite(ms)) throw new RangeError('Invalid time value');
        return native.formatDate(ms, this.#locale, JSON.stringify(this.#options));
      };
    }
    resolvedOptions() {
      const o = Object.fromEntries(Object.entries(this.#options).filter(([, v]) => v !== undefined));
      return { locale: this.#locale, calendar: 'gregory', numberingSystem: 'latn', timeZone: o.timeZone ?? native.localTimeZone(), ...o };
    }
    static supportedLocalesOf(locales) {
      return supportedLocalesOf(locales);
    }
  }

  const RELATIVE_UNITS = ['second', 'minute', 'hour', 'day', 'week', 'month', 'quarter', 'year'];

  class RelativeTimeFormat {
    #locale;
    #style;
    #numeric;
    constructor(locales, options = undefined) {
      this.#locale = resolveLocale(locales);
      this.#style = option(options, 'style', ['long', 'short', 'narrow'], 'long');
      this.#numeric = option(options, 'numeric', ['always', 'auto'], 'always');
    }
    format(value, unit) {
      const n = Number(value);
      if (!Number.isFinite(n)) throw new RangeError('Invalid value');
      const u = String(unit).replace(/s$/, '');
      if (!RELATIVE_UNITS.includes(u)) throw new RangeError(`Invalid unit argument for format() '${unit}'`);
      return native.formatRelative(n, u, this.#locale, this.#style, this.#numeric === 'auto');
    }
    resolvedOptions() {
      return { locale: this.#locale, style: this.#style, numeric: this.#numeric, numberingSystem: 'latn' };
    }
    static supportedLocalesOf(locales) {
      return supportedLocalesOf(locales);
    }
  }

  class ListFormat {
    #locale;
    #type;
    #style;
    constructor(locales, options = undefined) {
      this.#locale = resolveLocale(locales);
      this.#type = option(options, 'type', ['conjunction', 'disjunction', 'unit'], 'conjunction');
      this.#style = option(options, 'style', ['long', 'short', 'narrow'], 'long');
    }
    format(list) {
      const items = Array.from(list ?? [], (x) => {
        if (typeof x !== 'string') throw new TypeError('Iterable yielded a non-String value');
        return x;
      });
      return native.formatList(JSON.stringify(items), this.#locale, this.#type, this.#style);
    }
    resolvedOptions() {
      return { locale: this.#locale, type: this.#type, style: this.#style };
    }
    static supportedLocalesOf(locales) {
      return supportedLocalesOf(locales);
    }
  }

  globalThis.Intl = {
    getCanonicalLocales: (locales) => canonicalList(locales),
    Segmenter,
    NumberFormat,
    PluralRules,
    Collator,
    DateTimeFormat,
    RelativeTimeFormat,
    ListFormat,
  };

  for (const [name, required, defaults] of [
    ['toLocaleString', 'any', 'all'],
    ['toLocaleDateString', 'date', 'date'],
    ['toLocaleTimeString', 'time', 'time'],
  ]) {
    Object.defineProperty(Date.prototype, name, {
      value: function (locales, options) {
        if (Number.isNaN(this.getTime())) return 'Invalid Date';
        return new DateTimeFormat(locales, options, required, defaults).format(this);
      },
      writable: true,
      configurable: true,
    });
  }

  // The locale-aware built-ins, over the same classes.
  Object.defineProperty(Number.prototype, 'toLocaleString', {
    value: function toLocaleString(locales, options) {
      return new NumberFormat(locales, options).format(Number(this));
    },
    writable: true,
    configurable: true,
  });
  Object.defineProperty(String.prototype, 'localeCompare', {
    value: function localeCompare(that, locales, options) {
      return new Collator(locales, options).compare(String(this), String(that));
    },
    writable: true,
    configurable: true,
  });
})();
