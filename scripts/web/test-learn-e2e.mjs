// Learn Logic in a real browser: the interaction layer of the Learn page's tests.
//
// The rules (the journey, the question, practice, the symbol buttons) are pinned by Rust unit tests,
// the rendered structure by the SSR tests in crates/logicaffeine_tests/tests/learn_page.rs, and the
// content by learn_lessons.rs. What only a browser can show lives here: clicks, keys, the caret,
// focus, scrolling, the network, storage, a phone's width, and what a learner can or cannot see at
// each moment.
//
// Every scenario runs in a fresh browser context (clean localStorage), fails on any uncaught page
// error or console error, and leaves a screenshot of the failing moment. Scenarios drive the page
// through what a learner sees (button labels, prompts, ARIA state), never through internal hooks, so
// the same scenario can prove a bug on the old page (RED) and its absence on the fixed one (GREEN).
// Answers come from the curriculum JSON, read the way the app reads it.
//
// Usage: node scripts/web/test-learn-e2e.mjs --public <built-public-dir> (--chrome <path> | --firefox <path>)
//          [--junit <report.xml>] [--only name,name] [--shots <dir>] [--list]
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { createRequire } from 'node:module';

const ROOT = path.resolve(import.meta.dirname, '../..');
const CURRICULUM = path.join(ROOT, 'apps/logicaffeine_web/assets/curriculum');
const require = createRequire(import.meta.url);

// ---------------------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------------------

