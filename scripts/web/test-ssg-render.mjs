// Per-route render lock — the gate a blank homepage slipped past.
//
// For EVERY prerendered route: load it, and WITHOUT pumping the page's event loop
// (no `waitForFunction`, no virtual-time — those keep the loop serviced and hide a
// stalled/crashed render), confirm the user sees a real, rendered page:
//   1. the screenshot is not blank (edge-density > threshold — screenshots do not
//      run page JS, so they are the honest blank/not-blank signal),
//   2. the LIVE app committed real content into #app-root inside #app — the
//      prerendered copy in #main carries an #app-root of its own, so the static
//      fallback can never pass for the app, and
//   3. no uncaught page error fired (the wasm-split boot crash
//      `reading 'listening'` shows up here).
//
// Then, for every route again with the wasm refused:
//   4. the page keeps its prerendered copy. The shell's takeover watchdog may hide
//      #main only while the live app holds real content; an app that never boots
//      must leave the reader on the prerendered page, well past the watchdog's hold
//      window — never on a blank screen — and nothing may pass for a live app. That
//      page must also be styled: the site palette's tokens resolve on it.
//   5. an app that boots late still takes over. With the wasm held back past the
//      watchdog's give-up window (read from index.html), the live app must replace
//      the prerendered copy once it arrives — not render beside a copy that stays.
//      The watchdog is shell code shared by every page, so the landing and /learn
//      stand in for all routes; each run waits out the whole window.
//
// The existing test-ssg-screenshots.mjs uses waitForFunction, which pumps the loop
// and passed while production rendered blank — this lock is deliberately different.
//
// Usage: node scripts/web/test-ssg-render.mjs <base-url> [chrome-path]
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { createRequire } from 'node:module';

const base = process.argv[2] || 'http://127.0.0.1:8782';
const chromePath = process.argv[3] || process.env.CHROME_BIN;
const require = createRequire(import.meta.url);

let puppeteer;
try { puppeteer = require('puppeteer-core'); }
catch { console.error('SKIP: puppeteer-core not installed'); process.exit(0); }
if (!chromePath || !fs.existsSync(chromePath)) { console.error('SKIP: no Chrome binary (set CHROME_BIN)'); process.exit(0); }

