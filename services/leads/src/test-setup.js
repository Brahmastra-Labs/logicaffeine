import { applyD1Migrations, reset } from 'cloudflare:test';
import { env } from 'cloudflare:workers';
import { beforeEach, inject } from 'vitest';

// `reset()` deletes every binding's storage, schema included, so each test starts from a freshly
// migrated, empty database.
beforeEach(async () => {
  await reset();
  await applyD1Migrations(env.DB, inject('migrations'));
});
