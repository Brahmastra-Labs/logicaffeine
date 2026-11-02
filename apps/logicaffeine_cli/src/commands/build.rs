//! `largo build` — compile the project (Rust path, wasm emit, native bundle).

use crate::commands::emit::{emit_isa_module, emit_riscv_module, emit_wasm_module};
use crate::commands::require_project_root;
use crate::commands::verify::run_verification;
use crate::project::build::{self, Backend, BuildConfig, BuildError, CargoFailureKind};
use crate::project::manifest::Manifest;
use crate::project::shape::{BuildPlan, Project};
use crate::ui::{self, CliError};

/// Attach the actionable `help:` hint that matches a build failure's class.
///
/// The raw cargo output already streamed to the terminal; this shapes the
/// closing `error:`/`help:` lines.
pub(crate) fn friendly_build_error(e: BuildError) -> Box<dyn std::error::Error> {
    match e {
        BuildError::Cargo(ref failure) => {
            let hint = match failure.kind {
                CargoFailureKind::DependencyResolution => {
                    "check the crate name and version in your `## Requires` block"
                }
                CargoFailureKind::GeneratedCode => {
                    "please report this at https://github.com/Brahmastra-Labs/logicaffeine/issues"
                }
            };
            CliError::with_hint(e.to_string(), hint).into()
        }
        BuildError::Toolchain(_) => CliError::with_hint(
            e.to_string(),
            "install Rust from https://rustup.rs — or use `largo run --interpret` / `largo build --emit wasm`, which need no Rust toolchain",
        )
        .into(),
        BuildError::CToolchain(_) => CliError::with_hint(
            e.to_string(),
            "install gcc or clang, or set CC to a C compiler",
        )
        .into(),
        BuildError::CCompiler(_) => CliError::with_hint(
            e.to_string(),
            "please report this at https://github.com/Brahmastra-Labs/logicaffeine/issues",
        )
        .into(),
        other => Box::new(other),
    }
}

/// `largo build --static` — export the site to `dist/` as plain files.
///
/// ★ WHY THIS EXISTS AS A SEPARATE VERB rather than an extra output of the normal build:
/// it needs NO backend, no cargo and no rustc. It is also what a plain `largo build` does for a
/// project that IS a site — pages and assets, no `Serve` backend — because there is no program
/// to compile; `--static` asks for the same export from a full-stack project too.
///
/// ★ THE LAYOUT IS THE DESIGN. A static host maps a URL to a file and does nothing else,
/// so a page at `/about` is written `about/index.html` — the one spelling every host
/// serves for a directory URL. Writing `dist/about` would lean on an extensionless-file
/// rule that some hosts have and some do not, and the ones that do not offer a download
/// instead of a page.
pub(crate) fn cmd_build_static() -> Result<(), Box<dyn std::error::Error>> {
    let project = Project::load(&require_project_root()?)?;
    export_site(&project, false)
}

/// Export `project`'s pages and `static/` to `dist/`. With `json`, the pages are checked first and
/// one `logos-check/1` envelope reports the result — the same diagnostics `largo check --json`
/// gives, at each page's own `file:line:col`.
fn export_site(project: &Project, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = &project.root;
    let pages_dir = project.pages_dir();
    let static_dir = project.static_dir();
    if !project.has_site() {
        return Err(format!(
            "`largo build --static` exports a website, and this project has neither a `pages/` \
             nor a `static/` directory. `largo new <name> --site` scaffolds one."
        )
        .into());
    }

    if json {
        let dep_roots = crate::project::deps::resolve_dep_roots(project_root, &project.manifest)?;
        let checked = crate::commands::check::check_pages(project, &dep_roots, true);
        if !checked.errors.is_empty() {
            println!(
                "{}",
                crate::diagnostics::check_envelope(false, &checked.diagnostics)
            );
            return Err(CliError::new(checked.errors.join("\n\n")).into());
        }
    }

    let assets = crate::commands::web_bundle::plan_site(&pages_dir, &static_dir)
        .map_err(|e| CliError::new(e.to_string()))?;

    // ★ A REBUILD REPLACES, IT DOES NOT ACCUMULATE. A page deleted from the project whose
    // file survived here would be a URL that still answers after the page is gone —
    // stale in the worst way, because nothing in the project mentions it any more.
    let dist = project_root.join("dist");
    if dist.exists() {
        std::fs::remove_dir_all(&dist)?;
    }
    std::fs::create_dir_all(&dist)?;

    let mut written = 0usize;
    for asset in &assets {
        let rel = static_export_path(&asset.path);
        let out = dist.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, &asset.bytes)?;
        written += 1;
    }

    let headers = export_headers(&assets);
    std::fs::write(dist.join("_headers"), headers.as_bytes())?;
    written += 1;

    // ★ A PATTERN HAS TO TRAVEL WITH THE SITE. The dev server and a `largo build` binary both
    // match `## Page /posts/:id` themselves; a static host cannot — it maps a URL to a file and
    // nothing else, so `_redirects` carries the rule that one document answers the whole family.
    if let Some(rules) = export_redirects(&pages_dir)? {
        std::fs::write(dist.join("_redirects"), rules.as_bytes())?;
        written += 1;
    }

    if json {
        println!("{}", crate::diagnostics::check_envelope(true, &[]));
    } else {
        println!("Exported {written} files to {}", dist.display());
        println!("Deploy the `dist/` directory to any static host.");
    }
    Ok(())
}