function parseArgs(argv) {
  const out = { only: null, junit: null, shots: '/tmp/learn-e2e-shots', list: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--public') out.public = argv[++i];
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
// The curriculum, read the way the app reads it
// ---------------------------------------------------------------------------------------

function naturalCompare(a, b) {
  const re = /(\d+)|(\D+)/g;
  const pa = a.match(re) || [], pb = b.match(re) || [];
  for (let i = 0; i < Math.min(pa.length, pb.length); i++) {
    const x = pa[i], y = pb[i];
    const nx = /^\d/.test(x), ny = /^\d/.test(y);
    if (nx && ny) {
      const d = BigInt(x) - BigInt(y);
      if (d !== 0n) return d < 0n ? -1 : 1;
    } else if (x !== y) return x < y ? -1 : 1;
  }
  return pa.length - pb.length || (a < b ? -1 : a > b ? 1 : 0);
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

function loadCurriculum() {
  const eras = [];
  for (const eraDir of fs.readdirSync(CURRICULUM).sort()) {
    const eraPath = path.join(CURRICULUM, eraDir);
    if (!fs.statSync(eraPath).isDirectory()) continue;
    const era = { ...readJson(path.join(eraPath, 'meta.json')), modules: [] };
    for (const modDir of fs.readdirSync(eraPath).sort()) {
      const modPath = path.join(eraPath, modDir);
      if (!fs.statSync(modPath).isDirectory()) continue;
      const files = fs.readdirSync(modPath);
      const exercises = files
        .filter((f) => f.startsWith('ex_') && f.endsWith('.json'))
        .map((f) => readJson(path.join(modPath, f)))
        .sort((a, b) => naturalCompare(a.id, b.id));
      const sections = files
        .filter((f) => f.startsWith('sec_') && f.endsWith('.json'))
        .map((f) => readJson(path.join(modPath, f)))
        .sort((a, b) => a.order - b.order);
      era.modules.push({ ...readJson(path.join(modPath, 'meta.json')), exercises, sections });
    }
    era.modules.sort((a, b) => a.order - b.order);
    eras.push(era);
  }
  return eras.sort((a, b) => a.order - b.order);
}

const curriculum = loadCurriculum();
const modules = curriculum.flatMap((era) => era.modules);
const lessons = modules.flatMap((m) => m.sections.map((s) => ({ ...s, module: m })));

function lesson(id) {
  const l = lessons.find((x) => x.id === id);
  if (!l) throw new Error(`no lesson ${id} in the curriculum`);
  return l;
}

function module(id) {
  const m = modules.find((x) => x.id === id);
  if (!m) throw new Error(`no module ${id} in the curriculum`);
  return m;
}

/** The lesson's practice exercises, in the order practice walks them. */
function practiceOf(id) {
  return lesson(id).module.exercises.filter((e) => e.section === id);
}

/** The first lesson whose question is typed, or a choice, as asked. */
function firstLesson(kind) {
  const l = lessons.find((x) => x.question && x.question.type === kind);
  if (!l) throw new Error(`no lesson with a ${kind} question`);
  return l;
}

/** The key as a learner types it: the modal strength LOGOS prints (`_{0.5}`) left out. */
function learnerForm(key) {
  return key.replace(/_\{[^}]*\}/g, '');
}

const PAD = ['∀', '∃', '¬', '∧', '∨', '→', '↔', '□', '◇', '=', '(', ')', ','];

// ---------------------------------------------------------------------------------------
// A static server with the production SPA fallback: a missing file is answered with index.html,
// exactly as the deployed site and `dx serve` answer it.
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

// A debug build carries dx's hot-reload client, which dials a devserver socket a plain static
// server cannot answer. That error belongs to the build's tooling, never to the page. Chrome reports
// the refusal as a console error; Firefox raises it as an uncaught error, worded exactly as
// DEV_CLIENT_REFUSED reads: a socket to /_dioxus on this harness's own server, and nothing else.
// Any other uncaught error still fails its scenario. `selfTest` holds both sides of that line.
const DEV_CLIENT_REFUSED = /^Firefox can[’']t establish a connection to the server at ws:\/\/127\.0\.0\.1:\d+\/_dioxus\?build_id=\d+\.$/;
const DEV_CLIENT_NOISE = /WebSocket connection to '[^']*\/_dioxus\?|_dioxus.*(WebSocket|websocket)/;
const isDevClientNoise = (text) => DEV_CLIENT_NOISE.test(text) || DEV_CLIENT_REFUSED.test(text);

// The noise filter's own check: the refusals it drops, and errors that look like them but must fail.
function selfTest() {
  const dropped = [
    'Firefox can’t establish a connection to the server at ws://127.0.0.1:40177/_dioxus?build_id=0.',
    "WebSocket connection to 'ws://127.0.0.1:38119/_dioxus?build_id=0' failed: Error in connection establishment",
  ];
  const kept = [
    'Firefox can’t establish a connection to the server at ws://127.0.0.1:40177/socket.',
    'Firefox can’t establish a connection to the server at ws://127.0.0.1:40177/relay.',
    "WebSocket connection to 'ws://127.0.0.1:38119/relay' failed: Error in connection establishment",
    'Firefox can’t establish a connection to the server at ws://example.com/_dioxus?build_id=0.',
    'Firefox can’t establish a connection to the server at wss://127.0.0.1:40177/_dioxus?build_id=0.',
    'TypeError: can’t access property "value", field is null',
    'ReferenceError: window is not defined',
  ];
  const wrong = [...dropped.filter((t) => !isDevClientNoise(t)).map((t) => `not dropped: ${t}`), ...kept.filter(isDevClientNoise).map((t) => `dropped: ${t}`)];
  if (wrong.length) throw new Error(`the dev-client noise filter is wrong:\n${wrong.join('\n')}`);
}
function assert(cond, message) { if (!cond) throw new Failure(message); }
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const PHONE = { width: 360, height: 740, isMobile: true, hasTouch: true, deviceScaleFactor: 2 };

function helpers(page, base) {
  const t = {
    page,

    async open({ path: route = '/learn', storage = {}, blockLexicon = false } = {}) {
      if (Object.keys(storage).length) {
        await page.evaluateOnNewDocument((entries) => {
          for (const [k, v] of Object.entries(entries)) localStorage.setItem(k, v);
        }, storage);
      }
      if (blockLexicon) {
        await page.setRequestInterception(true);
        page.on('request', (req) => {
          if (req.isInterceptResolutionHandled?.()) return;
          if (new URL(req.url()).pathname === '/data/lexicon.json') {
            req.respond({ status: 404, contentType: 'text/plain', body: 'not found' });
          } else req.continue();
        });
      }
      await page.goto(base + route, { waitUntil: 'domcontentloaded', timeout: 60000 });
      await t.live();
    },

    /** Wait for the LIVE app, then for its roadmap. A prerendered (SSG) page carries a static copy in
     *  #main, markup with no handlers, while the wasm app mounts into #app. The shell's watchdog
     *  removes #main once the live app has held for a moment (index.html), so "#app holds the page and
     *  no #main" is the app, and only the app. */
    async live() {
      try {
        await page.waitForFunction(() => (document.getElementById('app')?.innerText || '').trim().length > 200 && !document.getElementById('main'), { timeout: 60000 });
      } catch (e) {
        if (e.name !== 'TimeoutError') throw e;
        const s = await page.evaluate(() => {
          const app = document.getElementById('app'), main = document.getElementById('main');
          return { text: (app?.innerText || '').trim().length, copy: !main ? 'removed' : getComputedStyle(main).display === 'none' ? 'hidden' : 'shown' };
        });
        throw new Failure(`the live app never came up within 60s: #app holds ${s.text} chars; the prerendered copy is ${s.copy}`);
      }
      try {
        await page.waitForFunction((n) => document.querySelectorAll('#app .roadmap-lesson').length >= n, { timeout: 10000 }, lessons.length);
      } catch (e) {
        if (e.name !== 'TimeoutError') throw e;
        const n = await page.evaluate(() => document.querySelectorAll('#app .roadmap-lesson').length);
        throw new Failure(`the live page shows ${n} of the roadmap's ${lessons.length} lessons`);
      }
    },

    /** The lesson open on the page. */
    async openLessonId() {
      return page.evaluate(() => document.querySelector('#app article.lesson')?.getAttribute('data-lesson') ?? null);
    },

    async waitForLesson(id) {
      try {
        await page.waitForFunction((id) => document.querySelector('#app article.lesson')?.getAttribute('data-lesson') === id, { timeout: 15000 }, id);
      } catch {
        throw new Failure(`lesson ${id} never opened (the page shows ${await t.openLessonId()})`);
      }
    },

    /** Open a lesson by clicking it on the roadmap (opening its module first if it is folded). */
    async openLesson(id) {
      const ok = await page.evaluate((id) => {
        const b = document.querySelector(`#app .roadmap-lesson[data-lesson="${id}"]`);
        if (!b) return false;
        const details = b.closest('details');
        if (details && !details.open) details.open = true;
        b.click();
        return true;
      }, id);
      assert(ok, `the roadmap has no lesson ${id}`);
      await t.waitForLesson(id);
    },

    async text(selector = '#app') {
      return page.evaluate((s) => document.querySelector(s)?.innerText ?? '', selector);
    },

    async count(selector) {
      return page.evaluate((s) => document.querySelectorAll(s).length, selector);
    },

    async visible(selector) {
      return page.evaluate((s) => [...document.querySelectorAll(s)].some((e) => e.offsetParent !== null && !e.closest('[hidden]')), selector);
    },

    /** Click the first visible element matching `selector` whose text matches `pattern`, as a real
     *  pointer does (mousedown, mouseup, click). */
    async click(selector, pattern = null) {
      const handles = await page.$$(selector);
      for (const h of handles) {
        const ok = await h.evaluate((e, source, flags) => {
          if (e.offsetParent === null || e.closest('[hidden]')) return false;
          return !source || new RegExp(source, flags).test((e.innerText || e.textContent || '').trim());
        }, pattern?.source ?? null, pattern?.flags ?? '');
        if (ok) {
          await h.scrollIntoView();
          await h.click();
          await sleep(150);
          return;
        }
      }
      throw new Failure(`nothing visible matching ${selector}${pattern ? ` ${pattern}` : ''}`);
    },

    /** Pick an option of the lesson's choice question: the right one, or a wrong one. */
    async choose(correct) {
      const id = await t.openLessonId();
      const q = lesson(id).question;
      assert(q.type === 'choice', `lesson ${id} asks a ${q.type} question`);
      const text = correct ? q.options[q.correct] : q.options.find((_, i) => i !== q.correct);
      const handles = await page.$$('#app .lesson-options button');
      for (const h of handles) {
        if ((await h.evaluate((e) => e.innerText.trim())) === text.trim()) {
          await h.click();
          await sleep(150);
          return text;
        }
      }
      throw new Failure(`no option button reading ${JSON.stringify(text)}`);
    },

    /** Write `answer` into the typed field: every pad symbol by its button, everything else by key. */
    async compose(answer, field = '#lesson-answer') {
      await page.click(field);
      for (const ch of answer) {
        if (PAD.includes(ch)) {
          await page.click(`#app .symbol-key[data-symbol="${ch}"]`);
        } else {
          await page.keyboard.type(ch);
        }
      }
      await sleep(100);
    },

    async fieldValue(field = '#lesson-answer') {
      return page.$eval(field, (e) => e.value);
    },

    /** The verdict line under the question, or null before an answer. */
    async verdict() {
      return page.evaluate(() => {
        const v = document.querySelector('#app .lesson-question .lesson-verdict');
        return v ? { right: v.classList.contains('correct'), text: v.innerText.trim() } : null;
      });
    },

    async next() {
      await t.click('#app .lesson-next');
    },
  };
  return t;
}

// ---------------------------------------------------------------------------------------
// Scenarios. Each names the ledger entry (work/learn-page-bug-squash.md) it locks.
// ---------------------------------------------------------------------------------------

const scenarios = [];
function scenario(name, ledger, fn, opts = {}) { scenarios.push({ name, ledger, fn, opts }); }

scenario('word_bank_is_served_as_json', 'B1', async (t) => {
  await t.open();
  const r = await t.page.evaluate(async () => {
    const res = await fetch('/data/lexicon.json');
    const text = await res.text();
    let json = false;
    try { JSON.parse(text); json = true; } catch {}
    return { status: res.status, type: res.headers.get('content-type') || '', json, head: text.slice(0, 30) };
  });
  assert(r.json, `/data/lexicon.json is not JSON (HTTP ${r.status}, ${r.type}, begins ${JSON.stringify(r.head)}): the build shipped without the /data bundle`);
});

scenario('roadmap_lists_every_lesson_in_order', 'R1', async (t) => {
  await t.open();
  const shown = await t.page.$$eval('#app .roadmap-lesson', (els) => els.map((e) => e.getAttribute('data-lesson')));
  const expected = lessons.map((l) => l.id);
  assert(JSON.stringify(shown) === JSON.stringify(expected), `the roadmap lists ${shown.length} lessons ${shown.slice(0, 6).join(', ')}…; the curriculum has ${expected.length}`);
  const text = await t.text('#app .learn-roadmap');
  for (const m of modules) assert(text.includes(m.title), `the roadmap does not name the module ${m.title}`);
  const count = await t.text('#app .learn-progress');
  assert(new RegExp(`\\b0 of ${lessons.length} lessons done`).test(count), `a first visit reads ${JSON.stringify(count)}`);
});

scenario('a_first_visit_opens_the_first_lesson', 'R1', async (t) => {
  await t.open();
  const first = lessons[0].id;
  assert((await t.openLessonId()) === first, `a first visit opens ${await t.openLessonId()}, not ${first}`);
  const current = await t.page.$$eval('#app .roadmap-lesson[aria-current="step"]', (els) => els.map((e) => e.getAttribute('data-lesson')));
  assert(JSON.stringify(current) === JSON.stringify([first]), `the roadmap marks ${JSON.stringify(current)} as open`);
});

scenario('a_lesson_shows_two_examples_and_one_question', 'R2', async (t) => {
  await t.open();
  for (const id of [lessons[0].id, firstLesson('translate').id]) {
    await t.openLesson(id);
    const r = await t.page.evaluate(() => ({
      examples: [...document.querySelectorAll('#app article.lesson .lesson-example')].filter((e) => e.offsetParent !== null).length,
      questions: document.querySelectorAll('#app article.lesson .lesson-question').length,
      next: [...document.querySelectorAll('#app .lesson-next')].filter((e) => e.offsetParent !== null).length,
    }));
    assert(r.examples === 2, `lesson ${id} shows ${r.examples} examples`);
    assert(r.questions === 1, `lesson ${id} asks ${r.questions} questions`);
    assert(r.next === 0, `lesson ${id} offers Next before its question is answered`);
  }
});

scenario('a_choice_answer_moves_to_the_next_lesson', 'R3', async (t) => {
  await t.open();
  const start = firstLesson('choice').id;
  await t.openLesson(start);
  await t.choose(true);
  const v = await t.verdict();
  assert(v && v.right, `the right option was not marked right (${JSON.stringify(v)})`);
  const explanation = lesson(start).question.explanation;
  assert((await t.text('#app .lesson-question')).includes(explanation.slice(0, 40)), 'the explanation is not shown after a right answer');
  await t.next();
  const after = lessons[lessons.findIndex((l) => l.id === start) + 1].id;
  await t.waitForLesson(after);
  const done = await t.page.$eval(`#app .roadmap-lesson[data-lesson="${start}"]`, (e) => e.getAttribute('data-done'));
  assert(done === 'true', `lesson ${start} is not marked done on the roadmap (data-done=${done})`);
  assert(/\b1 of \d+ lessons done/.test(await t.text('#app .learn-progress')), 'the count did not move to 1');
});

scenario('a_typed_answer_from_buttons_and_keys_moves_on', 'R4', async (t) => {
  await t.open();
  const l = firstLesson('translate');
  await t.openLesson(l.id);
  const typed = learnerForm(l.question.answer);
  await t.compose(typed);
  assert((await t.fieldValue()) === typed, `the field reads ${JSON.stringify(await t.fieldValue())}, composed ${JSON.stringify(typed)}`);
  await t.click('#app .lesson-check');
  const v = await t.verdict();
  assert(v && v.right, `a key composed with buttons and keys was marked ${JSON.stringify(v)}`);
  await t.next();
  await t.waitForLesson(lessons[lessons.findIndex((x) => x.id === l.id) + 1].id);
});

scenario('symbol_buttons_type_at_the_caret_and_keep_focus', 'R5', async (t) => {
  await t.open();
  await t.openLesson(firstLesson('translate').id);
  await t.page.click('#lesson-answer');
  await t.page.keyboard.type('Man(x)');
  await t.page.keyboard.press('Home');
  await t.page.click('#app .symbol-key[data-symbol="∀"]');
  await t.page.keyboard.type('x');
  const r = await t.page.evaluate(() => ({ value: document.getElementById('lesson-answer').value, focus: document.activeElement?.id }));
  assert(r.value === '∀xMan(x)', `typing around a button gives ${JSON.stringify(r.value)}, not "∀xMan(x)"`);
  assert(r.focus === 'lesson-answer', `after a button the focus is on ${r.focus}, not the field`);
  await t.page.keyboard.press('End');
  await t.page.keyboard.down('Shift');
  for (let i = 0; i < 3; i++) await t.page.keyboard.press('ArrowLeft');
  await t.page.keyboard.up('Shift');
  await t.page.click('#app .symbol-key[data-symbol="¬"]');
  const replaced = await t.fieldValue();
  assert(replaced === '∀xMan¬', `a button over a selection gives ${JSON.stringify(replaced)}, not "∀xMan¬"`);
});

scenario('enter_checks_a_typed_answer', 'R4', async (t) => {
  await t.open();
  const l = firstLesson('translate');
  await t.openLesson(l.id);
  await t.compose(learnerForm(l.question.answer));
  await t.page.keyboard.press('Enter');
  await sleep(300);
  const v = await t.verdict();
  assert(v && v.right, `Enter did not check the answer (${JSON.stringify(v)})`);
});

scenario('a_wrong_answer_gets_the_socratic_hint', 'R6', async (t) => {
  await t.open();
  const l = firstLesson('translate');
  await t.openLesson(l.id);
  await t.compose('Wrong(x)');
  await t.click('#app .lesson-check');
  const v = await t.verdict();
  assert(v && !v.right, `a wrong answer was marked ${JSON.stringify(v)}`);
  const hint = await t.text('#app .lesson-hint');
  assert(hint.includes(l.question.hint.slice(0, 30)), `the hint box reads ${JSON.stringify(hint.slice(0, 120))}`);
  assert(!(await t.visible('#app .lesson-next')), 'a wrong answer offers Next');
  const live = await t.page.$eval('#app .lesson-hint', (e) => !!e.closest('[aria-live]'));
  assert(live, 'the hint is not announced (it sits outside a live region)');
});

scenario('the_answer_can_be_shown_after_a_try', 'R6', async (t) => {
  await t.open();
  const l = firstLesson('translate');
  await t.openLesson(l.id);
  assert(!(await t.visible('#app .lesson-show-answer')), 'the answer is offered before a try');
  await t.compose('Wrong(x)');
  await t.click('#app .lesson-check');
  await t.click('#app .lesson-show-answer');
  const shown = await t.text('#app .lesson-question');
  assert(shown.includes(l.question.answer), `the shown answer is not the key ${l.question.answer}`);
  assert(await t.visible('#app .lesson-next'), 'after the answer is shown there is no way on');
});

scenario('progress_survives_a_reload', 'R7', async (t) => {
  await t.open();
  const first = lessons[0];
  await t.choose(true);
  await t.next();
  await t.waitForLesson(lessons[1].id);
  await t.page.reload({ waitUntil: 'domcontentloaded' });
  await t.live();
  assert((await t.openLessonId()) === lessons[1].id, `after a reload the page opens ${await t.openLessonId()}, not ${lessons[1].id}`);
  const done = await t.page.$eval(`#app .roadmap-lesson[data-lesson="${first.id}"]`, (e) => e.getAttribute('data-done'));
  assert(done === 'true', `after a reload lesson ${first.id} is no longer marked done`);
  const stored = await t.page.evaluate(() => localStorage.getItem('logos_user_progress'));
  assert(stored && JSON.parse(stored).lessons?.includes(first.id), `progress was not saved: ${stored}`);
});

scenario('the_roadmap_opens_any_lesson', 'R8', async (t) => {
  await t.open();
  const far = lessons[lessons.length - 3];
  await t.openLesson(far.id);
  const top = await t.page.evaluate(() => document.querySelector('#app article.lesson')?.getBoundingClientRect().top ?? null);
  const height = await t.page.evaluate(() => window.innerHeight);
  assert(top !== null && top > -40 && top < height - 100, `lesson ${far.id} opened but sits at ${top}px, off screen`);
  const text = await t.text('#app article.lesson');
  assert(text.includes(far.title), `the open lesson does not read ${far.title}`);
});

scenario('more_examples_open_on_demand', 'R9', async (t) => {
  const l = lessons.find((x) => x.examples && x.examples.length > 2);
  assert(l, 'no lesson has more than two examples');
  await t.open();
  await t.openLesson(l.id);
  assert((await t.count('#app article.lesson .lesson-example')) === 2, 'extra examples show before they are asked for');
  await t.click('#app .lesson-more-examples');
  const n = await t.count('#app article.lesson .lesson-example');
  assert(n === l.examples.length, `after More examples the lesson shows ${n} of ${l.examples.length}`);
  const expanded = await t.page.$eval('#app .lesson-more-examples', (e) => e.getAttribute('aria-expanded'));
  assert(expanded === 'true', `the toggle reads aria-expanded=${expanded}`);
});

scenario('more_practice_is_optional', 'R9', async (t) => {
  const l = lessons.find((x) => x.question?.type === 'choice' && practiceOf(x.id).length > 0);
  await t.open();
  await t.openLesson(l.id);
  assert(!(await t.visible('#app .practice-card')), 'practice shows before it is asked for');
  await t.choose(true);
  assert(await t.visible('#app .lesson-next'), 'Next waits on practice');
  await t.click('#app .lesson-more-practice');
  await t.page.waitForSelector('#app .practice-card', { timeout: 30000 });
  const label = await t.text('#app .practice-progress');
  assert(new RegExp(`of ${practiceOf(l.id).length}\\b`).test(label), `practice counts ${JSON.stringify(label)}, the lesson has ${practiceOf(l.id).length}`);
  await t.next();
  await t.waitForLesson(lessons[lessons.findIndex((x) => x.id === l.id) + 1].id);
});

/** Open a lesson's practice and wait for its first question. */
async function openPractice(t, id) {
  await t.openLesson(id);
  await t.click('#app .lesson-more-practice');
  await t.page.waitForSelector('#app .practice-card .practice-sentence', { timeout: 30000 });
}

async function practiceSentence(t) {
  return t.page.$eval('#app .practice-card .practice-sentence', (e) => e.innerText.trim());
}

/** Answer the practice question on screen (a multiple-choice one), right or wrong. */
async function choosePractice(t, id, correct) {
  const prompt = await practiceSentence(t);
  const ex = practiceOf(id).find((e) => e.type === 'multiple_choice' && !e.template && e.prompt.trim() === prompt);
  assert(ex, `the practice question on screen (${JSON.stringify(prompt.slice(0, 80))}) is not a multiple-choice prompt of lesson ${id}`);
  const option = correct ? ex.options[ex.correct] : ex.options.find((_, i) => i !== ex.correct);
  const handles = await t.page.$$('#app .practice-card .practice-options button');
  for (const h of handles) {
    if ((await h.evaluate((e) => e.innerText.trim())) === option.trim()) {
      await h.click();
      await sleep(150);
      return ex;
    }
  }
  throw new Failure(`no practice option reading ${JSON.stringify(option)}`);
}

/** The lesson with the most multiple-choice practice. */
function busiestPractice() {
  return [...lessons].sort((a, b) => practiceOf(b.id).length - practiceOf(a.id).length)[0];
}

scenario('practice_comes_in_numbered_order', 'C2', async (t) => {
  const l = busiestPractice();
  await t.open();
  await openPractice(t, l.id);
  const expected = practiceOf(l.id).slice(0, 4).map((e) => e.prompt.trim());
  const seen = [await practiceSentence(t)];
  for (let i = 0; i < 3; i++) {
    await t.click('#app .practice-card .practice-skip');
    seen.push(await practiceSentence(t));
  }
  assert(JSON.stringify(seen) === JSON.stringify(expected), `practice order was ${JSON.stringify(seen)}; expected ${JSON.stringify(expected)}`);
});

scenario('a_skipped_miss_comes_back_after_three_others', 'C8', async (t) => {
  const l = busiestPractice();
  await t.open();
  await openPractice(t, l.id);
  const missed = await practiceSentence(t);
  await choosePractice(t, l.id, false);
  const seen = [];
  for (let i = 0; i < 4; i++) {
    await t.click('#app .practice-card .practice-skip');
    seen.push(await practiceSentence(t));
  }
  assert(!seen.slice(0, 3).includes(missed), 'the missed question came back before three others');
  assert(seen[3] === missed, 'a missed and skipped question never came back');
});

scenario('practice_progress_survives_a_reload', 'P2', async (t) => {
  const l = busiestPractice();
  await t.open();
  await openPractice(t, l.id);
  const first = await practiceSentence(t);
  const ex = await choosePractice(t, l.id, true);
  await t.page.reload({ waitUntil: 'domcontentloaded' });
  await t.live();
  await openPractice(t, l.id);
  assert((await practiceSentence(t)) !== first, 'after a reload practice starts over at the question already answered');
  const stored = await t.page.evaluate(() => localStorage.getItem('logos_user_progress'));
  assert(stored && stored.includes(ex.id), 'the answered practice question was never saved');
});

scenario('ambiguity_asks_for_every_reading', 'C7', async (t) => {
  const l = lessons.find((x) => practiceOf(x.id).some((e) => e.type === 'ambiguity'));
  assert(l, 'no lesson practises an ambiguity exercise');
  await t.open();
  await openPractice(t, l.id);
  let found = false;
  for (let i = 0; i < practiceOf(l.id).length && !found; i++) {
    found = (await t.count('#app .practice-card [data-answer-kind="readings"]')) > 0;
    if (!found) await t.click('#app .practice-card .practice-skip');
  }
  assert(found, 'never reached the ambiguity exercise');
  const r = await t.page.evaluate(() => ({
    inputs: document.querySelectorAll('#app .practice-card input[type=text]').length,
    shown: document.querySelectorAll('#app .practice-card .practice-key').length,
    pads: document.querySelectorAll('#app .practice-card .symbol-pad').length,
  }));
  assert(r.shown === 0, `the ${r.shown} readings are printed before the learner answers`);
  assert(r.inputs >= 2, `an ambiguity exercise offers ${r.inputs} answer field(s); it needs one per reading`);
  assert(r.pads === 1, `the readings have ${r.pads} symbol pads`);
  await t.page.click('#app .practice-card input[data-reading="1"]');
  await t.page.click('#app .practice-card .symbol-key[data-symbol="∃"]');
  const values = await t.page.$$eval('#app .practice-card input[type=text]', (els) => els.map((e) => e.value));
  assert(values[0] === '' && values[1] === '∃', `a button typed into the wrong reading: ${JSON.stringify(values)}`);
});

scenario('prompts_are_shown_for_generated_exercises', 'C16', async (t) => {
  const l = lessons.find((x) => practiceOf(x.id)[0]?.template);
  await t.open();
  await openPractice(t, l.id);
  const ex = practiceOf(l.id)[0];
  assert((await t.text('#app .practice-card')).includes(ex.prompt), `the practice question shows no instruction: its prompt ${JSON.stringify(ex.prompt)} is never displayed`);
});

scenario('notation_reads_as_symbols', 'D6', async (t) => {
  const l = lessons.find((x) => practiceOf(x.id).some((e) => (e.options || []).some((o) => /[→∧∨¬↔□◇]/.test(o))));
  assert(l, 'no practice question offers formulas as options');
  await t.open();
  await openPractice(t, l.id);
  const text = await t.text('#app article.lesson');
  assert(!/\$|\\[a-z]+/.test(text), `the lesson shows raw TeX: ${JSON.stringify(text.match(/.{0,30}(\$|\\[a-z]+).{0,30}/)?.[0])}`);
});

scenario('lesson_survives_a_missing_word_bank', 'B2', async (t) => {
  await t.open({ blockLexicon: true });
  const l = lessons.find((x) => practiceOf(x.id).length > 0);
  await t.openLesson(l.id);
  assert((await t.count('#app article.lesson .lesson-example')) === 2, 'with the word bank unavailable the lesson shows no examples');
  assert((await t.count('#app article.lesson .lesson-question')) === 1, 'with the word bank unavailable the lesson asks nothing');
  await t.click('#app .lesson-more-practice');
  await t.page.waitForFunction(() => /word bank/i.test(document.querySelector('#app article.lesson')?.innerText || ''), { timeout: 15000 });
  assert(await t.visible('#app .lexicon-gate-retry'), 'practice must offer Retry when the word bank fails');
}, { allowErrors: /lexicon\.json|404|Failed to load resource|NetworkError|fetch/i });

scenario('lesson_bold_and_emphasis_close', 'C1', async (t) => {
  await t.open();
  const r = await t.page.evaluate(() => {
    const ps = [...document.querySelectorAll('#app .lesson-teaching p')];
    return {
      paragraphs: ps.length,
      nested: document.querySelectorAll('#app .lesson-teaching strong strong, #app .lesson-teaching em em').length,
      stars: ps.filter((p) => p.textContent.includes('**')).length,
      swallowed: ps.filter((p) => { const st = p.querySelector('strong'); return st && st.textContent.length > 60; }).map((p) => p.innerHTML.slice(0, 120)),
    };
  });
  assert(r.paragraphs > 0, 'the open lesson teaches nothing');
  assert(r.nested === 0, `${r.nested} nested <strong>/<em>: an opening marker was never closed`);
  assert(r.stars === 0, `${r.stars} paragraph(s) still show a literal **`);
  assert(r.swallowed.length === 0, `bold runs on past its term: ${r.swallowed[0]}`);
});

scenario('deep_link_opens_its_module', 'C17', async (t) => {
  await t.open({ path: '/learn#modal' });
  const first = module('modal').sections[0].id;
  await t.waitForLesson(first);
  await sleep(500);
  const top = await t.page.evaluate(() => document.querySelector('#app article.lesson')?.getBoundingClientRect().top ?? null);
  assert(top !== null && top > -40 && top < 400, `/learn#modal opens lesson ${first} at ${top}px, off screen`);
});

scenario('deep_link_survives_the_trailing_slash_redirect', 'C17', async (t) => {
  // Cloudflare Pages answers /learn with a redirect to /learn/, so a shared /learn#modal reaches the
  // app as /learn/#modal.
  await t.open({ path: '/learn/#modal' });
  await t.waitForLesson(module('modal').sections[0].id);
  const bar = await t.page.evaluate(() => location.pathname + location.hash);
  assert(bar === '/learn#modal', `the address bar reads ${bar}, not /learn#modal`);
});

scenario('learn_page_ignores_a_stale_theme_choice', 'V2', async (t) => {
  // A visitor who chose Sunrise before the theme picker was removed still has it stored.
  await t.open({ storage: { 'logicaffeine-theme': '"Sunrise"' } });
  await t.openLesson(firstLesson('translate').id);
  const r = await t.page.evaluate(() => {
    const cs = (sel, prop) => { const e = document.querySelector(sel); return e ? getComputedStyle(e)[prop] : ''; };
    return {
      theme: document.getElementById('app-root')?.getAttribute('data-theme') ?? null,
      accent: getComputedStyle(document.documentElement).getPropertyValue('--accent-primary-rgb').trim(),
      page: cs('.learn-page', 'backgroundImage'),
      button: cs('#app .lesson-check', 'backgroundImage'),
    };
  });
  assert(r.theme === null, `the app root still carries a theme (data-theme ${r.theme})`);
  assert(!/rgb\(12, 10, 9\)|rgb\(28, 20, 16\)/.test(r.page), `the Learn page background is still Sunrise: ${r.page.slice(0, 160)}`);
  assert(!/249, 115, 22|251, 191, 36/.test(r.button), `the Check button is still Sunrise: ${r.button.slice(0, 160)}`);
  assert(r.accent !== '' && r.button.includes(`rgb(${r.accent})`), `the Check button does not draw the site accent (${r.accent}): ${r.button.slice(0, 160)}`);
});

scenario('answer_fields_are_one_line_tall', 'V4', async (t) => {
  await t.open();
  await t.openLesson(firstLesson('translate').id);
  const h = await t.page.$eval('#lesson-answer', (e) => Math.round(e.getBoundingClientRect().height));
  assert(h <= 64, `the answer field is ${h}px tall; a one-line field should be under 64px`);
});

scenario('symbol_buttons_are_padded_and_easy_to_tap', 'R10', async (t) => {
  await t.open();
  await t.openLesson(firstLesson('translate').id);
  const desk = await t.page.$$eval('#app .symbol-key', (els) => els.map((e) => {
    const s = getComputedStyle(e);
    return { symbol: e.getAttribute('data-symbol'), pad: [s.paddingTop, s.paddingRight, s.paddingBottom, s.paddingLeft].map(parseFloat) };
  }));
  assert(desk.length === PAD.length, `the pad has ${desk.length} buttons, not ${PAD.length}`);
  const thin = desk.filter((b) => b.pad.some((p) => p < 5));
  assert(thin.length === 0, `buttons padded under 5px: ${JSON.stringify(thin)}`);
  await t.page.setViewport(PHONE);
  await sleep(400);
  const small = await t.page.$$eval('#app .symbol-key', (els) => els.map((e) => {
    const r = e.getBoundingClientRect();
    return { symbol: e.getAttribute('data-symbol'), w: Math.round(r.width), h: Math.round(r.height) };
  }).filter((b) => b.w < 44 || b.h < 44));
  assert(small.length === 0, `tap targets under 44px on a phone: ${JSON.stringify(small)}`);
});

scenario('learn_fits_a_360px_phone', 'V3', async (t) => {
  await t.page.setViewport(PHONE);
  await t.open();
  const widest = async () => t.page.evaluate(() => Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - window.innerWidth);
  assert((await widest()) <= 0, `the roadmap page scrolls sideways by ${await widest()}px on a 360px phone`);
  await t.openLesson(firstLesson('translate').id);
  assert((await widest()) <= 0, `a typed lesson scrolls sideways by ${await widest()}px on a 360px phone`);
  const l = busiestPractice();
  await openPractice(t, l.id);
  assert((await widest()) <= 0, `practice scrolls sideways by ${await widest()}px on a 360px phone`);
});

scenario('profile_redirects_to_learn', 'R11', async (t) => {
  await t.open({ path: '/profile' });
  const bar = await t.page.evaluate(() => location.pathname);
  assert(bar === '/learn', `/profile leaves the address bar at ${bar}`);
  const text = await t.text();
  assert(!/Logic Learner|Your Profile/.test(text), 'the profile page still renders');
});

scenario('the_page_keeps_no_score', 'R12', async (t) => {
  await t.open();
  await t.choose(true);
  await t.next();
  await openPractice(t, busiestPractice().id);
  const text = await t.text('body');
  const hit = text.match(/\b(XP|Level \d|levels?|streaks?|combos?|achievements?|badges?|trophy|First Blood)\b/i);
  assert(!hit, `the Learn page keeps score: ${JSON.stringify(hit?.[0])} in ${JSON.stringify(text.slice(Math.max(0, hit?.index - 40), hit?.index + 40))}`);
});

// ---------------------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------------------

function xmlEscape(s) {
  return String(s).replace(/[<>&"']/g, (c) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&apos;' }[c]));
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  try {
    selfTest();
  } catch (e) {
    console.error(`learn-e2e: ${e.message}`);
    return 2;
  }
  if (opts.list) {
    for (const s of scenarios) console.log(`${s.name.padEnd(52)} ${s.ledger}`);
    return 0;
  }
  if (!opts.public || !fs.existsSync(path.join(opts.public, 'index.html'))) {
    console.error(`learn-e2e: no built site at ${opts.public} (expected index.html)`);
    return 2;
  }
  if (!opts.chrome === !opts.firefox) {
    console.error('learn-e2e: give exactly one of --chrome <path> or --firefox <path>');
    return 2;
  }
  const selected = opts.only ? scenarios.filter((s) => opts.only.includes(s.name)) : scenarios;
  const unknown = (opts.only || []).filter((n) => !scenarios.some((s) => s.name === n));
  if (unknown.length) { console.error(`learn-e2e: unknown scenario(s): ${unknown.join(', ')}`); return 2; }

  const puppeteer = require('puppeteer-core');
  fs.mkdirSync(opts.shots, { recursive: true });
  const server = await serve(path.resolve(opts.public));
  const engine = opts.firefox ? 'firefox' : 'chrome';
  const protocolTimeout = 300000;
  const browser = await puppeteer.launch(opts.firefox
    ? { browser: 'firefox', executablePath: opts.firefox, headless: true, protocolTimeout }
    : { executablePath: opts.chrome, headless: true, protocolTimeout, args: ['--no-sandbox', '--disable-gpu', '--window-size=1280,900', '--disable-dev-shm-usage'] });

  const results = [];
  for (const s of selected) {
    const started = Date.now();
    const context = await browser.createBrowserContext();
    const page = await context.newPage();
    await page.setViewport({ width: 1280, height: 900 });
    const errors = [];
    const network = [];
    // Firefox reports the dev client's refused socket as an uncaught error, Chrome as a console error.
    page.on('pageerror', (e) => {
      const text = String(e.message || e);
      if (DEV_CLIENT_REFUSED.test(text)) network.push(`dev client: ${text.slice(0, 200)}`);
      else errors.push(`pageerror: ${text.slice(0, 300)}`);
    });
    page.on('console', (m) => { if (m.type() === 'error' && !isDevClientNoise(m.text())) errors.push(`console.error: ${m.text().slice(0, 300)}`); });
    page.on('requestfailed', (r) => network.push(`request failed: ${new URL(r.url()).pathname} (${r.failure()?.errorText ?? 'unknown'})`));
    let failure = null;
    try {
      await Promise.race([
        s.fn(helpers(page, server.url)),
        sleep(150000).then(() => { throw new Failure('timed out after 150s'); }),
      ]);
      const unexpected = errors.filter((e) => !(s.opts.allowErrors && s.opts.allowErrors.test(e)));
      if (unexpected.length) throw new Failure(`page errors:\n${unexpected.join('\n')}`);
    } catch (e) {
      failure = e instanceof Failure ? e.message : `harness error: ${e.stack || e}`;
      // Whatever the page reported before the failure is usually its cause.
      const seen = [...errors, ...network].filter((line) => !failure.includes(line));
      if (seen.length) failure += `\nthe page also saw:\n${seen.join('\n')}`;
      await page.screenshot({ path: path.join(opts.shots, `${s.name}.${engine}.png`) }).catch(() => {});
    }
    await context.close();
    const seconds = (Date.now() - started) / 1000;
    results.push({ ...s, failure, seconds });
    console.log(failure ? `FAIL ${s.name} [${s.ledger}] (${seconds.toFixed(1)}s)\n      ${failure.split('\n').join('\n      ')}` : `  ok  ${s.name} [${s.ledger}] (${seconds.toFixed(1)}s)`);
  }

  await browser.close();
  server.close();

  const failed = results.filter((r) => r.failure).length;
  if (opts.junit) {
    const cases = results.map((r) => `    <testcase classname="learn-e2e.${engine}" name="${xmlEscape(r.name)}" time="${r.seconds.toFixed(3)}">` +
      (r.failure ? `\n      <failure message="${xmlEscape(r.failure.split('\n')[0])}">${xmlEscape(r.failure)}</failure>\n    ` : '') + '</testcase>');
    fs.writeFileSync(opts.junit, `<?xml version="1.0" encoding="UTF-8"?>\n<testsuites name="learn-e2e" tests="${results.length}" failures="${failed}">\n  <testsuite name="learn-e2e.${engine}" tests="${results.length}" failures="${failed}">\n${cases.join('\n')}\n  </testsuite>\n</testsuites>\n`);
  }
  console.log(`learn-e2e (${engine}): ${results.length - failed}/${results.length} passed${failed ? `; screenshots of each failure in ${opts.shots}` : ''}`);
  return failed ? 1 : 0;
}

main().then((code) => process.exit(code), (e) => { console.error(e); process.exit(2); });
