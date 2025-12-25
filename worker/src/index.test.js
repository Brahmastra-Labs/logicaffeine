import { describe, it, expect, vi, beforeEach } from 'vitest';
import { env, createExecutionContext, waitOnExecutionContext } from 'cloudflare:test';
import worker from './index.js';

describe('License Validator Worker', () => {
  describe('CORS', () => {
    it('responds to OPTIONS with CORS headers', async () => {
      const request = new Request('https://api.logicaffeine.com/validate', {
        method: 'OPTIONS',
      });
      const ctx = createExecutionContext();
      const response = await worker.fetch(request, env, ctx);
      await waitOnExecutionContext(ctx);

      expect(response.status).toBe(200);
      expect(response.headers.get('Access-Control-Allow-Origin')).toBe('*');
      expect(response.headers.get('Access-Control-Allow-Methods')).toBe('GET, POST, OPTIONS');
    });
  });

  describe('Health check', () => {
    it('returns ok status', async () => {
      const request = new Request('https://api.logicaffeine.com/health');
      const ctx = createExecutionContext();
      const response = await worker.fetch(request, env, ctx);
      await waitOnExecutionContext(ctx);

      expect(response.status).toBe(200);
      const data = await response.json();
      expect(data.status).toBe('ok');
    });
  });

  describe('Not found', () => {
    it('returns 404 for unknown routes', async () => {
      const request = new Request('https://api.logicaffeine.com/unknown');
      const ctx = createExecutionContext();
      const response = await worker.fetch(request, env, ctx);
      await waitOnExecutionContext(ctx);

      expect(response.status).toBe(404);
      const data = await response.json();
      expect(data.error).toBe('Not found');
    });
  });

  describe('/validate endpoint', () => {
    it('rejects missing license key', async () => {
      const request = new Request('https://api.logicaffeine.com/validate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });
      const ctx = createExecutionContext();
      const response = await worker.fetch(request, env, ctx);
      await waitOnExecutionContext(ctx);

      expect(response.status).toBe(400);
      const data = await response.json();
      expect(data.valid).toBe(false);
      expect(data.error).toBe('No license key provided');
    });

    it('rejects invalid license key format', async () => {
      const request = new Request('https://api.logicaffeine.com/validate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ licenseKey: 'invalid_key' }),
      });
      const ctx = createExecutionContext();
      const response = await worker.fetch(request, env, ctx);
      await waitOnExecutionContext(ctx);

      expect(response.status).toBe(400);
      const data = await response.json();
      expect(data.valid).toBe(false);
      expect(data.error).toBe('Invalid license key format');
    });
  });

  describe('/session endpoint', () => {
    it('rejects missing session ID', async () => {
      const request = new Request('https://api.logicaffeine.com/session', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });
      const ctx = createExecutionContext();
      const response = await worker.fetch(request, env, ctx);
      await waitOnExecutionContext(ctx);

      expect(response.status).toBe(400);
      const data = await response.json();
      expect(data.error).toBe('No session ID provided');
    });

    it('rejects invalid session ID format', async () => {
      const request = new Request('https://api.logicaffeine.com/session', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ sessionId: 'invalid_session' }),
      });
      const ctx = createExecutionContext();
      const response = await worker.fetch(request, env, ctx);
      await waitOnExecutionContext(ctx);

      expect(response.status).toBe(400);
      const data = await response.json();
      expect(data.error).toBe('Invalid session ID format');
    });
  });
});
