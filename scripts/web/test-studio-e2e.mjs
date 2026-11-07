// The Studio in a real browser — the layer its resizers and drawer live in.
//
// The layout rules (bounds, which columns a drag may move, how a narrow window squeezes) are
// pinned by the Rust unit tests in apps/logicaffeine_web/src/ui/pages/studio_layout.rs. What
// only a browser can show lives here: a pointer dragging a bar, the columns the page actually
// paints, the window changing size, an iframe under the pointer, a phone-sized screen.
//
// Every scenario runs in a fresh browser context, fails on any uncaught page error or console
// error, and leaves a screenshot of the failing moment. Scenarios find the page's parts by
// what the page renders (the panel row, its bars, the drawer), never by internal hooks, so the
// same scenario proves a bug on the old page (RED) and its absence on the fixed one (GREEN).
//
// Usage: node scripts/web/test-studio-e2e.mjs (--public <built-public-dir> | --url <running-site>)
//          (--chrome <path> | --firefox <path>) [--junit <report.xml>] [--only name,name]
//          [--shots <dir>] [--desktop-baseline <dir>] [--list]
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import os from 'node:os';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

// ---------------------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------------------

function parseArgs(argv) {
  const out = { only: null, junit: null, shots: '/tmp/studio-e2e-shots', list: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--public') out.public = argv[++i];
    else if (a === '--url') out.url = argv[++i].replace(/\/$/, '');
    else if (a === '--chrome') out.chrome = argv[++i];
    else if (a === '--firefox') out.firefox = argv[++i];
    else if (a === '--junit') out.junit = argv[++i];
    else if (a === '--only') out.only = argv[++i].split(',').map((s) => s.trim()).filter(Boolean);
    else if (a === '--shots') out.shots = argv[++i];
    else if (a === '--desktop-baseline') out.desktopBaseline = argv[++i];
    else if (a === '--list') out.list = true;
    else throw new Error(`unknown argument ${a}`);
  }
  return out;
}

// ---------------------------------------------------------------------------------------
// A built site, served the way production serves it (SPA fallback)
// ---------------------------------------------------------------------------------------

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.mjs': 'text/javascript',
  '.wasm': 'application/wasm', '.css': 'text/css', '.json': 'application/json',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.jpg': 'image/jpeg', '.ico': 'image/x-icon',
  '.woff2': 'font/woff2', '.woff': 'font/woff', '.txt': 'text/plain; charset=utf-8',
  '.xml': 'application/xml', '.pdf': 'application/pdf', '.webp': 'image/webp',
};

function serve(dir) {
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://x');
    let file = path.join(dir, decodeURIComponent(url.pathname));
    if (!file.startsWith(dir)) { res.writeHead(403).end(); return; }
    if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, 'index.html');
    if (!fs.existsSync(file)) file = path.join(dir, 'index.html');
    res.writeHead(200, { 'content-type': MIME[path.extname(file)] || 'application/octet-stream' });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => {
    resolve({ url: `http://127.0.0.1:${server.address().port}`, close: () => server.close() });
  }));
}

// ---------------------------------------------------------------------------------------
// The layout's rules, as the page must honour them (mirrors studio_layout.rs)
// ---------------------------------------------------------------------------------------

const MIN = { sidebar: 140, left: 200, center: 200, right: 200 };
const BAR = 6;
const DRAWER_MIN = 44;
const PANELS_MIN_HEIGHT = 100;
const STATUS_BAR_HEIGHT = 25;
const STATUS_BAR_LINE = 2;
const DESKTOP = { width: 1920, height: 1080 };
const PHONE = { width: 390, height: 844, isMobile: true, hasTouch: true };
// The Studio's one surface — the editor's dark — which every column body paints, and how far every
// column holds its content from its edges.
const SURFACE = 'rgb(15,20,25)';
const INSET = 20;
// The file tree's rhythm: its first line stands this far from the sidebar's edge, every level's
// line the same distance further in, and every branch runs the same distance, from its parent's
// line to its own. A row's first glyph sits TREE_GAP past the end of its branch.
const TREE_STEP = 18;
const TREE_GAP = 4;
// The zooms the tree's lines must keep one weight at: the stick's 90% and the fractional ratios
// where a line left to itself lands on a different fraction of a device pixel at every level. They
// are set for real, as a screen's density or a browser zoom sets them: the browser then lays the
// page out in device pixels (Chrome in device pixels outright, Firefox in app units a whole number
// of which make one device pixel). A scale set through the protocol lays the page out in CSS
// pixels and scales only its painting, and Chrome does not tell the page that its ratio changed.
// Headless Firefox cannot open a window at 90% or 95% (its software compositor fails to map the
// frame), so it is held at 80%, where a device pixel is likewise more than one CSS pixel and not a
// whole number of them.
const REAL_SCALES = { chrome: [0.9, 1.25, 1.5, 2], firefox: [0.8, 1.25, 1.5, 2] };

const FILES = {
  threePanels: 'examples/hardware/handshake.hw',
  code: 'examples/code/algorithms/fibonacci.logos',
  web: 'examples/code/web/counter.logos',
  recursion: 'examples/code/functions/recursion.logos',
  math: 'examples/math/natural-numbers.logos',
  logic: 'examples/logic/prover-demo.logic',
  sentences: 'examples/logic/quantifiers.logic',
  longProof: 'examples/logic/simon.logic',
};

// One example per kind of file, each opening the panels that kind shows.
const EVERY_KIND = [
  ['Logic theorem', FILES.logic],
  ['Logic sentences', FILES.sentences],
  ['Code', FILES.code],
  ['Math', FILES.math],
  ['Hardware', FILES.threePanels],
  ['Web', FILES.web],
];

const FIBONACCI_FIRST_LINE = 'The first 10 Fibonacci numbers:';

// ---------------------------------------------------------------------------------------
// Assertions and page helpers
// ---------------------------------------------------------------------------------------

class Failure extends Error {}

// A debug build carries dx's hot-reload client, which dials a devserver socket a plain static
// server cannot answer. That error belongs to the build's tooling, never to the page.
// The dev server's own client, injected into dev builds only: its socket, and the rebuild notices
// it cannot parse when the server and client versions disagree.
// Chrome logs the refused socket to the console; Firefox raises it as a page error, in its own words.
const DEV_CLIENT_NOISE = /WebSocket connection to '[^']*\/_dioxus\?|Error parsing devserver message|can’t establish a connection to the server at ws:\/\/[^ ]*\/_dioxus\?/;
// The dev server swaps each build into place; a page that loads mid-swap is sent the app's loader
// script with no content type, the browser refuses it, and the app never starts. Chrome and
// Firefox word the refusal differently, and Firefox raises it as a page error where Chrome logs
// it to the console. It is the server's race, never the app's error: `t.load` waits the swap out
// and loads again.
const DEV_SERVER_SWAP = /MIME type of ""|disallowed MIME type \(“”\)/;

// Resolve once the dev server serves a whole build: its loader script as JavaScript and its
// module as wasm.
async function wholeBuild(origin, ms = 600000) {
  const until = Date.now() + ms;
  for (;;) {
    const type = async (asset) => (await fetch(`${origin}/wasm/${asset}`, { method: 'HEAD' }).catch(() => null))?.headers.get('content-type') || '';
    if (/javascript/.test(await type('logicaffeine-web.js')) && /wasm/.test(await type('logicaffeine-web_bg.wasm'))) return;
    if (Date.now() > until) throw new Failure(`the dev server served no whole build within ${ms / 1000}s`);
    await new Promise((r) => setTimeout(r, 500));
  }
}
// A fetch a page abandons when the next navigation cuts it off. Chrome words it as an AbortError,
// Firefox as a NetworkError; the page that raised it is already gone.
const NAVIGATION_ABORT = /AbortError: The operation was aborted|TypeError: NetworkError when attempting to fetch resource/;
function assert(cond, message) { if (!cond) throw new Failure(message); }
const near = (a, b, tol) => Math.abs(a - b) <= tol;
const px = (n) => `${n.toFixed(1)}px`;

function helpers(page, base, engine, watch = () => {}) {
  // WebDriver BiDi (Firefox) refuses a pointer move outside the viewport, which CDP (Chrome)
  // allows the way a captured pointer outside the window does; under BiDi an overshooting drag
  // stops at the window's edge instead, still past every limit it probes.
  const reach = (x, y) => {
    if (engine !== 'firefox') return [x, y];
    const { width, height } = page.viewport();
    return [Math.min(Math.max(x, 0), width - 1), Math.min(Math.max(y, 0), height - 1)];
  };
  const t = {
    page,
    base,
    engine,

    async open(file, viewport = DESKTOP) {
      await t.openFrom(base, file, viewport);
    },

    // Swap in a fresh page in the same context and close the old one. Firefox keeps the memory of
    // every load a page has made (about 280 MB per load of a debug build), so a sweep that loaded
    // dozens of examples into one page ran out of memory. A page per example keeps it flat.
    async fresh() {
      const next = await page.browserContext().newPage();
      watch(next);
      await page.close();
      page = next;
      t.page = next;
    },

    // The same site reached through a non-loopback address: plain http there is not a secure
    // context, so the browser offers no OPFS — the path a LAN display takes. Null when the
    // site is not on this machine's loopback or the machine has no LAN address.
    insecureOrigin() {
      const url = new URL(base);
      if (url.protocol !== 'http:' || !['127.0.0.1', 'localhost'].includes(url.hostname)) return null;
      const lan = Object.values(os.networkInterfaces()).flat().find((i) => i && i.family === 'IPv4' && !i.internal);
      if (!lan) return null;
      url.hostname = lan.address;
      return url.origin;
    },

    // Open a file, failing after `ms` instead of the cold-start allowance — for sweeps, where
    // one example that never loads must be reported and the sweep carried on.
    async openWithin(file, ms) {
      await Promise.race([
        t.open(file),
        new Promise((_, reject) => setTimeout(() => reject(new Failure(`the page did not finish loading within ${ms / 1000}s`)), ms)),
      ]);
    },

    // Load a Studio URL until the app has taken the page over: the live app in #app holds the
    // Studio, and the shell's watchdog has removed #main — the prerendered copy of a deploy
    // build, which repeats every element the app renders and would pass for it (an empty div in
    // a dev build). A load the dev server's build swap refused is waited out and loaded again;
    // any other failure to start fails at once.
    async load(url) {
      for (let attempt = 1; ; attempt++) {
        let refused;
        const swapped = new Promise((resolve) => { refused = (text) => { if (DEV_SERVER_SWAP.test(text)) resolve(text); }; });
        const onConsole = (m) => refused(m.text());
        const onPageError = (e) => refused(String(e.message || e));
        page.on('console', onConsole);
        page.on('pageerror', onPageError);
        try {
          await page.goto(url, { waitUntil: 'load', timeout: 300000 });
          const outcome = await Promise.race([
            page
              .waitForFunction(() => document.querySelector('#app .studio-main > section') && !document.getElementById('main'), { timeout: 300000 })
              .then(() => null),
            swapped,
          ]);
          if (outcome === null) return;
          if (attempt === 3) throw new Failure(`the dev server refused the app's loader script ${attempt} times: ${outcome.slice(0, 160)}`);
        } finally {
          page.off('console', onConsole);
          page.off('pageerror', onPageError);
        }
        await wholeBuild(new URL(url).origin);
      }
    },

    // A null viewport keeps the window the browser was launched with.
    async openFrom(origin, file, viewport = DESKTOP) {
      if (viewport) await page.setViewport(viewport);
      await t.load(`${origin}/studio?file=${file}`);
      // The file system seeds and the file loads asynchronously: wait until the first panel's
      // editor holds the file's text.
      await page.waitForFunction(() => {
        const editor = document.querySelector('.studio-main > section textarea');
        return editor && editor.value.trim().length > 0;
      }, { timeout: 120000 });
      await t.frames(6);
    },

    // Two animation frames: long enough for any per-frame repaint to land.
    async frames(n = 2) {
      for (let i = 0; i < n; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
    },

    // The columns as painted: each one's left edge and width, and the bars between them.
    async layout() {
      return page.evaluate(() => {
        const rect = (el) => { if (!el) return null; const r = el.getBoundingClientRect(); return { x: r.left, w: r.width, h: r.height, y: r.top }; };
        const shown = (el) => el && getComputedStyle(el).display !== 'none';
        const panels = [...document.querySelectorAll('.studio-main > section')];
        const aside = document.querySelector('.studio-main > aside');
        const sidebar = document.querySelector('.studio-sidebar');
        return {
          viewport: document.documentElement.clientWidth,
          scrollWidth: document.documentElement.scrollWidth,
          sidebar: shown(sidebar) ? rect(sidebar) : null,
          left: rect(panels[0]),
          center: rect(panels[1]),
          right: shown(aside) ? rect(aside) : null,
          bars: [...document.querySelectorAll('.panel-resizer')].filter(shown).map(rect),
        };
      });
    },

    // The share of a region's painted pixels that are light (luma above 180 of 255).
    async lightShare(clip) {
      const png = await page.screenshot({ clip, encoding: 'base64' });
      return page.evaluate(async (src) => {
        const img = new Image();
        img.src = src;
        await img.decode();
        const canvas = document.createElement('canvas');
        canvas.width = img.width;
        canvas.height = img.height;
        const g = canvas.getContext('2d');
        g.drawImage(img, 0, 0);
        const d = g.getImageData(0, 0, canvas.width, canvas.height).data;
        let light = 0;
        for (let i = 0; i < d.length; i += 4) if (0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2] > 180) light++;
        return light / (d.length / 4);
      }, `data:image/png;base64,${png}`);
    },

    // Drag from the middle of `el` by (dx, dy), in `steps` pointer moves.
    async dragElement(el, dx, dy, steps = 12) {
      const box = await el.boundingBox();
      assert(box, 'the drag handle is not on screen');
      const x0 = box.x + box.width / 2, y0 = box.y + box.height / 2;
      await page.mouse.move(x0, y0);
      await page.mouse.down();
      for (let i = 1; i <= steps; i++) await page.mouse.move(...reach(x0 + (dx * i) / steps, y0 + (dy * i) / steps));
      await t.frames();
      const during = await t.layout();
      await page.mouse.up();
      await t.frames(3);
      return { start: { x: x0, y: y0 }, during };
    },

    // The bars in page order: sidebar|left, left|center, center|right (those shown).
    async bars() {
      const all = await page.$$('.panel-resizer');
      const shown = [];
      for (const b of all) if (await b.evaluate((el) => getComputedStyle(el).display !== 'none')) shown.push(b);
      return shown;
    },

    async openDebugger() {
      const debug = await page.waitForSelector('xpath/.//button[contains(., "Debug")]', { timeout: 30000 });
      await debug.click();
      await page.waitForSelector('.dbg-drawer', { timeout: 30000 });
      await t.frames(3);
    },

    async grip() {
      const grip = await page.$('.dbg-grip');
      assert(grip, 'the debugger drawer has no drag grip');
      return grip;
    },

    // A debugger toolbar button, by the start of its title.
    async dbgButton(title) {
      const b = await page.$(`.dbg-controls button[title^="${title}"]`);
      assert(b, `the debugger has no "${title}" button`);
      return b;
    },

    async dbgTab(name) {
      const tab = await page.waitForSelector(`xpath/.//button[contains(@class, "dbg-tab") and normalize-space(.)="${name}"]`, { timeout: 10000 });
      await tab.click();
      await t.frames(3);
    },

    async stepInto(times = 1) {
      for (let i = 0; i < times; i++) {
        await (await t.dbgButton('Step into')).click();
        await t.frames(2);
      }
    },

    async runToEnd() {
      await (await t.dbgButton('Continue')).click();
      await page.waitForFunction(() => /done/i.test(document.querySelector('.dbg-status')?.innerText || ''), { timeout: 30000 });
      await t.frames(3);
    },

    // The names in the Variables tab, top to bottom.
    async variableNames() {
      return page.$$eval('.dbg-vars .dbg-var-name', (els) => els.map((e) => e.innerText.trim()));
    },

    // The height the debugger's tabs have, or null with the drawer closed.
    async dbgBodyHeight() {
      return page.evaluate(() => document.querySelector('.dbg-body')?.getBoundingClientRect().height ?? null);
    },

    // The Code-mode Output column as it reads: its program lines and the note shown instead.
    async outputColumn() {
      return page.evaluate(() => {
        const out = document.querySelector('.interpreter-output');
        if (!out) return null;
        return {
          lines: [...out.querySelectorAll('.interpreter-line')].map((l) => l.innerText),
          note: out.querySelector('.interpreter-empty')?.innerText.trim() || '',
          size: parseFloat(getComputedStyle(out.querySelector('.interpreter-line') || out).fontSize),
        };
      });
    },

    // The status bar's items, by name: "position", "problems", "language", … (their text).
    async statusBar() {
      return page.evaluate(() => {
        const bar = document.querySelector('.studio-statusbar');
        if (!bar) return null;
        const items = {};
        for (const item of bar.querySelectorAll('[data-status]')) items[item.dataset.status] = item.innerText.trim();
        return items;
      });
    },

    async drawer() {
      return page.evaluate(() => {
        const d = document.querySelector('.dbg-drawer');
        const c = document.querySelector('.studio-content');
        return d && c ? { drawer: d.getBoundingClientRect().height, content: c.getBoundingClientRect().height } : null;
      });
    },
  };
  return t;
}

// Every column except the named ones must be exactly where it was.
function assertOthersStill(before, after, moved, context) {
  for (const col of ['sidebar', 'left', 'center', 'right']) {
    if (moved.includes(col) || !before[col] || !after[col]) continue;
    assert(near(before[col].w, after[col].w, 0.5), `${context}: the ${col} column changed width ${px(before[col].w)} → ${px(after[col].w)}`);
  }
}

function assertFits(l, context) {
  assert(l.scrollWidth <= l.viewport, `${context}: the page scrolls sideways (${l.scrollWidth}px in ${l.viewport}px)`);
  const cols = ['sidebar', 'left', 'center', 'right'].filter((c) => l[c]);
  const used = cols.reduce((s, c) => s + l[c].w, 0) + l.bars.reduce((s, b) => s + b.w, 0);
  assert(used <= l.viewport + 1, `${context}: columns and bars take ${px(used)} of ${l.viewport}px`);
  for (const c of cols) assert(l[c].w >= MIN[c] - 1, `${context}: the ${c} column is ${px(l[c].w)}, under its ${MIN[c]}px minimum`);
}

// ---------------------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------------------

const scenarios = [];
// opts.allowErrors: page errors the scenario expects. opts.engines: the browsers able to show
// what it checks — elsewhere it is reported as skipped, never as passed.
function scenario(name, fn, opts = {}) { scenarios.push({ name, fn, opts }); }

scenario('each_bar_moves_only_its_two_neighbours', async (t) => {
  await t.open(FILES.threePanels);
  const pairs = [['sidebar', 'left'], ['left', 'center'], ['center', 'right']];
  const bars = await t.bars();
  assert(bars.length === 3, `expected three column bars with the file browser and three panels, found ${bars.length}`);
  const widths = (await t.layout()).bars.map((b) => b.w);
  assert(widths.every((w) => near(w, BAR, 0.5)), `the column bars are ${widths.map(px).join(', ')} wide, not ${BAR}px`);
  for (let i = 0; i < 3; i++) {
    const [a, b] = pairs[i];
    for (const dx of [80, -80]) {
      const before = await t.layout();
      await t.dragElement((await t.bars())[i], dx, 0);
      const after = await t.layout();
      const ctx = `dragging the ${a}|${b} bar by ${dx}px`;
      assertOthersStill(before, after, [a, b], ctx);
      assert(near(after[a].w - before[a].w, dx, 1.5), `${ctx}: the ${a} column changed by ${px(after[a].w - before[a].w)}`);
      assert(near(after[b].w - before[b].w, -dx, 1.5), `${ctx}: the ${b} column changed by ${px(after[b].w - before[b].w)}`);
    }
  }
});

scenario('a_bar_follows_the_pointer', async (t) => {
  await t.open(FILES.threePanels);
  for (let i = 0; i < 3; i++) {
    const { start, during } = await t.dragElement((await t.bars())[i], 60, 0);
    const bar = during.bars[i];
    assert(near(bar.x + bar.w / 2, start.x + 60, 2), `bar ${i} sits at ${px(bar.x + bar.w / 2)} mid-drag; the pointer is at ${px(start.x + 60)}`);
  }
});

scenario('a_bar_stops_at_each_neighbours_minimum', async (t) => {
  await t.open(FILES.threePanels);
  const pull = async (i, dx) => { await t.dragElement((await t.bars())[i], dx, 0, 20); return t.layout(); };
  let l = await pull(1, -2000);
  assert(near(l.left.w, MIN.left, 1), `left panel pulled to ${px(l.left.w)}, not its ${MIN.left}px minimum`);
  l = await pull(1, 3000);
  assert(near(l.center.w, MIN.center, 1), `center panel pushed to ${px(l.center.w)}, not its ${MIN.center}px minimum`);
  l = await pull(2, 3000);
  assert(near(l.right.w, MIN.right, 1), `right panel pushed to ${px(l.right.w)}, not its ${MIN.right}px minimum`);
  l = await pull(0, -2000);
  assert(near(l.sidebar.w, MIN.sidebar, 1), `file browser pulled to ${px(l.sidebar.w)}, not its ${MIN.sidebar}px minimum`);
  l = await pull(0, 3000);
  assert(near(l.left.w, MIN.left, 1), `widening the file browser stopped with the left panel at ${px(l.left.w)}, not its ${MIN.left}px minimum`);
  assert(l.sidebar.w > 400, `the file browser stopped at ${px(l.sidebar.w)}; it has no fixed maximum`);
  assertFits(l, 'after every pull');
});

scenario('the_row_fits_every_desktop_width', async (t) => {
  await t.open(FILES.threePanels);
  for (const width of [769, 900, 1024, 1280, 1600, 1920, 2560]) {
    await t.page.setViewport({ ...DESKTOP, width });
    await t.frames(4);
    assertFits(await t.layout(), `at ${width}px`);
  }
});

scenario('widening_the_window_again_restores_the_dragged_layout', async (t) => {
  await t.open(FILES.threePanels);
  await t.dragElement((await t.bars())[1], 120, 0);
  const shaped = await t.layout();
  await t.page.setViewport({ ...DESKTOP, width: 1000 });
  await t.frames(4);
  assertFits(await t.layout(), 'squeezed to 1000px');
  await t.page.setViewport(DESKTOP);
  await t.frames(4);
  const back = await t.layout();
  for (const col of ['sidebar', 'left', 'center', 'right']) {
    assert(near(back[col].w, shaped[col].w, 1), `the ${col} column came back at ${px(back[col].w)}, not ${px(shaped[col].w)}`);
  }
});

scenario('a_bar_keeps_tracking_over_the_web_preview', async (t) => {
  await t.open(FILES.web);
  await t.page.waitForSelector('.studio-main iframe', { timeout: 60000 });
  const before = await t.layout();
  // Drag the left|center bar well into the live preview's iframe.
  const { during } = await t.dragElement((await t.bars())[1], 300, 0, 24);
  assert(near(during.left.w - before.left.w, 300, 2), `over the preview the left panel grew ${px(during.left.w - before.left.w)}, not 300px`);
});

scenario('the_drawer_grip_drags_freely', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  const grip = await t.grip();
  const cols = await t.layout();
  const start = await t.drawer();

  await t.dragElement(grip, 0, -250);
  let d = await t.drawer();
  assert(near(d.drawer - start.drawer, 250, 1.5), `dragging the grip up 250px grew the drawer by ${px(d.drawer - start.drawer)}`);
  assertOthersStill(cols, await t.layout(), [], 'dragging the drawer');

  await t.dragElement(await t.grip(), 0, 3000, 20);
  d = await t.drawer();
  assert(near(d.drawer, DRAWER_MIN, 2), `dragged all the way down, the drawer is ${px(d.drawer)}, not its ${DRAWER_MIN}px control bar`);

  await t.dragElement(await t.grip(), 0, -3000, 20);
  d = await t.drawer();
  assert(near(d.content, PANELS_MIN_HEIGHT, 2), `dragged all the way up, the panels keep ${px(d.content)}, not ${PANELS_MIN_HEIGHT}px`);
});

scenario('the_drawer_keeps_its_height_when_reopened', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  await t.dragElement(await t.grip(), 0, -180);
  const shaped = (await t.drawer()).drawer;
  await (await t.page.$('.dbg-close')).click();
  await t.page.waitForFunction(() => !document.querySelector('.dbg-drawer'));
  await t.openDebugger();
  const again = (await t.drawer()).drawer;
  assert(near(again, shaped, 1), `the drawer reopened at ${px(again)}, not the ${px(shaped)} it was left at`);
});

scenario('the_debugger_has_no_question_mark_toggle', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  await t.stepInto(2);
  const questionButtons = await t.page.$$eval('.dbg-drawer button', (bs) => bs.filter((b) => b.innerText.trim() === '?').length);
  assert(questionButtons === 0, `the debugger still shows ${questionButtons} "?" button(s)`);
});

// While the debugger runs, the Output column shows what the program has printed so far — the
// drawer keeps no output pane of its own. Opening the debugger shows the Output view, and closing
// it gives the column back to the last Run.
scenario('the_debugger_output_is_what_the_program_printed', async (t) => {
  await t.open(FILES.code);
  // The column as the last Run left it, once the run that opening the example starts settles.
  let lastRun = await t.outputColumn();
  for (let i = 0; i < 40; i++) {
    await new Promise((r) => setTimeout(r, 150));
    const again = await t.outputColumn();
    const same = JSON.stringify(again) === JSON.stringify(lastRun);
    lastRun = again;
    if (same && (again.lines.length > 0 || again.note)) break;
  }
  const rust = await t.page.waitForSelector('xpath/.//button[contains(@class, "output-mode-btn") and normalize-space(.)="Rust"]', { timeout: 10000 });
  await rust.click();
  await t.frames(2);
  await t.openDebugger();
  assert(!(await t.page.$('.dbg-out')) && !(await t.page.$('.dbg-out-grip')), 'the debugger drawer still has an output pane of its own');
  const active = await t.page.$eval('.output-mode-btn.active', (b) => b.innerText.trim()).catch(() => null);
  assert(active === 'Output', `opening the debugger left the Output column on ${JSON.stringify(active)}`);

  const before = await t.outputColumn();
  assert(before && before.lines.length === 0, `before a step the Output column shows ${JSON.stringify(before?.lines)}`);
  assert(/nothing printed yet/i.test(before.note), `before a step the Output column says ${JSON.stringify(before.note)}`);

  await t.runToEnd();
  const after = await t.outputColumn();
  assert(after.lines[0] === FIBONACCI_FIRST_LINE, `the Output column shows ${JSON.stringify(after.lines[0])}, not ${JSON.stringify(FIBONACCI_FIRST_LINE)}`);
  assert(after.size >= 12, `program output is set at ${after.size}px`);

  await (await t.dbgButton('Restart')).click();
  await t.frames(3);
  const rewound = await t.outputColumn();
  assert(rewound.lines.length === 0, `rewound to the entry, the Output column still shows ${rewound.lines.length} lines`);

  await (await t.page.$('.dbg-close')).click();
  await t.page.waitForFunction(() => !document.querySelector('.dbg-drawer'));
  await t.frames(2);
  const closed = await t.outputColumn();
  assert(JSON.stringify(closed.lines) === JSON.stringify(lastRun.lines) && closed.note === lastRun.note,
    `with the debugger closed the Output column shows ${JSON.stringify(closed)}, not the last Run ${JSON.stringify(lastRun)}`);
});

scenario('stepping_never_shifts_the_debugger_tabs', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  const top = () => t.page.$eval('.dbg-tabs', (e) => e.getBoundingClientRect().top);
  const first = await top();
  for (let i = 1; i <= 30; i++) {
    await t.stepInto();
    const now = await top();
    assert(near(now, first, 0.5), `step ${i} moved the tabs from ${px(first)} to ${px(now)}`);
  }
});

scenario('the_teaching_line_explains_every_step', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  for (let i = 1; i <= 30; i++) {
    await t.stepInto();
    const text = await t.page.$eval('.dbg-narr .dbg-narr-text', (e) => e.innerText.trim()).catch(() => '');
    assert(text.length > 0, `step ${i} has no teaching line`);
  }
});

scenario('auto_play_says_it_is_playing', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  await (await t.dbgButton('Auto-play')).click();
  await t.frames(3);
  const chip = await t.page.$eval('.dbg-status .dbg-state', (e) => e.innerText.trim());
  assert(/playing/i.test(chip), `while auto-playing the status reads ${JSON.stringify(chip)}`);
  await (await t.dbgButton('Pause')).click();
});

scenario('variables_lists_the_programs_named_variables', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  await t.runToEnd();
  const names = await t.variableNames();
  const temporaries = names.filter((n) => /^R\d+$/.test(n));
  assert(temporaries.length === 0, `the Variables tab lists compiler temporaries: ${temporaries.join(', ')}`);
  for (const n of ['n', 'a', 'b', 'i', 'next']) assert(names.includes(n), `the Variables tab is missing ${n}: ${names.join(', ')}`);
  await t.dbgTab('Stack');
  const slots = await t.page.$$eval('.dbg-slot-name', (els) => els.map((e) => e.innerText.trim()));
  assert(slots.some((n) => /^R\d+$/.test(n)), 'the Stack tab no longer shows the full register file');
});

scenario('a_function_frame_names_its_variables', async (t) => {
  await t.open(FILES.recursion);
  await t.openDebugger();
  let frames = 0;
  for (let i = 0; i < 60 && frames < 2; i++) {
    await t.stepInto();
    await t.dbgTab('Call Stack');
    frames = await t.page.$$eval('.dbg-frame', (els) => els.length);
  }
  assert(frames >= 2, 'never stepped into a function');
  const fn = await t.page.$eval('.dbg-frame.current .dbg-frame-fn', (e) => e.innerText.trim());
  assert(fn === 'factorial', `the call stack shows ${JSON.stringify(fn)}, not factorial`);
  await t.dbgTab('Variables');
  const names = await t.variableNames();
  assert(names.includes('n'), `inside factorial the Variables tab shows ${names.join(', ') || 'nothing'}, not the parameter n`);
});

scenario('the_timeline_keeps_the_playhead_in_view', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  await t.runToEnd();
  await t.page.evaluate(() => {
    const r = document.querySelector('.dbg-scrub-range');
    r.value = String(Math.floor(Number(r.max) * 0.6));
    r.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await t.frames(3);
  await t.dbgTab('Timeline');
  await t.frames(4);
  const seen = await t.page.evaluate(() => {
    const box = document.querySelector('.dbg-osc').getBoundingClientRect();
    const head = document.querySelector('.dbg-osc .tlph').getBoundingClientRect();
    const labels = [...document.querySelectorAll('.dbg-osc .tln')].map((l) => {
      const r = l.getBoundingClientRect();
      return { text: l.textContent, left: r.left, right: r.right };
    });
    return { head: head.left, left: box.left, right: box.right, labels };
  });
  assert(seen.head >= seen.left && seen.head <= seen.right, `the playhead is at ${px(seen.head)}, outside the timeline's view ${px(seen.left)}–${px(seen.right)}`);
  assert(seen.labels.length > 0, 'the timeline names none of its lanes');
  const hidden = seen.labels.filter((l) => l.right <= seen.left || l.left >= seen.right).map((l) => l.text);
  assert(hidden.length === 0, `following the playhead scrolled the lane names out of view: ${hidden.join(', ')}`);
});

scenario('the_hardware_view_shows_every_register', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  await t.runToEnd();
  await (await t.dbgButton('Virtual hardware')).click();
  await t.frames(3);
  const view = await t.page.evaluate(() => {
    const frame = document.querySelector('.dbg-hw').getBoundingClientRect();
    const cells = [...document.querySelectorAll('.dbg-hw .rcell rect')].map((r) => r.getBoundingClientRect());
    return { frameLeft: frame.left, frameRight: frame.right, cells: cells.map((c) => [c.left, c.right]) };
  });
  const slots = await (async () => { await t.dbgTab('Stack'); return t.page.$$eval('.dbg-slot', (els) => els.length); })();
  assert(view.cells.length === slots, `the hardware view draws ${view.cells.length} register cells for ${slots} registers`);
  const clipped = view.cells.filter(([l, r]) => l < view.frameLeft - 1 || r > view.frameRight + 1).length;
  assert(clipped === 0, `${clipped} register cell(s) fall outside the hardware view`);
});