function decodePng(buf) {
  let pos = 8, w = 0, h = 0, bpp = 4, idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos), type = buf.toString('ascii', pos + 4, pos + 8);
    const body = buf.subarray(pos + 8, pos + 8 + len);
    if (type === 'IHDR') { w = body.readUInt32BE(0); h = body.readUInt32BE(4); bpp = body[9] === 6 ? 4 : 3; }
    else if (type === 'IDAT') idat.push(body); else if (type === 'IEND') break;
    pos += 12 + len;
  }
  const raw = zlib.inflateSync(Buffer.concat(idat)), stride = w * bpp, px = Buffer.alloc(h * stride);
  let prev = Buffer.alloc(stride), p = 0;
  const paeth = (a, b, c) => { const pp = a + b - c, pa = Math.abs(pp - a), pb = Math.abs(pp - b), pc = Math.abs(pp - c); return pa <= pb && pa <= pc ? a : pb <= pc ? b : c; };
  for (let y = 0; y < h; y++) {
    const f = raw[p++]; const line = Buffer.from(raw.subarray(p, p + stride)); p += stride;
    for (let i = 0; i < stride; i++) { const a = i >= bpp ? line[i - bpp] : 0, b = prev[i], c = i >= bpp ? prev[i - bpp] : 0;
      if (f === 1) line[i] = (line[i] + a) & 255; else if (f === 2) line[i] = (line[i] + b) & 255;
      else if (f === 3) line[i] = (line[i] + ((a + b) >> 1)) & 255; else if (f === 4) line[i] = (line[i] + paeth(a, b, c)) & 255; }
    line.copy(px, y * stride); prev = line;
  }
  return { w, h, px, bpp };
}
function edgeDensity(png) {
  const { w, h, px, bpp } = png; let edges = 0;
  for (let y = 0; y < h; y += 2) for (let x = 1; x < w; x += 2) {
    const i = (y * w + x) * bpp, j = i - bpp;
    if (Math.abs(px[i] - px[j]) > 24 || Math.abs(px[i + 1] - px[j + 1]) > 24 || Math.abs(px[i + 2] - px[j + 2]) > 24) edges++;
  }
  return edges / ((w / 2) * (h / 2));
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const MIN_DENSITY = 0.006;   // a styled-but-empty gradient measures ~0
const MIN_APP_TEXT = 50;     // the live app committed real content
const MAX_WAIT_MS = 18000;   // generous: heavy routes (studio) boot slowly
const STRANDED_MS = 4000;    // past the watchdog's 1.5 s hold, with margin
const shell = fs.readFileSync(path.join(import.meta.dirname, '../../apps/logicaffeine_web/index.html'), 'utf8');
const GIVE_UP_MS = Number(shell.match(/GIVE_UP_MS\s*=\s*(\d+)/)?.[1]);
if (!GIVE_UP_MS) { console.log('test-ssg-render: FAILED — no GIVE_UP_MS in index.html\'s takeover watchdog'); process.exit(1); }
const LATE_ROUTES = ['/', '/learn'];

// What the live app has rendered — judged inside #app, where the wasm mounts, and
// nowhere else: the prerendered copy in #main repeats every id the app renders.
function liveApp() {
  const root = document.querySelector('#app #app-root');
  return { kids: root?.childElementCount ?? 0, textLen: (root?.innerText || '').trim().length };
}

const routes = [];
const sitemap = fs.readFileSync(path.join(import.meta.dirname, '../../apps/logicaffeine_web/public/sitemap.xml'), 'utf8');
for (const m of sitemap.matchAll(/<loc>https:\/\/logicaffeine\.com([^<]*)<\/loc>/g)) routes.push(m[1] || '/');

const browser = await puppeteer.launch({ executablePath: chromePath, headless: 'new', args: ['--no-sandbox', '--disable-gpu', '--window-size=1280,900'] });

async function checkRoute(route, viewport, label) {
  const page = await browser.newPage();
  await page.setViewport(viewport);
  const errors = [];
  page.on('pageerror', (e) => errors.push(`pageerror: ${String(e).slice(0, 160)}`));
  const problems = [];
  try {
    await page.goto(base + route, { waitUntil: 'domcontentloaded', timeout: 45000 });
    let density = 0, kids = 0, textLen = 0;
    const deadline = Date.now() + MAX_WAIT_MS;
    // Bounded poll — NOT waitForFunction. Pass as soon as the live app has painted.
    while (Date.now() < deadline) {
      await sleep(1000);
      density = edgeDensity(decodePng(Buffer.from(await page.screenshot())));
      const dom = await page.evaluate(liveApp);
      kids = dom.kids; textLen = dom.textLen;
      if (errors.length) break;
      if (density >= MIN_DENSITY && kids > 0 && textLen >= MIN_APP_TEXT) break;
    }
    if (density < MIN_DENSITY) problems.push(`viewport blank (edge density ${(density * 100).toFixed(3)}%)`);
    if (kids === 0 || textLen < MIN_APP_TEXT) problems.push(`live app did not render into #app-root (kids=${kids}, textLen=${textLen})`);
    problems.push(...errors);
  } catch (e) { problems.push(`load failed: ${String(e).slice(0, 160)}`); }
  await page.close();
  return problems;
}

async function checkStranded(route) {
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 900 });
  await page.setRequestInterception(true);
  page.on('request', (req) => (new URL(req.url()).pathname.endsWith('.wasm') ? req.abort() : req.continue()));
  const problems = [];
  try {
    await page.goto(base + route, { waitUntil: 'domcontentloaded', timeout: 45000 });
    await sleep(STRANDED_MS);
    const density = edgeDensity(decodePng(Buffer.from(await page.screenshot())));
    const copy = await page.evaluate(() => {
      const main = document.getElementById('main');
      return {
        present: !!main,
        shown: !!main && getComputedStyle(main).display !== 'none',
        palette: getComputedStyle(document.documentElement).getPropertyValue('--bg-gradient-start').trim(),
      };
    });
    const app = await page.evaluate(liveApp);
    if (!copy.palette) problems.push('the prerendered page paints without the site palette (--bg-gradient-start is undefined)');
    if (!copy.present) problems.push('the prerendered copy (#main) was removed though the app never booted');
    else if (!copy.shown) problems.push('the prerendered copy (#main) is hidden though the app never booted');
    if (density < MIN_DENSITY) problems.push(`viewport blank (edge density ${(density * 100).toFixed(3)}%)`);
    if (app.kids > 0 || app.textLen > 0) problems.push(`something passed for the live app with no wasm (kids=${app.kids}, textLen=${app.textLen})`);
  } catch (e) { problems.push(`load failed: ${String(e).slice(0, 160)}`); }
  await page.close();
  return problems;
}

