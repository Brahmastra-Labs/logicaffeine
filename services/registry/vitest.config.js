import { fileURLToPath } from 'node:url';
import { defineWorkersConfig, readD1Migrations } from '@cloudflare/vitest-pool-workers/config';

export default defineWorkersConfig(async () => {
  // The schema the deployed database runs, applied to each test file's own D1 before its tests.
  const migrations = await readD1Migrations(fileURLToPath(new URL('./migrations', import.meta.url)));
  return {
    test: {
      setupFiles: ['./test/apply-migrations.js'],
      poolOptions: {
        workers: {
          wrangler: { configPath: './wrangler.toml' },
          miniflare: {
            bindings: {
              TEST_MIGRATIONS: migrations,
              JWT_SECRET: 'registry-test-jwt-secret',
              GITHUB_CLIENT_ID: 'registry-test-client',
              GITHUB_CLIENT_SECRET: 'registry-test-client-secret',
            },
          },
        },
      },
    },
  };
});
