/**
 * LOGOS Package Registry - Download Route
 * GET /packages/:name/:version/download - Download tarball
 */

import { jsonResponse, CORS_HEADERS } from '../index.js';

/**
 * GET /packages/:name/:version/download - Download tarball
 */
export async function handleDownload(request, env, ctx, name, version) {
  // Normalize name
  const normalizedName = name.toLowerCase();

  // Look up version
  const result = await env.DB.prepare(`
    SELECT v.tarball_key, v.tarball_sha256, v.tarball_size, v.yanked, p.id as package_id
    FROM versions v
    JOIN packages p ON v.package_id = p.id
    WHERE p.name = ? AND v.version = ?
  `).bind(normalizedName, version).first();

  if (!result) {
    return jsonResponse({ error: 'Version not found' }, 404);
  }

  // Yanked packages are still downloadable but with warning header
  const isYanked = result.yanked === 1;
  if (isYanked) {
    console.warn(`Downloading yanked package: ${normalizedName}@${version}`);
  }

  // Get from R2
  const object = await env.PACKAGES.get(result.tarball_key);
  if (!object) {
    console.error(`R2 object not found: ${result.tarball_key}`);
    return jsonResponse({ error: 'Tarball not found in storage' }, 500);
  }

  // Increment download count (non-blocking)
  ctx.waitUntil(incrementDownloads(env, result.package_id));

  return new Response(object.body, {
    headers: {
      ...CORS_HEADERS,
      'Content-Type': 'application/gzip',
      'Content-Length': result.tarball_size.toString(),
      'Content-Disposition': `attachment; filename="${normalizedName}-${version}.tar.gz"`,
      'X-Checksum-SHA256': result.tarball_sha256,
      'X-Package-Yanked': isYanked ? 'true' : 'false',
      'Cache-Control': 'public, max-age=31536000, immutable', // Packages are immutable
    },
  });
}

/**
 * Increment download counters (runs in background)
 */
async function incrementDownloads(env, packageId) {
  const today = new Date().toISOString().split('T')[0]; // YYYY-MM-DD

  try {
    // Upsert daily stats (package-level)
    await env.DB.prepare(`
      INSERT INTO download_stats (package_id, version_id, date, count)
      VALUES (?, NULL, ?, 1)
      ON CONFLICT(package_id, version_id, date) DO UPDATE SET count = count + 1
    `).bind(packageId, today).run();

    // Increment total on package
    await env.DB.prepare(`
      UPDATE packages SET downloads = downloads + 1 WHERE id = ?
    `).bind(packageId).run();
  } catch (err) {
    // Non-critical, log but don't fail the download
    console.error('Failed to update download stats:', err);
  }
}
