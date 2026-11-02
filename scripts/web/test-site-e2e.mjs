// The site's pages in a real browser — the pages with no browser suite of their own.
//
// A page's data and rendered structure are pinned by Rust unit and SSR tests (the roadmap's in
// apps/logicaffeine_web/src/ui/pages/roadmap*.rs and crates/logicaffeine_tests/tests/roadmap_page.rs,
// the contact page's in crates/logicaffeine_tests/tests/contact_page.rs). What only a browser can
// show lives here: where things are painted, at every screen size, and how the contact form behaves.
// The contact form never reaches the network: its endpoint is answered inside the page and
// Turnstile's script by request interception (see stubPricing).
//
// Every scenario runs in a fresh browser context, fails on any uncaught page error or console
// error, and leaves a screenshot of the failing moment. Scenarios find the page's parts by what
// the page renders and measure what it paints, never by internal hooks, so the same scenario
// proves a bug on the old page (RED) and its absence on the fixed one (GREEN).
//
// Usage: node scripts/web/test-site-e2e.mjs (--public <built-public-dir> | --url <running-site>)
//          (--chrome <path> | --firefox <path>) [--junit <report.xml>] [--only name,name]
//          [--shots <dir>] [--list]
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

// ---------------------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------------------

function parseArgs(argv) {
  const out = { only: null, junit: null, shots: '/tmp/site-e2e-shots', list: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--public') out.public = argv[++i];
    else if (a === '--url') out.url = argv[++i].replace(/\/$/, '');
    else if (a === '--chrome') out.chrome = argv[++i];
    else if (a === '--firefox') out.firefox = argv[++i];
    else if (a === '--junit') out.junit = argv[++i];
    else if (a === '--only') out.only = argv[++i].split(',').map((s) => s.trim()).filter(Boolean);
    else if (a === '--shots') out.shots = argv[++i];
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
// Assertions and page helpers
// ---------------------------------------------------------------------------------------

class Failure extends Error {}

// The dev server's own client, injected into dev builds only: its socket, and the rebuild notices
// it cannot parse when the server and client versions disagree.
const DEV_CLIENT_NOISE = /WebSocket connection to '[^']*\/_dioxus\?|Error parsing devserver message/;
// The dev server swaps each build into place; a page that loads mid-swap is sent the app's loader
// script with no content type, the browser refuses it, and the app never starts. It is the
// server's race, never the app's error: `t.load` waits the swap out and loads again.
const DEV_SERVER_SWAP = /MIME type of ""|disallowed MIME type \(“”\)/;
// A fetch the page was still making when the harness navigated it somewhere else. Chrome drops
// it silently; Firefox raises it on the page being left. It is the harness's navigation, never
// the app's error — but only while the harness is navigating away from a loaded page: an abort
// at any other moment is the app's, and fails the scenario.
const NAVIGATION_ABORT = /AbortError: The operation was aborted|TypeError: NetworkError when attempting to fetch resource/;

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

function assert(cond, message) { if (!cond) throw new Failure(message); }
const px = (n) => `${n.toFixed(1)}px`;

// The screen sizes a layout scenario is measured at: a wide desktop, a laptop, a tablet held
// upright and a phone.
const SCREENS = [
  { width: 1920, height: 1080 },
  { width: 1280, height: 900 },
  { width: 768, height: 1024 },
  { width: 390, height: 844, isMobile: true, hasTouch: true },
];
const screenName = (s) => `${s.width}×${s.height}`;

function helpers(page, base, navigation) {
  const t = {
    page,
    base,

    // Load a page until the app has taken it over: the live app in #app holds `ready`, and the
    // shell's watchdog has removed #main — the prerendered copy of a deploy build, which repeats
    // every element the app renders and would pass for it (an empty div in a dev build). A load
    // the dev server's build swap refused is waited out and loaded again; any other failure to
    // start fails at once. Layout waits for the page's fonts, which move every line box.
    async load(route, ready) {
      const url = `${base}${route}`;
      navigation.leaving = page.url() !== 'about:blank';
      try {
        await t.navigate(url, ready);
      } finally {
        navigation.leaving = false;
      }
      await page.evaluate(() => document.fonts.ready);
      await t.frames(4);
    },

    async navigate(url, ready) {
      for (let attempt = 1; ; attempt++) {
        let refused;
        const swapped = new Promise((resolve) => { refused = (text) => { if (DEV_SERVER_SWAP.test(text)) resolve(text); }; });
        const onConsole = (m) => refused(m.text());
        const onPageError = (e) => refused(String(e.message || e));
        page.on('console', onConsole);
        page.on('pageerror', onPageError);
        try {
          await page.goto(url, { waitUntil: 'load', timeout: 300000 });
          navigation.leaving = false;
          const outcome = await Promise.race([
            page
              .waitForFunction((sel) => document.querySelector(`#app ${sel}`) && !document.getElementById('main'), { timeout: 300000 }, ready)
              .then(() => null),
            swapped,
          ]);
          if (outcome === null) break;
          if (attempt === 3) throw new Failure(`the dev server refused the app's loader script ${attempt} times: ${outcome.slice(0, 160)}`);
        } finally {
          page.off('console', onConsole);
          page.off('pageerror', onPageError);
        }
        await wholeBuild(new URL(url).origin);
      }
    },

    async openRoadmap(screen = SCREENS[1]) {
      await page.setViewport({ deviceScaleFactor: 1, ...screen });
      await t.load('/roadmap', '.timeline .milestone');
    },

    // Two animation frames: long enough for any per-frame repaint to land.
    async frames(n = 2) {
      for (let i = 0; i < n; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
    },

    // The roadmap's timeline as painted, in page coordinates. For each card: its status (read off
    // its badge), its dot's centre and border style, and the centre of its title's first line.
    // `rail` is every piece of the line the timeline draws, wherever it is drawn: the ::before of
    // the timeline or of a card, when it paints a box. Each piece carries its left and right edges,
    // its top and bottom, and its background image, whose colours the line is painted in.
    async timeline() {
      return page.evaluate(() => {
        const sx = window.scrollX, sy = window.scrollY;
        const box = (r) => ({ left: r.left + sx, right: r.right + sx, top: r.top + sy, bottom: r.bottom + sy });
        const cards = [...document.querySelectorAll('#app .timeline .milestone')].map((card) => {
          const badge = card.querySelector('.milestone-badge');
          const status = ['done', 'progress', 'planned'].find((s) => badge && badge.classList.contains(s)) || null;
          const dotEl = card.querySelector('.milestone-dot');
          const dot = dotEl ? box(dotEl.getBoundingClientRect()) : null;
          const titleEl = card.querySelector('.milestone-title');
          let title = null;
          if (titleEl && titleEl.firstChild) {
            const range = document.createRange();
            range.selectNodeContents(titleEl);
            const first = range.getClientRects()[0];
            if (first) title = { text: titleEl.textContent, centerY: (first.top + first.bottom) / 2 + sy };
          }
          return {
            status,
            title,
            dot: dot && { x: (dot.left + dot.right) / 2, y: (dot.top + dot.bottom) / 2, border: getComputedStyle(dotEl).borderTopStyle },
          };
        });
        const rail = [];
        for (const host of [document.querySelector('#app .timeline'), ...document.querySelectorAll('#app .timeline .milestone')]) {
          if (!host) continue;
          const s = getComputedStyle(host, '::before');
          if (s.content === 'none' || s.display === 'none' || s.position !== 'absolute') continue;
          const width = parseFloat(s.width), height = parseFloat(s.height);
          if (!(width > 0) || !(height > 0)) continue;
          const r = host.getBoundingClientRect();
          const left = r.left + host.clientLeft + parseFloat(s.left) + sx;
          const top = r.top + host.clientTop + parseFloat(s.top) + sy;
          rail.push({ left, right: left + width, top, bottom: top + height, image: s.backgroundImage, color: s.backgroundColor });
        }
        rail.sort((a, b) => a.top - b.top);
        return { cards, rail };
      });
    },
  };
  return t;
}

// The colour the line takes at each status: the status badge's own colour, and a muted slate for
// work not yet started.
const RAIL_COLOR = {
  done: 'rgb(34, 197, 94)',
  progress: 'rgb(167, 139, 250)',
  planned: 'rgba(148, 163, 184, 0.45)',
};

// The colours a painted line piece runs through, top to bottom: the stops of its gradient, or its
// one background colour.
function railColors(piece) {
  const stops = piece.image.match(/rgba?\([^)]*\)/g);
  if (stops && stops.length) return stops;
  return [piece.color];
}

// ---------------------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------------------

const scenarios = [];
function scenario(name, fn, opts = {}) { scenarios.push({ name, fn, opts }); }

scenario('every_roadmap_card_has_a_dot_and_a_title', async (t) => {
  await t.openRoadmap();
  const { cards } = await t.timeline();
  assert(cards.length > 0, 'the roadmap shows no timeline cards');
  cards.forEach((card, i) => {
    assert(card.dot, `card ${i + 1} has no status dot`);
    assert(card.title && card.title.text.trim(), `card ${i + 1} has no title`);
    assert(card.status, `card ${i + 1} ("${card.title?.text}") has no status badge`);
  });
});

scenario('every_roadmap_dot_sits_on_the_line', async (t) => {
  for (const screen of SCREENS) {
    await t.openRoadmap(screen);
    const { cards, rail } = await t.timeline();
    assert(rail.length > 0, `at ${screenName(screen)} the timeline draws no line`);
    for (const piece of rail) {
      for (const card of cards) {
        const center = (piece.left + piece.right) / 2;
        const off = card.dot.x - center;
        assert(Math.abs(off) <= 0.5, `at ${screenName(screen)} the dot of "${card.title.text}" is centred ${px(off)} from the line (x ${px(card.dot.x)} against ${px(center)})`);
      }
    }
  }
});

scenario('every_roadmap_dot_centres_on_its_title', async (t) => {
  for (const screen of SCREENS) {
    await t.openRoadmap(screen);
    const { cards } = await t.timeline();
    for (const card of cards) {
      const off = card.dot.y - card.title.centerY;
      assert(Math.abs(off) <= 1, `at ${screenName(screen)} the dot of "${card.title.text}" sits ${px(off)} from the middle of its title's first line`);
    }
  }
});

scenario('the_roadmap_line_runs_from_the_first_dot_to_the_last', async (t) => {
  for (const screen of SCREENS) {
    await t.openRoadmap(screen);
    const { cards, rail } = await t.timeline();
    assert(rail.length > 0, `at ${screenName(screen)} the timeline draws no line`);
    const first = cards[0].dot.y, last = cards[cards.length - 1].dot.y;
    const top = rail[0].top, bottom = Math.max(...rail.map((p) => p.bottom));
    assert(Math.abs(top - first) <= 1, `at ${screenName(screen)} the line starts ${px(top - first)} from the first dot's centre`);
    assert(Math.abs(bottom - last) <= 1, `at ${screenName(screen)} the line ends ${px(bottom - last)} from the last dot's centre`);
    let reach = rail[0].bottom;
    for (const piece of rail.slice(1)) {
      assert(piece.top <= reach + 1, `at ${screenName(screen)} the line breaks for ${px(piece.top - reach)} at y ${px(reach)}`);
      reach = Math.max(reach, piece.bottom);
    }
  }
});

scenario('the_roadmap_line_takes_the_colour_of_each_status', async (t) => {
  await t.openRoadmap();
  const { cards, rail } = await t.timeline();
  for (let i = 0; i + 1 < cards.length; i++) {
    const from = cards[i], to = cards[i + 1];
    const mid = (from.dot.y + to.dot.y) / 2;
    const piece = rail.find((p) => p.top <= mid && p.bottom >= mid);
    assert(piece, `no line is drawn between "${from.title.text}" and "${to.title.text}"`);
    const colors = railColors(piece);
    const want = [RAIL_COLOR[from.status], RAIL_COLOR[to.status]];
    const got = [colors[0], colors[colors.length - 1]];
    assert(
      got[0] === want[0] && got[1] === want[1],
      `the line from "${from.title.text}" (${from.status}) to "${to.title.text}" (${to.status}) runs ${got[0]} → ${got[1]}, not ${want[0]} → ${want[1]}`,
    );
  }
});

scenario('a_planned_dot_has_a_dashed_rim', async (t) => {
  await t.openRoadmap();
  const { cards } = await t.timeline();
  const planned = cards.filter((c) => c.status === 'planned');
  assert(planned.length > 0, 'the roadmap has no planned card to look at');
  for (const card of planned) {
    assert(card.dot.border === 'dashed', `the dot of "${card.title.text}" has a ${card.dot.border} rim, not a dashed one`);
  }
  for (const card of cards.filter((c) => c.status !== 'planned')) {
    assert(card.dot.border !== 'dashed', `the dot of "${card.title.text}" (${card.status}) has a dashed rim, which marks planned work`);
  }
});

// ---------------------------------------------------------------------------------------
// The contact page: its anchors, its form, and its fit on a phone
// ---------------------------------------------------------------------------------------

const LEAD_ENDPOINT = 'https://api.logicaffeine.com/lead';
const TURNSTILE_SCRIPT = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit';
const TEAM_EMAIL = 'team@datacenter.dev';
const INSTALL_UNIX = 'curl -fsSL https://logicaffeine.com/install.sh | sh';

// Turnstile's stand-in: it passes at once, remembers how the page asked for it, and, like the real
// widget, runs the challenge again on a reset and hands the page a fresh token.
const TURNSTILE_STUB = `window.turnstile = {
  render(el, options) {
    window.__turnstileOptions = Object.fromEntries(Object.entries(options).filter(([, v]) => typeof v !== 'function'));
    window.__turnstileIssue = () => setTimeout(() => options.callback('stub-token'), 0);
    window.__turnstileIssue();
    return 'stub-widget';
  },
  reset() {
    window.__turnstileResets = (window.__turnstileResets || 0) + 1;
    if (window.__turnstileIssue) window.__turnstileIssue();
  },
  remove() {},
  getResponse() { return 'stub-token'; },
};`;

// Keep every request of the contact form on this machine. The form's endpoint is answered by the
// page's own fetch: a JSON POST across origins needs a CORS preflight, which Chrome does not show
// request interception, and Firefox does not hand interception a request's body. Turnstile's
// script is answered by request interception with the stand-in above. Anything else headed for
// either host is aborted and recorded in `escaped`, which a scenario must find empty.
// `answer` is `{ status, json }` for every POST until the scenario changes `window.__leadAnswer`.
async function stubPricing(page, answer = { status: 202, json: { ok: true } }) {
  await page.evaluateOnNewDocument((endpoint, first) => {
    window.__leads = [];
    window.__leadAnswer = first;
    const real = window.fetch.bind(window);
    window.fetch = (input, init) => {
      const request = input instanceof Request ? input : new Request(input, init);
      if (request.url !== endpoint) return real(input, init);
      return request.clone().text().then((text) => {
        window.__leads.push({ method: request.method, type: request.headers.get('content-type'), body: JSON.parse(text) });
        const { status, json } = window.__leadAnswer;
        return new Response(typeof json === 'string' ? json : JSON.stringify(json), { status, headers: { 'content-type': 'application/json' } });
      });
    };
  }, LEAD_ENDPOINT, answer);
  const log = { turnstile: 0, escaped: [] };
  await page.setRequestInterception(true);
  page.on('request', (request) => {
    const url = request.url();
    if (url === TURNSTILE_SCRIPT) {
      log.turnstile++;
      request.respond({ status: 200, contentType: 'text/javascript', body: TURNSTILE_STUB });
    } else if (/^https:\/\/(api\.logicaffeine\.com|challenges\.cloudflare\.com)\//.test(url)) {
      log.escaped.push(`${request.method()} ${url}`);
      request.abort();
    } else {
      request.continue();
    }
  });
  return log;
}

// Open a route of the contact page, ready once the page's headline is up (any page has one, so a
// page that is missing its parts fails on what it lacks, not on a load that never finishes).
async function openPricing(t, route = '/contact', screen = SCREENS[1]) {
  await t.page.setViewport({ deviceScaleFactor: 1, ...screen });
  await t.load(route, 'h1');
}

// The element's box against the viewport.
const rectOf = (page, selector) => page.evaluate((sel) => {
  const el = document.querySelector(sel);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, width: r.width, height: r.height, margin: parseFloat(getComputedStyle(el).scrollMarginTop) || 0 };
}, selector);

