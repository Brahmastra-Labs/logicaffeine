# LOGOS Blog Crate Plan

A standalone Rust/Dioxus blog crate with federation, interactive LOGOS code blocks, and world-class SEO.

## Architecture Overview

```
logos_blog/                    # Standalone workspace crate
├── worker/                    # Cloudflare Worker API
│   ├── wrangler.toml         # D1 + R2 bindings
│   └── src/
│       ├── index.js          # Router (follows registry pattern)
│       └── routes/           # posts, auth, federation, media, seo
├── src/
│   ├── lib.rs                # Public API: BlogRoutes(), BlogConfig
│   ├── types.rs              # Post, Author, Tag, FederationLink
│   ├── federation/           # Private tokens + public discovery
│   ├── components/           # LogosBlock, PostCard, SeoMeta
│   ├── pages/                # home, post, tag, archive, search
│   └── admin/                # dashboard, editor, media manager
└── migrations/               # D1 schema
```

## Key Decisions

| Aspect | Choice | Rationale |
|--------|--------|-----------|
| Storage | R2 + D1 | Matches registry pattern, scalable |
| Auth | GitHub OAuth | Reuse registry auth.js |
| Code blocks | LOGOS + Rust dual display | Extends GuideCodeBlock |
| Federation | Private tokens + public discovery | Both controlled sharing and open federation |
| SEO | Pre-rendered HTML + JSON-LD | Crawler-optimized |

## D1 Schema

```sql
-- Core content
posts (
    id, slug, title, content_key, author_id,
    status,              -- draft | published | archived
    visibility,          -- local | syndicatable | submittable
    published_at, meta_title, meta_description
)

tags (id, name, slug, color, post_count)
post_tags (post_id, tag_id)
media (id, key, filename, mime_type, width, height, alt_text)

-- Users & Roles (GitHub OAuth)
users (id, github_login, github_name, avatar_url, role, invited_by, created_at)

-- Federation (see detailed schema in Federation section)
trust_relationships (id, blog_url, trust_level, author_id, sync_tags)
submissions (id, remote_blog_url, remote_post_id, author_id, status)
syndicated_posts (id, local_post_id, origin_blog_url, canonical_url)
```

## Roles & Permissions

### Role Hierarchy

| Role | Create | Edit Own | Edit Any | Publish | Delete | Manage Users | Settings |
|------|--------|----------|----------|---------|--------|--------------|----------|
| **Owner** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| **Admin** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ |
| **Editor** | ✓ | ✓ | ✓ | ✓ | own only | ✗ | ✗ |
| **Author** | ✓ | ✓ | ✗ | ✓ | own only | ✗ | ✗ |
| **Contributor** | ✓ | ✓ | ✗ | ✗ (drafts only) | own only | ✗ | ✗ |

### Role Definitions

```rust
enum Role {
    Owner,        // Full control, transfers ownership, deletes blog
    Admin,        // Manages users + all content, can't delete blog
    Editor,       // Edits any post, publishes, can't manage users
    Author,       // Creates + publishes own posts only
    Contributor,  // Creates drafts, requires approval to publish
}
```

### Permission Logic

```
┌─────────────────────────────────────────────────────────────┐
│                    WHO CAN DO WHAT                          │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Create Post:     Everyone (Owner → Contributor)            │
│                                                             │
│  Edit Post:                                                 │
│  ├── Own post:    Everyone                                  │
│  └── Any post:    Owner, Admin, Editor                      │
│                                                             │
│  Publish Post:                                              │
│  ├── Own post:    Owner, Admin, Editor, Author              │
│  ├── Any post:    Owner, Admin, Editor                      │
│  └── Contributor: Cannot publish (creates drafts only)      │
│                                                             │
│  Delete Post:                                               │
│  ├── Own post:    Everyone                                  │
│  └── Any post:    Owner, Admin                              │
│                                                             │
│  Manage Users:    Owner, Admin                              │
│  ├── Invite:      Can invite roles ≤ their own              │
│  ├── Change role: Can change roles < their own              │
│  └── Remove:      Can remove roles < their own              │
│                                                             │
│  Blog Settings:   Owner only                                │
│  ├── Federation settings                                    │
│  ├── Trust relationships                                    │
│  └── Delete blog                                            │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Database Schema (Roles)

```sql
CREATE TABLE users (
    id TEXT PRIMARY KEY,              -- GitHub user ID
    github_login TEXT NOT NULL UNIQUE,
    github_name TEXT,
    avatar_url TEXT,
    role TEXT NOT NULL,               -- owner, admin, editor, author, contributor
    invited_by TEXT REFERENCES users(id),
    created_at TEXT DEFAULT (datetime('now'))
);

