// The browser host for a compiled LOGOS page: the `env` imports the module calls, the table that
// names DOM nodes to the module, the patch stream a paint commits through, the event trampoline, the
// fetch engine, the relay connection and the router.
//
// The caller prepends one line, `const WASM_URL = "<module>";`; everything else is this file.
// `globalThis.__logosHostCensus()` reports the size of every table the host keeps, for tests and
// devtools.
let inst;
const dv = () => new DataView(inst.exports.memory.buffer);
const u8 = () => new Uint8Array(inst.exports.memory.buffer);
const dec = new TextDecoder(), enc = new TextEncoder();

// ── Entries into the module ─────────────────────────────────────────────────────────────────────
//
// A compiled LOGOS program reports a runtime failure by trapping. `logos_rt_fail`/`logos_rt_fail_at`
// run immediately before the trap and leave the compiler's own sentence in `failMessage`. Every
// entry into the module — `main`, a listener, a fetch completion, a relay message, a timer, the
// router — goes through `guarded`, which counts how deep the entries nest. The first failure of an
// outermost entry is captured with the message of that moment and reported once, when the
// outermost entry unwinds; a nested entry the browser fires synchronously (a `blur` raised by a
// commit) is covered by the entry that caused it. `failMessage` is cleared when an outermost entry
// begins, so a report only ever carries its own entry's words. A failed entry leaves the document as
// the last commit left it: a paint reaches the host only as a whole commit's patch stream.
let failMessage = '';
let depth = 0;
let failure = null;
// Work that waits until no entry is on the stack — releasing a finished fetch, and an event the
// platform raised in the middle of one (the `focus` and `blur` of an `element.focus()` a turn
// asked for, the `blur` of a focused node a commit removed): it reaches the page once the entry
// that caused it has committed, and finds its listener as the page stands then.
const afterTurn = [];
const atTurnEnd = (f) => { if (depth === 0) f(); else afterTurn.push(f); };

// The page clock. The page keeps its own time and decides every timer in compiled LOGOS
// (`__lgx_tick(ms)` fires what the milliseconds that passed make due, in due order); the host says
// how much time passed, before every outermost entry — so a timer armed during a click counts from
// the moment of the click — and when the page's earliest timer comes due (`dom_wake_after`).
// `clockAt` is null until `main` has run: the page's clock starts with the page.
let clockAt = null;
let wakeTimer = null;
// Where the pending wake comes due on `clockAt`'s scale, and, while a wake runs, the time it is owed:
// the browser runs a timer no earlier than its delay on its own clock, so that much time has passed
// when the wake runs, however short a coarsened `performance.now()` reads then. A wake `main` asks for
// comes due its delay after the clock starts.
let wakeDue = null;
let wakeBeforeClock = null;
let owed = null;
const passTime = () => {
  if (clockAt === null || !inst.exports.__lgx_tick) return;
  const atLeast = owed === null ? 0 : owed - clockAt;
  owed = null;
  const elapsed = Math.max(Math.floor(performance.now() - clockAt), atLeast);
  if (elapsed <= 0) return;
  clockAt += elapsed;
  inst.exports.__lgx_tick(BigInt(elapsed));
  settle();
};

const guarded = (fn) => {
  if (depth === 0) { failMessage = ''; failure = null; }
  depth++;
  try { if (depth === 1) passTime(); return fn(); } catch (err) {
    if (failure === null) failure = { err, why: failMessage };
    throw err;
  } finally {
    depth--;
    if (depth === 0) {
      const f = failure;
      failure = null;
      if (f) reportRuntimeFailure(f.err, f.why);
      for (const task of afterTurn.splice(0)) task();
      if (inst && inst.exports.__logos_safepoint) inst.exports.__logos_safepoint();
    }
  }
};

// Call a closure handle the module handed the host, as one entry: `currentEvent` is the event the
// module reads through `eventValue`/`eventKey`/`eventTarget` while it runs. A handle the host no
// longer holds (`undefined`) is not called.
let currentEvent = null;
const dispatch = (handle, ev, arg = 0n) => {
  if (handle === undefined || handle === null) return;
  guarded(() => {
    const prev = currentEvent;
    currentEvent = ev;
    try { inst.exports.__logos_call_closure(handle, arg); settle(); } finally { currentEvent = prev; }
  });
};

// `turn` run once no entry is on the stack, for an event that may carry a dropped file. A drop's
// first file is taken off its transfer while the event is being dispatched (a trusted transfer is
// emptied once dispatch ends) and read in full BEFORE the turn runs, so `dom_event_file_*` answer
// from the file itself, synchronously, inside the turn. A file that cannot be read is reported,
// and no turn runs for it.
const atTurnEndWithFile = (ev, turn) => {
  const files = ev && ev.dataTransfer && ev.dataTransfer.files;
  const file = files && files.length ? files[0] : null;
  if (!file) { atTurnEnd(turn); return; }
  ev.__logosFile = file;
  file.arrayBuffer().then(
    (content) => { ev.__logosFileBytes = new Uint8Array(content); atTurnEnd(turn); },
    (err) => reportRuntimeFailure(err, `the dropped file ${JSON.stringify(file.name)} could not be read: ${err && err.message ? err.message : err}`),
  );
};

// A DOM event reaching one of the page's closure listeners: its turn runs against the closure
// `handle()` names then.
const hear = (ev, handle) => atTurnEndWithFile(ev, () => dispatch(handle(), ev));

// A closure handle the host stops holding — a replaced or removed listener, a finished fetch, a
// superseded route handler. A module that counts host references exports `__logos_release`.
const releaseHandle = (h) => {
  if (h !== undefined && h !== null && inst && inst.exports.__logos_release) inst.exports.__logos_release(h);
};

// Say why the program stopped: a console line (the channel that survives a document too broken to
// paint into), then the page's own `When an action fails` handler, then the failure panel. The
// message is the module's own sentence; a failure the HOST raised (a malformed patch stream, a
// DOM exception) carries its own message; a bare trap with neither says so.
const reportRuntimeFailure = (err, captured) => {
  const hostMessage = err && !(err instanceof WebAssembly.RuntimeError) && err.message ? String(err.message) : '';
  // What the page was doing when it trapped (a directive's block) names the failure, in the same
  // sentence every tier puts in front of it.
  let context = '';
  try {
    if (inst && inst.exports.__lgx_failure_context) context = readText(Number(inst.exports.__lgx_failure_context()));
  } catch (_) { context = ''; }
  const why = context + (captured || hostMessage || 'the program stopped without saying why');
  console.error('LOGOS runtime failure: ' + why, err);
  // A trap leaves the instance callable and the turn's state unwritten (a turn writes back only at
  // its end), so the handler runs against the record the failed turn started from. The module
  // knows which component was mid-turn; the host only carries the message. A handler that fails
  // must not replace the failure being reported.
  try {
    if (inst && inst.exports.__lgx_on_failure) {
      inst.exports.__lgx_on_failure(allocText(why));
      settle();
    }
  } catch (_) {
    console.error('LOGOS: the failure handler itself failed');
  }
  showFailure(why);
};

// One panel per page, reused by every failure: the latest message and a button that dismisses it.
// `data-logos-host` marks it as the host's, so a hydration never claims it as page content.
let failurePanel = null;
const showFailure = (why) => {
  try {
    let panel = failurePanel;
    if (!panel || !panel.isConnected) {
      panel = document.createElement('div');
      panel.setAttribute('role', 'alert');
      panel.setAttribute('data-logos-host', 'failure');
      panel.style.cssText = 'position:fixed;inset:auto 0 0 0;z-index:2147483647;margin:0;padding:1rem 3rem 1rem 1.25rem;'
        + 'font:14px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace;white-space:pre-wrap;'
        + 'background:#2b1416;color:#ffd7d7;border-top:2px solid #d9534f;max-height:50vh;overflow:auto';
      const close = document.createElement('button');
      close.type = 'button';
      close.setAttribute('aria-label', 'Dismiss');
      close.textContent = '×';
      close.style.cssText = 'position:absolute;top:.5rem;right:.75rem;background:none;border:none;color:#ffd7d7;'
        + 'font:20px/1 sans-serif;cursor:pointer;padding:.25rem .5rem';
      close.addEventListener('click', () => panel.remove());
      const title = document.createElement('strong');
      title.style.cssText = 'display:block;margin-bottom:.35rem;color:#ff9a9a';
      title.textContent = 'LOGOS runtime failure';
      const text = document.createElement('div');
      text.setAttribute('data-logos-why', '');
      panel.append(close, title, text);
      (document.body || document.documentElement).appendChild(panel);
      failurePanel = panel;
    }
    panel.querySelector('[data-logos-why]').textContent = why;
  } catch (_) {
    // The document may be unusable; the console line already carried the message.
  }
};