// Wait until the element's top sits where scrolling to it puts it: its scroll margin from the top.
async function waitAtTop(t, selector, what) {
  const until = Date.now() + 8000;
  let rect = null;
  while (Date.now() < until) {
    rect = await rectOf(t.page, selector);
    if (rect && Math.abs(rect.top - rect.margin) <= 2) return rect;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Failure(`${what} did not come to the top of the page: ${JSON.stringify(rect)}`);
}

// A JSON object as text with its keys in order, so two objects compare by content.
const canonical = (object) => JSON.stringify(object, Object.keys(object).sort());

const activeId = (page) => page.evaluate(() => document.activeElement && (document.activeElement.id || document.activeElement.tagName.toLowerCase()));

async function statusText(page, needle) {
  await page.waitForFunction((n) => (document.getElementById('lead-status')?.textContent || '').includes(n), { timeout: 30000 }, needle)
    .catch(async () => { throw new Failure(`the form's status never said ${JSON.stringify(needle)}; it says ${JSON.stringify(await page.evaluate(() => document.getElementById('lead-status')?.textContent))}`); });
}

// Replace a field's text the way a person does: select all of it, delete, type.
// A Turnstile token is good for one check: after every request the form sends, the widget is reset.
async function resetsAfter(page, sends) {
  await page.waitForFunction((n) => (window.__turnstileResets || 0) >= n, { timeout: 5000 }, sends).catch(() => {});
  const [resets, leads] = await page.evaluate(() => [window.__turnstileResets || 0, window.__leads.length]);
  assert(leads === sends, `the form sent ${leads} requests, not ${sends}`);
  assert(resets === sends, `the form reset Turnstile ${resets} times after ${sends} request(s)`);
}

async function fill(page, selector, text) {
  await page.focus(selector);
  await page.$eval(selector, (el) => el.select());
  await page.keyboard.press('Backspace');
  if (text) await page.keyboard.type(text);
}

scenario('pricing_redirects_to_the_contact_page', async (t) => {
  await openPricing(t, '/contact');
  await t.page.waitForFunction(() => location.pathname === '/contact', { timeout: 10000 })
    .catch(async () => { throw new Failure(`/contact left the address bar at ${await t.page.evaluate(() => location.href)}`); });
  await t.page.evaluate(() => document.getElementById('contact').scrollIntoView());
  await waitAtTop(t, '#contact', 'the contact form');
  const heading = await t.page.$eval('#contact h2', (h) => h.textContent);
  assert(heading === 'Contact the team', `the section at the top reads ${JSON.stringify(heading)}`);
});

scenario('the_nav_leads_to_contact', async (t) => {
  await t.openRoadmap();
  // A link's label as painted: `innerText`, since each nav icon carries a <style> of its own.
  const links = await t.page.$$eval('a.main-nav-link', (as) => as.map((a) => a.innerText.trim()));
  assert(links.includes('Contact') && !links.includes('Pricing'), `the nav reads ${JSON.stringify(links)}`);
  await Promise.all([
    t.page.waitForFunction(() => document.querySelector('#app .contact-hero h1'), { timeout: 30000 }),
    t.page.evaluate(() => [...document.querySelectorAll('a.main-nav-link')].find((a) => a.innerText.trim() === 'Contact').click()),
  ]);
  assert(await t.page.evaluate(() => location.pathname) === '/contact', 'the nav link did not open /contact');
  await t.openRoadmap(SCREENS[3]);
  await t.page.click('button.mobile-menu-btn');
  await t.frames(4);
  await Promise.all([
    t.page.waitForFunction(() => location.pathname === '/contact' && document.querySelector('#app .contact-hero h1'), { timeout: 30000 }),
    t.page.evaluate(() => [...document.querySelectorAll('a.mobile-nav-link')].find((a) => a.innerText.trim() === 'Contact').click()),
  ]);
});

scenario('a_keyboard_alone_sends_a_contact_request', async (t) => {
  const log = await stubPricing(t.page);
  await openPricing(t, '/contact#contact');
  let presses = 0;
  while ((await activeId(t.page)) !== 'lead-email') {
    assert(++presses <= 400, 'Tab never reached the work email field');
    await t.page.keyboard.press('Tab');
  }
  await t.page.keyboard.type('ada@example.com');
  await t.page.keyboard.press('Tab');
  assert(await activeId(t.page) === 'lead-phone', `Tab from the email went to ${await activeId(t.page)}, not the phone`);
  await t.page.keyboard.press('Tab');
  const submit = await t.page.evaluate(() => document.activeElement.type === 'submit' && document.activeElement.textContent.trim());
  assert(submit === 'Send', `Tab from the phone went to ${await activeId(t.page)}, not the submit button (the honeypot is never in the tab order)`);
  await t.page.keyboard.press('Enter');
  await statusText(t.page, 'Thanks. The team will reach out to ada@example.com.');
  await t.frames(4);
  assert(await activeId(t.page) === 'lead-status', `focus is on ${await activeId(t.page)}, not the status that says the request was sent`);
  const leads = await t.page.evaluate(() => window.__leads);
  assert(leads.length === 1, `the form sent ${leads.length} requests`);
  const want = { email: 'ada@example.com', phone: null, turnstile_token: 'stub-token', source: 'pricing', website: '' };
  assert(canonical(leads[0].body) === canonical(want), `the form sent ${canonical(leads[0].body)}, not ${canonical(want)}`);
  assert(leads[0].method === 'POST' && /^application\/json/.test(leads[0].type || ''), `the form sent a ${leads[0].method} of ${leads[0].type}`);
  const options = await t.page.evaluate(() => window.__turnstileOptions);
  assert(options && options.action === 'pricing-lead' && options.appearance === 'interaction-only' && options.sitekey, `Turnstile was asked for ${JSON.stringify(options)}`);
  await resetsAfter(t.page, 1);
  assert(log.turnstile === 1, `Turnstile's script was fetched ${log.turnstile} times`);
  assert(await t.page.evaluate(() => document.querySelector('[data-theme]') === null), 'the page carries a data-theme attribute');
  assert(log.escaped.length === 0, `requests left the machine: ${log.escaped.join(', ')}`);
});

scenario('an_optional_phone_is_sent_as_typed', async (t) => {
  const log = await stubPricing(t.page);
  await openPricing(t, '/contact#contact');
  await fill(t.page, '#lead-email', '  Ada@Example.COM ');
  await fill(t.page, '#lead-phone', '+1 (555) 123-4567');
  await t.page.click('#contact button[type=submit]');
  await statusText(t.page, 'Thanks. The team will reach out to Ada@example.com.');
  const body = (await t.page.evaluate(() => window.__leads))[0].body;
  assert(body.email === 'Ada@example.com' && body.phone === '+1 (555) 123-4567', `the form sent ${JSON.stringify(body)}`);
  await resetsAfter(t.page, 1);
  assert(log.escaped.length === 0, `requests left the machine: ${log.escaped.join(', ')}`);
});

scenario('a_failed_send_offers_the_team_address', async (t) => {
  const log = await stubPricing(t.page, { status: 500, json: { ok: false, error: 'internal' } });
  await openPricing(t, '/contact#contact');
  await fill(t.page, '#lead-email', 'ada@example.com');
  await t.page.click('#contact button[type=submit]');
  await statusText(t.page, "We couldn't send that.");
  const mail = await t.page.$$eval('#lead-status a[href^="mailto:"]', (as) => as.map((a) => a.getAttribute('href')));
  assert(JSON.stringify(mail) === JSON.stringify([`mailto:${TEAM_EMAIL}`]), `the failure offers ${JSON.stringify(mail)}`);
  await t.frames(4);
  assert(await activeId(t.page) === 'lead-status', `focus is on ${await activeId(t.page)}, not the status`);
  assert(await t.page.$('#lead-email'), 'a failed send took the form away, so it cannot be sent again');
  await resetsAfter(t.page, 1);
  assert(log.escaped.length === 0, `requests left the machine: ${log.escaped.join(', ')}`);
});

scenario('a_rate_limited_send_offers_the_team_address', async (t) => {
  const log = await stubPricing(t.page, { status: 429, json: { ok: false, error: 'rate_limited' } });
  await openPricing(t, '/contact#contact');
  await fill(t.page, '#lead-email', 'ada@example.com');
  await t.page.click('#contact button[type=submit]');
  await statusText(t.page, "We couldn't send that.");
  const mail = await t.page.$$eval('#lead-status a[href^="mailto:"]', (as) => as.map((a) => a.getAttribute('href')));
  assert(JSON.stringify(mail) === JSON.stringify([`mailto:${TEAM_EMAIL}`]), `a refused send offers ${JSON.stringify(mail)}`);
  await resetsAfter(t.page, 1);
  await t.page.click('#contact button[type=submit]');
  await t.page.waitForFunction(() => window.__leads.length === 2, { timeout: 30000 });
  await resetsAfter(t.page, 2);
  assert(log.escaped.length === 0, `requests left the machine: ${log.escaped.join(', ')}`);
});

scenario('invalid_input_says_what_is_wrong_under_each_field', async (t) => {
  await stubPricing(t.page);
  await openPricing(t, '/contact#contact');
  await fill(t.page, '#lead-email', 'not-an-email');
  await fill(t.page, '#lead-phone', '12');
  await t.page.click('#contact button[type=submit]');
  await t.page.waitForSelector('#lead-email-error', { timeout: 10000 });
  const fields = await t.page.evaluate(() => ['lead-email', 'lead-phone'].map((id) => {
    const input = document.getElementById(id);
    const described = (input.getAttribute('aria-describedby') || '').split(/\s+/).filter(Boolean);
    return { id, invalid: input.getAttribute('aria-invalid'), described: described.map((d) => document.getElementById(d)?.textContent || `missing #${d}`) };
  }));
  const [email, phone] = fields;
  assert(email.invalid === 'true' && email.described.includes('Enter a whole email address, with an @ and a domain.'), `the email field says ${JSON.stringify(email)}`);
  assert(phone.invalid === 'true' && phone.described.includes('Enter the whole number: it needs at least 7 digits.'), `the phone field says ${JSON.stringify(phone)}`);
  await t.frames(4);
  assert(await activeId(t.page) === 'lead-email', `focus is on ${await activeId(t.page)}, not the first field to fix`);
  assert((await t.page.evaluate(() => window.__leads.length)) === 0, 'the form sent a request its own rules refuse');
  await fill(t.page, '#lead-email', 'ada@example.com');
  await fill(t.page, '#lead-phone', '');
  await t.page.click('#contact button[type=submit]');
  await statusText(t.page, 'Thanks.');
  await resetsAfter(t.page, 1);
});

scenario('the_worker_s_refusal_shows_under_its_field', async (t) => {
  await stubPricing(t.page, { status: 400, json: { ok: false, error: 'invalid_phone', reason: 'too_many_digits' } });
  await openPricing(t, '/contact#contact');
  await fill(t.page, '#lead-email', 'ada@example.com');
  await fill(t.page, '#lead-phone', '555 123 4567');
  await t.page.click('#contact button[type=submit]');
  await t.page.waitForSelector('#lead-phone-error', { timeout: 30000 });
  const said = await t.page.$eval('#lead-phone-error', (p) => p.textContent);
  assert(said === 'A phone number has at most 15 digits.', `the phone field says ${JSON.stringify(said)}`);
  await statusText(t.page, "We couldn't send that.");
  await t.frames(4);
  assert(await activeId(t.page) === 'lead-phone', `focus is on ${await activeId(t.page)}, not the field the worker refused`);
  await resetsAfter(t.page, 1);
});

scenario('turnstile_loads_only_when_the_form_is_used', async (t) => {
  const log = await stubPricing(t.page);
  await openPricing(t, '/contact');
  await new Promise((r) => setTimeout(r, 1500));
  assert(log.turnstile === 0, 'Turnstile was fetched before anyone touched the form');
  assert(await t.page.evaluate(() => !document.querySelector('script[src*="challenges.cloudflare.com"]')), 'the page carries Turnstile\'s script at load');
  await t.page.focus('#lead-email');
  await t.page.waitForFunction(() => window.__turnstileOptions, { timeout: 10000 });
  await t.page.focus('#lead-phone');
  await t.page.focus('#lead-email');
  await t.frames(4);
  assert(log.turnstile === 1, `Turnstile's script was fetched ${log.turnstile} times`);
});

scenario('the_contact_page_fits_a_phone', async (t) => {
  for (const screen of [{ width: 360, height: 740, isMobile: true, hasTouch: true }, SCREENS[3]]) {
    await openPricing(t, '/contact', screen);
    const fit = await t.page.evaluate(() => {
      const width = innerWidth;
      const page = document.scrollingElement;
      // The page's own content: everything under its root but the site's nav, its drawer (parked
      // off-screen until opened) and the footer, and what is hidden from everyone.
      const chrome = '.main-nav, .mobile-drawer, .mobile-drawer-overlay, .site-footer, [aria-hidden="true"]';
      const skip = (el) => el.closest(chrome) || el.getClientRects().length === 0;
      const content = [...document.querySelectorAll('#app .contact *')].filter((el) => !skip(el));
      const out = [];
      for (const el of content) {
        const r = el.getBoundingClientRect();
        if (r.width === 0 && r.height === 0) continue;
        if (r.right > width + 0.5 || r.left < -0.5) out.push(`${el.tagName.toLowerCase()}.${el.className} spans ${r.left.toFixed(1)}–${r.right.toFixed(1)}`);
      }
      const small = [];
      for (const el of content.filter((el) => el.matches('a, button, input'))) {
        if (getComputedStyle(el).display === 'inline') continue;
        const r = el.getBoundingClientRect();
        if (r.height < 44 - 0.5 || r.width < 44 - 0.5) small.push(`${el.tagName.toLowerCase()} "${(el.textContent || el.name || '').trim().slice(0, 30)}" is ${r.width.toFixed(0)}×${r.height.toFixed(0)}`);
      }
      const fonts = content.filter((el) => el.matches('input')).map((el) => [el.name, parseFloat(getComputedStyle(el).fontSize)]).filter(([, px]) => px < 16);
      const inputs = content.filter((el) => el.matches('input')).length;
      // The vibe coders' excerpt keeps inside its own box: it wraps, or it scrolls itself.
      const pre = document.querySelector('#vibe-coders pre');
      const code = pre && (() => {
        const r = pre.getBoundingClientRect();
        return { left: r.left, right: r.right, overflows: pre.scrollWidth > pre.clientWidth + 0.5, scrolls: ['auto', 'scroll'].includes(getComputedStyle(pre).overflowX) };
      })();
      return { measured: content.length, inputs, sideways: page.scrollWidth - page.clientWidth, body: document.body.scrollWidth - document.body.clientWidth, out, small, fonts, code };
    });
    const name = screenName(screen);
    assert(fit.measured >= 100 && fit.inputs >= 2, `at ${name} there is too little pricing page to measure (${fit.measured} elements, ${fit.inputs} inputs)`);
    assert(fit.sideways <= 0 && fit.body <= 0, `at ${name} the page scrolls sideways (${fit.sideways}px, body ${fit.body}px)`);
    assert(fit.out.length === 0, `at ${name} parts of the page run past the screen:\n${fit.out.slice(0, 12).join('\n')}`);
    assert(fit.small.length === 0, `at ${name} tap targets are smaller than 44px:\n${fit.small.slice(0, 12).join('\n')}`);
    assert(fit.fonts.length === 0, `at ${name} inputs are set below 16px (the phone zooms in on them): ${JSON.stringify(fit.fonts)}`);
    assert(fit.code, `at ${name} there is no vibe coders' code block to measure`);
    assert(fit.code.left >= -0.5 && fit.code.right <= screen.width + 0.5, `at ${name} the vibe coders' code block spans ${fit.code.left.toFixed(1)}–${fit.code.right.toFixed(1)}`);
    assert(!fit.code.overflows || fit.code.scrolls, `at ${name} the vibe coders' code runs out of its box without scrolling it`);
  }
});

scenario('the_use_cases_tile_two_by_two_and_stack_on_a_phone', async (t) => {
  const tiles = () => t.page.evaluate(() => [...document.querySelectorAll('#use-cases article')].map((el) => {
    const r = el.getBoundingClientRect();
    return { left: Math.round(r.left), top: Math.round(r.top + scrollY) };
  }));
  await openPricing(t, '/contact', SCREENS[1]);
  const wide = await tiles();
  assert(wide.length === 4, `the use cases are ${wide.length} tiles`);
  const [a, b, c, d] = wide;
  assert(a.top === b.top && c.top === d.top && c.top > a.top && a.left === c.left && b.left === d.left && b.left > a.left,
    `at ${screenName(SCREENS[1])} the use cases are not two rows of two: ${JSON.stringify(wide)}`);
  await openPricing(t, '/contact', SCREENS[3]);
  const narrow = await tiles();
  assert(narrow.length === 4 && narrow.every((x, i) => x.left === narrow[0].left && (i === 0 || x.top > narrow[i - 1].top)),
    `at ${screenName(SCREENS[3])} the use cases are not one column: ${JSON.stringify(narrow)}`);
});

scenario('reduced_motion_leaves_the_pricing_page_still', async (t) => {
  await t.page.emulateMediaFeatures([{ name: 'prefers-reduced-motion', value: 'reduce' }]);
  await openPricing(t, '/contact');
  const moving = await t.page.evaluate(() => {
    const found = [];
    for (const el of document.querySelectorAll('.pricing, .pricing *')) {
      const s = getComputedStyle(el);
      if (s.animationName !== 'none') found.push(`${el.tagName.toLowerCase()}.${el.className} animates ${s.animationName}`);
      if (s.transitionDuration.split(',').some((d) => parseFloat(d) > 0)) found.push(`${el.tagName.toLowerCase()}.${el.className} transitions over ${s.transitionDuration}`);
    }
    const running = document.getAnimations().filter((a) => a.effect && a.effect.target && a.effect.target.closest && a.effect.target.closest('.pricing'));
    return { found, running: running.length };
  });
  assert(moving.found.length === 0 && moving.running === 0, `with reduced motion the page still moves (${moving.running} running):\n${moving.found.slice(0, 10).join('\n')}`);
  await t.page.evaluate(() => [...document.querySelectorAll('#top a')].find((a) => a.textContent.trim() === 'Contact for pricing').click());
  await t.frames(3);
  await new Promise((r) => setTimeout(r, 150));
  const rect = await rectOf(t.page, '#contact');
  assert(rect && Math.abs(rect.top - rect.margin) <= 2, `with reduced motion the jump to the form is not instant: its top is at ${rect && rect.top.toFixed(1)}px`);
  // Media-feature emulation is a DevTools-protocol command; puppeteer's WebDriver BiDi driver for
  // Firefox has none. The rule under test is a plain `prefers-reduced-motion` media query.
}, { engines: ['chrome'] });

scenario('a_section_link_opens_at_its_section', async (t) => {
  await openPricing(t, '/contact');
  await t.page.evaluate(() => [...document.querySelectorAll('#plans a')].find((a) => a.textContent.trim() === 'Install largo').click());
  await waitAtTop(t, '#install', 'the install commands');
  assert(await t.page.evaluate(() => location.hash) === '#install', 'the address bar does not name the install section');
  await t.page.evaluate(() => [...document.querySelectorAll('#plans a')].find((a) => a.textContent.trim() === 'Request pricing').click());
  await t.page.evaluate(() => document.getElementById('contact').scrollIntoView());
  await waitAtTop(t, '#contact', 'the contact form');
  await t.page.evaluate(() => [document.scrollingElement, document.body].forEach((el) => el.scrollTo(0, 0)));
  await t.page.evaluate(() => [...document.querySelectorAll('#licensing a')].find((a) => a.textContent.trim() === 'Contact us for pricing').click());
  await waitAtTop(t, '#contact', 'the pricing form, a second time');
});

scenario('the_privacy_note_opens_at_its_paragraph', async (t) => {
  await openPricing(t, '/contact#contact');
  await Promise.all([
    t.page.waitForFunction(() => location.pathname === '/privacy', { timeout: 30000 }),
    t.page.evaluate(() => [...document.querySelectorAll('#contact a')].find((a) => a.textContent.trim() === 'Privacy policy').click()),
  ]);
  assert(await t.page.evaluate(() => location.hash) === '#pricing-requests', 'the privacy link lost its paragraph');
  await t.page.waitForSelector('#pricing-requests', { timeout: 30000 });
  await waitAtTop(t, '#pricing-requests', 'the pricing-requests paragraph');
});

scenario('the_install_command_copies', async (t) => {
  await t.page.browserContext().overridePermissions(t.base, ['clipboard-read', 'clipboard-write', 'clipboard-sanitized-write']);
  await openPricing(t, '/contact');
  await t.page.bringToFront();
  await t.page.click('button[aria-label="Copy the macOS and Linux install command"]');
  await t.page.waitForFunction(() => document.querySelector('button[aria-label="Copy the macOS and Linux install command"]').textContent === 'Copied', { timeout: 5000 });
  const copied = await t.page.evaluate(() => navigator.clipboard.readText());
  assert(copied === INSTALL_UNIX, `the clipboard holds ${JSON.stringify(copied)}`);
}, { engines: ['chrome'] });

// ---------------------------------------------------------------------------------------
// The front page: the heading, the line on how, the install box and a copy button per system
// (the prompt itself hidden until a browser refuses the copy), then the Studio in a frame
// ---------------------------------------------------------------------------------------

// The macOS button, which copies the shell prompt.
const COPY_BUTTON = '#app button.landing-copy[data-system="macos"]';
const FRAME = '#app #studio-frame';
// The shelf the frame opens on, in its Files tree's order.
const SHELF = [
  'examples/code/web/website.logos',
  'examples/code/basics/control-flow.logos',
  'examples/code/verification/refinement.logos',
  'examples/code/distributed/merge.logos',
  'examples/logic/prover-demo.logic',
  'examples/math/auto-tactic.logos',
  'examples/hardware/handshake.hw',
];

// Open the front page, ready once the live app has painted it and its Studio frame holds an editor.
async function openLanding(t, screen = SCREENS[1]) {
  await t.page.setViewport({ deviceScaleFactor: 1, ...screen });
  await t.load('/', 'h1');
  await t.page.waitForSelector(`${FRAME} .code-editor-textarea`, { timeout: 20000 })
    .catch(() => { throw new Failure('the front page has no Studio with an editor'); });
}

const isFirefox = async (page) => /firefox/i.test(await page.browser().version());

// Where two texts first part, for a failure message.
function firstDifference(a, b) {
  let i = 0;
  while (i < a.length && i < b.length && a[i] === b[i]) i++;
  return i === a.length && i === b.length ? '' : ` (they part at character ${i}: ${JSON.stringify(a.slice(i, i + 40))} against ${JSON.stringify(b.slice(i, i + 40))})`;
}

// The prompt a system's button copies: its install's block of #agent-instructions, by its text,
// shown or not (every block stays hidden until a copy is refused).
async function promptFor(page, install, root = '#app') {
  return page.$eval(`${root} #agent-instructions pre[data-install="${install}"]`, (pre) => pre.textContent);
}

// What the frame's output pane says.
const frameOutput = (page) => page.$eval(`${FRAME} .studio-main > .col-center .panel-content`, (out) => out.innerText.trim());

// The live preview's own document: the sandboxed frame the Studio builds from the editor. A rebuild
// loads a new document, so the frame is found afresh on every look.
async function previewFrame(page) {
  const handle = await page.$(`${FRAME} iframe`);
  return handle ? handle.contentFrame() : null;
}

// Wait until `check` holds inside the preview, and say what it showed when it never does.
async function inPreview(page, check, arg, what, ms = 30000) {
  const until = Date.now() + ms;
  let last = null;
  for (;;) {
    try {
      const frame = await previewFrame(page);
      if (frame) {
        if (await frame.evaluate(check, arg)) return frame;
        last = await frame.evaluate(() => document.body ? document.body.innerText.trim().slice(0, 300) : '(no body)');
      }
    } catch (e) {
      last = `(the frame was reloading: ${String(e.message || e).slice(0, 80)})`;
    }
    if (Date.now() > until) throw new Failure(`${what}; the preview shows ${JSON.stringify(last)}`);
    await new Promise((r) => setTimeout(r, 200));
  }
}

// Open a shelf file from the frame's Files tree, as a reader does, and wait until it is the open one.
async function openFromShelf(page, file) {
  const row = await page.$(`${FRAME} [role="treeitem"][title="/${file}"]`);
  if (!row) throw new Failure(`the frame's Files tree has no row for ${file}`);
  await row.click();
  await page.waitForFunction((root, path) => document.querySelector(`${root} [aria-selected="true"][role="treeitem"]`)?.title === path, { timeout: 15000 }, FRAME, `/${file}`)
    .catch(() => { throw new Failure(`pressing ${file} in the frame did not open it`); });
}

scenario('the_macos_button_copies_the_shell_prompt', async (t) => {
  const firefox = await isFirefox(t.page);
  if (!firefox) await t.page.browserContext().overridePermissions(t.base, ['clipboard-read', 'clipboard-write', 'clipboard-sanitized-write']);
  await openLanding(t);
  await t.page.bringToFront();
  const prompt = await promptFor(t.page, 'unix');
  const lines = prompt.split('\n').length;
  assert(lines === 5, `the page holds a prompt of ${lines} lines`);
  const buttons = await t.page.$$eval('#app button.landing-copy', (bs) => bs.map((b) => [b.dataset.system, b.innerText.trim(), b.querySelector('svg') !== null]));
  assert(JSON.stringify(buttons) === JSON.stringify([['macos', 'Copy agent instructions for macOS', true], ['linux', 'Copy agent instructions for Linux', true], ['windows', 'Copy agent instructions for Windows', true]]), `the copy buttons read ${JSON.stringify(buttons)}`);
  const promptShown = () => t.page.$eval('#app #agent-instructions', (el) => getComputedStyle(el).display !== 'none');
  assert(!(await promptShown()), 'the prompt shows before a copy button is pressed; it should only copy');
  await t.page.click(COPY_BUTTON);
  const outcome = await t.page.waitForFunction((sel) => {
    const button = document.querySelector(sel);
    const status = document.querySelector('#app .landing-pitch [role="status"]');
    if (button && button.innerText.trim() === 'Copied') return 'copied';
    if (status && /blocked copying/.test(status.textContent)) return 'refused';
    return false;
  }, { timeout: 5000 }, COPY_BUTTON).then((h) => h.jsonValue()).catch(() => null);
  assert(outcome, 'pressing the macOS button changed nothing on the page');
  if (outcome === 'copied') {
    const copied = await t.page.evaluate(() => navigator.clipboard.readText().catch(() => null));
    if (!firefox) assert(copied !== null, 'Chrome would not read the clipboard back');
    if (copied !== null) assert(copied === prompt, `the clipboard holds ${copied.length} characters, not the ${prompt.length} of the shell prompt${firstDifference(copied, prompt)}`);
    await t.page.waitForFunction((sel) => document.querySelector(sel).innerText.trim() === 'Copy agent instructions for macOS', { timeout: 8000 }, COPY_BUTTON)
      .catch(() => { throw new Failure('the macOS button never went back to its label'); });
    assert(!(await promptShown()), 'the prompt shows after a copy that went through');
  } else {
    assert(firefox, 'Chrome refused the copy though the page holds the clipboard permission');
    assert(await promptShown(), 'the copy was refused and the prompt is not shown for the reader');
    const selected = await t.page.evaluate(() => String(window.getSelection()));
    assert(selected === prompt, `the copy was refused and the shell prompt is not selected for the reader${firstDifference(selected, prompt)}`);
  }
});

// The Windows button copies the PowerShell prompt, which installs with PowerShell.
scenario('the_windows_button_copies_the_powershell_prompt', async (t) => {
  if (await isFirefox(t.page)) return;
  await t.page.browserContext().overridePermissions(t.base, ['clipboard-read', 'clipboard-write', 'clipboard-sanitized-write']);
  await openLanding(t);
  await t.page.bringToFront();
  const prompt = await promptFor(t.page, 'windows');
  assert(/powershell/i.test(prompt), 'the Windows prompt does not install with PowerShell');
  await t.page.click('#app button.landing-copy[data-system="windows"]');
  await t.page.waitForFunction(() => document.querySelector('#app button.landing-copy[data-system="windows"]').innerText.trim() === 'Copied', { timeout: 5000 });
  const copied = await t.page.evaluate(() => navigator.clipboard.readText().catch(() => null));
  assert(copied === prompt, `the clipboard holds ${JSON.stringify((copied || '').slice(0, 60))}, not the PowerShell prompt`);
}, { engines: ['chrome'] });


scenario('the_heading_the_line_on_how_the_install_box_the_buttons_and_the_studio_stand_in_order', async (t) => {
  for (const screen of SCREENS) {
    await openLanding(t, screen);
    const r = await t.page.evaluate((sel, frame) => {
      const rect = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { top: b.top + scrollY, bottom: b.bottom + scrollY, left: b.left, right: b.right, width: b.width, shown: getComputedStyle(el).display !== 'none' && b.width > 0 }; };
      const h1 = document.querySelector('#app h1');
      return {
        width: document.querySelector('#app .landing-main').clientWidth,
        h1: rect(h1),
        lines: [...h1.querySelectorAll('span')].map(rect),
        how: rect(document.querySelector('#app .landing-how')),
        install: rect(document.querySelector('#app .landing-install')),
        button: rect(document.querySelector(sel)),
        pre: rect(document.querySelector('#app #agent-instructions')),
        studio: rect(document.querySelector(frame)),
        editor: rect(document.querySelector(`${frame} .studio-main > .col-left`)),
        preview: rect(document.querySelector(`${frame} .studio-main > .col-center`)),
      };
    }, COPY_BUTTON, FRAME);
    const name = screenName(screen);
    assert(r.lines.length === 3 && r.lines[0].bottom <= r.lines[1].top + 1 && r.lines[1].bottom <= r.lines[2].top + 1, `at ${name} the heading's three sentences do not stand one below the other`);
    assert(r.how.shown && r.how.top >= r.h1.bottom - 0.5 && r.how.top - r.h1.bottom <= 48, `at ${name} the line on how is not right below the heading (${px(r.how.top - r.h1.bottom)} apart)`);
    assert(r.install.shown && r.install.top >= r.how.bottom - 0.5, `at ${name} the install box is not below the line on how`);
    assert(r.button.top >= r.install.bottom - 0.5 && r.button.top - r.install.bottom <= 64, `at ${name} the copy buttons are not right below the install box (${px(r.button.top - r.install.bottom)} apart)`);
    assert(Math.abs(r.button.left - r.h1.left) <= 1, `at ${name} the copy buttons do not line up with the heading (${px(r.button.left - r.h1.left)})`);
    assert(!r.pre.shown, `at ${name} the prompt is on the page before anyone pressed copy`);
    assert(r.studio.shown && r.studio.top >= r.button.bottom, `at ${name} the Studio does not stand below the words`);
    const wide = Math.min(r.width - 48, 1600) - 1;
    if (screen.width >= 720) assert(r.studio.width >= wide, `at ${name} the Studio is ${px(r.studio.width)} wide, not across the page (${px(wide)})`);
    if (screen.width >= 769) {
      assert(r.preview.left >= r.editor.right - 1 && Math.abs(r.preview.top - r.editor.top) <= 1, `at ${name} the preview does not stand beside the editor`);
    } else if (screen.width > 600) {
      assert(r.preview.top >= r.editor.bottom - 1, `at ${name} the preview is not below the editor`);
    } else {
      assert(r.preview.shown && !r.editor.shown, `at ${name} a phone does not open on the preview alone`);
    }
  }
});

