/**
 * LOGOS Package Registry - Package Routes
 * GET /packages - List all packages
 * GET /packages/:name - Package metadata
 * GET /packages/:name/:version - Version metadata
 */

import { jsonResponse } from '../index.js';

/**
 * GET /packages - List all packages
 * Query params: ?page=1&limit=20&search=geometry&verified=true&sort=downloads
 */
export async function handlePackagesList(request, env) {
  const url = new URL(request.url);
  const page = Math.max(1, parseInt(url.searchParams.get('page')) || 1);
  const limit = Math.min(Math.max(1, parseInt(url.searchParams.get('limit')) || 20), 100);
  const search = url.searchParams.get('search');
  const verified = url.searchParams.get('verified');
  const sort = url.searchParams.get('sort') || 'downloads'; // downloads, name, newest
  const offset = (page - 1) * limit;

  let query = `
    SELECT
      p.id, p.name, p.description, p.is_verified, p.downloads, p.created_at,
      p.repository, p.license, p.keywords,
      u.github_login as owner_login, u.avatar_url as owner_avatar,
      (SELECT version FROM versions WHERE package_id = p.id ORDER BY published_at DESC LIMIT 1) as latest_version,
      (SELECT published_at FROM versions WHERE package_id = p.id ORDER BY published_at DESC LIMIT 1) as latest_published
    FROM packages p
    JOIN users u ON p.owner_id = u.id
    WHERE 1=1
  `;
  const params = [];

  if (search) {
    query += ` AND (p.name LIKE ? OR p.description LIKE ? OR p.keywords LIKE ?)`;
    const searchPattern = `%${search}%`;
    params.push(searchPattern, searchPattern, searchPattern);
  }
  if (verified === 'true') {
    query += ` AND p.is_verified = 1`;
  }

  // Sorting - verified packages always first
  query += ` ORDER BY p.is_verified DESC`;
  switch (sort) {
    case 'name':
      query += `, p.name ASC`;
      break;
    case 'newest':
      query += `, p.created_at DESC`;
      break;
    case 'downloads':
    default:
      query += `, p.downloads DESC`;
      break;
  }

  query += ` LIMIT ? OFFSET ?`;
  params.push(limit, offset);

  const results = await env.DB.prepare(query).bind(...params).all();

  // Get total count for pagination
  let countQuery = `SELECT COUNT(*) as total FROM packages p WHERE 1=1`;
  const countParams = [];
  if (search) {
    countQuery += ` AND (p.name LIKE ? OR p.description LIKE ? OR p.keywords LIKE ?)`;
    const searchPattern = `%${search}%`;
    countParams.push(searchPattern, searchPattern, searchPattern);
  }
  if (verified === 'true') {
    countQuery += ` AND p.is_verified = 1`;
  }
  const countResult = await env.DB.prepare(countQuery).bind(...countParams).first();

  return jsonResponse({
    packages: results.results.map(p => ({
      name: p.name,
      description: p.description,
      latest_version: p.latest_version,
      latest_published: p.latest_published,
      owner: p.owner_login,
      owner_avatar: p.owner_avatar,
      repository: p.repository,
      license: p.license,
      keywords: p.keywords ? JSON.parse(p.keywords) : [],
      verified: p.is_verified === 1,
      downloads: p.downloads,
      created_at: p.created_at,
    })),
    pagination: {
      page,
      limit,
      total: countResult.total,
      pages: Math.ceil(countResult.total / limit),
    },
  });
}

/**
 * GET /packages/:name - Package metadata
 */
export async function handlePackageInfo(request, env, name) {
  const pkg = await env.DB.prepare(`
    SELECT p.*, u.github_login as owner_login, u.avatar_url as owner_avatar
    FROM packages p
    JOIN users u ON p.owner_id = u.id
    WHERE p.name = ?
  `).bind(name.toLowerCase()).first();

  if (!pkg) {
    return jsonResponse({ error: 'Package not found' }, 404);
  }

  // Get all versions
  const versions = await env.DB.prepare(`
    SELECT version, published_at, tarball_size, yanked, readme
    FROM versions
    WHERE package_id = ?
    ORDER BY published_at DESC
  `).bind(pkg.id).all();

  // Get latest version's readme
  const latestVersion = versions.results[0];

  // Get collaborators
  const collaborators = await env.DB.prepare(`
    SELECT u.github_login, u.avatar_url, c.role
    FROM collaborators c
    JOIN users u ON c.user_id = u.id
    WHERE c.package_id = ?
  `).bind(pkg.id).all();

  return jsonResponse({
    name: pkg.name,
    description: pkg.description,
    owner: pkg.owner_login,
    owner_avatar: pkg.owner_avatar,
    repository: pkg.repository,
    homepage: pkg.homepage,
    license: pkg.license,
    keywords: pkg.keywords ? JSON.parse(pkg.keywords) : [],
    verified: pkg.is_verified === 1,
    downloads: pkg.downloads,
    created_at: pkg.created_at,
    updated_at: pkg.updated_at,
    readme: latestVersion?.readme || null,
    collaborators: collaborators.results.map(c => ({
      login: c.github_login,
      avatar: c.avatar_url,
      role: c.role,
    })),
    versions: versions.results.map(v => ({
      version: v.version,
      published_at: v.published_at,
      size: v.tarball_size,
      yanked: v.yanked === 1,
    })),
  });
}

/**
 * GET /packages/:name/:version - Specific version metadata
 */
export async function handleVersionInfo(request, env, name, version) {
  const result = await env.DB.prepare(`
    SELECT v.*, p.name, p.is_verified, p.description,
           u.github_login as owner_login
    FROM versions v
    JOIN packages p ON v.package_id = p.id
    JOIN users u ON p.owner_id = u.id
    WHERE p.name = ? AND v.version = ?
  `).bind(name.toLowerCase(), version).first();

  if (!result) {
    return jsonResponse({ error: 'Version not found' }, 404);
  }

  return jsonResponse({
    name: result.name,
    version: result.version,
    description: result.description,
    owner: result.owner_login,
    verified: result.is_verified === 1,
    tarball_sha256: result.tarball_sha256,
    tarball_size: result.tarball_size,
    entry_point: result.entry_point,
    dependencies: result.dependencies ? JSON.parse(result.dependencies) : {},
    logos_version: result.logos_version,
    readme: result.readme,
    changelog: result.changelog,
    published_at: result.published_at,
    yanked: result.yanked === 1,
    download_url: `/packages/${name}/${version}/download`,
  });
}