/// The file a URL becomes on a static host.
///
/// `/` → `index.html`; a path with no extension → `<path>/index.html`; anything else
/// (`/about.html`, `/style.css`, `/app.a1b2.wasm`) is written literally.
fn static_export_path(url: &str) -> String {
    let trimmed = url.trim_start_matches('/');
    if trimmed.is_empty() {
        return "index.html".to_string();
    }
    let last = trimmed.rsplit('/').next().unwrap_or(trimmed);
    match last.contains('.') {
        true => trimmed.to_string(),
        false => format!("{trimmed}/index.html"),
    }
}

/// A `_redirects` file naming every parameterized page, or `None` when the site has none —
/// an empty rules file in every `dist/` teaches people to ignore the file.
///
/// The format is Netlify's and Cloudflare Pages'. Their `:placeholder` matches exactly ONE
/// segment, which is the same rule `page_path_matches` and `web.lg`'s `routeParam` apply, so the
/// pattern travels verbatim rather than being translated into a `*` splat — a splat would also
/// claim `/posts/42/edit` and shadow a real page.
///
/// ★ STATUS 200, NOT 301. This is a REWRITE: the visitor's URL must stay `/posts/42`, because the
/// page reads its own `id` back out of `window.location` with `routeParam`. A redirect would send
/// the browser to `/posts/id`, where the parameter is the literal word "id".
fn export_redirects(pages_dir: &std::path::Path) -> std::io::Result<Option<String>> {
    let patterns = crate::commands::web_bundle::page_patterns(pages_dir);
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut rules = String::new();
    for (pattern, target) in &patterns {
        rules.push_str(&format!("{pattern}  {target}.html  200\n"));
    }
    Ok(Some(format!(
        "# Every `## Page` whose path has `:name` segments. A static host maps a URL to a file;\n\
         # these rules are how it learns that one document answers a whole family of URLs.\n\
         # `200` is a rewrite, not a redirect: the address bar keeps the id the page reads.\n\n{rules}"
    )))
}

/// A `_headers` file — the format Netlify and Cloudflare Pages read — carrying the same
/// cache rules the built binary sends.
///
/// ★ WITHOUT IT, CACHING IS LOST AT THE DOOR. The binary sends `Cache-Control` and an
/// `ETag` computed from the shipped bytes; a static host sends whatever its own defaults
/// say. The content-addressed module is the one that matters: a different body is a
/// different URL, so it can be cached forever — and a year of caching on one delivery
/// path and none on the other is the same site behaving differently depending on how it
/// was published.
fn export_headers(assets: &[crate::commands::web_bundle::Asset]) -> String {
    let mut out = String::from(
        "# Cache rules, matching what a `largo build` binary sends for the same files.\n\
         # Netlify and Cloudflare Pages read this file; other hosts ignore it harmlessly.\n",
    );
    // Content-addressed names (`app.<hash>.wasm`) can be immutable: a change of bytes is
    // a change of URL. Everything people LINK to is revalidated instead, or a deploy is
    // invisible until the cache expires.
    out.push_str("\n/*.wasm\n  Cache-Control: public, max-age=31536000, immutable\n");
    out.push_str("\n/*.js\n  Cache-Control: public, max-age=31536000, immutable\n");
    out.push_str("\n/*.html\n  Cache-Control: public, no-cache\n");
    out.push_str("\n/*.css\n  Cache-Control: public, no-cache\n");
    let _ = assets;
    out
}