scenario('the_web_tab_previews_its_page_live_and_the_page_works', async (t) => {
  await openLanding(t);
  const chosen = await t.page.$eval(`${FRAME} [role="treeitem"][aria-selected="true"]`, (row) => row.title);
  assert(chosen === '/examples/code/web/website.logos', `the frame opens on ${JSON.stringify(chosen)}`);
  const href = await t.page.$eval(`${FRAME} a.studio-open`, (a) => a.getAttribute('href'));
  assert(href.endsWith('/website.logos'), `the frame opens ${href} in the Studio`);
  const frame = await inPreview(t.page, () => document.querySelector('#title')?.textContent === 'A website in a website', null, 'the preview never painted the page');
  await frame.click('#wave');
  await inPreview(t.page, () => document.querySelector('#hellos')?.textContent === '1 hello so far', null, 'a click inside the preview did not reach the page');
  await (await previewFrame(t.page)).click('#lights');
  await inPreview(t.page, () => document.querySelectorAll('.dark').length === 1, null, 'the lights did not switch inside the preview');

  const editor = `${FRAME} .code-editor-textarea`;
  const before = await t.page.$eval(editor, (area) => area.value);
  await t.page.$eval(editor, (area) => {
    const at = area.value.indexOf('A website in a website</h1>');
    area.focus();
    area.setSelectionRange(at, at + 'A website in a website'.length);
  });
  const started = Date.now();
  await t.page.keyboard.type('A website I just changed');
  const typing = Date.now() - started;
  await inPreview(t.page, () => document.querySelector('#title')?.textContent === 'A website I just changed', null, 'the preview did not follow the edit');
  const after = await t.page.$eval(editor, (area) => area.value);
  assert(after === before.replace('A website in a website</h1>', 'A website I just changed</h1>'), 'the editor holds something other than the edit');
  assert(typing < 24 * 400, `typing 24 characters took ${typing}ms; the preview held up the editor`);
});

