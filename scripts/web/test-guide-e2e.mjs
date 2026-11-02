// The Syntax Guide in a real browser — the layer its editors and callouts live in.
//
// Every LOGOS example, exercise and prose snippet on /guide is edited in the Studio's own editor
// (apps/logicaffeine_web/src/ui/components/code_editor.rs) with the language server on, and every
// callout with a coloured left edge is square on that edge. What the page renders natively is
// pinned by crates/logicaffeine_tests/tests/guide_render.rs and what the language server says about
// the guide's code by tests/guide_language_server.rs; what only a browser can show lives here: a
// resting pointer's hover and its grace period, squiggles painted under words, the colours the
// page computes, an editor growing as the reader types, the corners the callouts are drawn with.
//
// Every scenario runs in a fresh browser context, fails on any uncaught page error or console
// error, and leaves a screenshot of the failing moment. Scenarios find the page's parts by what
// the page renders (an example's anchor, its words, its buttons), never by internal hooks, so the
// same scenario proves a bug on the old page (RED) and its absence on the fixed one (GREEN).
//
// Usage: node scripts/web/test-guide-e2e.mjs (--public <built-public-dir> | --url <running-site>)
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
  const out = { only: null, junit: null, shots: '/tmp/guide-e2e-shots', list: false };
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
// What the page promises
// ---------------------------------------------------------------------------------------

const DESKTOP = { width: 1920, height: 1080 };
const PHONE = { width: 390, height: 844, isMobile: true, hasTouch: true };

// The corners of a callout with a coloured left edge: square where the accent runs, 4px on the
// side it opens toward.
const ACCENT_RADII = { tl: '0px', bl: '0px', tr: '4px', br: '4px' };

// The kinds of left-accent callout the guide shows — the scan must meet every one, so a scan
// that stopped finding callouts can never pass for one that found them all square.
const ACCENT_KINDS = [
  ['the section hook', '.guide-hook'],
  ['a mental-model callout', 'blockquote.guide-mental-model'],
  ['a look-back callout', 'blockquote.guide-look-back'],
  ['a misconception box', '.guide-misconception'],
  ['the active sidebar entry', '.sidebar-section.active'],
];

// A program with a type mistake the language server reports as an error, and the same program
// put right.
const MISTAKE = '## To f () -> Int:\n    Return "oops".\n\n## Main\nShow f().\n';

// ---------------------------------------------------------------------------------------
// Assertions and page helpers
// ---------------------------------------------------------------------------------------

class Failure extends Error {}

// A debug build carries dx's hot-reload client, which dials a devserver socket a plain static
// server cannot answer. That error belongs to the build's tooling, never to the page.
const DEV_CLIENT_NOISE = /WebSocket connection to '[^']*\/_dioxus\?/;
// The dev server swaps each build into place; a page that loads mid-swap is sent the app's loader
// script with no content type, the browser refuses it, and the app never starts — Chrome says so
// on the console, Firefox as a page error. It is the server's race, never the app's error: `open`
// waits the swap out and loads again.
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
function assert(cond, message) { if (!cond) throw new Failure(message); }
const pause = (ms) => new Promise((r) => setTimeout(r, ms));