// ── Text across the host boundary ───────────────────────────────────────────────────────────────
const readText = (h) => { const d = dv(); const len = d.getInt32(h, true), dp = d.getInt32(h + 8, true); return dec.decode(u8().subarray(dp, dp + len)); };
// Read a `Seq of Text` handle: `[len][cap][data_ptr]`, each 8-byte slot holding a Text handle.
const readTextSeq = (h) => { const d = dv(); const n = d.getInt32(h, true), dp = d.getInt32(h + 8, true); const out = []; for (let i = 0; i < n; i++) out.push(readText(d.getInt32(dp + i * 8, true))); return out; };
// Host text is a heap block like any other: `__logos_text_new(len)` allocates it through the
// module's own allocator (a free-list class if one fits, else a bump that grows the module's memory
// on demand — see `emit_alloc`), so a string the host hands the module is visible to
// `__logos_heap_allocated` and reachable by the release protocol exactly as a module-built Text is.
// `allocText` is the only writer of host text, so this is the one place that matters.
const allocText = (s) => {
  const by = enc.encode(s);
  const t = inst.exports.__logos_text_new(by.length);
  u8().set(by, dv().getInt32(t + 8, true));
  return t;
};
// A list the host hands the module is built in the module's heap like one the module builds:
// `__logos_seq_new(len, 0)` allocates a header over `len` zeroed 8-byte slots through the module's
// own allocator, and each slot is written as the module writes an Int. `allocSeq` is the only
// writer of a host-built list.
const allocSeq = (values) => {
  const h = inst.exports.__logos_seq_new(values.length, 0);
  const d = dv(), p = d.getInt32(h + 8, true);
  for (let i = 0; i < values.length; i++) d.setBigInt64(p + 8 * i, BigInt(values[i]), true);
  return h;
};
// The areas a page can store text in, and the only place this host decides what an area IS.
// `local` survives closing the tab (`persisted as "key"`); `session` survives a reload and ends
// with the tab (`preserved across reloads`); `shared` is the topics every page on them shares.
const storageArea = (name) => {
  if (name === 'local') return window.localStorage;
  if (name === 'session') return window.sessionStorage;
  if (name === 'shared') return sharedArea;
  throw new Error(`I do not know a storage area called "${name}". A page can store text in "local", which survives closing the tab, "session", which survives a reload and ends with the tab, or "shared", which every page on a topic shares. Which did you mean?`);
};
// The "shared" area: `<topic>.<field>` keys every page on the topic shares. A write is kept here and
// published on the topic (`<key> NUL <frame>`); the first touch of a topic joins it, and a frame
// another page published there is handed to the module's `__lgx_shared(topic, key, frame)`, which
// merges it and writes the result back through `storageSet`. This host moves frames and never
// merges them.
const sharedCache = new Map();
const sharedTopics = new Set();
const topicOfKey = (key) => key.split('.')[0];
const joinShared = (topic) => {
  if (sharedTopics.has(topic)) return;
  sharedTopics.add(topic);
  relay.subscribe(topic, {
    deliver(data) {
      const at = data.indexOf('\u0000');
      if (at < 0 || !inst || !inst.exports.__lgx_shared) return;
      const key = data.slice(0, at), frame = data.slice(at + 1);
      guarded(() => { inst.exports.__lgx_shared(allocText(topic), allocText(key), allocText(frame)); settle(); });
    },
  });
};
const sharedArea = {
  getItem(key) { joinShared(topicOfKey(key)); return sharedCache.has(key) ? sharedCache.get(key) : null; },
  setItem(key, value) {
    const topic = topicOfKey(key);
    joinShared(topic);
    sharedCache.set(key, value);
    relay.send({ Publish: { topic, data: enc.encode(key + '\u0000' + value) } });
  },
  removeItem(key) { sharedCache.delete(key); },
};
// Write a formatted value into the interpolation buffer at `buf`, returning its
// byte length — the shape every `fmt_*_into` host uses to build an interpolated hole.
const writeStr = (buf, s) => { const b = enc.encode(s); u8().set(b, buf); return b.length; };
const fmtF = (v) => {
  if (Number.isNaN(v)) return 'NaN';
  if (v === Infinity) return 'inf';
  if (v === -Infinity) return '-inf';
  if (v === 0) return Object.is(v, -0) ? '-0' : '0';
  const sign = v < 0 ? '-' : '';
  const a = Math.abs(v);
  const s = a.toString();
  const e = s.indexOf('e');
  const mantissa = e < 0 ? s : s.slice(0, e);
  const dot = mantissa.indexOf('.');
  let digits = dot < 0 ? mantissa : mantissa.slice(0, dot) + mantissa.slice(dot + 1);
  let point = (dot < 0 ? mantissa.length : dot) + (e < 0 ? 0 : Number(s.slice(e + 1)));
  const lead = digits.length - digits.replace(/^0+/, '').length;
  digits = digits.slice(lead).replace(/0+$/, '');
  point -= lead;
  const spell = (d, p) => p <= 0 ? '0.' + '0'.repeat(-p) + d : p >= d.length ? d + '0'.repeat(p - d.length) : d.slice(0, p) + '.' + d.slice(p);
  if (Number(spell(digits + '5', point)) === a) {
    const view = new DataView(new ArrayBuffer(8));
    view.setFloat64(0, a);
    const bits = view.getBigUint64(0);
    const biased = Number(bits >> 52n);
    const m = biased === 0 ? bits & 0xfffffffffffffn : (bits & 0xfffffffffffffn) | (1n << 52n);
    const b = biased === 0 ? -1074 : biased - 1075;
    const q = point - digits.length - 1;
    const half = BigInt(digits + '5') * 10n ** BigInt(Math.max(q, 0)) * 2n ** BigInt(Math.max(-b, 0));
    const exact = m * 2n ** BigInt(Math.max(b, 0)) * 10n ** BigInt(Math.max(-q, 0));
    if (half === exact) {
      const up = (BigInt(digits) + 1n).toString();
      point += up.length - digits.length;
      digits = up.replace(/0+$/, '');
    }
  }
  return sign + spell(digits, point);
};
// A uniform draw from lo..=hi (`randomInt`): 64-bit words from `crypto.getRandomValues`, a word at
// or above the largest multiple of the span drawn again, so no value is likelier than another.
const randomWord = () => { const w = new BigUint64Array(1); crypto.getRandomValues(w); return w[0]; };
const randomIntIn = (lo, hi) => {
  if (lo > hi) throw new Error(`randomInt(${lo}, ${hi}): the low bound is above the high bound`);
  const span = hi - lo + 1n;
  if (span === 1n << 64n) return BigInt.asIntN(64, randomWord());
  const limit = (1n << 64n) - (1n << 64n) % span;
  for (;;) { const r = randomWord(); if (r < limit) return lo + r % span; }
};

// ── Calendar arithmetic ─────────────────────────────────────────────────────────────────────────
//
// `temporal_component`/`temporal_component_date` answer a `Date`/`Moment`'s calendar/clock
// components: exactly Howard Hinnant's `days_from_civil`/`civil_from_days` (BigInt throughout, so
// a far-future/far-past day count never loses precision) — the identical algorithm
// `logicaffeine_base::temporal` runs, so a page's `the weekday of` a date is bit-identical to the
// reference tier. `z`/`days` are always a day count since the Unix epoch (1970-01-01 = day 0).
const daysFromCivil = (year, month, day) => {
  const y = month <= 2n ? year - 1n : year;
  const era = (y >= 0n ? y : y - 399n) / 400n;
  const yoe = y - era * 400n;
  const m = month;
  const doy = (153n * (m > 2n ? m - 3n : m + 9n) + 2n) / 5n + day - 1n;
  const doe = yoe * 365n + yoe / 4n - yoe / 100n + doy;
  return era * 146097n + doe - 719468n;
};
const civilFromDays = (z) => {
  const zz = z + 719468n;
  const era = (zz >= 0n ? zz : zz - 146096n) / 146097n;
  const doe = zz - era * 146097n;
  const yoe = (doe - doe / 1460n + doe / 36524n - doe / 146096n) / 365n;
  const y = yoe + era * 400n;
  const doy = doe - (365n * yoe + yoe / 4n - yoe / 100n);
  const mp = (5n * doy + 2n) / 153n;
  const day = doy - (153n * mp + 2n) / 5n + 1n;
  const month = mp < 10n ? mp + 3n : mp - 9n;
  const year = month <= 2n ? y + 1n : y;
  return [year, month, day];
};
// ISO-8601 weekday of a day number: 1 = Monday … 7 = Sunday.
const isoWeekdayFromDays = (z) => ((((z + 3n) % 7n) + 7n) % 7n) + 1n;
const isoWeekFromDays = (z) => {
  const isoDow = isoWeekdayFromDays(z);
  const thursday = z + (4n - isoDow);
  const [isoYear] = civilFromDays(thursday);
  const jan4 = daysFromCivil(isoYear, 1n, 4n);
  const week1Monday = jan4 - (isoWeekdayFromDays(jan4) - 1n);
  return (z - week1Monday) / 7n + 1n;
};
const NANOS_PER_DAY = 86400000000000n;
const NANOS_PER_SECOND = 1000000000n;
const divEuclid = (a, b) => { const q = a / b; return a % b < 0n ? q - 1n : q; };
const remEuclid = (a, b) => { const r = a % b; return r < 0n ? r + b : r; };
const isGregorianLeap = (year) => (year % 4n === 0n && year % 100n !== 0n) || year % 400n === 0n;
const lastDayOfMonth = (year, month) => {
  if ([1n, 3n, 5n, 7n, 8n, 10n, 12n].includes(month)) return 31n;
  if ([4n, 6n, 9n, 11n].includes(month)) return 30n;
  if (month === 2n) return isGregorianLeap(year) ? 29n : 28n;
  return 0n;
};
// `logicaffeine_base::temporal::parse_rfc3339`, ported byte-for-byte: a fixed 19-byte
// `YYYY-MM-DDTHH:MM:SS` prefix, optional fractional seconds, then `Z` or a `±HH:MM` offset.
const parseRfc3339 = (s) => {
  const bad = () => new Error(`invalid RFC 3339 timestamp: ${s}`);
  if (s.length < 20) throw bad();
  const digits = (start, len) => {
    let v = 0n;
    for (let i = start; i < start + len; i++) {
      const c = s.charCodeAt(i);
      if (c < 48 || c > 57) throw bad();
      v = v * 10n + BigInt(c - 48);
    }
    return v;
  };
  if (s[4] !== '-' || s[7] !== '-' || s[10] !== 'T' || s[13] !== ':' || s[16] !== ':') throw bad();
  const year = digits(0, 4), month = digits(5, 2), day = digits(8, 2);
  const hour = digits(11, 2), minute = digits(14, 2), second = digits(17, 2);
  if (month < 1n || month > 12n || day < 1n || day > lastDayOfMonth(year, month)) throw bad();
  if (hour > 23n || minute > 59n || second > 59n) throw bad();
  let pos = 19;
  let nanosecond = 0n;
  if (s[pos] === '.') {
    pos += 1;
    let frac = 0n, count = 0;
    while (pos < s.length && s[pos] >= '0' && s[pos] <= '9') {
      if (count < 9) { frac = frac * 10n + BigInt(s.charCodeAt(pos) - 48); count += 1; }
      pos += 1;
    }
    if (count === 0) throw bad();
    for (let i = count; i < 9; i++) frac *= 10n;
    nanosecond = frac;
  }
  let offsetSeconds = 0n;
  if (s[pos] === 'Z') {
    pos += 1;
  } else if (s[pos] === '+' || s[pos] === '-') {
    const sign = s[pos] === '+' ? 1n : -1n;
    const oh = digits(pos + 1, 2);
    if (s[pos + 3] !== ':') throw bad();
    const om = digits(pos + 4, 2);
    if (oh > 23n || om > 59n) throw bad();
    pos += 6;
    offsetSeconds = sign * (oh * 3600n + om * 60n);
  } else {
    throw bad();
  }
  if (pos !== s.length) throw bad();
  const days = daysFromCivil(year, month, day);
  const nanos = days * NANOS_PER_DAY + (hour * 3600n + minute * 60n + second) * NANOS_PER_SECOND + nanosecond;
  return nanos - offsetSeconds * NANOS_PER_SECOND;
};
const temporalComponent = (nanos, which) => {
  const day = divEuclid(nanos, NANOS_PER_DAY);
  const rem = remEuclid(nanos, NANOS_PER_DAY);
  const [year, month, d] = civilFromDays(day);
  const secs = rem / NANOS_PER_SECOND;
  switch (Number(which)) {
    case 0: return year;
    case 1: return month;
    case 2: return d;
    case 3: return secs / 3600n;
    case 4: return (secs % 3600n) / 60n;
    case 5: return secs % 60n;
    case 6: return isoWeekdayFromDays(day);
    case 7: return isoWeekFromDays(day);
    case 8: return (month - 1n) / 3n + 1n;
    default: throw new Error(`temporal_component: unknown component selector ${which}`);
  }
};
const temporalComponentDate = (days, which) => {
  const z = BigInt(days);
  const [year, month, d] = civilFromDays(z);
  switch (Number(which)) {
    case 0: return year;
    case 1: return month;
    case 2: return d;
    case 6: return isoWeekdayFromDays(z);
    case 7: return isoWeekFromDays(z);
    case 8: return (month - 1n) / 3n + 1n;
    default: throw new Error(`temporal_component_date: unknown component selector ${which}`);
  }
};

