import { env } from 'cloudflare:workers';
import { afterEach, describe, expect, inject, it, vi } from 'vitest';
import { CLIENT_IP, HEALED_TEXT, SITE, TYPO_TEXT, UPSTREAM, harness, healRequest, preflight, upstreamAnswer, validBody } from './test-support.js';

afterEach(() => {
  vi.restoreAllMocks();
});

const ALLOWED_ORIGINS = ['https://logicaffeine.com', 'https://www.logicaffeine.com', 'https://logicaffeine.pages.dev', 'https://main.logicaffeine.pages.dev'];
const REFUSED_ORIGINS = ['https://evil.example', 'http://logicaffeine.com', 'https://logicaffeine.com.evil.example', 'https://api.logicaffeine.com', 'https://a.b.logicaffeine.pages.dev'];

describe('the door', () => {
  it('answers a preflight from the site with the grant and nothing else', async () => {
    const { response } = await harness(preflight());
    expect(response.status).toBe(204);
    expect(response.headers.get('Access-Control-Allow-Origin')).toBe(SITE);
    expect(response.headers.get('Access-Control-Allow-Methods')).toBe('POST, OPTIONS');
    expect(await response.text()).toBe('');
  });

  it.each(ALLOWED_ORIGINS)('grants %s', async (origin) => {
    const { response } = await harness(healRequest({ origin }));
    expect(response.status).toBe(200);
    expect(response.headers.get('Access-Control-Allow-Origin')).toBe(origin);
  });

  it.each(REFUSED_ORIGINS)('refuses %s before reading the body', async (origin) => {
    const { response, calls } = await harness(healRequest({ origin }));
    expect(response.status).toBe(403);
    expect(await response.json()).toEqual({ ok: false, error: 'origin_not_allowed' });
    expect(response.headers.get('Access-Control-Allow-Origin')).toBeNull();
    expect(calls).toEqual([]);
  });

  it('refuses a request with no origin', async () => {
    const { response } = await harness(healRequest({ origin: null }));
    expect(response.status).toBe(403);
  });

  it('serves /heal and nothing else', async () => {
    const { response } = await harness(healRequest({ path: '/lead' }));
    expect(response.status).toBe(404);
  });

  it('takes POST only', async () => {
    const { response } = await harness(healRequest({ method: 'GET' }));
    expect(response.status).toBe(405);
    expect(response.headers.get('Allow')).toBe('POST, OPTIONS');
  });

  it('takes JSON only, read up to its cap', async () => {
    expect((await harness(healRequest({ contentType: 'text/plain' }))).response.status).toBe(415);
    expect((await harness(healRequest({ raw: '{not json' }))).response.status).toBe(400);
    expect((await harness(healRequest({ raw: 'x'.repeat(70_000) }))).response.status).toBe(413);
  });

  it('refuses a bad request by name, without calling upstream', async () => {
    for (const [body, error] of [[{ mode: 'math', text: 'x' }, 'unknown_mode'], [{ mode: 'code' }, 'text_missing'], [{ mode: 'code', text: 'a'.repeat(9000) }, 'text_too_long']]) {
      const { response, calls } = await harness(healRequest({ body }));
      expect(response.status).toBe(400);
      expect(await response.json()).toEqual({ ok: false, error });
      expect(calls).toEqual([]);
    }
  });
});

