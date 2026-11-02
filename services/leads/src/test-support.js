import { createExecutionContext, waitOnExecutionContext } from 'cloudflare:test';
import { env } from 'cloudflare:workers';
import { inject, vi } from 'vitest';
import worker from './index.js';

export const SITE = 'https://logicaffeine.com';
export const SITE_HOST = 'logicaffeine.com';
export const SITEVERIFY = 'https://challenges.cloudflare.com/turnstile/v0/siteverify';
export const RESEND = 'https://api.resend.com/emails';
export const CLIENT_IP = '203.0.113.7';
export const CLIENT_IPV6 = '2001:db8::7';
export const TURNSTILE_ACTION = 'pricing-lead';
export const SALES_INBOX = 'team@datacenter.dev';

export function validBody(overrides = {}) {
  return {
    email: 'buyer@example.com',
    phone: '+1 (555) 123-4567',
    turnstile_token: 'turnstile-token-ok',
    source: 'pricing',
    website: '',
    ...overrides,
  };
}

/**
 * A request as a browser on the pricing page sends it. `null` for `origin`, `ip` or `contentType`
 * leaves that header out; `raw` replaces the JSON body with a string, bytes or a stream.
 */
export function leadRequest({
  body = validBody(),
  raw,
  origin = SITE,
  ip = CLIENT_IP,
  method = 'POST',
  path = '/lead',
  contentType = 'application/json',
  headers = {},
  cf,
} = {}) {
  const h = new Headers(headers);
  if (origin !== null) h.set('Origin', origin);
  if (ip !== null) h.set('CF-Connecting-IP', ip);
  const init = { method, headers: h };
  if (method === 'POST' || raw !== undefined) {
    if (contentType !== null) h.set('Content-Type', contentType);
    init.body = raw !== undefined ? raw : JSON.stringify(body);
    if (init.body instanceof ReadableStream) init.duplex = 'half';
  }
  if (cf) init.cf = cf;
  return new Request(`https://api.logicaffeine.com${path}`, init);
}

export function preflight({ origin = SITE, method = 'POST', path = '/lead', requestHeaders = 'content-type' } = {}) {
  const h = new Headers();
  if (origin !== null) h.set('Origin', origin);
  if (method !== null) h.set('Access-Control-Request-Method', method);
  if (requestHeaders !== null) h.set('Access-Control-Request-Headers', requestHeaders);
  return new Request(`https://api.logicaffeine.com${path}`, { method: 'OPTIONS', headers: h });
}

/** A siteverify answer, as `Response`, for the stubbed endpoint. */
export function siteverifyAnswer({
  success = true,
  action = TURNSTILE_ACTION,
  hostname = SITE_HOST,
  errorCodes = [],
  status = 200,
} = {}) {
  return () =>
    new Response(
      JSON.stringify({
        success,
        challenge_ts: new Date().toISOString(),
        hostname,
        'error-codes': errorCodes,
        action,
        cdata: '',
      }),
      { status, headers: { 'Content-Type': 'application/json' } },
    );
}

/** A Resend answer, as `Response`, for the stubbed endpoint: the email's id, or Resend's error body. */
export function resendAnswer({ status = 200, body } = {}) {
  return (attempt) =>
    new Response(
      JSON.stringify(
        body ?? (status < 300 ? { id: `email-${attempt}` } : { statusCode: status, name: 'stubbed_error', message: 'stubbed' }),
      ),
      { status, headers: { 'Content-Type': 'application/json' } },
    );
}

/**
 * The worker with every outside party replaced by a recorder: siteverify and Resend (both outbound
 * fetches) and both rate limiters. D1 and the vars are the real ones from wrangler.toml.
 *
 * `mailed` holds every request made to Resend; `sent` holds the bodies Resend accepted.
 */