// A width-`w` field, zero-padded, with the sign OUTSIDE the padding (Rust's `{:0w$}` on a signed
// value: `format!("{:04}", -5)` is `"-005"`, not `"00-5"`).
const padSigned = (n, w) => {
  const neg = n < 0;
  const digits = String(neg ? -n : n).padStart(neg ? w - 1 : w, '0');
  return neg ? '-' + digits : digits;
};
const pad2 = (n) => String(n).padStart(2, '0');
const printDate = (days) => {
  const [y, m, d] = civilFromDays(BigInt(days));
  return `${padSigned(Number(y), 4)}-${pad2(m)}-${pad2(d)}`;
};
const printMoment = (nanos) => {
  const totalSeconds = divEuclid(nanos, NANOS_PER_SECOND);
  const days = divEuclid(totalSeconds, 86400n);
  const daySeconds = remEuclid(totalSeconds, 86400n);
  const hours = daySeconds / 3600n, minutes = (daySeconds % 3600n) / 60n;
  const [y, m, d] = civilFromDays(days);
  return `${padSigned(Number(y), 4)}-${pad2(m)}-${pad2(d)} ${pad2(hours)}:${pad2(minutes)}`;
};
// `logicaffeine_base::temporal::format_time_of_day`: `HH:MM:SS[.frac]`, lossless to the nanosecond.
const printTime = (nanosFromMidnight) => {
  const rem = remEuclid(nanosFromMidnight, NANOS_PER_DAY);
  const secs = rem / NANOS_PER_SECOND;
  let s = `${pad2(secs / 3600n)}:${pad2((secs % 3600n) / 60n)}:${pad2(secs % 60n)}`;
  const nanos = rem % NANOS_PER_SECOND;
  if (nanos !== 0n) {
    let frac = nanos.toString().padStart(9, '0');
    while (frac.endsWith('0')) frac = frac.slice(0, -1);
    s += '.' + frac;
  }
  return s;
};
// `RuntimeValue::Duration::to_display_string`: magnitude-bucketed (h/min/s/ms/μs/ns).
const printDuration = (nanos) => {
  const abs = nanos < 0n ? -nanos : nanos;
  const sign = nanos < 0n ? '-' : '';
  if (abs >= 3600000000000n) return `${sign}${abs / 3600000000000n}h`;
  if (abs >= 60000000000n) return `${sign}${abs / 60000000000n}min`;
  if (abs >= 1000000000n) return `${sign}${abs / 1000000000n}s`;
  if (abs >= 1000000n) return `${sign}${abs / 1000000n}ms`;
  if (abs >= 1000n) return `${sign}${abs / 1000n}μs`;
  return `${sign}${abs}ns`;
};
// `RuntimeValue::Span::to_display_string`: a `Span`'s two i32 fields packed months-high/days-low
// (`vm/wasm/module.rs`'s `Constant::Span` lowering).
const printSpan = (packed) => {
  const months = Number(BigInt.asIntN(32, packed >> 32n));
  const days = Number(BigInt.asIntN(32, packed));
  const parts = [];
  const years = Math.trunc(months / 12), remMonths = months % 12;
  if (years !== 0) parts.push(Math.abs(years) === 1 ? `${years} year` : `${years} years`);
  if (remMonths !== 0) parts.push(Math.abs(remMonths) === 1 ? `${remMonths} month` : `${remMonths} months`);
  if (days !== 0 || parts.length === 0) parts.push(Math.abs(days) === 1 ? `${days} day` : `${days} days`);
  return parts.join(' and ');
};
// Every `Seq of <scalar>` / `Set of <scalar>` display sink: read the `[len][cap][data_ptr]` header
// and each 8-byte slot, matching `RuntimeValue::List`/`Set::to_display_string` exactly (insertion
// order, `[e0, e1]` / `{e0, e1}`).
const seqSlotAddrs = (h) => {
  const d = dv();
  const len = d.getInt32(h, true), dp = d.getInt32(h + 8, true);
  const out = [];
  for (let i = 0; i < len; i++) out.push(dp + i * 8);
  return out;
};
const displaySeqI64 = (h) => '[' + seqSlotAddrs(h).map((a) => dv().getBigInt64(a, true).toString()).join(', ') + ']';
const displaySeqBool = (h) => '[' + seqSlotAddrs(h).map((a) => (dv().getBigInt64(a, true) !== 0n).toString()).join(', ') + ']';
const displaySeqWord32 = (h) => '[' + seqSlotAddrs(h).map((a) => dv().getUint32(a, true).toString()).join(', ') + ']';
const displaySeqWord64 = (h) => '[' + seqSlotAddrs(h).map((a) => dv().getBigUint64(a, true).toString()).join(', ') + ']';
const displaySeqF64 = (h) => '[' + seqSlotAddrs(h).map((a) => fmtF(dv().getFloat64(a, true))).join(', ') + ']';
const displaySeqText = (h) => '[' + seqSlotAddrs(h).map((a) => readText(dv().getInt32(a, true))).join(', ') + ']';
const displaySetI64 = (h) => '{' + seqSlotAddrs(h).map((a) => dv().getBigInt64(a, true).toString()).join(', ') + '}';
const displaySetText = (h) => '{' + seqSlotAddrs(h).map((a) => readText(dv().getInt32(a, true))).join(', ') + '}';
// `apply_format_spec`'s alignment: right-pad (`<`, 1), left-pad (`>`, 0, also the bare-width
// default), or split the padding (`^`, 2, the extra column going to the right) — space fill,
// counted in Unicode code points (`Array.from`), matching Rust's `char`-counted width.
const alignPad = (s, width, align) => {
  const chars = Array.from(s);
  const pad = width - chars.length;
  if (pad <= 0) return s;
  if (align === 1) return s + ' '.repeat(pad);
  if (align === 2) { const l = Math.floor(pad / 2); return ' '.repeat(l) + s + ' '.repeat(pad - l); }
  return ' '.repeat(pad) + s;
};

// ── The node table ──────────────────────────────────────────────────────────────────────────────
//
// The module names DOM nodes by integer id (an i64 at the boundary); id 0 is `document.body`. Ids
// are handed out from a counter and never reused, so a stale id can never name a different node.
// `nodes` maps an id to its node; `ids` maps a node to its current id, so re-querying a node the
// host already named returns the same id. A node a patch stream frees is deleted from both, with every
// node under it; an operation on a deleted id does nothing.
const nodes = new Map([[0, document.body]]);
const ids = new WeakMap([[document.body, 0]]);
let nextId = 1;
const nodeId = (n) => { let id = ids.get(n); if (id === undefined) { id = nextId++; nodes.set(id, n); ids.set(n, id); } return BigInt(id); };
const node = (h) => nodes.get(Number(h));

// `preventDefault` for one event on one element, held by however many owners asked for it — a
// stream's registration, an imperative one — through ONE listener, added when the first owner holds
// it and removed when the last lets go.
const preventIt = (e) => e.preventDefault();
const holdPrevent = (el, event) => {
  const held = el.__logosPrevented || (el.__logosPrevented = new Map());
  const n = held.get(event) || 0;
  held.set(event, n + 1);
  if (n === 0) el.addEventListener(event, preventIt);
};
const dropPrevent = (el, event) => {
  const held = el.__logosPrevented;
  const n = (held && held.get(event)) || 0;
  if (n > 1) { held.set(event, n - 1); return; }
  if (held) held.delete(event);
  el.removeEventListener(event, preventIt);
};
// An imperative registration is permanent and idempotent: asking twice holds it once.
const preventAlways = (el, event) => {
  const always = el.__logosPreventedAlways || (el.__logosPreventedAlways = new Set());
  if (always.has(event)) return;
  always.add(event);
  holdPrevent(el, event);
};

// A listener's flags (`listenFlags(el, event, flags)`, the bits of `semantics::dom::LISTEN_*`):
// capture and passive are how its DOM listener is attached, stop is what that listener does before
// the page hears the event, and prevent is held on the element — unless the listener is passive,
// which promises the browser it will not cancel. Once (4) is the lowering's own bookkeeping; a host
// never retires a listener.
const LISTEN_PREVENT = 1, LISTEN_STOP = 2, LISTEN_CAPTURE = 8, LISTEN_PASSIVE = 16;
const listenOptions = (flags) => ({ capture: (flags & LISTEN_CAPTURE) !== 0, passive: (flags & LISTEN_PASSIVE) !== 0 });
const attachedAlike = (a, b) => ((a ^ b) & (LISTEN_CAPTURE | LISTEN_PASSIVE)) === 0;
const cancels = (flags) => (flags & LISTEN_PREVENT) !== 0 && (flags & LISTEN_PASSIVE) === 0;
// The DOM listener behind a registration: it stops the event when the registration's CURRENT flags
// say so, then hands it to the page through `hear`.
const heard = (flagsNow, handle) => (ev) => {
  if (flagsNow() & LISTEN_STOP) ev.stopPropagation();
  hear(ev, handle);
};

// Where a browser delivers `event` for `el`: the viewport's own scroll is fired at the document,
// never at `body` (node 0, the page) or the root element, so the page's scroll is heard there.
const deliveredAt = (el, event) => (event === 'scroll' && (el === document.body || el === document.documentElement) ? document : el);
// A closure imperative code attaches (`addEventListener`) is a DOM listener of its own, for the life
// of the element.
const ownListen = (el, event, handler) => {
  const binding = { target: deliveredAt(el, event), event, handler };
  binding.listener = heard(() => 0, () => binding.handler);
  try {
    binding.target.addEventListener(event, binding.listener);
  } catch (err) {
    binding.handler = undefined;
    releaseHandle(handler);
    throw err;
  }
  (el.__logosOwnListeners || (el.__logosOwnListeners = new Set())).add(binding);
};
const dropOwnListeners = (el) => {
  const bindings = el.__logosOwnListeners;
  if (!bindings) return;
  for (const binding of bindings) {
    binding.target.removeEventListener(binding.event, binding.listener);
    const handler = binding.handler;
    binding.handler = undefined;
    releaseHandle(handler);
  }
  bindings.clear();
};