describe('the heal', () => {
  it('calls the upstream once, as a chat completion, with the key and the model, and answers with the healed text', async () => {
    const { response, calls } = await harness(healRequest());
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ ok: true, healed: HEALED_TEXT, model: env.MODEL });
    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe(UPSTREAM);
    expect(calls[0].url).toBe(env.UPSTREAM_URL);
    const { init } = calls[0];
    expect(init.method).toBe('POST');
    expect(new Headers(init.headers).get('Authorization')).toBe(`Bearer ${inject('nvidiaKey')}`);
    expect(new Headers(init.headers).get('Content-Type')).toMatch(/application\/json/);
    const sent = JSON.parse(init.body);
    expect(sent.model).toBe(env.MODEL);
    expect(sent.messages.map((m) => m.role)).toEqual(['system', 'user']);
    expect(sent.messages[1].content).toBe(TYPO_TEXT);
    expect(sent.stream).toBeFalsy();
    expect(sent.temperature).toBeLessThanOrEqual(0.3);
    expect(init.signal).toBeInstanceOf(AbortSignal);
  });

  it('answers 502 by name when the upstream fails, and never leaks its words', async () => {
    const { response: r1, logs: l1 } = await harness(healRequest(), new Response('{"error":"quota"}', { status: 503 }));
    expect(r1.status).toBe(502);
    expect(await r1.json()).toEqual({ ok: false, error: 'upstream_failed' });
    expect(l1.join('\n')).not.toMatch(/quota/);
    const { response: r2 } = await harness(healRequest(), new Error('socket hang up'));
    expect(r2.status).toBe(502);
    const { response: r3 } = await harness(healRequest(), upstreamAnswer(''));
    expect(r3.status).toBe(502);
    expect(await r3.json()).toEqual({ ok: false, error: 'empty_answer' });
  });

  it('answers 500 by name when the key is not set', async () => {
    const saved = env.NVIDIA_API_KEY;
    env.NVIDIA_API_KEY = '';
    try {
      const { response, calls } = await harness(healRequest());
      expect(response.status).toBe(500);
      expect(await response.json()).toEqual({ ok: false, error: 'internal' });
      expect(calls).toEqual([]);
    } finally {
      env.NVIDIA_API_KEY = saved;
    }
  });

  it('logs the event and the sizes, never the key and never the text', async () => {
    const { logs } = await harness(healRequest());
    const all = logs.join('\n');
    expect(all).toMatch(/"event":"healed"/);
    expect(all).toMatch(/"mode":"code"/);
    expect(all).not.toMatch(/nvapi-/);
    expect(all).not.toMatch(/atleast/);
    expect(all).not.toMatch(/Grade/);
  });

  it('rate limits by IP with Retry-After, after the limit lets ten through', async () => {
    const limited = vi.spyOn(env.HEAL_IP_LIMIT, 'limit').mockResolvedValue({ success: false });
    try {
      const { response, calls } = await harness(healRequest({ ip: CLIENT_IP }));
      expect(response.status).toBe(429);
      expect(await response.json()).toEqual({ ok: false, error: 'rate_limited' });
      expect(response.headers.get('Retry-After')).toBe('60');
      expect(calls).toEqual([]);
      expect(limited).toHaveBeenCalledWith({ key: `ip:${CLIENT_IP}` });
    } finally {
      limited.mockRestore();
    }
  });

  it('sends no cache and no sniff on every answer', async () => {
    const { response } = await harness(healRequest());
    expect(response.headers.get('Cache-Control')).toBe('no-store');
    expect(response.headers.get('X-Content-Type-Options')).toBe('nosniff');
  });
});

describe('the deploy', () => {
  it('is the worker named for the route, with the key a required secret and the limit at ten a minute', () => {
    const config = inject('wranglerConfig');
    expect(config.name).toBe('logicaffeine-heal');
    expect(config.main).toBe('src/index.js');
    expect(config.workers_dev).toBe(false);
    expect(config.routes).toEqual([{ pattern: 'api.logicaffeine.com/heal', zone_name: 'logicaffeine.com' }]);
    expect(config.secrets).toEqual({ required: ['NVIDIA_API_KEY'] });
    expect(config.vars.UPSTREAM_URL).toBe(UPSTREAM);
    expect(config.vars.MODEL).toMatch(/\S/);
    expect(config.ratelimits).toEqual([{ name: 'HEAL_IP_LIMIT', namespace_id: '1303', simple: { limit: 10, period: 60 } }]);
    expect(config.observability).toEqual({ enabled: true });
  });
});
