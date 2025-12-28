/**
 * LOGOS Package Registry - Auth Routes
 * GitHub OAuth flow and API token management
 */

import { jsonResponse, sha256, CORS_HEADERS } from '../index.js';

const GITHUB_AUTHORIZE_URL = 'https://github.com/login/oauth/authorize';
const GITHUB_TOKEN_URL = 'https://github.com/login/oauth/access_token';
const GITHUB_USER_URL = 'https://api.github.com/user';

/**
 * GET /auth/github - Redirect to GitHub OAuth
 */
export function handleGitHubAuth(request, env) {
  const url = new URL(request.url);
  const redirectUri = `${env.REGISTRY_URL}/auth/callback`;

  // Generate state for CSRF protection
  const state = crypto.randomUUID();

  const authUrl = new URL(GITHUB_AUTHORIZE_URL);
  authUrl.searchParams.set('client_id', env.GITHUB_CLIENT_ID);
  authUrl.searchParams.set('redirect_uri', redirectUri);
  authUrl.searchParams.set('scope', 'read:user user:email');
  authUrl.searchParams.set('state', state);

  return new Response(null, {
    status: 302,
    headers: {
      'Location': authUrl.toString(),
      'Set-Cookie': `oauth_state=${state}; HttpOnly; Secure; SameSite=Lax; Max-Age=600; Path=/`,
    },
  });
}

/**
 * GET /auth/callback - GitHub OAuth callback
 */
export async function handleGitHubCallback(request, env) {
  const url = new URL(request.url);
  const code = url.searchParams.get('code');
  const error = url.searchParams.get('error');

  if (error) {
    return redirectWithError(env, `GitHub error: ${error}`);
  }

  if (!code) {
    return redirectWithError(env, 'No authorization code received');
  }

  try {
    // Exchange code for access token
    const tokenResponse = await fetch(GITHUB_TOKEN_URL, {
      method: 'POST',
      headers: {
        'Accept': 'application/json',
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        client_id: env.GITHUB_CLIENT_ID,
        client_secret: env.GITHUB_CLIENT_SECRET,
        code,
      }),
    });

    const tokenData = await tokenResponse.json();
    if (tokenData.error) {
      return redirectWithError(env, tokenData.error_description || tokenData.error);
    }

    // Fetch GitHub user info
    const userResponse = await fetch(GITHUB_USER_URL, {
      headers: {
        'Authorization': `Bearer ${tokenData.access_token}`,
        'Accept': 'application/vnd.github.v3+json',
        'User-Agent': 'LOGOS-Registry/1.0',
      },
    });

    const githubUser = await userResponse.json();

    // Upsert user in D1
    const userId = String(githubUser.id);
    await env.DB.prepare(`
      INSERT INTO users (id, github_login, github_name, email, avatar_url, updated_at)
      VALUES (?, ?, ?, ?, ?, datetime('now'))
      ON CONFLICT(id) DO UPDATE SET
        github_login = excluded.github_login,
        github_name = excluded.github_name,
        email = excluded.email,
        avatar_url = excluded.avatar_url,
        updated_at = datetime('now')
    `).bind(
      userId,
      githubUser.login,
      githubUser.name || null,
      githubUser.email || null,
      githubUser.avatar_url || null
    ).run();

    // Check if user is banned
    const user = await env.DB.prepare('SELECT * FROM users WHERE id = ?').bind(userId).first();
    if (user.is_banned) {
      return redirectWithError(env, `Account suspended: ${user.ban_reason || 'Contact support'}`);
    }

    // Create session token (JWT)
    const sessionToken = await createSessionToken(env, userId, githubUser.login);

    // Redirect to frontend with token
    const successUrl = new URL(`${env.ALLOWED_ORIGIN}/registry`);
    successUrl.searchParams.set('token', sessionToken);
    successUrl.searchParams.set('login', githubUser.login);

    return new Response(null, {
      status: 302,
      headers: { 'Location': successUrl.toString() },
    });
  } catch (err) {
    console.error('OAuth callback error:', err);
    return redirectWithError(env, 'Authentication failed');
  }
}

/**
 * GET /auth/me - Get current user info
 */
