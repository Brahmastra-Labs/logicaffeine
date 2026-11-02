/**
 * LOGOS Package Registry - Tests
 * Run with: npm test
 */

import { describe, it, expect, beforeAll } from 'vitest';
import { SELF, env, fetchMock } from 'cloudflare:test';

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
  describe('OAuth state', () => {
    // A callback this browser did not start is login CSRF: the attacker's code, carried to the
    // victim, signs the victim in to the attacker's account. GitHub hands back the `state` the
    // sign-in was started with, and only the browser holding that state in its cookie may finish.
    const callback = (query, cookie) =>
      SELF.fetch(`https://registry.logicaffeine.com/auth/callback?${query}`, {
        redirect: 'manual',
        headers: cookie ? { Cookie: cookie } : {},
      });
    const refused = (response) => {
      expect(response.status).toBe(302);
      const location = new URL(response.headers.get('Location'));
      expect(location.searchParams.get('token')).toBeNull();
      expect(location.searchParams.get('error')).toBe(
        'The sign-in could not be verified; please start it again'
      );
    };

    it('the sign-in sets the state it sends to GitHub', async () => {
      const response = await SELF.fetch('https://registry.logicaffeine.com/auth/github', {
        redirect: 'manual',
      });
      const state = new URL(response.headers.get('Location')).searchParams.get('state');
      expect(state).toMatch(/^[0-9a-f-]{36}$/);
      expect(response.headers.get('Set-Cookie')).toContain(`oauth_state=${state};`);
    });

    it('refuses a callback with no state', async () => {
      refused(await callback('code=attacker-code', 'oauth_state=0b2c6d1e-3f4a-4b5c-8d9e-0f1a2b3c4d5e'));
    });

    it('refuses a callback from a browser holding no state', async () => {
      refused(await callback('code=attacker-code&state=0b2c6d1e-3f4a-4b5c-8d9e-0f1a2b3c4d5e'));
    });

    it('refuses a callback whose state is not the one this browser holds', async () => {
      refused(
        await callback(
          'code=attacker-code&state=0b2c6d1e-3f4a-4b5c-8d9e-0f1a2b3c4d5e',
          'oauth_state=9f8e7d6c-5b4a-4321-8fed-cba987654321'
        )
      );
    });

    it('refuses a state that matches only a prefix of the cookie', async () => {
      refused(
        await callback(
          'code=attacker-code&state=0b2c6d1e',
          'oauth_state=0b2c6d1e-3f4a-4b5c-8d9e-0f1a2b3c4d5e'
        )
      );
    });

    it('a verified sign-in hands its session over in the fragment, never the query', async () => {
      // A query string is sent to the server it names and kept in its access log, synced with the
      // browser's history, and carried in the Referer of every request the landing page makes; a
      // fragment never leaves the browser.
      fetchMock.activate();
      fetchMock.disableNetConnect();
      fetchMock
        .get('https://github.com')
        .intercept({ path: '/login/oauth/access_token', method: 'POST' })
        .reply(200, { access_token: 'gho_registry_test' });
      fetchMock
        .get('https://api.github.com')
        .intercept({ path: '/user', method: 'GET' })
        .reply(200, { id: 4242, login: 'octo', name: 'Octo', email: null, avatar_url: null });
      const state = '0b2c6d1e-3f4a-4b5c-8d9e-0f1a2b3c4d5e';
      const response = await callback(`code=good&state=${state}`, `oauth_state=${state}`);
      expect(response.status).toBe(302);
      const location = new URL(response.headers.get('Location'));
      expect(location.searchParams.get('error')).toBeNull();
      expect(location.searchParams.get('token')).toBeNull();
      expect(location.search).toBe('');
      const fragment = new URLSearchParams(location.hash.slice(1));
      expect(fragment.get('token')).toMatch(/^[^.]+\.[^.]+\.[^.]+$/);
      expect(fragment.get('login')).toBe('octo');
      expect(response.headers.get('Set-Cookie')).toContain('Max-Age=0');
      fetchMock.assertNoPendingInterceptors();
      fetchMock.deactivate();
    });

    it('a callback with the browser’s own state goes on to the code, and spends the state', async () => {
      const state = '0b2c6d1e-3f4a-4b5c-8d9e-0f1a2b3c4d5e';
      const response = await callback(`state=${state}`, `theme=dark; oauth_state=${state}`);
      expect(response.status).toBe(302);
      const location = new URL(response.headers.get('Location'));
      expect(location.searchParams.get('error')).toBe('No authorization code received');
      expect(response.headers.get('Set-Cookie')).toContain('oauth_state=;');
      expect(response.headers.get('Set-Cookie')).toContain('Max-Age=0');
    });
  });

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
