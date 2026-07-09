//! `largo doctor` — diagnose the environment largo runs in.
//!
//! Reports a ✓/!/✗ line per check. Philosophy: anything that only degrades
//! largo (no cargo → `build` unavailable but `run --interpret` fine; no
//! network → can't check freshness) is a **warning**; only a genuinely
//! broken state the user must fix (a corrupt project manifest, a missing
//! entry file) is a **failure**. Doctor itself must be useful offline.

use std::process::Command;

use anstyle::AnsiColor;

use crate::project::build::find_project_root;
use crate::project::manifest::Manifest;
use crate::project::registry::RegistryClient;
use crate::ui::CliError;

/// One diagnostic's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Working as intended.
    Ok,
    /// Degraded but not broken (build path unavailable, no network, ...).
    Warn,
    /// Broken — the user must act; fails the run.
    Fail,
}

/// One rendered check row.
struct Check {
    status: Status,
    name: &'static str,
    detail: String,
}

impl Check {
    /// The row as (status symbol, rest-of-line) — split so the print site
    /// can color just the symbol without slicing multi-byte glyphs.
    fn parts(&self) -> (&'static str, String) {
        let symbol = match self.status {
            Status::Ok => "✓",
            Status::Warn => "!",
            Status::Fail => "✗",
        };
        (symbol, format!(" {:<14} {}", self.name, self.detail))
    }
}

/// Render a check row (plain text — the spec the unit tests pin; the print
/// site colors the symbol from [`Check::parts`]).
#[cfg(test)]
fn render_check(check: &Check) -> String {
    let (symbol, rest) = check.parts();
    format!("{symbol}{rest}")
}

