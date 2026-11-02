import { env } from 'cloudflare:workers';
import { afterEach, describe, expect, inject, it, vi } from 'vitest';
import vectors from '../../../crates/logicaffeine_tests/tests/fixtures/lead_vectors.json';
import {
  CLIENT_IP,
  CLIENT_IPV6,
  SALES_INBOX,
  SITE,
  SITE_HOST,
  capturingLogs,
  everyStoredValue,
  harness,
  insertLead,
  leadRequest,
  leadRows,
  minutesAgo,
  preflight,
  resendAnswer,
  siteverifyAnswer,
  validBody,
} from './test-support.js';

afterEach(() => {
  vi.restoreAllMocks();
});

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/;

const ALLOWED_ORIGINS = [
  'https://logicaffeine.com',
  'https://www.logicaffeine.com',
  'https://logicaffeine.pages.dev',
  'https://main.logicaffeine.pages.dev',
  'https://3f9a1c2e.logicaffeine.pages.dev',
  'https://pricing-page.logicaffeine.pages.dev',
  `https://${'a'.repeat(63)}.logicaffeine.pages.dev`,
];

const REFUSED_ORIGINS = [
  'https://evil.example',
  'http://logicaffeine.com',
  'http://www.logicaffeine.com',
  'https://logicaffeine.com:8443',
  'https://logicaffeine.com.evil.example',
  'https://evil-logicaffeine.com',
  'https://api.logicaffeine.com',
  'https://registry.logicaffeine.com',
  'https://LOGICAFFEINE.COM',
  'https://a.b.logicaffeine.pages.dev',
  'https://xlogicaffeine.pages.dev',
  'https://-bad.logicaffeine.pages.dev',
  'https://bad-.logicaffeine.pages.dev',
  'https://logicaffeine.pages.dev.evil.example',
  'http://main.logicaffeine.pages.dev',
  'http://logicaffeine.pages.dev',
  'http://pricing-page.logicaffeine.pages.dev',
  'wss://main.logicaffeine.pages.dev',
  'xhttps://main.logicaffeine.pages.dev',
  'https://evil-logicaffeine.pages.dev',
  'https://logicaffeine-evil.pages.dev',
  'https://evil-logicaffeine.pages.dev.attacker.com',
  'https://main.logicaffeine.pages.dev.attacker.com',
  'https://main.logicaffeine.pages.dev.',
  'https://main.logicaffeine.pages.dev:443',
  'https://main.logicaffeine.pages.dev/pricing',
  'https://MAIN.logicaffeine.pages.dev',
  'https://main_x.logicaffeine.pages.dev',
  'https://.logicaffeine.pages.dev',
  `https://${'a'.repeat(64)}.logicaffeine.pages.dev`,
  'https://*.logicaffeine.pages.dev',
  'http://localhost:8080',
  'http://127.0.0.1:9000',
  'null',
  '',
];

/** Nothing reached storage, siteverify, the rate limiters or the mailer. */
async function expectUntouched(h) {
  expect(await leadRows()).toEqual([]);
  expect(h.verified).toEqual([]);
  expect(h.limited).toEqual({ ip: [], email: [] });
  expect(h.mailed).toEqual([]);
}

describe('the harness', () => {
  it('starts every test with an empty leads table', async () => {
    expect(await leadRows()).toEqual([]);
  });

  it('lets a test write a lead', async () => {
    await insertLead({ email: 'leak@example.com', createdAt: minutesAgo(1) });
    expect(await leadRows()).toHaveLength(1);
  });

  it('never shows one test the rows another test wrote', async () => {
    expect(await leadRows()).toEqual([]);
  });
});

describe('routing', () => {
  it.each(['/', '/leads', '/lead/', '/lead/extra', '/LEAD', '/validate', '/health', '/lead.json'])(
    'answers 404 for %s, to GET and to POST',
    async (path) => {
      const h = harness();
      for (const method of ['GET', 'POST']) {
        const r = await h.send(leadRequest({ path, method }));
        expect(r.status, method).toBe(404);
        expect(r.body, method).toEqual({ ok: false, error: 'not_found' });
      }
      await expectUntouched(h);
    },
  );

  it.each(['GET', 'PUT', 'DELETE', 'PATCH'])('answers 405 to %s /lead, naming the allowed methods', async (method) => {
    const h = harness();
    const r = await h.send(leadRequest({ method }));
    expect(r.status).toBe(405);
    expect(r.headers.get('Allow')).toBe('POST, OPTIONS');
    expect(r.body).toEqual({ ok: false, error: 'method_not_allowed' });
    await expectUntouched(h);
  });

  it('answers 405 to HEAD /lead', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ method: 'HEAD' }));
    expect(r.status).toBe(405);
    expect(r.headers.get('Allow')).toBe('POST, OPTIONS');
  });

  it('ignores a query string on /lead', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ path: '/lead?utm_source=newsletter' }));
    expect(r.status).toBe(202);
    expect(await leadRows()).toHaveLength(1);
  });
});

describe('CORS', () => {
  it.each(ALLOWED_ORIGINS)('accepts a lead from %s and lets the page read the answer', async (origin) => {
    const h = harness({ siteverify: siteverifyAnswer({ hostname: new URL(origin).hostname }) });
    const r = await h.send(leadRequest({ origin }));
    expect(r.status).toBe(202);
    expect(r.body).toEqual({ ok: true });
    expect(r.headers.get('Access-Control-Allow-Origin')).toBe(origin);
    expect(r.headers.get('Vary')).toMatch(/\bOrigin\b/);
    expect(r.headers.get('Access-Control-Allow-Credentials')).toBeNull();
    expect(await leadRows()).toHaveLength(1);
  });

  it.each(REFUSED_ORIGINS)('refuses a lead from %j with 403 and no CORS grant', async (origin) => {
    const h = harness();
    const r = await h.send(leadRequest({ origin }));
    expect(r.status).toBe(403);
    expect(r.body).toEqual({ ok: false, error: 'origin_not_allowed' });
    expect(r.headers.get('Access-Control-Allow-Origin')).toBeNull();
    expect(r.headers.get('Vary')).toMatch(/\bOrigin\b/);
    await expectUntouched(h);
  });

  it('refuses a lead with no Origin header', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ origin: null }));
    expect(r.status).toBe(403);
    expect(r.body).toEqual({ ok: false, error: 'origin_not_allowed' });
    await expectUntouched(h);
  });

  it('grants CORS on error answers too, so the page can read them', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ email: 'not-an-email' }) }));
    expect(r.status).toBe(400);
    expect(r.headers.get('Access-Control-Allow-Origin')).toBe(SITE);
  });

  it('never answers with a wildcard origin', async () => {
    const h = harness();
    const granted = await h.send(leadRequest({ origin: SITE }));
    expect(granted.headers.get('Access-Control-Allow-Origin')).toBe(SITE);
    const refused = await h.send(leadRequest({ origin: 'https://evil.example' }));
    expect(refused.headers.get('Access-Control-Allow-Origin')).toBeNull();
    const pre = await h.send(preflight());
    expect(pre.headers.get('Access-Control-Allow-Origin')).toBe(SITE);
  });
});