// ── The page's listeners are DATA ───────────────────────────────────────────────────────────────
//
// The page keeps which element answers which event in its own tables; a registration
// (`listenFlags`, a stream's `Listen`) tells the host only that something listens there, and how.
// An element holds one DOM listener per event, under the node id the registration named, and it
// hands the event to the page's one entry, `__lgx_dispatch_node(node, event, value)`. `value` is
// what the event carries: the name of a dropped file, the key of a keyboard event, the state of a
// checkbox or radio, the value of any other control. An element no paint holds any more answers
// nothing. A `drop` that carries a file is also heard as `filedropped`, the event a page's `When a
// file is dropped on …:` answers, once the file is read.
const FILE_DROPPED = 'filedropped';
const pageListeners = new Set();
const eventValueOf = (ev) => {
  if (ev && ev.__logosFile) return ev.__logosFile.name;
  if (ev && typeof ev.key === 'string') return ev.key;
  const t = ev && ev.target;
  if (!t) return '';
  const ty = (t.type || '').toLowerCase();
  if (ty === 'checkbox' || ty === 'radio') return t.checked ? 'true' : 'false';
  return t.value == null ? '' : String(t.value);
};
// The page's one entry for an element's event: its dispatcher hears `event` at node `id`, carrying
// `value` (both Text handles), and answers whether it heard it. A visitor's event reaches it through
// `hearAtNode`, a program's own driving verb through `dom_page_entry`.
const dispatchNode = (id, event, value) => inst.exports.__lgx_dispatch_node(id, event, value);
const hearAtNode = (el, id, event, ev) => {
  if (!ids.has(el) || !inst || !inst.exports.__lgx_dispatch_node) return;
  guarded(() => {
    const prev = currentEvent;
    currentEvent = ev;
    try {
      dispatchNode(BigInt(id), allocText(event), allocText(eventValueOf(ev)));
      settle();
    } finally { currentEvent = prev; }
  });
};
const pageEntry = (el, event) => (el.__logosListen ? el.__logosListen[event] : undefined);
// The listener for `event` on `el`, registered under node `id`, attached again only when the flags
// change HOW it is attached (capture, passive); `.stop` is what it does before the page hears the
// event.
const pageListen = (el, event, flags, id) => {
  const on = el.__logosListen || (el.__logosListen = {});
  const had = on[event];
  if (had) had.id = id;
  if (had && attachedAlike(had.flags, flags)) { had.flags = flags; return; }
  if (had) deliveredAt(el, event).removeEventListener(event, had.fn, listenOptions(had.flags));
  const entry = had || { flags, id, fn: null };
  entry.flags = flags;
  entry.fn = (ev) => {
    if (entry.flags & LISTEN_STOP) ev.stopPropagation();
    if (event !== 'drop') { atTurnEnd(() => hearAtNode(el, entry.id, event, ev)); return; }
    atTurnEndWithFile(ev, () => {
      hearAtNode(el, entry.id, event, ev);
      if (ev.__logosFile) hearAtNode(el, entry.id, FILE_DROPPED, ev);
    });
  };
  deliveredAt(el, event).addEventListener(event, entry.fn, listenOptions(flags));
  on[event] = entry;
  pageListeners.add(entry);
};
const pageUnlisten = (el, event) => {
  const entry = pageEntry(el, event);
  if (!entry) return;
  deliveredAt(el, event).removeEventListener(event, entry.fn, listenOptions(entry.flags));
  delete el.__logosListen[event];
  pageListeners.delete(entry);
};
// A patch stream's `Listen`/`Unlisten`: the element's own listener, holding the event's prevent
// exactly while its flags cancel.
const streamListen = (el, event, flags, id) => {
  const had = pageEntry(el, event);
  const held = had !== undefined && cancels(had.flags);
  pageListen(el, event, flags, id);
  if (cancels(flags) && !held) holdPrevent(el, event);
  if (!cancels(flags) && held) dropPrevent(el, event);
};
const streamUnlisten = (el, event) => {
  const had = pageEntry(el, event);
  if (had && cancels(had.flags)) dropPrevent(el, event);
  pageUnlisten(el, event);
};

// ── The relay: one WebSocket for the page, many topics ─────────────────────────────────────────
//
// Opened on the first subscription and shared by every topic. Each message is one
// `logicaffeine_system::relay_proto::RelayFrame` in bincode, as the system's relay and its native
// and browser clients write it: the variant's index as a u32, then each field — a string or a
// byte string as a u64 length and its bytes — all little-endian. `Subscribe` and `Publish` go
// out, `SubAck` and `Event` come in; a page's payloads are UTF-8 text. A topic is
// subscribed when its first handler arrives, and every handler lasts for the life of the page.
const RELAY_VARIANTS = ['Subscribe', 'Unsubscribe', 'Publish', 'SubAck', 'Event'];
const RELAY_CARRIES_DATA = new Set(['Publish', 'Event']);
const strictUtf8 = new TextDecoder('utf-8', { fatal: true });
const relayCodec = {
  // `{ Variant: { topic, data } }`, `data` bytes, to the frame's bytes.
  encode(frame) {
    const kind = Object.keys(frame)[0];
    const fields = [enc.encode(frame[kind].topic)];
    if (RELAY_CARRIES_DATA.has(kind)) fields.push(frame[kind].data);
    const out = new Uint8Array(4 + fields.reduce((n, f) => n + 8 + f.length, 0));
    const view = new DataView(out.buffer);
    view.setUint32(0, RELAY_VARIANTS.indexOf(kind), true);
    let at = 4;
    for (const f of fields) {
      view.setBigUint64(at, BigInt(f.length), true);
      out.set(f, at + 8);
      at += 8 + f.length;
    }
    return out;
  },
  // A frame's bytes to `{ Variant: { topic, data } }`, or `null` for bytes that are no frame —
  // an unknown variant, a length past the end, a topic that is not UTF-8. Bytes after the frame
  // are ignored, as the system's reader ignores them.
  decode(bytes) {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (bytes.length < 4) return null;
    const kind = RELAY_VARIANTS[view.getUint32(0, true)];
    if (!kind) return null;
    let at = 4;
    const field = () => {
      if (at + 8 > bytes.length) return null;
      const len = view.getBigUint64(at, true);
      if (len > BigInt(bytes.length - at - 8)) return null;
      const f = bytes.subarray(at + 8, at + 8 + Number(len));
      at += 8 + Number(len);
      return f;
    };
    const topicBytes = field();
    if (!topicBytes) return null;
    let topic;
    try { topic = strictUtf8.decode(topicBytes); } catch { return null; }
    if (!RELAY_CARRIES_DATA.has(kind)) return { [kind]: { topic } };
    const data = field();
    if (!data) return null;
    return { [kind]: { topic, data: data.slice() } };
  },
};
globalThis.__logosRelayCodec = relayCodec;
const relay = {
  sock: null,
  subs: new Map(),
  queue: [],
  url() {
    const p = location.protocol === 'https:' ? 'wss:' : 'ws:';
    return `${p}//${location.host}/_relay`;
  },
  open() {
    if (this.sock) return;
    const s = new WebSocket(this.url());
    s.binaryType = 'arraybuffer';
    this.sock = s;
    s.onopen = () => { for (const f of this.queue.splice(0)) s.send(relayCodec.encode(f)); };
    s.onmessage = (ev) => {
      if (!(ev.data instanceof ArrayBuffer)) return;
      const frame = relayCodec.decode(new Uint8Array(ev.data));
      const e = frame && frame.Event;
      if (!e) return;
      // The payload is presented as an event carrying a value, read by `eventValue` like every
      // other carrying event.
      const carried = { target: { value: dec.decode(e.data) } };
      for (const slot of [...(this.subs.get(e.topic) || [])]) {
        if (slot.deliver) slot.deliver(carried.target.value); else dispatch(slot.handle, carried);
      }
    };
    s.onclose = () => { this.sock = null; };
  },
  send(frame) {
    if (this.sock && this.sock.readyState === 1) this.sock.send(relayCodec.encode(frame));
    else this.queue.push(frame);
  },
  subscribe(topic, slot) {
    let slots = this.subs.get(topic);
    if (!slots) {
      slots = new Set();
      this.subs.set(topic, slots);
      this.open();
      this.send({ Subscribe: { topic } });
    }
    slots.add(slot);
  },
};

// ── The fetch engine ────────────────────────────────────────────────────────────────────────────
//
// Every request gets an id; status, headers, body and error land PER REQUEST, and the completion
// closure receives its own request's id through the trampoline, so concurrent fetches cannot
// misdeliver. A request is finalized exactly once (an aborted or finished request ignores late
// results) and forgotten when the turn that completed it ends. Mirrors the native recorder
// (`semantics::dom`).
const FETCH_BODY_CAP = 4194304;
const FETCH_ALLOWED_HEADERS = ['accept', 'accept-language', 'authorization', 'cache-control', 'content-type', 'idempotency-key', 'if-match', 'if-none-match', 'x-csrf-token', 'x-expected-version', 'x-requested-with'];
// The headers a request goes out with, `[name, value, …]`: the page's own, and — for a request that
// changes state (`POST`, `PUT`, `PATCH`, `DELETE`) to the page's own server (a URL with no scheme and
// no host) that sets none — the CSRF token the document's `<meta name="csrf-token">` holds, the one
// the server that composed the page wrote there. `semantics::dom`'s `request_headers` is the same
// rule in process.
const requestHeaders = (method, url, headers) => {
  const changes = ['POST', 'PUT', 'PATCH', 'DELETE'].includes(method.toUpperCase());
  const own = !url.includes('://') && !url.startsWith('//');
  const sets = headers.some((h, i) => i % 2 === 0 && h.toLowerCase() === 'x-csrf-token');
  const meta = document.head && Array.from(document.head.querySelectorAll('meta[name]')).find((m) => m.getAttribute('name').toLowerCase() === 'csrf-token');
  if (!changes || !own || sets || !meta || !meta.hasAttribute('content')) return headers;
  return [...headers, 'x-csrf-token', meta.getAttribute('content')];
};
const fetchReqs = new Map();
let nextFetch = 1, currentFetch = 0;
const forgetFetch = (id) => {
  const q = fetchReqs.get(id);
  if (!q) return;
  fetchReqs.delete(id);
  releaseHandle(q.handler);
  if (currentFetch === id) currentFetch = 0;
};
const fetchDone = (id, r) => {
  const q = fetchReqs.get(id);
  if (!q || q.done) return;
  q.done = true;
  Object.assign(q, r);
  if (q.timer) { clearTimeout(q.timer); q.timer = 0; }
  currentFetch = id;
  afterTurn.push(() => forgetFetch(id));
  guarded(() => { inst.exports.__logos_call_closure(q.handler, BigInt(id)); settle(); });
};
const fetchSend = (method, url, body, pageHeaders, handler) => {
  const headers = requestHeaders(method, url, pageHeaders);
  const id = nextFetch++;
  const q = { handler, done: false, status: 0, headers: new Map(), body: '', error: '', ctrl: new AbortController(), timer: 0, timedOut: false };
  fetchReqs.set(id, q);
  if (body.length > FETCH_BODY_CAP) { queueMicrotask(() => fetchDone(id, { error: 'toolarge' })); return BigInt(id); }
  const bad = headers.filter((h, i) => i % 2 === 0 && !FETCH_ALLOWED_HEADERS.includes(h.toLowerCase()));
  if (bad.length) { queueMicrotask(() => fetchDone(id, { error: 'network: header not allowed: ' + bad[0].toLowerCase() })); return BigInt(id); }
  const hs = {};
  for (let i = 0; i + 1 < headers.length; i += 2) hs[headers[i]] = headers[i + 1];
  fetch(url, { method, headers: hs, body: method === 'GET' || method === 'HEAD' ? undefined : body, signal: q.ctrl.signal })
    .then(async (r) => {
      const t = await r.text();
      if (t.length > FETCH_BODY_CAP) { fetchDone(id, { error: 'toolarge' }); return; }
      const rh = new Map();
      r.headers.forEach((v, k) => rh.set(k.toLowerCase(), v));
      fetchDone(id, { status: r.status, headers: rh, body: t });
    })
    .catch((e) => fetchDone(id, { error: q.timedOut ? 'timeout' : (e && e.name === 'AbortError' ? 'abort' : 'network: ' + (e && e.message ? e.message : String(e))) }));
  return BigInt(id);
};

// The settle loop: after every entry into LOGOS, re-run a hand-written top-level `render`
// function (exported as `__logos_render` — the immediate-mode re-render idiom) if the module
// declares one.
const settle = () => { if (inst.exports.__logos_render) inst.exports.__logos_render(); };