// What every Studio example promises, published from its registry by `studio_example_contract.rs`.
function studioContract() {
  const file = path.join(path.dirname(new URL(import.meta.url).pathname), '../../crates/logicaffeine_tests/tests/fixtures/studio_example_contract.json');
  return JSON.parse(fs.readFileSync(file, 'utf8')).examples;
}

// Each shelf file opens in the frame and keeps the promise its registry makes, the Studio's way:
// Logic, Hardware and Code show their output as they open, Math after Execute.
scenario('the_front_page_studio_keeps_each_shelf_files_promise', async (t) => {
  await openLanding(t);
  const contract = studioContract();
  const panel = (title) => t.page.evaluate((root, title) => [...document.querySelectorAll(`${root} .studio-main > section, ${root} .studio-main > aside`)]
    .filter((p) => p.querySelector('.panel-header span')?.innerText.trim() === title).map((p) => p.querySelector('.panel-content')?.innerText || '').join('\n'), FRAME, title);
  const until = async (title, ok, ms = 20000) => {
    const end = Date.now() + ms;
    let text = await panel(title);
    while (!ok(text) && Date.now() < end) { await new Promise((r) => setTimeout(r, 250)); text = await panel(title); }
    return text;
  };
  const off = [];
  for (const file of SHELF.slice(1)) {
    const entry = contract.find((e) => e.file === file);
    if (!entry) { off.push(`${file} is in no contract`); continue; }
    await openFromShelf(t.page, file);
    const p = entry.promise;
    if (p.kind === 'output_exact') {
      const got = await until('Output', (text) => text.trim() === p.output.trim());
      if (got.trim() !== p.output.trim()) off.push(`${file}: Output shows ${JSON.stringify(got.slice(0, 120))}`);
    } else if (p.kind === 'fol') {
      const got = await until('Logic Output', (text) => p.needles.every((n) => text.includes(n)));
      const missing = p.needles.filter((n) => !got.includes(n));
      if (missing.length) off.push(`${file}: Logic Output lacks ${JSON.stringify(missing)}`);
    } else if (p.kind === 'kernel_ok') {
      const button = await t.page.$(`xpath/.//*[@id='studio-frame']//div[contains(@class,'studio-toolbar')]//button[contains(., 'Execute')]`);
      if (!button) { off.push(`${file}: no Execute in the frame's toolbar`); continue; }
      await button.click();
      const got = await until('Output', (text) => text.trim().length > 0);
      const errors = await t.page.$$eval(`${FRAME} .repl-output-line.error`, (els) => els.map((e) => e.innerText.trim()));
      if (!got.trim() || errors.length) off.push(`${file}: Execute shows ${JSON.stringify(errors[0] || got.slice(0, 120))}`);
    } else if (p.kind === 'sva') {
      const got = await until('SVA', (text) => /assert property/.test(text) && /Matches the spec/.test(text));
      if (!(/assert property/.test(got) && /Matches the spec/.test(got))) off.push(`${file}: no certified assertion`);
    } else {
      off.push(`${file}: the scenario reads no ${p.kind} promise`);
    }
  }
  assert(off.length === 0, `${off.length} broken promise(s):\n${off.join('\n')}`);
}, { timeout: 900000 });