scenario('the_heap_counts_the_programs_references', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  await t.runToEnd();
  await t.dbgTab('Heap');
  const objects = await t.page.$$eval('.dbg-hobj', (els) => els.map((o) => ({
    rc: Number((o.querySelector('.dbg-hobj-rc')?.innerText || 'rc 1').replace(/\D+/g, '')),
    refs: o.querySelectorAll('.dbg-ref').length,
  })));
  assert(objects.length > 0, 'the heap shows no objects');
  const wrong = objects.filter((o) => o.rc !== o.refs);
  assert(wrong.length === 0, `heap reference counts disagree with the references shown: ${JSON.stringify(wrong)}`);
});

scenario('the_column_bars_share_one_opaque_grey', async (t) => {
  await t.open(FILES.threePanels);
  const colors = await t.page.$$eval('.panel-resizer', (els) => els
    .filter((e) => getComputedStyle(e).display !== 'none')
    .map((e) => getComputedStyle(e).backgroundColor));
  assert(colors.length === 3, `expected three column bars, found ${colors.length}`);
  assert(new Set(colors).size === 1, `the column bars differ: ${colors.join(' | ')}`);
  assert(/^rgb\(/.test(colors[0]), `the bars are translucent (${colors[0]}), so each shows a different grey over what is behind it`);
});

scenario('every_mode_edits_in_the_code_editor', async (t) => {
  const look = async (file) => {
    await t.open(file);
    return t.page.$eval('.studio-main > section textarea', (e) => {
      const s = getComputedStyle(e);
      return { class: e.className, font: s.fontFamily, size: s.fontSize, line: s.lineHeight, padding: s.padding, highlighted: !!e.closest('section').querySelector('.code-editor-highlight') };
    });
  };
  const code = await look(FILES.code);
  for (const [mode, file] of [['Logic', FILES.logic], ['Math', FILES.math], ['Hardware', FILES.threePanels], ['Web', FILES.web]]) {
    const it = await look(file);
    // Math sets its text in Logos Math Mono, whose ligatures draw its symbols (2026-09-26, the
    // user's choice), falling back to exactly Code's typefaces.
    const font = mode === 'Math' ? `"Logos Math Mono", ${code.font}` : code.font;
    for (const key of Object.keys(code)) {
      const want = key === 'font' ? font : code[key];
      assert(it[key] === want, `${mode}'s editor differs from Code's in ${key}: ${JSON.stringify(it[key])} vs ${JSON.stringify(want)}`);
    }
  }
});

scenario('logic_mode_hovers_with_the_language_server', async (t) => {
  await t.open(FILES.logic);
  const word = (await t.page.evaluateHandle(() =>
    [...document.querySelectorAll('.studio-main > section .code-editor-highlight span')].find((s) => s.textContent === 'Theorem'))).asElement();
  assert(word, 'the Logic editor shows no highlighted "Theorem" token');
  const box = await word.boundingBox();
  await t.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 4 });
  const hint = await t.page.waitForSelector('.hover-hint', { timeout: 3000 }).catch(() => null);
  assert(hint, 'resting on "Theorem" in a .logic file shows no hint');
  const text = await hint.evaluate((h) => h.innerText);
  assert(/proof strategy/i.test(text), `the Theorem hint is not the language server's lesson: ${JSON.stringify(text.slice(0, 200))}`);
});

// Panels hold their content on the panel's own surface, the way the Code editor and its
// Output do: no bordered or rounded box wrapped around a formula, a card, or a list.
scenario('no_panel_draws_a_box_around_its_content', async (t) => {
  const offenders = [];
  for (const [kind, file] of EVERY_KIND) {
    await t.open(file);
    await new Promise((r) => setTimeout(r, 800));
    const found = await t.page.evaluate(() => {
      const alpha = (c) => { const m = c.match(/[\d.]+/g) || []; return m.length === 4 ? Number(m[3]) : m.length === 3 ? 1 : 0; };
      const out = [];
      for (const panel of document.querySelectorAll('.studio-main > section, .studio-main > aside')) {
        const title = panel.querySelector('.panel-header span')?.innerText.trim() || panel.className;
        const content = panel.querySelector('.panel-content') || panel;
        const width = content.getBoundingClientRect().width;
        for (const el of content.querySelectorAll('*')) {
          if (el.closest('.code-editor, iframe, .hover-hint, button, .dbg-drawer, svg')) continue;
          const r = el.getBoundingClientRect();
          if (r.width < width * 0.5 || r.height < 24) continue;
          const cs = getComputedStyle(el);
          const edges = ['Top', 'Right', 'Bottom', 'Left'].filter((side) =>
            parseFloat(cs[`border${side}Width`]) > 0 && cs[`border${side}Style`] !== 'none' && alpha(cs[`border${side}Color`]) > 0);
          const rounded = parseFloat(cs.borderTopLeftRadius) > 0;
          const filled = alpha(cs.backgroundColor) > 0;
          if (edges.length === 4 || (rounded && (edges.length >= 2 || filled))) {
            out.push(`${title}: <${el.tagName.toLowerCase()} class="${el.getAttribute('class') || ''}"> ${Math.round(r.width)}×${Math.round(r.height)}`);
          }
        }
      }
      return out;
    });
    offenders.push(...found.map((f) => `${kind} — ${f}`));
  }
  assert(offenders.length === 0, `panels draw boxes around their content:\n${offenders.join('\n')}`);
});