function helpers(page, base) {
  const t = {
    page,
    base,

    // Load the guide until the app is live — once the examples' buttons carry the listeners the
    // wasm attached. A load the dev server's build swap refused is waited out and loaded again;
    // any other failure to start fails at once.
    async open(viewport = DESKTOP) {
      await page.setViewport(viewport);
      for (let attempt = 1; ; attempt++) {
        let refused;
        const swapped = new Promise((resolve) => { refused = (text) => { if (DEV_SERVER_SWAP.test(text)) resolve(text); }; });
        const onConsole = (m) => refused(m.text());
        const onError = (e) => refused(String(e.message || e));
        page.on('console', onConsole);
        page.on('pageerror', onError);
        try {
          await page.goto(`${base}/guide`, { waitUntil: 'load', timeout: 300000 });
          const outcome = await Promise.race([
            page.waitForFunction(() => document.querySelector('.guide-code-block button[data-dioxus-id]'), { timeout: 300000 }).then(() => null),
            swapped,
          ]);
          if (outcome === null) break;
          if (attempt === 3) throw new Failure(`the dev server refused the app's loader script ${attempt} times: ${outcome.slice(0, 160)}`);
        } finally {
          page.off('console', onConsole);
          page.off('pageerror', onError);
        }
        await wholeBuild(base);
      }
      await t.frames(6);
    },

    // Two animation frames: long enough for any per-frame repaint to land.
    async frames(n = 2) {
      for (let i = 0; i < n; i++) await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
    },

    // An element by its anchor, scrolled to the middle of the window.
    async anchored(id) {
      const el = await page.$(`[id="${id}"]`);
      assert(el, `the guide has no #${id}`);
      await el.evaluate((e) => e.scrollIntoView({ block: 'center' }));
      await t.frames(3);
      return el;
    },

    // The highlighted word `text` inside `host`, waiting a moment for the editor to paint it.
    async word(host, text, what) {
      const handle = await page.waitForFunction(
        (h, w) => [...h.querySelectorAll('.code-editor-highlight span')].find((s) => s.textContent === w),
        { timeout: 5000 }, host, text,
      ).catch(() => null);
      const el = handle && handle.asElement();
      assert(el, `${what} shows no highlighted "${text}"`);
      return el;
    },

    async rest(el) {
      const box = await el.boundingBox();
      assert(box, 'the word is not on screen');
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 4 });
    },

    // The hover on screen, as its text, or null once it has waited `ms` for one.
    async hint(ms = 3000) {
      const el = await page.waitForSelector('.hover-hint', { timeout: ms }).catch(() => null);
      return el ? el.evaluate((h) => h.innerText) : null;
    },

    async hintGone(ms = 1500) {
      return page.waitForFunction(() => !document.querySelector('.hover-hint'), { timeout: ms }).then(() => true, () => false);
    },

    // A key pressed with the platform's command modifier held.
    async chord(key) {
      const modifier = process.platform === 'darwin' ? 'Meta' : 'Control';
      await page.keyboard.down(modifier);
      await page.keyboard.press(key);
      await page.keyboard.up(modifier);
    },

    // Replace an editor's whole text, as a person would: select all, then type.
    async retype(host, text) {
      const editor = await host.$('textarea');
      assert(editor, 'the editor has no text area to type in');
      await editor.click();
      await t.chord('KeyA');
      await page.keyboard.press('Backspace');
      await page.keyboard.type(text, { delay: 1 });
    },

    async button(host, label) {
      const handle = await page.waitForFunction(
        (h, l) => [...h.querySelectorAll('button')].find((b) => b.innerText.trim() === l),
        { timeout: 5000 }, host, label,
      ).catch(() => null);
      const el = handle && handle.asElement();
      assert(el, `there is no "${label}" button`);
      return el;
    },

    // How each shared editor on the page sits over its code: the text area and the highlight
    // layer drawn in one box, neither scrolling, and no empty line left under the last one.
    async editorFit() {
      return page.evaluate(() => {
        const px = (v) => parseFloat(v) || 0;
        return [...document.querySelectorAll('main .code-editor')].map((editor) => {
          const host = editor.closest('[id]');
          const ta = editor.querySelector('.code-editor-textarea');
          const hl = editor.querySelector('.code-editor-highlight');
          const a = ta.getBoundingClientRect();
          const b = hl.getBoundingClientRect();
          const s = getComputedStyle(hl);
          const range = document.createRange();
          range.selectNodeContents(hl);
          const lines = [...range.getClientRects()].filter((r) => r.height > 0);
          const textBottom = lines.length ? Math.max(...lines.map((r) => r.bottom)) : b.top + px(s.paddingTop);
          return {
            where: host ? host.id : '(no anchor)',
            sameBox: Math.abs(a.top - b.top) < 0.5 && Math.abs(a.left - b.left) < 0.5 && Math.abs(a.width - b.width) < 0.5 && Math.abs(a.height - b.height) < 0.5,
            boxes: `${a.width.toFixed(1)}×${a.height.toFixed(1)} vs ${b.width.toFixed(1)}×${b.height.toFixed(1)}`,
            textareaScrolls: ta.scrollHeight > ta.clientHeight + 1,
            highlightScrolls: hl.scrollHeight > hl.clientHeight + 1,
            trailingNewline: ta.value.endsWith('\n'),
            deadSpace: b.bottom - px(s.paddingBottom) - px(s.borderBottomWidth) - textBottom,
            lineHeight: px(s.lineHeight),
          };
        });
      });
    },
  };
  return t;
}