describe('preflight', () => {
  it.each(ALLOWED_ORIGINS)('grants %s a POST with a JSON body', async (origin) => {
    const h = harness();
    const r = await h.send(preflight({ origin }));
    expect(r.status).toBe(204);
    expect(r.body).toBeNull();
    expect(r.headers.get('Access-Control-Allow-Origin')).toBe(origin);
    expect(r.headers.get('Access-Control-Allow-Methods')).toBe('POST, OPTIONS');
    expect(r.headers.get('Access-Control-Allow-Headers')).toBe('Content-Type');
    expect(r.headers.get('Access-Control-Max-Age')).toBe('86400');
    expect(r.headers.get('Vary')).toMatch(/\bOrigin\b/);
    await expectUntouched(h);
  });

  it.each(REFUSED_ORIGINS)('refuses %j with 403 and no grant', async (origin) => {
    const h = harness();
    const r = await h.send(preflight({ origin }));
    expect(r.status).toBe(403);
    expect(r.headers.get('Access-Control-Allow-Origin')).toBeNull();
    expect(r.headers.get('Access-Control-Allow-Methods')).toBeNull();
    await expectUntouched(h);
  });

  it('refuses a preflight with no Origin header', async () => {
    const h = harness();
    const r = await h.send(preflight({ origin: null }));
    expect(r.status).toBe(403);
    expect(r.headers.get('Access-Control-Allow-Origin')).toBeNull();
  });

  it('grants only POST when the page asks for another method', async () => {
    const h = harness();
    const r = await h.send(preflight({ method: 'PUT' }));
    expect(r.headers.get('Access-Control-Allow-Methods')).toBe('POST, OPTIONS');
  });

  it('answers 404 to a preflight for another path', async () => {
    const h = harness();
    const r = await h.send(preflight({ path: '/validate' }));
    expect(r.status).toBe(404);
  });
});

describe('the body', () => {
  const CAP = 8192;
  const padded = (bytes) => JSON.stringify(validBody()).padEnd(bytes, ' ');

  it('accepts a body of exactly 8 KiB', async () => {
    const h = harness();
    const raw = padded(CAP);
    expect(new TextEncoder().encode(raw).byteLength).toBe(CAP);
    const r = await h.send(leadRequest({ raw }));
    expect(r.status).toBe(202);
  });

  it('refuses a declared body one byte over 8 KiB with 413', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ raw: padded(CAP + 1) }));
    expect(r.status).toBe(413);
    expect(r.body).toEqual({ ok: false, error: 'body_too_large' });
    await expectUntouched(h);
  });

  it('refuses a streamed body with no length that runs past 8 KiB', async () => {
    const h = harness();
    const bytes = new TextEncoder().encode(padded(CAP * 4));
    const stream = new ReadableStream({
      start(controller) {
        for (let i = 0; i < bytes.length; i += 1000) controller.enqueue(bytes.slice(i, i + 1000));
        controller.close();
      },
    });
    const request = leadRequest({ raw: stream });
    expect(request.headers.get('Content-Length')).toBeNull();
    const r = await h.send(request);
    expect(r.status).toBe(413);
    expect(r.body).toEqual({ ok: false, error: 'body_too_large' });
    await expectUntouched(h);
  });

  it('accepts a streamed body with no length that stays within 8 KiB', async () => {
    const h = harness();
    const bytes = new TextEncoder().encode(padded(CAP));
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue(bytes.slice(0, 4000));
        controller.enqueue(bytes.slice(4000));
        controller.close();
      },
    });
    const r = await h.send(leadRequest({ raw: stream }));
    expect(r.status).toBe(202);
  });

  it('counts bytes, not characters', async () => {
    const h = harness();
    const raw = JSON.stringify(validBody({ website: '', source: 'pricing', note: '\u00e9'.repeat(4100) }));
    expect(raw.length).toBeLessThan(CAP);
    expect(new TextEncoder().encode(raw).byteLength).toBeGreaterThan(CAP);
    const r = await h.send(leadRequest({ raw }));
    expect(r.status).toBe(413);
  });

  it.each(['text/plain', 'application/x-www-form-urlencoded', 'multipart/form-data; boundary=x', 'application/jsonx'])(
    'refuses Content-Type %s with 415',
    async (contentType) => {
      const h = harness();
      const r = await h.send(leadRequest({ contentType }));
      expect(r.status).toBe(415);
      expect(r.body).toEqual({ ok: false, error: 'unsupported_media_type' });
      await expectUntouched(h);
    },
  );

  it('refuses a body with no Content-Type with 415', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ contentType: null }));
    expect(r.status).toBe(415);
    await expectUntouched(h);
  });

  it.each(['application/json; charset=utf-8', 'Application/JSON', 'application/json;charset=UTF-8'])(
    'accepts Content-Type %s',
    async (contentType) => {
      const h = harness();
      const r = await h.send(leadRequest({ contentType }));
      expect(r.status).toBe(202);
    },
  );

  it.each([['{'], [''], ['[]'], ['null'], ['"buyer@example.com"'], ['42'], ['true'], ['{"email": "a@b.co",}']])(
    'refuses %j as invalid JSON',
    async (raw) => {
      const h = harness();
      const r = await h.send(leadRequest({ raw }));
      expect(r.status).toBe(400);
      expect(r.body).toEqual({ ok: false, error: 'invalid_json' });
      await expectUntouched(h);
    },
  );

  it('refuses a body that is not UTF-8 as invalid JSON', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ raw: new Uint8Array([0x7b, 0x22, 0xff, 0xfe, 0x22, 0x3a, 0x31, 0x7d]) }));
    expect(r.status).toBe(400);
    expect(r.body).toEqual({ ok: false, error: 'invalid_json' });
  });
});

describe('the honeypot', () => {
  it.each([['https://spam.example'], ['x'], ['  x  '], [0], [1], [false], [true], [{}], [[]], [['x']]])(
    'answers %j like a success and drops the lead unseen',
    async (website) => {
      const h = harness();
      const r = await h.send(leadRequest({ body: validBody({ website }) }));
      expect(r.status).toBe(202);
      expect(r.body).toEqual({ ok: true });
      expect(r.headers.get('Access-Control-Allow-Origin')).toBe(SITE);
      await expectUntouched(h);
    },
  );

  it.each([[''], ['   '], ['\u00a0\u3000'], [null], [undefined]])('lets an empty honeypot %j through', async (website) => {
    const h = harness();
    const body = validBody({ website });
    if (website === undefined) delete body.website;
    const r = await h.send(leadRequest({ body }));
    expect(r.status).toBe(202);
    expect(await leadRows()).toHaveLength(1);
    expect(h.sent).toHaveLength(1);
  });

  it('gives a bot the same status, body and headers a person gets', async () => {
    const person = await harness().send(leadRequest());
    expect(person.status).toBe(202);
    const bot = await harness().send(leadRequest({ body: validBody({ website: 'https://spam.example' }) }));
    expect(bot.status).toBe(person.status);
    expect(bot.body).toEqual(person.body);
    expect([...bot.headers].sort()).toEqual([...person.headers].sort());
  });
});