// A panel's header names it once; a second copy of the title inside the panel is noise.
scenario('no_panel_repeats_its_own_title', async (t) => {
  const offenders = [];
  for (const [kind, file] of EVERY_KIND) {
    await t.open(file);
    await new Promise((r) => setTimeout(r, 800));
    const found = await t.page.evaluate(() => {
      const out = [];
      for (const panel of document.querySelectorAll('.studio-main > section, .studio-main > aside')) {
        const title = panel.querySelector('.panel-header span')?.innerText.trim();
        const content = panel.querySelector('.panel-content');
        if (!title || !content) continue;
        for (const el of content.querySelectorAll('*')) {
          const own = [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join('').trim();
          if (own.toLowerCase() === title.toLowerCase()) out.push(`${title}: <${el.tagName.toLowerCase()} class="${el.getAttribute('class') || ''}">`);
        }
      }
      return out;
    });
    offenders.push(...found.map((f) => `${kind} — ${f}`));
  }
  assert(offenders.length === 0, `panels repeat their own titles:\n${offenders.join('\n')}`);
});

// What the page shows is written for a reader: never a Rust `Debug` dump of the engine's
// internals (`NeoEvent(NeoEventData { event_var: Symbol(55), … })`).
scenario('no_panel_shows_raw_debug_text', async (t) => {
  const offenders = [];
  for (const [kind, file] of EVERY_KIND) {
    await t.open(file);
    await new Promise((r) => setTimeout(r, 800));
    const found = await t.page.evaluate(() => {
      const raw = /\b[A-Z]\w*Data \{|\bSymbol\(\d+\)|\b[a-z_]+: \[\(|suppress_existential/;
      const out = [];
      const main = document.querySelector('.studio-main');
      const walk = document.createTreeWalker(main, NodeFilter.SHOW_TEXT);
      for (let n = walk.nextNode(); n; n = walk.nextNode()) {
        if (n.parentElement.closest('.code-editor')) continue;
        const m = n.textContent.match(raw);
        if (m) out.push(n.textContent.trim().slice(0, 90));
      }
      return out;
    });
    offenders.push(...found.map((f) => `${kind} — ${f}`));
  }
  assert(offenders.length === 0, `panels show raw debug text:\n${offenders.slice(0, 12).join('\n')}`);
});

// Without OPFS (a plain-http LAN address, a private window) the Studio keeps files in
// IndexedDB and says so — as a muted note beside the Files title, not a banner row that
// takes space from the tree. Reached through the machine's LAN address, the same server
// offers no OPFS, exactly as it does to the display box.
scenario('private_mode_is_a_note_beside_the_files_header', async (t) => {
  const origin = t.insecureOrigin();
  assert(origin, 'no LAN address to reach the site through without a secure context');
  await t.openFrom(origin, FILES.code);
  const look = await t.page.evaluate(() => {
    const header = document.querySelector('.file-browser-header');
    const note = header.querySelector('.file-browser-note');
    const token = getComputedStyle(document.querySelector('.studio-container')).getPropertyValue('--studio-text-secondary').trim();
    const probe = document.createElement('span');
    probe.style.color = token;
    document.body.appendChild(probe);
    const secondary = getComputedStyle(probe).color;
    probe.remove();
    const banner = [...document.querySelectorAll('.file-browser *')]
      .some((e) => !e.closest('.file-browser-header') && /private mode/i.test([...e.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join('')));
    return {
      note: note ? note.innerText.trim() : null,
      color: note ? getComputedStyle(note).color : null,
      secondary,
      next: header.nextElementSibling ? header.nextElementSibling.className : null,
      banner,
    };
  });
  assert(look.note === 'Private mode', `the Files header carries no "Private mode" note: ${JSON.stringify(look.note)}`);
  assert(look.color === look.secondary, `the note is ${look.color}, not the theme's secondary text ${look.secondary}`);
  assert(!look.banner, 'a "Private mode" banner still takes a row of the file list');
  assert(look.next === 'file-tree', `something sits between the Files header and the tree: ${JSON.stringify(look.next)}`);
});

// Every piece of a column holds its own content: nothing paints outside the piece it belongs
// to unless something clips it there, and no two pieces of one column overlap — at any
// window size. The Logic editor's text once spilled over the proof panel beneath it.
scenario('no_column_piece_overlaps_another', async (t) => {
  const offenders = [];
  const sizes = [[1920, 1080], [1366, 768], [1280, 720], [PHONE.width, PHONE.height]];
  for (const [kind, file] of [...EVERY_KIND, ['Logic long proof', FILES.longProof]]) {
    for (const [width, height] of sizes) {
      await t.fresh();
      await t.open(file, width === PHONE.width ? PHONE : { ...DESKTOP, width, height });
      await new Promise((r) => setTimeout(r, 600));
      const found = await t.page.evaluate(() => {
        const out = [];
        const clips = (el) => { const cs = getComputedStyle(el); return cs.overflowX !== 'visible' || cs.overflowY !== 'visible'; };
        const shown = (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== 'hidden'; };
        for (const panel of document.querySelectorAll('.studio-main > section, .studio-main > aside')) {
          const title = panel.querySelector('.panel-header span')?.innerText.trim() || panel.className;
          const content = panel.querySelector('.panel-content');
          if (!content || !shown(content)) continue;
          const pieces = [...content.children].filter((el) => el.tagName !== 'STYLE' && shown(el));
          for (let i = 1; i < pieces.length; i++) {
            const a = pieces[i - 1].getBoundingClientRect(), b = pieces[i].getBoundingClientRect();
            if (a.bottom > b.top + 1) out.push(`${title}: <${pieces[i - 1].className}> overlaps <${pieces[i].className}> by ${Math.round(a.bottom - b.top)}px`);
          }
          for (const piece of pieces) {
            const box = piece.getBoundingClientRect();
            for (const el of piece.querySelectorAll('*')) {
              if (el.closest('svg') && el.tagName.toLowerCase() !== 'svg') continue;
              if (!shown(el) || getComputedStyle(el).position === 'fixed') continue;
              let clipped = false;
              for (let up = el.parentElement; up && up !== piece.parentElement; up = up.parentElement) {
                if (clips(up)) { clipped = true; break; }
              }
              if (clipped) continue;
              const r = el.getBoundingClientRect();
              if (r.top < box.top - 1 || r.bottom > box.bottom + 1 || r.left < box.left - 1 || r.right > box.right + 1) {
                out.push(`${title}: <${el.tagName.toLowerCase()} class="${el.getAttribute('class') || ''}"> paints outside <${piece.className}> (${Math.round(r.top)}–${Math.round(r.bottom)} vs ${Math.round(box.top)}–${Math.round(box.bottom)})`);
                break;
              }
            }
          }
        }
        return out;
      });
      offenders.push(...found.map((f) => `${kind} @ ${width}×${height} — ${f}`));
    }
  }
  assert(offenders.length === 0, `column pieces overlap:\n${offenders.slice(0, 20).join('\n')}${offenders.length > 20 ? `\n… ${offenders.length - 20} more` : ''}`);
// The sweep makes 28 loads, each in a fresh page; leaving a page mid-work (a long proof search)
// cancels its requests, which Firefox reports as page errors. Nothing a reader does.
}, { allowErrors: NAVIGATION_ABORT });

// What every Studio example promises, published from its registry (`ui::examples`) by
// `crates/logicaffeine_tests/tests/studio_example_contract.rs`.
function studioContract() {
  const file = path.join(path.dirname(new URL(import.meta.url).pathname), '../../crates/logicaffeine_tests/tests/fixtures/studio_example_contract.json');
  return JSON.parse(fs.readFileSync(file, 'utf8')).examples;
}

// Everything the Studio's panels say right now: each panel's title and text, the analysis row,
// and each Math output line that is an error.
function readPanels(page) {
  return page.evaluate(() => ({
    panels: [...document.querySelectorAll('.studio-main > section, .studio-main > aside')].map((p) => ({
      title: p.querySelector('.panel-header span')?.innerText.trim() || '',
      text: p.querySelector('.panel-content')?.innerText || '',
    })),
    guide: document.querySelector('.studio-guide')?.innerText || '',
    replLines: document.querySelectorAll('.repl-line').length,
    replErrors: [...document.querySelectorAll('.repl-output-line.error')].map((e) => e.innerText.trim()),
  }));
}

const panelText = (seen, title) => seen.panels.filter((p) => p.title === title).map((p) => p.text).join('\n');
const lines = (text) => text.split('\n').map((l) => l.trimEnd()).join('\n').trim();

// Poll the panels until `ok` holds or `ms` passes; the last reading either way.
async function settle(page, ok, ms = 12000) {
  const until = Date.now() + ms;
  let seen = await readPanels(page);
  while (!ok(seen) && Date.now() < until) {
    await new Promise((r) => setTimeout(r, 250));
    seen = await readPanels(page);
  }
  return seen;
}

async function press(page, label) {
  const button = await page.$(`xpath/.//div[contains(@class,'studio-toolbar')]//button[contains(., '${label}')]`);
  assert(button, `the toolbar has no ${label} button`);
  await button.click();
}

// A planted-bug demo says the failure is the point and names the same design fixed.
function plantedFindings(text, p) {
  const fixed = p.fixed.split('/').pop();
  if (!/planted bug/i.test(text)) return 'nothing says this failure is a planted bug';
  return text.includes(fixed) ? null : `the notice does not name the fixed design ${fixed}`;
}

// How the Studio shows each kind of promise. `check` returns null when kept, else why not.
const PROMISES = {
  output_exact: async (t, p) => {
    const seen = await settle(t.page, (s) => lines(panelText(s, 'Output')) === lines(p.output));
    const got = lines(panelText(seen, 'Output'));
    return got === lines(p.output) ? null : `Output shows ${JSON.stringify(got.slice(0, 160))}, not ${JSON.stringify(lines(p.output).slice(0, 160))}`;
  },
  output_contains: async (t, p) => {
    const seen = await settle(t.page, (s) => p.needles.every((n) => panelText(s, 'Output').includes(n)));
    const missing = p.needles.filter((n) => !panelText(seen, 'Output').includes(n));
    return missing.length ? `Output lacks ${JSON.stringify(missing)}` : null;
  },
  native_only: async (t, p) => {
    const ran = await settle(t.page, (s) => panelText(s, 'Output').includes(p.needs), 6000);
    const said = panelText(ran, 'Output');
    if (!said.includes(p.needs) || !/compiled runtime/.test(said) || !/largo build/.test(said)) {
      return `opening it shows ${JSON.stringify(said.trim().slice(0, 120))}, not that ${p.needs} needs the compiled runtime`;
    }
    await press(t.page, 'Compile');
    const rust = await settle(t.page, (s) => /fn main/.test(panelText(s, 'Output')), 20000);
    return /fn main/.test(panelText(rust, 'Output')) ? null : 'Compile shows no Rust for it';
  },
  fol: async (t, p) => {
    const seen = await settle(t.page, (s) => p.needles.every((n) => panelText(s, 'Logic Output').includes(n)));
    const missing = p.needles.filter((n) => !panelText(seen, 'Logic Output').includes(n));
    return missing.length ? `Logic Output lacks ${JSON.stringify(missing)}` : null;
  },
  proves: async (t) => {
    const seen = await settle(t.page, (s) => panelText(s, 'English Input').includes('Proof found') && /Proved/.test(s.guide), 20000);
    if (!panelText(seen, 'English Input').includes('Proof found')) return 'the proof panel does not say "Proof found"';
    return /Proved/.test(seen.guide) ? null : `the Analysis row says ${JSON.stringify(seen.guide.trim().slice(0, 100))}, not that it is proved`;
  },
  derivation_not_certified: async (t) => {
    const seen = await settle(t.page, (s) => /not certified|uncertified/i.test(panelText(s, 'English Input') + s.guide), 20000);
    return /not certified|uncertified/i.test(panelText(seen, 'English Input') + seen.guide) ? null
      : `it never says its derivation is not certified (the proof panel says ${JSON.stringify(panelText(seen, 'English Input').replace(/\s+/g, ' ').slice(-80))})`;
  },
  kernel_ok: async (t) => {
    await press(t.page, 'Execute');
    const seen = await settle(t.page, (s) => s.replLines > 0, 20000);
    if (seen.replLines === 0) return 'Execute prints nothing';
    return seen.replErrors.length ? `a statement fails: ${JSON.stringify(seen.replErrors[0].slice(0, 160))}` : null;
  },
  kernel_admits: async (t, p) => {
    await press(t.page, 'Execute');
    const seen = await settle(t.page, (s) => panelText(s, 'Output').includes(p.marker), 20000);
    return panelText(seen, 'Output').includes(p.marker) ? null : `Output never names the open point ${JSON.stringify(p.marker)}`;
  },
  theory_verified: async (t) => {
    await press(t.page, 'Execute');
    const seen = await settle(t.page, (s) => /kernel-certified/.test(panelText(s, 'Output')), 20000);
    if (!/kernel-certified/.test(panelText(seen, 'Output'))) return 'no theorem is reported kernel-certified';
    return seen.replErrors.length ? `a theorem fails: ${JSON.stringify(seen.replErrors[0].slice(0, 160))}` : null;
  },
  sva: async (t) => {
    const seen = await settle(t.page, (s) => /assert property/.test(panelText(s, 'SVA')) && /Matches the spec/.test(panelText(s, 'SVA')));
    return /assert property/.test(panelText(seen, 'SVA')) && /Matches the spec/.test(panelText(seen, 'SVA')) ? null : 'no certified assertion';
  },
  signal_plan: async (t) => {
    const seen = await settle(t.page, (s) => /Phase 1/.test(panelText(s, 'Signal Plan')));
    const text = panelText(seen, 'Signal Plan');
    if (!/Phase 1/.test(text)) return `no phase plan in a Signal Plan panel (panels: ${JSON.stringify(seen.panels.map((p) => p.title))})`;
    return /PHASE PLAN/i.test(text) && /GENERATED CONTROLLER/i.test(text) ? null : 'the plan and its controller are not labelled as such';
  },
  rtl_proven: async (t) => {
    const seen = await settle(t.page, (s) => /Always holds/.test(panelText(s, 'Model Check')), 20000);
    return /Always holds/.test(panelText(seen, 'Model Check')) ? null
      : `not proven in a Model Check panel: ${JSON.stringify(seen.panels.map((p) => `${p.title}: ${p.text.replace(/\s+/g, ' ').slice(0, 80)}`))}`;
  },
  rtl_refuted: async (t, p) => {
    const seen = await settle(t.page, (s) => /Can break/.test(panelText(s, 'Model Check')), 20000);
    const text = panelText(seen, 'Model Check');
    if (!/Can break/.test(text)) return `no counterexample in a Model Check panel (panels: ${JSON.stringify(seen.panels.map((q) => q.title))})`;
    if (/below/.test(text) || !/waveform/.test(text)) return `the verdict does not point at the waveform: ${JSON.stringify(text.replace(/\s+/g, ' ').slice(0, 160))}`;
    return plantedFindings(text, p);
  },
  regalloc_fits: async (t) => {
    const seen = await settle(t.page, (s) => /Allocated/.test(panelText(s, 'Allocation')));
    return /Allocated .* certified/.test(panelText(seen, 'Allocation').replace(/\s+/g, ' ')) ? null : 'no certified allocation';
  },
  regalloc_spills: async (t, p) => {
    const seen = await settle(t.page, (s) => /Must spill/.test(panelText(s, 'Allocation')));
    const text = panelText(seen, 'Allocation');
    return /Must spill/.test(text) ? plantedFindings(text, p) : 'no certified spill';
  },
  unsat_certified: async (t) => {
    const seen = await settle(t.page, (s) => /Certified UNSAT/.test(panelText(s, 'Proof')));
    if (!/Certified UNSAT/.test(panelText(seen, 'Proof'))) return 'the Proof panel never says its refutation is certified';
    const verdict = await t.page.$$eval('.studio-main .panel-verdict.proved', (vs) => vs.map((v) => v.innerText.trim()));
    return verdict.some((v) => v.startsWith('✓ Certified UNSAT')) ? null : `the certified result is not shown as a success: ${JSON.stringify(verdict)}`;
  },
  web_app: async (t) => {
    const until = Date.now() + 15000;
    let text = '';
    while (Date.now() < until) {
      const frame = t.page.frames().find((f) => f !== t.page.mainFrame());
      text = frame ? await frame.evaluate(() => document.body ? document.body.innerText.trim() : '').catch(() => '') : '';
      if (text) break;
      await new Promise((r) => setTimeout(r, 250));
    }
    return text ? null : 'the preview renders nothing';
  },
};

// A scroll area scrolls only when its content needs it: no scrollbar drawn over content that
// fits, none for a pixel or two of rounding, and no run of empty space past the last content.
function scrollFindings(page) {
  return page.evaluate(() => {
    const out = [];
    const scrolls = (v) => v === 'auto' || v === 'scroll';
    for (const panel of document.querySelectorAll('.studio-main > section, .studio-main > aside, .studio-sidebar')) {
      const title = panel.querySelector('.panel-header span, .file-browser-title')?.innerText.trim() || panel.className;
      for (const el of [panel, ...panel.querySelectorAll('*')]) {
        const cs = getComputedStyle(el);
        const r = el.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) continue;
        const name = `<${el.tagName.toLowerCase()} class="${el.getAttribute('class') || ''}">`;
        const over = el.scrollHeight - el.clientHeight;
        if (cs.overflowY === 'scroll' && over <= 0) out.push(`${title} ${name}: a vertical scrollbar over content that fits`);
        if (cs.overflowX === 'scroll' && el.scrollWidth - el.clientWidth <= 0) out.push(`${title} ${name}: a horizontal scrollbar over content that fits`);
        if (!scrolls(cs.overflowY) || over <= 0) continue;
        if (over <= 4) { out.push(`${title} ${name}: a scrollbar for ${over}px of rounding`); continue; }
        if (el.tagName === 'TEXTAREA') continue;
        let bottom = 0;
        for (const child of el.querySelectorAll('*')) {
          const c = child.getBoundingClientRect();
          if (c.height > 0) bottom = Math.max(bottom, c.bottom - r.top + el.scrollTop);
        }
        const dead = el.scrollHeight - bottom - parseFloat(cs.paddingBottom);
        if (dead > 32) out.push(`${title} ${name}: ${Math.round(dead)}px of empty space to scroll into`);
      }
    }
    return out;
  });
}

// No two panels share a title, no drawn caption runs past the picture that holds it, and none is
// drawn too small to read: a picture wider than its panel keeps its type and scrolls instead.
const LEGIBLE_TEXT_PX = 10;
function panelFindings(page) {
  return page.evaluate((minPx) => {
    const out = [];
    const titles = [...document.querySelectorAll('.studio-main > section, .studio-main > aside')]
      .map((p) => p.querySelector('.panel-header span')?.innerText.trim())
      .filter(Boolean);
    const twice = titles.find((t, i) => titles.indexOf(t) !== i);
    if (twice) out.push(`two panels are titled ${JSON.stringify(twice)}`);
    for (const text of document.querySelectorAll('.studio-main svg text')) {
      const svg = text.ownerSVGElement;
      const t = text.getBoundingClientRect(), b = svg.getBoundingClientRect();
      if (!t.width || !b.width) continue;
      const over = Math.max(b.left - t.left, t.right - b.right);
      if (over > 1) out.push(`the caption ${JSON.stringify(text.textContent.slice(0, 40))} runs ${Math.round(over)}px past its picture`);
    }
    const tiny = [...document.querySelectorAll('.studio-main svg text')]
      .map((text) => ({ text: text.textContent.trim(), height: text.getBoundingClientRect().height }))
      .filter((t) => t.height > 0 && t.text && t.height < minPx);
    if (tiny.length) {
      const least = tiny.reduce((a, b) => (b.height < a.height ? b : a));
      out.push(`${tiny.length} drawn caption(s) are too small to read, down to ${least.height.toFixed(1)}px (${JSON.stringify(least.text.slice(0, 30))})`);
    }
    return out;
  }, LEGIBLE_TEXT_PX);
}

// Every example of one file type, opened as a reader would: it keeps the promise its registry
// makes, its scroll areas scroll only when they must, its panels have their own titles, and its
// captions stay inside their pictures. One failure line per broken example.
async function keepsItsPromises(t, mode) {
  const broken = [];
  for (const example of studioContract().filter((e) => e.mode === mode)) {
    const check = PROMISES[example.promise.kind];
    assert(check, `no browser check for promises of kind ${example.promise.kind}`);
    try {
      await t.fresh();
      await t.openWithin(example.file, 90000);
      const why = await check(t, example.promise);
      if (why) broken.push(`${example.file}: ${why}`);
      for (const f of await scrollFindings(t.page)) broken.push(`${example.file}: ${f}`);
      for (const f of await panelFindings(t.page)) broken.push(`${example.file}: ${f}`);
    } catch (e) {
      broken.push(`${example.file}: ${String(e.message || e).split('\n')[0]}`);
    }
  }
  assert(broken.length === 0, `${broken.length} broken promise(s):\n${broken.join('\n')}`);
}

// The sweeps leave pages mid-work, as no_column_piece_overlaps_another does. A sweep's time grows
// with its registry — Firefox opens the Code examples in ~8s each — so its backstop is a budget
// per example, not the one fixed limit a single scenario gets.
const sweep = (mode) => ({ allowErrors: NAVIGATION_ABORT, timeout: 60000 + studioContract().filter((e) => e.mode === mode).length * 30000 });
scenario('every_logic_example_keeps_its_promise', (t) => keepsItsPromises(t, 'logic'), sweep('logic'));
scenario('every_code_example_keeps_its_promise', (t) => keepsItsPromises(t, 'code'), sweep('code'));
scenario('every_math_example_keeps_its_promise', (t) => keepsItsPromises(t, 'math'), sweep('math'));
scenario('every_hardware_example_keeps_its_promise', (t) => keepsItsPromises(t, 'hardware'), sweep('hardware'));
scenario('every_web_example_keeps_its_promise', (t) => keepsItsPromises(t, 'web'), sweep('web'));

// The file tree reads like VS Code's explorer: compact 22px rows in its font at 13px, each naming
// its whole path in a tooltip. Its geometry is one step: a row at depth d starts its first glyph —
// a folder's chevron, a file's icon — (d + 1) steps and one gap from the sidebar's edge, so the
// edge and the levels read as one scale.
scenario('the_file_tree_reads_like_vs_code', async (t) => {
  await t.open(FILES.code);
  const look = await t.page.evaluate(() => {
    const tree = document.querySelector('.file-tree');
    const rows = [...tree.querySelectorAll('.file-tree-item')].filter((r) => r.getBoundingClientRect().height > 0);
    const depth = (row) => { let d = 0; for (let n = row.parentElement.parentElement; n && n !== tree; n = n.parentElement) if (n.classList.contains('file-tree-children')) d++; return d; };
    const first = (row) => (row.querySelector('.twistie') || row.querySelector('.icon')).getBoundingClientRect().left - tree.getBoundingClientRect().left;
    return {
      heights: [...new Set(rows.map((r) => Math.round(r.getBoundingClientRect().height)))],
      font: getComputedStyle(rows[0]).fontFamily,
      size: getComputedStyle(rows[0]).fontSize,
      rows: rows.map((r) => ({ depth: depth(r), first: first(r), dir: !!r.querySelector('.chevron'), cell: !!r.querySelector('.twistie'), title: r.getAttribute('title'), name: r.querySelector('.name').innerText })),
    };
  });
  assert(look.heights.length === 1 && look.heights[0] === 22, `file tree rows are ${JSON.stringify(look.heights)}px tall, not 22px`);
  assert(/^-apple-system/.test(look.font) && look.size === '13px', `the tree is set in ${look.font} ${look.size}, not VS Code's explorer font at 13px`);
  for (const r of look.rows) {
    const want = TREE_STEP * (r.depth + 1) + TREE_GAP;
    assert(near(r.first, want, 0.5), `${r.name} (depth ${r.depth}) starts ${px(r.first)} from the sidebar's edge, not ${px(want)} — ${r.depth + 1} steps and a gap`);
    assert(r.dir === r.cell, r.dir ? `the folder ${r.name} has no chevron cell` : `the file ${r.name} keeps an empty chevron cell before its icon`);
  }
  assert(look.rows.every((r) => r.title), 'a row carries no tooltip naming its whole path');
});

// Chevrons are drawn, not typed, and read at a glance: a 2px stroke, 12px along their long side
// (two thirds larger than the 10px codicon they replaced, and a pixel heavier), filling their
// 12px cell — so an open folder's chevron points down from the cell's left edge — centred on
// their row, and 4px clear of the folder icon beside them.
scenario('file_tree_chevrons_sit_centred', async (t) => {
  await t.open(FILES.code);
  const off = await t.page.evaluate(() => [...document.querySelectorAll('.file-tree-item')].flatMap((row) => {
    const chevron = row.querySelector('.chevron');
    if (!chevron) return [];
    const r = row.getBoundingClientRect(), c = chevron.getBoundingClientRect(), cell = row.querySelector('.twistie').getBoundingClientRect();
    const icon = row.querySelector('.icon').getBoundingClientRect();
    const svg = chevron.querySelector('svg'), path = svg && svg.querySelector('path');
    if (!path) return [{ drawn: false }];
    // The drawn shape from its geometry (getBBox), scaled from the SVG's units to the page. A
    // path's client rect cannot be used: Firefox counts the stroke in it and Chrome does not.
    // The SVG is square, so a quarter turn leaves its scale alone.
    const scale = svg.getBoundingClientRect().width / svg.viewBox.baseVal.width;
    const ps = getComputedStyle(path), box = path.getBBox();
    const stroke = (ps.stroke === 'none' ? 0 : parseFloat(ps.strokeWidth)) * scale;
    return [{
      drawn: true,
      name: row.querySelector('.name').innerText,
      offset: (c.top + c.height / 2) - (r.top + r.height / 2),
      stroke,
      long: Math.max(box.width, box.height) * scale + stroke,
      cell: { width: cell.width, chevronWidth: c.width, chevronLeft: c.left - cell.left },
      toIcon: icon.left - cell.right,
    }];
  }));
  assert(off.length > 0, 'the tree shows no folder chevrons');
  for (const c of off) {
    assert(c.drawn, 'a chevron is a text glyph, not a drawn icon');
    assert(Math.abs(c.offset) <= 0.5, `the chevron of ${c.name} sits ${px(c.offset)} off its row's centre`);
    assert(near(c.stroke, 2, 0.01), `the chevron of ${c.name} is drawn with a ${c.stroke}px stroke, not 2px`);
    assert(near(c.long, 12, 0.5), `the chevron of ${c.name} is ${px(c.long)} along its long side, not 12px`);
    assert(near(c.cell.width, 12, 0.5) && near(c.cell.chevronWidth, c.cell.width, 0.5) && near(c.cell.chevronLeft, 0, 0.5),
      `the chevron of ${c.name} does not fill its 12px cell (cell ${px(c.cell.width)}, chevron ${px(c.cell.chevronWidth)} at ${px(c.cell.chevronLeft)})`);
    assert(near(c.toIcon, 4, 0.5), `the chevron of ${c.name} sits ${px(c.toIcon)} from its folder icon, not 4px`);
  }
});

// The tree is drawn as branches of one length. An open folder's children hang from one line, one
// step further in than the line the folder hangs from (the first level's, one step from the
// sidebar's edge). That line starts at the folder's own branch — a corner, never a gap; for a
// top-level folder, at its row's middle — runs unbroken from child to child and stops at the last
// child's branch. Each child's branch leaves the line at its row's middle and runs exactly one
// step, ending one gap before the row's first glyph.
scenario('the_file_tree_branches_connect', async (t) => {
  await t.open(FILES.code);
  const problems = await t.page.evaluate((STEP, GAP) => {
    const out = [];
    const tree = document.querySelector('.file-tree');
    const edge = tree.getBoundingClientRect().left;
    const alpha = (c) => { const m = c.match(/[\d.]+/g) || []; return m.length === 4 ? Number(m[3]) : 1; };
    const parts = (node) => {
      const row = node.querySelector(':scope > .file-tree-item');
      return { row, line: node.querySelector(':scope > .tree-line'), branch: row && row.querySelector(':scope > .tree-branch'), name: row?.querySelector('.name')?.innerText };
    };
    const rect = (el) => el && el.getBoundingClientRect();
    for (const group of document.querySelectorAll('.file-tree-children')) {
      const folder = parts(group.parentElement);
      const nodes = [...group.children].filter((n) => n.classList.contains('file-tree-node'));
      nodes.forEach((node, i) => {
        const { row, line, branch, name } = parts(node);
        if (!line || !branch) { out.push(`${name}: no tree line or branch`); return; }
        const l = rect(line), b = rect(branch), r = rect(row);
        if (alpha(getComputedStyle(line).backgroundColor) === 0 || alpha(getComputedStyle(branch).backgroundColor) === 0) out.push(`${name}: a line is invisible`);
        if (Math.abs((b.top + b.height / 2) - (r.top + r.height / 2)) > 1) out.push(`${name}: its branch is not at the row's middle`);
        if (Math.abs(b.left - l.left) > 0.5) out.push(`${name}: its branch does not leave the tree line`);
        if (Math.abs(b.width - STEP) > 0.5) out.push(`${name}: its branch runs ${b.width.toFixed(1)}px, not one ${STEP}px step`);
        const glyph = rect(row.querySelector('.twistie') || row.querySelector('.icon'));
        if (Math.abs(glyph.left - b.right - GAP) > 0.5) out.push(`${name}: ${(glyph.left - b.right).toFixed(1)}px between its branch and its first glyph, not ${GAP}px`);
        const from = folder.line ? rect(folder.line).left : edge;
        if (Math.abs(l.left - from - STEP) > 0.5) out.push(`${name}: its line stands ${(l.left - from).toFixed(1)}px in from ${folder.line ? `${folder.name}'s line` : "the sidebar's edge"}, not one ${STEP}px step`);
        if (i === 0) {
          const fr = rect(folder.row);
          const start = folder.branch ? rect(folder.branch).top : fr.top + fr.height / 2 - l.width / 2;
          if (Math.abs(l.top - start) > 0.5) out.push(`${name}: the line under ${folder.name} starts ${(l.top - start).toFixed(1)}px from ${folder.branch ? 'its branch' : "its row's middle"}`);
          if (folder.branch && Math.abs(rect(folder.branch).right - l.left) > 0.5) out.push(`${folder.name}: its branch ends ${(l.left - rect(folder.branch).right).toFixed(1)}px short of the line its children hang from`);
        }
        const last = i === nodes.length - 1;
        if (last && Math.abs(l.bottom - b.bottom) > 0.5) out.push(`${name}: the line does not stop at the last child's branch (${(l.bottom - b.bottom).toFixed(1)}px)`);
        if (!last) {
          const next = rect(nodes[i + 1].querySelector(':scope > .tree-line'));
          if (Math.abs(next.top - l.bottom) > 0.5) out.push(`${name}: the tree line breaks before the next sibling (${(next.top - l.bottom).toFixed(1)}px)`);
        }
      });
    }
    return out;
  }, TREE_STEP, TREE_GAP);
  assert(problems.length === 0, `the tree's lines do not connect:\n${problems.slice(0, 12).join('\n')}`);
});

// Every line of the tree, down its levels and across to each row, keeps one weight at every zoom:
// at each real device scale the tree reads the page's ratio, every line's edges land on the
// device-pixel grid wherever the layout above the tree puts its top, and all lines share one weight
// of at least two device pixels, so no line reads heavier or fainter than another.
scenario('the_file_tree_lines_keep_one_weight_at_every_zoom', async (t) => {
  const executable = t.page.browser().process()?.spawnfile;
  assert(executable, 'this run did not launch its browser, so it cannot launch one at a device scale');
  const read = (page) => page.evaluate(() => {
    const tree = document.querySelector('.file-tree');
    return {
      ratio: window.devicePixelRatio,
      dp: parseFloat(getComputedStyle(tree).getPropertyValue('--dp')),
      lines: [...tree.querySelectorAll('.tree-line, .tree-branch')].map((e) => {
        const r = e.getBoundingClientRect(), down = e.classList.contains('tree-line');
        return { weight: down ? r.width : r.height, edges: [r.left, r.top, r.right, r.bottom] };
      }),
    };
  });
  const between = (look) => look.lines.flatMap((l) => l.edges).map((v) => v * look.ratio).find((v) => Math.abs(v - Math.round(v)) > 0.02);
  for (const scale of REAL_SCALES[t.engine]) {
    const at = `at a real ${Math.round(scale * 100)}% scale`;
    const browser = await launchBrowser(t.engine, executable, scale);
    try {
      const page = await browser.newPage();
      const errors = [];
      page.on('pageerror', (e) => {
        const text = String(e.message || e);
        if (!DEV_CLIENT_NOISE.test(text) && !DEV_SERVER_SWAP.test(text)) errors.push(text.slice(0, 300));
      });
      await helpers(page, t.base, t.engine).openFrom(t.base, FILES.code, t.engine === 'firefox' ? DESKTOP : null);
      // The tree measures where it landed once it is laid out, and moves onto the grid a render later.
      let look = await read(page);
      for (const until = Date.now() + 5000; between(look) !== undefined && Date.now() < until;) {
        await new Promise((r) => setTimeout(r, 100));
        look = await read(page);
      }
      assert(Math.abs(look.ratio - scale) < 0.01, `${at} the page reports a device pixel ratio of ${look.ratio}`);
      assert(Math.abs(look.dp * look.ratio - 1) < 1e-6, `${at} the tree takes a device pixel for ${look.dp}px, not ${1 / look.ratio}px`);
      assert(look.lines.length > 0, `${at} the tree draws no lines`);
      const weights = [...new Set(look.lines.map((l) => Math.round(l.weight * look.ratio * 100) / 100))];
      assert(weights.length === 1, `${at} the tree's lines are ${weights.join(', ')} device pixels wide — not one weight`);
      assert(Number.isInteger(weights[0]) && weights[0] >= 2, `${at} the tree's lines are ${weights[0]} device pixels wide, not a whole number of at least two`);
      const off = between(look);
      assert(off === undefined, `${at} a line's edge lands at ${off?.toFixed(3)} device pixels, between two`);
      assert(errors.length === 0, `${at} the page raised errors:\n${errors.join('\n')}`);
    } finally {
      await browser.close();
    }
  }
});

// The horizontal grab bars — the debugger's top edge, Logic's proof divider — are solid 8px bars
// in the vertical column bars' own grey.
scenario('the_grab_bars_match_the_column_bars', async (t) => {
  const look = (sel) => t.page.$eval(sel, (el) => ({ height: el.getBoundingClientRect().height, bg: getComputedStyle(el).backgroundColor }));
  await t.open(FILES.code);
  const column = await t.page.$eval('.panel-resizer', (el) => getComputedStyle(el).backgroundColor);
  await t.openDebugger();
  const bars = [['the debugger grip', await look('.dbg-grip')]];
  await t.open(FILES.logic);
  bars.push(['the proof divider', await look('.panel-content-stack > .stack-grip')]);
  for (const [name, bar] of bars) {
    assert(near(bar.height, 8, 0.5), `${name} is ${px(bar.height)} tall, not 8px`);
    assert(bar.bg === column, `${name} is ${bar.bg}, not the column bars' ${column}`);
  }
});

// A first visit, with no file named, opens the first Code example the file tree shows.
scenario('a_first_visit_opens_the_first_code_example', async (t) => {
  await t.page.setViewport(DESKTOP);
  await t.load(`${t.base}/studio`);
  await t.page.waitForFunction(() => {
    const editor = document.querySelector('.studio-main > section textarea');
    return editor && editor.value.trim().length > 0;
  }, { timeout: 120000 });
  const look = await t.page.evaluate(() => {
    const rows = [...document.querySelectorAll('.file-tree-item')].filter((r) => !r.querySelector('.chevron'));
    const firstCode = rows.find((r) => (r.getAttribute('title') || '').startsWith('/examples/code/'));
    return {
      first: firstCode && firstCode.getAttribute('title'),
      selected: document.querySelector('.file-tree-item.selected')?.getAttribute('title'),
      editor: document.querySelector('.studio-main > section .panel-header span')?.innerText.trim(),
      url: location.search,
    };
  });
  assert(look.first, 'the file tree shows no Code example');
  assert(look.selected === look.first, `a first visit opened ${JSON.stringify(look.selected)}, not the first Code example ${JSON.stringify(look.first)}`);
  assert(look.editor === 'Code Editor', `a first visit shows ${JSON.stringify(look.editor)}, not the Code Editor`);
});

// The panel titles each mode draws, in order (studio.rs).
const MODE_TITLES = {
  Logic: ['English Input', 'Logic Output'],
  Code: ['Code Editor', 'Output'],
};

// Record, from the document's first moment, every panel title the Studio shows and the first text
// its editor shows: a MutationObserver reads each change to the page as it is made, and a sampler
// reads the page every animation frame, as it is painted. Both read the prerendered copy in #main
// and the live app in #app, each while it is visible.
async function recordFirstFrames(page) {
  await page.evaluateOnNewDocument(() => {
    const log = (window.__studioFrames = { titles: [], editors: [] });
    // `visibilityProperty` is the option's current name, `checkVisibilityCSS` its first.
    const visible = (el) => (el.checkVisibility
      ? el.checkVisibility({ visibilityProperty: true, checkVisibilityCSS: true })
      : el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden');
    const sample = () => {
      for (const id of ['main', 'app']) {
        const root = document.getElementById(id);
        if (!root || !visible(root)) continue;
        for (const header of root.querySelectorAll('.studio-main .panel-header')) {
          const title = visible(header) && header.querySelector(':scope > span')?.textContent.trim();
          if (title && !log.titles.some((t) => t.root === id && t.title === title)) log.titles.push({ root: id, title });
        }
        const editor = root.querySelector('.studio-main > section .code-editor-highlight');
        if (editor && visible(editor) && !log.editors.some((e) => e.root === id)) log.editors.push({ root: id, text: editor.textContent });
      }
    };
    new MutationObserver(sample).observe(document, { childList: true, subtree: true, characterData: true, attributes: true });
    const frame = () => { sample(); requestAnimationFrame(frame); };
    requestAnimationFrame(frame);
  });
}

// The Studio at rest: the live app alone, its file system answered (the tree shows files), a file
// in the editor and its address in the bar.
async function studioSettled(t) {
  await t.page.waitForFunction(() => {
    const editor = document.querySelector('#app .studio-main > section textarea');
    return !document.getElementById('main') && editor && editor.value.trim().length > 0
      && document.querySelector('#app .file-tree-item.selected') && /[?&]file=/.test(location.search);
  }, { timeout: 120000 });
  await t.frames(6);
}

// What the recorder saw, against the mode the Studio must show from its first frame to its last.
async function assertFirstFramesAre(t, mode, what) {
  const seen = await t.page.evaluate(() => ({
    ...window.__studioFrames,
    final: document.querySelector('#app .studio-main > section .code-editor-highlight')?.textContent ?? null,
  }));
  const shown = [...new Set(seen.titles.map((s) => s.title))];
  const expected = MODE_TITLES[mode];
  const where = (title) => seen.titles.filter((s) => s.title === title).map((s) => `#${s.root}`).join(' and ');
  assert(shown.length === expected.length && expected.every((title) => shown.includes(title)),
    `${what} showed the panels ${shown.map((title) => `${JSON.stringify(title)} (${where(title)})`).join(', ')}, not only ${mode}'s ${expected.join(', ')}`);
  assert(seen.final && seen.final.trim(), `${what} ends with an empty editor`);
  assert(seen.editors.length > 0, `${what}: the recorder never saw the editor, so it proves nothing`);
  for (const first of seen.editors) {
    assert(first.text === seen.final, `${what}: the editor in #${first.root} first showed ${JSON.stringify(first.text.slice(0, 60))}, not the ${JSON.stringify(seen.final.slice(0, 60))} it ends with`);
  }
}

// A first visit opens straight into the state it ends up in: Code mode with the first Code example
// in the editor, from the first frame — never Logic first and Code a moment later. Every scenario
// runs in a fresh browser context, so the file system holds nothing from an earlier visit.
scenario('a_fresh_visit_shows_code_mode_from_the_first_frame', async (t) => {
  await recordFirstFrames(t.page);
  await t.page.setViewport(DESKTOP);
  await t.load(`${t.base}/studio`);
  await studioSettled(t);
  await assertFirstFramesAre(t, 'Code', 'a fresh /studio');
});

// A deep link opens in its file's mode, with its text, from the first frame.
scenario('a_logic_deep_link_shows_logic_from_the_first_frame', async (t) => {
  await recordFirstFrames(t.page);
  await t.page.setViewport(DESKTOP);
  await t.load(`${t.base}/studio?file=${FILES.sentences}`);
  await studioSettled(t);
  await assertFirstFramesAre(t, 'Logic', `/studio?file=${FILES.sentences}`);
});

// A link to a file the file system does not hold — the retired example the landing page links, a
// Logic file that never existed — opens the first Code example once the file system says so.
scenario('a_missing_deep_link_opens_the_first_code_example', async (t) => {
  await t.page.setViewport(DESKTOP);
  for (const missing of ['examples/code/fibonacci.logos', 'examples/logic/no-such-file.logic']) {
    await t.load(`${t.base}/studio?file=${missing}`);
    await t.page.waitForFunction(() => document.querySelector('#app .file-tree-item'), { timeout: 120000 });
    await t.page.waitForFunction(() => document.querySelector('#app .file-tree-item.selected'), { timeout: 15000 }).catch(() => {});
    await t.frames(6);
    const look = await t.page.evaluate(() => {
      const rows = [...document.querySelectorAll('#app .file-tree-item')].filter((r) => !r.querySelector('.chevron'));
      return {
        first: rows.find((r) => (r.getAttribute('title') || '').startsWith('/examples/code/'))?.getAttribute('title'),
        selected: document.querySelector('#app .file-tree-item.selected')?.getAttribute('title') ?? null,
        title: document.querySelector('#app .studio-main > section .panel-header span')?.innerText.trim(),
        editor: document.querySelector('#app .studio-main > section textarea')?.value ?? '',
        url: location.search,
      };
    });
    const what = `/studio?file=${missing}`;
    assert(look.first, 'the file tree shows no Code example');
    assert(look.selected === look.first, `${what} ended on ${JSON.stringify(look.selected)}, not the first Code example ${look.first}`);
    assert(look.title === 'Code Editor', `${what} ended in ${JSON.stringify(look.title)}, not the Code Editor`);
    assert(look.editor.trim(), `${what} ended with an empty editor`);
    assert(look.url === `?file=${look.first.slice(1)}`, `${what} left the address bar at ${look.url}`);
  }
});

// Hold the Studio's file system back until the scenario releases it. The app reaches its files
// through a worker (OPFS), so its messages to the worker wait in a queue, and every step after —
// seeding, the tree, reading the file — waits with them.
async function holdFileSystem(page) {
  await page.evaluateOnNewDocument(() => {
    const Real = window.Worker;
    const waiting = [];
    const hold = (window.__fileSystemHold = {
      held: true,
      workers: 0,
      release() {
        hold.held = false;
        for (const [worker, args] of waiting.splice(0)) Real.prototype.postMessage.apply(worker, args);
      },
    });
    window.Worker = class extends Real {
      constructor(...args) { super(...args); hold.workers++; }
      postMessage(...args) { if (hold.held) waiting.push([this, args]); else super.postMessage(...args); }
    };
  });
}

// What the reader types the moment the Studio paints is theirs: the file system's answer, however
// late, never replaces it.
scenario('typing_right_after_the_first_paint_survives_the_file_system', async (t) => {
  const MARK = ' typed before the files';
  await holdFileSystem(t.page);
  await t.page.setViewport(DESKTOP);
  await t.load(`${t.base}/studio`);
  const editor = await t.page.waitForSelector('#app .studio-main > section textarea', { timeout: 120000 });
  // New file waits until the reader's store has answered: the page's own sign that it has not.
  const held = await t.page.evaluate(() => ({
    workers: window.__fileSystemHold.workers,
    waiting: document.querySelector('#app .file-browser-btn')?.getAttribute('title') ?? null,
  }));
  assert(held.workers > 0 && held.waiting === 'Your files are still loading', `the file system was not held back (${held.workers} workers, New file says ${JSON.stringify(held.waiting)}), so this proves nothing`);
  await editor.click();
  await t.page.keyboard.down('Control');
  await t.page.keyboard.press('End');
  await t.page.keyboard.up('Control');
  await t.page.keyboard.type(MARK);
  await t.frames(2);
  const typed = await t.page.$eval('#app .studio-main > section textarea', (e) => e.value);
  assert(typed.endsWith(MARK), `typing into the editor gave ${JSON.stringify(typed.slice(-60))}`);
  await t.page.evaluate(() => window.__fileSystemHold.release());
  const answered = await t.page.waitForFunction(() => document.querySelector('#app .file-browser-btn')?.disabled === false, { timeout: 120000 }).then(() => true, () => false);
  assert(answered, 'the file system never answered once released, so this proves nothing');
  await t.frames(6);
  const after = await t.page.evaluate(() => ({
    title: document.querySelector('#app .studio-main > section .panel-header span')?.innerText.trim(),
    value: document.querySelector('#app .studio-main > section textarea')?.value ?? '',
  }));
  assert(after.value === typed, `the file system's answer replaced what was typed: the editor now ends ${JSON.stringify(after.value.slice(-60))}`);
  assert(after.title === 'Code Editor', `the Studio ended in ${JSON.stringify(after.title)}, not the Code Editor`);
});

// The examples come from the site itself, so the file tree, with the open file selected, is on the
// screen in the same frame as the file's text: "the editor holds text" means "the tree is there".
scenario('the_file_tree_shows_the_open_file_from_the_first_frame', async (t) => {
  await t.page.evaluateOnNewDocument(() => {
    const seen = (window.__treeAtText = { sampled: false, selected: null, rows: 0 });
    const sample = () => {
      if (seen.sampled) return;
      const editor = document.querySelector('#app .studio-main > section textarea');
      if (!editor || !editor.value.trim()) return;
      seen.sampled = true;
      seen.selected = document.querySelector('#app .file-tree-item.selected')?.getAttribute('title') ?? null;
      seen.rows = document.querySelectorAll('#app .file-tree-item').length;
    };
    new MutationObserver(sample).observe(document, { childList: true, subtree: true, characterData: true, attributes: true });
    const frame = () => { sample(); if (!seen.sampled) requestAnimationFrame(frame); };
    requestAnimationFrame(frame);
  });
  await t.page.setViewport(DESKTOP);
  await t.load(`${t.base}/studio`);
  await studioSettled(t);
  const seen = await t.page.evaluate(() => ({ ...window.__treeAtText, settled: document.querySelector('#app .file-tree-item.selected')?.getAttribute('title') }));
  assert(seen.sampled, 'the recorder never saw the editor hold text, so it proves nothing');
  assert(seen.rows > 0, 'the editor held its file before the file tree showed a single row');
  assert(seen.selected === seen.settled, `when the editor first held its file the tree had ${JSON.stringify(seen.selected)} selected, not ${JSON.stringify(seen.settled)}`);
});

// An example opens the moment it is clicked, from the site itself, even while the reader's own
// files are still loading.
scenario('an_example_opens_before_the_file_system_answers', async (t) => {
  const FIBONACCI = '/examples/code/algorithms/fibonacci.logos';
  await holdFileSystem(t.page);
  await t.page.setViewport(DESKTOP);
  await t.load(`${t.base}/studio`);
  const held = await t.page.evaluate(() => ({ workers: window.__fileSystemHold.workers, held: window.__fileSystemHold.held }));
  assert(held.workers > 0 && held.held, 'the file system was not held back, so this proves nothing');
  const row = await t.page.waitForSelector(`#app .file-tree-item[title="${FIBONACCI}"]`, { timeout: 30000 }).catch(() => null);
  assert(row, 'while the reader\'s files load, the file tree shows no examples to click');
  await row.click();
  await t.page.waitForFunction((path) => document.querySelector('#app .file-tree-item.selected')?.getAttribute('title') === path
    && (document.querySelector('#app .studio-main > section textarea')?.value || '').startsWith('# Fibonacci'), { timeout: 15000 }, FIBONACCI)
    .catch(() => {});
  const look = await t.page.evaluate(() => ({
    selected: document.querySelector('#app .file-tree-item.selected')?.getAttribute('title') ?? null,
    editor: (document.querySelector('#app .studio-main > section textarea')?.value || '').split('\n')[0],
    stillHeld: window.__fileSystemHold.held,
  }));
  assert(look.stillHeld, 'the file system was released before the check, so this proves nothing');
  assert(look.selected === FIBONACCI && look.editor === '# Fibonacci', `clicking ${FIBONACCI} while the files load opened ${JSON.stringify(look.selected)}, whose editor starts ${JSON.stringify(look.editor)}`);
  await t.page.evaluate(() => window.__fileSystemHold.release());
});

// New file writes to the reader's store, so until that store answers it stays in place, disabled,
// and says why; then it makes a file.
scenario('new_file_waits_for_the_file_system', async (t) => {
  const button = () => t.page.$eval('#app .file-browser-btn', (b) => ({ disabled: b.disabled, title: b.getAttribute('title') }));
  await holdFileSystem(t.page);
  await t.page.setViewport(DESKTOP);
  await t.load(`${t.base}/studio`);
  await t.page.waitForSelector('#app .file-browser-btn', { timeout: 30000 });
  const waiting = await button();
  assert(waiting.disabled && waiting.title === 'Your files are still loading', `while the files load, New file is ${JSON.stringify(waiting)}`);
  await t.page.evaluate(() => window.__fileSystemHold.release());
  await t.page.waitForFunction(() => !document.querySelector('#app .file-browser-btn')?.disabled, { timeout: 120000 }).catch(() => {});
  const ready = await button();
  assert(!ready.disabled && ready.title === 'New file', `once the files load, New file is ${JSON.stringify(ready)}`);
  await t.page.click('#app .file-browser-btn');
  const made = await t.page.waitForSelector('#app .file-tree-item[title^="/workspace/untitled"]', { timeout: 30000 }).then(() => true, () => false);
  assert(made, 'New file made no file in the workspace');
});

// The reader's files join the tree when their store answers, below the examples: no example row
// moves, and the open file's row is the same row, still selected.
scenario('a_workspace_file_joins_the_tree_without_moving_the_examples', async (t) => {
  await holdFileSystem(t.page);
  await t.page.setViewport(DESKTOP);
  // A first visit makes a workspace file.
  await t.load(`${t.base}/studio`);
  await t.page.evaluate(() => window.__fileSystemHold.release());
  await t.page.waitForFunction(() => !document.querySelector('#app .file-browser-btn')?.disabled, { timeout: 120000 });
  await t.page.click('#app .file-browser-btn');
  await t.page.waitForSelector('#app .file-tree-item[title^="/workspace/"]', { timeout: 30000 });
  // The next visit opens with the store held: the examples are there, the workspace not yet.
  await t.load(`${t.base}/studio`);
  const before = await t.page.evaluate(() => {
    const rows = [...document.querySelectorAll('#app .file-tree-item')];
    const selected = document.querySelector('#app .file-tree-item.selected');
    if (selected) selected.__studioMark = true;
    return {
      held: window.__fileSystemHold.held,
      workspace: rows.filter((r) => (r.getAttribute('title') || '').startsWith('/workspace')).length,
      tops: Object.fromEntries(rows.filter((r) => (r.getAttribute('title') || '').startsWith('/examples')).map((r) => [r.getAttribute('title'), r.getBoundingClientRect().top])),
      selected: selected?.getAttribute('title') ?? null,
    };
  });
  assert(before.held, 'the file system was not held back, so this proves nothing');
  assert(Object.keys(before.tops).length > 0, 'while the files load, the tree shows no examples');
  assert(before.workspace === 0, 'the workspace showed before its store answered');
  await t.page.evaluate(() => window.__fileSystemHold.release());
  await t.page.waitForSelector('#app .file-tree-item[title^="/workspace/"]', { timeout: 120000 });
  await t.frames(6);
  const after = await t.page.evaluate((titles) => {
    const top = (title) => document.querySelector(`#app .file-tree-item[title="${title}"]`)?.getBoundingClientRect().top ?? null;
    const selected = document.querySelector('#app .file-tree-item.selected');
    return { tops: Object.fromEntries(titles.map((title) => [title, top(title)])), selected: selected?.getAttribute('title') ?? null, same: !!selected?.__studioMark };
  }, Object.keys(before.tops));
  const moved = Object.keys(before.tops).filter((title) => after.tops[title] === null || Math.abs(after.tops[title] - before.tops[title]) > 0.5);
  assert(moved.length === 0, `${moved.length} example row(s) moved or went when the workspace joined, e.g. ${moved.slice(0, 3).join(', ')}`);
  assert(after.selected === before.selected, `the open file's row changed from ${before.selected} to ${after.selected}`);
  assert(after.same, 'the open file\'s row was drawn again when the workspace joined');
});

// Copies of the examples an earlier Studio wrote into the reader's storage never show, and go: the
// shipped text opens, a retired example has no row, and the reader's workspace file stays.
scenario('old_example_copies_are_retired_and_the_workspace_kept', async (t) => {
  await t.page.setViewport(DESKTOP);
  await t.page.goto(`${t.base}/robots.txt`, { waitUntil: 'load', timeout: 300000 });
  const planted = await t.page.evaluate(async () => {
    if (!navigator.storage?.getDirectory) return false;
    const root = await navigator.storage.getDirectory();
    const put = async (path, text) => {
      const parts = path.split('/');
      let dir = root;
      for (const part of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(part, { create: true });
      const file = await (await dir.getFileHandle(parts.at(-1), { create: true })).createWritable();
      await file.write(text);
      await file.close();
    };
    await put('examples/code/basics/control-flow.logos', 'an old copy');
    await put('examples/code/fibonacci.logos', 'retired');
    await put('workspace/keep.logos', 'Show 1.');
    return true;
  });
  assert(planted, 'this browser offers no OPFS, so the upgrade path cannot be checked here');
  await t.load(`${t.base}/studio`);
  await t.page.waitForSelector('#app .file-tree-item[title="/workspace/keep.logos"]', { timeout: 120000 });
  assert(!(await t.page.$('#app .file-browser-note')), 'the Studio fell back to IndexedDB, so it never saw the copies in OPFS');
  const shown = await t.page.evaluate(() => ({
    editor: (document.querySelector('#app .studio-main > section textarea')?.value || '').split('\n')[0],
    retired: !!document.querySelector('#app .file-tree-item[title="/examples/code/fibonacci.logos"]'),
  }));
  assert(shown.editor === '# Control Flow', `the Studio opened an old copy: its editor starts ${JSON.stringify(shown.editor)}`);
  assert(!shown.retired, 'a retired example\'s old copy shows in the tree');
  const topLevel = () => t.page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const names = [];
    for await (const name of root.keys()) names.push(name);
    return names;
  });
  let left = await topLevel();
  for (const until = Date.now() + 60000; left.includes('examples') && Date.now() < until;) {
    await new Promise((r) => setTimeout(r, 500));
    left = await topLevel();
  }
  assert(!left.includes('examples'), `the old copies of the examples are still in the reader's storage (it holds ${left.join(', ')})`);
  const kept = await t.page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const file = await (await (await root.getDirectoryHandle('workspace')).getFileHandle('keep.logos')).getFile();
    return file.text();
  });
  assert(kept === 'Show 1.', `the reader's workspace file now reads ${JSON.stringify(kept)}`);
});

// The file browser's toggle is one square button in drawn icons: a files icon while the tree is
// hidden, an arrow folding left into a line while it shows — the same size in either state.
scenario('the_file_browser_toggle_is_one_square_button', async (t) => {
  const look = () => t.page.$eval('.sidebar-toggle-btn', (b) => {
    const r = b.getBoundingClientRect();
    const sidebar = document.querySelector('.studio-sidebar');
    return {
      w: r.width, h: r.height,
      icon: b.querySelector('svg')?.dataset.icon || null,
      text: b.innerText.trim(),
      label: b.getAttribute('aria-label'),
      expanded: b.getAttribute('aria-expanded'),
      shown: !!sidebar && getComputedStyle(sidebar).display !== 'none',
    };
  });
  for (const viewport of [DESKTOP, PHONE]) {
    await t.open(FILES.code, viewport);
    const before = await look();
    await t.page.click('.sidebar-toggle-btn');
    await t.frames(4);
    const after = await look();
    const at = `at ${viewport.width}px`;
    assert(before.shown !== after.shown, `${at} pressing the toggle does not show or hide the file tree`);
    for (const b of [before, after]) {
      const state = b.shown ? 'shown' : 'hidden';
      assert(near(b.w, b.h, 0.5), `${at} the toggle is ${px(b.w)} × ${px(b.h)}, not square`);
      assert(b.text === '', `${at} the toggle shows the text ${JSON.stringify(b.text)}, not a drawn icon`);
      assert(b.icon === (b.shown ? 'collapse' : 'files'), `${at} with the file tree ${state} the toggle shows the ${JSON.stringify(b.icon)} icon`);
      assert(b.label === (b.shown ? 'Hide files' : 'Show files'), `${at} with the file tree ${state} the toggle is labelled ${JSON.stringify(b.label)}`);
      assert(b.expanded === String(b.shown), `${at} with the file tree ${state} the toggle says aria-expanded=${b.expanded}`);
    }
    assert(near(before.w, after.w, 0.5) && near(before.h, after.h, 0.5), `${at} the toggle changes size: ${px(before.w)} × ${px(before.h)} → ${px(after.w)} × ${px(after.h)}`);
  }
});

scenario('comments_read_clearly', async (t) => {
  await t.open(FILES.code);
  const c = await t.page.evaluate(() => {
    const span = document.querySelector('.studio-main > section .tok-comment');
    if (!span) return null;
    const rgb = (s) => (s.match(/[\d.]+/g) || []).map(Number);
    const backdrop = (el) => {
      for (; el; el = el.parentElement) {
        const c = rgb(getComputedStyle(el).backgroundColor);
        if (c.length === 3 || (c.length === 4 && c[3] > 0)) return c.slice(0, 3);
      }
      return [0, 0, 0];
    };
    const lum = ([r, g, b]) => {
      const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
    };
    const fg = rgb(getComputedStyle(span).color).slice(0, 3);
    const bg = backdrop(span);
    const [hi, lo] = [lum(fg), lum(bg)].sort((a, b) => b - a);
    return { ratio: (hi + 0.05) / (lo + 0.05), fg: fg.join(','), bg: bg.join(',') };
  });
  assert(c, 'the Fibonacci example shows no comment token');
  assert(c.ratio >= 4.5, `comments read at ${c.ratio.toFixed(2)}:1 against the editor, under the 4.5:1 a reader needs (rgb ${c.fg} on rgb ${c.bg})`);
});

scenario('the_files_header_matches_the_panel_titles', async (t) => {
  await t.open(FILES.code);
  const look = await t.page.evaluate(() => {
    const style = (el) => {
      const s = getComputedStyle(el);
      return { family: s.fontFamily, size: s.fontSize, weight: s.fontWeight, spacing: s.letterSpacing, color: s.color, transform: s.textTransform };
    };
    const files = document.querySelector('.file-browser-title');
    const title = document.querySelector('.studio-panel .panel-header span');
    return { text: files.innerText.trim(), files: style(files), panel: style(title) };
  });
  assert(look.text === 'Files', `the file browser header reads ${JSON.stringify(look.text)}, not "Files"`);
  for (const key of Object.keys(look.panel)) {
    assert(look.files[key] === look.panel[key], `the Files header's ${key} is ${look.files[key]}; the panel titles use ${look.panel[key]}`);
  }
});

scenario('the_action_buttons_sit_centered_in_the_toolbar', async (t) => {
  for (const file of [FILES.code, FILES.threePanels]) {
    await t.open(file);
    const m = await t.page.evaluate(() => {
      const bar = document.querySelector('.studio-toolbar');
      const mid = bar.getBoundingClientRect().top + bar.clientTop + bar.clientHeight / 2;
      const buttons = [...document.querySelectorAll('.studio-toolbar-right .execute-btn')]
        .filter((b) => getComputedStyle(b).display !== 'none')
        .map((b) => {
          const r = b.getBoundingClientRect();
          const label = [...b.querySelectorAll('.desktop-label')].find((l) => getComputedStyle(l).display !== 'none');
          const lr = label.getBoundingClientRect();
          return { name: label.innerText.trim(), height: r.height, center: r.top + r.height / 2, labelCenter: lr.top + lr.height / 2 };
        });
      return { mid, buttons };
    });
    assert(m.buttons.length >= 2, `${file}: expected the action buttons in the toolbar, found ${m.buttons.length}`);
    for (const b of m.buttons) {
      assert(near(b.center, m.mid, 0.5), `${file}: ${b.name} sits at ${px(b.center)}, off the toolbar's middle ${px(m.mid)}`);
      assert(near(b.labelCenter, b.center, 1), `${file}: ${b.name}'s label sits ${px(b.labelCenter - b.center)} off its button's middle`);
    }
    const heights = [...new Set(m.buttons.map((b) => b.height.toFixed(1)))];
    assert(heights.length === 1, `${file}: the action buttons differ in height: ${heights.join(', ')}px`);
  }
});

// Replace the first editor's whole text at once, as a paste does — for programs too long to type.
async function paste(t, text) {
  await t.page.evaluate((text) => {
    const editor = document.querySelector('.studio-main > section textarea');
    editor.value = text;
    editor.dispatchEvent(new Event('input', { bubbles: true }));
  }, text);
  await t.frames(2);
}

// Record every node the toolbar gains or loses, and every text change inside it, from now on.
function watchToolbar(page) {
  return page.evaluate(() => {
    window.__toolbarChanges = [];
    const name = (n) => (n.textContent || n.nodeName).trim().slice(0, 40);
    new MutationObserver((records) => {
      for (const r of records) {
        for (const n of r.addedNodes) window.__toolbarChanges.push(`+${name(n)}`);
        for (const n of r.removedNodes) window.__toolbarChanges.push(`-${name(n)}`);
        if (r.type === 'characterData') window.__toolbarChanges.push(`~${name(r.target)}`);
      }
    }).observe(document.querySelector('.studio-toolbar'), { subtree: true, childList: true, characterData: true });
  });
}

// Execute and Compile leave the toolbar exactly as it was. A purple "Working…" button once
// appeared beside the buttons for the length of each run, so pressing again and again saw it flash
// and the row jump; nothing may appear there while the Studio works.
scenario('the_toolbar_holds_still_while_the_studio_works', async (t) => {
  const runs = [
    [FILES.math, ['Execute', 'Execute', 'Execute', 'Compile', 'Execute', 'Compile', 'Execute']],
    [FILES.logic, ['Execute', 'Compile', 'Execute', 'Execute', 'Compile']],
    [FILES.sentences, ['Execute', 'Execute', 'Compile']],
    [FILES.code, ['Compile', 'Compile']],
  ];
  for (const [file, presses] of runs) {
    await t.open(file);
    await watchToolbar(t.page);
    for (const label of presses) {
      await press(t.page, label);
      await t.frames(1);
    }
    await new Promise((r) => setTimeout(r, 1500));
    await t.frames(3);
    const changes = await t.page.evaluate(() => window.__toolbarChanges);
    assert(changes.length === 0, `${file}: the toolbar changed while the Studio worked: ${changes.slice(0, 8).join(', ')}`);
  }
});

// The Math Context panel as it reads: each section's title and the names under it, in order.
function readContext(page) {
  return page.evaluate(() => [...document.querySelectorAll('.context-view .context-section')].map((s) => ({
    title: s.querySelector('.context-section-header')?.innerText.trim() || '',
    names: [...s.querySelectorAll('.context-item-name')].map((n) => n.innerText.trim()),
  })));
}

const contextSummary = (sections) => sections.map((s) => `${s.title}: ${s.names.slice(0, 8).join(' ')}${s.names.length > 8 ? ' …' : ''}`).join(' | ');

// Math's Context panel lists the environment the last Execute built. Executing the same program
// again builds the same environment, so the panel holds still — the same entries in the same
// order at the same scroll — for a reader working down it. It once reshuffled on every press.
scenario('the_math_context_holds_still_across_executes', async (t) => {
  await t.open(FILES.math);
  await press(t.page, 'Execute');
  await settle(t.page, (s) => s.replLines > 0, 20000);
  await t.frames(3);
  const first = await readContext(t.page);
  const inductives = first.find((s) => /inductive/i.test(s.title));
  assert(inductives && inductives.names.length > 3, `the Context panel lists no inductive types to scroll through: ${contextSummary(first)}`);
  const scrolled = await t.page.evaluate(() => {
    const list = document.querySelector('.context-view .context-sections');
    list.scrollTop = Math.floor((list.scrollHeight - list.clientHeight) / 2);
    window.__contextChanges = 0;
    new MutationObserver((records) => { window.__contextChanges += records.length; })
      .observe(document.querySelector('.context-view'), { subtree: true, childList: true, characterData: true });
    return list.scrollTop;
  });
  for (let run = 2; run <= 7; run++) {
    await press(t.page, 'Execute');
    await new Promise((r) => setTimeout(r, 400));
    await t.frames(3);
    const now = await readContext(t.page);
    assert(JSON.stringify(now) === JSON.stringify(first), `Execute #${run} reordered the Context panel:\n  first: ${contextSummary(first)}\n  now:   ${contextSummary(now)}`);
  }
  const after = await t.page.evaluate(() => ({
    top: document.querySelector('.context-view .context-sections').scrollTop,
    changes: window.__contextChanges,
  }));
  assert(after.top === scrolled, `the Context panel's scroll moved from ${scrolled}px to ${after.top}px`);
  assert(after.changes === 0, `the Context panel's DOM changed ${after.changes} time(s) while the same program ran again`);
});

// A Math program the kernel cannot survive: a `## Theory` development's formula parser takes one
// level of its own stack for each parenthesis and has no depth limit, so an axiom nested this deep
// runs the wasm stack out. (The kernel's term parser refuses deep terms since it gained its depth
// limit. Should the formula parser learn to refuse them too, choose another input the Math path
// cannot survive.)
const TOO_DEEP_FOR_THE_KERNEL = `## Theory Deep\n\nAxiom a: ${'('.repeat(20000)}P${')'.repeat(20000)}.\nTheorem t: prove P.\n`;
const SOUND_MATH = 'Inductive Coin := heads : Coin | tails : Coin.\nCheck heads.\n';

// The toolbar's fault note, measured against the Execute button beside it.
function readFaultNote(page) {
  return page.evaluate(() => {
    const note = document.querySelector('.studio-toolbar [role="alert"]');
    if (!note) return null;
    const execute = [...document.querySelectorAll('.studio-toolbar button')].find((b) => /Execute/.test(b.textContent));
    const r = note.getBoundingClientRect();
    const e = execute.getBoundingClientRect();
    return {
      text: note.innerText.trim(),
      title: note.getAttribute('title') || '',
      color: getComputedStyle(note).color,
      left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width,
      execute: { left: e.left, right: e.right, top: e.top, bottom: e.bottom },
      buttons: [...document.querySelectorAll('.studio-toolbar button')].map((b) => b.getBoundingClientRect()).map((b) => ({ left: b.left, right: b.right, top: b.top, bottom: b.bottom })),
      viewport: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
    };
  });
}

// A failure nobody planned for is an orange note in the toolbar, just left of the buttons, and the
// Studio keeps working: the next sound program executes, and a clean run takes the note away. It
// once left the whole page dead — every later press trapped on the same exhausted stack.
scenario('an_unexpected_fault_is_an_orange_note_beside_the_buttons', async (t) => {
  await t.open(FILES.math);
  await paste(t, TOO_DEEP_FOR_THE_KERNEL);
  await press(t.page, 'Execute');
  const shown = await t.page.waitForSelector('.studio-toolbar [role="alert"]', { timeout: 30000 }).then(() => true, () => false);
  assert(shown, 'Execute died on a program the kernel cannot survive and the toolbar said nothing');
  const note = await readFaultNote(t.page);
  assert(/^\W*Execute failed/.test(note.text), `the note does not say Execute failed: ${JSON.stringify(note.text)}`);
  assert(/stack/.test(note.text), `the note does not say the kernel ran out of stack: ${JSON.stringify(note.text)}`);
  assert(note.color === 'rgb(245, 158, 11)', `the note is not the site's warning orange: ${note.color}`);
  assert(note.right <= note.execute.left + 0.5, `the note is not left of Execute: it ends at ${px(note.right)}, Execute starts at ${px(note.execute.left)}`);
  assert(note.bottom > note.execute.top && note.top < note.execute.bottom, 'the note is not on the buttons\' row');
  assert(note.left >= 0, `the note runs off the window's left edge (${px(note.left)})`);
  assert(/still running/.test(note.title), `the note's tooltip does not say the Studio is still running: ${JSON.stringify(note.title)}`);

  await paste(t, SOUND_MATH);
  await press(t.page, 'Execute');
  const seen = await settle(t.page, (s) => panelText(s, 'Output').includes('heads : Coin'), 20000);
  assert(panelText(seen, 'Output').includes('heads : Coin'), `after the fault a sound program printed ${JSON.stringify(panelText(seen, 'Output').slice(0, 160))}`);
  await t.frames(3);
  assert(!(await readFaultNote(t.page)), 'the note stayed after a clean run');
});

// The same fault, pressed again and again, keeps one steady note: nothing flashes in the toolbar.
scenario('a_repeated_fault_keeps_one_steady_note', async (t) => {
  await t.open(FILES.math);
  await paste(t, TOO_DEEP_FOR_THE_KERNEL);
  await press(t.page, 'Execute');
  const shown = await t.page.waitForSelector('.studio-toolbar [role="alert"]', { timeout: 30000 }).then(() => true, () => false);
  assert(shown, 'Execute died on a program the kernel cannot survive and the toolbar said nothing');
  await watchToolbar(t.page);
  for (let run = 0; run < 4; run++) {
    await press(t.page, 'Execute');
    await new Promise((r) => setTimeout(r, 1500));
  }
  await t.frames(3);
  const changes = await t.page.evaluate(() => window.__toolbarChanges);
  assert(changes.length === 0, `the note flashed while the same fault repeated: ${changes.slice(0, 8).join(', ')}`);
  assert(await readFaultNote(t.page), 'the note went away while the fault kept happening');
});

// A Logic theorem whose proof script the tactic parser cannot survive: it takes a level of its own
// stack for each parenthesis, with no depth limit. (Should it learn to refuse such a script, choose
// another the prover cannot survive.)
const TOO_DEEP_FOR_THE_PROVER = `## Theorem: Deep_Script
Given (h): Bob is happy.
Prove: Bob is happy.
Proof: ${'('.repeat(20000)}assumption${')'.repeat(20000)}.
`;

// The Logic page fails the same way: the prover's fault is the same orange note, and the page
// still proves the next theorem.
scenario('a_logic_fault_is_the_same_orange_note', async (t) => {
  await t.open(FILES.logic);
  await paste(t, TOO_DEEP_FOR_THE_PROVER);
  await press(t.page, 'Execute');
  const shown = await t.page.waitForSelector('.studio-toolbar [role="alert"]', { timeout: 30000 }).then(() => true, () => false);
  assert(shown, 'Execute died on a proof script the prover cannot survive and the toolbar said nothing');
  const note = await readFaultNote(t.page);
  assert(/^\W*Execute failed/.test(note.text) && /stack/.test(note.text), `the note does not say Execute ran out of stack: ${JSON.stringify(note.text)}`);
  assert(note.color === 'rgb(245, 158, 11)', `the note is not the site's warning orange: ${note.color}`);
  assert(note.right <= note.execute.left + 0.5, `the note is not left of Execute: it ends at ${px(note.right)}, Execute starts at ${px(note.execute.left)}`);

  await paste(t, '## Theorem: Again\nGiven: All men are mortal.\nGiven: Plato is a man.\nProve: Plato is mortal.\nProof: Auto.\n');
  await press(t.page, 'Execute');
  const seen = await settle(t.page, (s) => panelText(s, 'English Input').includes('Proof found'), 20000);
  assert(panelText(seen, 'English Input').includes('Proof found'), 'after the fault the next theorem was not proved');
  await t.frames(3);
  assert(!(await readFaultNote(t.page)), 'the note stayed after a clean run');
});

// Compile's Rust for a theorem with a couple of thousand premises is some forty thousand lines of
// highlighted code: the view shows it all, and the page keeps answering. Past 65,535 tokens the
// highlighter once overflowed dioxus's 16-bit node count and the whole Studio stopped.
scenario('a_huge_compile_output_renders_whole', async (t) => {
  const word = (i) => `wug${[...String(i)].map((d) => 'abcdefghij'[d]).join('')}`;
  const premises = Array.from({ length: 2000 }, (_, i) => `Given: Every ${word(i)} is a ${word(i + 1)}.`);
  await t.open(FILES.logic);
  await paste(t, ['## Theorem: Long_Chain', ...premises, `Given: Socrates is a ${word(0)}.`, `Prove: Socrates is a ${word(2000)}.`, 'Proof: Auto.', ''].join('\n'));
  await press(t.page, 'Compile');
  const seen = await settle(t.page, (s) => panelText(s, 'Logic Output').includes(word(1999)), 30000);
  const rust = panelText(seen, 'Logic Output');
  assert(rust.includes(word(0)) && rust.includes(word(1999)), `the Rust view does not show the whole output (${rust.length} characters)`);

  await paste(t, 'Every philosopher thinks.\n');
  await press(t.page, 'Execute');
  const after = await settle(t.page, (s) => panelText(s, 'Logic Output').includes('∀'), 20000);
  assert(panelText(after, 'Logic Output').includes('∀'), 'after the huge output the Studio no longer executes');
});

// Each column's body — below its header — as a clip, with its name. The Web preview is the user's
// own page in an iframe and paints its own colours, so it is left out.
function columnBodies(page) {
  return page.evaluate(() => {
    const shown = (el) => el && getComputedStyle(el).display !== 'none' && el.getBoundingClientRect().width > 0;
    const out = [];
    const sidebar = document.querySelector('.studio-sidebar');
    if (shown(sidebar)) {
      const header = sidebar.querySelector('.file-browser-header');
      const r = sidebar.getBoundingClientRect();
      const top = header ? header.getBoundingClientRect().bottom : r.top;
      out.push({ name: 'Files', clip: { x: r.left, y: top, width: r.width, height: r.bottom - top } });
    }
    for (const panel of document.querySelectorAll('.studio-main > section, .studio-main > aside')) {
      if (!shown(panel)) continue;
      const title = panel.querySelector('.panel-header span')?.innerText.trim() || panel.className;
      for (const body of panel.querySelectorAll(':scope > .panel-content')) {
        if (body.querySelector('iframe')) continue;
        const r = body.getBoundingClientRect();
        if (r.width < 40 || r.height < 40) continue;
        out.push({ name: title, clip: { x: r.left, y: r.top, width: r.width, height: r.height } });
      }
    }
    return out;
  });
}

// The colour that covers most of a clip, as the page painted it.
async function dominantColour(page, clip) {
  const png = await page.screenshot({ clip, encoding: 'base64' });
  return page.evaluate(async (src) => {
    const img = new Image();
    img.src = src;
    await img.decode();
    const canvas = document.createElement('canvas');
    canvas.width = img.width;
    canvas.height = img.height;
    const g = canvas.getContext('2d');
    g.drawImage(img, 0, 0);
    const d = g.getImageData(0, 0, canvas.width, canvas.height).data;
    const counts = new Map();
    for (let i = 0; i < d.length; i += 4) {
      const key = `rgb(${d[i]},${d[i + 1]},${d[i + 2]})`;
      counts.set(key, (counts.get(key) || 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1])[0][0];
  }, `data:image/png;base64,${png}`);
}

// Every column is one surface. The editor painted the Studio's dark while the Files and Context
// columns and most output panels painted a lighter grey beside it.
scenario('every_column_paints_the_editors_surface', async (t) => {
  const off = [];
  for (const [kind, file] of EVERY_KIND) {
    await t.open(file);
    await t.frames(4);
    for (const column of await columnBodies(t.page)) {
      const colour = await dominantColour(t.page, column.clip);
      if (colour !== SURFACE) off.push(`${kind} · ${column.name}: ${colour}`);
    }
  }
  assert(off.length === 0, `${off.length} column(s) paint another surface than the editor's ${SURFACE}:\n${off.join('\n')}`);
});

// Where each column's content sits against the column's own edges: the leftmost and topmost
// painted content (text line boxes, icons, pictures) at the top of its scroll, and the rightmost.
function contentInsets(page) {
  return page.evaluate(() => {
    const shown = (el) => el && getComputedStyle(el).display !== 'none' && el.getBoundingClientRect().width > 0;
    const regions = [];
    const sidebar = document.querySelector('.studio-sidebar');
    if (shown(sidebar)) {
      const header = sidebar.querySelector('.file-browser-header');
      const body = header ? header.nextElementSibling : sidebar;
      if (body) regions.push({ name: 'Files', body });
    }
    for (const panel of document.querySelectorAll('.studio-main > section, .studio-main > aside')) {
      if (!shown(panel)) continue;
      const title = panel.querySelector('.panel-header span')?.innerText.trim() || panel.className;
      for (const body of panel.querySelectorAll(':scope > .panel-content')) {
        if (!body.querySelector('iframe')) regions.push({ name: title, body });
      }
    }
    // Text is measured by the box that holds it: across, by its glyphs; down, by the content-box
    // top of its nearest block (a glyph sits inside its line by the half-leading, and a row's text
    // is centred in the row, so a glyph's top is never the padding the reader sees).
    const blockOf = (node, body) => {
      let el = node.parentElement;
      while (el && el !== body && getComputedStyle(el).display.startsWith('inline')) el = el.parentElement;
      return el || body;
    };
    // The right edge past which an ancestor clips its overflow — an ellipsised name's hidden tail
    // is not content.
    const clipRight = (el, body) => {
      let edge = Infinity;
      for (let n = el; n && n !== body.parentElement; n = n.parentElement) {
        if (getComputedStyle(n).overflowX !== 'visible') {
          const cs = getComputedStyle(n);
          edge = Math.min(edge, n.getBoundingClientRect().right - parseFloat(cs.borderRightWidth) - parseFloat(cs.paddingRight));
        }
      }
      return edge;
    };
    const contentTop = (el) => {
      const cs = getComputedStyle(el);
      return el.getBoundingClientRect().top + parseFloat(cs.borderTopWidth) + parseFloat(cs.paddingTop);
    };
    return regions.map(({ name, body }) => {
      for (const el of [body, ...body.querySelectorAll('*')]) if (el.scrollTop) el.scrollTop = 0;
      const b = body.getBoundingClientRect();
      const boxes = [];
      const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
      while (walker.nextNode()) {
        const node = walker.currentNode;
        if (!node.textContent.trim() || !node.parentElement || getComputedStyle(node.parentElement).visibility === 'hidden') continue;
        if (node.parentElement.closest('textarea')) continue;
        const range = document.createRange();
        range.selectNodeContents(node);
        const top = contentTop(blockOf(node, body));
        const clip = clipRight(node.parentElement, body);
        for (const r of range.getClientRects()) if (r.width > 0 && r.bottom > b.top && r.top < b.bottom) boxes.push({ left: r.left, right: Math.min(r.right, clip), top });
      }
      for (const el of body.querySelectorAll('svg, img, canvas')) {
        if (el.parentElement && el.parentElement.closest('svg')) continue;
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0 && r.bottom > b.top && r.top < b.bottom) boxes.push(r);
      }
      if (!boxes.length) return { name, empty: true };
      const scrollsSideways = [body, ...body.querySelectorAll('*')].some((el) => el.scrollWidth > el.clientWidth + 1 && ['auto', 'scroll'].includes(getComputedStyle(el).overflowX));
      return {
        name,
        left: Math.min(...boxes.map((r) => r.left)) - b.left,
        top: Math.min(...boxes.map((r) => r.top)) - b.top,
        right: scrollsSideways ? null : b.right - Math.max(...boxes.map((r) => r.right)),
      };
    });
  });
}

// Every column holds its content twenty pixels from its edges — the editor, the outputs, the
// Context and Files lists alike. They sat 12 to 16 pixels in, each by a rule of its own.
scenario('every_column_holds_its_content_twenty_pixels_in', async (t) => {
  const off = [];
  for (const [kind, file] of EVERY_KIND) {
    await t.open(file);
    if (kind === 'Math') {
      await press(t.page, 'Execute');
      await settle(t.page, (s) => s.replLines > 0, 20000);
    }
    await t.frames(4);
    for (const inset of await contentInsets(t.page)) {
      if (inset.empty) continue;
      // Within a glyph's side-bearing or an icon's own padding of the inset: 3px.
      if (!near(inset.left, INSET, 3)) off.push(`${kind} · ${inset.name}: content starts ${px(inset.left)} from the left edge`);
      if (!near(inset.top, INSET, 3)) off.push(`${kind} · ${inset.name}: content starts ${px(inset.top)} below the top edge`);
      if (inset.right !== null && inset.right < INSET - 3) off.push(`${kind} · ${inset.name}: content runs to ${px(inset.right)} from the right edge`);
    }
  }
  assert(off.length === 0, `${off.length} inset(s) are not ${INSET}px:\n${off.join('\n')}`);
});

// Every scroll area whose content ends before its scroll does: the empty run past its last
// painted line, beyond the twenty pixels of padding every column keeps. The existing scroll check
// subtracts a region's own padding, so an editor's `padding-bottom: 50%` "scroll past end" never
// counted; here nothing past the standard inset is allowed.
function emptyScrollFindings(page) {
  return page.evaluate((inset) => {
    const out = [];
    const scrolls = (v) => v === 'auto' || v === 'scroll';
    const areas = document.querySelectorAll('.studio-main > section, .studio-main > aside, .studio-sidebar');
    for (const panel of areas) {
      const title = panel.querySelector('.panel-header span, .file-browser-title')?.innerText.trim() || panel.className;
      for (const el of [panel, ...panel.querySelectorAll('*')]) {
        const cs = getComputedStyle(el);
        const r = el.getBoundingClientRect();
        if (r.width === 0 || r.height === 0 || !scrolls(cs.overflowY) || el.tagName === 'TEXTAREA') continue;
        if (el.scrollHeight - el.clientHeight <= 1) continue;
        let bottom = 0;
        const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
        while (walker.nextNode()) {
          if (!walker.currentNode.textContent.trim()) continue;
          const range = document.createRange();
          range.selectNodeContents(walker.currentNode);
          // A line reaches the bottom of its line box: the glyphs and the half-leading below them.
          const lh = parseFloat(getComputedStyle(walker.currentNode.parentElement).lineHeight);
          for (const c of range.getClientRects()) {
            if (c.height === 0) continue;
            const leading = Number.isFinite(lh) && lh > c.height ? (lh - c.height) / 2 : 0;
            bottom = Math.max(bottom, c.bottom + leading - r.top + el.scrollTop);
          }
        }
        for (const child of el.querySelectorAll('svg, img, canvas, iframe')) {
          const c = child.getBoundingClientRect();
          if (c.height > 0) bottom = Math.max(bottom, c.bottom - r.top + el.scrollTop);
        }
        // Past the inset, a last item may keep its own few pixels of padding; what the audit hunts is a
        // run of nothing — the tens and hundreds of pixels a stray rule leaves to scroll into.
        const dead = el.scrollHeight - bottom - inset;
        if (dead > 8) out.push(`${title} <${el.tagName.toLowerCase()} class="${el.getAttribute('class') || ''}">: ${Math.round(dead)}px of empty space to scroll into`);
      }
    }
    return out;
  }, INSET);
}

// The bulk audit: every example of every kind, opened and run as a reader would, and every scroll
// area in its columns checked for empty space past its last line. One line per finding.
scenario('no_example_scrolls_into_empty_space', async (t) => {
  const found = [];
  for (const example of studioContract()) {
    try {
      await t.fresh();
      await t.openWithin(example.file, 90000);
      const check = PROMISES[example.promise.kind];
      if (check) await check(t, example.promise);
      await t.frames(3);
      for (const f of await emptyScrollFindings(t.page)) found.push(`${example.file}: ${f}`);
    } catch (e) {
      found.push(`${example.file}: ${String(e.message || e).split('\n')[0]}`);
    }
  }
  assert(found.length === 0, `${found.length} scroll area(s) scroll into empty space:\n${found.join('\n')}`);
}, { allowErrors: NAVIGATION_ABORT, timeout: 60000 + studioContract().length * 30000 });

// With the file browser folded, a muted "Files" label sits beside its toggle, set like the
// Execute and Compile text, so the square button says what it opens. Unfolded, the browser's own
// header says it, and the label goes.
scenario('a_folded_file_browser_leaves_a_files_label', async (t) => {
  const read = () => t.page.evaluate(() => {
    const toggle = document.querySelector('.sidebar-toggle-btn');
    const bar = document.querySelector('.studio-toolbar-left');
    const label = [...bar.querySelectorAll('*')].find((el) => el !== toggle && !toggle.contains(el) && el.children.length === 0 && el.textContent.trim() === 'Files' && getComputedStyle(el).display !== 'none');
    const execute = [...document.querySelectorAll('.studio-toolbar .execute-btn')].find((b) => /Execute|Run/.test(b.textContent));
    const executeText = execute && [...execute.querySelectorAll('span')].find((s) => getComputedStyle(s).display !== 'none');
    const t = toggle.getBoundingClientRect();
    const l = label?.getBoundingClientRect();
    const sidebar = document.querySelector('.studio-sidebar');
    return {
      open: !!sidebar && getComputedStyle(sidebar).display !== 'none' && sidebar.getBoundingClientRect().width > 0,
      label: label ? { left: l.left, right: l.right, mid: l.top + l.height / 2, size: getComputedStyle(label).fontSize, colour: getComputedStyle(label).color } : null,
      toggle: { right: t.right, mid: t.top + t.height / 2 },
      executeSize: executeText ? getComputedStyle(executeText).fontSize : null,
      viewport: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
    };
  });
  for (const viewport of [DESKTOP, PHONE]) {
    await t.open(FILES.code, viewport);
    let now = await read();
    if (now.open) {
      assert(!now.label, `${viewport.width}px: with the files open the toolbar still shows a Files label`);
      await (await t.page.$('.sidebar-toggle-btn')).click();
      await t.frames(3);
      now = await read();
    }
    assert(!now.open, `${viewport.width}px: the file browser did not fold`);
    assert(now.label, `${viewport.width}px: with the files folded there is no "Files" label beside the toggle`);
    assert(now.label.left >= now.toggle.right && now.label.left - now.toggle.right <= 16, `${viewport.width}px: the label is not beside the toggle (${px(now.label.left - now.toggle.right)} away)`);
    assert(near(now.label.mid, now.toggle.mid, 1), `${viewport.width}px: the label is not centred on the toggle`);
    assert(now.label.size === now.executeSize, `${viewport.width}px: the label is ${now.label.size}, the Execute text ${now.executeSize}`);
    assert(now.label.colour === 'rgb(107, 114, 128)', `${viewport.width}px: the label is not the Studio's muted grey: ${now.label.colour}`);
    assert(now.scrollWidth <= now.viewport, `${viewport.width}px: the label makes the page scroll sideways`);
    await (await t.page.$('.sidebar-toggle-btn')).click();
    await t.frames(3);
    const reopened = await read();
    assert(reopened.open && !reopened.label, `${viewport.width}px: unfolding the files did not take the label away`);
  }
});

// On a phone the buttons take a row of their own; the note joins them without pushing anything
// off screen or covering a button.
scenario('the_fault_note_fits_a_phone', async (t) => {
  await t.open(FILES.math, PHONE);
  await paste(t, TOO_DEEP_FOR_THE_KERNEL);
  await press(t.page, 'Execute');
  const shown = await t.page.waitForSelector('.studio-toolbar [role="alert"]', { visible: true, timeout: 30000 }).then(() => true, () => false);
  assert(shown, 'on a phone, Execute died on a program the kernel cannot survive and the toolbar said nothing');
  const note = await readFaultNote(t.page);
  assert(note.scrollWidth <= note.viewport, `the note makes the page scroll sideways (${note.scrollWidth}px in ${note.viewport}px)`);
  assert(note.left >= 0 && note.right <= note.viewport, `the note runs off the screen (${px(note.left)}–${px(note.right)} of ${note.viewport}px)`);
  for (const b of note.buttons) {
    const overlaps = note.left < b.right && b.left < note.right && note.top < b.bottom && b.top < note.bottom;
    assert(!overlaps, 'the note covers a toolbar button');
  }
});

scenario('hovering_code_shows_a_vscode_style_hint', async (t) => {
  await t.open(FILES.code);
  const token = async (text) => {
    const handle = await t.page.evaluateHandle((word) =>
      [...document.querySelectorAll('.studio-main > section .code-editor-highlight span')].find((s) => s.textContent === word), text);
    const el = handle.asElement();
    assert(el, `the editor shows no "${text}" token`);
    return el;
  };
  const hintText = () => t.page.evaluate(() => {
    const h = document.querySelector('.hover-hint');
    return h && getComputedStyle(h).display !== 'none' ? h.innerText : null;
  });
  const rest = async (el) => {
    const box = await el.boundingBox();
    await t.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 4 });
  };

  await rest(await token('Let'));
  await t.page.waitForFunction(() => document.querySelector('.hover-hint'), { timeout: 3000 }).catch(() => {});
  const keyword = await hintText();
  assert(keyword && /variable/i.test(keyword), `resting on "Let" shows ${JSON.stringify(keyword)}, not the language server's hover`);
  const placed = await t.page.$eval('.hover-hint', (h) => {
    const r = h.getBoundingClientRect();
    return { fixed: getComputedStyle(h).position === 'fixed', inside: r.left >= 0 && r.top >= 0 && r.right <= innerWidth && r.bottom <= innerHeight };
  });
  assert(placed.fixed && placed.inside, `the hint is not a fixed overlay inside the window: ${JSON.stringify(placed)}`);

  const header = await t.page.$('.studio-main > section .panel-header');
  const hb = await header.boundingBox();
  await t.page.mouse.move(hb.x + 10, hb.y + hb.height / 2, { steps: 4 });
  const gone = await t.page.waitForFunction(() => !document.querySelector('.hover-hint'), { timeout: 1500 }).then(() => true, () => false);
  assert(gone, 'the hint stays up after the pointer leaves the code');
});

// A hover card is compact: its parts — a problem and where it comes from, the code it
// describes, the explanation — sit one even gap apart, with the same small padding above the
// first and below the last, and no empty space anywhere else.
scenario('the_hint_spaces_its_parts_evenly', async (t) => {
  await t.open(FILES.code);
  await paste(t, '## Main\nLet total be 1.\nShow 2.\n');
  const flagged = await t.page.waitForFunction(() => [...document.querySelectorAll('.studio-main > section .code-editor-highlight span')]
    .some((s) => s.textContent === 'total' && /\bsq-/.test(s.className)), { timeout: 8000 }).then(() => true, () => false);
  assert(flagged, 'the language server never flagged the unused "total"');
  const handle = await t.page.evaluateHandle(() =>
    [...document.querySelectorAll('.studio-main > section .code-editor-highlight span')].find((s) => s.textContent === 'total'));
  const box = await handle.asElement().boundingBox();
  await t.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 4 });
  await t.page.waitForSelector('.hover-hint', { timeout: 3000 });
  await t.frames(2);
  const card = await t.page.$eval('.hover-hint', (h) => {
    const parts = [];
    const walk = (el) => { for (const c of el.children) { if (c.classList.contains('hint-problem')) walk(c); else parts.push(c); } };
    walk(h);
    const s = getComputedStyle(h);
    const r = h.getBoundingClientRect();
    return {
      parts: parts.map((p) => { const b = p.getBoundingClientRect(); return { tag: p.tagName.toLowerCase(), top: b.top, bottom: b.bottom }; }),
      inner: { top: r.top + parseFloat(s.borderTopWidth), bottom: r.bottom - parseFloat(s.borderBottomWidth) },
      pad: { top: parseFloat(s.paddingTop), bottom: parseFloat(s.paddingBottom) },
      scrolls: h.scrollHeight > h.clientHeight,
    };
  });
  assert(card.parts.length >= 3, `the card for an unused variable has only ${card.parts.length} parts: ${JSON.stringify(card.parts)}`);
  const gaps = card.parts.slice(1).map((p, i) => p.top - card.parts[i].bottom);
  const [least, most] = [Math.min(...gaps), Math.max(...gaps)];
  assert(most - least <= 1, `the card's parts sit ${gaps.map((g) => px(g)).join(', ')} apart, not one even gap (${card.parts.map((p) => p.tag).join(' ')})`);
  assert(most <= 6, `the card's parts sit ${px(most)} apart — more space than a compact card needs`);
  assert(near(card.pad.top, card.pad.bottom, 0.5) && card.pad.top <= 6, `the card's padding is ${px(card.pad.top)} above and ${px(card.pad.bottom)} below`);
  const above = card.parts[0].top - card.inner.top;
  const below = card.inner.bottom - card.parts[card.parts.length - 1].bottom;
  assert(near(above, card.pad.top, 1) && near(below, card.pad.bottom, 1), `the card leaves ${px(above)} above its first part and ${px(below)} below its last, beyond its padding`);
  assert(!card.scrolls, 'a short hover card scrolls');
});