// What the reader writes in a file survives opening another file in the frame and coming back.
scenario('the_front_page_studio_keeps_what_the_reader_wrote_across_files', async (t) => {
  await openLanding(t);
  const editor = `${FRAME} .code-editor-textarea`;
  await t.page.$eval(editor, (area) => {
    const at = area.value.indexOf('Say hello</button>');
    area.focus();
    area.setSelectionRange(at, at + 'Say hello'.length);
  });
  await t.page.keyboard.type('Wave');
  await inPreview(t.page, () => document.querySelector('#wave')?.textContent === 'Wave', null, 'the preview did not follow the edit');
  await openFromShelf(t.page, 'examples/logic/prover-demo.logic');
  await t.page.waitForFunction((root) => /socrates/i.test(document.querySelector(`${root} .studio-main > .col-center .panel-content`)?.innerText || ''), { timeout: 30000 }, FRAME);
  await openFromShelf(t.page, 'examples/code/web/website.logos');
  const kept = await t.page.$eval(editor, (area) => area.value);
  assert(kept.includes('>Wave</button>'), `coming back lost the reader's page: ${JSON.stringify(kept.slice(0, 200))}`);
  await inPreview(t.page, () => document.querySelector('#wave')?.textContent === 'Wave', null, 'coming back lost the edited preview');
});