async function checkLateBoot(route) {
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 900 });
  await page.setRequestInterception(true);
  page.on('request', (req) => {
    if (new URL(req.url()).pathname.endsWith('.wasm')) setTimeout(() => req.continue(), GIVE_UP_MS + 3000);
    else req.continue();
  });
  const errors = [];
  page.on('pageerror', (e) => errors.push(`pageerror: ${String(e).slice(0, 160)}`));
  const problems = [];
  try {
    await page.goto(base + route, { waitUntil: 'domcontentloaded', timeout: 45000 });
    const deadline = Date.now() + GIVE_UP_MS + 3000 + MAX_WAIT_MS;
    let app = { kids: 0, textLen: 0 };
    while (Date.now() < deadline) {
      await sleep(1000);
      app = await page.evaluate(liveApp);
      if (errors.length || (app.kids > 0 && app.textLen >= MIN_APP_TEXT)) break;
    }
    if (app.kids === 0 || app.textLen < MIN_APP_TEXT) problems.push(`the late app never rendered into #app (kids=${app.kids}, textLen=${app.textLen})`);
    else {
      await sleep(STRANDED_MS);
      const copy = await page.evaluate(() => {
        const main = document.getElementById('main');
        return { present: !!main, shown: !!main && getComputedStyle(main).display !== 'none' };
      });
      if (copy.shown) problems.push('the prerendered copy (#main) is still shown beside the live app — the page appears twice');
      else if (copy.present) problems.push('the prerendered copy (#main) was hidden but never removed — the takeover did not complete');
    }
    problems.push(...errors);
  } catch (e) { problems.push(`load failed: ${String(e).slice(0, 160)}`); }
  await page.close();
  return problems;
}

let failures = 0;
for (const route of routes) {
  const problems = await checkRoute(route, { width: 1280, height: 900 }, 'desktop');
  if (problems.length) { failures++; console.log(`FAIL ${route}\n      ${problems.join('\n      ')}`); }
  else console.log(`  ok  ${route}`);
}
// The reported bug was mobile-first — assert the landing also renders on a phone.
const mobileProblems = await checkRoute('/', { width: 390, height: 844, isMobile: true, deviceScaleFactor: 2 }, 'mobile');
if (mobileProblems.length) { failures++; console.log(`FAIL / (mobile 390px)\n      ${mobileProblems.join('\n      ')}`); }
else console.log('  ok  / (mobile 390px)');

// A page is never stranded blank: with the wasm refused, every route keeps its prerendered copy.
for (const route of routes) {
  const problems = await checkStranded(route);
  if (problems.length) { failures++; console.log(`FAIL ${route} (app never boots)\n      ${problems.join('\n      ')}`); }
  else console.log(`  ok  ${route} (app never boots)`);
}

// A slow boot still takes over: the app arrives after the watchdog's give-up window.
for (const route of LATE_ROUTES) {
  const problems = await checkLateBoot(route);
  if (problems.length) { failures++; console.log(`FAIL ${route} (app boots after ${GIVE_UP_MS / 1000}s)\n      ${problems.join('\n      ')}`); }
  else console.log(`  ok  ${route} (app boots after ${GIVE_UP_MS / 1000}s)`);
}

await browser.close();
if (failures) { console.log(`test-ssg-render: FAILED (${failures} check(s) blank/crashed/stranded)`); process.exit(1); }
console.log(`test-ssg-render: OK — ${routes.length} routes + mobile landing all rendered live; every route keeps its prerendered page when the app never boots; a late app still takes over`);
