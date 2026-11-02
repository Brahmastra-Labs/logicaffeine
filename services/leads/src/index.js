import { trimField, validateEmail, validatePhone, validateSource } from './lead.js';

const LEAD_PATH = '/lead';
const MAX_BODY_BYTES = 8192;
const MAX_TOKEN_LENGTH = 2048;
const SITEVERIFY_URL = 'https://challenges.cloudflare.com/turnstile/v0/siteverify';
const SITEVERIFY_TIMEOUT_MS = 10_000;
const SITEVERIFY_ATTEMPTS = 2;
const TURNSTILE_ACTION = 'pricing-lead';
const NOTIFY_WINDOW_MS = 10 * 60 * 1000;
const RESEND_URL = 'https://api.resend.com/emails';
const RESEND_TIMEOUT_MS = 10_000;
const RESEND_ATTEMPTS = 2;
const RETENTION_MONTHS = 24;
const RETRY_AFTER_SECONDS = '60';
const SENDER_NAME = 'LOGICAFFEINE pricing form';
const DNS_LABEL = '[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?';
const COUNTRY = /^[A-Z0-9]{2}$/;

export default {
  async fetch(request, env, ctx) {
    const origin = request.headers.get('Origin');
    const grant = isAllowedOrigin(origin, env.ALLOWED_ORIGINS) ? origin : null;

    try {
      if (new URL(request.url).pathname !== LEAD_PATH) return answer(404, { ok: false, error: 'not_found' }, grant);
      if (request.method === 'OPTIONS') {
        return grant ? preflightGrant(grant) : answer(403, { ok: false, error: 'origin_not_allowed' }, null);
      }
      if (request.method !== 'POST') {
        return answer(405, { ok: false, error: 'method_not_allowed' }, grant, { Allow: 'POST, OPTIONS' });
      }
      return await acceptLead(request, env, ctx, grant);
    } catch (error) {
      // Only the error's name: a message can quote whatever the request carried.
      logError('unhandled', { error: error.name });
      return answer(500, { ok: false, error: 'internal' }, grant);
    }
  },

  async scheduled(controller, env) {
    const cutoff = new Date(controller.scheduledTime);
    cutoff.setUTCMonth(cutoff.getUTCMonth() - RETENTION_MONTHS);
    try {
      const { meta } = await env.DB.prepare('DELETE FROM leads WHERE created_at < ?1').bind(cutoff.toISOString()).run();
      log('retention', { deleted: meta.changes, cutoff: cutoff.toISOString() });
    } catch (error) {
      logError('retention_failed', { error: error.name });
      throw error;
    }
  },
};

