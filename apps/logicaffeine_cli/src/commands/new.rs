//! `largo new` / `largo init` — project scaffolding.

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::project::manifest::Manifest;
use crate::ui::CliError;

/// The AGENTS.md every scaffolded project carries: the check/run/test loop,
/// the five classic mistakes, and the MCP registration line — written for
/// the AI agents that are LOGOS's first users. Byte-locked by
/// `tests/agents_md.rs`, which also re-parses every `largo` line in it
/// against the real CLI.
const AGENTS_MD: &str = include_str!("../../templates/AGENTS.md");

/// Validate a project/package name: ASCII alphanumerics, `-` and `_`,
/// starting with a letter. Everything else breaks a downstream layer —
/// path separators split the directory, quotes corrupt the generated wasm
/// host shim, leading dashes are shell footguns.
fn validate_project_name(name: &str) -> Result<(), CliError> {
    let ok = !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(CliError::with_hint(
            format!("invalid project name {name:?}"),
            "a name starts with a letter and uses only letters, digits, `-` and `_` (like a crate name)",
        ))
    }
}

/// The `--web` backend entry: one `## Route` per path and the `Serve` statement that runs them.
/// `NAME` is substituted with the project name (LOGOS `{…}` is string interpolation, so the name
/// is baked in at scaffold time, never left as a brace).
const WEB_MAIN_LG: &str = r#"# NAME

The NAME backend: one `## Route` per path, and `Serve on port 8080.` to run
them. Under `largo serve`, largo owns the public port, serves `pages/` and
`static/`, and hands this program every request they do not answer. In
production, `LOGOS_PORT` (or `PORT`) overrides the port below.

A route's answer type is its content type: `Text` is served as text, `View`
is rendered as HTML.

## Route GET /api/hello -> Text:
    Return "Hello from NAME!".

## Route GET /api/about -> View:
    <h1>NAME</h1>
    <p>A pure-LOGOS site: backend, pages, and assets in one project.</p>

## Main

Serve on port 8080.
"#;

/// The front page every site scaffold writes: `## Page /`, one `## Component` it places, and a
/// `## Test` that drives it — the canonical shape of a page, and a project whose first
/// `largo test` has something to run.
const WEB_PAGE_LG: &str = r##"# App

The front page - a LOGOS page compiled to WASM and served at `/`.
Edit it while `largo serve` runs and the browser reloads.

`## Page /` says where it serves, and a page mounts itself - there is no entry
point to write. `State:` is what it knows, `Actions:` is what it does, `View:`
is real HTML with `{...}` holes, and `Style:` is real CSS scoped to it. One
click is one turn - the action runs, the page paints once, and if the action
fails the state goes back to what it was.

## Note

`Card` is a piece the page places. `title` is a prop - the page passes it, and
the card only reads it. `{children}` is where the markup the page writes
between `<Card>` and `</Card>` goes.

## Component Card (title: Text)

View:
    <section class="card">
      <h1>{title}</h1>
      {children}
    </section>

## Page /

State:
    Let clicks be 0.

Actions:
    To bump:
        Set clicks to clicks + 1.

View:
    <Card title="NAME is alive">
      <p>This page is LOGOS compiled to WASM.</p>
      <button id="more" on-click={bump}>Click me</button>
      <p id="count">Clicks so far: {clicks}</p>
    </Card>

## Test: the button counts clicks

Click "#more".
Click "#more".
Expect that the text of "#count" is equal to "Clicks so far: 2".
"##;

/// The `--web` DESIGN TOKENS: `static/theme.css`, linked into every page ahead of `style.css`.
///
/// ★ THE STYLESHEET USED FOUR TOKENS NOTHING DEFINED. `--border`, `--radius`, `--shadow` and
/// `--accent` were referenced by the scaffolded card and declared nowhere, so every declaration
/// using one was SILENTLY DROPPED — the card had no border, no rounded corners and no shadow, and
/// its heading had no colour. The first CSS anybody sees of LOGOS did nothing.
///
/// And CSS never says so: `var(--nope)` with no fallback makes the declaration invalid at
/// computed-value time, with no error, no warning and no console line. Same shape as the
/// scaffolded counter that never re-rendered — it LOOKS wired up.
///
/// They live in their OWN file because a project owns its theme: changing the accent is one line
/// here rather than a hunt through selectors, and a project that wants none of it deletes the
/// file. It is linked BEFORE `style.css` so a project's own rules win over the theme they are
/// customising.
const WEB_THEME_CSS: &str = r#"/* NAME - design tokens. Yours to change, or to delete entirely. */
:root {
  --accent: #cc785c;
  --ink: #1a1a18;
  --paper: #faf9f5;
  --muted: #6b6b66;
  --border: #e3e1d9;
  --radius: 10px;
  --shadow: 0 1px 3px rgba(0, 0, 0, .06), 0 8px 24px rgba(0, 0, 0, .04);
}

/* Dark is a preference a visitor already expressed; honour it rather than asking again. */
@media (prefers-color-scheme: dark) {
  :root {
    --ink: #f2f0e8;
    --paper: #14140f;
    --muted: #a3a29a;
    --border: #2e2e28;
    --shadow: 0 1px 3px rgba(0, 0, 0, .5), 0 8px 24px rgba(0, 0, 0, .35);
  }
}