/// `true` when `candidate` is a strictly newer semver than `current`.
fn version_is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Option<[u64; 3]> {
        let mut it = v.trim().trim_start_matches('v').splitn(3, '.');
        let maj = it.next()?.parse().ok()?;
        let min = it.next()?.parse().ok()?;
        let pat: u64 = it
            .next()?
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()?;
        Some([maj, min, pat])
    };
    match (parse(candidate), parse(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

/// The first line of a command's stdout, if it runs.
fn tool_version(cmd: &str, arg: &str) -> Option<String> {
    let out = Command::new(cmd).arg(arg).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|l| l.trim().to_string())
}

/// Handle `largo doctor [--registry URL]`.
pub(crate) fn cmd_doctor(registry: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let mut checks: Vec<Check> = Vec::new();

    // largo itself + crates.io freshness (warn-only; 2s budget).
    let current = env!("CARGO_PKG_VERSION");
    let freshness = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .get("https://crates.io/api/v1/crates/logicaffeine-cli")
        .call()
        .ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok())
        .and_then(|v| v["crate"]["max_version"].as_str().map(String::from));
    checks.push(match freshness {
        Some(latest) if version_is_newer(&latest, current) => Check {
            status: Status::Warn,
            name: "largo",
            detail: format!("{current} (v{latest} is available — re-run the installer to update)"),
        },
        Some(_) => Check { status: Status::Ok, name: "largo", detail: format!("{current} (latest)") },
        None => Check {
            status: Status::Warn,
            name: "largo",
            detail: format!("{current} (could not check crates.io for updates)"),
        },
    });

    // Rust toolchain — powers `largo build`/`run`; everything else works without.
    checks.push(match tool_version("cargo", "--version") {
        Some(v) => Check { status: Status::Ok, name: "cargo", detail: v },
        None => Check {
            status: Status::Warn,
            name: "cargo",
            detail: "not found — `largo build/run` need a Rust toolchain (https://rustup.rs); \
                     `largo run --interpret` and `--emit wasm` work without one"
                .to_string(),
        },
    });

    // wasm32 target — needed by `--target wasm` and `--emit wasm-linked`.
    let wasm_target = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-unknown-unknown"))
        .unwrap_or(false);
    checks.push(if wasm_target {
        Check { status: Status::Ok, name: "wasm32", detail: "wasm32-unknown-unknown installed".into() }
    } else {
        Check {
            status: Status::Warn,
            name: "wasm32",
            detail: "target not installed — `--target wasm` / `--emit wasm-linked` need it \
                     (rustup target add wasm32-unknown-unknown)"
                .to_string(),
        }
    });

    // node — runs the emitted host shim for `largo run --emit wasm`.
    checks.push(match tool_version("node", "--version") {
        Some(v) => Check { status: Status::Ok, name: "node", detail: v },
        None => Check {
            status: Status::Warn,
            name: "node",
            detail: "not found — `largo run --emit wasm` executes through node".to_string(),
        },
    });

    // Verification flavor.
    checks.push(if cfg!(feature = "verification") {
        Check { status: Status::Ok, name: "verification", detail: "compiled in (full build)".into() }
    } else {
        Check {
            status: Status::Warn,
            name: "verification",
            detail: "lean build — `largo verify` needs the full flavor \
                     (install.sh --full) or `--features verification`"
                .to_string(),
        }
    });

    // Registry reachability + credentials. Any HTTP response (401 included)
    // proves reachability; only transport errors are warnings.
    let registry_url = registry.as_deref().unwrap_or(RegistryClient::default_url());
    let reachable = {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(2))
            .build();
        match agent.get(&format!("{registry_url}/auth/me")).call() {
            Ok(_) => true,
            Err(ureq::Error::Status(_, _)) => true,
            Err(ureq::Error::Transport(_)) => false,
        }
    };
    let token = crate::project::credentials::get_token(registry_url).is_some();
    checks.push(if reachable {
        Check {
            status: Status::Ok,
            name: "registry",
            detail: format!(
                "{registry_url} reachable ({})",
                if token { "token stored" } else { "not logged in" }
            ),
        }
    } else {
        Check {
            status: Status::Warn,
            name: "registry",
            detail: format!("{registry_url} unreachable (offline? publish/login need it)"),
        }
    });

    // Project health, when inside one.
    if let Ok(cwd) = std::env::current_dir() {
        if find_project_root(&cwd).is_some() || cwd.join("Largo.toml").exists() {
            let root = find_project_root(&cwd).unwrap_or(cwd);
            match Manifest::load(&root) {
                Ok(manifest) => {
                    let entry = root.join(&manifest.package.entry);
                    let md_fallback = entry.with_extension("md");
                    if entry.exists() || md_fallback.exists() {
                        checks.push(Check {
                            status: Status::Ok,
                            name: "project",
                            detail: format!("{} v{}", manifest.package.name, manifest.package.version),
                        });
                    } else {
                        checks.push(Check {
                            status: Status::Fail,
                            name: "project",
                            detail: format!("entry point missing: {}", entry.display()),
                        });
                    }
                }
                Err(e) => checks.push(Check {
                    status: Status::Fail,
                    name: "project",
                    detail: format!("Largo.toml does not parse: {e}"),
                }),
            }
        }
    }

    // Render.
    let green = AnsiColor::Green.on_default().bold();
    let yellow = AnsiColor::Yellow.on_default().bold();
    let red = AnsiColor::Red.on_default().bold();
    for check in &checks {
        let (symbol, rest) = check.parts();
        match check.status {
            Status::Ok => anstream::println!("{green}{symbol}{green:#}{rest}"),
            Status::Warn => anstream::println!("{yellow}{symbol}{yellow:#}{rest}"),
            Status::Fail => anstream::println!("{red}{symbol}{red:#}{rest}"),
        }
    }

    let failures = checks.iter().filter(|c| c.status == Status::Fail).count();
    if failures > 0 {
        return Err(CliError::new(format!("{failures} check(s) failed")).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_semver_is_detected() {
        assert!(version_is_newer("0.10.1", "0.10.0"));
        assert!(version_is_newer("0.11.0", "0.10.9"));
        assert!(version_is_newer("1.0.0", "0.99.99"));
        assert!(!version_is_newer("0.10.0", "0.10.0"));
        assert!(!version_is_newer("0.9.9", "0.10.0"));
        assert!(!version_is_newer("garbage", "0.10.0"));
    }

    #[test]
    fn check_lines_align_and_carry_symbols() {
        let ok = render_check(&Check { status: Status::Ok, name: "cargo", detail: "1.80".into() });
        assert!(ok.starts_with("✓ cargo"), "{ok}");
        let fail = render_check(&Check { status: Status::Fail, name: "project", detail: "broken".into() });
        assert!(fail.starts_with("✗ project"), "{fail}");
        let warn = render_check(&Check { status: Status::Warn, name: "node", detail: "x".into() });
        // The detail column aligns in CHARACTER position across rows (the
        // symbols differ in UTF-8 byte width, so compare past the symbol).
        let rest_ok: String = ok.chars().skip(1).collect();
        let rest_warn: String = warn.chars().skip(1).collect();
        assert_eq!(rest_ok.find("1.80"), rest_warn.find('x'), "aligned detail column");
    }
}