// The blocks rendered for the guide's examples: the ones with an example header.
async function exampleBlocks(t) {
  return t.page.$$eval('.guide-code-block', (blocks) => blocks
    .filter((b) => b.querySelector('.guide-code-header'))
    .map((b) => ({ id: b.id, hasEditor: !!b.querySelector('.code-editor') })));
}

function assertFits(fits, context) {
  const wrong = [];
  for (const f of fits) {
    if (!f.sameBox) wrong.push(`${f.where}: the text area and the highlight are drawn in different boxes (${f.boxes})`);
    if (f.textareaScrolls) wrong.push(`${f.where}: the text area scrolls inside the editor`);
    if (f.highlightScrolls) wrong.push(`${f.where}: the highlight layer scrolls inside the editor`);
    const allowed = f.trailingNewline ? f.lineHeight : 0;
    if (f.deadSpace > allowed + f.lineHeight / 2) wrong.push(`${f.where}: ${f.deadSpace.toFixed(1)}px of empty editor under the code`);
  }
  assert(wrong.length === 0, `${context}: ${wrong.length} editor(s) do not fit their code:\n${wrong.slice(0, 25).join('\n')}`);
}

// ---------------------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------------------

const scenarios = [];
// opts.allowErrors: page errors the scenario expects. opts.engines: the browsers able to show
// what it checks — elsewhere it is reported as skipped, never as passed.
function scenario(name, fn, opts = {}) { scenarios.push({ name, fn, opts }); }

scenario('every_example_is_edited_in_the_shared_editor', async (t) => {
  await t.open();
  const blocks = await exampleBlocks(t);
  assert(blocks.length > 100, `the guide shows only ${blocks.length} examples`);
  const plain = blocks.filter((b) => !b.hasEditor).map((b) => b.id);
  assert(plain.length === 0, `${plain.length} of ${blocks.length} examples are a plain text box, not the shared editor: ${plain.slice(0, 12).join(', ')}`);
});

scenario('hovering_a_keyword_in_an_example_shows_the_language_servers_hint', async (t) => {
  await t.open();
  const example = await t.anchored('reading-twins');
  await t.rest(await t.word(example, 'Let', 'the "English and Symbols Are Twins" example'));
  const text = await t.hint();
  assert(text && /variable/i.test(text), `resting on "Let" shows ${JSON.stringify(text)}, not the language server's hover`);
  const placed = await t.page.$eval('.hover-hint', (h) => {
    const r = h.getBoundingClientRect();
    return { fixed: getComputedStyle(h).position === 'fixed', inside: r.left >= 0 && r.top >= 0 && r.right <= innerWidth && r.bottom <= innerHeight };
  });
  assert(placed.fixed && placed.inside, `the hint is not a fixed overlay inside the window: ${JSON.stringify(placed)}`);
  // Out to the empty margin right of the example, level with the word: off the code, off the hint.
  const box = await example.boundingBox();
  const word = await (await t.word(example, 'Let', 'the "English and Symbols Are Twins" example')).boundingBox();
  await t.page.mouse.move(box.x + box.width + 40, word.y + word.height / 2, { steps: 4 });
  assert(await t.hintGone(), 'the hint stays up after the pointer leaves the code');
});

scenario('the_hint_stays_while_the_pointer_moves_into_it', async (t) => {
  await t.open();
  const example = await t.anchored('for-each');
  await t.rest(await t.word(example, 'Repeat', 'the "For-Each Loop" example'));
  const hint = await t.page.waitForSelector('.hover-hint', { timeout: 3000 }).catch(() => null);
  assert(hint, 'resting on "Repeat" shows no hint');
  const hb = await hint.boundingBox();
  await t.page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2, { steps: 8 });
  await pause(600);
  assert(await t.page.$('.hover-hint'), 'the hint vanished while the pointer moved into it');
  const link = await t.page.$eval('.hover-hint', (h) => { const a = h.querySelector('a'); return a && { href: a.href, text: a.textContent }; });
  assert(link && /^https:/.test(link.href), `the hint's link is not a link: ${JSON.stringify(link)}`);
  await t.page.mouse.move(5, 5, { steps: 6 });
  assert(await t.hintGone(), 'the hint stays up after the pointer leaves it');
});

