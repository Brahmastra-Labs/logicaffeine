import { env } from 'cloudflare:workers';
import { describe, expect, inject, it } from 'vitest';

const config = inject('wranglerConfig');
const SALES_INBOX = 'team@datacenter.dev';

function origins() {
  return String(config.vars?.ALLOWED_ORIGINS ?? '')
    .split(',')
    .map((o) => o.trim())
    .filter(Boolean);
}

describe('wrangler.toml', () => {
  it('names the worker and its entry point', () => {
    expect(config.name).toBe('logicaffeine-leads');
    expect(config.main).toBe('src/index.js');
  });

  it('runs on a current compatibility date', () => {
    expect(config.compatibility_date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(config.compatibility_date >= '2026-09-30').toBe(true);
  });

  it('serves only api.logicaffeine.com/lead: no workers.dev URL, no preview URLs', () => {
    expect(config.routes).toEqual([{ pattern: 'api.logicaffeine.com/lead', zone_name: 'logicaffeine.com' }]);
    expect(config.workers_dev).toBe(false);
    expect(config.preview_urls).toBe(false);
  });

  it('binds D1 as DB to logos-leads, migrated from migrations/', () => {
    expect(config.d1_databases).toHaveLength(1);
    const [db] = config.d1_databases;
    expect(db).toMatchObject({ binding: 'DB', database_name: 'logos-leads', migrations_dir: 'migrations' });
    expect(db.database_id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
  });

  it('sends through Resend, not a send_email binding', () => {
    expect(config.send_email ?? []).toEqual([]);
  });

  it('names the sales inbox as the one recipient', () => {
    expect(config.vars.NOTIFY_TO).toBe(SALES_INBOX);
    expect(env.NOTIFY_TO).toBe(SALES_INBOX);
  });

  it('sends from datacenter.dev, the domain verified in the Resend account', () => {
    expect(config.vars.NOTIFY_FROM).toBe('pricing@datacenter.dev');
    expect(env.NOTIFY_FROM).toBe(config.vars.NOTIFY_FROM);
  });

  it('limits per client IP and per email address, each in its own namespace, over a minute', () => {
    const byName = Object.fromEntries((config.ratelimits ?? []).map((r) => [r.name, r]));
    expect(Object.keys(byName).sort()).toEqual(['LEAD_EMAIL_LIMIT', 'LEAD_IP_LIMIT']);
    expect(byName.LEAD_IP_LIMIT.simple).toEqual({ limit: 5, period: 60 });
    expect(byName.LEAD_EMAIL_LIMIT.simple).toEqual({ limit: 3, period: 60 });
    expect(byName.LEAD_IP_LIMIT.namespace_id).not.toBe(byName.LEAD_EMAIL_LIMIT.namespace_id);
    for (const r of config.ratelimits) expect(r.namespace_id).toMatch(/^[1-9][0-9]*$/);
  });

  it('allows the production site, www and the Pages deployments, over https only', () => {
    expect(origins()).toEqual([
      'https://logicaffeine.com',
      'https://www.logicaffeine.com',
      'https://logicaffeine.pages.dev',
      'https://*.logicaffeine.pages.dev',
    ]);
  });

  it('allows no local or plain-http origin', () => {
    expect(origins().length).toBeGreaterThan(0);
    for (const origin of origins()) {
      expect(origin.startsWith('https://'), origin).toBe(true);
      expect(origin, origin).not.toMatch(/localhost|127\.0\.0\.1|\[::1\]|192\.168\.|:\d+$/);
    }
  });

  it('declares the Turnstile secret and the Resend key as required secrets, never as vars', () => {
    expect(config.secrets).toEqual({ required: ['TURNSTILE_SECRET', 'RESEND_API_KEY'] });
    expect(Object.keys(config.vars).sort()).toEqual(['ALLOWED_ORIGINS', 'NOTIFY_FROM', 'NOTIFY_TO']);
  });

  it('runs the retention sweep once a day', () => {
    expect(config.triggers.crons).toEqual(['17 3 * * *']);
  });

  it('keeps logs and traces on, and invocation logs off so no request metadata is kept', () => {
    expect(config.observability).toMatchObject({
      enabled: true,
      logs: { enabled: true, invocation_logs: false },
      traces: { enabled: true },
    });
  });
});