-- Audit log for permission changes
CREATE TABLE role_changes (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    changed_by TEXT NOT NULL REFERENCES users(id),
    old_role TEXT,
    new_role TEXT,
    action TEXT NOT NULL,             -- invited, promoted, demoted, removed
    created_at TEXT DEFAULT (datetime('now'))
);
```

### API Endpoints (User Management)

```
# User Management (Admin+ required)
GET    /admin/users              # List all users with roles
POST   /admin/users/invite       # Invite user by GitHub login
PUT    /admin/users/:id/role     # Change user's role
DELETE /admin/users/:id          # Remove user from blog

# Self
GET    /admin/me                 # Current user info + permissions
```

### Admin Dashboard: Team Tab

```
┌─────────────────────────────────────────────────────────────┐
│  Dashboard  │  Posts  │  Queue  │  Trust  │  Team  │  Media │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Team Tab (Admin+ only):                                    │
│  ┌─────────────────────────────────────────────────────┐   │
│  │ [Invite User]                                        │   │
│  ├─────────────────────────────────────────────────────┤   │
│  │ @tristen          Owner      —                       │   │
│  │ @alice            Admin      [Role ▼] [Remove]       │   │
│  │ @bob              Editor     [Role ▼] [Remove]       │   │
│  │ @carol            Author     [Role ▼] [Remove]       │   │
│  │ @dave             Contributor [Role ▼] [Remove]      │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
│  Invite Modal:                                              │
│  ┌─────────────────────────────────────────────────────┐   │
│  │ GitHub username: [@_______________]                  │   │
│  │ Role: [Author ▼]                                     │   │
│  │                          [Cancel] [Send Invite]      │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

## R2 Storage Layout

```
logos-blog/
├── posts/{slug}/content.md       # Markdown source
├── posts/{slug}/content.html     # Pre-rendered for crawlers
├── media/images/{year}/{month}/  # Optimized images (webp)
├── seo/sitemap.xml               # Auto-generated
└── seo/feed.xml                  # RSS feed
```

## Worker API Endpoints

**Public (no auth):**
```
GET  /posts                    # List published posts (paginated)
GET  /posts/:slug              # Single post with content
GET  /posts/:slug/content      # Raw markdown
GET  /tags                     # All tags
GET  /tags/:slug               # Posts by tag
GET  /authors/:login           # Author profile + posts
GET  /sitemap.xml              # SEO sitemap
GET  /feed.xml                 # RSS feed
GET  /feed.json                # JSON Feed
```

**Federation (see detailed endpoints in Federation section):**
```
GET  /.well-known/blog-info    # Blog metadata + acceptance mode
POST /federation/submit        # Submit post to this blog
POST /federation/sync          # Webhook for trusted peer sync
GET  /federation/posts         # Fetch posts for sync
```

**Admin (GitHub OAuth required):**
```
# Auth
GET  /admin/auth/github        # Start OAuth flow
GET  /admin/auth/callback      # OAuth callback
GET  /admin/me                 # Current user info

# Posts CRUD
GET  /admin/posts              # All posts (inc. drafts)
POST /admin/posts              # Create post
PUT  /admin/posts/:id          # Update post
DELETE /admin/posts/:id        # Delete post

# Moderation Queue
GET  /admin/queue              # Pending submissions
POST /admin/queue/:id/approve  # Approve → publish
POST /admin/queue/:id/reject   # Reject with message

# Trust Management
GET  /admin/trust              # List relationships
POST /admin/trust              # Add peer/author trust
DELETE /admin/trust/:id        # Revoke trust

# Media
GET  /admin/media              # List uploaded media
POST /admin/media              # Upload to R2
DELETE /admin/media/:key       # Delete from R2
```

## LogosBlock Component

Extends `GuideCodeBlock` pattern from `src/ui/components/guide_code_block.rs`:

```
┌─────────────────────────────────────────────────────┐
│ [LOGOS + Rust] [FOL Output] [Source Only]  [Compile]│
├─────────────────────────┬───────────────────────────┤
│ LOGOS Source            │ Compiled Rust             │
│                         │                           │
│ Every person thinks.    │ fn main() {               │
│                         │   // Generated code...    │
│                         │ }                         │
└─────────────────────────┴───────────────────────────┘
```

- Tab switching: LOGOS+Rust | FOL Output | Source Only
- Uses `compile_for_ui()` for FOL and `generate_rust_code()` for Rust
- Editable with live recompilation
- Copy button with clipboard API

## Federation & Permission Model

### Post Visibility Levels (Author Controls)