// The page scrolls under a resting pointer; the hint belongs to a word that has moved away.
scenario('the_hint_leaves_when_the_page_scrolls', async (t) => {
  await t.open();
  const example = await t.anchored('reading-twins');
  await t.rest(await t.word(example, 'Let', 'the "English and Symbols Are Twins" example'));
  assert(await t.hint(), 'resting on "Let" shows no hint');
  await t.page.mouse.wheel({ deltaY: 400 });
  await pause(200);
  assert(await t.hintGone(), 'the hint stays on screen after the page scrolled its word away');
});

scenario('keywords_colour_at_any_case', async (t) => {
  await t.open();
  const example = await t.anchored('reading-guess');
  const what = 'the "The Phrasing You Would Guess" example';
  const colour = (el) => el.evaluate((e) => ({ cls: e.className, color: getComputedStyle(e).color }));
  const name = await colour(await t.word(example, 'scores', what));
  for (const word of ['Let', 'If', 'Show']) {
    const it = await colour(await t.word(example, word, what));
    assert(/\btok-keyword\b/.test(it.cls), `"${word}" is not read as a keyword (${it.cls})`);
    assert(it.color !== name.color, `"${word}" is painted the colour of a name (${it.color})`);
  }
});

// VS Code's Problems in the example: the language server's diagnostics as a squiggle under the
// words they name, explained by the hover — and Reset brings the example back, clean.
scenario('a_mistake_in_an_example_gets_a_squiggle_and_reset_clears_it', async (t) => {
  await t.open();
  const example = await t.anchored('first-program');
  await t.retype(example, MISTAKE);
  const squiggle = await t.page.waitForFunction((h) => h.querySelector('.code-editor-highlight .sq-error'), { timeout: 5000 }, example).then((h) => h.asElement(), () => null);
  assert(squiggle, 'a Text returned from an Int function draws no error squiggle');
  const look = await squiggle.evaluate((el) => {
    const cs = getComputedStyle(el);
    return { style: cs.textDecorationStyle, line: cs.textDecorationLine };
  });
  assert(look.line.includes('underline') && look.style === 'wavy', `the error is not a wavy underline: ${JSON.stringify(look)}`);
  await t.rest(squiggle);
  const problem = await t.page.waitForSelector('.hover-hint .hint-problem', { timeout: 3000 }).catch(() => null);
  assert(problem, 'resting on the squiggle shows no problem in the hover');
  const text = await problem.evaluate((h) => h.closest('.hover-hint').innerText);
  assert(/Int/.test(text) && /Text/.test(text), `the hover does not explain the mismatch: ${JSON.stringify(text.slice(0, 200))}`);
  assert(/logicaffeine/.test(text), `the hover does not say where the problem comes from: ${JSON.stringify(text.slice(0, 200))}`);
  await t.page.mouse.move(5, 5);
  await (await t.button(example, 'Reset')).click();
  const cleared = await t.page.waitForFunction((h) => !h.querySelector('.code-editor-highlight .sq-error') && h.querySelector('textarea').value.startsWith('## Main\nLet name be "Alice".'), { timeout: 5000 }, example).then(() => true, () => false);
  assert(cleared, 'Reset does not bring the example back without its squiggle');
});