// One named operation on one element. `focus`/`blur` raise the browser's own events, so a page
// listening for them is answered by the platform.
const elementCommand = (e, c) => { if (c === 'focus') e.focus(); else if (c === 'blur') e.blur(); else if (c === 'click') e.click(); else if (c === 'scrollto') e.scrollIntoView(); else if (c === 'selecttext') { if (e.select) e.select(); } else if (c === 'hover') e.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); };
// What makes two head tags THE SAME TAG — the identity a page's own version replaces (mirrors
// `web_host.rs`'s `head_identity`, which computes the SAME keys for the server-rendered shell, so a
// page merges identically whichever way its head reached the document). `null` means the tag
// replaces nothing and is simply added.
const headIdentity = (el) => {
  const tag = el.tagName.toLowerCase();
  if (tag === 'title') return 'title';
  if (tag === 'base') return 'base';
  if (tag === 'meta') {
    if (el.hasAttribute('charset')) return 'meta:charset';
    const name = el.getAttribute('name');
    if (name != null) {
      const media = el.getAttribute('media');
      return media != null ? `meta:name=${name.toLowerCase()}@${media.toLowerCase()}` : `meta:name=${name.toLowerCase()}`;
    }
    const prop = el.getAttribute('property');
    return prop != null ? `meta:property=${prop.toLowerCase()}` : null;
  }
  if (tag === 'link') {
    const rel = el.getAttribute('rel');
    return rel != null ? `link:rel=${rel.toLowerCase()}` : null;
  }
  return null;
};
// `Head:`'s rendered `<title>`/`<meta>`/`<link>`/`<base>` fragment, merged into the live
// `document.head` over whatever is already there (the default set the host shell ships, or an
// earlier merge): a tag with an identity above REPLACES the existing tag of that identity;
// everything else is simply added.
const mergeIntoHead = (html) => {
  const tmpl = document.createElement('template');
  tmpl.innerHTML = html;
  for (const el of Array.from(tmpl.content.children)) {
    const id = headIdentity(el);
    if (id) {
      for (const existing of Array.from(document.head.children)) {
        if (headIdentity(existing) === id) existing.remove();
      }
    }
    document.head.appendChild(el);
  }
};
// `on-<event>.prevent`: suppress the browser's own action for this event on this element, so a
// handled form does not also post itself. A property of the registration — held once however
// often it is asked for — rather than of any one firing.
const preventDefault = (id, name) => {
  const n = node(id);
  if (n) preventAlways(n, name);
};
// `listenFlags(el, event, flags)`: the page listens for `event` on `el`, with `flags`.
const listenFlags = (id, name, flags) => {
  const n = node(id);
  if (!n) return;
  pageListen(n, name, flags, id);
  if (cancels(flags)) preventAlways(n, name);
};

// ── The patch stream (`semantics::patch`), both wire formats ─────────────────────────────────────
//
// ONE decoder over the opcode table (`semantics::patch::OPCODES`), reading operands through the
// format's own readers: the byte format (u32 nodes, length-prefixed text) or wire v2 (a word per
// operand, text as a 1-based index into a string table). Decoding finishes before anything is
// applied, so a malformed stream paints nothing.
const decodePatches = (read) => {
  const ops = [];
  while (read.more()) {
    const code = read.code();
    switch (code) {
      case 0: throw new Error(`patch opcode 0 (BeginPaint) at ${read.where()} is retired: a paint's boundary is the commit that emits the stream, and no build since the retained tree writes it.`);
      case 1: ops.push(['createElement', read.node(), read.text()]); break;
      case 2: ops.push(['createText', read.node(), read.text()]); break;
      case 3: ops.push(['setText', read.node(), read.text()]); break;
      case 4: ops.push(['setHtml', read.node(), read.text()]); break;
      case 5: ops.push(['clearChildren', read.node()]); break;
      case 6: ops.push(['setAttribute', read.node(), read.text(), read.text()]); break;
      case 7: ops.push(['appendChild', read.node(), read.node()]); break;
      case 8: ops.push(['setValue', read.node(), read.text()]); break;
      case 9: ops.push(['setChecked', read.node(), read.flag()]); break;
      case 10: ops.push(['addEventListener', read.node(), read.text()]); break;
      case 11: ops.push(['setStream', read.node(), read.i64()]); break;
      case 12: ops.push(['preventDefault', read.node(), read.text()]); break;
      case 13: throw new Error(`patch opcode 13 (MoveChildren) at ${read.where()} is retired: a slot's children are placed by the commit that emits the stream, and no build since the retained tree writes it.`);
      case 14: ops.push(['removeNode', read.node()]); break;
      case 15: ops.push(['insertBefore', read.node(), read.node(), read.node()]); break;
      case 16: ops.push(['removeAttribute', read.node(), read.text()]); break;
      case 17: ops.push(['adopt', read.node(), read.node(), read.text()]); break;
      case 18: ops.push(['adoptText', read.node(), read.node(), read.text()]); break;
      case 19: ops.push(['listen', read.node(), read.text(), read.byte()]); break;
      case 20: ops.push(['unlisten', read.node(), read.text()]); break;
      case 21: ops.push(['freeNode', read.node()]); break;
      case 22: ops.push(['hydrate', read.node()]); break;
      default: throw new Error(`unknown patch opcode ${code} at ${read.where()}`);
    }
  }
  return ops;
};
const byteOperands = (b) => {
  let at = 0;
  const need = (k) => { if (at + k > b.length) throw new Error(`patch stream truncated at byte ${at}`); };
  const byte = () => { need(1); return b[at++]; };
  const u32 = () => { need(4); const v = b[at] | (b[at+1]<<8) | (b[at+2]<<16) | (b[at+3]*0x1000000); at += 4; return v; };
  return {
    more() { return at < b.length; },
    code() { return byte(); },
    where() { return `byte ${at - 1}`; },
    node() { return u32(); },
    byte() { return byte(); },
    flag() { return byte() !== 0; },
    i64() { need(8); let v = 0n; for (let k = 7; k >= 0; k--) v = (v << 8n) | BigInt(b[at+k]); at += 8; return BigInt.asIntN(64, v); },
    text() { const len = u32(); need(len); const t = dec.decode(b.subarray(at, at + len)); at += len; return t; },
  };
};
const wordOperands = (w, s) => {
  let at = 0;
  const word = () => { if (at >= w.length) throw new Error(`word patch stream truncated at word ${at}`); return w[at++]; };
  const within = (v, max, what) => { if (v < 0n || v > max) throw new Error(`a word patch record carries ${v}, which is no ${what}, at word ${at - 1}`); return Number(v); };
  return {
    more() { return at < w.length; },
    code() { return within(word(), 255n, 'opcode'); },
    where() { return `word ${at - 1}`; },
    node() { return within(word(), 0xFFFFFFFFn, 'node id'); },
    byte() { return within(word(), 255n, 'flags byte'); },
    flag() { return within(word(), 1n, 'flag') !== 0; },
    i64() { return word(); },
    text() {
      const i = within(word(), BigInt(s.length), 'string index');
      if (i < 1) throw new Error(`a word patch record's string index 0 is out of range at word ${at - 1}`);
      return s[i - 1];
    },
  };
};
// ONE op frees an entire removed subtree: the host walks it, taking every listener the page holds
// on each node off and forgetting each node's id both ways (a freed id is exactly as gone as one
// that never existed), before detaching the root — so an event the removal itself raises (the
// `blur` of a focused field) finds nothing to call.
const freeSubtree = (id) => {
  const n = node(id);
  if (!n) { nodes.delete(id); return; }
  const stack = [n];
  while (stack.length) {
    const cur = stack.pop();
    for (const c of cur.childNodes) stack.push(c);
    dropOwnListeners(cur);
    for (const event of Object.keys(cur.__logosListen || {})) streamUnlisten(cur, event);
    const known = ids.get(cur);
    if (known !== undefined) { nodes.delete(known); ids.delete(cur); }
  }
  if (n.parentNode) n.parentNode.removeChild(n);
};
// A node a stream creates takes the id the stream names for it, and keeps it until the stream frees
// it: the EMITTER is the id authority. A stream id is at or above 2^31, where `nodeId` never mints.
const bindNode = (id, el) => {
  if (nodes.has(id)) throw new Error(`the patch stream creates node ${id}, which is still live`);
  nodes.set(id, el);
  ids.set(el, id);
};
const STREAM_APPEND = 0xFFFFFFFF;

// ── Hydration: a stream adopting the markup a pre-render put in the document ────────────────────
//
// `Hydrate(root)` opens it: the markup under `root` is the page's own first paint, drawn before the
// program ran, and the `data-logos-prerendered` mark saying so goes. `Adopt(parent, node, tag)` binds
// `node` to the next element of `parent` the markup holds when its tag is `tag`, and counts a
// mismatch otherwise. `AdoptText(parent, node, text)` takes its place in the run of text before the
// next element claimed under `parent`: the markup's text is split along the claims when they spell
// it, and rebuilt when they do not — a text node carries nothing a visitor did to it. A write the
// stream makes to an adopted element brings the markup's node in line: an attribute already holding
// the value is left alone (writing it again restarts an image and reloads a frame), a control's
// value and checked state are set as the properties they are, and a control the visitor changed
// before the program ran keeps the change and is heard as the `input` it raised then. When the
// stream ends, whatever the markup holds that no claim bound leaves, and an adopted element drops the
// attributes the stream did not write, among them the `value` and `checked` markup spells a control's
// state with. The stream's apply answers the mismatches; the emitter decides what they mean.
const PRERENDERED = 'data-logos-prerendered';
let hydration = null;
const hostOwn = (n) => n.nodeType === 1 && n.hasAttribute('data-logos-host');
const unclaimedFrom = (h, p) => { const last = h.cursors.get(p); return last ? last.nextSibling : p.firstChild; };
const nextElement = (c) => { while (c && (c.nodeType !== 1 || hostOwn(c))) c = c.nextSibling; return c; };
const openHydration = (op) => {
  if (!hydration) throw new Error(`the patch stream sends \`${op}\` outside a hydration: only a stream that opened one with \`Hydrate\` claims markup the document holds`);
  return hydration;
};
// How the text claims `run` tile the markup's text nodes `held`, in order: per held node the range
// of claims whose texts spell it exactly (an empty claim rides in any tile), or null when they do not
// spell the held text. Claims past the last tile are empty.
const textTiles = (held, run) => {
  const tiles = [];
  let k = 0;
  for (const t of held) {
    const start = k;
    let spelled = '';
    while (spelled.length < t.data.length && k < run.length) spelled += run[k++].text;
    if (spelled !== t.data) return null;
    tiles.push([start, k]);
  }
  for (let j = k; j < run.length; j++) if (run[j].text !== '') return null;
  return { tiles, trailing: k };
};
// The text under `p` between the child claimed last and `upTo` becomes what the waiting text claims
// for `p` name; a comment there is no page content and goes.
const settleTexts = (h, p, upTo) => {
  const run = h.texts.get(p) || [];
  h.texts.delete(p);
  const held = [];
  for (let c = unclaimedFrom(h, p); c && c !== upTo; ) {
    const next = c.nextSibling;
    if (c.nodeType === 3) held.push(c);
    else if (c.nodeType !== 1) c.remove();
    c = next;
  }
  const fresh = (r) => { const t = document.createTextNode(r.text); p.insertBefore(t, upTo); bindNode(r.node, t); };
  const tiled = textTiles(held, run);
  if (tiled === null) {
    for (const t of held) t.remove();
    for (const r of run) fresh(r);
    return;
  }
  held.forEach((t, i) => {
    const [from, to] = tiled.tiles[i];
    if (from === to) { t.remove(); return; }
    let rest = t;
    for (let j = from; j < to; j++) {
      const r = run[j];
      if (r.text === '') { const e = document.createTextNode(''); p.insertBefore(e, rest); bindNode(r.node, e); continue; }
      if (rest.data.length > r.text.length) { const tail = rest.splitText(r.text.length); bindNode(r.node, rest); rest = tail; }
      else bindNode(r.node, rest);
    }
  });
  for (let j = tiled.trailing; j < run.length; j++) fresh(run[j]);
};
const claimElement = (parent, id, tag) => {
  const h = openHydration('Adopt');
  const p = node(parent);
  if (!p || !h.cursors.has(p)) { h.mismatches++; return; }
  const el = nextElement(unclaimedFrom(h, p));
  if (!el || el.localName.toLowerCase() !== tag.toLowerCase()) { h.mismatches++; return; }
  settleTexts(h, p, el);
  bindNode(id, el);
  h.cursors.set(p, el);
  h.cursors.set(el, null);
  h.written.set(el, new Set());
};
const claimText = (parent, id, text) => {
  const h = openHydration('AdoptText');
  const p = node(parent);
  if (!p || !h.cursors.has(p)) { h.mismatches++; return; }
  const run = h.texts.get(p) || [];
  run.push({ node: id, text });
  h.texts.set(p, run);
};
const adoptedElement = (e) => hydration !== null && hydration.written.has(e);
const toggles = (e) => e instanceof HTMLInputElement && (e.type === 'checkbox' || e.type === 'radio');
// Whether the visitor changed control `e` from what its markup says, before the program ran.
const visitorChanged = (e) => {
  if (toggles(e)) return e.checked !== e.defaultChecked;
  if (e instanceof HTMLSelectElement) {
    if (e.multiple) return Array.from(e.options).some((o) => o.selected !== o.defaultSelected);
    const chosen = Array.from(e.options).find((o) => o.defaultSelected) || e.options[0];
    return e.selectedIndex !== (chosen ? chosen.index : -1);
  }
  return 'defaultValue' in e && e.value !== e.defaultValue;
};
const keepTheVisitors = (e) => {
  if (!toggles(e) || e.checked) hydration.heard.push(e);
};
const endHydration = () => {
  const h = hydration;
  hydration = null;
  for (const [p, last] of h.cursors) {
    if (h.raw.has(p)) continue;
    const upTo = nextElement(last ? last.nextSibling : p.firstChild);
    settleTexts(h, p, upTo);
    for (let c = upTo; c; ) {
      const next = c.nextSibling;
      if (!hostOwn(c)) c.remove();
      c = next;
    }
  }
  for (const [e, written] of h.written) {
    for (const name of e.getAttributeNames()) if (!written.has(name)) e.removeAttribute(name);
  }
  for (const e of h.heard) {
    e.dispatchEvent(new Event('input', { bubbles: true }));
    if (toggles(e) || e instanceof HTMLSelectElement) e.dispatchEvent(new Event('change', { bubbles: true }));
  }
  return h.mismatches;
};

