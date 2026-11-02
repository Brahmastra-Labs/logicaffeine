import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { cloudflareTest, readD1Migrations } from '@cloudflare/vitest-plugin';
import { unstable_readConfig } from 'wrangler';
import { defineConfig } from 'vitest/config';

const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = path.join(here, 'wrangler.toml');

// Both secrets are deployed secrets, never config. The tests stub siteverify and Resend, and check
// that these values are what the worker sends.
const TEST_TURNSTILE_SECRET = 'test-turnstile-secret';
const TEST_RESEND_API_KEY = 're_test_key';

export default defineConfig(async () => {
  const config = unstable_readConfig({ config: configPath }, { hideWarnings: true });
  return {
    plugins: [
      cloudflareTest({
        wrangler: { configPath },
        miniflare: { bindings: { TURNSTILE_SECRET: TEST_TURNSTILE_SECRET, RESEND_API_KEY: TEST_RESEND_API_KEY } },
      }),
    ],
    test: {
      setupFiles: ['./src/test-setup.js'],
      provide: {
        migrations: await readD1Migrations(path.join(here, 'migrations')),
        turnstileSecret: TEST_TURNSTILE_SECRET,
        resendKey: TEST_RESEND_API_KEY,
        wranglerConfig: {
          name: config.name,
          main: config.main && path.relative(here, config.main),
          compatibility_date: config.compatibility_date,
          compatibility_flags: config.compatibility_flags,
          workers_dev: config.workers_dev,
          preview_urls: config.preview_urls,
          routes: config.routes ?? [],
          vars: config.vars,
          secrets: config.secrets ?? null,
          d1_databases: config.d1_databases,
          send_email: config.send_email,
          ratelimits: config.ratelimits,
          triggers: config.triggers,
          observability: config.observability ?? null,
        },
      },
    },
  };
});
