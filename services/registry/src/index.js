/**
 * LOGOS Package Registry - Main Entry Point
 * Phase 39: The Cloudflare Registry
 */

import { handlePackagesList, handlePackageInfo, handleVersionInfo } from './routes/packages.js';
import { handlePublish } from './routes/publish.js';
import { handleDownload } from './routes/download.js';
import { handleGitHubAuth, handleGitHubCallback, handleMe, handleCreateToken, handleListTokens, handleRevokeToken } from './routes/auth.js';

const CORS_HEADERS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, PUT, DELETE, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
};

export default {
  async fetch(request, env, ctx) {
    // Handle CORS preflight
    if (request.method === 'OPTIONS') {
      return new Response(null, { headers: CORS_HEADERS });
    }

    const url = new URL(request.url);
    const path = url.pathname;

    try {
      // Health check
      if (path === '/health' || path === '/') {
        return jsonResponse({
          status: 'ok',
          service: 'logos-package-registry',
          version: '1.0.0'
        });
      }

      // ========== Auth Routes (no auth required) ==========
      if (path === '/auth/github' && request.method === 'GET') {
        return handleGitHubAuth(request, env);
      }
      if (path === '/auth/callback' && request.method === 'GET') {
        return handleGitHubCallback(request, env);
      }

      // ========== Auth Routes (auth required) ==========
      if (path === '/auth/me' && request.method === 'GET') {
        return withAuth(request, env, handleMe);
      }
      if (path === '/auth/tokens' && request.method === 'GET') {
        return withAuth(request, env, handleListTokens);
      }
      if (path === '/auth/tokens' && request.method === 'POST') {
        return withAuth(request, env, handleCreateToken);
      }
      const tokenRevokeMatch = path.match(/^\/auth\/tokens\/([a-zA-Z0-9_-]+)$/);
      if (tokenRevokeMatch && request.method === 'DELETE') {
        return withAuth(request, env, (req, e, user) => handleRevokeToken(req, e, user, tokenRevokeMatch[1]));
      }

      // ========== Package Routes (public) ==========
      if (path === '/packages' && request.method === 'GET') {
        return handlePackagesList(request, env);
      }

      // Match /packages/:name
      const packageMatch = path.match(/^\/packages\/([a-z0-9_-]+)$/i);
      if (packageMatch && request.method === 'GET') {
        return handlePackageInfo(request, env, packageMatch[1]);
      }

      // Match /packages/:name/:version
      const versionMatch = path.match(/^\/packages\/([a-z0-9_-]+)\/([a-z0-9._-]+)$/i);
      if (versionMatch && request.method === 'GET') {
        return handleVersionInfo(request, env, versionMatch[1], versionMatch[2]);
      }

      // Match /packages/:name/:version/download
      const downloadMatch = path.match(/^\/packages\/([a-z0-9_-]+)\/([a-z0-9._-]+)\/download$/i);
      if (downloadMatch && request.method === 'GET') {
        return handleDownload(request, env, ctx, downloadMatch[1], downloadMatch[2]);
      }

      // ========== Publish Route (auth required) ==========
      if (path === '/packages/publish' && request.method === 'POST') {
        return withAuth(request, env, handlePublish);
      }

      return jsonResponse({ error: 'Not found' }, 404);
    } catch (error) {
      console.error('Unhandled error:', error);
      return jsonResponse({ error: 'Internal server error', message: error.message }, 500);
    }
  },
};

/**
 * Middleware: wrap handler with authentication
 */
async function withAuth(request, env, handler) {
  const user = await authenticate(request, env);
  if (!user) {
    return jsonResponse({ error: 'Unauthorized', message: 'Valid authentication required' }, 401);
  }
  if (user.is_banned) {
    return jsonResponse({
      error: 'Forbidden',
      message: `Account suspended: ${user.ban_reason || 'Contact support'}`
    }, 403);
  }
  return handler(request, env, user);
}

/**
 * Authenticate request via Bearer token or API token
 */
async function authenticate(request, env) {
  const authHeader = request.headers.get('Authorization');
  if (!authHeader || !authHeader.startsWith('Bearer ')) {
    return null;
  }

  const token = authHeader.slice(7);

  // Check if it's an API token (starts with lgr_)
  if (token.startsWith('lgr_')) {
    return authenticateApiToken(token, env);
  }

  // Otherwise treat as JWT session token
  return authenticateJwt(token, env);
}

/**
 * Authenticate API token (for CLI)
 */
async function authenticateApiToken(token, env) {
  const tokenHash = await sha256(token);

  const result = await env.DB.prepare(`
    SELECT u.*, t.scopes, t.id as token_id
    FROM api_tokens t
    JOIN users u ON t.user_id = u.id
    WHERE t.token_hash = ?
    AND (t.expires_at IS NULL OR t.expires_at > datetime('now'))
  `).bind(tokenHash).first();

  if (!result) {
    return null;
  }

  // Update last_used_at
  await env.DB.prepare(`
    UPDATE api_tokens SET last_used_at = datetime('now') WHERE id = ?
  `).bind(result.token_id).run();

  return {
    id: result.id,
    github_login: result.github_login,
    github_name: result.github_name,
    avatar_url: result.avatar_url,
    is_admin: result.is_admin === 1,
    is_banned: result.is_banned === 1,
    ban_reason: result.ban_reason,
    scopes: result.scopes ? result.scopes.split(',') : ['publish'],
  };
}

/**
 * Authenticate JWT session token (for web UI)
 */
async function authenticateJwt(token, env) {
  try {
    const parts = token.split('.');
    if (parts.length !== 3) return null;

    const payload = JSON.parse(atob(parts[1]));

    // Check expiration
    if (payload.exp && payload.exp < Math.floor(Date.now() / 1000)) {
      return null;
    }

    // Verify signature
    const encoder = new TextEncoder();
    const key = await crypto.subtle.importKey(
      'raw',
      encoder.encode(env.JWT_SECRET),
      { name: 'HMAC', hash: 'SHA-256' },
      false,
      ['verify']
    );

    const data = `${parts[0]}.${parts[1]}`;
    const signature = Uint8Array.from(atob(parts[2]), c => c.charCodeAt(0));

    const valid = await crypto.subtle.verify('HMAC', key, signature, encoder.encode(data));
    if (!valid) return null;

    // Fetch user from database
    const user = await env.DB.prepare(`
      SELECT * FROM users WHERE id = ?
    `).bind(payload.sub).first();

    if (!user) return null;

    return {
      id: user.id,
      github_login: user.github_login,
      github_name: user.github_name,
      avatar_url: user.avatar_url,
      is_admin: user.is_admin === 1,
      is_banned: user.is_banned === 1,
      ban_reason: user.ban_reason,
      scopes: ['publish', 'yank'], // Session tokens have full access
    };
  } catch {
    return null;
  }
}

/**
 * Helper: JSON response with CORS headers
 */
function jsonResponse(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      ...CORS_HEADERS,
      'Content-Type': 'application/json',
    },
  });
}

/**
 * Helper: SHA-256 hash
 */
async function sha256(str) {
  const encoder = new TextEncoder();
  const data = encoder.encode(str);
  const hash = await crypto.subtle.digest('SHA-256', data);
  return Array.from(new Uint8Array(hash))
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
}

// Export helpers for use in route handlers
export { jsonResponse, sha256, CORS_HEADERS };