// The frame's Files tree is one tab stop, and the arrows, Home and End walk it.
scenario('the_front_page_tree_follows_the_arrow_keys', async (t) => {
  await openLanding(t);
  const state = () => t.page.evaluate((root) => {
    const rows = [...document.querySelectorAll(`${root} [role="treeitem"]`)];
    return { rows: rows.map((r) => r.title), focused: rows.includes(document.activeElement) ? document.activeElement.title : null, stops: rows.filter((r) => r.tabIndex === 0).length };
  }, FRAME);
  let now = await state();
  assert(now.stops === 1, `${now.stops} rows sit in the Tab order; a tree keeps one`);
  await t.page.focus(`${FRAME} [role="treeitem"][aria-selected="true"]`);
  for (const [key, want] of [['Home', (s) => s.rows[0]], ['End', (s) => s.rows[s.rows.length - 1]], ['ArrowUp', (s) => s.rows[s.rows.length - 2]], ['ArrowDown', (s) => s.rows[s.rows.length - 1]]]) {
    await t.page.keyboard.press(key);
    await t.frames(2);
    now = await state();
    assert(now.focused === want(now), `${key} focused ${now.focused}, not ${want(now)}`);
    assert(now.stops === 1, `after ${key} ${now.stops} rows sit in the Tab order`);
  }
});

