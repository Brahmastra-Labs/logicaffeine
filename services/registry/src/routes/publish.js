/**
 * LOGOS Package Registry - Publish Route
 * POST /packages/publish - Publish a new package version
 */

import { jsonResponse, sha256 } from '../index.js';

// Reserved namespaces - only admins can publish
const RESERVED_PREFIXES = ['std', 'core', 'logos', 'sys'];

// Maximum tarball size (10MB)
const MAX_TARBALL_SIZE = 10 * 1024 * 1024;

/**
 * POST /packages/publish - Publish a new package version
 *
 * Body: multipart/form-data with:
 * - tarball: The .tar.gz file
 * - metadata: JSON string with package info
 *
 * Metadata format:
 * {
 *   name: string,
 *   version: string,
 *   description?: string,
 *   repository?: string,
 *   homepage?: string,
 *   license?: string,
 *   keywords?: string[],
 *   entry_point?: string,
 *   dependencies?: Record<string, string>,
 *   readme?: string,
 *   changelog?: string,
 *   logos_version?: string
 * }
 */
export async function handlePublish(request, env, user) {
  const contentType = request.headers.get('Content-Type') || '';

  if (!contentType.includes('multipart/form-data')) {
    return jsonResponse({ error: 'Expected multipart/form-data' }, 400);
  }

  let formData;
  try {
    formData = await request.formData();
  } catch (err) {
    return jsonResponse({ error: 'Invalid form data', message: err.message }, 400);
  }

  const tarball = formData.get('tarball');
  const metadataStr = formData.get('metadata');

  if (!tarball) {
    return jsonResponse({ error: 'Missing tarball file' }, 400);
  }
  if (!metadataStr) {
    return jsonResponse({ error: 'Missing metadata' }, 400);
  }

  let metadata;
  try {
    metadata = JSON.parse(metadataStr);
  } catch {
    return jsonResponse({ error: 'Invalid metadata JSON' }, 400);
  }

  // ========== Validate metadata ==========
  const { name, version, description, repository, homepage, license, keywords,
    entry_point, dependencies, readme, changelog, logos_version } = metadata;

  if (!name || typeof name !== 'string') {
    return jsonResponse({ error: 'Missing required field: name' }, 400);
  }
  if (!version || typeof version !== 'string') {
    return jsonResponse({ error: 'Missing required field: version' }, 400);
  }

  // Normalize name to lowercase
  const normalizedName = name.toLowerCase();

  // Validate package name format
  if (!/^[a-z][a-z0-9_-]{0,63}$/.test(normalizedName)) {
    return jsonResponse({
      error: 'Invalid package name',
      message: 'Must be lowercase, start with a letter, contain only a-z, 0-9, _, - and be max 64 chars'
    }, 400);
  }

  // Validate semver format (basic check)
  if (!/^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9_.]+)?(\+[a-zA-Z0-9_.]+)?$/.test(version)) {
    return jsonResponse({
      error: 'Invalid version',
      message: 'Must be valid semver (e.g., 1.0.0, 0.1.0-beta.1)'
    }, 400);
  }

  // ========== Security: Check reserved namespace ==========
  const isReserved = RESERVED_PREFIXES.some(prefix =>
    normalizedName === prefix || normalizedName.startsWith(`${prefix}-`) || normalizedName.startsWith(`${prefix}_`)
  );

  if (isReserved && !user.is_admin) {
    return jsonResponse({
      error: 'Reserved namespace',
      message: `Packages starting with ${RESERVED_PREFIXES.join(', ')} are reserved for official use`
    }, 403);
  }

  // ========== Get tarball as ArrayBuffer ==========
  let tarballBuffer;
  try {
    tarballBuffer = await tarball.arrayBuffer();
  } catch (err) {
    return jsonResponse({ error: 'Failed to read tarball', message: err.message }, 400);
  }

  const tarballSize = tarballBuffer.byteLength;

  // ========== Security: Check size limit ==========
  if (tarballSize > MAX_TARBALL_SIZE) {
    return jsonResponse({
      error: 'Package too large',
      message: `Maximum size is ${MAX_TARBALL_SIZE / 1024 / 1024}MB, got ${(tarballSize / 1024 / 1024).toFixed(2)}MB`
    }, 413);
  }

  // ========== Security: Validate gzip format ==========
  const bytes = new Uint8Array(tarballBuffer);
  if (bytes[0] !== 0x1f || bytes[1] !== 0x8b) {
    return jsonResponse({
      error: 'Invalid tarball format',
      message: 'File must be a valid gzip-compressed archive'
    }, 400);
  }

  // ========== Calculate SHA-256 hash ==========
  const hashBuffer = await crypto.subtle.digest('SHA-256', tarballBuffer);
  const tarballSha256 = Array.from(new Uint8Array(hashBuffer))
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');

  // ========== Check if package exists ==========
  let pkg = await env.DB.prepare('SELECT * FROM packages WHERE name = ?')
    .bind(normalizedName).first();

  if (pkg) {
    // Verify ownership or collaborator status
    const isOwner = pkg.owner_id === user.id;
    const collab = await env.DB.prepare(
      'SELECT * FROM collaborators WHERE package_id = ? AND user_id = ?'
    ).bind(pkg.id, user.id).first();

    if (!isOwner && !collab) {
      return jsonResponse({
        error: 'Permission denied',
        message: 'You do not have permission to publish to this package'
      }, 403);
    }

    // Check version doesn't already exist
    const existingVersion = await env.DB.prepare(
      'SELECT * FROM versions WHERE package_id = ? AND version = ?'
    ).bind(pkg.id, version).first();

    if (existingVersion) {
      return jsonResponse({
        error: 'Version exists',
        message: `Version ${version} of ${normalizedName} already exists. Bump the version number.`
      }, 409);
    }

    // Update package metadata if provided
    if (description || repository || homepage || license || keywords) {
      await env.DB.prepare(`
        UPDATE packages SET
          description = COALESCE(?, description),
          repository = COALESCE(?, repository),
          homepage = COALESCE(?, homepage),
          license = COALESCE(?, license),
          keywords = COALESCE(?, keywords),
          updated_at = datetime('now')
        WHERE id = ?
      `).bind(
        description || null,
        repository || null,
        homepage || null,
        license || null,
        keywords ? JSON.stringify(keywords) : null,
        pkg.id
      ).run();
    }
  } else {
    // Create new package
    const result = await env.DB.prepare(`
      INSERT INTO packages (name, owner_id, description, repository, homepage, license, keywords)
      VALUES (?, ?, ?, ?, ?, ?, ?)
    `).bind(
      normalizedName,
      user.id,
      description || null,
      repository || null,
      homepage || null,
      license || null,
      keywords ? JSON.stringify(keywords) : null
    ).run();

    pkg = { id: result.meta.last_row_id, name: normalizedName };
  }

  // ========== Upload tarball to R2 ==========
  const tarballKey = `${normalizedName}/${version}/${normalizedName}-${version}.tar.gz`;

  try {
    await env.PACKAGES.put(tarballKey, tarballBuffer, {
      customMetadata: {
        sha256: tarballSha256,
        published_by: user.github_login,
        published_at: new Date().toISOString(),
      },
    });
  } catch (err) {
    console.error('R2 upload error:', err);
    return jsonResponse({ error: 'Failed to upload package', message: 'Storage error' }, 500);
  }

  // ========== Create version record ==========
  await env.DB.prepare(`
    INSERT INTO versions (
      package_id, version, tarball_key, tarball_sha256, tarball_size,
      entry_point, readme, changelog, dependencies, logos_version
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).bind(
    pkg.id,
    version,
    tarballKey,
    tarballSha256,
    tarballSize,
    entry_point || 'src/main.lg',
    readme || null,
    changelog || null,
    dependencies ? JSON.stringify(dependencies) : null,
    logos_version || null
  ).run();

  // Update package's updated_at
  await env.DB.prepare(`
    UPDATE packages SET updated_at = datetime('now') WHERE id = ?
  `).bind(pkg.id).run();

  return jsonResponse({
    success: true,
    package: normalizedName,
    version,
    sha256: tarballSha256,
    size: tarballSize,
    download_url: `/packages/${normalizedName}/${version}/download`,
  }, 201);
}