| Level | Description | Use Case |
|-------|-------------|----------|
| `local` | Only on my blog | Personal drafts, site-specific content |
| `syndicatable` | Others can link/embed with attribution | Most public posts |
| `submittable` | Can be submitted to other blogs | Cross-posting to company blog |

### Blog Acceptance Modes (Owner Controls)

| Mode | Description | Workflow |
|------|-------------|----------|
| `closed` | No external content | Solo blog |
| `moderated` | Accept submissions into queue | Author submits → Owner reviews → Approve/Reject |
| `trusted_authors` | Specific authors bypass queue | Verified contributors publish directly |
| `auto_sync` | Trusted peers auto-syndicate | Company ↔ Personal blog mirroring |

### Trust Relationships

```
┌─────────────────────────────────────────────────────────────┐
│                    TRUST HIERARCHY                          │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Blog Owner (you)                                           │
│      │                                                      │
│      ├── Trusted Peers (auto-sync)                          │
│      │       └── Posts flow both ways automatically         │
│      │                                                      │
│      ├── Trusted Authors (direct publish)                   │
│      │       └── Can publish without approval               │
│      │                                                      │
│      └── Moderation Queue (anyone can submit)               │
│              └── You approve/reject each submission         │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Submission Flow

```
Author's Blog                          Target Blog
─────────────────                      ─────────────────

1. Author writes post
   visibility: syndicatable

2. Author clicks "Submit to
   [Target Blog]"
         │
         ▼
3. POST /federation/submit ─────────►  4. Receives submission
   {                                      - If author is trusted:
     post_id,                               → Auto-publish
     author_token,                        - If moderated:
     content_hash                           → Add to queue
   }                                      - If closed:
                                            → Reject
         ◄───────────────────────────  5. Returns status
                                          {accepted, queued, rejected}

                                       6. Owner reviews queue
                                          (if moderated)
                                          - Preview post
                                          - Approve → Publish
                                          - Reject → Notify author
```

### Auto-Sync Flow (Trusted Peers)

```
Blog A (Personal)                      Blog B (Company)
─────────────────                      ─────────────────

1. New post published
   tags: [logos, tutorial]
   sync_to: [blog-b-id]
         │
         ▼
2. Webhook fires to Blog B ─────────►  3. Receives webhook
                                          - Verify signature
                                          - Fetch full content
                                          - Auto-publish with
                                            attribution link

4. Post appears on both blogs
   - Original: blog-a.com/post
   - Mirror: blog-b.com/post
   - Canonical URL → Blog A
```

### Data Structures

```rust
// Post visibility
enum PostVisibility {
    Local,           // Only on origin blog
    Syndicatable,    // Can be linked/embedded
    Submittable,     // Can be submitted to other blogs
}

// Blog's acceptance mode
enum AcceptanceMode {
    Closed,          // No external content
    Moderated,       // Queue for review
    TrustedOnly,     // Only trusted authors
}

// Submission status
enum SubmissionStatus {
    Pending,         // In moderation queue
    Approved,        // Published on target
    Rejected,        // Declined by owner
    AutoPublished,   // Trusted author/peer
}

// Trust relationship
struct TrustRelationship {
    id: String,
    blog_url: String,           // The trusted blog
    trust_level: TrustLevel,    // peer | author
    author_id: Option<String>,  // For author-level trust
    created_at: DateTime,
    created_by: String,         // Admin who granted trust
}

enum TrustLevel {
    Peer,    // Full auto-sync both ways
    Author,  // Specific author can direct-publish
}
```

### API Endpoints (Federation)

```
# Discovery
GET  /.well-known/blog-info        # Blog metadata + acceptance mode

# Submission (author → target blog)
POST /federation/submit            # Submit post for publishing
GET  /federation/submissions       # My pending submissions (author)

# Moderation (blog owner)
GET  /admin/queue                  # View submission queue
POST /admin/queue/:id/approve      # Approve submission
POST /admin/queue/:id/reject       # Reject with optional message

# Trust Management (blog owner)
GET  /admin/trust                  # List trusted peers/authors
POST /admin/trust                  # Add trust relationship
DELETE /admin/trust/:id            # Revoke trust

# Auto-Sync (between trusted peers)
POST /federation/sync              # Webhook for new posts
GET  /federation/posts?since=      # Fetch posts for sync
```

### Database Schema (Federation Tables)

```sql
-- Trust relationships
CREATE TABLE trust_relationships (
    id TEXT PRIMARY KEY,
    blog_url TEXT NOT NULL,
    trust_level TEXT NOT NULL,       -- 'peer' or 'author'
    author_id TEXT,                  -- NULL for peer trust
    sync_tags TEXT,                  -- JSON array of tags to sync
    created_at TEXT DEFAULT (datetime('now')),
    created_by TEXT NOT NULL
);

