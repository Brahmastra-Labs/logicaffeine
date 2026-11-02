// The harness's view into a page, installed before any of the page's own scripts run
// (`Page.addScriptToEvaluateOnNewDocument`). It changes nothing the page's runtime can observe
// except where a capability has to be recorded instead of performed.
//
//   * A host-call trace. `WebAssembly.instantiate` is wrapped so every `env` import the module
//     calls, and every export the host calls into, is logged in order with its arguments and
//     result — the page's own host runtime is untouched.
//   * Readiness. `settled()` resolves once the module's `main` has returned (or thrown); the host
//     commits the first paint synchronously after `main`, so a promise reaction queued there runs
//     after it.
//   * A timer gate. Timers the page arms before the harness takes control of the clock are held
//     and armed when `openGate()` runs, so every timer starts counting at the same virtual instant
//     however long the load took in real time.
//   * Recorders for the three capabilities a page asks the platform for: clipboard writes,
//     notifications, downloads. They are recorded, never performed.
//   * `inFlight()`: every request the page has not finished reading, every download and every
//     dropped file still being read, and every relay message not yet through — what a driving verb
//     waits out before the page is observed.
//   * `drops`: every file drop the document received, as the browser's own file describes it.
//   * `snapshot()`: the document as JSON, plus title, URL and the focused element's path;
//     `storage()`: both storage areas.
(() => {
  const H = {
    trace: [],
    depth: 0,
    main: 'pending',
    mainError: '',
    waiters: [],
    clipboard: [],
    notifications: [],
    downloads: [],
    prerendered: [],
    exports: [],
  };
  Object.defineProperty(window, '__logosHarness', { value: H, enumerable: false });

  const show = (v) => (typeof v === 'bigint' ? v.toString() : String(v));
  const finishMain = (state, error) => {
    H.main = state;
    H.mainError = error || '';
    for (const w of H.waiters.splice(0)) w(state);
  };
  H.settled = () => new Promise((res) => (H.main === 'pending' ? H.waiters.push(res) : res(H.main)));

  const dec = new TextDecoder();
  // Set once instantiation resolves, below — read only from an import wrapper, which the module
  // can only call after that (there is no way to call an import before `main` exists to call it
  // from). Decodes a Text handle exactly as `runtime.js`'s own `readText` does, so the recorded
  // action name is the same string the page would have shown a note-taking host, without reading
  // (or duplicating logic from) `runtime.js` itself.
  let memory = null;
  const readText = (h) => {
    const d = new DataView(memory.buffer);
    const len = d.getInt32(h, true), dp = d.getInt32(h + 8, true);
    return dec.decode(new Uint8Array(memory.buffer, dp, len));
  };
  // A `Seq of Text` handle, `[len][cap][data_ptr]` over 8-byte slots each holding a Text handle.
  const readTextSeq = (h) => {
    const d = new DataView(memory.buffer);
    const n = d.getInt32(h, true), dp = d.getInt32(h + 8, true);
    const out = [];
    for (let i = 0; i < n; i++) out.push(readText(d.getInt32(dp + i * 8, true)));
    return out;
  };
  // Imports whose Text arguments are decoded into the trace the moment they are made, by argument
  // index: the journal's entry point (its name, instance key and failure; its changes are a
  // `Seq of Text`) and the relay's outbound frame.
  const TEXT_ARGS = { dom_note_turn: [0, 1, 3], dom_publish: [0, 1] };
  // The page's own request table, kept at the host boundary where the page makes its requests: one
  // entry per `dom_fetch_send`/`dom_fetch_start`, numbered as the host numbers them (from 1, in
  // order), with the headers it goes out with (the host's own `__logosRequestHeaders`, the page's
  // CSRF token included) and the timeout and abort the page later asks for — whether or not anything
  // ever reaches the network for it.
  H.requests = [];
  const sent = (method, url, headers) => (typeof window.__logosRequestHeaders === 'function' ? window.__logosRequestHeaders(method, url, headers) : headers);
  const REQUESTS = {
    dom_fetch_send: (a) => {
      const method = readText(a[0]), url = readText(a[1]);
      H.requests.push({ method, url, body: readText(a[2]), headers: sent(method, url, readTextSeq(a[3])), timeout: 0, aborted: false });
    },
    dom_fetch_start: (a) => H.requests.push({ method: 'GET', url: readText(a[0]), body: '', headers: [], timeout: 0, aborted: false }),
    dom_fetch_set_timeout: (a) => { const r = H.requests[Number(a[0]) - 1]; if (r) r.timeout = Math.max(0, Number(a[1])); },
    dom_fetch_abort: (a) => { const r = H.requests[Number(a[0]) - 1]; if (r) r.aborted = true; },
  };

  const instantiate = WebAssembly.instantiate;
  WebAssembly.instantiate = function (source, imports) {
    const wrapped = {};
    for (const [ns, table] of Object.entries(imports || {})) {
      const out = {};
      for (const [name, f] of Object.entries(table)) {
        out[name] = typeof f !== 'function' ? f : function (...args) {
          const rec = { kind: 'import', name, args: args.map(show), depth: H.depth };
          if (name in TEXT_ARGS) rec.texts = TEXT_ARGS[name].map((i) => readText(args[i]));
          if (name in REQUESTS) REQUESTS[name](args);
          // A turn the page notes joins the journal of the whole visit, which a reload does not
          // end: it is reported through the harness binding, like a submission.
          if (name === 'dom_note_turn' && typeof window.__logosHarnessReport === 'function') {
            const [action, instance, failure] = rec.texts;
            window.__logosHarnessReport(JSON.stringify({ turn: { action, instance, changes: readTextSeq(args[2]), failure } }));
          }
          H.trace.push(rec);
          const r = f.apply(this, args);
          if (r !== undefined) rec.result = show(r);
          return r;
        };
      }
      wrapped[ns] = out;
    }
    return instantiate.call(this, source, wrapped).then((res) => {
      memory = res.instance.exports.memory;
      H.exports = Object.keys(res.instance.exports);
      const exports = {};
      for (const [name, f] of Object.entries(res.instance.exports)) {
        exports[name] = typeof f !== 'function' ? f : function (...args) {
          const rec = { kind: 'export', name, args: args.map(show), depth: H.depth };
          H.trace.push(rec);
          H.depth++;
          try {
            const r = f.apply(this, args);
            if (r !== undefined) rec.result = show(r);
            if (name === 'main') finishMain('returned');
            return r;
          } catch (e) {
            rec.threw = String(e);
            if (name === 'main') finishMain('threw', String(e));
            throw e;
          } finally {
            H.depth--;
          }
        };
      }
      return { module: res.module, instance: { exports } };
    }, (e) => {
      finishMain('failed', String(e));
      throw e;
    });
  };

  const realSet = { timeout: window.setTimeout.bind(window), interval: window.setInterval.bind(window) };
  const realClear = { timeout: window.clearTimeout.bind(window), interval: window.clearInterval.bind(window) };
  const held = new Map();
  const armed = new Map();
  let gateOpen = false;
  let nextHeld = -1;
  const hold = (kind) => function (fn, ms, ...args) {
    if (gateOpen) return realSet[kind](fn, ms, ...args);
    const id = nextHeld--;
    held.set(id, [kind, fn, ms, args]);
    return id;
  };
  const release = (kind) => function (id) {
    if (held.delete(id)) return;
    if (armed.has(id)) { const real = armed.get(id); armed.delete(id); return realClear[kind](real); }
    return realClear[kind](id);
  };
  window.setTimeout = hold('timeout');
  window.setInterval = hold('interval');
  window.clearTimeout = release('timeout');
  window.clearInterval = release('interval');
  H.openGate = () => {
    gateOpen = true;
    for (const [id, [kind, fn, ms, args]] of held) armed.set(id, realSet[kind](fn, ms, ...args));
    held.clear();
  };

  if (window.Clipboard) {
    Clipboard.prototype.writeText = function (t) { H.clipboard.push(String(t)); return Promise.resolve(); };
  }
  const Notice = function Notification(title, options) {
    H.notifications.push({ title: String(title), body: (options && options.body) || '' });
  };
  Notice.permission = 'granted';
  Notice.requestPermission = () => Promise.resolve('granted');
  Object.defineProperty(window, 'Notification', { value: Notice, writable: true, configurable: true });
  const blobs = new Map();
  let unreadDownloads = 0;
  const createObjectURL = URL.createObjectURL;
  URL.createObjectURL = function (obj) {
    const url = createObjectURL.call(URL, obj);
    if (obj instanceof Blob) blobs.set(url, obj);
    return url;
  };
  const anchorClick = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () {
    if (!this.hasAttribute('download')) return anchorClick.call(this);
    const entry = { name: this.download, content: null };
    H.downloads.push(entry);
    const blob = blobs.get(this.href);
    if (blob) {
      unreadDownloads++;
      blob.text().then((t) => { entry.content = t; }).finally(() => { unreadDownloads--; });
    }
  };

  // Every `fetch` the page makes, and whether the page has read its answer. An entry turns
  // `done` in the reaction to the body read; the page's own continuation (the completion turn)
  // runs in the same microtask checkpoint, so any task that observes `done` runs after that turn.
  H.fetches = [];
  const fetchWaiters = [];
  const fetchSettled = (url) => !H.fetches.some((f) => f.url === url && f.state === 'pending');
  const finishFetch = (entry, state) => {
    entry.state = state;
    for (const w of fetchWaiters.splice(0)) if (!w.check()) fetchWaiters.push(w);
  };
  H.whenFetched = (url) => new Promise((res) => {
    const w = { check: () => (fetchSettled(url) ? (res(true), true) : false) };
    if (!w.check()) fetchWaiters.push(w);
  });
  // Every file content the page is still reading (`arrayBuffer()` on a dropped file). The count
  // falls in the reaction to the read, ahead of the page's own continuation in the same microtask
  // checkpoint, so any task that sees it at zero runs after the turn the content was read for.
  let unreadFiles = 0;
  const readContent = Blob.prototype.arrayBuffer;
  Blob.prototype.arrayBuffer = function () {
    unreadFiles++;
    return readContent.call(this).finally(() => { unreadFiles--; });
  };
  // Every drop the document received, as the browser's own file describes it — `{ name, size, type }`,
  // `null` for a drop that carried no file — taken at the window in the capture phase, before any
  // listener of the page's.
  H.drops = [];
  window.addEventListener('drop', (e) => {
    const f = e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files[0];
    H.drops.push(f ? { name: f.name, size: f.size, type: f.type } : null);
  }, true);
  // Every connection the document opens to the page's relay (`/_relay`), named to the harness's
  // relay by a `harness` query parameter, with how many messages the page has sent on it and how
  // many it has handled. The count of handled messages rises in a listener the recorder adds when
  // the socket is made, ahead of the page's own, in the task that delivers the message, so any
  // task that reads it runs after the page's handler.
  const relaySockets = [];
  const documentTag = Math.random().toString(36).slice(2);
  const RealWebSocket = window.WebSocket;
  window.WebSocket = class extends RealWebSocket {
    constructor(url, protocols) {
      const u = new URL(url, location.href);
      const relayed = u.pathname === '/_relay';
      const token = `${documentTag}-${relaySockets.length + 1}`;
      if (relayed) u.searchParams.set('harness', token);
      super(u.href, protocols);
      if (!relayed) return;
      const entry = { token, sent: 0, handled: 0, socket: this };
      relaySockets.push(entry);
      this.__logosRelayEntry = entry;
      this.addEventListener('message', () => { entry.handled++; });
    }
    send(data) {
      super.send(data);
      if (this.__logosRelayEntry) this.__logosRelayEntry.sent++;
    }
  };
  // What the page still has in flight: every request whose body it has not read, as
  // `[method, url]`; how many offered downloads and dropped files are still being read; and each
  // relay connection not yet closed, `{ token, connecting, sent, handled }`.
  H.inFlight = () => ({
    fetches: H.fetches.filter((f) => f.state === 'pending').map((f) => [f.method, f.url]),
    reads: unreadDownloads + unreadFiles,
    relay: relaySockets
      .filter((r) => r.socket.readyState <= 1)
      .map((r) => ({ token: r.token, connecting: r.socket.readyState === 0, sent: r.sent, handled: r.handled })),
  });
  const realFetch = window.fetch.bind(window);
  window.fetch = function (input, init) {
    const url = new URL(typeof input === 'string' ? input : input.url, location.href).href;
    const entry = { method: ((init && init.method) || 'GET').toUpperCase(), url, state: 'pending' };
    H.fetches.push(entry);
    return realFetch(input, init).then((r) => {
      for (const m of ['text', 'json', 'arrayBuffer', 'blob']) {
        const read = r[m].bind(r);
        r[m] = () => read().then((v) => { finishFetch(entry, 'done'); return v; }, (e) => { finishFetch(entry, 'failed'); throw e; });
      }
      return r;
    }, (e) => { finishFetch(entry, 'failed'); throw e; });
  };

  // Every move of the location within this document, as the page made it: `push <path>` for
  // `history.pushState`, `replace <path>` for `replaceState` — the navigation log `semantics::dom`
  // keeps for the same document.
  H.routeOps = [];
  const pathOfUrl = (u) => { const url = new URL(u, location.href); return url.pathname + url.search + url.hash; };
  for (const [name, kind] of [['pushState', 'push'], ['replaceState', 'replace']]) {
    const real = History.prototype[name];
    History.prototype[name] = function (state, title, url) {
      const r = real.apply(this, arguments);
      if (url !== undefined && url !== null) H.routeOps.push(`${kind} ${pathOfUrl(url)}`);
      return r;
    };
  }

  // Every form submission, and whether anything stopped the browser's own action for it. A
  // window listener in the bubble phase runs after every listener on the form itself. Reported
  // through the harness binding, so the record outlives a document the submission navigated away.
  window.addEventListener('submit', (e) => {
    const report = window.__logosHarnessReport;
    if (typeof report === 'function') {
      report(JSON.stringify({ submit: { form: pathOf(e.target), prevented: e.defaultPrevented } }));
    }
  });

  // This document leaving. `navigate` fires inside the task that asks the browser for another
  // document — a link followed, a form's submission begun, a `location` assigned — before the
  // browser is asked. A microtask queued there runs once the JavaScript stack is empty: after the
  // handler that assigned the location has returned, its remaining work and its own reports done;
  // for a trusted click's link, after every click listener, which ran before the link's default
  // action. The browser hands binding calls on at once and in order, so when this report reaches
  // the harness, everything the document reported before it has too — and the harness holds the
  // next document back until then, so nothing a leaving document reported is lost with it. A move
  // within this document (`pushState`, a fragment) replaces nothing and is not reported.
  if (window.navigation) {
    navigation.addEventListener('navigate', (e) => {
      if (e.destination.sameDocument) return;
      const url = e.destination.url;
      queueMicrotask(() => {
        const report = window.__logosHarnessReport;
        if (typeof report === 'function') report(JSON.stringify({ leaving: url }));
      });
    });
  }

  const pathOf = (el) => {
    if (!el || el.nodeType !== 1 || el === document.body || el === document.documentElement) return null;
    const parts = [];
    for (let n = el; n && n.nodeType === 1; n = n.parentElement) {
      let i = 1;
      for (let s = n.previousElementSibling; s; s = s.previousElementSibling) if (s.localName === n.localName) i++;
      parts.unshift(`${n.localName}[${i}]`);
    }
    return '/' + parts.join('/');
  };
  H.pathOf = pathOf;

  // The DOM half of the driving verbs. Each names what it could not do rather than guessing.
  // The `<input>` types a visitor types text into — `semantics::dom`'s own list.
  const TEXT_ENTRY_TYPES = ['text', 'search', 'url', 'tel', 'email', 'password', 'number'];
  const find = (sel) => {
    const el = document.querySelector(sel);
    if (!el) throw new Error(`nothing on the page matches ${JSON.stringify(sel)}`);
    return el;
  };
  const describe = (el) => (el.nodeType === 1 ? pathOf(el) || el.localName : el.nodeName);
  H.drive = {
    // The point a visitor would click to click THIS element: one where the element itself is the
    // topmost thing drawn — its centre if that is so, else the first such point of an inset grid
    // over it, so a click on a dialog lands on the dialog and not on the button drawn at its
    // centre, and a click on a backdrop lands on the backdrop and not on the dialog it holds. An
    // element its own content covers everywhere (a button filled by its label) is clicked at its
    // centre, inside it; one something else covers is refused, naming what covers it.
    point(sel) {
      const el = find(sel);
      el.scrollIntoView({ block: 'center', inline: 'center' });
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) throw new Error(`${JSON.stringify(sel)} is not visible — it has no size`);
      const centre = { x: r.left + r.width / 2, y: r.top + r.height / 2 };
      const hitAt = (p) => document.elementFromPoint(p.x, p.y);
      if (hitAt(centre) === el) return centre;
      const steps = [0.5, 0.25, 0.75, 0.1, 0.9];
      for (const fy of steps) {
        for (const fx of steps) {
          const p = { x: r.left + r.width * fx, y: r.top + r.height * fy };
          if (hitAt(p) === el) return p;
        }
      }
      const hit = hitAt(centre);
      if (hit && el.contains(hit)) return centre;
      throw new Error(`${JSON.stringify(sel)} is covered at its centre by ${hit ? describe(hit) : 'nothing'}`);
    },
    // Focus a control and select what it holds, so the text typed next replaces it.
    focusForTyping(sel) {
      const el = find(sel);
      const textEntry = el.localName === 'textarea' || (el.localName === 'input' && TEXT_ENTRY_TYPES.includes(el.type));
      if (!textEntry || !el.matches(':read-write')) {
        throw new Error(`Typing into ${JSON.stringify(sel)} enters nothing: it takes no typed text — it is read-only, disabled, or not a text field — so the browser raises no \`input\` or \`change\`.`);
      }
      el.focus();
      if (document.activeElement !== el) throw new Error(`${JSON.stringify(sel)} cannot take focus, so it cannot be typed into`);
      if (typeof el.select === 'function') el.select();
    },
    clear(sel) {
      const el = find(sel);
      el.value = '';
      el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'deleteContentBackward' }));
    },
    fire(sel, type) { find(sel).dispatchEvent(new Event(type, { bubbles: true })); },
    // The pointer arriving over the element: the `mouseover` it raises there, bubbling.
    hover(sel) { find(sel).dispatchEvent(new MouseEvent('mouseover', { bubbles: true, cancelable: true, composed: true, view: window })); },
    focus(sel) { find(sel).focus(); },
    blur(sel) { find(sel).blur(); },
    choose(sel, value) {
      const el = find(sel);
      const opts = el instanceof HTMLSelectElement ? Array.from(el.options) : [];
      const opt = opts.find((o) => o.value === value) || opts.find((o) => o.text === value);
      if (!opt || el.matches(':disabled')) {
        throw new Error(`Choosing ${JSON.stringify(value)} in ${JSON.stringify(sel)} selects nothing: it is not a \`<select>\` a visitor can reach that offers that option, so the browser raises no \`input\` or \`change\`.`);
      }
      el.focus();
      el.value = opt.value;
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    },
    submit(sel) {
      const el = find(sel);
      const form = el instanceof HTMLFormElement ? el : el.form;
      if (!form) throw new Error(`${JSON.stringify(sel)} is not a form and belongs to none`);
      const submitter = (el instanceof HTMLButtonElement || el instanceof HTMLInputElement) && el.type === 'submit' ? el : undefined;
      form.requestSubmit(submitter);
    },
    // The element — or, for the page itself, the document — scrolled to its end, and the `scroll`
    // the browser raises for that, where the browser raises it: on the element that scrolled, or
    // on the document when the page did.
    scrollToBottom(sel) {
      const el = find(sel);
      if (el === document.body || el === document.documentElement) {
        window.scrollTo(0, document.documentElement.scrollHeight);
        document.dispatchEvent(new Event('scroll', { bubbles: true }));
        return;
      }
      el.scrollTop = el.scrollHeight;
      el.dispatchEvent(new Event('scroll'));
    },
  };

  // The markup a pre-rendered document arrived with, captured when parsing finishes — before
  // any deferred or module script runs — so a test can ask which of those nodes survived the
  // module's first paint.
  document.addEventListener('readystatechange', () => {
    if (document.readyState !== 'interactive' || !document.body) return;
    const walk = (n) => {
      for (const c of n.children) { H.prerendered.push({ node: c, path: pathOf(c) }); walk(c); }
    };
    walk(document.body);
  });
  H.prerenderedFate = () => ({
    total: H.prerendered.length,
    replaced: H.prerendered.filter((p) => !p.node.isConnected).map((p) => p.path),
  });

  const defaultValue = (n) => {
    if (n instanceof HTMLSelectElement) {
      const opt = Array.from(n.options).find((o) => o.defaultSelected) || n.options[0];
      return opt ? opt.value : '';
    }
    return n.defaultValue;
  };
  const shape = (n) => {
    if (n.nodeType === 3) return { text: n.data };
    if (n.nodeType !== 1) return null;
    const attrs = Array.from(n.attributes, (a) => [a.name, a.value]).sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
    const out = { tag: n.localName, attrs, children: [] };
    // A checkbox's or radio's `value` is not something a visitor moves — its state is `checked` —
    // and with no `value` attribute the property reads its default, `on`.
    const toggles = n instanceof HTMLInputElement && (n.type === 'checkbox' || n.type === 'radio');
    if (!toggles && (n instanceof HTMLInputElement || n instanceof HTMLTextAreaElement || n instanceof HTMLSelectElement)) {
      const dirty = n.value !== defaultValue(n);
      if (dirty && n.value !== '') out.value = n.value;
    }
    if (n instanceof HTMLInputElement && n.checked && !n.defaultChecked) out.checked = true;
    for (const c of n.childNodes) {
      const s = shape(c);
      if (s) out.children.push(s);
    }
    return out;
  };
  // Both storage areas, `[area, key, value]`, `local` then `session`, each in key order.
  H.storage = () => {
    const out = [];
    for (const [area, store] of [['local', window.localStorage], ['session', window.sessionStorage]]) {
      const keys = [];
      for (let i = 0; i < store.length; i++) keys.push(store.key(i));
      keys.sort();
      for (const k of keys) out.push([area, k, store.getItem(k)]);
    }
    return out;
  };
  H.snapshot = () => ({
    url: location.href,
    title: document.title,
    focus: pathOf(document.activeElement),
    document: shape(document.documentElement),
  });
})();