describe('validation walks the shared vectors through the endpoint', () => {
  const invalidEmails = vectors.email.filter((v) => !v.valid);
  const validEmails = vectors.email.filter((v) => v.valid);
  const invalidPhones = vectors.phone.filter((v) => !v.valid);
  const validPhones = vectors.phone.filter((v) => v.valid);

  it.each(invalidEmails)('400 invalid_email ($reason): $note', async (v) => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ email: v.input }) }));
    expect(r.status).toBe(400);
    expect(r.body).toEqual({ ok: false, error: 'invalid_email', reason: v.reason });
    await expectUntouched(h);
  });

  it.each(validEmails)('202 and stored normalized: $note', async (v) => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ email: v.input }) }));
    expect(r.status).toBe(202);
    const [row] = await leadRows();
    expect(row.email).toBe(v.normalized);
    expect(h.sent[0].reply_to).toBe(v.normalized);
  });

  it.each(invalidPhones)('400 invalid_phone ($reason): $note', async (v) => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ phone: v.input }) }));
    expect(r.status).toBe(400);
    expect(r.body).toEqual({ ok: false, error: 'invalid_phone', reason: v.reason });
    await expectUntouched(h);
  });

  it.each(validPhones)('202 and stored normalized: $note', async (v) => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ phone: v.input }) }));
    expect(r.status).toBe(202);
    const [row] = await leadRows();
    expect(row.phone).toBe(v.normalized);
    expect(row.phone_digits).toBe(v.digits);
  });

  it('calls a missing email required', async () => {
    const h = harness();
    const body = validBody();
    delete body.email;
    const r = await h.send(leadRequest({ body }));
    expect(r.status).toBe(400);
    expect(r.body).toEqual({ ok: false, error: 'invalid_email', reason: 'required' });
  });

  it.each([[42], [true], [{}], [['buyer@example.com']]])('calls a non-string email %j malformed', async (email) => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ email }) }));
    expect(r.status).toBe(400);
    expect(r.body).toEqual({ ok: false, error: 'invalid_email', reason: 'malformed' });
  });

  it.each([[5551234567], [true], [{}], [['555-123-4567']]])('calls a non-string phone %j malformed', async (phone) => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ phone }) }));
    expect(r.status).toBe(400);
    expect(r.body).toEqual({ ok: false, error: 'invalid_phone', reason: 'malformed' });
  });

  it('reports the email before the phone when both are wrong', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ email: 'nope', phone: 'call me' }) }));
    expect(r.body).toEqual({ ok: false, error: 'invalid_email', reason: 'malformed' });
  });

  it('reports the phone before the source when both are wrong', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ phone: 'call me', source: '<script>' }) }));
    expect(r.body).toEqual({ ok: false, error: 'invalid_phone', reason: 'bad_character' });
  });

  it.each([['<script>'], [''], ['a'.repeat(65)], [42], [{}]])('refuses the source %j', async (source) => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ source }) }));
    expect(r.status).toBe(400);
    expect(r.body).toEqual({ ok: false, error: 'invalid_source' });
    await expectUntouched(h);
  });

  it('stores the source the page names', async () => {
    const h = harness();
    await h.send(leadRequest({ body: validBody({ source: '/pricing#contact' }) }));
    expect((await leadRows())[0].source).toBe('/pricing#contact');
  });

  it('stores NULL for a missing or null source', async () => {
    const h = harness();
    const body = validBody();
    delete body.source;
    await h.send(leadRequest({ body }));
    await h.send(leadRequest({ body: validBody({ email: 'second@example.com', source: null }) }));
    expect((await leadRows()).map((row) => row.source)).toEqual([null, null]);
  });

  it('ignores fields it does not know', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ company: 'Acme', status: 'closed', id: 99 }) }));
    expect(r.status).toBe(202);
    const [row] = await leadRows();
    expect(row.status).toBe('new');
    expect(row.id).not.toBe(99);
  });
});

describe('the optional phone', () => {
  it.each([
    ['null', null],
    ['empty', ''],
    ['spaces', '   '],
    ['Unicode whitespace', '\u00a0\u3000\ufeff'],
  ])('stores %s as NULL phone and NULL digits', async (_, phone) => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ phone }) }));
    expect(r.status).toBe(202);
    const [row] = await leadRows();
    expect(row.phone).toBeNull();
    expect(row.phone_digits).toBeNull();
  });

  it('stores a missing phone as NULL phone and NULL digits', async () => {
    const h = harness();
    const body = validBody();
    delete body.phone;
    const r = await h.send(leadRequest({ body }));
    expect(r.status).toBe(202);
    const [row] = await leadRows();
    expect(row.phone).toBeNull();
    expect(row.phone_digits).toBeNull();
  });

  it('says so in the notification when no phone was given', async () => {
    const h = harness();
    await h.send(leadRequest({ body: validBody({ phone: null }) }));
    expect(h.sent).toHaveLength(1);
    expect(h.sent[0].text).toMatch(/^Phone: +not given$/m);
  });

  it('keeps phone and digits NULL together: the table refuses one without the other', async () => {
    await expect(
      env.DB.prepare("INSERT INTO leads (email, phone) VALUES ('a@b.co', '555-123-4567')").run(),
    ).rejects.toThrow(/CHECK constraint failed/);
    await expect(
      env.DB.prepare("INSERT INTO leads (email, phone_digits) VALUES ('a@b.co', '5551234567')").run(),
    ).rejects.toThrow(/CHECK constraint failed/);
  });
});