// A stream applied by the ids it names; answers its hydration's mismatches (0 when it opened none).
const applyByIds = (ops) => {
  try { applyEach(ops); } catch (err) { hydration = null; throw err; }
  return hydration ? endHydration() : 0;
};
const applyEach = (ops) => {
  for (const op of ops) {
    switch (op[0]) {
      case 'createElement': bindNode(op[1], document.createElement(op[2])); break;
      case 'createText': bindNode(op[1], document.createTextNode(op[2])); break;
      case 'setText': { const e = node(op[1]); if (e) e.textContent = op[2]; break; }
      case 'setHtml': {
        const e = node(op[1]);
        if (!e) break;
        if (adoptedElement(e)) {
          hydration.raw.add(e);
          const t = document.createElement('template');
          t.innerHTML = op[2];
          if (t.innerHTML === e.innerHTML) break;
        }
        e.innerHTML = op[2];
        break;
      }
      case 'clearChildren': { const p = node(op[1]); if (p) while (p.firstChild) p.removeChild(p.firstChild); break; }
      case 'setAttribute': {
        const e = node(op[1]);
        if (!e) break;
        if (adoptedElement(e)) { hydration.written.get(e).add(op[2]); if (e.getAttribute(op[2]) === op[3]) break; }
        e.setAttribute(op[2], op[3]);
        break;
      }
      case 'removeAttribute': { const e = node(op[1]); if (e) e.removeAttribute(op[2]); break; }
      case 'appendChild': { const p = node(op[1]), c = node(op[2]); if (p && c) p.appendChild(c); break; }
      case 'insertBefore': { const p = node(op[1]), c = node(op[2]); if (p && c) p.insertBefore(c, op[3] === STREAM_APPEND ? null : node(op[3])); break; }
      case 'removeNode': { const n = node(op[1]); if (n && n.parentNode) n.parentNode.removeChild(n); break; }
      case 'setValue': {
        const e = node(op[1]);
        if (!e) break;
        if (adoptedElement(e)) {
          if (visitorChanged(e)) { keepTheVisitors(e); break; }
          e.value = op[2];
          break;
        }
        if (e.value !== op[2]) e.value = op[2];
        break;
      }
      case 'setChecked': {
        const e = node(op[1]);
        if (!e) break;
        if (adoptedElement(e)) {
          if (visitorChanged(e)) { keepTheVisitors(e); break; }
          e.checked = op[2];
          break;
        }
        if (e.checked !== op[2]) e.checked = op[2];
        break;
      }
      case 'preventDefault': preventDefault(op[1], op[2]); break;
      case 'listen': { const e = node(op[1]); if (e) streamListen(e, op[2], op[3], op[1]); break; }
      case 'unlisten': { const e = node(op[1]); if (e) streamUnlisten(e, op[2]); break; }
      case 'freeNode': freeSubtree(op[1]); break;
      case 'hydrate': {
        if (hydration) throw new Error(`the patch stream hydrates node ${op[1]} while an earlier hydration in it is still open`);
        const root = node(op[1]);
        if (root) root.removeAttribute(PRERENDERED);
        hydration = { cursors: new Map(root ? [[root, null]] : []), texts: new Map(), written: new Map(), raw: new Set(), heard: [], mismatches: root ? 0 : 1 };
        break;
      }
      case 'adopt': claimElement(op[1], op[2], op[3]); break;
      case 'adoptText': claimText(op[1], op[2], op[3]); break;
      case 'addEventListener': case 'setStream': break;
    }
  }
};

const exactHeld = [], exactFree = [];
const exactValue = (t) => ((t & 1n) === 0n ? t >> 1n : exactHeld[Number(t >> 1n)]);
const exactWord = (v) => {
  if (v >= -(1n << 62n) && v < (1n << 62n)) return v << 1n;
  const h = exactFree.length > 0 ? exactFree.pop() : exactHeld.length;
  exactHeld[h] = v;
  return (BigInt(h) << 1n) | 1n;
};
const exactImports = {
  exact_box: (v) => exactWord(v),
  exact_narrow: (t) => { const v = exactValue(t); return v >= -(1n << 63n) && v < (1n << 63n) ? v : 0n; },
  exact_add: (a, b) => exactWord(exactValue(a) + exactValue(b)),
  exact_sub: (a, b) => exactWord(exactValue(a) - exactValue(b)),
  exact_mul: (a, b) => exactWord(exactValue(a) * exactValue(b)),
  exact_div: (a, b) => exactWord(exactValue(a) / exactValue(b)),
  exact_mod: (a, b) => exactWord(exactValue(a) % exactValue(b)),
  exact_floordiv: (a, b) => { const x = exactValue(a), y = exactValue(b), q = x / y; return exactWord(x % y !== 0n && (x < 0n) !== (y < 0n) ? q - 1n : q); },
  exact_pow: (a, e) => exactWord(exactValue(a) ** e),
  exact_cmp: (a, b) => { const x = exactValue(a), y = exactValue(b); return x < y ? -1 : x > y ? 1 : 0; },
  exact_f64: (t) => Number(exactValue(t)),
  exact_clone: (t) => exactWord(exactValue(t)),
  exact_drop: (t) => { const h = Number(t >> 1n); exactHeld[h] = undefined; exactFree.push(h); },
};