-- Submission queue
CREATE TABLE submissions (
    id TEXT PRIMARY KEY,
    remote_blog_url TEXT NOT NULL,
    remote_post_id TEXT NOT NULL,
    author_id TEXT NOT NULL,
    author_name TEXT,
    title TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    status TEXT DEFAULT 'pending',   -- pending, approved, rejected
    reviewed_at TEXT,
    reviewed_by TEXT,
    rejection_reason TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);

-- Syndicated posts (mirrors from other blogs)
CREATE TABLE syndicated_posts (
    id TEXT PRIMARY KEY,
    local_post_id TEXT REFERENCES posts(id),
    origin_blog_url TEXT NOT NULL,
    origin_post_id TEXT NOT NULL,
    origin_author TEXT NOT NULL,
    canonical_url TEXT NOT NULL,     -- Always points to origin
    synced_at TEXT DEFAULT (datetime('now'))
);
```

## SEO Strategy

1. **Meta tags** - SeoMeta component with OG + Twitter cards
2. **JSON-LD** - Article schema with author, publisher, dates
3. **Sitemap** - Auto-generated, cached in R2
4. **RSS/JSON Feed** - Standard formats
5. **Pre-rendering** - HTML in R2 for crawler detection

## Admin Dashboard

Simple CRUD interface with role-based tab visibility:

```
┌─────────────────────────────────────────────────────────────┐
│  Dashboard │ Posts │ Queue │ Trust │ Team │ Media │ Settings│
│            │       │ (Own+)│ (Own) │(Adm+)│       │ (Owner) │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Dashboard Tab (Everyone):                                  │
│  ├── Stats cards (posts, views, submissions pending)        │
│  ├── Recent activity feed                                   │
│  └── Quick actions (new post, review queue)                 │
│                                                             │
│  Posts Tab (Everyone, filtered by permission):              │
│  ├── List with status badges (draft/published/archived)     │
│  ├── Visibility indicator (local/syndicatable/submittable)  │
│  ├── "Edit" shown only if user can edit that post           │
│  └── Click → Split-pane editor with live preview            │
│                                                             │
│  Queue Tab (Owner/Admin only):                              │
│  ├── Pending submissions from other blogs                   │
│  ├── Contributor drafts awaiting approval                   │
│  ├── Preview content inline                                 │
│  ├── One-click approve → publish                            │
│  └── Reject with optional message                           │
│                                                             │
│  Trust Tab (Owner only):                                    │
│  ├── List trusted peers (auto-sync blogs)                   │
│  ├── List trusted authors (direct publish)                  │
│  ├── Add new trust relationship                             │
│  └── Revoke trust                                           │
│                                                             │
│  Team Tab (Admin+ only):                                    │
│  ├── List all users with roles                              │
│  ├── Invite new users by GitHub username                    │
│  ├── Change roles (can only modify roles < own)             │
│  └── Remove users (can only remove roles < own)             │
│                                                             │
│  Media Tab (Everyone):                                      │
│  ├── R2 browser with drag-drop upload                       │
│  ├── Image preview grid                                     │
│  └── Copy URL to clipboard                                  │
│                                                             │
│  Settings Tab (Owner only):                                 │
│  ├── Blog name, description, URL                            │
│  ├── Federation mode (closed/moderated/trusted/auto-sync)   │
│  ├── Default post visibility                                │
│  └── Danger zone: Transfer ownership, delete blog           │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Post Editor

```
┌────────────────────────────────────────────────────────────────┐
│ Title: [________________________]  [Save Draft] [Publish ▼]    │
├────────────────────────┬───────────────────────────────────────┤
│ Markdown               │ Preview                               │
│                        │                                       │
│ # My Post              │ My Post                                │
│                        │ ═══════                                │
│ Some text here.        │ Some text here.                        │
│                        │                                       │
│ ```logos               │ ┌─────────────────────────────────┐   │
│ Every person thinks.   │ │ [LOGOS+Rust] [FOL] [Compile]    │   │
│ ```                    │ │ Every person...  │ fn main()... │   │
│                        │ └─────────────────────────────────┘   │
├────────────────────────┴───────────────────────────────────────┤
│ Visibility: ○ Local  ● Syndicatable  ○ Submittable             │
│ Tags: [logos] [tutorial] [+]                                   │
│ Submit to: [Select blog...] [Submit]                           │
└────────────────────────────────────────────────────────────────┘
```

## Integration Pattern