describe('Turnstile', () => {
  it('asks siteverify once, with the secret, the token, the client IP, an idempotency key and a timeout', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ turnstile_token: 'tok-123' }) }));
    expect(r.status).toBe(202);
    expect(h.verified).toHaveLength(1);
    const [call] = h.verified;
    expect(call.method).toBe('POST');
    expect(call.contentType).toMatch(/^application\/x-www-form-urlencoded/);
    expect(call.form.secret).toBe(inject('turnstileSecret'));
    expect(call.form.response).toBe('tok-123');
    expect(call.form.remoteip).toBe(CLIENT_IP);
    expect(call.form.idempotency_key).toMatch(UUID);
    expect(Object.keys(call.form).sort()).toEqual(['idempotency_key', 'remoteip', 'response', 'secret']);
    expect(call.hasSignal).toBe(true);
  });

  it('passes an IPv6 client address through', async () => {
    const h = harness();
    await h.send(leadRequest({ ip: CLIENT_IPV6 }));
    expect(h.verified).toHaveLength(1);
    expect(h.verified[0].form.remoteip).toBe(CLIENT_IPV6);
  });

  it('leaves remoteip out when Cloudflare names no client IP', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ ip: null }));
    expect(r.status).toBe(202);
    expect(h.verified).toHaveLength(1);
    expect('remoteip' in h.verified[0].form).toBe(false);
  });

  it.each([
    ['missing', undefined],
    ['null', null],
    ['empty', ''],
    ['a number', 42],
    ['an object', {}],
    ['2049 characters', 't'.repeat(2049)],
  ])('refuses a %s token with 403 without asking siteverify', async (_, token) => {
    const h = harness();
    const body = validBody({ turnstile_token: token });
    if (token === undefined) delete body.turnstile_token;
    const r = await h.send(leadRequest({ body }));
    expect(r.status).toBe(403);
    expect(r.body).toEqual({ ok: false, error: 'turnstile_failed' });
    await expectUntouched(h);
  });

  it('asks siteverify about a token of exactly 2048 characters', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ turnstile_token: 't'.repeat(2048) }) }));
    expect(r.status).toBe(202);
    expect(h.verified[0].form.response).toHaveLength(2048);
  });

  it.each([
    ['an invalid token', { success: false, errorCodes: ['invalid-input-response'] }],
    ['a spent token', { success: false, errorCodes: ['timeout-or-duplicate'] }],
    ['a wrong secret', { success: false, errorCodes: ['invalid-input-secret'] }],
    ['another action', { action: 'signup' }],
    ['the endpoint name as the action', { action: 'lead' }],
    ['no action', { action: null }],
    ['another site', { hostname: 'evil.example' }],
    ['a sibling host', { hostname: `www.${SITE_HOST}` }],
    ['the test widget', { hostname: 'localhost', action: 'test' }],
  ])('refuses %s with 403', async (_, answer) => {
    const h = harness({ siteverify: siteverifyAnswer(answer) });
    const r = await h.send(leadRequest());
    expect(r.status).toBe(403);
    expect(r.body).toEqual({ ok: false, error: 'turnstile_failed' });
    expect(h.verified).toHaveLength(1);
    expect(await leadRows()).toEqual([]);
    expect(h.limited).toEqual({ ip: [], email: [] });
    expect(h.sent).toEqual([]);
  });

  it('checks the hostname against the page that sent the lead', async () => {
    const origin = 'https://www.logicaffeine.com';
    const good = harness({ siteverify: siteverifyAnswer({ hostname: 'www.logicaffeine.com' }) });
    expect((await good.send(leadRequest({ origin }))).status).toBe(202);
    vi.restoreAllMocks();
    const bad = harness({ siteverify: siteverifyAnswer({ hostname: SITE_HOST }) });
    expect((await bad.send(leadRequest({ origin }))).status).toBe(403);
  });

  it('retries once after a siteverify outage, with the same idempotency key', async () => {
    const pass = siteverifyAnswer();
    const h = harness({ siteverify: (n) => (n === 1 ? new Response('upstream', { status: 503 }) : pass()) });
    const r = await h.send(leadRequest());
    expect(r.status).toBe(202);
    expect(h.verified).toHaveLength(2);
    expect(h.verified[1].form.idempotency_key).toBe(h.verified[0].form.idempotency_key);
  });

  it('retries once after a network error', async () => {
    const pass = siteverifyAnswer();
    const h = harness({
      siteverify: (n) => {
        if (n === 1) throw new TypeError('network connection lost');
        return pass();
      },
    });
    const r = await h.send(leadRequest());
    expect(r.status).toBe(202);
    expect(h.verified).toHaveLength(2);
  });

  it('fails closed with 403 when siteverify stays down, after exactly two tries', async () => {
    const h = harness({ siteverify: () => new Response('upstream', { status: 502 }) });
    const r = await h.send(leadRequest());
    expect(r.status).toBe(403);
    expect(r.body).toEqual({ ok: false, error: 'turnstile_failed' });
    expect(h.verified).toHaveLength(2);
    expect(await leadRows()).toEqual([]);
  });

  it('does not retry a siteverify answer of 400', async () => {
    const h = harness({ siteverify: () => new Response('{}', { status: 400 }) });
    const r = await h.send(leadRequest());
    expect(r.status).toBe(403);
    expect(h.verified).toHaveLength(1);
  });

  it('fails closed on an answer that is not JSON', async () => {
    const h = harness({ siteverify: () => new Response('<html>', { status: 200 }) });
    const r = await h.send(leadRequest());
    expect(r.status).toBe(403);
    expect(h.verified).toHaveLength(1);
  });

  it('answers 500 and asks nobody when the secret is not configured', async () => {
    const h = harness({ env: { TURNSTILE_SECRET: undefined } });
    const { result: r, lines } = await capturingLogs(() => h.send(leadRequest()));
    expect(r.status).toBe(500);
    expect(r.body).toEqual({ ok: false, error: 'internal' });
    await expectUntouched(h);
    expect(lines.some((l) => l.level === 'error' && JSON.parse(l.args[0]).event === 'turnstile_secret_missing')).toBe(
      true,
    );
  });
});

describe('rate limits', () => {
  it('answers 429 with Retry-After when the client IP is over its limit, and stores nothing', async () => {
    const h = harness({ ipAllows: false });
    const r = await h.send(leadRequest());
    expect(r.status).toBe(429);
    expect(r.body).toEqual({ ok: false, error: 'rate_limited' });
    expect(r.headers.get('Retry-After')).toBe('60');
    expect(r.headers.get('Access-Control-Allow-Origin')).toBe(SITE);
    expect(h.limited.ip).toEqual([`ip:${CLIENT_IP}`]);
    expect(h.limited.email).toEqual([]);
    expect(await leadRows()).toEqual([]);
    expect(h.sent).toEqual([]);
  });

  it('answers 429 when the email address is over its limit', async () => {
    const h = harness({ emailAllows: false });
    const r = await h.send(leadRequest());
    expect(r.status).toBe(429);
    expect(r.body).toEqual({ ok: false, error: 'rate_limited' });
    expect(h.limited.ip).toEqual([`ip:${CLIENT_IP}`]);
    expect(h.limited.email).toEqual(['email:buyer@example.com']);
    expect(await leadRows()).toEqual([]);
  });

  it('keys the email limit on the whole address in lower case', async () => {
    const h = harness();
    await h.send(leadRequest({ body: validBody({ email: '  Buyer.Name@Example.COM ' }) }));
    expect(h.limited.email).toEqual(['email:buyer.name@example.com']);
  });

  it('keys an unknown client IP as unknown', async () => {
    const h = harness();
    await h.send(leadRequest({ ip: null }));
    expect(h.limited.ip).toEqual(['ip:unknown']);
  });

  it('lets a lead through when a rate limiter itself fails, and logs it', async () => {
    const h = harness({
      env: {
        LEAD_IP_LIMIT: {
          limit: async () => {
            throw new Error('limiter unavailable');
          },
        },
      },
    });
    const { result: r, lines } = await capturingLogs(() => h.send(leadRequest()));
    expect(r.status).toBe(202);
    expect(await leadRows()).toHaveLength(1);
    expect(lines.some((l) => l.level === 'error' && JSON.parse(l.args[0]).event === 'rate_limiter_failed')).toBe(true);
  });
});

