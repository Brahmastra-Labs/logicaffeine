// Draw the share card: apps/logicaffeine_web/share-card/card.html, at 1200x630, into
// apps/logicaffeine_web/public/assets/OG-photo.png, the image every page's og:image names.
//
// Before it draws, it checks the page drew what it says: every font loaded, every image decoded,
// nothing spilled past the card's margins and each heading line stayed one line. A failed check
// writes nothing. The PNG it writes carries a tEXt chunk, keyword `logos-share-card`, with one
// line per file the card is drawn from (the page, then each local file it references, in order):
// the file's FNV-1a 64 digest in hex and its path from the repository root.
// tests/share_card_lock.rs recomputes those digests, so a card edited and not redrawn is caught.
//
// Usage: node scripts/web/render-share-card.mjs <chrome-path>   (see render-share-card.sh)
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';

const ROOT = path.resolve(import.meta.dirname, '../..');
const CARD = 'apps/logicaffeine_web/share-card/card.html';
const OUT = 'apps/logicaffeine_web/public/assets/OG-photo.png';
const WIDTH = 1200;
const HEIGHT = 630;
const KEYWORD = 'logos-share-card';
// A remote file the card draws from is pinned to one published version, so a redraw draws the same.
const PINNED = /^https:\/\/cdn\.jsdelivr\.net\/npm\/(@[a-z0-9-]+\/)?[a-z0-9.-]+@\d+\.\d+\.\d+\//;

const chromePath = process.argv[2] || process.env.CHROME_BIN;

function fail(message) {
  console.error(`render-share-card: ${message}`);
  process.exit(1);
}

function fnv1a64(bytes) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash.toString(16).padStart(16, '0');
}

// The card and every local file it references, as paths from the repository root.
function sources() {
  const html = fs.readFileSync(path.join(ROOT, CARD), 'utf8');
  const found = [CARD];
  for (const [, reference] of html.matchAll(/(?:src="|url\(")([^"]+)"/g)) {
    if (/^[a-z]+:/.test(reference)) {
      if (!PINNED.test(reference)) fail(`the card draws from ${reference}, which is not pinned to a published version`);
      continue;
    }
    const file = path.relative(ROOT, path.resolve(ROOT, path.dirname(CARD), reference));
    if (!fs.existsSync(path.join(ROOT, file))) fail(`the card references ${reference}, and ${file} does not exist`);
    if (!found.includes(file)) found.push(file);
  }
  return found;
}

function chunk(type, data) {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, 'latin1');
  data.copy(out, 8);
  out.writeUInt32BE(zlib.crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}

// The screenshot rewritten: its image data deflated at the highest level, and the sources recorded
// right after the header.
function withSources(png, record) {
  const signature = png.subarray(0, 8);
  const ancillary = [];
  const data = [];
  let header;
  for (let at = 8; at < png.length; ) {
    const length = png.readUInt32BE(at);
    const type = png.toString('latin1', at + 4, at + 8);
    const body = png.subarray(at + 8, at + 8 + length);
    if (type === 'IHDR') header = body;
    else if (type === 'IDAT') data.push(body);
    else if (type !== 'IEND' && type !== 'tEXt') ancillary.push(chunk(type, body));
    at += 12 + length;
  }
  const pixels = zlib.inflateSync(Buffer.concat(data));
  return Buffer.concat([
    signature,
    chunk('IHDR', header),
    chunk('tEXt', Buffer.from(`${KEYWORD}\0${record}`, 'latin1')),
    ...ancillary,
    chunk('IDAT', zlib.deflateSync(pixels, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

if (typeof zlib.crc32 !== 'function') fail(`node ${process.version} has no zlib.crc32 (node 22.2 or later)`);
if (!chromePath || !fs.existsSync(chromePath)) fail('no Chrome binary (pass its path or set CHROME_BIN)');
let puppeteer;
try {
  puppeteer = createRequire(import.meta.url)('puppeteer-core');
} catch {
  fail('puppeteer-core is not installed (npm install --no-save puppeteer-core, or put it on NODE_PATH)');
}

const files = sources();
const record = files.map((file) => `${fnv1a64(fs.readFileSync(path.join(ROOT, file)))} ${file}`).join('\n');

const browser = await puppeteer.launch({
  executablePath: chromePath,
  headless: true,
  args: ['--allow-file-access-from-files', '--hide-scrollbars', '--force-color-profile=srgb'],
});
try {
  const page = await browser.newPage();
  await page.setViewport({ width: WIDTH, height: HEIGHT, deviceScaleFactor: 1 });
  const response = await page.goto(pathToFileURL(path.join(ROOT, CARD)).href, { waitUntil: 'networkidle0' });
  if (!response || !response.ok()) fail(`the card did not load from ${CARD}`);
  await page.evaluate(() => document.fonts.ready);

  const drawn = await page.evaluate(() => {
    const card = document.querySelector('.card');
    const line = (span) => parseFloat(getComputedStyle(span).lineHeight);
    return {
      fonts: [...document.fonts].map((font) => ({ family: font.family, status: font.status })),
      images: [...document.images].map((image) => ({ src: image.getAttribute('src'), drawn: image.complete && image.naturalWidth > 0 })),
      spill: card.scrollWidth > card.clientWidth || card.scrollHeight > card.clientHeight,
      wrapped: [...document.querySelectorAll('h1 span')]
        .filter((span) => span.getBoundingClientRect().height > line(span) * 1.5)
        .map((span) => span.textContent),
    };
  });
  for (const font of drawn.fonts) {
    if (font.status !== 'loaded') fail(`the font ${font.family} did not load (${font.status})`);
  }
  if (drawn.fonts.length === 0) fail('the card declares no font, so it would draw in whatever the machine has');
  for (const image of drawn.images) {
    if (!image.drawn) fail(`the image ${image.src} did not decode`);
  }
  if (drawn.spill) fail('the card spills past its margins');
  if (drawn.wrapped.length) fail(`a heading line wrapped: ${drawn.wrapped.join(' | ')}`);

  const shot = await page.screenshot({ type: 'png', clip: { x: 0, y: 0, width: WIDTH, height: HEIGHT } });
  const png = withSources(Buffer.from(shot), record);
  fs.writeFileSync(path.join(ROOT, OUT), png);
  console.log(`render-share-card: wrote ${OUT} (${WIDTH}x${HEIGHT}, ${png.length} bytes) from\n${record}`);
} finally {
  await browser.close();
}