/// Handle `largo build` with all of its flags.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_build(
    release: bool,
    verify: bool,
    lib: bool,
    target: Option<String>,
    native_functions: bool,
    emit: Option<String>,
    json: bool,
    manifest: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;

    // `--manifest`: describe the linked build (package identity, features,
    // resolved deps, transitive closure) as JSON and stop — no verify, no cargo,
    // no rustc. This is the registry-free local-closure resolution path: linking a
    // fully-local (`path`/`git`) closure reaches only the filesystem.
    if manifest {
        let m = Manifest::load(&project_root)?;
        let entry_path = crate::commands::resolve_entry_path(&project_root, &m)?;
        let build_manifest = crate::project::deps::build_manifest(&project_root, &m, &entry_path)?;
        println!("{}", build_manifest.to_json());
        return Ok(());
    }

    // What a plain build produces is the project's build plan. A SITE has no program to compile,
    // so it exports; the flags that name a program artifact (`--lib`, `--target`, `--emit`,
    // `--native-functions`) are about the entry program and take the program path below.
    let names_a_program = lib || target.is_some() || native_functions || emit.is_some();
    if !names_a_program {
        let project = Project::load(&project_root)?;
        // A shape that cannot be decided means the entry does not parse; the compile below reports
        // that at its line (and in the `--json` envelope), so it falls through.
        if let Ok(shape) = project.shape() {
            if shape.build_plan() == BuildPlan::StaticExport {
                if verify {
                    run_verification(&project_root)?;
                }
                if !json {
                    ui::info(format!(
                        "{} is {}, so `largo build` exports it to dist/ — there is no program to compile{}",
                        project.manifest.package.name,
                        shape.describe(),
                        if release { " (`--release` changes nothing for a site)" } else { "" },
                    ));
                }
                return export_site(&project, json);
            }
        }
    }

    // `--json`: pre-flight the LOGOS compile so a parse error reports as a
    // structured `logos-check/1` envelope before cargo ever runs. The human
    // rendering still follows on stderr through the normal error path.
    if json {
        let manifest = Manifest::load(&project_root)?;
        let entry_path = crate::commands::resolve_entry_path(&project_root, &manifest)?;
        let dep_roots = crate::project::deps::resolve_dep_roots(&project_root, &manifest)?;
        if let Err(e) = crate::compile::compile_project_with_deps(&entry_path, dep_roots.clone()) {
            let diagnostic = match &e {
                logicaffeine_compile::compile::CompileError::Parse(pe) => {
                    crate::diagnostics::entry_parse_diagnostic(pe, &entry_path, &dep_roots)
                }
                other => crate::diagnostics::file_failure(
                    &entry_path,
                    "build-failure",
                    &other.to_string(),
                ),
            };
            println!(
                "{}",
                crate::diagnostics::check_envelope(false, &[diagnostic])
            );
            return Err(crate::diagnostics::compile_error(&e, &entry_path, &dep_roots).into());
        }
    }

    // Run verification if requested
    if verify {
        run_verification(&project_root)?;
    }

    // `--emit wasm`: compile the entry DIRECTLY to a `.wasm` via the built-in backend — no rustc,
    // cargo, or wasm-bindgen. Bypasses the whole Cargo path; the module is self-contained (its host
    // `print_*`/`args`/… imports are supplied by any wasm runtime or a ~10-line browser shim).
    let mut backend = Backend::Rust;
    if let Some(kind) = emit.as_deref() {
        match kind {
            "wasm" => return emit_wasm_module(&project_root, false),
            "wasm-linked" => return emit_wasm_module(&project_root, true),
            "isa" => return emit_isa_module(&project_root),
            "riscv" | "risc-v" => return emit_riscv_module(&project_root),
            other => {
                backend = Backend::from_emit(other).ok_or_else(|| {
                    format!("unknown --emit target '{other}' (expected 'wasm', 'wasm-linked', 'isa', 'riscv', 'c', or 'c-bytecode')")
                })?;
            }
        }
    }

    let config = BuildConfig {
        project_dir: project_root.clone(),
        release,
        lib_mode: lib,
        target,
    };

    let result = match build::build_backend(config, backend) {
        Ok(result) => result,
        Err(e) => {
            if json {
                // The compile phase already passed pre-flight, so this is the
                // cargo phase: one build-failure diagnostic carrying the raw
                // story, never dropped.
                println!(
                    "{}",
                    crate::diagnostics::build_failure_envelope(
                        "cargo build failed",
                        &e.to_string()
                    )
                );
            }
            return Err(friendly_build_error(e));
        }
    };

    if json {
        println!("{}", crate::diagnostics::check_envelope(true, &[]));
    }
    let mode = if release { "release" } else { "debug" };
    ui::info(format!("Built {} [{}]", result.binary_path.display(), mode));

    if native_functions {
        build_native_function_bundle(&project_root)?;
    }

    Ok(())
}

/// Pre-build the AOT-native tier bundle (HOTSWAP §Axis-3): every `is exported for
/// native` function compiled to a cached cdylib under `.logos-native/`. Functions
/// outside the sound scalar subset are skipped — they keep running on VM+JIT.
pub(crate) fn build_native_function_bundle(
    project_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Manifest::load(project_root)?;
    let entry_path = crate::commands::resolve_entry_path(project_root, &manifest)?;
    // Assemble imports (including linked `path`/`git`-local deps) so `is exported
    // for native` functions defined in imported modules are bundled too.
    let dep_roots = crate::project::deps::resolve_dep_roots(project_root, &manifest)?;
    let assembled =
        logicaffeine_compile::analysis::assemble_project_with_deps(&entry_path, dep_roots)
            .map_err(|e| format!("could not assemble project: {e}"))?;

    let bundle_dir = project_root.join(".logos-native");
    let built = logicaffeine_compile::compile::build_native_bundle(&assembled.source, &bundle_dir)
        .map_err(|pe| {
            let at = crate::diagnostics::Placed::locate(
                pe.span,
                &assembled.source,
                &assembled.source_map,
                &entry_path,
            );
            CliError::new(format!(
                "native bundle build failed:\n{}",
                crate::diagnostics::render_parse_error(&pe, &at)
            ))
        })?;

    if built.is_empty() {
        println!("No `is exported for native` functions to bundle.");
    } else {
        println!(
            "Bundled {} native function(s) into {}:",
            built.len(),
            bundle_dir.display()
        );
        for (name, so) in &built {
            println!("  {name} -> {}", so.display());
        }
    }
    Ok(())
}
