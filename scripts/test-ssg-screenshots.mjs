// Screenshot tests for every prerendered route.
//
// For each route in the sitemap: load it in headless Chrome, wait for the wasm
// CSR takeover, then assert
//   1. no page errors / failed requests,
//   2. the route's content marker is present in the live DOM,
//   3. the takeover completed (#main removed, content committed by the app),
//   4. the rendered PIXELS carry real content — a sharp-edge density check that
//      fails on a blank/gradient-only viewport (the styled-but-empty bug class),
// and save the screenshot for human review.
//
// Usage: node scripts/test-ssg-screenshots.mjs <base-url> [chrome-path] [shots-dir]
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { createRequire } from 'node:module';

const base = process.argv[2] || 'http://127.0.0.1:8080';
const chromePath = process.argv[3] || process.env.CHROME_BIN;
const shotsDir = process.argv[4] || '/tmp/ssg-shots';

const require = createRequire(import.meta.url);
let puppeteer;
try {
  puppeteer = require('puppeteer-core');
} catch {
  console.error('SKIP: puppeteer-core not installed (npm install --no-save puppeteer-core)');
  process.exit(0);
}
if (!chromePath || !fs.existsSync(chromePath)) {
  console.error('SKIP: no Chrome binary (set CHROME_BIN)');
  process.exit(0);
}

// Route -> marker the live DOM must contain after takeover. News slugs are
// checked generically (their <h1> must be non-empty).
const MARKERS = {
  '/': 'Debug Your Thoughts.',
  '/pricing': 'Contact Us',
  '/privacy': 'Privacy Policy',
  '/terms': 'legally binding agreement',
  '/roadmap': 'LOGOS Roadmap',
  '/guide': 'LOGOS Syntax Guide',
  '/crates': 'Crate Documentation',
  '/news': 'Latest updates, release notes, and announcements',
  '/learn': 'Learn Logic',
  '/benchmarks': 'Benchmarks',
  '/studio': 'English Input',
  '/registry': 'Package Registry',
  '/profile': 'Logic Learner',
};

// --- minimal PNG decode (RGB8/RGBA8, non-interlaced) for the pixel check ----
function decodePng(buf) {
  let pos = 8;
  let w = 0, h = 0, bpp = 4, idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString('ascii', pos + 4, pos + 8);
    const body = buf.subarray(pos + 8, pos + 8 + len);
    if (type === 'IHDR') {
      w = body.readUInt32BE(0);
      h = body.readUInt32BE(4);
      if (body[8] !== 8 || (body[9] !== 6 && body[9] !== 2) || body[12] !== 0)
        throw new Error(`unsupported PNG shape (depth ${body[8]} color ${body[9]})`);
      bpp = body[9] === 6 ? 4 : 3;
    } else if (type === 'IDAT') idat.push(body);
    else if (type === 'IEND') break;
    pos += 12 + len;
  }
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = w * bpp;
  const px = Buffer.alloc(h * stride);
  let prev = Buffer.alloc(stride);
  let p = 0;
  const paeth = (a, b, c) => {
    const pp = a + b - c, pa = Math.abs(pp - a), pb = Math.abs(pp - b), pc = Math.abs(pp - c);
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
  };
  for (let y = 0; y < h; y++) {
    const f = raw[p++];
    const line = Buffer.from(raw.subarray(p, p + stride));
    p += stride;
    for (let i = 0; i < stride; i++) {
      const a = i >= bpp ? line[i - bpp] : 0, b = prev[i], c = i >= bpp ? prev[i - bpp] : 0;
      if (f === 1) line[i] = (line[i] + a) & 0xff;
      else if (f === 2) line[i] = (line[i] + b) & 0xff;
      else if (f === 3) line[i] = (line[i] + ((a + b) >> 1)) & 0xff;
      else if (f === 4) line[i] = (line[i] + paeth(a, b, c)) & 0xff;
    }
    line.copy(px, y * stride);
    prev = line;
  }
  return { w, h, px, bpp };
}

// Sharp-edge density: fraction of pixels differing from their left neighbor by
// >24 in any channel. Text/UI-rich pages measure percents; a blank gradient
// measures ~0.
function edgeDensity(png) {
  const { w, h, px, bpp } = png;
  let edges = 0;
  for (let y = 0; y < h; y += 2) {
    for (let x = 1; x < w; x += 2) {
      const i = (y * w + x) * bpp, j = i - bpp;
      if (
        Math.abs(px[i] - px[j]) > 24 ||
        Math.abs(px[i + 1] - px[j + 1]) > 24 ||
        Math.abs(px[i + 2] - px[j + 2]) > 24
      ) edges++;
    }
  }
  return edges / ((w / 2) * (h / 2));
}

const routes = [];
const sitemap = fs.readFileSync(
  path.join(import.meta.dirname, '../apps/logicaffeine_web/public/sitemap.xml'),
  'utf8',
);
for (const m of sitemap.matchAll(/<loc>https:\/\/logicaffeine\.com([^<]*)<\/loc>/g)) {
  routes.push(m[1] || '/');
}

fs.mkdirSync(shotsDir, { recursive: true });
const browser = await puppeteer.launch({
  executablePath: chromePath,
  headless: 'new',
  args: ['--no-sandbox', '--disable-gpu', '--window-size=1280,900'],
});

let failures = 0;
for (const route of routes) {
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 900 });
  const errors = [];
  page.on('pageerror', (e) => errors.push(`pageerror: ${String(e).slice(0, 200)}`));
  page.on('requestfailed', (r) => errors.push(`requestfailed: ${r.url().split('/').pop()}`));
  page.on('console', (m) => {
    if (m.type() === 'error' && !m.text().includes('OPFS')) errors.push(`console.error: ${m.text().slice(0, 200)}`);
  });

  const problems = [];
  try {
    await page.goto(base + route, { waitUntil: 'networkidle2', timeout: 45000 });
    // Wait for the takeover to complete (app committed + prerendered copy gone).
    await page
      .waitForFunction(
        () => !document.getElementById('main') && document.body.innerText.trim().length > 50,
        { timeout: 20000 },
      )
      .catch(() => problems.push('takeover never completed (#main still present or app text empty)'));

    const state = await page.evaluate(() => ({
      text: document.body.innerText,
      h1: document.querySelector('h1')?.innerText?.trim() ?? '',
    }));
    const marker = MARKERS[route];
    if (marker && !state.text.includes(marker)) problems.push(`marker ${JSON.stringify(marker)} missing`);
    if (!marker && route.startsWith('/news/') && state.h1.length === 0) problems.push('article has no h1');

    const shotFile = path.join(shotsDir, (route === '/' ? 'landing' : route.slice(1).replace(/\//g, '_')) + '.png');
    const shot = await page.screenshot({ path: shotFile });
    const density = edgeDensity(decodePng(Buffer.from(shot)));
    if (density < 0.005) problems.push(`viewport looks blank (edge density ${(density * 100).toFixed(3)}%)`);

    problems.push(...errors);
  } catch (e) {
    problems.push(`load failed: ${String(e).slice(0, 200)}`);
  }
  await page.close();

  if (problems.length) {
    failures++;
    console.log(`FAIL ${route}\n      ${problems.join('\n      ')}`);
  } else {
    console.log(`  ok  ${route}`);
  }
}

await browser.close();
if (failures) {
  console.log(`test-ssg-screenshots: FAILED (${failures}/${routes.length} routes)`);
  process.exit(1);
}
console.log(`test-ssg-screenshots: OK — ${routes.length} routes rendered, screenshots in ${shotsDir}`);