describe('storage', () => {
  it('creates the leads table with exactly the planned columns', async () => {
    const { results } = await env.DB.prepare('PRAGMA table_info(leads)').all();
    const columns = results.map(({ name, type, notnull, dflt_value, pk }) => ({ name, type, notnull, dflt_value, pk }));
    expect(columns).toEqual([
      { name: 'id', type: 'INTEGER', notnull: 0, dflt_value: null, pk: 1 },
      { name: 'email', type: 'TEXT', notnull: 1, dflt_value: null, pk: 0 },
      { name: 'phone', type: 'TEXT', notnull: 0, dflt_value: null, pk: 0 },
      { name: 'phone_digits', type: 'TEXT', notnull: 0, dflt_value: null, pk: 0 },
      { name: 'source', type: 'TEXT', notnull: 0, dflt_value: null, pk: 0 },
      { name: 'country', type: 'TEXT', notnull: 0, dflt_value: null, pk: 0 },
      { name: 'created_at', type: 'TEXT', notnull: 1, dflt_value: "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", pk: 0 },
      { name: 'notified_at', type: 'TEXT', notnull: 0, dflt_value: null, pk: 0 },
      { name: 'status', type: 'TEXT', notnull: 1, dflt_value: "'new'", pk: 0 },
    ]);
  });

  it('indexes the lookups it makes: email with notified_at, and created_at', async () => {
    const { results } = await env.DB.prepare("SELECT name, sql FROM sqlite_master WHERE type = 'index' AND tbl_name = 'leads'").all();
    const byName = Object.fromEntries(results.map((r) => [r.name, r.sql]));
    expect(byName.idx_leads_email_notified).toMatch(/\(\s*email COLLATE NOCASE,\s*notified_at\s*\)/);
    expect(byName.idx_leads_created).toMatch(/\(\s*created_at\s*\)/);
  });

  it('stores the lead as one row', async () => {
    const h = harness();
    const before = new Date().toISOString();
    const r = await h.send(leadRequest({ cf: { country: 'DE' } }));
    const after = new Date().toISOString();
    expect(r.status).toBe(202);
    const rows = await leadRows();
    expect(rows).toHaveLength(1);
    const [row] = rows;
    expect(row).toEqual({
      id: expect.any(Number),
      email: 'buyer@example.com',
      phone: '+1 (555) 123-4567',
      phone_digits: '15551234567',
      source: 'pricing',
      country: 'DE',
      created_at: expect.stringMatching(ISO),
      notified_at: expect.stringMatching(ISO),
      status: 'new',
    });
    expect(row.created_at >= before && row.created_at <= after).toBe(true);
    expect(row.notified_at >= row.created_at && row.notified_at <= after).toBe(true);
  });

  it('stores a NULL country when Cloudflare names none', async () => {
    const h = harness();
    await h.send(leadRequest());
    expect((await leadRows())[0].country).toBeNull();
  });

  it.each([
    ['T1', 'T1'],
    ['XX', 'XX'],
    ['Germany', null],
    ['de', null],
    [49, null],
  ])('stores the country %j as %j', async (country, stored) => {
    const h = harness();
    await h.send(leadRequest({ cf: { country } }));
    expect((await leadRows())[0].country).toBe(stored);
  });

  it('stores every submission, even a repeat', async () => {
    const h = harness();
    await h.send(leadRequest());
    await h.send(leadRequest());
    expect(await leadRows()).toHaveLength(2);
  });

  it('answers 500 and sends nothing when the database fails', async () => {
    const broken = {
      prepare() {
        throw new Error('D1 unavailable');
      },
    };
    const h = harness({ env: { DB: broken } });
    const { result: r, lines } = await capturingLogs(() => h.send(leadRequest()));
    expect(r.status).toBe(500);
    expect(r.body).toEqual({ ok: false, error: 'internal' });
    expect(h.sent).toEqual([]);
    expect(lines.some((l) => l.level === 'error' && JSON.parse(l.args[0]).event === 'store_failed')).toBe(true);
  });
});