scenario('the_hint_stays_while_the_pointer_moves_into_it', async (t) => {
  await t.open(FILES.code);
  const handle = await t.page.evaluateHandle(() =>
    [...document.querySelectorAll('.studio-main > section .code-editor-highlight span')].find((s) => s.textContent === 'Repeat'));
  const word = handle.asElement();
  assert(word, 'the editor shows no "Repeat" token');
  const box = await word.boundingBox();
  await t.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 4 });
  const hint = await t.page.waitForSelector('.hover-hint', { timeout: 3000 }).catch(() => null);
  assert(hint, 'resting on "Repeat" shows no hint');
  const hb = await hint.boundingBox();
  await t.page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2, { steps: 8 });
  await new Promise((r) => setTimeout(r, 600));
  assert(await t.page.$('.hover-hint'), 'the hint vanished while the pointer moved into it');
  const link = await t.page.$eval('.hover-hint', (h) => { const a = h.querySelector('a'); return a && { href: a.href, text: a.textContent }; });
  assert(link && /^https:/.test(link.href), `the hint's link is not a link: ${JSON.stringify(link)}`);
  await t.page.mouse.move(5, 5, { steps: 6 });
  const gone = await t.page.waitForFunction(() => !document.querySelector('.hover-hint'), { timeout: 1500 }).then(() => true, () => false);
  assert(gone, 'the hint stays up after the pointer leaves it');
});