```rust
// Consumer app
use logos_blog::{BlogRoutes, BlogConfig};

let config = BlogConfig {
    api_url: "https://blog.logicaffeine.com",
    r2_url: "https://cdn.logicaffeine.com",
    site_name: "LOGOS Blog",
};

// Mount at /blog, /blog/:slug, /blog/admin
```

## Wrangler Config

```toml
name = "logos-blog"
main = "src/index.js"
compatibility_flags = ["nodejs_compat"]

[[d1_databases]]
binding = "DB"
database_name = "logos-blog"

[[r2_buckets]]
binding = "BLOG_BUCKET"
bucket_name = "logos-blog"

# Secrets: GITHUB_CLIENT_ID, GITHUB_CLIENT_SECRET, JWT_SECRET
```

## User Experience: Personal ↔ Company Blog Mirroring

**Scenario:** Tristen has a personal blog (`tristen.dev/blog`) and the company blog (`logicaffeine.com/blog`). Some posts should appear on both.

```
┌─────────────────────────────────────────────────────────────────┐
│                    SETUP (one-time)                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ 1. Both blogs running logos_blog crate                          │
│                                                                 │
│ 2. On logicaffeine.com/blog/admin:                              │
│    → Trust Tab → Add Peer                                       │
│    → Enter: tristen.dev                                         │
│    → Trust Level: Peer (auto-sync)                              │
│    → Tags to sync: [logos, tutorial]                            │
│    → Save                                                       │
│                                                                 │
│ 3. On tristen.dev/blog/admin:                                   │
│    → Accept incoming trust request                              │
│    → Both blogs now connected                                   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                 DAILY USE: Auto-Sync Post                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ 1. Write post on tristen.dev/blog/admin                         │
│    → Title: "Understanding LOGOS Logic Mode"                    │
│    → Tags: [logos, tutorial]  ← matches sync filter             │
│    → Visibility: Syndicatable                                   │
│    → Publish                                                    │
│                                                                 │
│ 2. Webhook fires to logicaffeine.com                            │
│    → Post auto-appears on company blog                          │
│    → Marked as "via tristen.dev" with link                      │
│    → Canonical URL points to tristen.dev (SEO correct)          │
│                                                                 │
│ 3. Post now live on both:                                       │
│    → tristen.dev/blog/understanding-logos-logic-mode            │
│    → logicaffeine.com/blog/understanding-logos-logic-mode       │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                 DAILY USE: Manual Submit                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ For posts that DON'T match auto-sync tags:                      │
│                                                                 │
│ 1. Write post with tags: [personal, opinion]                    │
│    → NOT auto-synced (doesn't match [logos, tutorial])          │
│                                                                 │
│ 2. Later decide to share with company blog:                     │
│    → Click "Submit to..." → Select logicaffeine.com             │
│    → Company blog owner sees in Queue tab                       │
│    → Reviews and approves/rejects                               │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                    PERSONAL-ONLY POST                           │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ Post that should NEVER appear on company blog:                  │
│                                                                 │
│ → Visibility: Local                                             │
│ → Only visible on tristen.dev/blog                              │
│ → Cannot be submitted or synced                                 │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Implementation Order

**Phase 1: Core (MVP)**
1. Worker skeleton with wrangler.toml (D1 + R2)
2. D1 schema (posts, tags, authors, media)
3. Post CRUD endpoints
4. GitHub OAuth (adapt from registry)
5. Basic admin dashboard (posts + editor)
6. Public blog pages (home, post, tags)
7. Media upload to R2

**Phase 2: Polish**
8. LogosBlock dual-display component
9. Markdown renderer with LOGOS detection
10. SEO (meta tags, JSON-LD, sitemap, RSS)
11. Image optimization pipeline

**Phase 3: Federation**
12. Trust relationships (peer + author)
13. Submission flow (submit → queue → approve)
14. Auto-sync webhooks
15. Moderation queue UI
16. Syndicated post display (attribution, canonical)

## Critical Files to Reference

- `registry/wrangler.toml` - D1/R2 binding pattern
- `registry/src/routes/auth.js` - GitHub OAuth flow
- `src/ui/components/guide_code_block.rs` - Interactive code pattern
- `src/ui/theme.rs` - Design tokens to extend
- `src/lib.rs` - `compile_for_ui()`, `generate_rust_code()`

## Verification

1. **Unit tests** - Worker routes, federation token generation
2. **Integration** - Post creation → R2 storage → D1 metadata
3. **Manual** - Create post with LOGOS block, verify dual display
4. **SEO** - Test with Google Rich Results Test
5. **Federation** - Link two local instances, verify sync