// Every example the guide presents as working code opens without an error or warning squiggle;
// the ones that fail on purpose say so in their output panel.
scenario('examples_open_without_error_or_warning_squiggles', async (t) => {
  await t.open();
  const blocks = await exampleBlocks(t);
  const plain = blocks.filter((b) => !b.hasEditor).map((b) => b.id);
  assert(plain.length === 0, `${plain.length} examples have no shared editor to draw a squiggle in (${plain.slice(0, 8).join(', ')})`);
  for (const { id } of blocks) await t.anchored(id);
  await pause(2500);
  // The language server did read the examples: "Program Structure" builds a Point it never uses.
  const unused = await t.page.$eval('[id="program-structure"]', (b) =>
    [...b.querySelectorAll('.code-editor-highlight .sq-unnecessary')].map((s) => s.textContent));
  assert(unused.includes('p'), `the unused "p" in "Program Structure" is not faded (${JSON.stringify(unused)}) — the examples were never analysed`);
  const loud = await t.page.$$eval('.guide-code-block', (blocks) => blocks
    .filter((b) => b.querySelector('.guide-code-header'))
    .filter((b) => !/Expected error|What error do you expect\?/.test(b.querySelector('.guide-code-output-header')?.textContent || ''))
    .flatMap((b) => [...b.querySelectorAll('.code-editor-highlight .sq-error, .code-editor-highlight .sq-warning')].map((s) => `${b.id}: "${s.textContent}" (${s.className})`)));
  assert(loud.length === 0, `${loud.length} squiggle(s) under examples presented as working code:\n${loud.slice(0, 25).join('\n')}`);
});

scenario('every_editor_fits_its_code', async (t) => {
  await t.open();
  const plain = (await exampleBlocks(t)).filter((b) => !b.hasEditor);
  assert(plain.length === 0, `${plain.length} examples have no shared editor to fit (${plain.slice(0, 8).map((b) => b.id).join(', ')})`);
  assertFits(await t.editorFit(), 'on a desktop');
});

scenario('every_editor_fits_its_code_on_a_phone', async (t) => {
  await t.open(PHONE);
  const plain = (await exampleBlocks(t)).filter((b) => !b.hasEditor);
  assert(plain.length === 0, `${plain.length} examples have no shared editor to fit (${plain.slice(0, 8).map((b) => b.id).join(', ')})`);
  assertFits(await t.editorFit(), 'on a phone');
});

// On a phone an example's header keeps every control inside the card, and a title that fits
// the header's width is never broken to make room for the badge or the controls beside it
// ("An / Expression / Closure").
scenario('every_example_header_fits_a_phone', async (t) => {
  await t.open(PHONE);
  const r = await t.page.evaluate(() => {
    const clipped = [];
    const broken = [];
    const blocks = [...document.querySelectorAll('.guide-code-block')];
    for (const block of blocks) {
      const card = block.getBoundingClientRect();
      const header = block.querySelector('.guide-code-header');
      const title = block.querySelector('.guide-code-title');
      if (!header || !title) continue;
      for (const control of header.querySelectorAll('.guide-code-actions > *')) {
        const b = control.getBoundingClientRect();
        if (b.right > card.right + 1 || b.left < card.left - 1) clipped.push(`${block.id}: ${control.innerText.trim()}`);
      }
      const hs = getComputedStyle(header);
      const room = header.clientWidth - parseFloat(hs.paddingLeft) - parseFloat(hs.paddingRight);
      const probe = title.cloneNode(true);
      Object.assign(probe.style, { position: 'absolute', visibility: 'hidden', whiteSpace: 'nowrap' });
      document.body.appendChild(probe);
      const oneLine = probe.getBoundingClientRect().width;
      probe.remove();
      const ts = getComputedStyle(title);
      const lineHeight = parseFloat(ts.lineHeight) || parseFloat(ts.fontSize) * 1.4;
      if (oneLine <= room && title.getBoundingClientRect().height > lineHeight * 1.5) {
        broken.push(`${block.id}: ${title.innerText.replace(/\n/g, ' / ')}`);
      }
    }
    return { clipped, broken, count: blocks.length };
  });
  assert(r.count > 0, 'the guide shows no examples');
  assert(r.clipped.length === 0, `${r.clipped.length} header controls are cut off at the card's edge: ${r.clipped.slice(0, 8).join('; ')}`);
  assert(r.broken.length === 0, `${r.broken.length} titles that fit a row are broken to make room beside them: ${r.broken.slice(0, 8).join('; ')}`);
});