describe('the notification', () => {
  const failedNotify = (lines) => lines.map((l) => JSON.parse(l.args[0])).filter((l) => l.event === 'notify_failed');

  it('posts one email to Resend for the sales inbox, from the configured sender, replying to the prospect', async () => {
    const h = harness();
    await h.send(leadRequest({ cf: { country: 'US' } }));
    expect(h.mailed).toHaveLength(1);
    const [{ body }] = h.mailed;
    expect(body.to).toEqual([SALES_INBOX]);
    expect(body.from).toBe(`LOGICAFFEINE pricing form <${env.NOTIFY_FROM}>`);
    expect(body.reply_to).toBe('buyer@example.com');
    expect(body.subject).toBe('Pricing request \u2014 buyer@example.com');
    expect(Object.keys(body).sort()).toEqual(['from', 'html', 'reply_to', 'subject', 'text', 'to']);
  });

  it('authenticates with the API key, sends JSON with a timeout, and keys the send to the lead', async () => {
    const h = harness();
    await h.send(leadRequest());
    const [row] = await leadRows();
    expect(h.mailed).toHaveLength(1);
    const [attempt] = h.mailed;
    expect(attempt.method).toBe('POST');
    expect(attempt.authorization).toBe(`Bearer ${inject('resendKey')}`);
    expect(attempt.contentType).toMatch(/^application\/json/);
    expect(attempt.hasSignal).toBe(true);
    expect(attempt.idempotencyKey).toBe(`pricing-lead/${row.id}/${row.created_at}`);
    expect(attempt.idempotencyKey.length).toBeLessThanOrEqual(256);
  });

  it('logs the id Resend gives the email', async () => {
    const h = harness();
    const { lines } = await capturingLogs(() => h.send(leadRequest()));
    const [row] = await leadRows();
    const event = lines.map((l) => JSON.parse(l.args[0])).find((l) => l.event === 'lead_notified');
    expect(event).toEqual({ event: 'lead_notified', lead: row.id, email_id: 'email-1' });
  });

  it('gives every lead its own idempotency key', async () => {
    const h = harness();
    await h.send(leadRequest({ body: validBody({ email: 'first@example.com' }) }));
    await h.send(leadRequest({ body: validBody({ email: 'second@example.com' }) }));
    const keys = h.mailed.map((m) => m.idempotencyKey);
    expect(keys).toHaveLength(2);
    expect(new Set(keys).size).toBe(2);
  });

  it('writes the lead into the plain text', async () => {
    const h = harness();
    await h.send(leadRequest({ cf: { country: 'US' } }));
    const [row] = await leadRows();
    expect(h.sent).toHaveLength(1);
    const { text } = h.sent[0];
    expect(text).toMatch(/^Email: +buyer@example\.com$/m);
    expect(text).toMatch(/^Phone: +\+1 \(555\) 123-4567$/m);
    expect(text).toMatch(/^Source: +pricing$/m);
    expect(text).toMatch(/^Country: +US$/m);
    expect(text).toMatch(new RegExp(`^Received: +${row.created_at.replace(/\./g, '\\.')}$`, 'm'));
    expect(text).toMatch(new RegExp(`^Lead: +#${row.id}$`, 'm'));
    expect(text).toMatch(/Reply to this email to answer the prospect\./);
  });

  it('escapes the lead in the HTML part', async () => {
    const h = harness();
    await h.send(leadRequest({ body: validBody({ email: "o'brien&co@example.com" }) }));
    expect(h.sent).toHaveLength(1);
    const [message] = h.sent;
    expect(message.subject).toBe("Pricing request \u2014 o'brien&co@example.com");
    expect(message.reply_to).toBe("o'brien&co@example.com");
    expect(message.html).toContain('o&#39;brien&amp;co@example.com');
    expect(message.html).not.toContain("o'brien&co");
    expect(message.text).toContain("o'brien&co@example.com");
  });

  it('never writes to the prospect', async () => {
    const h = harness();
    for (let i = 0; i < 3; i++) {
      await h.send(leadRequest({ body: validBody({ email: `buyer${i}@example.com` }) }));
    }
    expect(h.mailed).toHaveLength(3);
    for (const { body } of h.mailed) {
      expect(body.to).toEqual([SALES_INBOX]);
      expect(body).not.toHaveProperty('cc');
      expect(body).not.toHaveProperty('bcc');
    }
  });

  it('addresses whatever NOTIFY_TO names, and nothing else', async () => {
    const h = harness({ env: { NOTIFY_TO: 'sales@datacenter.dev' } });
    await h.send(leadRequest());
    expect(h.mailed.map((m) => m.body.to)).toEqual([['sales@datacenter.dev']]);
  });

  it('stores a repeat within 10 minutes but does not email again, whatever the case of the address', async () => {
    const h = harness();
    await h.send(leadRequest({ body: validBody({ email: 'buyer@example.com' }) }));
    const r = await h.send(leadRequest({ body: validBody({ email: 'BUYER@Example.com' }) }));
    expect(r.status).toBe(202);
    const rows = await leadRows();
    expect(rows).toHaveLength(2);
    expect(h.mailed).toHaveLength(1);
    expect(rows[0].notified_at).toMatch(ISO);
    expect(rows[1].notified_at).toBeNull();
  });

  it('emails about a different address at once', async () => {
    const h = harness();
    await h.send(leadRequest({ body: validBody({ email: 'first@example.com' }) }));
    await h.send(leadRequest({ body: validBody({ email: 'second@example.com' }) }));
    expect(h.sent.map((m) => m.reply_to)).toEqual(['first@example.com', 'second@example.com']);
  });

  it('stays quiet when the address was emailed about 9 minutes 59 seconds ago', async () => {
    await insertLead({ email: 'buyer@example.com', createdAt: minutesAgo(9, 59), notifiedAt: minutesAgo(9, 59) });
    const h = harness();
    await h.send(leadRequest());
    expect(h.mailed).toEqual([]);
    expect(await leadRows()).toHaveLength(2);
  });

  it('emails again once 10 minutes have passed', async () => {
    await insertLead({ email: 'buyer@example.com', createdAt: minutesAgo(10, 1), notifiedAt: minutesAgo(10, 1) });
    const h = harness();
    await h.send(leadRequest());
    expect(h.sent).toHaveLength(1);
  });

  it('counts only a sent email: an earlier lead that was never emailed about does not silence the next', async () => {
    await insertLead({ email: 'buyer@example.com', createdAt: minutesAgo(1), notifiedAt: null });
    const h = harness();
    await h.send(leadRequest());
    expect(h.sent).toHaveLength(1);
  });

  it('sends exactly one email when leads for one address arrive together', async () => {
    const h = harness();
    const results = await h.sendAll([leadRequest(), leadRequest(), leadRequest()]);
    expect(results.map((r) => r.status)).toEqual([202, 202, 202]);
    expect(await leadRows()).toHaveLength(3);
    expect(h.mailed).toHaveLength(1);
  });

  it('writes the lead before it posts to Resend', async () => {
    const seen = [];
    const pass = resendAnswer();
    const h = harness({
      resend: async (attempt) => {
        seen.push((await leadRows()).length);
        return pass(attempt);
      },
    });
    await h.send(leadRequest());
    expect(seen).toEqual([1]);
  });

  it.each([400, 401, 403, 409, 422, 429])(
    'keeps the lead and answers 202 when Resend answers %i, frees the claim, does not retry, and logs only the status',
    async (status) => {
      const echo = { statusCode: status, name: 'validation_error', message: `buyer@example.com cannot reach ${SALES_INBOX}` };
      const h = harness({ resend: resendAnswer({ status, body: echo }) });
      const { result: r, lines } = await capturingLogs(() => h.send(leadRequest()));
      expect(r.status).toBe(202);
      expect(r.body).toEqual({ ok: true });
      const [row] = await leadRows();
      expect(row.notified_at).toBeNull();
      expect(h.mailed).toHaveLength(1);
      expect(failedNotify(lines)).toEqual([{ event: 'notify_failed', lead: row.id, status }]);
      const logged = JSON.stringify(lines);
      for (const leak of ['buyer@example.com', SALES_INBOX, 'validation_error', 'cannot reach']) {
        expect(logged).not.toContain(leak);
      }
    },
  );

  it('emails about the next lead for the address after a failed send', async () => {
    const fail = resendAnswer({ status: 422 });
    const pass = resendAnswer();
    const h = harness({ resend: (attempt) => (attempt === 1 ? fail(attempt) : pass(attempt)) });
    await capturingLogs(() => h.send(leadRequest()));
    await h.send(leadRequest());
    expect(h.mailed).toHaveLength(2);
    expect(h.sent).toHaveLength(1);
    const rows = await leadRows();
    expect(rows.map((row) => row.notified_at === null)).toEqual([true, false]);
  });

  it('retries once after a Resend outage, with the same idempotency key, and keeps the claim', async () => {
    const pass = resendAnswer();
    const h = harness({ resend: (attempt) => (attempt === 1 ? resendAnswer({ status: 503 })(attempt) : pass(attempt)) });
    await capturingLogs(() => h.send(leadRequest()));
    expect(h.mailed).toHaveLength(2);
    expect(h.mailed[1].idempotencyKey).toBe(h.mailed[0].idempotencyKey);
    expect(h.mailed[1].body).toEqual(h.mailed[0].body);
    expect(h.sent).toHaveLength(1);
    expect((await leadRows())[0].notified_at).toMatch(ISO);
  });

  it('retries once after a network error', async () => {
    const pass = resendAnswer();
    const h = harness({
      resend: (attempt) => {
        if (attempt === 1) throw new TypeError('network connection lost');
        return pass(attempt);
      },
    });
    await capturingLogs(() => h.send(leadRequest()));
    expect(h.mailed).toHaveLength(2);
    expect(h.sent).toHaveLength(1);
    expect((await leadRows())[0].notified_at).toMatch(ISO);
  });

  it('frees the claim after two failed tries, and logs the last status', async () => {
    const h = harness({ resend: resendAnswer({ status: 500 }) });
    const { lines } = await capturingLogs(() => h.send(leadRequest()));
    const [row] = await leadRows();
    expect(h.mailed).toHaveLength(2);
    expect(row.notified_at).toBeNull();
    expect(failedNotify(lines)).toEqual([{ event: 'notify_failed', lead: row.id, status: 500 }]);
  });

  it('frees the claim when the network fails twice, and logs the error name', async () => {
    const h = harness({
      resend: () => {
        throw new TypeError('connect ECONNREFUSED buyer@example.com');
      },
    });
    const { lines } = await capturingLogs(() => h.send(leadRequest()));
    const [row] = await leadRows();
    expect(h.mailed).toHaveLength(2);
    expect(row.notified_at).toBeNull();
    expect(failedNotify(lines)).toEqual([{ event: 'notify_failed', lead: row.id, error: 'TypeError' }]);
    expect(JSON.stringify(lines)).not.toContain('ECONNREFUSED');
  });

  it.each([
    ['missing', undefined],
    ['empty', ''],
  ])('stores the lead and answers 202 when the Resend key is %s, without calling Resend', async (_, key) => {
    const h = harness({ env: { RESEND_API_KEY: key } });
    const { result: r, lines } = await capturingLogs(() => h.send(leadRequest()));
    expect(r.status).toBe(202);
    const [row] = await leadRows();
    expect(row.notified_at).toBeNull();
    expect(h.mailed).toEqual([]);
    const events = lines.map((l) => JSON.parse(l.args[0]));
    expect(events.filter((e) => e.event === 'resend_key_missing')).toEqual([{ event: 'resend_key_missing', lead: row.id }]);
    expect(lines.find((l) => JSON.parse(l.args[0]).event === 'resend_key_missing').level).toBe('error');
  });

  it.each([
    ['NOTIFY_TO', { NOTIFY_TO: undefined }],
    ['NOTIFY_FROM', { NOTIFY_FROM: '' }],
  ])('stores the lead and sends nothing when %s is not configured', async (_, overrides) => {
    const h = harness({ env: overrides });
    const { result: r, lines } = await capturingLogs(() => h.send(leadRequest()));
    expect(r.status).toBe(202);
    expect(h.mailed).toEqual([]);
    expect((await leadRows())[0].notified_at).toBeNull();
    expect(lines.some((l) => JSON.parse(l.args[0]).event === 'notify_misconfigured')).toBe(true);
  });
});