// The frame wears the Studio's own look: the same toolbar, Files header, rows and indent, panel
// headers, editor type and status bar, in the same colours, as /studio on the same file.
scenario('the_front_page_studio_wears_the_studios_layout', async (t) => {
  const look = (root) => t.page.evaluate((root) => {
    const at = (sel) => document.querySelector(`${root} ${sel}`);
    const style = (el, keys) => { if (!el) return null; const s = getComputedStyle(el); return Object.fromEntries(keys.map((k) => [k, s[k]])); };
    const box = ['height', 'fontSize', 'fontFamily', 'fontWeight', 'lineHeight', 'color', 'backgroundColor'];
    const rows = [...document.querySelectorAll(`${root} .file-tree [role="treeitem"]`)].slice(0, 4);
    return {
      toolbar: style(at('.studio-toolbar'), box),
      filesHeader: style(at('.file-browser-header'), box),
      firstRow: style(rows[0], box),
      indents: rows.map((r) => getComputedStyle(r).paddingLeft),
      panelHeaders: [...document.querySelectorAll(`${root} .studio-main > section .panel-header`)].map((h) => style(h, box)),
      editor: style(at('.code-editor-textarea'), ['fontSize', 'fontFamily', 'lineHeight', 'color']),
      statusBar: style(at('.studio-statusbar'), box),
      panel: style(at('.studio-main > section'), ['backgroundColor']),
    };
  }, root);
  await t.page.setViewport({ deviceScaleFactor: 1, width: 1280, height: 900 });
  await t.load('/studio?file=examples/code/web/website.logos', '.studio-main > section');
  const studio = await look('.studio-container');
  await openLanding(t, { width: 1280, height: 900 });
  const frame = await look(FRAME);
  const off = Object.keys(studio).filter((k) => JSON.stringify(studio[k]) !== JSON.stringify(frame[k]))
    .map((k) => `${k}:\n  /studio ${JSON.stringify(studio[k]).slice(0, 300)}\n  frame   ${JSON.stringify(frame[k]).slice(0, 300)}`);
  assert(off.length === 0, `the frame does not wear the Studio's look:\n${off.join('\n')}`);
});

// Opening and running every shelf file leaves the address at / and writes nothing into the
// reader's own files: the frame has no store.
scenario('the_front_page_studio_leaves_the_address_and_the_readers_files_alone', async (t) => {
  await openLanding(t);
  const stores = () => t.page.evaluate(async () => {
    const dbs = indexedDB.databases ? (await indexedDB.databases()).map((d) => d.name) : [];
    let files = [];
    try {
      const root = await navigator.storage.getDirectory();
      for await (const name of root.keys()) files.push(name);
    } catch (e) { files = ['(no OPFS)']; }
    return { dbs, files };
  });
  const before = await stores();
  for (const file of SHELF.slice(1)) await openFromShelf(t.page, file);
  await openFromShelf(t.page, SHELF[0]);
  await new Promise((r) => setTimeout(r, 1500));
  const path = await t.page.evaluate(() => location.pathname + location.search);
  assert(path === '/', `the frame moved the address to ${path}`);
  const after = await stores();
  assert(JSON.stringify(after) === JSON.stringify(before), `the frame wrote into the reader's files: ${JSON.stringify(before)} became ${JSON.stringify(after)}`);
});

// Open in Studio opens the file the frame shows, in the Studio.
scenario('open_in_studio_opens_the_file_the_front_page_shows', async (t) => {
  await openLanding(t);
  await openFromShelf(t.page, 'examples/logic/prover-demo.logic');
  await Promise.all([
    t.page.waitForFunction(() => location.pathname === '/studio', { timeout: 30000 }),
    t.page.click(`${FRAME} a.studio-open`),
  ]);
  await t.page.waitForFunction(() => document.querySelector('#app .studio-container [role="treeitem"][aria-selected="true"]')?.title === '/examples/logic/prover-demo.logic', { timeout: 60000 })
    .catch(() => { throw new Failure('the Studio did not open the file the front page showed'); });
  const query = await t.page.evaluate(() => location.search);
  assert(/file=examples\/logic\/prover-demo\.logic/.test(decodeURIComponent(query)), `the Studio's address names ${query}`);
});

// On a phone a swipe inside the editor scrolls the editor, and at its end, or outside the frame,
// the page scrolls on.
scenario('the_editor_scrolls_without_taking_the_page', async (t) => {
  await openLanding(t, SCREENS[3]);
  const tab = await t.page.$(`${FRAME} .mobile-tabs [role="tab"]:nth-child(1)`);
  assert(tab && await tab.isVisible(), 'the phone shows no Editor tab in the frame');
  await tab.tap();
  await t.frames(3);
  await t.page.evaluate((root) => document.querySelector(`${root}`).scrollIntoView({ block: 'center' }), FRAME);
  const read = () => t.page.evaluate((root) => ({ page: document.scrollingElement.scrollTop, editor: document.querySelector(`${root} .studio-main > .col-left textarea`).scrollTop }), FRAME);
  const swipe = async (x, y, dy) => {
    const touch = await t.page.touchscreen.touchStart(x, y);
    for (let i = 1; i <= 10; i++) await touch.move(x, y - (dy * i) / 10);
    await touch.end();
    await new Promise((r) => setTimeout(r, 400));
  };
  const box = await (await t.page.$(`${FRAME} .studio-main > .col-left textarea`)).boundingBox();
  const start = await read();
  await swipe(box.x + box.width / 2, box.y + box.height * 0.7, box.height * 0.4);
  const inside = await read();
  assert(inside.editor > start.editor && inside.page === start.page, `a swipe in the editor moved the editor ${inside.editor - start.editor}px and the page ${inside.page - start.page}px`);
  const outside = await t.page.evaluate(() => document.querySelector('#app h1').getBoundingClientRect());
  await t.page.evaluate(() => window.scrollTo(0, 0));
  const top = await read();
  await swipe(outside.x + outside.width / 2, Math.max(outside.y + 20, 40), 200);
  const after = await read();
  assert(after.page > top.page, 'a swipe outside the frame did not scroll the page');
}, { engines: ['chrome'] });

scenario('the_front_page_fits_a_phone', async (t) => {
  for (const screen of [{ width: 360, height: 740, isMobile: true, hasTouch: true }, SCREENS[3]]) {
    await openLanding(t, screen);
    const fit = await t.page.evaluate(() => {
      const width = innerWidth;
      const chrome = '.main-nav, .mobile-drawer, .mobile-drawer-overlay, .site-footer, [aria-hidden="true"], .landing-visually-hidden';
      const skip = (el) => el.closest(chrome) || el.getClientRects().length === 0;
      const content = [...document.querySelectorAll('#app .landing-main *')].filter((el) => !skip(el));
      const scrollers = [...document.querySelectorAll('#app .landing-main *')].filter((el) => ['auto', 'scroll'].includes(getComputedStyle(el).overflowX));
      const inScroller = (el) => scrollers.some((s) => s !== el && s.contains(el));
      const out = [];
      for (const el of content) {
        if (inScroller(el)) continue;
        const r = el.getBoundingClientRect();
        if (r.width === 0 && r.height === 0) continue;
        if (r.right > width + 0.5 || r.left < -0.5) out.push(`${el.tagName.toLowerCase()}.${el.className} spans ${r.left.toFixed(1)}–${r.right.toFixed(1)}`);
      }
      const small = [];
      for (const el of content.filter((el) => el.matches('a, button'))) {
        if (getComputedStyle(el).display === 'inline') continue;
        const r = el.getBoundingClientRect();
        if (r.height < 44 - 0.5 || r.width < 44 - 0.5) small.push(`${el.tagName.toLowerCase()} "${el.textContent.trim().slice(0, 30)}" is ${r.width.toFixed(0)}×${r.height.toFixed(0)}`);
      }
      const pre = [...document.querySelectorAll('#app #agent-instructions pre')].find((p) => getComputedStyle(p).display !== 'none');
      const editor = document.querySelector('#app #studio-frame .code-editor-textarea');
      return {
        measured: content.length,
        sideways: document.scrollingElement.scrollWidth - document.scrollingElement.clientWidth,
        body: document.body.scrollWidth - document.body.clientWidth,
        out,
        small,
        preSideways: pre.scrollWidth - pre.clientWidth,
        editorFont: parseFloat(getComputedStyle(editor).fontSize),
      };
    });
    const name = screenName(screen);
    assert(fit.measured >= 40, `at ${name} there is too little front page to measure (${fit.measured} elements)`);
    assert(fit.sideways <= 0 && fit.body <= 0, `at ${name} the page scrolls sideways (${fit.sideways}px, body ${fit.body}px)`);
    assert(fit.out.length === 0, `at ${name} parts of the page run past the screen:\n${fit.out.slice(0, 12).join('\n')}`);
    assert(fit.small.length === 0, `at ${name} tap targets are smaller than 44px:\n${fit.small.slice(0, 12).join('\n')}`);
    assert(fit.preSideways <= 0, `at ${name} the prompt scrolls sideways (${fit.preSideways}px); it wraps`);
    assert(fit.editorFont >= 16, `at ${name} the editor is set at ${fit.editorFont}px, so the phone zooms into it`);
  }
});