/** The checks run in the plan's order; each one that fails answers before the next runs. */
async function acceptLead(request, env, ctx, grant) {
  const refuse = (status, error, detail = {}, headers = {}) => {
    log('lead_rejected', { status, error, ...detail });
    return answer(status, { ok: false, error, ...detail }, grant, headers);
  };

  if (!grant) return refuse(403, 'origin_not_allowed');

  const bytes = await readCapped(request, MAX_BODY_BYTES);
  if (bytes === null) return refuse(413, 'body_too_large');
  if (!isJson(request.headers.get('Content-Type'))) return refuse(415, 'unsupported_media_type');
  const body = parseObject(bytes);
  if (body === null) return refuse(400, 'invalid_json');

  if (honeypotFilled(body.website)) {
    log('honeypot');
    return answer(202, { ok: true }, grant);
  }

  const email = validateEmail(body.email);
  if (!email.valid) return refuse(400, email.error, { reason: email.reason });
  const phone = validatePhone(body.phone);
  if (!phone.valid) return refuse(400, phone.error, { reason: phone.reason });
  const source = validateSource(body.source);
  if (!source.valid) return refuse(400, source.error);

  if (!env.TURNSTILE_SECRET) {
    logError('turnstile_secret_missing');
    return answer(500, { ok: false, error: 'internal' }, grant);
  }
  const token = body.turnstile_token;
  if (typeof token !== 'string' || token.length === 0 || token.length > MAX_TOKEN_LENGTH) {
    return refuse(403, 'turnstile_failed');
  }
  const clientIp = request.headers.get('CF-Connecting-IP');
  if (!(await passesTurnstile(env.TURNSTILE_SECRET, token, clientIp, new URL(grant).hostname))) {
    return refuse(403, 'turnstile_failed');
  }

  const withinLimits =
    (await withinLimit(env.LEAD_IP_LIMIT, `ip:${clientIp ?? 'unknown'}`, 'ip')) &&
    (await withinLimit(env.LEAD_EMAIL_LIMIT, `email:${email.normalized.toLowerCase()}`, 'email'));
  if (!withinLimits) return refuse(429, 'rate_limited', {}, { 'Retry-After': RETRY_AFTER_SECONDS });

  const lead = {
    email: email.normalized,
    phone: phone.normalized,
    digits: phone.digits,
    source: source.source,
    country: countryOf(request),
    createdAt: new Date().toISOString(),
  };
  try {
    const row = await env.DB.prepare(
      `INSERT INTO leads (email, phone, phone_digits, source, country, created_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING id`,
    )
      .bind(lead.email, lead.phone, lead.digits, lead.source, lead.country, lead.createdAt)
      .first();
    lead.id = row.id;
  } catch (error) {
    logError('store_failed', { error: error.message });
    return answer(500, { ok: false, error: 'internal' }, grant);
  }
  log('lead_stored', { lead: lead.id });

  ctx.waitUntil(notify(env, lead));
  return answer(202, { ok: true }, grant);
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

/** The honeypot is a text field people never see; any value in it but blank text marks a bot. */
function honeypotFilled(website) {
  if (website === undefined || website === null) return false;
  if (typeof website !== 'string') return true;
  return trimField(website) !== '';
}

/**
 * Siteverify, retried once on an outage with the same idempotency key. Fails closed: a token passes
 * only when siteverify says so for this action on the page that sent the lead.
 */
async function passesTurnstile(secret, token, clientIp, hostname) {
  const form = new URLSearchParams({ secret, response: token, idempotency_key: crypto.randomUUID() });
  if (clientIp) form.set('remoteip', clientIp);

  for (let attempt = 1; attempt <= SITEVERIFY_ATTEMPTS; attempt++) {
    let response;
    try {
      response = await fetch(SITEVERIFY_URL, {
        method: 'POST',
        body: form,
        signal: AbortSignal.timeout(SITEVERIFY_TIMEOUT_MS),
      });
    } catch (error) {
      logError('turnstile_unavailable', { attempt, error: error.name });
      continue;
    }
    if (response.status >= 500) {
      await response.body?.cancel();
      logError('turnstile_unavailable', { attempt, status: response.status });
      continue;
    }
    if (!response.ok) {
      await response.body?.cancel();
      logError('turnstile_unusable', { status: response.status });
      return false;
    }
    let result;
    try {
      result = await response.json();
    } catch {
      logError('turnstile_unusable', { status: response.status });
      return false;
    }
    const passed = result.success === true && result.action === TURNSTILE_ACTION && result.hostname === hostname;
    if (!passed) {
      log('turnstile_refused', {
        codes: Array.isArray(result['error-codes']) ? result['error-codes'] : [],
        action_matches: result.action === TURNSTILE_ACTION,
        hostname_matches: result.hostname === hostname,
      });
    }
    return passed;
  }
  return false;
}

/** A limiter that fails lets the lead through: Turnstile has already vouched for the sender. */
async function withinLimit(limiter, key, name) {
  try {
    const { success } = await limiter.limit({ key });
    return success;
  } catch (error) {
    logError('rate_limiter_failed', { limiter: name, error: error.name });
    return true;
  }
}

function countryOf(request) {
  const country = request.cf?.country;
  return typeof country === 'string' && COUNTRY.test(country) ? country : null;
}

/**
 * Emails the sales inbox about a stored lead, at most once per address per 10 minutes. The claim is
 * one statement, so leads that arrive together cannot both win it; a send that fails gives it back.
 */
async function notify(env, lead) {
  if (!env.RESEND_API_KEY) {
    logError('resend_key_missing', { lead: lead.id });
    return;
  }
  if (!env.NOTIFY_TO || !env.NOTIFY_FROM) {
    logError('notify_misconfigured', { lead: lead.id });
    return;
  }

  const now = Date.now();
  try {
    const { meta } = await env.DB.prepare(
      `UPDATE leads SET notified_at = ?1
       WHERE id = ?2
         AND NOT EXISTS (SELECT 1 FROM leads WHERE email = ?3 COLLATE NOCASE AND notified_at > ?4)`,
    )
      .bind(new Date(now).toISOString(), lead.id, lead.email, new Date(now - NOTIFY_WINDOW_MS).toISOString())
      .run();
    if (meta.changes !== 1) {
      log('notify_skipped', { lead: lead.id });
      return;
    }
  } catch (error) {
    logError('notify_failed', { lead: lead.id, stage: 'claim', error: error.name });
    return;
  }

  const outcome = await sendThroughResend(env, lead);
  if (outcome.sent) {
    log('lead_notified', { lead: lead.id, ...(outcome.id ? { email_id: outcome.id } : {}) });
    return;
  }
  logError('notify_failed', { lead: lead.id, ...outcome.failure });
  try {
    await env.DB.prepare('UPDATE leads SET notified_at = NULL WHERE id = ?1').bind(lead.id).run();
  } catch (release) {
    logError('notify_release_failed', { lead: lead.id, error: release.name });
  }
}

/**
 * Posts the notification to Resend, retried once on an outage under the same idempotency key, so a
 * retry can never mail twice. A failure is described by its status or its error's name alone: Resend's
 * answer and an error's message can quote the addresses.
 */
async function sendThroughResend(env, lead) {
  const init = {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${env.RESEND_API_KEY}`,
      'Content-Type': 'application/json',
      'Idempotency-Key': `pricing-lead/${lead.id}/${lead.createdAt}`,
    },
    body: JSON.stringify(notification(env, lead)),
  };

  let failure = null;
  for (let attempt = 1; attempt <= RESEND_ATTEMPTS; attempt++) {
    let response;
    try {
      response = await fetch(RESEND_URL, { ...init, signal: AbortSignal.timeout(RESEND_TIMEOUT_MS) });
    } catch (error) {
      failure = { error: error.name };
      logError('resend_unavailable', { lead: lead.id, attempt, ...failure });
      continue;
    }
    if (response.ok) {
      const answer = await response.json().catch(() => null);
      return { sent: true, id: typeof answer?.id === 'string' ? answer.id : null };
    }
    await response.body?.cancel();
    failure = { status: response.status };
    if (response.status < 500) break;
    logError('resend_unavailable', { lead: lead.id, attempt, ...failure });
  }
  return { sent: false, failure };
}

/** The only recipient is the sales inbox; the prospect appears only as the address replies go to. */
function notification(env, lead) {
  const fields = [
    ['Email', lead.email],
    ['Phone', lead.phone ?? 'not given'],
    ['Source', lead.source ?? 'not given'],
    ['Country', lead.country ?? 'unknown'],
    ['Received', lead.createdAt],
    ['Lead', `#${lead.id}`],
  ];
  const intro = 'A pricing request came in through the pricing page.';
  const outro = 'Reply to this email to answer the prospect.';
  const text = [intro, '', ...fields.map(([name, value]) => `${`${name}:`.padEnd(10)}${value}`), '', outro, ''].join(
    '\n',
  );
  const rows = fields
    .map(([name, value]) => `<tr><th align="left">${name}</th><td>${escapeHtml(value)}</td></tr>`)
    .join('');
  const html = `<p>${intro}</p><table>${rows}</table><p>${outro}</p>`;
  return {
    from: `${SENDER_NAME} <${env.NOTIFY_FROM}>`,
    to: [env.NOTIFY_TO],
    reply_to: lead.email,
    subject: `Pricing request \u2014 ${lead.email}`,
    text,
    html,
  };
}

function escapeHtml(value) {
  return String(value).replace(
    /[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c],
  );
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
