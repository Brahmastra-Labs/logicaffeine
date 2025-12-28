-- LOGOS Package Registry Schema
-- Phase 39: The Cloudflare Registry

-- Users table (GitHub OAuth)
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,              -- GitHub user ID (as string)
    github_login TEXT NOT NULL UNIQUE,
    github_name TEXT,
    email TEXT,
    avatar_url TEXT,
    is_admin INTEGER DEFAULT 0,       -- 1 = admin (can verify packages)
    is_banned INTEGER DEFAULT 0,      -- 1 = banned (cannot publish)
    ban_reason TEXT,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_users_login ON users(github_login);

-- API tokens for CLI authentication
CREATE TABLE IF NOT EXISTS api_tokens (
    id TEXT PRIMARY KEY,              -- Token ID (UUID)
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,               -- User-provided name for the token
    token_hash TEXT NOT NULL,         -- SHA-256 hash of the actual token
    scopes TEXT DEFAULT 'publish',    -- Comma-separated: publish, yank, admin
    last_used_at TEXT,
    expires_at TEXT,                  -- NULL = never expires
    created_at TEXT DEFAULT (datetime('now')),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_tokens_user ON api_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_tokens_hash ON api_tokens(token_hash);

-- Packages table
CREATE TABLE IF NOT EXISTS packages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,        -- Package name (lowercase, alphanumeric, hyphens)
    owner_id TEXT NOT NULL,           -- User who owns this package
    description TEXT,
    repository TEXT,                  -- GitHub repo URL
    homepage TEXT,
    license TEXT,
    keywords TEXT,                    -- JSON array of keywords
    is_verified INTEGER DEFAULT 0,    -- 1 = official/std package (admin-only)
    downloads INTEGER DEFAULT 0,      -- Total download count
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now')),
    FOREIGN KEY (owner_id) REFERENCES users(id)
);

CREATE INDEX IF NOT EXISTS idx_packages_name ON packages(name);
CREATE INDEX IF NOT EXISTS idx_packages_owner ON packages(owner_id);
CREATE INDEX IF NOT EXISTS idx_packages_downloads ON packages(downloads DESC);

-- Package versions
CREATE TABLE IF NOT EXISTS versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    version TEXT NOT NULL,            -- Semver string (1.0.0, 0.1.0-beta.1)
    tarball_key TEXT NOT NULL,        -- R2 object key
    tarball_sha256 TEXT NOT NULL,     -- SHA-256 hash of tarball
    tarball_size INTEGER NOT NULL,    -- Size in bytes
    entry_point TEXT DEFAULT 'src/main.lg',
    readme TEXT,                      -- README content (markdown)
    changelog TEXT,                   -- Version-specific changelog
    dependencies TEXT,                -- JSON: {"math": "^1.0", "io": "~2.0"}
    logos_version TEXT,               -- Minimum LOGOS version required
    published_at TEXT DEFAULT (datetime('now')),
    yanked INTEGER DEFAULT 0,         -- 1 = yanked (hidden but downloadable)
    FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE,
    UNIQUE(package_id, version)
);

CREATE INDEX IF NOT EXISTS idx_versions_package ON versions(package_id);
CREATE INDEX IF NOT EXISTS idx_versions_published ON versions(published_at DESC);

-- Package collaborators (shared ownership)
CREATE TABLE IF NOT EXISTS collaborators (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    user_id TEXT NOT NULL,
    role TEXT DEFAULT 'maintainer',   -- owner, maintainer, publisher
    created_at TEXT DEFAULT (datetime('now')),
    FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    UNIQUE(package_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_collaborators_package ON collaborators(package_id);
CREATE INDEX IF NOT EXISTS idx_collaborators_user ON collaborators(user_id);

-- Download statistics (aggregated daily)
CREATE TABLE IF NOT EXISTS download_stats (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    version_id INTEGER,               -- NULL = aggregate for entire package
    date TEXT NOT NULL,               -- YYYY-MM-DD
    count INTEGER DEFAULT 0,
    FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE,
    UNIQUE(package_id, version_id, date)
);

CREATE INDEX IF NOT EXISTS idx_downloads_date ON download_stats(date);
CREATE INDEX IF NOT EXISTS idx_downloads_package ON download_stats(package_id);