// Replace the first editor's whole text, as a person would: select all, then type.
async function retype(t, text) {
  const editor = await t.page.$('.studio-main > section textarea');
  await editor.click();
  await t.page.keyboard.down(process.platform === 'darwin' ? 'Meta' : 'Control');
  await t.page.keyboard.press('KeyA');
  await t.page.keyboard.up(process.platform === 'darwin' ? 'Meta' : 'Control');
  await t.page.keyboard.press('Backspace');
  await t.page.keyboard.type(text, { delay: 1 });
}

// VS Code's Problems in the editor: the language server's diagnostics as squiggles under the
// words they name, explained by the hover, and gone once the text is right.
scenario('a_mistake_gets_a_squiggle_and_its_hover_explains_it', async (t) => {
  await t.open(FILES.code);
  await retype(t, '## To f () -> Int:\n    Return "oops".\n\n## Main\nShow f().\n');
  const squiggle = await t.page.waitForSelector('.studio-main > section .code-editor-highlight .sq-error', { timeout: 4000 }).catch(() => null);
  assert(squiggle, 'a Text returned from an Int function draws no error squiggle');
  const look = await squiggle.evaluate((el) => {
    const cs = getComputedStyle(el);
    return { style: cs.textDecorationStyle, line: cs.textDecorationLine, color: cs.textDecorationColor };
  });
  assert(look.line.includes('underline') && look.style === 'wavy', `the error is not a wavy underline: ${JSON.stringify(look)}`);
  const box = await squiggle.boundingBox();
  await t.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 4 });
  const hint = await t.page.waitForSelector('.hover-hint .hint-problem', { timeout: 3000 }).catch(() => null);
  assert(hint, 'resting on the squiggle shows no problem in the hover');
  const text = await hint.evaluate((h) => h.closest('.hover-hint').innerText);
  assert(/Int/.test(text) && /Text/.test(text), `the hover does not explain the mismatch: ${JSON.stringify(text.slice(0, 200))}`);
  assert(/logicaffeine/.test(text), `the hover does not say where the problem comes from: ${JSON.stringify(text.slice(0, 200))}`);
  await t.page.mouse.move(5, 5);
  await retype(t, '## To f () -> Int:\n    Return 1.\n\n## Main\nShow f().\n');
  const cleared = await t.page.waitForFunction(() => !document.querySelector('.studio-main > section .code-editor-highlight .sq-error'), { timeout: 4000 }).then(() => true, () => false);
  assert(cleared, 'the squiggle stays after the mistake is fixed');
});

// A Hardware spec gets the editor's language service too: resting on a field explains it, and a
// value its parser would refuse is squiggled, with the reason in the hover.
scenario('a_hardware_spec_hovers_and_squiggles_like_code', async (t) => {
  await t.open('examples/hardware/pigeonhole.hw');
  const field = await t.page.evaluateHandle(() => [...document.querySelectorAll('.studio-main > section .code-editor-highlight span')].find((s) => s.textContent === 'pigeons'));
  const fieldBox = await field.asElement()?.boundingBox();
  assert(fieldBox, 'the spec shows no `pigeons` field');
  await t.page.mouse.move(fieldBox.x + fieldBox.width / 2, fieldBox.y + fieldBox.height / 2, { steps: 4 });
  const hint = await t.page.waitForSelector('.hover-hint', { timeout: 3000 }).catch(() => null);
  assert(hint, 'resting on `pigeons` shows no hover');
  const said = await hint.evaluate((h) => h.innerText);
  assert(/holes/.test(said), `the hover does not explain the field: ${JSON.stringify(said.slice(0, 160))}`);
  await t.page.mouse.move(5, 5);
  await retype(t, 'pigeons: lots\n');
  const squiggle = await t.page.waitForSelector('.studio-main > section .code-editor-highlight .sq-error', { timeout: 4000 }).catch(() => null);
  assert(squiggle, 'a count that is no number draws no error squiggle');
  const under = await squiggle.evaluate((e) => e.textContent);
  assert(under === 'lots', `the squiggle is under ${JSON.stringify(under)}, not the value at fault`);
  const box = await squiggle.boundingBox();
  await t.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 4 });
  const problem = await t.page.waitForSelector('.hover-hint .hint-problem', { timeout: 3000 }).catch(() => null);
  assert(problem, 'resting on the squiggle shows no problem');
  const text = await problem.evaluate((p) => p.closest('.hover-hint').innerText);
  assert(/whole number/.test(text), `the hover does not give the reason: ${JSON.stringify(text.slice(0, 160))}`);
});

// Compile makes a Rust monitor from an English spec. For a design with nothing to compile it says
// what the design is and what Execute does with it, never that it is "not a hardware property".
// The toolbar's buttons as they stand: each one's label, whether it can be pressed, and its tooltip.
function toolbarButtons(page) {
  return page.$$eval('.studio-toolbar-right button', (buttons) => buttons.map((b) => ({
    label: b.textContent.replace(/\s+/g, ' ').trim(),
    disabled: b.disabled,
    title: b.getAttribute('title') || '',
  })));
}

const buttonNamed = (buttons, name) => buttons.find((b) => b.label.includes(name));

scenario('compile_says_what_each_hardware_design_makes', async (t) => {
  for (const [file, expect] of [
    ['examples/hardware/handshake.hw', /Monitor/],
    ['examples/hardware/arbiter.v', /Verilog is already hardware/],
    ['examples/hardware/intersection-design.hw', /compiles to hardware, not Rust/],
    ['examples/hardware/register-alloc-fits.hw', /nothing to compile/],
    ['examples/hardware/pigeonhole.hw', /nothing to compile/],
  ]) {
    await t.open(file);
    const compile = buttonNamed(await toolbarButtons(t.page), 'Compile');
    assert(compile, `${file} has no Compile button`);
    if (compile.disabled) {
      assert(expect.test(compile.title), `${file}: Compile is disabled and its tooltip does not say what it makes (${expect}): ${JSON.stringify(compile.title)}`);
      continue;
    }
    await press(t.page, 'Compile');
    const seen = await settle(t.page, (s) => s.panels.some((p) => expect.test(p.text)), 8000);
    const all = seen.panels.map((p) => p.text).join('\n');
    assert(!/not a hardware property/.test(all), `Compile on ${file} blames the design: ${JSON.stringify(all.replace(/\s+/g, ' ').slice(0, 160))}`);
    assert(expect.test(all), `Compile on ${file} does not say what it makes (${expect})`);
    assert(!/nothing to compile|already hardware|compiles to hardware, not Rust/.test(all), `Compile on ${file} was offered and made nothing`);
  }
});

// A button the open file cannot use stays in its place, disabled, and its tooltip says why;
// pressing it changes nothing. Every other button is live.
scenario('an_action_the_file_cannot_take_is_disabled_and_says_why', async (t) => {
  const cases = [
    ['examples/code/algorithms/fibonacci.logos', {}],
    ['examples/code/networking/server.logos', { Run: /compiled runtime/, Debug: /compiled runtime/ }],
    ['examples/logic/simon.logic', { Compile: /Execute/ }],
    ['examples/logic/syllogism.logic', {}],
    ['examples/math/tarski-geometry.logos', { Compile: /Definition/ }],
    ['examples/math/natural-numbers.logos', {}],
    ['examples/hardware/arbiter.v', { Compile: /Verilog is already hardware/ }],
    ['examples/hardware/handshake.hw', {}],
  ];
  for (const [file, withheld] of cases) {
    await t.open(file);
    const buttons = await toolbarButtons(t.page);
    for (const name of ['Run', 'Execute', 'Compile', 'Debug']) {
      const button = buttonNamed(buttons, name);
      if (!button) continue;
      if (withheld[name]) {
        assert(button.disabled, `${file}: ${name} is live, but the file cannot use it`);
        assert(withheld[name].test(button.title), `${file}: the disabled ${name} does not say why (${withheld[name]}): ${JSON.stringify(button.title)}`);
      } else {
        assert(!button.disabled, `${file}: ${name} is disabled (${JSON.stringify(button.title)}), but the file can use it`);
      }
    }
    for (const name of Object.keys(withheld)) {
      const before = await readPanels(t.page);
      await t.page.evaluate((n) => [...document.querySelectorAll('.studio-toolbar-right button')].find((b) => b.textContent.includes(n))?.click(), name);
      await new Promise((r) => setTimeout(r, 600));
      const after = await readPanels(t.page);
      assert(JSON.stringify(after.panels) === JSON.stringify(before.panels), `${file}: pressing the disabled ${name} changed the panels`);
      assert(!(await t.page.$('.dbg-drawer')), `${file}: pressing the disabled ${name} opened the debugger`);
    }
  }
});

// The Symbol Dictionary describes the formula on screen, in the view on screen: every symbol it
// lists is in that formula, and a tense, an aspect or a thematic role is never called a predicate.
scenario('the_symbol_dictionary_describes_the_formula_on_screen', async (t) => {
  const OPERATORS = ['P', 'F', 'Past', 'Future', 'HAB', 'Prog', 'Perf', 'ITER', 'Agent', 'Patient', 'Theme', 'Recipient', 'Goal', 'Source', 'Instrument', 'Location', 'Time', 'Manner'];
  for (const file of ['examples/logic/negation-polarity.logic', 'examples/logic/events-adverbs.logic']) {
    await t.open(file);
    for (const view of ['Simple', 'Full', 'Deep']) {
      const button = await t.page.$(`xpath/.//button[contains(@class,'format-btn') and normalize-space(.)='${view}']`);
      assert(button, `the Logic Output has no ${view} view`);
      await button.click();
      await t.frames(4);
      const look = await t.page.evaluate(() => ({
        formula: [...document.querySelectorAll('.logic-display')].map((d) => d.innerText).join('\n'),
        entries: [...document.querySelectorAll('.symbol-entry')].map((e) => ({
          symbol: e.querySelector('.symbol-glyph')?.innerText.trim(),
          meaning: e.querySelector('.symbol-desc')?.innerText.trim(),
          group: e.closest('.symbol-group')?.querySelector('.symbol-group-title')?.innerText.trim(),
        })),
      }));
      assert(look.entries.length > 0, `${file} (${view}): the dictionary lists nothing`);
      // An operator's usage elides the formula it wraps (`Past(…)`); anything else is quoted as written.
      const name = (usage) => usage.replace(/\(.*$/, '');
      const absent = look.entries
        .filter((e) => !look.formula.includes(name(e.symbol)) || (!e.symbol.endsWith('(…)') && !look.formula.includes(e.symbol)))
        .map((e) => e.symbol);
      assert(absent.length === 0, `${file} (${view}): the dictionary lists ${JSON.stringify(absent)}, which the formula on screen does not show`);
      const repeated = look.entries.filter((e) => e.meaning === name(e.symbol) || /^(Predicate|Constant|Variable): /.test(e.meaning));
      assert(repeated.length === 0, `${file} (${view}): entries only repeat their symbol: ${JSON.stringify(repeated.slice(0, 3))}`);
      const misfiled = look.entries.filter((e) => /predicate/i.test(e.group) && OPERATORS.includes(name(e.symbol))).map((e) => e.symbol);
      assert(misfiled.length === 0, `${file} (${view}): ${JSON.stringify(misfiled)} filed as predicates`);
    }
  }
});

// The Logic column's blocks as a reader meets them — and as assistive technology does: each is a
// button heading that says whether it is expanded and names the region it controls. The column is
// the one holding the formula; a block is the nearest element holding both its heading and its
// region. Read in page order, with the parts a reader clicks, the pieces of the column in order,
// and whether the page shows a column beside it.
function readLogicColumn(page) {
  return page.evaluate(() => {
    const rect = (el) => {
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height };
    };
    const shown = (el) => !!el && getComputedStyle(el).display !== 'none' && el.getBoundingClientRect().height > 0;
    const aside = document.querySelector('.studio-main > aside');
    const beside = aside && shown(aside) ? aside.querySelector('.panel-header span')?.innerText.trim() || aside.className : null;
    const column = document.querySelector('.studio-main .logic-display')?.closest('.studio-main > section');
    if (!column) return { column: null, blocks: [], pieces: [], beside, viewport: document.documentElement.clientWidth };
    const content = column.querySelector('.panel-content');
    const blocks = [...content.querySelectorAll('button[aria-controls]')].map((heading) => {
      const body = document.getElementById(heading.getAttribute('aria-controls'));
      let block = heading.parentElement;
      while (block && body && !block.contains(body)) block = block.parentElement;
      const words = [...heading.querySelectorAll('*')].find((el) => [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()));
      return {
        title: heading.innerText.trim(),
        expanded: heading.getAttribute('aria-expanded'),
        tag: heading.tagName.toLowerCase(),
        type: heading.getAttribute('type'),
        bodyFound: !!body,
        bodyShown: shown(body),
        bodyText: shown(body) ? body.innerText.trim() : '',
        heading: rect(heading),
        block: rect(block),
        body: shown(body) ? rect(body) : null,
        chevron: rect(heading.querySelector('svg')),
        words: rect(words),
      };
    });
    const pieces = [...content.children].filter((el) => el.tagName !== 'STYLE' && shown(el)).map((el) => ({
      block: !!el.querySelector('button[aria-controls]'),
      ...rect(el),
    }));
    return {
      title: column.querySelector('.panel-header span')?.innerText.trim(),
      column: rect(column),
      content: rect(content),
      // The content's inner right edge: a scrolling column keeps its scrollbar's gutter beside it.
      innerRight: content.getBoundingClientRect().left + content.clientLeft + content.clientWidth,
      formulaBottom: Math.max(...[...content.querySelectorAll('.logic-display')].map((d) => d.getBoundingClientRect().bottom)),
      blocks,
      pieces,
      beside,
      viewport: document.documentElement.clientWidth,
    };
  });
}

// The block titled `title` in the Logic column, or a failure naming the blocks there are.
function logicBlock(look, title) {
  const block = look.blocks.find((b) => b.title === title);
  assert(block, `the Logic column has no block titled ${JSON.stringify(title)} (its blocks: ${JSON.stringify(look.blocks.map((b) => b.title))})`);
  return block;
}

// Read the Logic column until the block titled `title` satisfies `ok`, or `ms` passes; the last
// reading either way.
async function settleBlock(t, title, ok, ms = 3000) {
  const until = Date.now() + ms;
  let look = await readLogicColumn(t.page);
  while (!(look.blocks.some((b) => b.title === title && ok(b))) && Date.now() < until) {
    await new Promise((r) => setTimeout(r, 100));
    look = await readLogicColumn(t.page);
  }
  return look;
}

const middle = (r) => [r.left + r.width / 2, r.top + r.height / 2];

// Scroll the Logic column until the part of the block titled `title` named by `part` — its
// heading, or its region — is in view, as a reader scrolls to what they mean to click.
async function reveal(t, title, part = 'heading') {
  await t.page.evaluate((title, part) => {
    const column = document.querySelector('.studio-main .logic-display').closest('.studio-main > section');
    const heading = [...column.querySelectorAll('button[aria-controls]')].find((h) => h.innerText.trim() === title);
    const target = part === 'heading' ? heading : heading && document.getElementById(heading.getAttribute('aria-controls'));
    if (target) target.scrollIntoView({ block: part === 'heading' ? 'nearest' : 'center' });
  }, title, part);
  await t.frames(2);
}

// Click the block titled `title` at (x, y) and read what it did, once the page has settled.
async function clickBlock(t, title, x, y, expect) {
  await t.page.mouse.click(x, y);
  return logicBlock(await settleBlock(t, title, (b) => b.expanded === expect), title);
}

// Open (or close) the block titled `title` by its heading, whatever state it is in; the block as it
// then reads, its heading in view.
async function setBlock(t, title, open) {
  await reveal(t, title);
  const block = logicBlock(await readLogicColumn(t.page), title);
  if ((block.expanded === 'true') === open) return block;
  await clickBlock(t, title, ...middle(block.words), String(open));
  await reveal(t, title);
  return logicBlock(await readLogicColumn(t.page), title);
}

// Where a reader clicks inside the open block titled `title`'s content, with that content scrolled
// into view: its middle, its left edge and its last visible line — each within what the column shows.
async function contentSpots(t, title) {
  await reveal(t, title, 'body');
  const look = await readLogicColumn(t.page);
  const body = logicBlock(look, title).body;
  assert(body, `the open ${title} shows no content`);
  const top = Math.max(body.top, look.content.top), bottom = Math.min(body.bottom, look.content.bottom);
  assert(bottom - top > 8, `the open ${title}'s content is not in view`);
  return [
    ['its content', body.left + body.width / 2, (top + bottom) / 2],
    // Clear of the column bar's grab area, which reaches 4px into the column.
    ['its content\'s left edge', body.left + 8, top + Math.min(20, (bottom - top) / 2)],
    ['its content\'s last line', body.left + body.width / 2, bottom - 4],
  ];
}

// A Logic sentences file sets its Logic column's blocks in one stack under the formula: the Symbol
// Dictionary over the Syntax Tree, each across the column's width. The Syntax Tree has no column
// of its own beside the Logic Output any more.
scenario('the_logic_column_stacks_the_symbol_dictionary_over_the_syntax_tree', async (t) => {
  for (const viewport of [DESKTOP, PHONE]) {
    const at = `at ${viewport.width}px`;
    await t.open(FILES.sentences, viewport);
    const look = await readLogicColumn(t.page);
    assert(look.column, `${at}: no column shows the formula`);
    const titles = look.blocks.map((b) => b.title);
    assert(JSON.stringify(titles) === JSON.stringify(['Symbol Dictionary', 'Syntax Tree']),
      `${at}: the Logic column's blocks are ${JSON.stringify(titles)}, not the Symbol Dictionary over the Syntax Tree`);
    const [dictionary, tree] = look.blocks;
    assert(dictionary.block.top >= look.formulaBottom - 0.5, `${at}: the Symbol Dictionary starts ${px(look.formulaBottom - dictionary.block.top)} above the formula's end`);
    assert(tree.block.top >= dictionary.block.bottom - 0.5, `${at}: the Syntax Tree starts ${px(dictionary.block.bottom - tree.block.top)} inside the Symbol Dictionary`);
    for (const b of look.blocks) {
      assert(near(b.block.left, look.content.left, 0.5) && near(b.block.right, look.innerRight, 0.5),
        `${at}: the ${b.title} block spans ${px(b.block.left)}–${px(b.block.right)}, not the column's ${px(look.content.left)}–${px(look.innerRight)}`);
    }
    assert(!look.beside, `${at}: Logic still shows a column beside the Logic Output: ${JSON.stringify(look.beside)}`);
  }
// Each load leaves the page before it has settled; the requests it cancels are page errors to
// Firefox. Nothing a reader does.
}, { allowErrors: NAVIGATION_ABORT });

// A Logic file opens with its Symbol Dictionary open and its Syntax Tree closed to its heading.
scenario('the_syntax_tree_starts_collapsed_and_the_dictionary_open', async (t) => {
  await t.open(FILES.sentences);
  const look = await readLogicColumn(t.page);
  const dictionary = logicBlock(look, 'Symbol Dictionary');
  const tree = logicBlock(look, 'Syntax Tree');
  assert(dictionary.expanded === 'true' && dictionary.bodyShown && dictionary.bodyText.length > 0,
    `the Symbol Dictionary does not open shown (aria-expanded=${dictionary.expanded}, body ${dictionary.bodyShown ? 'shown' : 'hidden'})`);
  assert(tree.expanded === 'false', `the Syntax Tree opens with aria-expanded=${tree.expanded}`);
  assert(tree.bodyFound, 'the Syntax Tree heading names a region the page does not have');
  assert(!tree.bodyShown, 'the Syntax Tree opens with its tree shown');
  assert(near(tree.block.height, tree.heading.height, 1.5), `the closed Syntax Tree takes ${px(tree.block.height)}, more than its ${px(tree.heading.height)} heading`);
});

// A closed block opens wherever the reader clicks it: its title, either end of its row, its top
// and bottom edges — at a desk and on a phone.
scenario('a_closed_block_opens_when_clicked_anywhere_on_it', async (t) => {
  for (const viewport of [DESKTOP, PHONE]) {
    await t.open(FILES.sentences, viewport);
    for (const title of ['Syntax Tree', 'Symbol Dictionary']) {
      const spots = (r) => [
        ['its title', ...middle(logicBlock(r, title).words)],
        // The column bar's grab area reaches 4px into the column, so the row's ends are taken 8px in.
        ['its row\'s left end', logicBlock(r, title).block.left + 8, middle(logicBlock(r, title).block)[1]],
        ['its row\'s right end', logicBlock(r, title).block.right - 8, middle(logicBlock(r, title).block)[1]],
        ['its top edge', middle(logicBlock(r, title).block)[0], logicBlock(r, title).block.top + 1.5],
        ['its bottom edge', middle(logicBlock(r, title).block)[0], logicBlock(r, title).block.bottom - 1.5],
      ];
      for (let i = 0; i < 5; i++) {
        await setBlock(t, title, false);
        const [where, x, y] = spots(await readLogicColumn(t.page))[i];
        const after = await clickBlock(t, title, x, y, 'true');
        assert(after.expanded === 'true' && after.bodyShown, `at ${viewport.width}px a click on the closed ${title}'s ${where} left it closed`);
      }
    }
  }
// Each load leaves the page before it has settled; the requests it cancels are page errors to
// Firefox. Nothing a reader does.
}, { allowErrors: NAVIGATION_ABORT });

// An open block closes from its heading — the chevron, the title, the rest of the row — and never
// from its content, however the reader clicks in it.
scenario('an_open_block_closes_by_its_heading_or_chevron_not_its_content', async (t) => {
  await t.open(FILES.sentences);
  for (const title of ['Syntax Tree', 'Symbol Dictionary']) {
    let block = await setBlock(t, title, true);
    for (let i = 0; i < 3; i++) {
      const [where, x, y] = (await contentSpots(t, title))[i];
      await t.page.mouse.click(x, y);
      await new Promise((r) => setTimeout(r, 400));
      block = logicBlock(await readLogicColumn(t.page), title);
      assert(block.expanded === 'true' && block.bodyShown, `a click on the open ${title}'s ${where} closed it`);
    }
    const heading = (b) => [
      ['chevron', ...middle(b.chevron)],
      ['title', ...middle(b.words)],
      ['heading\'s far right', b.heading.right - 4, middle(b.heading)[1]],
    ];
    for (let i = 0; i < 3; i++) {
      block = await setBlock(t, title, true);
      assert(block.chevron, `the ${title} heading draws no chevron`);
      const [where, x, y] = heading(block)[i];
      const after = await clickBlock(t, title, x, y, 'false');
      assert(after.expanded === 'false' && !after.bodyShown, `a click on the open ${title}'s ${where} did not close it`);
    }
  }
});

// Each heading is a real button: Tab reaches it with a focus ring a reader can see, Enter and
// Space toggle it, and it names the region it opens.
scenario('a_block_toggles_from_the_keyboard', async (t) => {
  await t.open(FILES.sentences);
  const look = await readLogicColumn(t.page);
  for (const title of ['Symbol Dictionary', 'Syntax Tree']) {
    const b = logicBlock(look, title);
    assert(b.tag === 'button' && b.type === 'button', `the ${title} heading is a <${b.tag} type=${b.type}>, not a button`);
    assert(b.bodyFound, `the ${title} heading names a region the page does not have`);
  }
  // Tab from the dictionary's heading lands on the tree's: nothing between them takes focus.
  await t.page.evaluate(() => {
    const column = document.querySelector('.studio-main .logic-display').closest('.studio-main > section');
    column.querySelector('button[aria-controls]').focus();
  });
  await t.page.keyboard.press('Tab');
  await t.frames(2);
  const focused = await t.page.evaluate(() => {
    const el = document.activeElement;
    const cs = getComputedStyle(el);
    return {
      title: el.innerText?.trim(),
      visible: el.matches(':focus-visible'),
      outline: { style: cs.outlineStyle, width: parseFloat(cs.outlineWidth), color: cs.outlineColor },
    };
  });
  assert(focused.title === 'Syntax Tree', `Tab from the Symbol Dictionary's heading focused ${JSON.stringify(focused.title)}, not the Syntax Tree's`);
  assert(focused.visible && focused.outline.style !== 'none' && focused.outline.width >= 1 && !/rgba\(\d+, \d+, \d+, 0\)|transparent/.test(focused.outline.color),
    `the focused heading shows no focus ring: ${JSON.stringify(focused.outline)}`);
  for (const [key, want] of [['Enter', 'true'], [' ', 'false'], [' ', 'true'], ['Enter', 'false']]) {
    await t.page.keyboard.press(key);
    const b = logicBlock(await settleBlock(t, 'Syntax Tree', (x) => x.expanded === want), 'Syntax Tree');
    const name = key === ' ' ? 'Space' : key;
    assert(b.expanded === want && b.bodyShown === (want === 'true'), `${name} left the Syntax Tree aria-expanded=${b.expanded}, its tree ${b.bodyShown ? 'shown' : 'hidden'}`);
  }
});

// A block's chevron is the file tree's — the same drawn path, a 2px stroke, 12px along its long
// side — just left of the title and centred on it. It points right while the block is closed and
// turns to point down when it opens.
scenario('a_blocks_chevron_sits_left_of_its_title_and_turns_down_when_open', async (t) => {
  await t.open(FILES.sentences);
  const read = (title) => t.page.evaluate((title) => {
    const column = document.querySelector('.studio-main .logic-display').closest('.studio-main > section');
    const heading = [...column.querySelectorAll('button[aria-controls]')].find((h) => h.innerText.trim() === title);
    if (!heading) return null;
    const svg = heading.querySelector('svg');
    const path = svg && svg.querySelector('path');
    if (!path) return { drawn: false };
    const words = [...heading.querySelectorAll('*')].find((el) => [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim())).getBoundingClientRect();
    const c = svg.getBoundingClientRect();
    const scale = c.width / svg.viewBox.baseVal.width;
    const box = path.getBBox();
    const ps = getComputedStyle(path);
    // Which way the drawn path points, in its own drawing: from its ends toward its middle vertex.
    const length = path.getTotalLength();
    const [start, vertex, end] = [0, length / 2, length].map((l) => path.getPointAtLength(l));
    const drawn = Math.atan2(vertex.y - (start.y + end.y) / 2, vertex.x - (start.x + end.x) / 2) * 180 / Math.PI;
    // How far the page turns it: every CSS rotation between the drawing and the heading.
    let turned = 0;
    for (let el = svg; el && el !== heading.parentElement; el = el.parentElement) {
      const m = getComputedStyle(el).transform;
      if (m && m !== 'none') {
        const [a, b] = m.match(/-?[\d.e-]+/g).map(Number);
        turned += Math.atan2(b, a) * 180 / Math.PI;
      }
    }
    const tree = document.querySelector('.file-tree .chevron svg path');
    return {
      drawn: true,
      path: path.getAttribute('d'),
      treePath: tree && tree.getAttribute('d'),
      stroke: (ps.stroke === 'none' ? 0 : parseFloat(ps.strokeWidth)) * scale,
      long: Math.max(box.width, box.height) * scale + (ps.stroke === 'none' ? 0 : parseFloat(ps.strokeWidth)) * scale,
      points: Math.round(drawn + turned),
      gap: words.left - c.right,
      offset: (c.top + c.height / 2) - (words.top + words.height / 2),
    };
  }, title);
  for (const title of ['Symbol Dictionary', 'Syntax Tree']) {
    for (const open of [false, true]) {
      await setBlock(t, title, open);
      await new Promise((r) => setTimeout(r, 400));
      const c = await read(title);
      const state = open ? 'open' : 'closed';
      assert(c, `the Logic column has no ${title} heading`);
      assert(c.drawn, `the ${title} heading's chevron is not a drawn icon`);
      assert(c.treePath && c.path === c.treePath, `the ${title} chevron is drawn ${JSON.stringify(c.path)}, not the file tree's ${JSON.stringify(c.treePath)}`);
      assert(near(c.stroke, 2, 0.01), `the ${title} chevron has a ${c.stroke}px stroke, not 2px`);
      assert(near(c.long, 12, 0.5), `the ${title} chevron is ${px(c.long)} along its long side, not 12px`);
      assert(c.gap >= 0 && c.gap <= 12, `the ${title} chevron sits ${px(c.gap)} from its title, not just left of it`);
      assert(Math.abs(c.offset) <= 1, `the ${title} chevron sits ${px(c.offset)} off its title's middle`);
      const want = open ? 90 : 0;
      assert(((c.points - want) % 360 + 360) % 360 === 0, `the ${state} ${title}'s chevron points ${c.points}°, not ${open ? 'down (90°)' : 'right (0°)'}`);
    }
  }
});