export function harness({
  siteverify = siteverifyAnswer(),
  resend = resendAnswer(),
  ipAllows = true,
  emailAllows = true,
  env: overrides = {},
} = {}) {
  const sent = [];
  const mailed = [];
  const limited = { ip: [], email: [] };
  const verified = [];
  const order = [];

  const limiter = (kind, allows) => ({
    limit: vi.fn(async ({ key }) => {
      order.push(`limit:${kind}`);
      limited[kind].push(key);
      return { success: typeof allows === 'function' ? allows(key) : allows };
    }),
  });

  vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
    const request = new Request(input, init);
    const hasSignal = init?.signal instanceof AbortSignal;
    if (request.url === SITEVERIFY) {
      order.push('siteverify');
      const text = await request.text();
      verified.push({
        method: request.method,
        contentType: request.headers.get('Content-Type'),
        form: Object.fromEntries(new URLSearchParams(text)),
        hasSignal,
      });
      return siteverify(verified.length, verified.at(-1));
    }
    if (request.url === RESEND) {
      order.push('notify');
      const attempt = {
        method: request.method,
        authorization: request.headers.get('Authorization'),
        contentType: request.headers.get('Content-Type'),
        idempotencyKey: request.headers.get('Idempotency-Key'),
        hasSignal,
        body: JSON.parse(await request.text()),
      };
      mailed.push(attempt);
      const response = await resend(mailed.length, attempt);
      if (response.ok) sent.push(attempt.body);
      return response;
    }
    throw new Error(`unexpected outbound fetch to ${request.url}`);
  });

  const testEnv = {
    DB: env.DB,
    ALLOWED_ORIGINS: env.ALLOWED_ORIGINS,
    NOTIFY_TO: env.NOTIFY_TO,
    NOTIFY_FROM: env.NOTIFY_FROM,
    TURNSTILE_SECRET: inject('turnstileSecret'),
    RESEND_API_KEY: inject('resendKey'),
    LEAD_IP_LIMIT: limiter('ip', ipAllows),
    LEAD_EMAIL_LIMIT: limiter('email', emailAllows),
    ...overrides,
  };

  async function read(response) {
    const text = await response.text();
    let body = null;
    if (text) {
      try {
        body = JSON.parse(text);
      } catch {
        body = { unparsable: text };
      }
    }
    return { status: response.status, headers: response.headers, body };
  }

  async function send(request) {
    const ctx = createExecutionContext();
    const response = await worker.fetch(request, testEnv, ctx);
    await waitOnExecutionContext(ctx);
    return read(response);
  }

  async function sendAll(requests) {
    const ctx = createExecutionContext();
    const responses = await Promise.all(requests.map((r) => worker.fetch(r, testEnv, ctx)));
    await waitOnExecutionContext(ctx);
    return Promise.all(responses.map(read));
  }

  async function runScheduled(scheduledTime) {
    const { createScheduledController } = await import('cloudflare:test');
    const ctx = createExecutionContext();
    await worker.scheduled(createScheduledController({ scheduledTime, cron: '17 3 * * *' }), testEnv, ctx);
    await waitOnExecutionContext(ctx);
  }

  return { env: testEnv, sent, mailed, limited, verified, order, send, sendAll, runScheduled };
}

export async function leadRows() {
  const { results } = await env.DB.prepare('SELECT * FROM leads ORDER BY id').all();
  return results;
}

export async function insertLead({ email, createdAt, notifiedAt = null }) {
  await env.DB.prepare('INSERT INTO leads (email, created_at, notified_at) VALUES (?1, ?2, ?3)')
    .bind(email, createdAt, notifiedAt)
    .run();
}

export function minutesAgo(minutes, seconds = 0) {
  return new Date(Date.now() - (minutes * 60 + seconds) * 1000).toISOString();
}

/** Every value in every row of every table the database holds, as text. */
export async function everyStoredValue() {
  const { results: tables } = await env.DB.prepare(
    "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE '_cf_%'",
  ).all();
  const values = [];
  for (const { name } of tables) {
    const { results } = await env.DB.prepare(`SELECT * FROM "${name}"`).all();
    for (const row of results) {
      for (const [column, value] of Object.entries(row)) values.push({ table: name, column, value: String(value) });
    }
  }
  return values;
}

/** Captures every console call made while `fn` runs. */
export async function capturingLogs(fn) {
  const lines = [];
  const spies = ['log', 'info', 'warn', 'error', 'debug'].map((level) =>
    vi.spyOn(console, level).mockImplementation((...args) => lines.push({ level, args })),
  );
  try {
    return { result: await fn(), lines };
  } finally {
    for (const spy of spies) spy.mockRestore();
  }
}