describe('the notification can never reach a stranger', () => {
  const cases = [
    ...vectors.email.map((v) => ({ field: 'email', note: v.note, input: v.input })),
    ...vectors.phone.map((v) => ({ field: 'phone', note: v.note, input: v.input })),
  ];

  it.each(cases)('$field vector: $note', async ({ field, input }) => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ [field]: input }) }));
    expect(h.mailed).toHaveLength(r.status === 202 ? 1 : 0);
    for (const { body } of h.mailed) {
      expect(body.to).toEqual([SALES_INBOX]);
      expect(body).not.toHaveProperty('cc');
      expect(body).not.toHaveProperty('bcc');
      expect(typeof body.reply_to).toBe('string');
    }
  });

  it('ignores recipient fields smuggled into the request', async () => {
    const stranger = 'victim@example.org';
    const h = harness();
    const r = await h.send(
      leadRequest({
        body: validBody({ to: stranger, cc: stranger, bcc: [stranger], reply_to: stranger, from: stranger, NOTIFY_TO: stranger }),
      }),
    );
    expect(r.status).toBe(202);
    expect(h.mailed).toHaveLength(1);
    const [{ body }] = h.mailed;
    expect(body.to).toEqual([SALES_INBOX]);
    expect(body.reply_to).toBe('buyer@example.com');
    expect(body.from).toBe(`LOGICAFFEINE pricing form <${env.NOTIFY_FROM}>`);
    expect(JSON.stringify(body)).not.toContain(stranger);
  });
});

describe('privacy', () => {
  const IPS = [CLIENT_IP, '198.51.100.9', '198.51.100.10', '198.51.100.11'];
  const forwarded = {
    'X-Forwarded-For': `198.51.100.9, ${CLIENT_IP}`,
    'X-Real-IP': '198.51.100.10',
    'True-Client-IP': '198.51.100.11',
  };

  it('stores no IP address anywhere in the database', async () => {
    const h = harness();
    await h.send(leadRequest({ headers: forwarded, cf: { country: 'US' } }));
    await h.send(leadRequest({ ip: CLIENT_IPV6, body: validBody({ email: 'v6@example.com' }) }));
    const values = await everyStoredValue();
    expect(values.length).toBeGreaterThan(0);
    for (const ip of [...IPS, CLIENT_IPV6]) {
      expect(values.filter((v) => v.value.includes(ip))).toEqual([]);
    }
  });

  it('puts no IP address in the notification', async () => {
    const h = harness();
    await h.send(leadRequest({ headers: forwarded }));
    expect(h.sent).toHaveLength(1);
    const message = JSON.stringify(h.sent);
    for (const ip of IPS) expect(message).not.toContain(ip);
  });

  it('logs structured JSON events that carry no IP, email or phone', async () => {
    const h = harness();
    const { lines } = await capturingLogs(async () => {
      await h.send(leadRequest({ headers: forwarded }));
      await h.send(leadRequest({ body: validBody({ email: 'nope' }) }));
      await h.send(leadRequest({ body: validBody({ website: 'https://spam.example' }) }));
    });
    expect(lines.length).toBeGreaterThan(0);
    for (const { args } of lines) {
      expect(args).toHaveLength(1);
      const event = JSON.parse(args[0]);
      expect(typeof event.event).toBe('string');
      for (const secret of [...IPS, 'buyer@', 'example.com', '+1 (555) 123-4567', '15551234567', '123-4567', 'nope']) {
        expect(args[0]).not.toContain(secret);
      }
    }
  });
});