// A new line at the very end is a line the reader can see at once: the editor grows to hold it,
// the text never scrolls under the highlight, and the words stay under the caret.
scenario('typing_grows_the_editor_without_scrolling_it', async (t) => {
  await t.open();
  const example = await t.anchored('hello-world');
  const ta = await example.$('.code-editor-textarea');
  assert(ta, 'the "Hello World" example has no shared editor');
  const height = () => ta.evaluate((e) => e.getBoundingClientRect().height);
  const before = await height();
  const lineHeight = await ta.evaluate((e) => parseFloat(getComputedStyle(e).lineHeight));
  await ta.click();
  await t.chord('End');
  await t.page.keyboard.press('Enter');
  await t.frames(4);
  const grown = await height();
  assert(Math.abs(grown - before - lineHeight) < 1.5, `a new last line grew the editor by ${(grown - before).toFixed(1)}px, not one ${lineHeight}px line`);
  await t.page.keyboard.type('Show "again".', { delay: 1 });
  await t.page.keyboard.press('Enter');
  await t.page.keyboard.type('Show "and again".', { delay: 1 });
  await t.frames(4);
  const grownTwice = await height();
  assert(Math.abs(grownTwice - before - 2 * lineHeight) < 1.5, `two new lines grew the editor by ${(grownTwice - before).toFixed(1)}px, not two ${lineHeight}px lines`);
  assertFits((await t.editorFit()).filter((f) => f.where === 'hello-world'), 'after typing');
  const scrolled = await ta.evaluate((e) => e.scrollTop);
  assert(scrolled === 0, `the text area scrolled ${scrolled}px under its highlight`);
});

scenario('an_exercise_speaks_with_the_language_server', async (t) => {
  await t.open();
  const exercise = await t.anchored('ex-greet-logos');
  const show = await t.word(exercise, 'Show', 'the "Change the greeting" exercise');
  assert(/\btok-keyword\b/.test(await show.evaluate((e) => e.className)), '"Show" in the exercise is not read as a keyword');
  await t.rest(show);
  const text = await t.hint();
  assert(text && /Show/.test(text), `resting on "Show" in the exercise shows ${JSON.stringify(text)}`);
  await t.retype(exercise, MISTAKE);
  const squiggle = await t.page.waitForFunction((h) => h.querySelector('.code-editor-highlight .sq-error'), { timeout: 5000 }, exercise).then(() => true, () => false);
  assert(squiggle, 'a mistake typed into the exercise draws no error squiggle');
});

// The code quoted in a section's prose is read-only, but it is still LOGOS: coloured the same,
// and explained by the same hover.
scenario('a_prose_snippet_speaks_with_the_language_server', async (t) => {
  await t.open();
  const section = await t.anchored('building-web-apps');
  const snippet = await section.$('.guide-snippet');
  assert(snippet, 'the web-apps section quotes its counter as plain text, not in the shared editor');
  await snippet.evaluate((e) => e.scrollIntoView({ block: 'center' }));
  await t.frames(3);
  const word = await t.word(snippet, 'Let', 'the counter snippet');
  assert(/\btok-keyword\b/.test(await word.evaluate((e) => e.className)), '"Let" in the snippet is not read as a keyword');
  await t.rest(word);
  const text = await t.hint();
  assert(text && /variable/i.test(text), `resting on "Let" in the snippet shows ${JSON.stringify(text)}`);
  const ta = await snippet.$('textarea');
  const before = await ta.evaluate((e) => e.value);
  await ta.click();
  await t.page.keyboard.type('xyz');
  assert((await ta.evaluate((e) => e.value)) === before, 'typing changes a snippet the guide quotes');
  await t.button(snippet, 'Copy');
});

// Copy puts the snippet's code, exactly as quoted, on the clipboard, and says so on the button.
scenario('a_prose_snippet_copies_its_code', async (t) => {
  await t.open();
  await t.page.browserContext().overridePermissions(t.base, ['clipboard-read', 'clipboard-write', 'clipboard-sanitized-write']);
  const section = await t.anchored('building-web-apps');
  const snippet = await section.$('.guide-snippet');
  assert(snippet, 'the web-apps section quotes no snippet');
  await snippet.evaluate((e) => e.scrollIntoView({ block: 'center' }));
  const quoted = await snippet.$eval('textarea', (e) => e.value);
  await (await t.button(snippet, 'Copy')).click();
  const said = await t.page.waitForFunction((s) => [...s.querySelectorAll('button')].some((b) => b.innerText.trim() === 'Copied!'), { timeout: 3000 }, snippet).then(() => true, () => false);
  assert(said, 'the Copy button does not say the code was copied');
  const copied = await t.page.evaluate(() => navigator.clipboard.readText());
  assert(copied === quoted, `the clipboard holds ${JSON.stringify(copied.slice(0, 80))}, not the snippet's code`);
  assert(quoted.startsWith('## Component Counter'), `the snippet quotes ${JSON.stringify(quoted.slice(0, 40))}`);
}, { engines: ['chrome'] });

