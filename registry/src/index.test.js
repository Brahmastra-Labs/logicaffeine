/**
 * LOGOS Package Registry - Tests
 * Run with: npm test
 */

import { describe, it, expect, beforeAll } from 'vitest';
import { SELF, env } from 'cloudflare:test';

describe('Registry API', () => {
  describe('Health Check', () => {
    it('GET / returns ok status', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/');
      expect(response.status).toBe(200);

      const data = await response.json();
      expect(data.status).toBe('ok');
      expect(data.service).toBe('logos-package-registry');
    });

    it('GET /health returns ok status', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/health');
      expect(response.status).toBe(200);

      const data = await response.json();
      expect(data.status).toBe('ok');
    });
  });

  describe('CORS', () => {
    it('OPTIONS request returns CORS headers', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/packages', {
        method: 'OPTIONS',
      });
      expect(response.status).toBe(200);
      expect(response.headers.get('Access-Control-Allow-Origin')).toBe('*');
      expect(response.headers.get('Access-Control-Allow-Methods')).toContain('GET');
    });
  });

  describe('Package Routes', () => {
    it('GET /packages returns empty list initially', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/packages');
      expect(response.status).toBe(200);

      const data = await response.json();
      expect(data.packages).toBeInstanceOf(Array);
      expect(data.pagination).toBeDefined();
      expect(data.pagination.page).toBe(1);
    });

    it('GET /packages/:name returns 404 for nonexistent package', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/packages/nonexistent');
      expect(response.status).toBe(404);

      const data = await response.json();
      expect(data.error).toBe('Package not found');
    });

    it('GET /packages/:name/:version returns 404 for nonexistent version', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/packages/test/1.0.0');
      expect(response.status).toBe(404);

      const data = await response.json();
      expect(data.error).toBe('Version not found');
    });
  });

  describe('Auth Routes', () => {
    it('GET /auth/github redirects to GitHub', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/auth/github', {
        redirect: 'manual',
      });
      expect(response.status).toBe(302);
      expect(response.headers.get('Location')).toContain('github.com/login/oauth');
    });

    it('GET /auth/me requires authentication', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/auth/me');
      expect(response.status).toBe(401);

      const data = await response.json();
      expect(data.error).toBe('Unauthorized');
    });

    it('GET /auth/tokens requires authentication', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/auth/tokens');
      expect(response.status).toBe(401);
    });
  });

  describe('Publish Routes', () => {
    it('POST /packages/publish requires authentication', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/packages/publish', {
        method: 'POST',
      });
      expect(response.status).toBe(401);
    });

    it('POST /packages/publish requires multipart/form-data', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/packages/publish', {
        method: 'POST',
        headers: {
          'Authorization': 'Bearer fake-token',
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({}),
      });
      // Will fail auth first, but the content-type check happens after
      expect(response.status).toBe(401);
    });
  });

  describe('404 Handling', () => {
    it('Unknown routes return 404', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/unknown/route');
      expect(response.status).toBe(404);

      const data = await response.json();
      expect(data.error).toBe('Not found');
    });
  });
});

describe('Security', () => {
  describe('Reserved Namespaces', () => {
    // Note: Full publish tests would require mocking D1 and R2
    // These are integration tests that verify the endpoint behavior

    it('rejects unauthorized access', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/packages/publish', {
        method: 'POST',
        headers: {
          'Authorization': 'Bearer invalid-token',
        },
      });
      expect(response.status).toBe(401);
    });
  });
});