describe('the order of checks', () => {
  it('1. origin before the body cap', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ origin: 'https://evil.example', raw: ' '.repeat(9000) }));
    expect(r.status).toBe(403);
  });

  it('2. body cap before the honeypot', async () => {
    const h = harness();
    const raw = JSON.stringify(validBody({ website: 'x'.repeat(9000) }));
    const r = await h.send(leadRequest({ raw }));
    expect(r.status).toBe(413);
  });

  it('3. honeypot before validation', async () => {
    const h = harness();
    const r = await h.send(leadRequest({ body: validBody({ website: 'x', email: 'nope', phone: 'call me' }) }));
    expect(r.status).toBe(202);
    await expectUntouched(h);
  });

  it('4. validation before Turnstile', async () => {
    const h = harness({ siteverify: siteverifyAnswer({ success: false }) });
    const r = await h.send(leadRequest({ body: validBody({ email: 'nope', turnstile_token: '' }) }));
    expect(r.status).toBe(400);
    expect(h.verified).toEqual([]);
  });

  it('5. Turnstile before the rate limits', async () => {
    const h = harness({ siteverify: siteverifyAnswer({ success: false }), ipAllows: false, emailAllows: false });
    const r = await h.send(leadRequest());
    expect(r.status).toBe(403);
    expect(h.limited).toEqual({ ip: [], email: [] });
  });

  it('6. rate limits before storage', async () => {
    const h = harness({ ipAllows: false });
    const r = await h.send(leadRequest());
    expect(r.status).toBe(429);
    expect(await leadRows()).toEqual([]);
  });

  it('7-9. store, then notify, then 202', async () => {
    const h = harness();
    const r = await h.send(leadRequest());
    expect(h.order).toEqual(['siteverify', 'limit:ip', 'limit:email', 'notify']);
    expect(r.status).toBe(202);
    expect(r.body).toEqual({ ok: true });
  });
});

describe('every answer', () => {
  it.each([
    ['202', () => leadRequest()],
    ['400', () => leadRequest({ body: validBody({ email: 'nope' }) })],
    ['403', () => leadRequest({ origin: 'https://evil.example' })],
    ['404', () => leadRequest({ path: '/nope' })],
    ['405', () => leadRequest({ method: 'GET' })],
    ['413', () => leadRequest({ raw: ' '.repeat(9000) })],
    ['415', () => leadRequest({ contentType: 'text/plain' })],
  ])('is %s JSON that no cache keeps', async (status, request) => {
    const h = harness();
    const r = await h.send(request());
    expect(String(r.status)).toBe(status);
    expect(r.headers.get('Content-Type')).toBe('application/json; charset=utf-8');
    expect(r.headers.get('Cache-Control')).toBe('no-store');
    expect(r.headers.get('X-Content-Type-Options')).toBe('nosniff');
    expect(r.body).toHaveProperty('ok');
  });
});

describe('an unexpected failure', () => {
  it('answers 500 JSON the page can read, logs it, and keeps nothing, when the body cannot be read', async () => {
    const h = harness();
    const stream = new ReadableStream({
      pull(controller) {
        controller.error(new Error('client went away'));
      },
    });
    const { result: r, lines } = await capturingLogs(() => h.send(leadRequest({ raw: stream })));
    expect(r.status).toBe(500);
    expect(r.body).toEqual({ ok: false, error: 'internal' });
    expect(r.headers.get('Access-Control-Allow-Origin')).toBe(SITE);
    expect(r.headers.get('Content-Type')).toBe('application/json; charset=utf-8');
    const event = lines.map((l) => JSON.parse(l.args[0])).find((l) => l.event === 'unhandled');
    expect(event).toEqual({ event: 'unhandled', error: expect.any(String) });
    expect(JSON.stringify(lines)).not.toContain('client went away');
    await expectUntouched(h);
  });

  it('logs a failed retention sweep and lets the cron run fail', async () => {
    const broken = {
      prepare() {
        throw new Error('D1 unavailable');
      },
    };
    const h = harness({ env: { DB: broken } });
    const { result, lines } = await capturingLogs(() =>
      h.runScheduled(Date.parse('2030-06-15T03:17:00.000Z')).then(
        () => 'resolved',
        (error) => error.message,
      ),
    );
    expect(result).toBe('D1 unavailable');
    const event = lines.map((l) => JSON.parse(l.args[0])).find((l) => l.event === 'retention_failed');
    expect(event).toEqual({ event: 'retention_failed', error: 'Error' });
  });
});

describe('retention', () => {
  const RUN = Date.parse('2030-06-15T03:17:00.000Z');

  it('deletes leads older than 24 months and keeps the rest', async () => {
    await insertLead({ email: 'old@example.com', createdAt: '2028-06-15T03:16:59.999Z' });
    await insertLead({ email: 'older@example.com', createdAt: '2027-01-01T00:00:00.000Z' });
    await insertLead({ email: 'edge@example.com', createdAt: '2028-06-15T03:17:00.000Z' });
    await insertLead({ email: 'recent@example.com', createdAt: '2030-06-01T00:00:00.000Z' });
    const h = harness();
    const { lines } = await capturingLogs(() => h.runScheduled(RUN));
    expect((await leadRows()).map((row) => row.email)).toEqual(['edge@example.com', 'recent@example.com']);
    const event = lines.map((l) => JSON.parse(l.args[0])).find((l) => l.event === 'retention');
    expect(event).toEqual({ event: 'retention', deleted: 2, cutoff: '2028-06-15T03:17:00.000Z' });
  });

  it('counts 24 months back from a leap day to the day after the date that does not exist', async () => {
    await insertLead({ email: 'drop@example.com', createdAt: '2030-03-01T03:16:59.999Z' });
    await insertLead({ email: 'keep@example.com', createdAt: '2030-03-01T03:17:00.000Z' });
    const h = harness();
    const { lines } = await capturingLogs(() => h.runScheduled(Date.parse('2032-02-29T03:17:00.000Z')));
    expect((await leadRows()).map((row) => row.email)).toEqual(['keep@example.com']);
    const event = lines.map((l) => JSON.parse(l.args[0])).find((l) => l.event === 'retention');
    expect(event.cutoff).toBe('2030-03-01T03:17:00.000Z');
  });

  it('deletes a lead 24 months and a day old and keeps one 24 months less a day old', async () => {
    await insertLead({ email: 'plus-a-day@example.com', createdAt: '2028-06-14T03:17:00.000Z' });
    await insertLead({ email: 'less-a-day@example.com', createdAt: '2028-06-16T03:17:00.000Z' });
    const h = harness();
    await h.runScheduled(RUN);
    expect((await leadRows()).map((row) => row.email)).toEqual(['less-a-day@example.com']);
  });

  it('deletes nothing when nothing is old', async () => {
    await insertLead({ email: 'recent@example.com', createdAt: '2030-06-01T00:00:00.000Z' });
    const h = harness();
    const { lines } = await capturingLogs(() => h.runScheduled(RUN));
    expect(await leadRows()).toHaveLength(1);
    expect(lines.map((l) => JSON.parse(l.args[0])).find((l) => l.event === 'retention').deleted).toBe(0);
  });
});