// Every title in the Studio is set in one type — the panel headers ("English Input", "Logic
// Output", "Code Editor", …), the Files header and the Logic column's "Symbol Dictionary" and
// "Syntax Tree" — the same family, size, weight, letter-spacing, colour and case.
scenario('every_title_in_the_studio_shares_the_panel_title_type', async (t) => {
  const off = [];
  for (const [kind, file] of EVERY_KIND) {
    await t.open(file);
    const seen = await t.page.evaluate(() => {
      const style = (el) => {
        const s = getComputedStyle(el);
        return { family: s.fontFamily, size: s.fontSize, weight: s.fontWeight, spacing: s.letterSpacing, color: s.color, transform: s.textTransform };
      };
      const titles = [];
      for (const header of document.querySelectorAll('.studio-panel .panel-header')) {
        const span = header.querySelector('span');
        if (span) titles.push({ name: span.innerText.trim(), style: style(span) });
      }
      const files = document.querySelector('.file-browser-title');
      if (files) titles.push({ name: 'Files', style: style(files) });
      // The Logic column's section titles, wherever they sit: the element whose own text they are.
      for (const el of document.querySelectorAll('.studio-main *')) {
        const own = [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join('').trim();
        if (/^(symbol dictionary|syntax tree)$/i.test(own) && !el.closest('.panel-header')) titles.push({ name: own, style: style(el) });
      }
      return titles;
    });
    const [first, ...rest] = seen;
    for (const title of rest) {
      for (const key of Object.keys(first.style)) {
        if (title.style[key] !== first.style[key]) off.push(`${kind}: "${title.name}" has ${key} ${title.style[key]}; "${first.name}" has ${first.style[key]}`);
      }
    }
    if (kind.startsWith('Logic sentences')) {
      for (const name of ['Symbol Dictionary', 'Syntax Tree']) {
        if (!seen.some((s) => s.name.toLowerCase() === name.toLowerCase())) off.push(`${kind}: no "${name}" title`);
      }
    }
  }
  assert(off.length === 0, `${off.length} title(s) are not in the panel-title type:\n${off.join('\n')}`);
// Each load leaves the page before it has settled; the requests it cancels are page errors to
// Firefox. Nothing a reader does.
}, { allowErrors: NAVIGATION_ABORT });

// Opened, the Syntax Tree draws the tree of what was last executed — its root's first glyph at the
// column's 20px inset, each level indented under its parent — and follows the next Execute while it
// stays open. It never scrolls inside the column: the column is the one scroll.
scenario('the_syntax_tree_renders_and_follows_the_input_when_open', async (t) => {
  await t.open(FILES.sentences);
  await setBlock(t, 'Syntax Tree', true);
  const readTree = () => t.page.evaluate(() => {
    const column = document.querySelector('.studio-main .logic-display').closest('.studio-main > section');
    const heading = [...column.querySelectorAll('button[aria-controls]')].find((h) => h.innerText.trim() === 'Syntax Tree');
    const body = heading && document.getElementById(heading.getAttribute('aria-controls'));
    if (!body) return null;
    const tree = body.querySelector('[data-tree]');
    const labels = tree ? [...tree.querySelectorAll('.tree-item')].filter((l) => l.getBoundingClientRect().height > 0) : [];
    const glyph = (row) => (row.querySelector(':scope > .twistie') || [...row.children].find((c) => !c.classList.contains('tree-branch'))).getBoundingClientRect().left;
    const indents = tree ? [...tree.querySelectorAll('.tree-children')].flatMap((kids) => {
      const parent = glyph(kids.parentElement.querySelector(':scope > .tree-item'));
      return [...kids.querySelectorAll(':scope > .tree-node > .tree-item')].map((row) => glyph(row) - parent);
    }) : [];
    const range = document.createRange();
    const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
    let first = null;
    while (walker.nextNode()) {
      if (!walker.currentNode.textContent.trim() || walker.currentNode.parentElement.closest('style')) continue;
      range.selectNodeContents(walker.currentNode);
      first = range.getClientRects()[0];
      break;
    }
    const inner = [body, ...body.querySelectorAll('*')].filter((el) => ['auto', 'scroll'].includes(getComputedStyle(el).overflowY) && el.scrollHeight > el.clientHeight + 1);
    return {
      text: (tree || body).innerText,
      labels: labels.length,
      indents,
      firstLeft: first ? first.left - column.getBoundingClientRect().left : null,
      innerScrolls: inner.map((el) => el.className),
    };
  });
  let tree = await readTree();
  assert(tree && tree.labels > 1, `the open Syntax Tree draws ${tree ? tree.labels : 'no'} node(s)`);
  assert(tree.firstLeft !== null, 'the open Syntax Tree shows no text');
  assert(near(tree.firstLeft, INSET, 3), `the Syntax Tree's content starts ${px(tree.firstLeft)} from the column's edge, not ${INSET}px`);
  assert(tree.indents.length > 0 && tree.indents.every((d) => d > 4), `a level of the tree does not sit indented under its parent: ${JSON.stringify(tree.indents)}`);
  assert(tree.innerScrolls.length === 0, `the tree scrolls inside the column: ${JSON.stringify(tree.innerScrolls)}`);
  assert(/student/i.test(tree.text), `the tree does not show the example's sentences: ${JSON.stringify(tree.text.slice(0, 160))}`);

  await paste(t, 'John loves Mary.\n');
  await press(t.page, 'Execute');
  const until = Date.now() + 20000;
  while (!/mary/i.test(tree.text) && Date.now() < until) {
    await new Promise((r) => setTimeout(r, 250));
    tree = (await readTree()) || tree;
  }
  const block = logicBlock(await readLogicColumn(t.page), 'Syntax Tree');
  assert(block.expanded === 'true' && block.bodyShown, 'Execute closed the Syntax Tree the reader had opened');
  assert(/mary/i.test(tree.text) && /john/i.test(tree.text), `after "John loves Mary." the tree shows ${JSON.stringify(tree.text.slice(0, 160))}`);
  assert(!/student|professor/i.test(tree.text), `after "John loves Mary." the tree still shows the example's sentences: ${JSON.stringify(tree.text.slice(0, 160))}`);
});

// The Logic column scrolls only when its content needs it and never into empty space, whichever
// blocks are open, at every size; its pieces — the formula, then each block — follow one another
// with no gap; and with no column beside it, it reaches the window's right edge.
scenario('the_logic_column_has_no_needless_scrollbar_or_dead_space', async (t) => {
  const off = [];
  const states = [
    ['as it opens', null],
    ['both open', ['Syntax Tree', true]],
    ['the tree alone open', ['Symbol Dictionary', false]],
    ['both closed', ['Syntax Tree', false]],
    ['the dictionary open again', ['Symbol Dictionary', true]],
  ];
  for (const [file, viewport] of [[FILES.sentences, DESKTOP], [FILES.sentences, { ...DESKTOP, width: 1366, height: 768 }], [FILES.sentences, PHONE], [FILES.logic, DESKTOP]]) {
    await t.open(file, viewport);
    const at = `${file} at ${viewport.width}×${viewport.height}`;
    for (const [state, change] of states) {
      const look0 = await readLogicColumn(t.page);
      if (change) {
        if (!look0.blocks.some((b) => b.title === change[0])) {
          if (file === FILES.logic && change[0] === 'Syntax Tree') continue;
          off.push(`${at}: no ${change[0]} block to set ${state}`);
          continue;
        }
        await setBlock(t, ...change);
        await new Promise((r) => setTimeout(r, 300));
      }
      const look = await readLogicColumn(t.page);
      const where = `${at}, ${state}`;
      for (const f of [...await scrollFindings(t.page), ...await emptyScrollFindings(t.page)]) {
        if (f.startsWith(look.title)) off.push(`${where}: ${f}`);
      }
      for (let i = 1; i < look.pieces.length; i++) {
        const gap = look.pieces[i].top - look.pieces[i - 1].bottom;
        if (Math.abs(gap) > 1) off.push(`${where}: ${px(gap)} between the column's piece ${i} and the one above it`);
      }
      const inner = await t.page.evaluate(() => {
        const content = document.querySelector('.studio-main .logic-display').closest('.studio-main > section').querySelector('.panel-content');
        return [...content.querySelectorAll('*')].filter((el) => ['auto', 'scroll'].includes(getComputedStyle(el).overflowY) && el.scrollHeight > el.clientHeight + 1).map((el) => el.className);
      });
      for (const name of inner) off.push(`${where}: <${name}> scrolls inside the column`);
      if (viewport !== PHONE) {
        if (look.beside) off.push(`${where}: a column beside the Logic Output (${look.beside})`);
        if (!near(look.column.right, look.viewport, 1)) off.push(`${where}: the Logic column stops ${px(look.viewport - look.column.right)} short of the window's right edge`);
      }
    }
  }
  assert(off.length === 0, `${off.length} finding(s) in the Logic column:\n${off.join('\n')}`);
// Each load leaves the page before it has settled; the requests it cancels are page errors to
// Firefox. Nothing a reader does.
}, { allowErrors: NAVIGATION_ABORT });

// With the Syntax Tree inside the Logic column, the Logic row is the file browser and two panels:
// two bars, each moving only the two columns beside it, and the row still fitting the window.
scenario('the_logic_rows_two_bars_still_drag', async (t) => {
  await t.open(FILES.sentences);
  const count = (await t.bars()).length;
  assert(count === 2, `the Logic row shows ${count} column bars, not the two beside its file browser and two panels`);
  for (const [i, [a, b]] of [['sidebar', 'left'], ['left', 'center']].entries()) {
    for (const dx of [80, -80]) {
      const before = await t.layout();
      await t.dragElement((await t.bars())[i], dx, 0);
      const after = await t.layout();
      const ctx = `dragging the ${a}|${b} bar by ${dx}px`;
      assertOthersStill(before, after, [a, b], ctx);
      assert(near(after[a].w - before[a].w, dx, 1.5), `${ctx}: the ${a} column changed by ${px(after[a].w - before[a].w)}`);
      assert(near(after[b].w - before[b].w, -dx, 1.5), `${ctx}: the ${b} column changed by ${px(after[b].w - before[b].w)}`);
      assertFits(after, ctx);
    }
  }
});

// A block keeps the reader's choice: an Execute and a switch to another Logic file leave each block
// open or closed as the reader left it.
scenario('a_block_keeps_the_readers_choice_across_executes_and_files', async (t) => {
  await t.open(FILES.sentences);
  await setBlock(t, 'Symbol Dictionary', false);
  await setBlock(t, 'Syntax Tree', true);
  const states = async () => {
    const look = await readLogicColumn(t.page);
    return ['Symbol Dictionary', 'Syntax Tree'].map((title) => `${title}: ${look.blocks.find((b) => b.title === title)?.expanded ?? 'absent'}`).join(', ');
  };
  const want = 'Symbol Dictionary: false, Syntax Tree: true';
  await press(t.page, 'Execute');
  await new Promise((r) => setTimeout(r, 1500));
  let now = await states();
  assert(now === want, `after Execute the blocks read ${now}, not ${want}`);
  const next = '/examples/logic/tense-aspect.logic';
  const before = await t.page.$eval('.studio-main > section textarea', (e) => e.value);
  await t.page.click(`.file-tree-item[title="${next}"]`);
  await t.page.waitForFunction((was) => document.querySelector('.studio-main > section textarea')?.value !== was, { timeout: 20000 }, before);
  await new Promise((r) => setTimeout(r, 1000));
  now = await states();
  assert(now === want, `after opening ${next} the blocks read ${now}, not ${want}`);
});

// The Studio's trees — the file browser's and the Syntax Tree — are one component, and read alike.
// A tree is found by its root (`.file-tree`, `.syntax-tree`); its parts are the shared tree's.

// Every shown row of the tree under `root`: its depth, the left of its first glyph (a node's
// chevron cell, else the first thing after its branch) from the tree's edge, its height, and
// whether it leads with a chevron cell and has children to open.
function treeRows(page, root) {
  return page.evaluate((root) => {
    const tree = document.querySelector(root);
    if (!tree) return null;
    // The tree's edge is where its content starts: the device-pixel grid nudges it under a pixel in.
    const edge = tree.getBoundingClientRect().left + parseFloat(getComputedStyle(tree).paddingLeft);
    return [...tree.querySelectorAll('.tree-item')].filter((r) => r.getBoundingClientRect().height > 0).map((row) => {
      let depth = 0;
      for (let n = row.parentElement.parentElement; n && n !== tree; n = n.parentElement) if (n.classList.contains('tree-children')) depth++;
      const first = row.querySelector(':scope > .twistie') || [...row.children].find((c) => !c.classList.contains('tree-branch'));
      return {
        depth,
        first: first ? first.getBoundingClientRect().left - edge : null,
        height: row.getBoundingClientRect().height,
        cell: !!row.querySelector(':scope > .twistie'),
        // Every level of the Syntax Tree opens by default, so a node with parts shows them.
        parent: !!row.parentElement.querySelector(':scope > .tree-children'),
        name: row.innerText.trim().split('\n')[0],
      };
    });
  }, root);
}

// Where the lines of the tree under `root` fail to connect: every open node's children hang from
// one line one step further in than its own, starting at the node's branch (or its row's middle
// at the top level) and stopping at the last child's branch; every branch leaves its line at the
// row's middle, runs exactly one step, and ends `gap` before the row's first glyph.
function treeBranchProblems(page, root, step, gap) {
  return page.evaluate((root, STEP, GAP) => {
    const out = [];
    const tree = document.querySelector(root);
    if (!tree) return [`no ${root} on the page`];
    const edge = tree.getBoundingClientRect().left + parseFloat(getComputedStyle(tree).paddingLeft);
    const alpha = (c) => { const m = c.match(/[\d.]+/g) || []; return m.length === 4 ? Number(m[3]) : 1; };
    const parts = (node) => {
      const row = node.querySelector(':scope > .tree-item');
      return { row, line: node.querySelector(':scope > .tree-line'), branch: row && row.querySelector(':scope > .tree-branch'), name: row?.innerText.trim().split('\n')[0] };
    };
    const rect = (el) => el && el.getBoundingClientRect();
    const glyphOf = (row) => row.querySelector(':scope > .twistie') || [...row.children].find((c) => !c.classList.contains('tree-branch'));
    const groups = [...tree.querySelectorAll('.tree-children')];
    if (!groups.length) out.push('no node of the tree is open');
    for (const group of groups) {
      const folder = parts(group.parentElement);
      const nodes = [...group.children].filter((n) => n.classList.contains('tree-node'));
      nodes.forEach((node, i) => {
        const { row, line, branch, name } = parts(node);
        if (!line || !branch) { out.push(`${name}: no tree line or branch`); return; }
        const l = rect(line), b = rect(branch), r = rect(row);
        if (alpha(getComputedStyle(line).backgroundColor) === 0 || alpha(getComputedStyle(branch).backgroundColor) === 0) out.push(`${name}: a line is invisible`);
        if (Math.abs((b.top + b.height / 2) - (r.top + r.height / 2)) > 1) out.push(`${name}: its branch is not at the row's middle`);
        if (Math.abs(b.left - l.left) > 0.5) out.push(`${name}: its branch does not leave the tree line`);
        if (Math.abs(b.width - STEP) > 0.5) out.push(`${name}: its branch runs ${b.width.toFixed(1)}px, not one ${STEP}px step`);
        const glyph = rect(glyphOf(row));
        if (Math.abs(glyph.left - b.right - GAP) > 0.5) out.push(`${name}: ${(glyph.left - b.right).toFixed(1)}px between its branch and its first glyph, not ${GAP}px`);
        const from = folder.line ? rect(folder.line).left : edge;
        if (Math.abs(l.left - from - STEP) > 0.5) out.push(`${name}: its line stands ${(l.left - from).toFixed(1)}px in from ${folder.line ? `${folder.name}'s line` : "the tree's edge"}, not one ${STEP}px step`);
        if (i === 0) {
          const fr = rect(folder.row);
          const start = folder.branch ? rect(folder.branch).top : fr.top + fr.height / 2 - l.width / 2;
          if (Math.abs(l.top - start) > 0.5) out.push(`${name}: the line under ${folder.name} starts ${(l.top - start).toFixed(1)}px from ${folder.branch ? 'its branch' : "its row's middle"}`);
          if (folder.branch && Math.abs(rect(folder.branch).right - l.left) > 0.5) out.push(`${folder.name}: its branch ends ${(l.left - rect(folder.branch).right).toFixed(1)}px short of the line its children hang from`);
        }
        const last = i === nodes.length - 1;
        if (last && Math.abs(l.bottom - b.bottom) > 0.5) out.push(`${name}: the line does not stop at the last child's branch (${(l.bottom - b.bottom).toFixed(1)}px)`);
        if (!last) {
          const next = rect(nodes[i + 1].querySelector(':scope > .tree-line'));
          if (Math.abs(next.top - l.bottom) > 0.5) out.push(`${name}: the tree line breaks before the next sibling (${(next.top - l.bottom).toFixed(1)}px)`);
        }
      });
    }
    return out;
  }, root, step, gap);
}

// Open a Logic sentences file with its Syntax Tree block open: the tree the Syntax Tree scenarios read.
async function openSyntaxTree(t, viewport) {
  await t.open(FILES.sentences, viewport);
  await setBlock(t, 'Syntax Tree', true);
  await t.page.waitForFunction(() => document.querySelector('.syntax-tree .tree-item'), { timeout: 5000 }).catch(() => {});
  await t.frames(4);
}

// The Syntax Tree reads like the file tree: 22px rows in the tree's font, each row at depth d
// starting its first glyph (d + 1) steps and a gap from the tree's edge, and a chevron cell only
// on a node that has parts to open.
scenario('the_syntax_tree_reads_like_the_file_tree', async (t) => {
  await openSyntaxTree(t);
  const rows = await treeRows(t.page, '.syntax-tree');
  assert(rows && rows.length > 1, `the open Syntax Tree draws ${rows ? rows.length : 'no'} row(s) of the shared tree`);
  const heights = [...new Set(rows.map((r) => Math.round(r.height)))];
  assert(heights.length === 1 && heights[0] === 22, `the Syntax Tree's rows are ${JSON.stringify(heights)}px tall, not the file tree's 22px`);
  for (const r of rows) {
    const want = TREE_STEP * (r.depth + 1) + TREE_GAP;
    assert(near(r.first, want, 0.5), `${r.name} (depth ${r.depth}) starts ${px(r.first)} from the tree's edge, not ${px(want)} — ${r.depth + 1} steps and a gap`);
    assert(r.cell === r.parent, r.parent ? `${r.name} has parts but no chevron cell` : `${r.name} has no parts, yet keeps a chevron cell`);
  }
  assert(rows.some((r) => r.cell) && rows.some((r) => !r.cell), 'the Syntax Tree shows no mix of nodes and leaves');
});

// Its lines connect as the file tree's do: one step between levels, one step for every branch, an
// elbow where an open node's line starts, and a line that stops at the last child.
scenario('the_syntax_tree_branches_connect', async (t) => {
  await openSyntaxTree(t);
  const problems = await treeBranchProblems(t.page, '.syntax-tree', TREE_STEP, TREE_GAP);
  assert(problems.length === 0, `the Syntax Tree's lines do not connect:\n${problems.slice(0, 12).join('\n')}`);
});

// At every real device scale the Syntax Tree's lines, like the file tree's, cover whole device
// pixels: the tree reads the page's ratio, every line's edges land on the device-pixel grid wherever
// the Logic column puts the tree, and all lines share one weight of at least two device pixels.
scenario('the_syntax_tree_lines_keep_one_weight_at_every_zoom', async (t) => {
  const executable = t.page.browser().process()?.spawnfile;
  assert(executable, 'this run did not launch its browser, so it cannot launch one at a device scale');
  const read = (page) => page.evaluate(() => {
    const tree = document.querySelector('.syntax-tree');
    if (!tree) return null;
    return {
      ratio: window.devicePixelRatio,
      dp: parseFloat(getComputedStyle(tree).getPropertyValue('--dp')),
      lines: [...tree.querySelectorAll('.tree-line, .tree-branch')].map((e) => {
        const r = e.getBoundingClientRect(), down = e.classList.contains('tree-line');
        return { weight: down ? r.width : r.height, edges: [r.left, r.top, r.right, r.bottom] };
      }),
    };
  });
  const between = (look) => look.lines.flatMap((l) => l.edges).map((v) => v * look.ratio).find((v) => Math.abs(v - Math.round(v)) > 0.02);
  for (const scale of REAL_SCALES[t.engine]) {
    const at = `at a real ${Math.round(scale * 100)}% scale`;
    const browser = await launchBrowser(t.engine, executable, scale);
    try {
      const page = await browser.newPage();
      const errors = [];
      page.on('pageerror', (e) => {
        const text = String(e.message || e);
        if (!DEV_CLIENT_NOISE.test(text) && !DEV_SERVER_SWAP.test(text) && !NAVIGATION_ABORT.test(text)) errors.push(text.slice(0, 300));
      });
      const scaled = helpers(page, t.base, t.engine);
      await scaled.openFrom(t.base, FILES.sentences, t.engine === 'firefox' ? DESKTOP : null);
      await setBlock(scaled, 'Syntax Tree', true);
      // Read once the tree has settled on the grid (it measures where it landed, and moves onto the
      // grid a render later), and check it there.
      const check = async (when) => {
        let look = await read(page);
        assert(look, `${at} the open Syntax Tree is not the shared tree`);
        for (const until = Date.now() + 5000; between(look) !== undefined && Date.now() < until;) {
          await new Promise((r) => setTimeout(r, 100));
          look = await read(page);
        }
        assert(Math.abs(look.ratio - scale) < 0.01, `${at} the page reports a device pixel ratio of ${look.ratio}`);
        assert(Math.abs(look.dp * look.ratio - 1) < 1e-6, `${at} the tree takes a device pixel for ${look.dp}px, not ${1 / look.ratio}px`);
        assert(look.lines.length > 0, `${at} the tree draws no lines`);
        const weights = [...new Set(look.lines.map((l) => Math.round(l.weight * look.ratio * 100) / 100))];
        assert(weights.length === 1, `${at}, ${when}, the tree's lines are ${weights.join(', ')} device pixels wide — not one weight`);
        assert(Number.isInteger(weights[0]) && weights[0] >= 2, `${at}, ${when}, the tree's lines are ${weights[0]} device pixels wide, not a whole number of at least two`);
        const off = between(look);
        assert(off === undefined, `${at}, ${when}, a line's edge lands at ${off?.toFixed(3)} device pixels, between two`);
      };
      await check('as it opens');
      // Folding the Symbol Dictionary above it moves the tree up by the dictionary's height; the tree
      // keeps its lines on the grid wherever it lands.
      await setBlock(scaled, 'Symbol Dictionary', false);
      await check('with the Symbol Dictionary folded above it');
      assert(errors.length === 0, `${at} the page raised errors:\n${errors.join('\n')}`);
    } finally {
      await browser.close();
    }
  }
});

// Every pill that names a node's kind reads clearly: at least the rows' 13px, its words at 4.5:1
// or better against the tint behind them, inside its row, and in a colour of the one syntax palette.
scenario('the_syntax_tree_pills_read_clearly', async (t) => {
  await openSyntaxTree(t);
  const look = await t.page.evaluate(() => {
    const nums = (c) => {
      const srgb = c.match(/color\(srgb ([\d.]+) ([\d.]+) ([\d.]+)(?: \/ ([\d.]+))?\)/);
      if (srgb) return [srgb[1], srgb[2], srgb[3]].map((v) => Number(v) * 255).concat(srgb[4] === undefined ? 1 : Number(srgb[4]));
      const m = (c.match(/[\d.]+/g) || []).map(Number);
      return m.length === 3 ? [...m, 1] : m;
    };
    const lum = ([r, g, b]) => {
      const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
    };
    // The colour an element paints over: its own background laid over its ancestors' down to an opaque one.
    const under = (el) => {
      const layers = [];
      for (let n = el; n; n = n.parentElement) {
        const c = nums(getComputedStyle(n).backgroundColor);
        if (c.length === 4 && c[3] > 0) { layers.push(c); if (c[3] >= 1) break; }
      }
      let out = [0, 0, 0];
      for (const [r, g, b, a] of layers.reverse()) out = [r * a + out[0] * (1 - a), g * a + out[1] * (1 - a), b * a + out[2] * (1 - a)];
      return out;
    };
    const probe = document.createElement('span');
    document.querySelector('.studio-container').appendChild(probe);
    const tokens = ['quantifier', 'connective', 'predicate', 'individual', 'role', 'variable', 'modal', 'tense'].map((k) => {
      probe.style.color = `var(--syntax-${k})`;
      return getComputedStyle(probe).color;
    }).filter((c) => c && c !== getComputedStyle(document.querySelector('.studio-container')).color);
    probe.remove();
    const pills = [...document.querySelectorAll('.syntax-tree .tree-item .syntax-kind')].filter((p) => p.getBoundingClientRect().height > 0);
    return {
      tokens,
      pills: pills.map((p) => {
        const s = getComputedStyle(p), r = p.getBoundingClientRect(), row = p.closest('.tree-item').getBoundingClientRect();
        const fg = nums(s.color).slice(0, 3), bg = under(p);
        const [hi, lo] = [lum(fg), lum(bg)].sort((a, b) => b - a);
        return { text: p.innerText.trim(), size: parseFloat(s.fontSize), ratio: (hi + 0.05) / (lo + 0.05), color: s.color, inside: r.top >= row.top - 0.5 && r.bottom <= row.bottom + 0.5 };
      }),
    };
  });
  assert(look.pills.length > 0, 'the Syntax Tree names no node kinds in pills');
  assert(look.tokens.length === 8, `the Studio defines ${look.tokens.length} of the syntax palette's 8 tokens`);
  const off = [];
  for (const p of look.pills) {
    if (p.size < 13) off.push(`"${p.text}" is set at ${p.size}px, under 13px`);
    if (p.ratio < 4.5) off.push(`"${p.text}" reads at ${p.ratio.toFixed(2)}:1 against its tint, under 4.5:1`);
    if (!p.inside) off.push(`"${p.text}" spills out of its row`);
    if (!look.tokens.includes(p.color)) off.push(`"${p.text}" is ${p.color}, not a colour of the syntax palette`);
  }
  assert(off.length === 0, `${off.length} pill finding(s):\n${[...new Set(off)].slice(0, 12).join('\n')}`);
});

// The Syntax Tree says how to read it, above the tree and quietly: the top row is the whole sentence
// as parsed and each row below splits its phrase into parts; a legend names every pill colour the
// tree shows.
scenario('the_syntax_tree_says_how_to_read_it', async (t) => {
  await openSyntaxTree(t);
  const look = await t.page.evaluate(() => {
    const tree = document.querySelector('.syntax-tree');
    const body = tree && tree.closest('[role="region"]');
    if (!body) return null;
    const note = [...body.querySelectorAll('*')].find((el) => el.closest('.syntax-tree') === null && /whole sentence/i.test(el.innerText || '') && el.children.length <= 12 && /branch|splits|parts/i.test(el.innerText));
    const probe = document.createElement('span');
    probe.style.color = 'var(--studio-text-secondary)';
    body.appendChild(probe);
    const secondary = getComputedStyle(probe).color;
    probe.remove();
    const legend = [...body.querySelectorAll('.syntax-kind')].filter((p) => !p.closest('.syntax-tree'));
    const shown = [...new Set([...tree.querySelectorAll('.tree-item .syntax-kind')].map((p) => getComputedStyle(p).color))];
    return {
      note: note ? { text: note.innerText.trim(), color: getComputedStyle(note).color, size: parseFloat(getComputedStyle(note).fontSize), above: note.getBoundingClientRect().bottom <= tree.getBoundingClientRect().top + 0.5 } : null,
      secondary,
      legend: legend.map((p) => ({ text: p.innerText.trim(), color: getComputedStyle(p).color })),
      shown,
    };
  });
  assert(look, 'the Syntax Tree block holds no shared tree');
  assert(look.note, 'the Syntax Tree says nothing about how to read it (the whole sentence at the top, its parts below)');
  assert(look.note.above, 'the note on reading the tree is not above the tree');
  assert(look.note.color === look.secondary, `the note is ${look.note.color}, not the Studio's secondary text ${look.secondary}`);
  assert(look.note.size >= 12, `the note is set at ${look.note.size}px`);
  assert(look.legend.length > 0 && look.legend.every((l) => l.text), `the note shows no legend of the pill colours: ${JSON.stringify(look.legend)}`);
  const missing = look.shown.filter((c) => !look.legend.some((l) => l.color === c));
  assert(missing.length === 0, `the legend leaves out pill colour(s) the tree shows: ${missing.join(', ')}`);
});

// A node with parts opens and closes like a folder: every level starts open, pressing its chevron
// folds its parts away and turns the chevron back, pressing its row opens it again, and the block
// stays open throughout.
scenario('a_syntax_tree_node_opens_and_closes_like_a_folder', async (t) => {
  await openSyntaxTree(t);
  const read = () => t.page.evaluate(() => {
    const tree = document.querySelector('.syntax-tree');
    if (!tree) return null;
    const nodes = [...tree.querySelectorAll('.tree-node')].filter((n) => n.querySelector(':scope > .tree-item > .twistie'));
    return {
      rows: [...tree.querySelectorAll('.tree-item')].filter((r) => r.getBoundingClientRect().height > 0).length,
      closed: nodes.filter((n) => !n.querySelector(':scope > .tree-item .chevron.open')).length,
      nodes: nodes.length,
    };
  });
  const before = await read();
  assert(before && before.nodes > 1, 'the Syntax Tree shows no nodes with parts to open');
  assert(before.closed === 0, `${before.closed} of ${before.nodes} nodes start closed; every level starts open`);
  // The second node with parts: below the root, so its fold leaves the rest of the tree in place.
  const target = async () => (await t.page.evaluateHandle(() => [...document.querySelectorAll('.syntax-tree .tree-node')]
    .filter((n) => n.querySelector(':scope > .tree-item > .twistie'))[1])).asElement();
  let node = await target();
  await node.evaluate((n) => n.querySelector(':scope > .tree-item').scrollIntoView({ block: 'center' }));
  await t.frames(2);
  const chevron = await node.$(':scope > .tree-item > .twistie');
  const box = await chevron.boundingBox();
  await t.page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await new Promise((r) => setTimeout(r, 400));
  const folded = await read();
  node = await target();
  const state = () => node.evaluate((n) => ({ children: !!n.querySelector(':scope > .tree-children'), open: !!n.querySelector(':scope > .tree-item .chevron.open') }));
  let s = await state();
  assert(!s.children && !s.open && folded.rows < before.rows, `pressing a node's chevron did not fold its parts away (${before.rows} → ${folded.rows} rows, chevron ${s.open ? 'open' : 'closed'})`);
  const label = await node.$(':scope > .tree-item');
  await label.evaluate((row) => row.scrollIntoView({ block: 'center' }));
  await t.frames(2);
  const lb = await label.boundingBox();
  await t.page.mouse.click(lb.x + lb.width - 12, lb.y + lb.height / 2);
  await new Promise((r) => setTimeout(r, 400));
  s = await state();
  const again = await read();
  assert(s.children && s.open && again.rows === before.rows, `pressing the folded node's row did not open it again (${again.rows} of ${before.rows} rows)`);
  const block = logicBlock(await readLogicColumn(t.page), 'Syntax Tree');
  assert(block.expanded === 'true', 'opening and closing a node closed the Syntax Tree block');
});

// A theorem file has no syntax tree (its question is open with the compiler engineer:
// docs/plans/questions-for-tristen.md, item 1), so its Logic column shows the Symbol Dictionary
// block alone and no column beside it. A theorem tree must arrive as a deliberate change here.
scenario('a_theorem_shows_the_dictionary_and_no_syntax_tree', async (t) => {
  await t.open(FILES.logic);
  const look = await readLogicColumn(t.page);
  const titles = look.blocks.map((b) => b.title);
  assert(JSON.stringify(titles) === JSON.stringify(['Symbol Dictionary']), `a theorem's Logic column shows the blocks ${JSON.stringify(titles)}, not the Symbol Dictionary alone`);
  assert(!look.beside, `a theorem shows a column beside the Logic Output: ${JSON.stringify(look.beside)}`);
});

scenario('an_example_opens_without_squiggles', async (t) => {
  await t.open(FILES.code);
  await new Promise((r) => setTimeout(r, 1500));
  const loud = await t.page.$$eval('.studio-main > section .code-editor-highlight .sq-error, .studio-main > section .code-editor-highlight .sq-warning', (els) => els.map((e) => e.textContent));
  assert(loud.length === 0, `the Fibonacci example opens with squiggles under ${JSON.stringify(loud)}`);
});

// Logic's proof sits below the English input across a divider the reader can drag, the way
// the debugger's output does: the two share the column, each within its own bounds.
scenario('the_proof_divider_shares_the_logic_column', async (t) => {
  await t.open(FILES.logic);
  const grip = await t.page.$('.panel-content-stack > .stack-grip');
  assert(grip, 'the Logic column has no divider between the input and the proof');
  const heights = () => t.page.evaluate(() => ({
    input: document.querySelector('.panel-content-stack > .code-editor').getBoundingClientRect().height,
    proof: document.querySelector('.panel-content-stack > .stack-below').getBoundingClientRect().height,
  }));
  const before = await heights();
  await t.dragElement(grip, 0, -120);
  const raised = await heights();
  assert(near(raised.proof - before.proof, 120, 3), `dragging the divider up 120px grew the proof by ${px(raised.proof - before.proof)}`);
  assert(near(before.input - raised.input, 120, 3), `dragging the divider up 120px shrank the input by ${px(before.input - raised.input)}`);
  await t.dragElement(await t.page.$('.panel-content-stack > .stack-grip'), 0, 3000, 20);
  assert(near((await heights()).proof, 44, 1), `the proof shrank below its status line: ${px((await heights()).proof)}`);
  await t.dragElement(await t.page.$('.panel-content-stack > .stack-grip'), 0, -3000, 20);
  assert(near((await heights()).input, 120, 1), `the input shrank below a few lines: ${px((await heights()).input)}`);
});

scenario('the_debugger_opens_with_room_for_its_tabs', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  const body = await t.dbgBodyHeight();
  assert(body >= 100, `the debugger opens with only ${px(body)} for its tabs`);
});

// A VS Code-style status bar runs along the Studio's bottom edge: 25px tall under a 2px line in
// the column bars' grey, the full width of the window, in every mode. The debugger drawer docks
// on it, and it never pushes the page into scrolling — on a phone either.
scenario('the_status_bar_runs_along_the_bottom', async (t) => {
  const measure = () => t.page.evaluate(() => {
    const el = document.querySelector('.studio-statusbar');
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    const bar = document.querySelector('.panel-resizer');
    return {
      bottom: r.bottom, left: r.left, right: r.right, height: r.height,
      line: parseFloat(s.borderTopWidth), lineStyle: s.borderTopStyle, lineColor: s.borderTopColor,
      columnGrey: bar ? getComputedStyle(bar).backgroundColor : null,
      width: document.documentElement.clientWidth, heightOfWindow: innerHeight,
      scrollHeight: document.documentElement.scrollHeight, scrollWidth: document.documentElement.scrollWidth,
    };
  });
  for (const [kind, file] of EVERY_KIND) {
    await t.open(file);
    const bar = await measure();
    assert(bar, `${kind}: the Studio has no status bar`);
    assert(near(bar.height, STATUS_BAR_HEIGHT, 0.5), `${kind}: the status bar is ${px(bar.height)} tall, not ${STATUS_BAR_HEIGHT}px`);
    assert(near(bar.line, STATUS_BAR_LINE, 0.1) && bar.lineStyle === 'solid', `${kind}: the status bar's top line is ${bar.line}px ${bar.lineStyle}, not a ${STATUS_BAR_LINE}px line`);
    assert(bar.lineColor === bar.columnGrey, `${kind}: the status bar's line is ${bar.lineColor}, not the column bars' ${bar.columnGrey}`);
    assert(near(bar.bottom, bar.heightOfWindow, 0.5) && near(bar.left, 0, 0.5) && near(bar.right, bar.width, 0.5),
      `${kind}: the status bar does not run along the window's bottom edge (${JSON.stringify(bar)})`);
    assert(bar.scrollHeight <= bar.heightOfWindow, `${kind}: the status bar pushes the page into scrolling (${bar.scrollHeight}px in ${bar.heightOfWindow}px)`);
  }
  await t.open(FILES.code);
  await t.openDebugger();
  const edges = await t.page.evaluate(() => ({
    drawer: document.querySelector('.dbg-drawer').getBoundingClientRect().bottom,
    bar: document.querySelector('.studio-statusbar').getBoundingClientRect().top,
  }));
  assert(near(edges.drawer, edges.bar, 0.5), `the debugger drawer ends at ${px(edges.drawer)}, not on the status bar at ${px(edges.bar)}`);

  await t.open(FILES.code, PHONE);
  const phone = await measure();
  assert(phone && near(phone.bottom, phone.heightOfWindow, 0.5), `on a phone the status bar is not on the bottom edge (${JSON.stringify(phone)})`);
  assert(phone.scrollWidth <= phone.width, `on a phone the status bar makes the page scroll sideways (${phone.scrollWidth}px in ${phone.width}px)`);
});

// The status bar says where the caret is, the way VS Code does — "Ln 3, Col 4", and
// "(2 selected)" while text is selected — as the caret moves by keyboard, pointer or typing.
scenario('the_status_bar_follows_the_caret', async (t) => {
  await t.open(FILES.code);
  await paste(t, '## Main\nLet x be 1.\nShow x.\n');
  const position = async () => (await t.statusBar())?.position;
  const editor = await t.page.$('.studio-main > section textarea');
  await editor.click();
  const key = async (name, times = 1) => { for (let i = 0; i < times; i++) await t.page.keyboard.press(name); await t.frames(2); };
  await t.page.keyboard.down('Control');
  await key('Home');
  await t.page.keyboard.up('Control');
  assert(await position() === 'Ln 1, Col 1', `at the start of the file the status bar says ${JSON.stringify(await position())}`);
  await key('ArrowDown', 2);
  await key('ArrowRight', 3);
  assert(await position() === 'Ln 3, Col 4', `three characters into line 3 the status bar says ${JSON.stringify(await position())}`);
  await t.page.keyboard.down('Shift');
  await key('ArrowRight', 2);
  await t.page.keyboard.up('Shift');
  assert(await position() === 'Ln 3, Col 6 (2 selected)', `with two characters selected the status bar says ${JSON.stringify(await position())}`);

  const handle = await t.page.evaluateHandle(() =>
    [...document.querySelectorAll('.studio-main > section .code-editor-highlight span')].find((s) => s.textContent === 'Let'));
  const word = handle.asElement();
  assert(word, 'the editor shows no "Let" token');
  const box = await word.boundingBox();
  await t.page.mouse.click(box.x + 1, box.y + box.height / 2);
  await t.frames(2);
  assert(await position() === 'Ln 2, Col 1', `clicking before "Let" the status bar says ${JSON.stringify(await position())}`);
  await t.page.keyboard.type('ab');
  await t.frames(2);
  assert(await position() === 'Ln 2, Col 3', `after typing two characters the status bar says ${JSON.stringify(await position())}`);
});

// The status bar counts the problems the language server finds, as VS Code does: none in a clean
// program, the errors in a broken one, none again once it is fixed.
scenario('the_status_bar_counts_the_problems', async (t) => {
  await t.open(FILES.code);
  const counts = () => t.page.evaluate(() => {
    const p = document.querySelector('.studio-statusbar [data-status="problems"]');
    return p ? { errors: Number(p.dataset.errors), warnings: Number(p.dataset.warnings) } : null;
  });
  const settleOn = async (ok) => {
    const until = Date.now() + 5000;
    let c = await counts();
    while (!(c && ok(c)) && Date.now() < until) { await new Promise((r) => setTimeout(r, 150)); c = await counts(); }
    return c;
  };
  await paste(t, '## Main\nLet n be 1.\nShow n.\n');
  const clean = await settleOn((c) => c.errors === 0 && c.warnings === 0);
  assert(clean && clean.errors === 0 && clean.warnings === 0, `a clean program shows ${JSON.stringify(clean)} problems`);
  await paste(t, '## Main\nLet n: Int be "a".\nShow n.\n');
  const broken = await settleOn((c) => c.errors > 0);
  assert(broken && broken.errors >= 1, `a type error shows ${JSON.stringify(broken)} problems`);
  await paste(t, '## Main\nLet n be 1.\nShow n.\n');
  const fixed = await settleOn((c) => c.errors === 0);
  assert(fixed && fixed.errors === 0, `once fixed the status bar still shows ${JSON.stringify(fixed)} problems`);
});

// The status bar names the language the editor holds, as VS Code's language mode does.
scenario('the_status_bar_names_the_language', async (t) => {
  const expected = [
    [FILES.code, 'LOGOS'],
    [FILES.logic, 'Logic'],
    [FILES.math, 'Vernacular'],
    [FILES.threePanels, 'Hardware'],
    ['examples/hardware/arbiter.v', 'Verilog'],
    [FILES.web, 'LOGOS'],
  ];
  for (const [file, name] of expected) {
    await t.open(file);
    const shown = (await t.statusBar())?.language;
    assert(shown === name, `${file}: the status bar names the language ${JSON.stringify(shown)}, not ${name}`);
  }
});

// Enter keeps the line's indentation and deepens it after a line that opens a block — LOGOS's
// `:`, Math's `## Theorem:`, Verilog's `module …;` and `begin` — and a Verilog `end` steps back
// on its own. Tab and Shift-Tab indent and outdent without leaving the editor, a selection of
// lines moves together, each edit undoes like typing, and Escape then Tab leaves the editor.
// The status bar says how the editor indents, as VS Code's "Spaces: 4" does: the step Tab and
// Enter use — two spaces in Verilog, four everywhere else.
// Math files stay ASCII, and the editor shows the symbols they spell: `forall` is drawn as ∀ in one
// cell, `->` as →, while `funny` and `<->` stay as typed. Both of the editor's layers set the same
// font, so a click just past the ∀ puts the caret after `forall`. Code mode keeps its own font.
scenario('math_shows_its_symbols_in_place_of_their_spellings', async (t) => {
  await t.open(FILES.math);
  await paste(t, 'Check forall.\nCheck funny.\nCheck (A -> B).\nCheck (a <-> b).\n');
  await t.page.evaluate(() => document.fonts.ready);
  await t.frames(4);
  const seen = await t.page.evaluate(() => {
    const spans = [...document.querySelectorAll('.studio-main > section .code-editor-highlight span')];
    const box = (text) => spans.find((e) => e.textContent === text)?.getBoundingClientRect() ?? null;
    const area = document.querySelector('.studio-main > section textarea');
    const w = (text) => box(text)?.width ?? null;
    return {
      loaded: document.fonts.check("14px 'Logos Math Mono'"),
      textareaFont: getComputedStyle(area).fontFamily,
      highlightFont: getComputedStyle(area.previousElementSibling).fontFamily,
      cell: w('Check') / 5, forall: w('forall'), funny: w('funny'), arrow: w('->'), both: w('<->'),
      forallBox: box('forall') && { right: box('forall').right, y: box('forall').top + box('forall').height / 2 },
    };
  });
  assert(seen.loaded, 'the Math font never loaded');
  assert(/Logos Math Mono/.test(seen.textareaFont) && /Logos Math Mono/.test(seen.highlightFont),
    `the editor's layers do not both set the Math font: ${seen.textareaFont} / ${seen.highlightFont}`);
  assert(seen.forall !== null && seen.forall < 1.5 * seen.cell, `forall is ${px(seen.forall)} wide, not one ${px(seen.cell)} cell: it is not drawn as ∀`);
  assert(seen.arrow !== null && seen.arrow < 1.5 * seen.cell, `-> is ${px(seen.arrow)} wide, not one cell: it is not drawn as →`);
  assert(near(seen.funny, 5 * seen.cell, 1), `funny is ${px(seen.funny)} wide: a ligature fired inside a word`);
  assert(near(seen.both, 3 * seen.cell, 1), `<-> is ${px(seen.both)} wide: a ligature fired inside a longer operator`);

  // ∀ is one character to the reader: a click on its right half lands after it, and the arrow
  // keys step over it whole.
  await t.page.mouse.click(seen.forallBox.right - 1, seen.forallBox.y);
  await t.frames(2);
  let position = (await t.statusBar())?.position;
  assert(position === 'Ln 1, Col 13', `a click on the right of the ∀ put the caret at ${JSON.stringify(position)}, not after forall`);
  await t.page.keyboard.press('ArrowLeft');
  await t.frames(3);
  position = (await t.statusBar())?.position;
  assert(position === 'Ln 1, Col 7', `ArrowLeft from after the ∀ put the caret at ${JSON.stringify(position)}, not before forall`);
  await t.page.keyboard.press('ArrowRight');
  await t.frames(3);
  position = (await t.statusBar())?.position;
  assert(position === 'Ln 1, Col 13', `ArrowRight over the ∀ put the caret at ${JSON.stringify(position)}, not after forall`);

  await t.open(FILES.code);
  const codeFont = await t.page.$eval('.studio-main > section textarea', (e) => getComputedStyle(e).fontFamily);
  assert(!/Logos Math Mono/.test(codeFont), `Code mode sets the Math font: ${codeFont}`);
});

scenario('the_status_bar_says_how_the_editor_indents', async (t) => {
  for (const [file, indent] of [[FILES.code, 'Spaces: 4'], [FILES.logic, 'Spaces: 4'], [FILES.math, 'Spaces: 4'], ['examples/hardware/arbiter.v', 'Spaces: 2']]) {
    await t.open(file);
    const shown = (await t.statusBar())?.indent;
    assert(shown === indent, `${file}: the status bar says ${JSON.stringify(shown)}, not ${indent}`);
  }
});

scenario('enter_and_tab_indent_as_the_language_does', async (t) => {
  const value = () => t.page.$eval('.studio-main > section textarea', (e) => e.value);
  const chord = async (mod, key) => { await t.page.keyboard.down(mod); await t.page.keyboard.press(key); await t.page.keyboard.up(mod); await t.frames(2); };
  const write = async (keys) => {
    for (const k of keys) {
      if (k === '\n') await t.page.keyboard.press('Enter');
      else await t.page.keyboard.type(k);
    }
    await t.frames(2);
  };
  const startAtEnd = async (file, text) => {
    await t.open(file);
    await paste(t, text);
    await (await t.page.$('.studio-main > section textarea')).click();
    await chord('Control', 'End');
  };

  await startAtEnd(FILES.code, '## Main\nIf 1 is 1:');
  await write(['\n', 'Show 1.', '\n', 'Show 2.']);
  let got = await value();
  assert(got === '## Main\nIf 1 is 1:\n    Show 1.\n    Show 2.', `LOGOS: Enter indented ${JSON.stringify(got)}`);

  await chord('Shift', 'Tab');
  got = await value();
  assert(got.endsWith('\n    Show 1.\nShow 2.'), `Shift-Tab did not outdent the line: ${JSON.stringify(got)}`);
  assert(await t.page.evaluate(() => document.activeElement?.tagName === 'TEXTAREA'), 'Shift-Tab left the editor');
  await t.page.keyboard.press('Home');
  await t.page.keyboard.press('Tab');
  await t.frames(2);
  got = await value();
  assert(got.endsWith('\n    Show 1.\n    Show 2.'), `Tab at the start of the line did not indent it: ${JSON.stringify(got)}`);
  await chord('Control', 'z');
  got = await value();
  assert(got.endsWith('\n    Show 1.\nShow 2.'), `undo did not take back the Tab: ${JSON.stringify(got)}`);

  await chord('Control', 'Home');
  await t.page.keyboard.down('Shift');
  await t.page.keyboard.press('ArrowDown');
  await t.page.keyboard.press('ArrowDown');
  await t.page.keyboard.up('Shift');
  await t.page.keyboard.press('Tab');
  await t.frames(2);
  got = await value();
  assert(got.startsWith('    ## Main\n    If 1 is 1:\n    Show 1.'), `Tab over two selected lines did not indent both: ${JSON.stringify(got)}`);
  await chord('Shift', 'Tab');
  got = await value();
  assert(got.startsWith('## Main\nIf 1 is 1:\n    Show 1.'), `Shift-Tab over the selection did not give the lines back: ${JSON.stringify(got)}`);

  await t.page.keyboard.press('Escape');
  await t.page.keyboard.press('Tab');
  await t.frames(2);
  assert(await t.page.evaluate(() => document.activeElement?.tagName !== 'TEXTAREA'), 'Escape then Tab did not leave the editor');

  await startAtEnd(FILES.math, '## Theorem: Refl');
  await write(['\n', 'Statement: (Eq x x).']);
  got = await value();
  assert(got === '## Theorem: Refl\n    Statement: (Eq x x).', `Math: Enter after a theorem header gave ${JSON.stringify(got)}`);

  await startAtEnd('examples/hardware/arbiter.v', 'module m(input clk);');
  await write(['\n', 'always @(posedge clk) begin', '\n', 'end', '\n', 'endmodule']);
  got = await value();
  assert(got === 'module m(input clk);\n  always @(posedge clk) begin\n  end\n  endmodule', `Verilog: Enter gave ${JSON.stringify(got)}`);
});

scenario('the_scrollbars_paint_dark', async (t) => {
  await t.open(FILES.code);
  await t.openDebugger();
  await t.frames(4);
  const gutters = await t.page.evaluate(() => [...document.querySelectorAll('.studio-container *')].flatMap((el) => {
    const cs = getComputedStyle(el);
    const r = el.getBoundingClientRect();
    const [bl, br, bt, bb] = [cs.borderLeftWidth, cs.borderRightWidth, cs.borderTopWidth, cs.borderBottomWidth].map(parseFloat);
    const name = `${el.tagName.toLowerCase()}.${[...el.classList].join('.')}`;
    const across = el.offsetWidth - el.clientWidth - bl - br;
    const down = el.offsetHeight - el.clientHeight - bt - bb;
    const out = [];
    if (across >= 4 && el.clientHeight >= 40) out.push({ name, x: r.right - br - across, y: r.top + bt, width: across, height: el.clientHeight });
    if (down >= 4 && el.clientWidth >= 40) out.push({ name, x: r.left + bl, y: r.bottom - bb - down, width: el.clientWidth, height: down });
    return out.map((g) => {
      const x = Math.max(g.x, 0), y = Math.max(g.y, 0);
      return { ...g, x, y, width: Math.min(g.x + g.width, innerWidth) - x, height: Math.min(g.y + g.height, innerHeight) - y };
    }).filter((g) => g.width >= 1 && g.height >= 1);
  }));
  assert(gutters.some((g) => /file/.test(g.name)), `the file tree overflows, yet no scrollbar gutter was found to check: ${JSON.stringify(gutters.map((g) => g.name))}`);
  for (const g of gutters) {
    const share = await t.lightShare({ x: g.x, y: g.y, width: g.width, height: g.height });
    assert(share < 0.05, `${g.name} paints a light scrollbar: ${(share * 100).toFixed(0)}% of its ${px(g.width)}×${px(g.height)} gutter is light`);
  }
}, { engines: ['firefox'] });

scenario('phone_split_bar_is_shown_and_drags', async (t) => {
  await t.open(FILES.code, PHONE);
  const bar = await t.page.$('.mobile-panel-resizer');
  assert(bar && await bar.evaluate((el) => getComputedStyle(el).display !== 'none'), 'the phone layout hides the bar between its stacked panels');
  const before = await t.layout();
  await t.dragElement(bar, 0, 120);
  const after = await t.layout();
  assert(after.left.h - before.left.h > 60, `dragging the split bar down 120px grew the editor by only ${px(after.left.h - before.left.h)}`);
  assert(after.scrollWidth <= after.viewport, 'the phone layout scrolls sideways');
});

// ---------------------------------------------------------------------------------------
// The approved Studio changes of the homepage plan (step 5, D2, D3, D5)
// ---------------------------------------------------------------------------------------

// The Web preview is painted from the app's first frame on: the page shows in the frame at once, and
// a rebuild after an edit never flashes the frame blank.
scenario('a_web_page_paints_its_preview_from_the_first_frame', async (t) => {
  await t.open(FILES.web);
  // The preview is sandboxed away from the page, so it is read through the browser's own handle on
  // its frame. A frame between two documents cannot be read, and says so: null, never blank.
  const painted = async () => {
    const handle = await t.page.$('.studio-main iframe');
    if (!handle) return { titled: '', srcdoc: 0, elements: null };
    const [titled, srcdoc] = await handle.evaluate((f) => [f.getAttribute('title') || '', (f.getAttribute('srcdoc') || '').length]);
    const frame = await handle.contentFrame();
    const elements = frame ? await frame.evaluate(() => (document.body ? document.body.querySelectorAll('*').length : 0)).catch(() => null) : null;
    return { titled, srcdoc, elements };
  };
  const first = await painted();
  assert(first.titled === 'The page, built from the code', `the preview frame is titled ${JSON.stringify(first.titled)}`);
  assert(first.srcdoc > 0 && first.elements > 0, `the app's first frame shows an empty preview (${JSON.stringify(first)})`);
  const editor = await t.page.$('.studio-main > section textarea');
  await editor.click();
  await t.page.keyboard.down('Control');
  await t.page.keyboard.press('End');
  await t.page.keyboard.up('Control');
  await t.page.keyboard.type('\n');
  const blank = [];
  const until = Date.now() + 2000;
  while (Date.now() < until) {
    const now = await painted();
    if (now.elements === 0) blank.push(now);
    await t.frames(1);
  }
  assert(blank.length === 0, `a rebuild flashed the preview blank ${blank.length} times`);
});

// The Files tree is one tab stop, and the arrows, Home, End and Enter walk and open it (WAI-ARIA).
scenario('the_file_tree_answers_the_keyboard', async (t) => {
  await t.open(FILES.code);
  const look = () => t.page.evaluate(() => {
    const rows = [...document.querySelectorAll('.file-tree [role="treeitem"]')];
    const active = document.activeElement;
    return {
      stops: rows.filter((r) => r.tabIndex === 0).map((r) => r.title),
      focused: rows.includes(active) ? active.title : null,
      expanded: active && rows.includes(active) ? active.getAttribute('aria-expanded') : null,
      rows: rows.map((r) => r.title),
      selected: rows.filter((r) => r.getAttribute('aria-selected') === 'true').map((r) => r.title),
      ring: active && rows.includes(active) ? getComputedStyle(active).outlineStyle : null,
      tree: document.querySelector('.file-tree')?.getAttribute('role'),
    };
  });
  const key = async (name) => { await t.page.keyboard.press(name); await t.frames(3); return look(); };
  let now = await look();
  assert(now.tree === 'tree', `the Files tree is role=${now.tree}`);
  assert(now.stops.length === 1 && now.stops[0] === `/${FILES.code}`, `the tree's tab stops are ${JSON.stringify(now.stops)}, not the open file`);
  await t.page.focus(`.file-tree [role="treeitem"][title="/${FILES.code}"]`);
  now = await key('ArrowDown');
  const at = now.rows.indexOf(`/${FILES.code}`);
  assert(now.focused === now.rows[at + 1], `ArrowDown moved to ${now.focused}, not ${now.rows[at + 1]}`);
  assert(now.ring !== 'none', 'a row the keyboard reached shows no focus ring');
  now = await key('ArrowUp');
  assert(now.focused === `/${FILES.code}`, `ArrowUp moved to ${now.focused}`);
  now = await key('Home');
  assert(now.focused === now.rows[0], `Home moved to ${now.focused}, not the first row`);
  now = await key('End');
  assert(now.focused === now.rows[now.rows.length - 1], `End moved to ${now.focused}, not the last row`);
  await t.page.focus(`.file-tree [role="treeitem"][title="/${FILES.code}"]`);
  now = await key('ArrowLeft');
  const folder = `/${FILES.code.split('/').slice(0, -1).join('/')}`;
  assert(now.focused === folder, `ArrowLeft on a file moved to ${now.focused}, not its folder ${folder}`);
  now = await key('ArrowLeft');
  assert(now.expanded === 'false' && !now.rows.includes(`/${FILES.code}`), `ArrowLeft on an open folder left it ${now.expanded === 'false' ? 'closed but drawn' : 'open'}`);
  now = await key('ArrowRight');
  assert(now.expanded === 'true', 'ArrowRight on a closed folder did not open it');
  now = await key('ArrowRight');
  assert(now.focused === now.rows[now.rows.indexOf(folder) + 1], `ArrowRight on an open folder moved to ${now.focused}, not its first part`);
  const target = now.focused;
  now = await key('Enter');
  await t.page.waitForFunction((path) => document.querySelector('.file-tree [aria-selected="true"]')?.title === path, { timeout: 15000 }, target);
  now = await look();
  assert(now.selected[0] === target && now.stops.length === 1 && now.stops[0] === target, `Enter did not open ${target}: selected ${JSON.stringify(now.selected)}, tab stops ${JSON.stringify(now.stops)}`);
});

// What the reader wrote in a file survives opening another and coming back, for the visit.
scenario('an_edit_survives_opening_another_file', async (t) => {
  await t.open(FILES.code);
  const editor = await t.page.$('.studio-main > section textarea');
  await editor.click();
  await t.page.keyboard.down('Control');
  await t.page.keyboard.press('End');
  await t.page.keyboard.up('Control');
  await t.page.keyboard.type('\nShow "kept".');
  await t.frames(3);
  const edited = await t.page.$eval('.studio-main > section textarea', (e) => e.value);
  assert(edited.endsWith('Show "kept".'), 'typing did not reach the editor');
  const open = async (path) => {
    await t.page.click(`.file-tree [role="treeitem"][title="/${path}"]`);
    await t.page.waitForFunction((p) => document.querySelector('.file-tree [aria-selected="true"]')?.title === `/${p}`, { timeout: 15000 }, path);
    await t.frames(4);
  };
  await open(FILES.sentences);
  const other = await t.page.$eval('.studio-main > section textarea', (e) => e.value);
  assert(!other.includes('Show "kept".'), 'another file opened with the edit');
  await open(FILES.code);
  const back = await t.page.$eval('.studio-main > section textarea', (e) => e.value);
  assert(back === edited, `coming back to ${FILES.code} lost the edit (it ends ${JSON.stringify(back.slice(-30))})`);
});

// On a phone the Files tree is folded from the first frame: no tree ever covers the editor while the
// page loads, and the toggle opens it.
scenario('a_phone_opens_with_the_files_folded_from_the_first_frame', async (t) => {
  await t.page.evaluateOnNewDocument(() => {
    window.__treeSeen = 0;
    const look = () => {
      for (const s of document.querySelectorAll('.studio-sidebar')) {
        const r = s.getBoundingClientRect();
        if (getComputedStyle(s).display !== 'none' && r.width > 0 && r.height > 0) window.__treeSeen++;
      }
      requestAnimationFrame(look);
    };
    requestAnimationFrame(look);
  });
  await t.open(FILES.code, PHONE);
  const seen = await t.page.evaluate(() => window.__treeSeen);
  assert(seen === 0, `the Files tree showed over the editor in ${seen} frames while the page loaded`);
  await t.page.click('.sidebar-toggle-btn');
  await t.frames(4);
  const shown = await t.page.$eval('.studio-sidebar', (s) => getComputedStyle(s).display !== 'none' && s.getBoundingClientRect().width > 0).catch(() => false);
  assert(shown, 'the toggle did not open the Files tree');
});

// ---------------------------------------------------------------------------------------
// Phones, tablets and touch (3.8 of the homepage plan)
// ---------------------------------------------------------------------------------------

// Devices as the browser emulates them: a mobile viewport, its device scale, and touch.
const DEVICES = [
  { name: 'phone', band: 'phone', width: 390, height: 844, deviceScaleFactor: 3, isMobile: true, hasTouch: true },
  { name: 'small phone', band: 'phone', width: 360, height: 740, deviceScaleFactor: 3, isMobile: true, hasTouch: true },
  { name: 'phone on its side', band: 'phone-landscape', width: 844, height: 390, deviceScaleFactor: 3, isMobile: true, hasTouch: true, isLandscape: true },
  { name: 'iPad upright', band: 'tablet-portrait', width: 768, height: 1024, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
  { name: 'iPad Air upright', band: 'tablet-portrait', width: 820, height: 1180, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
  { name: 'iPad Air on its side', band: 'tablet-landscape', width: 1180, height: 820, deviceScaleFactor: 2, isMobile: true, hasTouch: true, isLandscape: true },
  { name: 'desktop', band: 'desktop', width: 1280, height: 900 },
  { name: 'wide desktop', band: 'desktop', width: 1920, height: 1080 },
];
const TOUCH_DEVICES = DEVICES.filter((d) => d.hasTouch);
// A file of every mode, each opening its own panes.
const BAND_FILES = [FILES.sentences, FILES.code, FILES.math, FILES.threePanels, FILES.web];

// What the workbench lays out: the band the page's own queries say it is in, which panes show and
// where, the switch, the files and the bars.
async function readBand(page) {
  return page.evaluate(() => {
    const q = (s) => matchMedia(s).matches;
    const band = q('(max-width: 600px)') ? 'phone'
      : q('(max-height: 500px) and (orientation: landscape)') ? 'phone-landscape'
      : q('(min-width: 601px) and (max-width: 768px)') || q('(min-width: 769px) and (max-width: 1024px) and (orientation: portrait) and (pointer: coarse)') ? 'tablet-portrait'
      : q('(min-width: 769px) and (min-height: 501px) and (pointer: coarse)') ? 'tablet-landscape' : 'desktop';
    const shown = (el) => !!el && getComputedStyle(el).display !== 'none' && el.getBoundingClientRect().width > 0 && el.getBoundingClientRect().height > 0;
    const rect = (el) => { const r = el.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; };
    const panes = [...document.querySelectorAll('.studio-main > .studio-panel')].map((p) => ({ col: ['col-left', 'col-center', 'col-right'].find((c) => p.classList.contains(c)), shown: shown(p), ...rect(p) }));
    const tabs = [...document.querySelectorAll('.mobile-tabs [role="tab"]')].filter(shown).map((b) => b.innerText.trim());
    return {
      band,
      coarse: q('(pointer: coarse)'),
      panes: panes.filter((p) => p.shown),
      switchShown: shown(document.querySelector('.mobile-tabs')),
      tabs,
      sidebarShown: shown(document.querySelector('.studio-sidebar')),
      columnBars: [...document.querySelectorAll('.panel-resizer')].filter(shown).length,
      splitBar: shown(document.querySelector('.mobile-panel-resizer')),
      scrollWidth: document.documentElement.scrollWidth,
      viewport: document.documentElement.clientWidth,
      statusBar: shown(document.querySelector('.studio-statusbar')),
    };
  });
}

// Each band lays the workbench out as designed (3.8): one pane on a phone with the switch, halves
// on its side, the editor over the output on an upright tablet, the desktop's columns on a tablet on
// its side; the files a drawer, folded, wherever the panes stack or a phone is turned.
scenario('each_band_lays_the_workbench_out_as_designed', async (t) => {
  const off = [];
  for (const device of DEVICES) {
    if (device.hasTouch && t.engine === 'firefox' && /tablet/.test(device.band)) continue;
    for (const file of BAND_FILES) {
      await t.fresh();
      await t.open(file, device);
      await new Promise((r) => setTimeout(r, 800));
      const b = await readBand(t.page);
      const at = `${device.name} (${device.width}×${device.height}) on ${file}`;
      if (device.hasTouch && t.engine === 'chrome' && b.band !== device.band) off.push(`${at}: the page reads as ${b.band}, not ${device.band}`);
      const cols = b.panes.map((p) => p.col);
      if (b.band === 'phone') {
        if (b.panes.length !== 1) off.push(`${at}: ${b.panes.length} panes show, not one`);
        if (!b.switchShown || b.tabs.length < 2) off.push(`${at}: the switch shows ${JSON.stringify(b.tabs)}`);
        if (b.panes[0] && Math.abs(b.panes[0].w - b.viewport) > 1) off.push(`${at}: the pane is ${b.panes[0].w}px wide in a ${b.viewport}px window`);
        if (b.splitBar || b.columnBars) off.push(`${at}: a bar shows`);
      } else if (b.band === 'phone-landscape') {
        if (b.panes.length !== 2 || cols[0] !== 'col-left') off.push(`${at}: the panes are ${JSON.stringify(cols)}, not the editor beside one other`);
        else if (Math.abs(b.panes[0].w - b.panes[1].w) > 2 || Math.abs(b.panes[0].y - b.panes[1].y) > 1) off.push(`${at}: the halves are ${b.panes[0].w}px and ${b.panes[1].w}px`);
        if (b.statusBar) off.push(`${at}: the status bar takes the height`);
        if (b.splitBar || b.columnBars) off.push(`${at}: a bar shows`);
      } else if (b.band === 'tablet-portrait') {
        if (b.panes.length !== 2 || cols[0] !== 'col-left') off.push(`${at}: the panes are ${JSON.stringify(cols)}, not the editor over one other`);
        else if (!(b.panes[0].y + b.panes[0].h <= b.panes[1].y + 1)) off.push(`${at}: the editor is not over the lower pane`);
        if (!b.splitBar) off.push(`${at}: no split bar between the editor and the lower pane`);
        if (b.columnBars) off.push(`${at}: a column bar shows`);
      } else {
        if (b.panes.length < 2 || b.panes.some((p, i) => i > 0 && p.x < b.panes[i - 1].x + b.panes[i - 1].w - 1)) off.push(`${at}: the panes are not side by side`);
        if (b.switchShown) off.push(`${at}: the phone's switch shows`);
      }
      const drawer = ['phone', 'phone-landscape', 'tablet-portrait'].includes(b.band) && (b.band !== 'tablet-portrait' || device.width <= 768 || b.coarse);
      if (drawer && b.sidebarShown) off.push(`${at}: the files are open over the workbench at first`);
      if (!drawer && b.band !== 'tablet-portrait' && !b.sidebarShown) off.push(`${at}: the Files column is folded at first`);
      if (b.scrollWidth > b.viewport) off.push(`${at}: the page scrolls sideways (${b.scrollWidth}px in ${b.viewport}px)`);
    }
  }
  assert(off.length === 0, `${off.length} band finding(s):\n${off.join('\n')}`);
}, { allowErrors: NAVIGATION_ABORT, timeout: 1800000 });

// On every touch device the workbench fits: no sideways page scroll, every control a finger wide,
// the mode's actions on screen, and the editor and the output each shown or one tap away. A line
// typed into the editor and the mode's action show output.
scenario('the_workbench_fits_every_device', async (t) => {
  const off = [];
  for (const device of TOUCH_DEVICES) {
    for (const file of [FILES.code, FILES.sentences]) {
      await t.fresh();
      await t.open(file, device);
      await new Promise((r) => setTimeout(r, 800));
      const at = `${device.name} on ${file}`;
      const small = await t.page.evaluate(() => {
        const shown = (el) => { const r = el.getBoundingClientRect(); return getComputedStyle(el).display !== 'none' && getComputedStyle(el).visibility !== 'hidden' && r.width > 0 && r.height > 0; };
        return [...document.querySelectorAll('.studio-container a, .studio-container button, .studio-container [role="tab"], .studio-container [role="treeitem"]')]
          .filter(shown)
          .filter((el) => !el.closest('.code-editor, .main-nav, .mobile-drawer, .mobile-drawer-overlay'))
          .map((el) => ({ what: `${el.tagName.toLowerCase()}.${[...el.classList].join('.')} ${JSON.stringify((el.innerText || el.getAttribute('aria-label') || el.title || '').trim().slice(0, 24))}`, w: el.getBoundingClientRect().width, h: el.getBoundingClientRect().height }))
          .filter((c) => c.w < 43.5 || c.h < 43.5);
      });
      for (const c of small) off.push(`${at}: ${c.what} is ${px(c.w)} × ${px(c.h)}`);
      const fit = await t.page.evaluate(() => {
        const actions = [...document.querySelectorAll('.studio-toolbar .execute-btn')].map((b) => b.getBoundingClientRect());
        return { scrollWidth: document.documentElement.scrollWidth, viewport: document.documentElement.clientWidth, actionsOff: actions.filter((r) => r.right > innerWidth + 0.5 || r.left < -0.5 || r.bottom > innerHeight).length };
      });
      if (fit.scrollWidth > fit.viewport) off.push(`${at}: the page scrolls sideways (${fit.scrollWidth}px in ${fit.viewport}px)`);
      if (fit.actionsOff) off.push(`${at}: ${fit.actionsOff} actions are off screen`);
      const reach = async (which) => {
        const shownNow = await t.page.$eval(`.studio-main > .${which}`, (p) => getComputedStyle(p).display !== 'none').catch(() => false);
        if (shownNow) return true;
        const tab = await t.page.$(`.mobile-tabs [role="tab"]:nth-child(${which === 'col-left' ? 1 : 2})`);
        if (!tab || !(await tab.isVisible())) return false;
        await tab.tap();
        await t.frames(4);
        return t.page.$eval(`.studio-main > .${which}`, (p) => getComputedStyle(p).display !== 'none').catch(() => false);
      };
      if (!(await reach('col-left'))) off.push(`${at}: the editor is neither shown nor one tap away`);
      const editor = await t.page.$('.studio-main > .col-left textarea');
      await editor.tap();
      await t.page.keyboard.down('Control');
      await t.page.keyboard.press('End');
      await t.page.keyboard.up('Control');
      await t.page.keyboard.type(file === FILES.code ? '\nShow "fits".' : ' ');
      await press(t.page, file === FILES.code ? 'Run' : 'Execute');
      if (!(await reach('col-center'))) off.push(`${at}: the output is neither shown nor one tap away`);
      const said = await t.page.waitForFunction((code) => {
        const out = document.querySelector('.studio-main > .col-center .panel-content');
        return out && (code ? out.innerText.includes('fits') : out.innerText.trim().length > 0);
      }, { timeout: 20000 }, file === FILES.code).then(() => true, () => false);
      if (!said) off.push(`${at}: the mode's action showed no output`);
    }
  }
  assert(off.length === 0, `${off.length} finding(s):\n${off.join('\n')}`);
}, { engines: ['chrome'], allowErrors: NAVIGATION_ABORT, timeout: 1800000 });

// On a phone the switch shows one pane at a time: the output first for Code, and a tap on Editor
// shows the editor at the pane's whole height.
scenario('a_phone_switches_between_the_editor_and_the_output', async (t) => {
  await t.open(FILES.code, DEVICES[0]);
  const look = () => readBand(t.page);
  let b = await look();
  assert(b.switchShown, 'the phone shows no switch');
  assert(b.panes.length === 1 && b.panes[0].col === 'col-center', `a phone opens Code on ${JSON.stringify(b.panes.map((p) => p.col))}, not its output`);
  const selected = await t.page.$eval('.mobile-tabs [aria-selected="true"]', (el) => el.innerText.trim());
  assert(/Output/.test(selected), `the switch selects ${JSON.stringify(selected)}, not Output`);
  const editorTab = await t.page.$('.mobile-tabs [role="tab"]:nth-child(1)');
  assert(editorTab && await editorTab.isVisible(), 'the switch has no Editor tab to tap');
  await editorTab.tap();
  await t.frames(4);
  b = await look();
  assert(b.panes.length === 1 && b.panes[0].col === 'col-left', `after a tap on Editor the phone shows ${JSON.stringify(b.panes.map((p) => p.col))}`);
  const room = await t.page.evaluate(() => {
    const pane = document.querySelector('.studio-main > .col-left').getBoundingClientRect();
    const main = document.querySelector('.studio-main').getBoundingClientRect();
    return { pane: pane.height, main: main.height };
  });
  assert(Math.abs(room.pane - room.main) <= 1, `the editor takes ${px(room.pane)} of the pane's ${px(room.main)}`);
  assert(b.scrollWidth <= b.viewport, 'the phone scrolls sideways');
}, { engines: ['chrome'], allowErrors: NAVIGATION_ABORT });

// An upright tablet keeps the split bar between the editor and the output, and a finger drags it.
scenario('a_tablet_split_bar_drags_with_a_finger', async (t) => {
  await t.open(FILES.code, DEVICES[4]);
  const bar = await t.page.$('.mobile-panel-resizer');
  assert(bar && await bar.evaluate((el) => getComputedStyle(el).display !== 'none'), 'the upright tablet shows no split bar');
  const grab = await bar.evaluate((el) => { const r = el.getBoundingClientRect(); const before = getComputedStyle(el, '::before'); return r.height + (parseFloat(before.bottom) < 0 ? -parseFloat(before.bottom) - parseFloat(before.top) : 0); });
  assert(grab >= 43.5, `the split bar's grab area is ${px(grab)} tall`);
  const box = await bar.boundingBox();
  const before = await t.layout();
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  const touch = await t.page.touchscreen.touchStart(x, y);
  for (let i = 1; i <= 12; i++) await touch.move(x, y + (120 * i) / 12);
  await touch.end();
  await t.frames(4);
  const after = await t.layout();
  assert(after.left.h - before.left.h > 60, `a finger dragging the split bar down 120px grew the editor by only ${px(after.left.h - before.left.h)}`);
  assert(after.scrollWidth <= after.viewport, 'the tablet scrolls sideways');
}, { engines: ['chrome'], allowErrors: NAVIGATION_ABORT });

// A tablet on its side keeps the desktop's column bars, each drawn 6px and grabbed 44px wide, and a
// finger moves a boundary.
scenario('the_column_bars_take_a_finger_on_a_tablet', async (t) => {
  await t.open(FILES.threePanels, DEVICES[5]);
  const bars = await t.bars();
  assert(bars.length >= 2, `the tablet on its side shows ${bars.length} column bars`);
  for (const bar of bars) {
    const look = await bar.evaluate((el) => { const r = el.getBoundingClientRect(); const after = getComputedStyle(el, '::after'); return { drawn: r.width, grab: r.width - parseFloat(after.left) - parseFloat(after.right) }; });
    assert(near(look.drawn, BAR, 0.5), `a column bar is drawn ${px(look.drawn)} wide, not ${BAR}px`);
    assert(look.grab >= 43.5, `a column bar's grab area is ${px(look.grab)} wide`);
  }
  const before = await t.layout();
  const box = await bars[1].boundingBox();
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  const touch = await t.page.touchscreen.touchStart(x, y);
  for (let i = 1; i <= 12; i++) await touch.move(x + (80 * i) / 12, y);
  await touch.end();
  await t.frames(4);
  const after = await t.layout();
  assert(after.left.w - before.left.w > 40, `a finger dragging the bar 80px moved the boundary by ${px(after.left.w - before.left.w)}`);
}, { engines: ['chrome'], allowErrors: NAVIGATION_ABORT });

// The on-screen keyboard never hides the caret's line: with the caret on the editor's last line, the
// window loses 320px of height (Android's resizes-content) and the line is still in view.
scenario('the_keyboard_never_hides_the_caret_line', async (t) => {
  const device = DEVICES[0];
  await t.open(FILES.code, device);
  const editorTab = await t.page.$('.mobile-tabs [role="tab"]:nth-child(1)');
  if (editorTab && await editorTab.isVisible()) { await editorTab.tap(); await t.frames(3); }
  await paste(t, Array.from({ length: 80 }, (_, i) => `Let x${i} be ${i}.`).join('\n'));
  const editor = await t.page.$('.studio-main > .col-left textarea');
  await editor.tap();
  await t.page.keyboard.down('Control');
  await t.page.keyboard.press('End');
  await t.page.keyboard.up('Control');
  await t.page.setViewport({ ...device, height: device.height - 320 });
  await new Promise((r) => setTimeout(r, 400));
  await t.page.keyboard.type('\nShow x79.');
  await new Promise((r) => setTimeout(r, 300));
  const seen = await t.page.evaluate(() => {
    const ta = document.querySelector('.studio-main > .col-left textarea');
    const cs = getComputedStyle(ta);
    const lh = parseFloat(cs.lineHeight);
    const lines = ta.value.slice(0, ta.selectionStart).split('\n').length - 1;
    const r = ta.getBoundingClientRect();
    const top = r.top + parseFloat(cs.paddingTop) + lines * lh - ta.scrollTop;
    const view = { top: Math.max(r.top, 0), bottom: Math.min(r.bottom, (window.visualViewport ? visualViewport.height : innerHeight)) };
    return { top, bottom: top + lh, view };
  });
  assert(seen.top >= seen.view.top - 1 && seen.bottom <= seen.view.bottom + 1, `the caret's line sits at ${px(seen.top)}–${px(seen.bottom)}, outside the ${px(seen.view.top)}–${px(seen.view.bottom)} the keyboard leaves`);
}, { engines: ['chrome'], allowErrors: NAVIGATION_ABORT });

// What a hover shows on a desk, a finger reaches: a disabled action's reason is a line on screen, a
// tap on a word with a squiggle shows its problem, and the fault note opens its whole story.
scenario('a_touch_reaches_what_a_hover_shows', async (t) => {
  await t.open('workspace/untitled.logos', DEVICES[0]);
  const reasons = await t.page.$eval('.studio-reasons', (el) => getComputedStyle(el).display !== 'none' ? el.innerText : '').catch(() => '');
  assert(/Run: The editor is empty/.test(reasons) && /Compile: The editor is empty/.test(reasons), `a disabled action's reason is not on screen: ${JSON.stringify(reasons)}`);
  await t.open(FILES.code, DEVICES[0]);
  const editorTab = await t.page.$('.mobile-tabs [role="tab"]:nth-child(1)');
  if (editorTab && await editorTab.isVisible()) { await editorTab.tap(); await t.frames(3); }
  await paste(t, '## Main\nShow undefinedname.\n');
  await t.page.waitForSelector('.studio-main > .col-left .code-editor-highlight [class*="squiggle"]', { timeout: 15000 });
  const word = await t.page.$('.studio-main > .col-left .code-editor-highlight [class*="squiggle"]');
  const box = await word.boundingBox();
  await t.page.touchscreen.tap(box.x + box.width / 2, box.y + box.height / 2);
  const hint = await t.page.waitForSelector('.hover-hint', { visible: true, timeout: 5000 }).then(() => true, () => false);
  assert(hint, 'a tap on a word with a squiggle shows no problem');
}, { engines: ['chrome'], allowErrors: NAVIGATION_ABORT });

// ---------------------------------------------------------------------------------------
// The desktop Studio stays as it is
// ---------------------------------------------------------------------------------------

// /studio in every mode, the shots the desktop is held to.
const DESKTOP_SHOTS = [
  ['studio', null],
  ['logic-sentences', 'examples/logic/simple-sentences.logic'],
  ['logic-theorem', 'examples/logic/syllogism.logic'],
  ['code', 'examples/code/hello-world.logos'],
  ['math', 'examples/math/auto-tactic.logos'],
  ['hardware-spec', 'examples/hardware/handshake.hw'],
  ['hardware-verilog', 'examples/hardware/arbiter.v'],
  ['web', 'examples/code/web/counter.logos'],
];

// The share of two same-sized screenshots' pixels that differ by more than a rounding's worth.
async function changedShare(page, before, after) {
  return page.evaluate(async (a, b) => {
    const pixels = async (src) => {
      const img = new Image();
      img.src = src;
      await img.decode();
      const canvas = document.createElement('canvas');
      canvas.width = img.width;
      canvas.height = img.height;
      const g = canvas.getContext('2d');
      g.drawImage(img, 0, 0);
      return { w: img.width, h: img.height, d: g.getImageData(0, 0, img.width, img.height).data };
    };
    const [x, y] = [await pixels(a), await pixels(b)];
    if (x.w !== y.w || x.h !== y.h) return 1;
    let changed = 0;
    for (let i = 0; i < x.d.length; i += 4) {
      if (Math.max(Math.abs(x.d[i] - y.d[i]), Math.abs(x.d[i + 1] - y.d[i + 1]), Math.abs(x.d[i + 2] - y.d[i + 2])) > 32) changed++;
    }
    return changed / (x.d.length / 4);
  }, `data:image/png;base64,${before}`, `data:image/png;base64,${after}`);
}

// The phone and tablet work changes nothing on a desktop: every rule it adds sits under a phone,
// tablet or touch query. /studio in every mode at 1920x1080 and 1280x900, held to the shots recorded
// into --desktop-baseline before that work, each within a small share of changed pixels. A shot the
// baseline lacks is recorded there, and the run says so.
scenario('the_desktop_studio_is_unchanged', async (t) => {
  const dir = path.join(t.run.desktopBaseline, t.engine);
  fs.mkdirSync(dir, { recursive: true });
  const recorded = [];
  const changed = [];
  for (const [width, height] of [[1920, 1080], [1280, 900]]) {
    for (const [name, file] of DESKTOP_SHOTS) {
      await t.fresh();
      if (file) {
        await t.open(file, { width, height });
      } else {
        await t.page.setViewport({ width, height });
        await t.load(`${t.base}/studio`);
        await t.page.waitForFunction(() => document.querySelector('.studio-main > section textarea')?.value.trim().length > 0, { timeout: 120000 });
      }
      await new Promise((r) => setTimeout(r, 2500));
      // Still pictures: every animation at its start and no transition under way, in both engines.
      await t.page.addStyleTag({ content: '*, *::before, *::after { animation: none !important; transition: none !important; caret-color: transparent !important; }' });
      await t.frames(6);
      const shot = await t.page.screenshot({ encoding: 'base64' });
      const at = path.join(dir, `${name}-${width}x${height}.png`);
      if (!fs.existsSync(at)) {
        fs.writeFileSync(at, Buffer.from(shot, 'base64'));
        recorded.push(at);
        continue;
      }
      const share = await changedShare(t.page, fs.readFileSync(at).toString('base64'), shot);
      if (share > 0.002) {
        fs.writeFileSync(path.join(t.run.shots, `desktop-${t.engine}-${name}-${width}x${height}.png`), Buffer.from(shot, 'base64'));
        changed.push(`${name} at ${width}×${height}: ${(share * 100).toFixed(2)}% of its pixels changed`);
      }
    }
  }
  if (recorded.length) console.log(`      recorded ${recorded.length} desktop shots in ${dir}`);
  assert(changed.length === 0, `the desktop Studio changed (this run's shots are in ${t.run.shots}):\n${changed.join('\n')}`);
}, { needs: { arg: 'desktopBaseline', why: 'no --desktop-baseline to hold the desktop to' }, allowErrors: NAVIGATION_ABORT, timeout: 1200000 });

// ---------------------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------------------

// A headless browser of `engine`. `scale` launches it at that device scale, the way a screen's
// density or a browser zoom sets it (see REAL_SCALES); without it the device pixel is the CSS pixel.
function launchBrowser(engine, executablePath, scale = null) {
  const puppeteer = require('puppeteer-core');
  // A cold first load after a rebuild can outlast puppeteer's default 180 s protocol ceiling;
  // `open` already allows 300 s, so the ceiling must sit above every wait we ask for.
  const protocolTimeout = 660000;
  if (engine === 'firefox') {
    const extraPrefsFirefox = scale ? { 'layout.css.devPixelsPerPx': String(scale) } : {};
    return puppeteer.launch({ browser: 'firefox', executablePath, headless: true, protocolTimeout, extraPrefsFirefox });
  }
  const args = ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage'];
  // A viewport set through the protocol would emulate a scale over the real one, so a scaled
  // Chrome keeps its window, sized here.
  if (scale) args.push(`--force-device-scale-factor=${scale}`, `--window-size=${DESKTOP.width},${DESKTOP.height}`);
  return puppeteer.launch({ executablePath, headless: true, protocolTimeout, args, ...(scale ? { defaultViewport: null } : {}) });
}

function xmlEscape(s) {
  return String(s).replace(/[<>&"']/g, (c) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&apos;' }[c]));
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.list) {
    for (const s of scenarios) console.log(s.name);
    return 0;
  }
  if (!opts.url && (!opts.public || !fs.existsSync(path.join(opts.public, 'index.html')))) {
    console.error('studio-e2e: give --url <running site> or --public <built site with index.html>');
    return 2;
  }
  const selected = opts.only ? scenarios.filter((s) => opts.only.includes(s.name)) : scenarios;
  const unknown = (opts.only || []).filter((n) => !scenarios.some((s) => s.name === n));
  if (unknown.length) { console.error(`studio-e2e: unknown scenario(s): ${unknown.join(', ')}`); return 2; }

  fs.mkdirSync(opts.shots, { recursive: true });
  const server = opts.url ? { url: opts.url, close() {} } : await serve(path.resolve(opts.public));
  const engine = opts.firefox ? 'firefox' : 'chrome';
  const browser = await launchBrowser(engine, opts.firefox || opts.chrome);

  const results = [];
  for (const s of selected) {
    if (s.opts.engines && !s.opts.engines.includes(engine)) {
      results.push({ ...s, skipped: `${engine} cannot show it`, seconds: 0 });
      console.log(`skip  ${s.name} (${engine} cannot show it)`);
      continue;
    }
    if (s.opts.needs && !opts[s.opts.needs.arg]) {
      results.push({ ...s, skipped: s.opts.needs.why, seconds: 0 });
      console.log(`skip  ${s.name} (${s.opts.needs.why})`);
      continue;
    }
    const started = Date.now();
    const context = await browser.createBrowserContext();
    const page = await context.newPage();
    const errors = [];
    // What the page said that is the dev server's doing, not the app's: reported with a failure.
    const failedRequests = [];
    // Every page a scenario uses reports here, the first and any fresh one (`t.fresh`).
    const watch = (p) => {
      p.on('pageerror', (e) => {
        const text = String(e.message || e);
        if (DEV_CLIENT_NOISE.test(text)) return;
        if (DEV_SERVER_SWAP.test(text)) failedRequests.push(`dev server mid-swap: ${text.slice(0, 200)}`);
        else errors.push(`pageerror: ${text.slice(0, 300)}`);
      });
      p.on('console', (m) => {
        if (m.type() !== 'error' || DEV_CLIENT_NOISE.test(m.text())) return;
        if (DEV_SERVER_SWAP.test(m.text())) failedRequests.push(`dev server mid-swap: ${m.text().slice(0, 200)}`);
        else errors.push(`console.error: ${m.text().slice(0, 300)}`);
      });
      p.on('requestfailed', (r) => failedRequests.push(`request failed: ${r.url().slice(0, 200)} (${r.failure()?.errorText || 'no reason'})`));
    };
    watch(page);
    const t = helpers(page, server.url, engine, watch);
    t.run = opts;
    let failure = null;
    try {
      const limit = s.opts.timeout || 600000;
      await Promise.race([
        s.fn(t),
        new Promise((_, reject) => setTimeout(() => reject(new Failure(`timed out after ${limit / 1000}s`)), limit)),
      ]);
      const unexpected = errors.filter((e) => !(s.opts.allowErrors && s.opts.allowErrors.test(e)));
      if (unexpected.length) throw new Failure(`page errors:\n${unexpected.join('\n')}`);
    } catch (e) {
      failure = e instanceof Failure ? e.message : `harness error: ${e.stack || e}`;
      // A failure carries what the page itself said, so a page that never booted says why.
      const said = [...(failure.startsWith('page errors:') ? [] : errors), ...failedRequests];
      if (said.length) failure += `\npage said:\n${said.slice(0, 12).join('\n')}`;
      await t.page.screenshot({ path: path.join(opts.shots, `${s.name}.png`) }).catch(() => {});
    }
    await context.close();
    const seconds = (Date.now() - started) / 1000;
    results.push({ ...s, failure, seconds });
    console.log(failure ? `FAIL ${s.name} (${seconds.toFixed(1)}s)\n      ${failure.split('\n').join('\n      ')}` : `  ok  ${s.name} (${seconds.toFixed(1)}s)`);
  }

  await browser.close();
  server.close();

  const failed = results.filter((r) => r.failure).length;
  const skipped = results.filter((r) => r.skipped).length;
  const ran = results.length - skipped;
  if (opts.junit) {
    const cases = results.map((r) => `    <testcase classname="studio-e2e.${engine}" name="${xmlEscape(r.name)}" time="${r.seconds.toFixed(3)}">` +
      (r.failure ? `\n      <failure message="${xmlEscape(r.failure.split('\n')[0])}">${xmlEscape(r.failure)}</failure>\n    ` : '') +
      (r.skipped ? `<skipped message="${xmlEscape(r.skipped)}"/>` : '') + '</testcase>');
    fs.writeFileSync(opts.junit, `<?xml version="1.0" encoding="UTF-8"?>\n<testsuites name="studio-e2e" tests="${results.length}" failures="${failed}" skipped="${skipped}">\n  <testsuite name="studio-e2e" tests="${results.length}" failures="${failed}" skipped="${skipped}">\n${cases.join('\n')}\n  </testsuite>\n</testsuites>\n`);
  }
  console.log(`studio-e2e (${engine}): ${ran - failed}/${ran} passed${skipped ? `, ${skipped} skipped` : ''}${failed ? ` — screenshots of each failure in ${opts.shots}` : ''}`);
  return failed ? 1 : 0;
}

// LLM Heal: the fairy mends a typo through the worker, which the page's own
// fetch answers here, so no request leaves the machine. The card shows both texts; Keep lands
// the mend, Undo does not, and a failing worker is said so.
// The address the build under test was given (LOGOS_HEAL_URL at build time), else production.
const HEAL_ENDPOINT = process.env.LOGOS_HEAL_URL || 'https://api.logicaffeine.com/heal';
async function stubHeal(page, answer) {
  await page.evaluateOnNewDocument((endpoint, answer) => {
    window.__heals = [];
    const real = window.fetch.bind(window);
    window.fetch = (input, init) => {
      const request = input instanceof Request ? input : new Request(input, init);
      if (request.url !== endpoint) return real(input, init);
      return request.clone().text().then((text) => {
        window.__heals.push(JSON.parse(text));
        return new Response(JSON.stringify(answer.json), { status: answer.status, headers: { 'content-type': 'application/json' } });
      });
    };
  }, HEAL_ENDPOINT, answer);
}
const HEAL_BEFORE = '## Main\nShwo "hello".\n';
const HEAL_AFTER = '## Main\nShow "hello".\n';
async function editorText(t) {
  return t.page.$eval('.studio-main > section textarea', (e) => e.value);
}

scenario('heal_mends_a_typo_and_keep_lands_it', async (t) => {
  await stubHeal(t.page, { status: 200, json: { ok: true, healed: HEAL_AFTER, model: 'stub' } });
  await t.open(FILES.code);
  await retype(t, HEAL_BEFORE);
  // What the editor holds after typing (it indents and ends lines its own way) is the Before.
  const typed = await editorText(t);
  assert(typed.includes('Shwo'), `the editor does not hold the typo: ${JSON.stringify(typed)}`);
  await t.page.click('.heal-btn');
  await t.page.waitForSelector('.heal-card', { timeout: 15000 });
  const texts = await t.page.$$eval('.heal-card pre', (ps) => ps.map((p) => p.innerText));
  assert(texts.length === 2 && texts[0].includes('Shwo') && texts[1].includes('Show "hello"'), `the card shows ${JSON.stringify(texts)}`);
  const sent = await t.page.evaluate(() => window.__heals);
  assert(sent.length === 1 && sent[0].mode === 'code' && sent[0].fragment === true && sent[0].text.includes('Shwo'), `the worker was asked ${JSON.stringify(sent)}`);
  assert(!sent[0].text.includes('Fibonacci'), 'the worker was sent the file around the typed lines');
  const dock = await t.page.$eval('.studio-dock-text', (e) => e.innerText);
  assert(/Healed \d+ lines?\. Keep or Undo\./.test(dock), `the dock reads ${JSON.stringify(dock)}`);
  assert(await editorText(t) === typed, 'the editor changed before Keep');
  await t.page.click('.heal-keep');
  await t.page.waitForFunction(() => !document.querySelector('.heal-card'), { timeout: 5000 });
  const kept = await editorText(t);
  assert(kept.includes('Show "hello"') && !kept.includes('Shwo'), `Keep did not land the mend: ${JSON.stringify(kept)}`);
});

scenario('heal_undo_leaves_the_editor_alone', async (t) => {
  await stubHeal(t.page, { status: 200, json: { ok: true, healed: HEAL_AFTER, model: 'stub' } });
  await t.open(FILES.code);
  await retype(t, HEAL_BEFORE);
  const typed = await editorText(t);
  await t.page.click('.heal-btn');
  await t.page.waitForSelector('.heal-card', { timeout: 15000 });
  await t.page.evaluate(() => [...document.querySelectorAll('.heal-card button')].find((b) => b.innerText.trim() === 'Undo').click());
  await t.page.waitForFunction(() => !document.querySelector('.heal-card'), { timeout: 5000 });
  assert(await editorText(t) === typed, 'Undo changed the editor');
  assert(!(await t.page.$('.heal-fairy')), 'the fairy did not go home after Undo');
});

scenario('heal_says_so_when_the_worker_fails', async (t) => {
  await stubHeal(t.page, { status: 502, json: { ok: false, error: 'upstream_failed' } });
  await t.open(FILES.code);
  await retype(t, HEAL_BEFORE);
  const typed = await editorText(t);
  await t.page.click('.heal-btn');
  await t.page.waitForSelector('.heal-card-failed', { timeout: 15000 });
  const said = await t.page.$eval('.heal-card-failed p', (e) => e.innerText);
  assert(/did not answer/.test(said), `the failure reads ${JSON.stringify(said)}`);
  assert(await editorText(t) === typed, 'a failed heal changed the editor');
  await t.page.evaluate(() => [...document.querySelectorAll('.heal-card button')].find((b) => b.innerText.trim() === 'Close').click());
  await t.page.waitForFunction(() => !document.querySelector('.heal-card'), { timeout: 5000 });
});

main().then((code) => process.exit(code), (e) => { console.error(e); process.exit(2); });