scenario('a_theorem_snippet_reads_as_logic', async (t) => {
  await t.open();
  const section = await t.anchored('proofs');
  const snippet = await section.$('.guide-snippet');
  assert(snippet, 'the proof-engine section quotes its theorem as plain text, not in the shared editor');
  await snippet.evaluate((e) => e.scrollIntoView({ block: 'center' }));
  await t.frames(3);
  for (const word of ['Theorem', 'Given', 'Prove', 'Proof']) {
    const el = await t.word(snippet, word, 'the Socrates theorem snippet');
    assert(/\btok-keyword\b/.test(await el.evaluate((e) => e.className)), `"${word}" in the theorem is not read as a keyword`);
  }
  await t.rest(await t.word(snippet, 'Theorem', 'the Socrates theorem snippet'));
  const text = await t.hint();
  assert(text && /proof strategy/i.test(text), `the Theorem hint is not the language server's lesson: ${JSON.stringify(text && text.slice(0, 200))}`);
});

scenario('a_logic_example_reads_as_logic', async (t) => {
  await t.open();
  const example = await t.anchored('logic-universal');
  const all = await t.word(example, 'All', 'the "All birds fly." example');
  assert(/\btok-builtin\b/.test(await all.evaluate((e) => e.className)), '"All" is not marked as a quantifier');
});

