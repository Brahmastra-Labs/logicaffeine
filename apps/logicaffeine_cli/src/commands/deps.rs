//! `largo add` / `largo remove` — Largo.toml dependency editing.
//!
//! Edits go through [`toml_edit`], so the manifest's comments, spacing, and
//! ordering are preserved byte-for-byte outside the one change. After every
//! edit, the result is round-tripped through [`Manifest`] to prove it still
//! parses.

use std::fs;
use std::path::Path;

use toml_edit::{value, DocumentMut, Item, Table};

use crate::commands::require_project_root;
use crate::project::manifest::Manifest;
use crate::ui::{self, CliError};

/// What a dependency SPEC resolved to.
#[derive(Debug, PartialEq, Eq)]
enum DepValue {
    /// A plain version-or-URI string: `foo = "1.2"`, `std = "logos:std"`.
    Simple(String),
    /// `{ path = "…" }`
    Path(String),
    /// `{ git = "…" }`
    Git(String),
}

/// Parse an `add` SPEC (`name`, `name@version`, `logos:name`) together with
/// the `--path`/`--git` options into a key + value.
fn parse_spec(
    spec: &str,
    path: Option<String>,
    git: Option<String>,
) -> Result<(String, DepValue), CliError> {
    if let Some(p) = path {
        if spec.contains('@') || spec.contains(':') {
            return Err(CliError::new(format!(
                "`--path` takes a bare dependency name, not `{spec}`"
            ))
            .exit_code(ui::EXIT_USAGE));
        }
        return Ok((spec.to_string(), DepValue::Path(p)));
    }
    if let Some(g) = git {
        if spec.contains('@') || spec.contains(':') {
            return Err(CliError::new(format!(
                "`--git` takes a bare dependency name, not `{spec}`"
            ))
            .exit_code(ui::EXIT_USAGE));
        }
        return Ok((spec.to_string(), DepValue::Git(g)));
    }
    if let Some(name) = spec.strip_prefix("logos:") {
        if name.contains('@') {
            return Err(CliError::new(format!(
                "a `logos:` URI takes no version — `{spec}` is malformed"
            ))
            .exit_code(ui::EXIT_USAGE));
        }
        return Ok((name.to_string(), DepValue::Simple(spec.to_string())));
    }
    if let Some((name, version)) = spec.split_once('@') {
        if name.is_empty() || version.is_empty() || version.contains('@') {
            return Err(CliError::new(format!("malformed dependency spec `{spec}`"))
                .exit_code(ui::EXIT_USAGE));
        }
        return Ok((name.to_string(), DepValue::Simple(version.to_string())));
    }
    Ok((spec.to_string(), DepValue::Simple("*".to_string())))
}

/// Handle `largo add <SPEC> [--path DIR | --git URL]`.
pub(crate) fn cmd_add(
    spec: String,
    path: Option<String>,
    git: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = require_project_root()?;
    let (name, dep) = parse_spec(&spec, path, git)?;

    let mut doc = load_manifest_doc(&root)?;
    // `as_table_like_mut` covers both `[dependencies]` tables and the inline
    // `dependencies = { … }` form.
    let deps = doc
        .entry("dependencies")
        .or_insert(Item::Table(Table::new()))
        .as_table_like_mut()
        .ok_or_else(|| CliError::new("`dependencies` in Largo.toml is not a table"))?;

    let item = match &dep {
        DepValue::Simple(s) => value(s.as_str()),
        DepValue::Path(p) => {
            let mut inline = toml_edit::InlineTable::new();
            inline.insert("path", p.as_str().into());
            value(inline)
        }
        DepValue::Git(g) => {
            let mut inline = toml_edit::InlineTable::new();
            inline.insert("git", g.as_str().into());
            value(inline)
        }
    };
    deps.insert(&name, item);

    save_manifest_doc(&root, &doc)?;
    ui::info(format!("Added {name} to Largo.toml"));
    Ok(())
}

/// Handle `largo remove <NAME>`.
pub(crate) fn cmd_remove(name: String) -> Result<(), Box<dyn std::error::Error>> {
    let root = require_project_root()?;

    let mut doc = load_manifest_doc(&root)?;
    let removed = doc
        .get_mut("dependencies")
        .and_then(|d| d.as_table_like_mut())
        .map(|t| t.remove(&name).is_some())
        .unwrap_or(false);

    if !removed {
        return Err(CliError::with_hint(
            format!("`{name}` is not a dependency in Largo.toml"),
            "run `largo add <name>` to add one, or check the [dependencies] section",
        )
        .into());
    }

    save_manifest_doc(&root, &doc)?;
    ui::info(format!("Removed {name} from Largo.toml"));
    Ok(())
}

/// Load Largo.toml as a format-preserving document.
fn load_manifest_doc(root: &Path) -> Result<DocumentMut, Box<dyn std::error::Error>> {
    let manifest_path = root.join("Largo.toml");
    let text = fs::read_to_string(&manifest_path)
        .map_err(|e| CliError::new(format!("cannot read {}: {e}", manifest_path.display())))?;
    text.parse::<DocumentMut>()
        .map_err(|e| CliError::new(format!("Largo.toml is not valid TOML: {e}")).into())
}

/// Write the edited document back, after proving the result still loads as
/// a valid [`Manifest`].
fn save_manifest_doc(root: &Path, doc: &DocumentMut) -> Result<(), Box<dyn std::error::Error>> {
    let text = doc.to_string();
    toml::from_str::<Manifest>(&text)
        .map_err(|e| CliError::new(format!("edit would corrupt Largo.toml: {e}")))?;
    fs::write(root.join("Largo.toml"), text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_name_is_wildcard() {
        let (name, dep) = parse_spec("foo", None, None).unwrap();
        assert_eq!(name, "foo");
        assert_eq!(dep, DepValue::Simple("*".into()));
    }

    #[test]
    fn at_version_splits() {
        let (name, dep) = parse_spec("foo@1.2", None, None).unwrap();
        assert_eq!(name, "foo");
        assert_eq!(dep, DepValue::Simple("1.2".into()));
    }

    #[test]
    fn logos_uri_keeps_full_value() {
        let (name, dep) = parse_spec("logos:std", None, None).unwrap();
        assert_eq!(name, "std");
        assert_eq!(dep, DepValue::Simple("logos:std".into()));
    }

    #[test]
    fn path_option_wins() {
        let (name, dep) = parse_spec("math", Some("./math".into()), None).unwrap();
        assert_eq!(name, "math");
        assert_eq!(dep, DepValue::Path("./math".into()));
    }

    #[test]
    fn git_option_wins() {
        let (name, dep) = parse_spec("r", None, Some("https://x/r.git".into())).unwrap();
        assert_eq!(name, "r");
        assert_eq!(dep, DepValue::Git("https://x/r.git".into()));
    }

    #[test]
    fn version_spec_with_path_is_usage_error() {
        let err = parse_spec("foo@1.2", Some("./x".into()), None).unwrap_err();
        assert_eq!(err.exit_code, ui::EXIT_USAGE);
    }

    #[test]
    fn empty_version_is_malformed() {
        assert!(parse_spec("foo@", None, None).is_err());
        assert!(parse_spec("@1.0", None, None).is_err());
    }

    #[test]
    fn logos_uri_with_version_is_malformed() {
        let err = parse_spec("logos:std@1.0", None, None).unwrap_err();
        assert_eq!(err.exit_code, ui::EXIT_USAGE);
    }

    #[test]
    fn double_at_is_malformed() {
        assert!(parse_spec("foo@1.0@2.0", None, None).is_err());
    }
}
