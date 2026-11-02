import { createExecutionContext, waitOnExecutionContext } from 'cloudflare:test';
import { env } from 'cloudflare:workers';
import { vi } from 'vitest';
import worker from './index.js';

export const SITE = 'https://logicaffeine.com';
export const CLIENT_IP = '203.0.113.7';
export const UPSTREAM = 'https://integrate.api.nvidia.com/v1/chat/completions';

export const TYPO_TEXT = '## Main\nLet score be 85.\nIf score is atleast 90:\n    Show "Grade: A".\n';
export const HEALED_TEXT = '## Main\nLet score be 85.\nIf score is at least 90:\n    Show "Grade: A".\n';

export function validBody(overrides = {}) {
  return { mode: 'code', text: TYPO_TEXT, ...overrides };
}

/** A request as the Studio sends it. `null` for `origin`, `ip` or `contentType` leaves that header out. */
export function healRequest({ body = validBody(), raw, origin = SITE, ip = CLIENT_IP, method = 'POST', path = '/heal', contentType = 'application/json', headers = {} } = {}) {
  const h = new Headers(headers);
  if (origin !== null) h.set('Origin', origin);
  if (ip !== null) h.set('CF-Connecting-IP', ip);
  const init = { method, headers: h };
  if (method === 'POST' || raw !== undefined) {
    if (contentType !== null) h.set('Content-Type', contentType);
    init.body = raw !== undefined ? raw : JSON.stringify(body);
    if (init.body instanceof ReadableStream) init.duplex = 'half';
  }
  return new Request(`https://api.logicaffeine.com${path}`, init);
}

export function preflight({ origin = SITE, method = 'POST', path = '/heal', requestHeaders = 'content-type' } = {}) {
  const h = new Headers();
  if (origin !== null) h.set('Origin', origin);
  if (method !== null) h.set('Access-Control-Request-Method', method);
  if (requestHeaders !== null) h.set('Access-Control-Request-Headers', requestHeaders);
  return new Request(`https://api.logicaffeine.com${path}`, { method: 'OPTIONS', headers: h });
}

/** An upstream answer in the OpenAI chat shape, with `content` as the model's whole reply. */
export function upstreamAnswer(content, { status = 200, model = 'z-ai/glm-5.3-flash' } = {}) {
  return new Response(JSON.stringify({ id: 'chatcmpl-test', model, choices: [{ index: 0, message: { role: 'assistant', content }, finish_reason: 'stop' }] }), { status, headers: { 'Content-Type': 'application/json' } });
}

/**
 * Run the worker on `request` with `fetch` stubbed to `upstream` (a Response, an Error to throw, or
 * a function of the request). Returns the response, the stub's calls, and every line logged.
 */
export async function harness(request, upstream = upstreamAnswer(HEALED_TEXT)) {
  const calls = [];
  const fetchStub = vi.spyOn(globalThis, 'fetch').mockImplementation(async (url, init) => {
    calls.push({ url: String(url), init });
    if (typeof upstream === 'function') return upstream(url, init);
    if (upstream instanceof Error) throw upstream;
    return upstream.clone();
  });
  const logs = [];
  const logStub = vi.spyOn(console, 'log').mockImplementation((line) => logs.push(String(line)));
  const errStub = vi.spyOn(console, 'error').mockImplementation((line) => logs.push(String(line)));
  const ctx = createExecutionContext();
  try {
    const response = await worker.fetch(request, env, ctx);
    await waitOnExecutionContext(ctx);
    return { response, calls, logs };
  } finally {
    fetchStub.mockRestore();
    logStub.mockRestore();
    errStub.mockRestore();
  }
}