// Callouts with a coloured left edge: square on that edge, 4px on the other.
scenario('every_left_accent_is_square_on_the_left', async (t) => {
  await t.open();
  const accents = await t.page.evaluate(() => {
    const alpha = (c) => {
      const m = c.match(/rgba?\(([^)]*)\)/);
      if (!m) return c === 'transparent' ? 0 : 1;
      const parts = m[1].split(/[\s,/]+/).filter(Boolean);
      return parts.length > 3 ? parseFloat(parts[3]) : 1;
    };
    const px = (v) => parseFloat(v) || 0;
    const out = [];
    for (const el of document.querySelectorAll('body *')) {
      const s = getComputedStyle(el);
      if (s.display === 'none' || s.visibility === 'hidden') continue;
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      const left = px(s.borderLeftWidth);
      if (s.borderLeftStyle === 'none' || left < 2 || alpha(s.borderLeftColor) === 0) continue;
      const painted = alpha(s.backgroundColor) > 0 || s.backgroundImage !== 'none'
        || ['Top', 'Right', 'Bottom'].some((side) => px(s[`border${side}Width`]) > 0 && s[`border${side}Style`] !== 'none');
      const distinct = left > px(s.borderRightWidth) || s.borderLeftColor !== s.borderRightColor;
      if (!painted || !distinct) continue;
      out.push({
        what: `${el.tagName.toLowerCase()}${el.id ? `#${el.id}` : ''}.${[...el.classList].join('.')}`,
        tl: s.borderTopLeftRadius, bl: s.borderBottomLeftRadius, tr: s.borderTopRightRadius, br: s.borderBottomRightRadius,
        el,
      });
    }
    return out.map(({ el, ...rest }) => rest);
  });
  const kinds = await t.page.evaluate((list) => list.map(([name, sel]) => [name, sel, !!document.querySelector(sel)]), ACCENT_KINDS);
  for (const [name, sel, present] of kinds) {
    assert(present, `the guide shows no ${name} (${sel}) — the scan must meet every kind of callout`);
    const met = accents.some((a) => sel.split(/(?=[.#])/).filter((p) => p.startsWith('.')).every((p) => a.what.includes(p)));
    assert(met, `the scan found no ${name} (${sel}) among the left accents: ${accents.map((a) => a.what).slice(0, 10).join(', ')}`);
  }
  const wrong = accents.filter((a) => a.tl !== ACCENT_RADII.tl || a.bl !== ACCENT_RADII.bl || a.tr !== ACCENT_RADII.tr || a.br !== ACCENT_RADII.br);
  assert(wrong.length === 0, `${wrong.length} of ${accents.length} left accents are not square-left, 4px-right:\n${wrong.slice(0, 20).map((a) => `${a.what}: ${a.tl} ${a.tr} ${a.br} ${a.bl}`).join('\n')}`);
});

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
    console.error('guide-e2e: give --url <running site> or --public <built site with index.html>');
    return 2;
  }
  const selected = opts.only ? scenarios.filter((s) => opts.only.includes(s.name)) : scenarios;
  const unknown = (opts.only || []).filter((n) => !scenarios.some((s) => s.name === n));
  if (unknown.length) { console.error(`guide-e2e: unknown scenario(s): ${unknown.join(', ')}`); return 2; }

  const puppeteer = require('puppeteer-core');
  fs.mkdirSync(opts.shots, { recursive: true });
  const server = opts.url ? { url: opts.url, close() {} } : await serve(path.resolve(opts.public));
  const engine = opts.firefox ? 'firefox' : 'chrome';
  // A cold first load after a rebuild can outlast puppeteer's default 180 s protocol ceiling;
  // `open` already allows 300 s, so the ceiling must sit above every wait we ask for.
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
    const started = Date.now();
    const context = await browser.createBrowserContext();
    const page = await context.newPage();
    const errors = [];
    // What the dev server did rather than the app — a refused build swap, a failed request — only
    // explains a failure; on a passing page it is not one.
    const network = [];
    const heard = (text, as) => (DEV_SERVER_SWAP.test(text) ? network : errors).push(`${as}: ${text.slice(0, 300)}`);
    page.on('pageerror', (e) => heard(String(e.message || e), 'pageerror'));
    page.on('console', (m) => { if (m.type() === 'error' && !DEV_CLIENT_NOISE.test(m.text())) heard(m.text(), 'console.error'); });
    page.on('requestfailed', (r) => network.push(`requestfailed: ${r.url().slice(0, 200)} (${r.failure()?.errorText})`));
    let failure = null;
    try {
      await Promise.race([
        s.fn(helpers(page, server.url)),
        new Promise((_, reject) => setTimeout(() => reject(new Failure('timed out after 600s')), 600000)),
      ]);
      const unexpected = errors.filter((e) => !(s.opts.allowErrors && s.opts.allowErrors.test(e)));
      if (unexpected.length) throw new Failure(`page errors:\n${unexpected.join('\n')}`);
    } catch (e) {
      failure = e instanceof Failure ? e.message : `harness error: ${e.stack || e}`;
      // What the page said on the way down: a page that never rendered explains itself here.
      const said = [...errors, ...network].filter((err) => !failure.includes(err));
      if (said.length) failure += `\npage errors:\n${said.join('\n')}`;
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
    const cases = results.map((r) => `    <testcase classname="guide-e2e.${engine}" name="${xmlEscape(r.name)}" time="${r.seconds.toFixed(3)}">` +
      (r.failure ? `\n      <failure message="${xmlEscape(r.failure.split('\n')[0])}">${xmlEscape(r.failure)}</failure>\n    ` : '') +
      (r.skipped ? `<skipped message="${xmlEscape(r.skipped)}"/>` : '') + '</testcase>');
    fs.writeFileSync(opts.junit, `<?xml version="1.0" encoding="UTF-8"?>\n<testsuites name="guide-e2e" tests="${results.length}" failures="${failed}" skipped="${skipped}">\n  <testsuite name="guide-e2e" tests="${results.length}" failures="${failed}" skipped="${skipped}">\n${cases.join('\n')}\n  </testsuite>\n</testsuites>\n`);
  }
  console.log(`guide-e2e (${engine}): ${ran - failed}/${ran} passed${skipped ? `, ${skipped} skipped` : ''}${failed ? ` — screenshots of each failure in ${opts.shots}` : ''}`);
  return failed ? 1 : 0;
}

main().then((code) => process.exit(code), (e) => { console.error(e); process.exit(2); });