export async function handleMe(request, env, user) {
  // Get user's packages
  const packages = await env.DB.prepare(`
    SELECT name, description, is_verified, downloads
    FROM packages WHERE owner_id = ?
    ORDER BY downloads DESC
  `).bind(user.id).all();

  return jsonResponse({
    id: user.id,
    login: user.github_login,
    name: user.github_name,
    avatar_url: user.avatar_url,
    is_admin: user.is_admin,
    scopes: user.scopes,
    packages: packages.results.map(p => ({
      name: p.name,
      description: p.description,
      verified: p.is_verified === 1,
      downloads: p.downloads,
    })),
  });
}

/**
 * GET /auth/tokens - List user's API tokens
 */
export async function handleListTokens(request, env, user) {
  const tokens = await env.DB.prepare(`
    SELECT id, name, scopes, last_used_at, expires_at, created_at
    FROM api_tokens WHERE user_id = ?
    ORDER BY created_at DESC
  `).bind(user.id).all();

  return jsonResponse({
    tokens: tokens.results.map(t => ({
      id: t.id,
      name: t.name,
      scopes: t.scopes ? t.scopes.split(',') : ['publish'],
      last_used_at: t.last_used_at,
      expires_at: t.expires_at,
      created_at: t.created_at,
    })),
  });
}

/**
 * POST /auth/tokens - Create new API token
 * Body: { name: string, expires_in_days?: number }
 */
export async function handleCreateToken(request, env, user) {
  let body;
  try {
    body = await request.json();
  } catch {
    return jsonResponse({ error: 'Invalid JSON body' }, 400);
  }

  const name = body.name || 'CLI Token';
  const expiresInDays = body.expires_in_days;

  if (name.length > 100) {
    return jsonResponse({ error: 'Token name too long (max 100 chars)' }, 400);
  }

  // Generate secure random token
  const tokenBytes = new Uint8Array(32);
  crypto.getRandomValues(tokenBytes);
  const token = 'lgr_' + bytesToHex(tokenBytes);

  // Hash for storage
  const tokenHash = await sha256(token);
  const tokenId = crypto.randomUUID();

  // Calculate expiration
  let expiresAt = null;
  if (expiresInDays && expiresInDays > 0) {
    const date = new Date();
    date.setDate(date.getDate() + expiresInDays);
    expiresAt = date.toISOString().replace('T', ' ').split('.')[0];
  }

  await env.DB.prepare(`
    INSERT INTO api_tokens (id, user_id, name, token_hash, scopes, expires_at)
    VALUES (?, ?, ?, ?, 'publish', ?)
  `).bind(tokenId, user.id, name, tokenHash, expiresAt).run();

  return jsonResponse({
    token, // Only returned once! User must save it.
    token_id: tokenId,
    name,
    scopes: ['publish'],
    expires_at: expiresAt,
    warning: 'Save this token now. It will not be shown again.',
  }, 201);
}

/**
 * DELETE /auth/tokens/:id - Revoke an API token
 */
export async function handleRevokeToken(request, env, user, tokenId) {
  // Verify token belongs to user
  const token = await env.DB.prepare(`
    SELECT id FROM api_tokens WHERE id = ? AND user_id = ?
  `).bind(tokenId, user.id).first();

  if (!token) {
    return jsonResponse({ error: 'Token not found' }, 404);
  }

  await env.DB.prepare('DELETE FROM api_tokens WHERE id = ?').bind(tokenId).run();

  return jsonResponse({ success: true, message: 'Token revoked' });
}

// ========== Helpers ==========

async function createSessionToken(env, userId, login) {
  const header = { alg: 'HS256', typ: 'JWT' };
  const payload = {
    sub: userId,
    login,
    iat: Math.floor(Date.now() / 1000),
    exp: Math.floor(Date.now() / 1000) + (7 * 24 * 60 * 60), // 7 days
  };

  const encoder = new TextEncoder();
  const key = await crypto.subtle.importKey(
    'raw',
    encoder.encode(env.JWT_SECRET),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign']
  );

  const headerB64 = btoa(JSON.stringify(header));
  const payloadB64 = btoa(JSON.stringify(payload));
  const data = `${headerB64}.${payloadB64}`;

  const signature = await crypto.subtle.sign('HMAC', key, encoder.encode(data));
  const signatureB64 = btoa(String.fromCharCode(...new Uint8Array(signature)));

  return `${data}.${signatureB64}`;
}

function bytesToHex(bytes) {
  return Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join('');
}

function redirectWithError(env, message) {
  const errorUrl = new URL(`${env.ALLOWED_ORIGIN}/registry`);
  errorUrl.searchParams.set('error', message);
  return new Response(null, {
    status: 302,
    headers: { 'Location': errorUrl.toString() },
  });
}
