import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { cloudflareTest } from '@cloudflare/vitest-plugin';
import { unstable_readConfig } from 'wrangler';
import { defineConfig } from 'vitest/config';

const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = path.join(here, 'wrangler.toml');

// The key is a deployed secret, never config. The tests stub the upstream and check that this
// value is what the worker sends, and that it reaches no log line.
const TEST_NVIDIA_API_KEY = 'nvapi-test-key-0000';

export default defineConfig(async () => {
  const config = unstable_readConfig({ config: configPath }, { hideWarnings: true });
  return {
    plugins: [
      cloudflareTest({
        wrangler: { configPath },
        miniflare: { bindings: { NVIDIA_API_KEY: TEST_NVIDIA_API_KEY } },
      }),
    ],
    test: {
      provide: {
        nvidiaKey: TEST_NVIDIA_API_KEY,
        wranglerConfig: {
          name: config.name,
          main: config.main && path.relative(here, config.main),
          compatibility_date: config.compatibility_date,
          workers_dev: config.workers_dev,
          preview_urls: config.preview_urls,
          routes: config.routes ?? [],
          vars: config.vars,
          secrets: config.secrets ?? null,
          ratelimits: config.ratelimits,
          observability: config.observability ?? null,
        },
      },
    },
  };
});
