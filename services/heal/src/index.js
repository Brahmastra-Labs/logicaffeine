import { healedFrom, promptFor, validateRequest } from './heal.js';

const HEAL_PATH = '/heal';
const MAX_BODY_BYTES = 65_536;
const RETRY_AFTER_SECONDS = '60';
const TEMPERATURE = 0.2;
const MAX_TOKENS = 4096;
const DNS_LABEL = '[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?';

/**
 * The heal worker: the Studio's Heal button posts a mode and a text here; the worker asks the model
 * behind `UPSTREAM_URL` for the mended text and answers with it. The key lives in the secret
 * `NVIDIA_API_KEY` and reaches nothing but the upstream's Authorization header; no request text is
 * ever logged, only the event, the mode and the sizes.
 */
export default {
  async fetch(request, env, ctx) {
    const origin = request.headers.get('Origin');
    const grant = isAllowedOrigin(origin, env.ALLOWED_ORIGINS) ? origin : null;
    try {
      if (new URL(request.url).pathname !== HEAL_PATH) return answer(404, { ok: false, error: 'not_found' }, grant);
      if (request.method === 'OPTIONS') {
        return grant ? preflightGrant(grant) : answer(403, { ok: false, error: 'origin_not_allowed' }, null);
      }
      if (request.method !== 'POST') {
        return answer(405, { ok: false, error: 'method_not_allowed' }, grant, { Allow: 'POST, OPTIONS' });
      }
      return await heal(request, env, grant);
    } catch (error) {
      // Only the error's name: a message can quote whatever the request carried.
      logError('unhandled', { error: error.name });
      return answer(500, { ok: false, error: 'internal' }, grant);
    }
  },
};

/** The checks run in order; each one that fails answers before the next runs. */
async function heal(request, env, grant) {
  const refuse = (status, error, headers = {}) => {
    log('heal_refused', { status, error });
    return answer(status, { ok: false, error }, grant, headers);
  };

  if (!grant) return refuse(403, 'origin_not_allowed');

  const bytes = await readCapped(request, MAX_BODY_BYTES);
  if (bytes === null) return refuse(413, 'body_too_large');
  if (!isJson(request.headers.get('Content-Type'))) return refuse(415, 'unsupported_media_type');
  const body = parseObject(bytes);
  if (body === null) return refuse(400, 'invalid_json');

  const asked = validateRequest(body, Number(env.MAX_TEXT_BYTES) || 8192);
  if (!asked.valid) return refuse(400, asked.error);

  if (!env.NVIDIA_API_KEY) {
    logError('key_missing');
    return answer(500, { ok: false, error: 'internal' }, grant);
  }

  const clientIp = request.headers.get('CF-Connecting-IP');
  if (!(await withinLimit(env.HEAL_IP_LIMIT, `ip:${clientIp ?? 'unknown'}`))) {
    return refuse(429, 'rate_limited', { 'Retry-After': RETRY_AFTER_SECONDS });
  }

  const { system, user } = promptFor(asked.mode, asked.text, asked.fragment);
  const upstream = await askUpstream(env, system, user);
  if (!upstream.ok) {
    logError('upstream_failed', { status: upstream.status ?? null, error: upstream.error });
    return answer(502, { ok: false, error: 'upstream_failed' }, grant);
  }
  const read = healedFrom(upstream.body);
  if (!read.ok) {
    logError('upstream_unusable', { error: read.error });
    return answer(502, { ok: false, error: read.error }, grant);
  }
  log('healed', { mode: asked.mode, bytes_in: bytes.byteLength, bytes_out: read.healed.length, model: env.MODEL });
  return answer(200, { ok: true, healed: read.healed, model: env.MODEL }, grant);
}

/** One chat completion, non-streaming, within the configured time. Fails closed on any surprise. */
async function askUpstream(env, system, user) {
  const timeout = Number(env.UPSTREAM_TIMEOUT_MS) || 25_000;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);
  try {
    const response = await fetch(env.UPSTREAM_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json', Authorization: `Bearer ${env.NVIDIA_API_KEY}` },
      body: JSON.stringify({
        model: env.MODEL,
        messages: [
          { role: 'system', content: system },
          { role: 'user', content: user },
        ],
        temperature: TEMPERATURE,
        max_tokens: MAX_TOKENS,
        stream: false,
      }),
      signal: controller.signal,
    });
    if (!response.ok) return { ok: false, status: response.status, error: 'status' };
    let body;
    try {
      body = await response.json();
    } catch {
      return { ok: false, status: response.status, error: 'not_json' };
    }
    return { ok: true, body };
  } catch (error) {
    return { ok: false, error: error.name };
  } finally {
    clearTimeout(timer);
  }
}

/**
 * An allowed origin is either listed exactly, or matches a `scheme://*.rest` entry as an anchored
 * pattern: that scheme, one DNS label in place of the star, then exactly the rest.
 */
function isAllowedOrigin(origin, list) {
  if (!origin) return false;
  return String(list ?? '')
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean)
    .some((entry) => {
      const star = entry.indexOf('://*.');
      if (star < 0) return origin === entry;
      const pattern = `^${escapeRegExp(entry.slice(0, star + 3))}${DNS_LABEL}${escapeRegExp(entry.slice(star + 4))}$`;
      return new RegExp(pattern).test(origin);
    });
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&');
}

/** The body's bytes, or `null` once it runs past `max`, whether its length is declared or streamed. */
async function readCapped(request, max) {
  const declared = request.headers.get('Content-Length');
  if (declared !== null && Number(declared) > max) return null;
  if (!request.body) return new Uint8Array(0);
  const reader = request.body.getReader();
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > max) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

function isJson(contentType) {
  return (contentType ?? '').split(';')[0].trim().toLowerCase() === 'application/json';
}

/** The body as a JSON object, or `null` when it is not UTF-8, not JSON, or not an object. */
function parseObject(bytes) {
  try {
    const value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
    return value !== null && typeof value === 'object' && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

/** Fails open on a broken limiter, as the lead worker does: a heal lost to an outage is the worse error. */
async function withinLimit(limiter, key) {
  try {
    const { success } = await limiter.limit({ key });
    return success;
  } catch (error) {
    logError('rate_limiter_failed', { error: error.name });
    return true;
  }
}

function answer(status, body, grant, headers = {}) {
  const h = new Headers({
    'Content-Type': 'application/json; charset=utf-8',
    'Cache-Control': 'no-store',
    'X-Content-Type-Options': 'nosniff',
    Vary: 'Origin',
    ...headers,
  });
  if (grant) h.set('Access-Control-Allow-Origin', grant);
  return new Response(JSON.stringify(body), { status, headers: h });
}

function preflightGrant(origin) {
  return new Response(null, {
    status: 204,
    headers: {
      'Access-Control-Allow-Origin': origin,
      'Access-Control-Allow-Methods': 'POST, OPTIONS',
      'Access-Control-Allow-Headers': 'Content-Type',
      'Access-Control-Max-Age': '86400',
      Vary: 'Origin',
    },
  });
}

function log(event, fields = {}) {
  console.log(JSON.stringify({ event, ...fields }));
}

function logError(event, fields = {}) {
  console.error(JSON.stringify({ event, ...fields }));
}