scenario('reduced_motion_leaves_the_front_page_still', async (t) => {
  await t.page.emulateMediaFeatures([{ name: 'prefers-reduced-motion', value: 'reduce' }]);
  await openLanding(t);
  const moving = await t.page.evaluate(() => {
    const found = [];
    for (const el of document.querySelectorAll('#app .landing, #app .landing *')) {
      const s = getComputedStyle(el);
      if (s.animationName !== 'none') found.push(`${el.tagName.toLowerCase()}.${el.className} animates ${s.animationName}`);
      if (s.transitionDuration.split(',').some((d) => parseFloat(d) > 0)) found.push(`${el.tagName.toLowerCase()}.${el.className} transitions over ${s.transitionDuration}`);
    }
    const running = document.getAnimations().filter((a) => a.effect && a.effect.target && a.effect.target.closest && a.effect.target.closest('#app .landing'));
    return { found, running: running.length };
  });
  assert(moving.found.length === 0 && moving.running === 0, `with reduced motion the front page still moves (${moving.running} running):\n${moving.found.slice(0, 10).join('\n')}`);
  // Media-feature emulation is a DevTools-protocol command; puppeteer's WebDriver BiDi driver for
  // Firefox has none.
}, { engines: ['chrome'] });

// The prerendered copy of the front page is the app's first frame: the same words in the same
// places, so the takeover changes nothing a reader sees. Only a built site carries the copy.
scenario('the_live_front_page_is_its_prerendered_copy', async (t) => {
  const snapshot = (root) => {
    const at = (sel) => document.querySelector(`${root} ${sel}`);
    const rect = (sel) => { const b = at(sel).getBoundingClientRect(); return [b.left, b.top, b.width, b.height].map((n) => Math.round(n)); };
    const prompt = [...document.querySelectorAll(`${root} #agent-instructions pre`)].find((p) => getComputedStyle(p).display !== 'none');
    return {
      heading: at('h1').innerText,
      button: at('button[aria-controls="agent-instructions"]').innerText.trim(),
      prompt: prompt ? prompt.textContent : null,
      selected: at('#studio-frame [role="treeitem"][aria-selected="true"]').title,
      panels: [...document.querySelectorAll(`${root} #studio-frame .studio-main > section .panel-header > span:first-child`)].map((s) => s.textContent.trim()),
      code: at('#studio-frame .code-editor-textarea').value,
      preview: at('#studio-frame iframe').getAttribute('srcdoc'),
      boxes: ['h1', 'button[aria-controls="agent-instructions"]', '#agent-instructions', '#studio-frame', '#studio-frame iframe'].map(rect),
    };
  };
  const prerendered = await t.page.browserContext().newPage();
  await prerendered.setViewport({ deviceScaleFactor: 1, ...SCREENS[1] });
  await prerendered.setRequestInterception(true);
  prerendered.on('request', (request) => (new URL(request.url()).pathname.endsWith('.wasm') ? request.abort() : request.continue()));
  await prerendered.goto(`${t.base}/`, { waitUntil: 'load', timeout: 120000 });
  await prerendered.evaluate(() => document.fonts.ready);
  const hasCopy = await prerendered.evaluate(() => !!document.querySelector('#main #studio-frame'));
  if (!hasCopy) throw new Failure('the build carries no prerendered front page (build it with --ssg)');
  const before = await prerendered.evaluate(snapshot, '#main');
  await prerendered.close();
  await openLanding(t);
  const after = await t.page.evaluate(snapshot, '#app');
  for (const key of Object.keys(before)) {
    assert(JSON.stringify(before[key]) === JSON.stringify(after[key]), `the live page's ${key} differs from the prerendered copy's:\n  prerendered ${JSON.stringify(before[key]).slice(0, 300)}\n  live        ${JSON.stringify(after[key]).slice(0, 300)}`);
  }
}, { needsBuild: true });


// ---------------------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------------------

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
    console.error('site-e2e: give --url <running site> or --public <built site with index.html>');
    return 2;
  }
  const selected = opts.only ? scenarios.filter((s) => opts.only.includes(s.name)) : scenarios;
  const unknown = (opts.only || []).filter((n) => !scenarios.some((s) => s.name === n));
  if (unknown.length) { console.error(`site-e2e: unknown scenario(s): ${unknown.join(', ')}`); return 2; }

  const puppeteer = require('puppeteer-core');
  fs.mkdirSync(opts.shots, { recursive: true });
  const server = opts.url ? { url: opts.url, close() {} } : await serve(path.resolve(opts.public));
  const engine = opts.firefox ? 'firefox' : 'chrome';
  // A cold first load after a rebuild can outlast puppeteer's default 180 s protocol ceiling;
  // `load` already allows 300 s, so the ceiling must sit above every wait we ask for.
  const protocolTimeout = 660000;
  const browser = await puppeteer.launch(opts.firefox
    ? { browser: 'firefox', executablePath: opts.firefox, headless: true, protocolTimeout }
    : { executablePath: opts.chrome, headless: true, protocolTimeout, args: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage'] });

  const results = [];
  for (const s of selected) {
    if (s.opts.engines && !s.opts.engines.includes(engine)) {
      results.push({ ...s, skipped: `${engine} cannot show it`, seconds: 0 });
      console.log(`skip  ${s.name} (${engine} cannot show it)`);
      continue;
    }
    // A dev server renders no prerendered copy: what is measured against one needs a built site.
    if (s.opts.needsBuild && opts.url) {
      results.push({ ...s, skipped: 'it needs a built site (--public)', seconds: 0 });
      console.log(`skip  ${s.name} (it needs a built site, --public)`);
      continue;
    }
    const started = Date.now();
    const context = await browser.createBrowserContext();
    const page = await context.newPage();
    const errors = [];
    // What the page said that is the dev server's or the harness's doing, not the app's: reported
    // with a failure.
    const failedRequests = [];
    const navigation = { leaving: false };
    page.on('pageerror', (e) => {
      const text = String(e.message || e);
      if (DEV_SERVER_SWAP.test(text)) failedRequests.push(`dev server mid-swap: ${text.slice(0, 200)}`);
      else if (navigation.leaving && NAVIGATION_ABORT.test(text)) failedRequests.push(`left behind by a navigation: ${text.slice(0, 200)}`);
      else errors.push(`pageerror: ${text.slice(0, 300)}`);
    });
    page.on('console', (m) => {
      if (m.type() !== 'error' || DEV_CLIENT_NOISE.test(m.text())) return;
      if (DEV_SERVER_SWAP.test(m.text())) failedRequests.push(`dev server mid-swap: ${m.text().slice(0, 200)}`);
      else errors.push(`console.error: ${m.text().slice(0, 300)}`);
    });
    page.on('requestfailed', (r) => failedRequests.push(`request failed: ${r.url().slice(0, 200)} (${r.failure()?.errorText || 'no reason'})`));
    let failure = null;
    try {
      const limit = s.opts.timeout || 600000;
      await Promise.race([
        s.fn(helpers(page, server.url, navigation)),
        new Promise((_, reject) => setTimeout(() => reject(new Failure(`timed out after ${limit / 1000}s`)), limit)),
      ]);
      const unexpected = errors.filter((e) => !(s.opts.allowErrors && s.opts.allowErrors.test(e)));
      if (unexpected.length) throw new Failure(`page errors:\n${unexpected.join('\n')}`);
    } catch (e) {
      failure = e instanceof Failure ? e.message : `harness error: ${e.stack || e}`;
      // A failure carries what the page itself said, so a page that never booted says why.
      const said = [...(failure.startsWith('page errors:') ? [] : errors), ...failedRequests];
      if (said.length) failure += `\npage said:\n${said.slice(0, 12).join('\n')}`;
      await page.screenshot({ path: path.join(opts.shots, `${s.name}.png`) }).catch(() => {});
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
    const cases = results.map((r) => `    <testcase classname="site-e2e.${engine}" name="${xmlEscape(r.name)}" time="${r.seconds.toFixed(3)}">` +
      (r.failure ? `\n      <failure message="${xmlEscape(r.failure.split('\n')[0])}">${xmlEscape(r.failure)}</failure>\n    ` : '') +
      (r.skipped ? `<skipped message="${xmlEscape(r.skipped)}"/>` : '') + '</testcase>');
    fs.writeFileSync(opts.junit, `<?xml version="1.0" encoding="UTF-8"?>\n<testsuites name="site-e2e" tests="${results.length}" failures="${failed}" skipped="${skipped}">\n  <testsuite name="site-e2e" tests="${results.length}" failures="${failed}" skipped="${skipped}">\n${cases.join('\n')}\n  </testsuite>\n</testsuites>\n`);
  }
  console.log(`site-e2e (${engine}): ${ran - failed}/${ran} passed${skipped ? `, ${skipped} skipped` : ''}${failed ? ` — screenshots of each failure in ${opts.shots}` : ''}`);
  return failed ? 1 : 0;
}

main().then((code) => process.exit(code), (e) => { console.error(e); process.exit(2); });