const env = {
  ...exactImports,
  print_exact: (t) => console.log(exactValue(t).toString()),
  exact_text: (t) => allocText(exactValue(t).toString()),
  print_i64: (v) => console.log(v.toString()),
  print_text: (h) => console.log(readText(h)),
  print_bool: (v) => console.log(v ? 'true' : 'false'),
  print_f64: (v) => console.log(fmtF(v)),
  print_char: (v) => console.log(String.fromCodePoint(Number(v))),
  print_word: (v) => console.log(BigInt.asUintN(64, v).toString()),
  print_date: (d) => console.log(printDate(d)),
  print_moment: (n) => console.log(printMoment(n)),
  print_time: (n) => console.log(printTime(n)),
  print_duration: (n) => console.log(printDuration(n)),
  print_span: (packed) => console.log(printSpan(packed)),
  print_rational: (num, den) => console.log(den === 1n ? num.toString() : `${num}/${den}`),
  print_nothing: () => console.log('nothing'),
  print_seq_i64: (h) => console.log(displaySeqI64(h)),
  print_seq_bool: (h) => console.log(displaySeqBool(h)),
  print_seq_word32: (h) => console.log(displaySeqWord32(h)),
  print_seq_word64: (h) => console.log(displaySeqWord64(h)),
  print_seq_f64: (h) => console.log(displaySeqF64(h)),
  print_seq_text: (h) => console.log(displaySeqText(h)),
  print_set_i64: (h) => console.log(displaySetI64(h)),
  print_set_text: (h) => console.log(displaySetText(h)),
  fmt_i64_into: (buf, v) => writeStr(buf, v.toString()),
  fmt_f64_into: (buf, v) => writeStr(buf, fmtF(v)),
  fmt_bool_into: (buf, v) => writeStr(buf, v ? 'true' : 'false'),
  fmt_f64_prec_into: (buf, v, prec) => writeStr(buf, v.toFixed(Number(prec))),
  fmt_align_into: (buf, text, width, align) => writeStr(buf, alignPad(readText(text), Number(width), Number(align))),
  fmt_seq_i64_into: (buf, h) => writeStr(buf, displaySeqI64(h)),
  fmt_seq_bool_into: (buf, h) => writeStr(buf, displaySeqBool(h)),
  fmt_set_i64_into: (buf, h) => writeStr(buf, displaySetI64(h)),
  parse_int: (h) => BigInt(parseInt(readText(h).trim(), 10) || 0),
  parse_float: (h) => parseFloat(readText(h).trim()),
  read_reals: (_path) => { throw new Error('readReals: a page in the browser has no files to read'); },
  parse_timestamp: (h) => parseRfc3339(readText(h)),
  text_contains: (haystack, needle) => (readText(haystack).includes(readText(needle)) ? 1n : 0n),
  random_int: (lo, hi) => randomIntIn(lo, hi),
  pow_ff: (a, b) => Math.pow(a, b),
  pow_fi: (a, b) => Math.pow(a, Number(b)),
  // Real wall-clock time: no test-fixed-clock concept applies to a compiled module actually
  // running in a browser or under node — only the Rust `wasmi` test host (which reads the
  // tree-walker's own thread-local fixed clock) needs that determinism.
  today: () => Math.floor(Date.now() / 86400000),
  now: () => BigInt(Date.now()) * 1000000n,
  args: () => 0,
  // A page has no process environment: every variable `get` names is unset.
  env_len: (h) => -1,
  env_into: (buf, h) => 0,
  time_now: () => BigInt(Date.now()),
  // `write_wire_residual(data_ptr, len) -> the byte count` — the raw bytes written straight out;
  // a browser has no stdout, so the decoded text is logged (best-effort — this protocol is a
  // native/CLI wire-program mechanism, not a browser-page one).
  write_wire_residual: (dataPtr, len) => {
    const bytes = u8().slice(Number(dataPtr), Number(dataPtr) + Number(len));
    console.log(dec.decode(bytes));
    return BigInt(len);
  },
  // Wave 13 observability: `Op::Obs` is populated only by the OBSERVED compile profile, which the
  // browser/CLI build never uses — declared for ABI completeness, never actually reached today.
  obs: (_site, _p0, _p1, _p2) => {},
  temporal_component: (nanos, which) => temporalComponent(nanos, which),
  temporal_component_date: (days, which) => temporalComponentDate(days, which),
  dom_body: () => 0n,
  // The imperative DOM calls act on the document directly. An op on an id the host no longer holds
  // does nothing.
  dom_create_element: (tag) => nodeId(document.createElement(readText(tag))),
  dom_create_text_node: (t) => nodeId(document.createTextNode(readText(t))),
  dom_set_text: (el, t) => { const n = node(el); if (n) n.textContent = readText(t); },
  dom_set_html: (el, h) => { const n = node(el); if (n) n.innerHTML = readText(h); },
  dom_clear_children: (el) => { const n = node(el); if (n) while (n.firstChild) n.removeChild(n.firstChild); },
  dom_set_attribute: (el, n, v) => { const e = node(el); if (e) e.setAttribute(readText(n), readText(v)); },
  // A boolean HTML attribute's PRESENCE (`disabled={not formValid}`): `on` is the Bool itself
  // (i64 in the module's own value model, arriving here as a JS BigInt from wasm), never text —
  // see `RuntimeFn`/`HostFn::DomSetBoolAttribute`'s doc in `vm/wasm/module.rs`.
  dom_set_bool_attribute: (el, n, on) => {
    const name = readText(n);
    const e = node(el);
    if (!e) return;
    if (on) e.setAttribute(name, ''); else e.removeAttribute(name);
  },
  dom_append_child: (p, c) => { const a = node(p), b = node(c); if (a && b) a.appendChild(b); },
  // A relay subscription, for the life of the page.
  dom_on_message: (topic, handler) => { relay.subscribe(readText(topic), { handle: handler }); },
  // A publish also reaches this page's own subscribers, decided in compiled LOGOS; this sends the
  // frame. The relay excludes the originating connection, so the two never deliver twice.
  dom_publish: (topic, message) => { const t = readText(topic); relay.open(); relay.send({ Publish: { topic: t, data: enc.encode(readText(message)) } }); },
  // `noteTurn(name, key, changes, failure)` — the lowered tier's journal entry. The reference
  // tier's `the journal`/`Replay` are tree-walker constructs with no browser equivalent, so the
  // browser keeps nothing; it exists so a lowered turn that journals itself still loads.
  dom_note_turn: (_name, _key, _changes, _failure) => {},
  // `journalDetail()` — whether a lowered turn records the fields it changed: only under the test
  // harness, which reads the journal; a shipped page keeps no journal and skips the capture.
  dom_journal_detail: () => (globalThis.__logosHarness ? 1n : 0n),
  // Browser storage. The area is validated rather than defaulted: a page written against an area
  // nobody defines would store its state where nothing reads it. `storage_get` answers handle `0`
  // for a missing key, which the compiled side reads as `Nothing` (an empty string is a value a
  // page can store). Storage can throw (private mode, a full quota, blocked site data): a failed
  // read reads as absent, a failed write is reported and dropped.
  storage_set: (area, key, value) => { const s = storageArea(readText(area)); try { s.setItem(readText(key), readText(value)); } catch (e) { console.warn('storage is unavailable:', e && e.message); } },
  storage_get: (area, key) => { const s = storageArea(readText(area)); let v = null; try { v = s.getItem(readText(key)); } catch (e) { v = null; } return v === null ? 0 : allocText(v); },
  storage_remove: (area, key) => { const s = storageArea(readText(area)); try { s.removeItem(readText(key)); } catch (e) {} },
  dom_get_text: (el) => { const n = node(el); return allocText((n && n.textContent) || ''); },
  dom_get_value: (el) => { const n = node(el); return allocText((n && n.value) || ''); },
  // A whole paint as ONE buffer, decoded and applied without decisions of its own. The format is
  // `crates/logicaffeine_compile/src/semantics/patch.rs`: a 1-byte opcode then its operands; nodes
  // are u32; text is a u32 BYTE length then that many UTF-8 bytes. The whole buffer is decoded
  // before anything is applied, so a malformed stream is refused whole and never half-paints.
  // `patch_opcode_parity_lock` holds this decoder to the encoder's opcode list.
  dom_apply: (patches) => {
    const d = dv(), n = d.getInt32(patches, true), dp = d.getInt32(patches + 8, true);
    const b = new Uint8Array(n);
    for (let i = 0; i < n; i++) b[i] = Number(d.getBigInt64(dp + i * 8, true)) & 0xff;
    const ops = decodePatches(byteOperands(b));
    for (const op of ops) {
      switch (op[0]) {
        case 'createElement': nodeId(document.createElement(op[2])); break;
        case 'createText': nodeId(document.createTextNode(op[2])); break;
        case 'setText': { const e = node(op[1]); if (e) e.textContent = op[2]; break; }
        case 'setHtml': { const e = node(op[1]); if (e) e.innerHTML = op[2]; break; }
        case 'clearChildren': { const p = node(op[1]); if (p) while (p.firstChild) p.removeChild(p.firstChild); break; }
        case 'setAttribute': { const e = node(op[1]); if (e) e.setAttribute(op[2], op[3]); break; }
        case 'appendChild': { const p = node(op[1]), c = node(op[2]); if (p && c) p.appendChild(c); break; }
        case 'setValue': { const e = node(op[1]); if (e) e.value = op[2]; break; }
        case 'setChecked': { const e = node(op[1]); if (e) e.checked = op[2]; break; }
        // A listener's handler is a closure in the module, not data: the stream carries only the
        // registration, and the module attaches the handler through `dom_add_event_listener`.
        case 'addEventListener': break;
        // The same registration `dom_prevent_default` makes, so the two paths cannot disagree
        // about what a modifier does.
        case 'preventDefault': preventDefault(op[1], op[2]); break;
        // An opaque host MediaStream, which this host does not mint; carried by the format for a
        // host that does, and ignored here.
        case 'setStream': break;
        // A keyed diff's single-row removal — the retained node's own `.remove()`, never a
        // wholesale `clearChildren`/rebuild of its parent.
        case 'removeNode': { const n = node(op[1]); if (n) n.remove(); break; }
        // A keyed reorder's minimal move: `child` keeps its identity (and whatever focus,
        // selection or typed value it carries) by being MOVED, never recreated. `before ===
        // 0xFFFFFFFF` is the "no such element" sentinel `patch.rs` documents — append.
        case 'insertBefore': {
          const p = node(op[1]), c = node(op[2]);
          if (p && c) p.insertBefore(c, op[3] === 0xFFFFFFFF ? null : node(op[3]));
          break;
        }
        case 'removeAttribute': { const e = node(op[1]); if (e) e.removeAttribute(op[2]); break; }
        // A hydration's claims name the ids the retained tree emits them under; they arrive only
        // in the word stream a commit hands over, which `dom_apply_words` applies by those ids.
        case 'hydrate':
        case 'adopt':
        case 'adoptText':
          throw new Error(`a byte patch stream sends \`${op[0]}\`: a hydration arrives only in the word stream a vdom commit hands over`);
        // A listener's handler is a closure the module attaches directly through
        // `dom_add_event_listener`, exactly like `addEventListener` above; the stream carries its
        // flags, applied as `dom_listen_flags` applies them.
        case 'listen': listenFlags(op[1], op[2], op[3]); break;
        case 'unlisten': break;
        case 'freeNode': freeSubtree(op[1]); break;
      }
    }
  },
  // The word-format (wire v2) stream `vdom.lg`'s `vdApply` hands over: the first `wordCount` words
  // of `words`, whose text operands index the first `stringCount` strings, applied by the node ids
  // they name. Answers the stream's hydration mismatches.
  dom_apply_words: (words, wordCount, strings, stringCount) => {
    const d = dv(), wp = d.getInt32(words + 8, true), sp = d.getInt32(strings + 8, true);
    const w = [];
    for (let i = 0; i < Number(wordCount); i++) w.push(d.getBigInt64(wp + i * 8, true));
    const s = [];
    for (let i = 0; i < Number(stringCount); i++) s.push(readText(d.getInt32(sp + i * 8, true)));
    return BigInt(applyByIds(decodePatches(wordOperands(w, s))));
  },
  dom_set_value: (el, v) => { const n = node(el); if (n) n.value = readText(v); },
  // A program's own `Type`/`Choose`: the browser's default action on the control, answering whether
  // it took the value — a text field a visitor can edit takes typing, an enabled `<select>` offering
  // the option (by value, else by text) takes the choice.
  dom_enter_value: (verb, sel, value) => {
    const n = document.querySelector(readText(sel));
    const v = readText(value);
    if (!n || n.matches(':disabled')) return 0n;
    if (readText(verb) === 'choose') {
      const opts = n instanceof HTMLSelectElement ? Array.from(n.options) : [];
      const opt = opts.find((o) => o.value === v) || opts.find((o) => o.text === v);
      if (!opt) return 0n;
      n.value = opt.value;
      return 1n;
    }
    const typed = n.localName === 'textarea' || (n.localName === 'input' && ['text', 'search', 'url', 'tel', 'email', 'password', 'number'].includes(n.type));
    if (!typed || !n.matches(':read-write')) return 0n;
    n.value = v;
    return 1n;
  },
  // A driving statement the program runs on its own page (`Click "#go".` in `## Main`): the page
  // hears the event as an entry of its own, as it hears a visitor's, so a turn that fails there is
  // contained to it — the failure reported through the page's failure door (`reportRuntimeFailure`)
  // — and the program runs on. The dispatcher answers whether the page heard the event; a failed
  // turn was heard. The enclosing entry's captured message is kept aside and put back, so a later
  // failure of the program reports its own words.
  dom_page_entry: (node, event, value) => {
    const outer = failMessage;
    failMessage = '';
    try {
      return dispatchNode(node, event, value);
    } catch (err) {
      reportRuntimeFailure(err, failMessage);
      return 1n;
    } finally {
      failMessage = outer;
    }
  },
  // The PROPERTY, never `setAttribute('checked', …)` — the attribute is `defaultChecked`, so
  // writing it leaves the box the visitor sees as it was. `on` arrives as an i64 Bool.
  dom_set_checked: (el, on) => { const n = node(el); if (n) n.checked = Number(on) !== 0; },
  dom_prevent_default: (el, ev) => preventDefault(Number(el), readText(ev)),
  dom_query_selector: (sel) => { const n = document.querySelector(readText(sel)); return n ? nodeId(n) : -1n; },
  // A DOM listener of the element's own, for the life of the element.
  dom_add_event_listener: (el, event, handler) => {
    const n = node(el);
    if (n) ownListen(n, readText(event), handler);
    else releaseHandle(handler);
  },
  dom_listen_flags: (el, event, flags) => listenFlags(Number(el), readText(event), Number(flags)),
  // A control's reported state, as text. A checkbox's `value` is its markup's `"on"` whether it is
  // ticked or not, so a bound checkbox hears `checked` as `"true"`/`"false"` — what `bound_value`
  // in the lowering converts back to a Bool, and what the native tiers report.
  dom_event_value: (_ev) => {
    const t = currentEvent && currentEvent.target;
    if (!t) return allocText('');
    const ty = (t.type || '').toLowerCase();
    if (ty === 'checkbox' || ty === 'radio') return allocText(t.checked ? 'true' : 'false');
    return allocText(t.value || '');
  },
  // The key of a keyboard event as the browser names it (`"Enter"`, `"Escape"`, `"a"`); a key
  // modifier compares it case-insensitively. Not a keyboard event: the empty string.
  dom_event_key: (_ev) => allocText((currentEvent && currentEvent.key) || ''),
  dom_event_target: (_ev) => currentEvent && currentEvent.target ? nodeId(currentEvent.target) : -1n,
  dom_fetch_send: (method, url, body, headers, handler) => fetchSend(readText(method), readText(url), readText(body), readTextSeq(headers), handler),
  dom_fetch_start: (url, handler) => { fetchSend('GET', readText(url), '', [], handler); },
  dom_fetch_abort: (req) => { const id = Number(req), q = fetchReqs.get(id); if (!q || q.done) return; q.done = true; q.error = 'abort'; if (q.timer) { clearTimeout(q.timer); q.timer = 0; } q.ctrl.abort(); atTurnEnd(() => forgetFetch(id)); },
  dom_fetch_set_timeout: (req, millis) => { const q = fetchReqs.get(Number(req)); if (!q || q.done) return; if (q.timer) { clearTimeout(q.timer); q.timer = 0; } const ms = Number(millis); if (ms > 0) q.timer = setTimeout(() => { q.timedOut = true; q.ctrl.abort(); }, ms); },
  dom_response_status: (req) => { const q = fetchReqs.get(Number(req)); return BigInt(q && q.done ? q.status : 0); },
  dom_response_body: (req) => { const q = fetchReqs.get(Number(req)); return allocText(q && q.done ? q.body : ''); },
  dom_response_header: (req, name) => { const q = fetchReqs.get(Number(req)); return allocText((q && q.done && q.headers.get(readText(name).toLowerCase())) || ''); },
  dom_response_error: (req) => { const q = fetchReqs.get(Number(req)); return allocText(q && q.done ? q.error : ''); },
  dom_response_text: () => { const q = fetchReqs.get(currentFetch); return allocText(q ? q.body : ''); },
  // The history/location router: the path and query read `window.location`; navigating is
  // `history.pushState`/`replaceState`, which fire nothing — the page delivers its own move to its
  // own listeners; `dom_on_route` adds a route handler, and a browser back/forward (`popstate`)
  // runs every one of them.
  dom_route_path: () => allocText(window.location.pathname || '/'),
  dom_route_query: (name) => { const v = new URLSearchParams(window.location.search).get(readText(name)); return allocText(v == null ? '' : v); },
  dom_navigate: (path) => { window.history.pushState({}, '', readText(path)); },
  dom_replace_route: (path) => { window.history.replaceState({}, '', readText(path)); },
  // Another document at `url`: the browser leaves this page for it.
  dom_assign_location: (url) => { window.location.assign(readText(url)); },
  dom_on_route: (handler) => { routeHandlers.push(handler); },
  // The page's own title. `document.title` is the only place that can hold it.
  dom_set_title: (title) => { document.title = readText(title); },
  // `Head:`'s rendered fragment, merged into `document.head` (see `mergeIntoHead`).
  dom_set_head_html: (html) => { mergeIntoHead(readText(html)); },
  dom_reload: () => { window.location.reload(); },
  // The browser raises `popstate`, which the router already listens for.
  dom_go_back: () => { window.history.back(); },
  // The read half of the attribute pair; a missing attribute is empty, never null.
  dom_get_attribute: (el, name) => { const n = node(el); return allocText((n && n.getAttribute(readText(name))) || ''); },
  // Three capabilities, three permission models. Each is attempted and each may be refused by the
  // browser or the viewer; a refusal is not the page's failure, so none of them traps.
  dom_clipboard_write: (t) => { try { navigator.clipboard.writeText(readText(t)); } catch (_) {} },
  dom_download_file: (content, name) => { try { const b = new Blob([readText(content)]); const u = URL.createObjectURL(b); const a = document.createElement('a'); a.href = u; a.download = readText(name); a.click(); URL.revokeObjectURL(u); } catch (_) {} },
  dom_notify: (message, tone) => { try { const m = readText(message); if (window.Notification && Notification.permission === 'granted') new Notification(m); else console.info('LOGOS notify (' + readText(tone) + '): ' + m); } catch (_) {} },
  dom_element_command: (el, cmd) => { const e = node(el); if (e) elementCommand(e, readText(cmd)); },
  // The first file a drop carried — its name, size, type and content — as `hear` took it off the
  // transfer and read it before the turn began; nothing, empty and zero when the event carried none.
  dom_event_file_name: (_ev) => { const f = currentEvent && currentEvent.__logosFile; return allocText(f ? f.name : ''); },
  dom_event_file_size: (_ev) => { const f = currentEvent && currentEvent.__logosFile; return BigInt(f ? f.size : 0); },
  dom_event_file_type: (_ev) => { const f = currentEvent && currentEvent.__logosFile; return allocText(f ? f.type : ''); },
  dom_event_file_bytes: (_ev) => allocSeq((currentEvent && currentEvent.__logosFileBytes) || []),
  // An interval a program arms itself: its handler, every `ms` milliseconds.
  dom_set_interval: (ms, handler) => { setInterval(() => dispatch(handler, null), Number(ms)); },
  // Wake the page when its earliest timer comes due: one pending wake at a time, replaced by every
  // request, cancelled by a negative delay. Waking is an entry like any other, which passes the
  // time that went by to the page — at least the time to the timer it woke for.
  dom_wake_after: (ms) => {
    if (wakeTimer !== null) clearTimeout(wakeTimer);
    wakeTimer = null;
    wakeDue = null;
    wakeBeforeClock = null;
    const delay = Number(ms);
    if (delay < 0) return;
    if (clockAt === null) wakeBeforeClock = delay;
    else wakeDue = clockAt + delay;
    wakeTimer = setTimeout(() => {
      wakeTimer = null;
      owed = wakeDue;
      wakeDue = null;
      guarded(() => {});
    }, delay);
  },
  // Pixels of content below what is visible. Node 0 reads the document and the viewport; any other
  // node reads its own scroll box. The threshold is decided in compiled LOGOS.
  dom_scroll_gap: (el) => { const n = Number(el); if (n === 0) { const d = document.documentElement; return BigInt(Math.max(0, d.scrollHeight - (window.scrollY + window.innerHeight))); } const e = node(el); return e ? BigInt(Math.max(0, e.scrollHeight - (e.scrollTop + e.clientHeight))) : 0n; },
  // An event that happens to the PAGE (`online`, `offline`, `resize`), for the life of the page.
  dom_on_window_event: (event, handler) => {
    window.addEventListener(readText(event), (e) => hear(e, () => handler));
  },
  // A runtime failure hands its message over immediately before the module traps; without it the
  // trap reads `RuntimeError: unreachable`.
  logos_rt_fail: (h) => { failMessage = readText(h); },
  // An INLINE check (a bounds test, a division guard) has no message constant — its sentence
  // needs a value that only exists now. The module reports the code and the value; these
  // cases are held byte-identical to `semantics::failcode::message` by `fail_code_lock`.
  logos_rt_fail_at: (code, value) => {
    const v = typeof value === 'bigint' ? value.toString() : String(value);
    switch (code) {
      case 1: failMessage = 'Index ' + v + ' out of bounds'; break;
      case 2: failMessage = 'Division by zero'; break;
      case 3: failMessage = 'text_from_bytes(): invalid UTF-8'; break;
      case 4: failMessage = 'Out of memory'; break;
      case 5: failMessage = 'Cannot index List with Int'; break;
      case 6: failMessage = 'Cannot index Text with Int'; break;
      case 7: failMessage = 'List index must be an integer'; break;
      case 8: failMessage = 'integer value out of range for this int64 context'; break;
      default: failMessage = 'the program failed with an unrecognized code ' + code;
    }
  },
};
// The router's route handlers (LOGOS closure handles), in the order the page registered them: a
// back/forward runs each, and each reads the new path through `routePath()`.
const routeHandlers = [];
window.addEventListener('popstate', () => { for (const handler of [...routeHandlers]) dispatch(handler, null); });
// A click that follows a link to a path this page answers itself is client navigation: the location
// moves in place and the route handlers run, with no document load. The page decides which paths
// are its own (`__lgx_owns`, the same `matchRoute` its `Navigate to` branches on). Everything else
// — another page's path, a cancelled click, a modified or non-primary click, a download, a link to
// another window or origin — is the browser's to carry out. Heard at the document after the page's
// own listeners, so a listener's `.prevent` comes first.
document.addEventListener('click', (e) => {
  if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
  const a = e.target instanceof Element ? e.target.closest('a[href]') : null;
  if (!a || a.hasAttribute('download') || !['', '_self', '_top', '_parent'].includes((a.getAttribute('target') || '').trim().toLowerCase())) return;
  const url = new URL(a.href, location.href);
  if (url.origin !== location.origin || !inst || !inst.exports.__lgx_owns) return;
  if (Number(guarded(() => inst.exports.__lgx_owns(allocText(url.pathname)))) === 0) return;
  e.preventDefault();
  window.history.pushState({}, '', url.pathname + url.search + url.hash);
  for (const handler of [...routeHandlers]) dispatch(handler, null);
});
globalThis.__logosHostCensus = () => ({
  nodes: nodes.size,
  listeners: pageListeners.size,
  fetches: fetchReqs.size,
  topics: relay.subs.size,
});
// The identity `mergeIntoHead` replaces head elements by, for tests and devtools to read.
globalThis.__logosHeadIdentity = headIdentity;
// The headers a request the page makes goes out with, for tests and devtools to read.
globalThis.__logosRequestHeaders = requestHeaders;
// Mount a component under the element `selector` names (the page's body when it names none), as one
// guarded entry that commits its paint — what `main` does for the page's own mount. Only a module
// built for a host that mounts components on demand exports `__mount_<C>`; a shipped page has none.
globalThis.__logosMount = (component, selector) => {
  const mount = inst && inst.exports['__mount_' + component];
  if (typeof mount !== 'function') throw new Error(`this page was not built to mount ${component} on demand`);
  const parent = selector == null ? document.body : document.querySelector(selector);
  if (!parent) throw new Error(`nothing on the page matches ${JSON.stringify(selector)}, so ${component} has nowhere to mount`);
  guarded(() => { mount(nodeId(parent)); settle(); });
};
const bytes = await (await fetch(WASM_URL)).arrayBuffer();
const { instance } = await WebAssembly.instantiate(bytes, { env });
inst = instance;
// Through `guarded`, like every other entry. A trap still throws: nothing here pretends the program
// completed.
guarded(() => {
  instance.exports.main();
  clockAt = performance.now();
  if (wakeBeforeClock !== null) wakeDue = clockAt + wakeBeforeClock;
  wakeBeforeClock = null;
});