body {
  margin: 0;
  background: var(--paper);
  color: var(--ink);
  font: 16px/1.6 ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
}
"#;

/// The `--web` stylesheet: `static/**` is served at the site root, so this
/// file is `/style.css` (and `largo serve` links top-level css into every
/// built page).
const WEB_STYLE_CSS: &str = r#"/* NAME - static assets are served at the site root (this file is /style.css). */
.card {
  max-width: 40rem;
  margin: 3rem auto;
  padding: 1.5rem 1.75rem;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow);
}
.card h1 { color: var(--accent); }
"#;

/// What `largo new` scaffolds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scaffold {
    /// A program: `src/main.lg`.
    Program,
    /// A full-stack site: a `## Route` backend in `src/main.lg`, `pages/`, `static/`.
    Web,
    /// A site: `pages/` and `static/`, and no program at all — a page mounts itself.
    Site,
}

/// A site's manifest: the ordinary one, without an `entry` — a site has no program, and naming a
/// file that does not exist would make every command look for it.
fn site_manifest(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut table = toml::Table::try_from(Manifest::new(name))?;
    if let Some(package) = table.get_mut("package").and_then(|p| p.as_table_mut()) {
        package.remove("entry");
    }
    Ok(toml::to_string_pretty(&table)?)
}

/// The front page, the theme and the stylesheet every site scaffold writes.
fn write_site_files(project_dir: &std::path::Path, name: &str) -> std::io::Result<()> {
    fs::create_dir_all(project_dir.join("pages"))?;
    fs::write(
        project_dir.join("pages/app.lg"),
        WEB_PAGE_LG.replace("NAME", name),
    )?;
    fs::create_dir_all(project_dir.join("static"))?;
    fs::write(
        project_dir.join("static/theme.css"),
        WEB_THEME_CSS.replace("NAME", name),
    )?;
    fs::write(
        project_dir.join("static/style.css"),
        WEB_STYLE_CSS.replace("NAME", name),
    )?;
    Ok(())
}

/// Handle `largo new <name> [--web | --site]`: scaffold a fresh project directory. `--web` and
/// `--site` are layouts by CONVENTION — `pages/` served as pages, `static/` served verbatim — with
/// no new manifest keys; the plain scaffold's files stay byte-identical.
pub(crate) fn cmd_new(name: &str, scaffold: Scaffold) -> Result<(), Box<dyn std::error::Error>> {
    validate_project_name(name)?;
    let project_dir = PathBuf::from(name);

    if project_dir.exists() {
        return Err(format!("Directory '{}' already exists", project_dir.display()).into());
    }
    fs::create_dir_all(&project_dir)?;

    match scaffold {
        Scaffold::Program => {
            fs::create_dir_all(project_dir.join("src"))?;
            fs::write(
                project_dir.join("Largo.toml"),
                Manifest::new(name).to_toml()?,
            )?;
            let main_lg = r#"# Main

A simple LOGOS program.

## Main

Show "Hello, world!".
"#;
            fs::write(project_dir.join("src/main.lg"), main_lg)?;
        }
        Scaffold::Web => {
            fs::create_dir_all(project_dir.join("src"))?;
            fs::write(
                project_dir.join("Largo.toml"),
                Manifest::new(name).to_toml()?,
            )?;
            fs::write(
                project_dir.join("src/main.lg"),
                WEB_MAIN_LG.replace("NAME", name),
            )?;
            write_site_files(&project_dir, name)?;
        }
        Scaffold::Site => {
            fs::write(project_dir.join("Largo.toml"), site_manifest(name)?)?;
            write_site_files(&project_dir, name)?;
        }
    }

    // Write .gitignore
    fs::write(project_dir.join(".gitignore"), "/target\n")?;

    // Write AGENTS.md — the language, taught to the agent that lands here.
    fs::write(project_dir.join("AGENTS.md"), AGENTS_MD)?;

    println!("Created LOGOS project '{}'", name);
    println!("  cd {}", project_dir.display());
    println!(
        "  largo {}",
        if scaffold == Scaffold::Program {
            "run"
        } else {
            "serve"
        }
    );

    Ok(())
}

/// Handle `largo init [--name]`: scaffold in the current directory.
pub(crate) fn cmd_init(name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_name = name
        .map(String::from)
        .or_else(|| {
            current_dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
        })
        .unwrap_or_else(|| "project".to_string());
    validate_project_name(&project_name)?;

    if current_dir.join("Largo.toml").exists() {
        return Err("Largo.toml already exists".into());
    }

    // Create src directory if needed
    fs::create_dir_all(current_dir.join("src"))?;

    // Write Largo.toml
    let manifest = Manifest::new(&project_name);
    fs::write(current_dir.join("Largo.toml"), manifest.to_toml()?)?;

    // Write src/main.lg if it doesn't exist
    let main_path = current_dir.join("src/main.lg");
    if !main_path.exists() {
        let main_lg = r#"# Main

A simple LOGOS program.

## Main

Show "Hello, world!".
"#;
        fs::write(main_path, main_lg)?;
    }

    // Write AGENTS.md if the project doesn't already carry one.
    let agents_path = current_dir.join("AGENTS.md");
    if !agents_path.exists() {
        fs::write(agents_path, AGENTS_MD)?;
    }

    println!("Initialized LOGOS project '{}'", project_name);

    Ok(())
}
