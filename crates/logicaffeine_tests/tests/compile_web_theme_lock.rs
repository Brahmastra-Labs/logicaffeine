//! ★ RATCHET: the default web theme can only get richer, never bare ★
//!
//! Every generated LOGOS web page ships the Claude-design theme, a favicon, and a
//! light/dark `theme-color`. These locks fail the build if the theme regresses — a
//! required design token or styled element vanishes, the CSS goes unbalanced, or the
//! HTML stops carrying the theme/favicon. A never-shrink floor on tokens + selectors
//! means the palette and component coverage can only grow.

use logicaffeine_compile::web_host::{browser_host_html, DEFAULT_THEME_CSS, FAVICON_DATA_URI};

/// The generated page embeds the theme, the favicon, and a light+dark theme-color.
#[test]
fn generated_html_carries_the_theme_favicon_and_theme_color() {
    let html = browser_host_html("demo");
    assert!(html.contains("<style>"), "the page inlines a stylesheet");
    assert!(html.contains(":root {"), "…the design-token theme");
    assert!(
        html.contains("prefers-color-scheme: dark"),
        "…with an automatic dark mode"
    );
    assert!(html.contains("rel=\"icon\""), "a favicon is linked");
    assert!(
        html.contains("theme-color")
            && html.contains("(prefers-color-scheme: light)")
            && html.contains("(prefers-color-scheme: dark)"),
        "a light AND dark theme-color are set"
    );
    // The original shell contract still holds.
    assert!(html.starts_with("<!doctype html>") && html.contains("<title>demo</title>"));
    assert!(
        html.contains("src=\"./demo.js\""),
        "still loads the app module"
    );
}

/// The favicon is a self-contained SVG data-URI (no asset file), clay-tinted.
#[test]
fn favicon_is_a_clay_svg_data_uri() {
    assert!(
        FAVICON_DATA_URI.starts_with("data:image/svg+xml,"),
        "an inline SVG data-URI"
    );
    assert!(
        FAVICON_DATA_URI.contains("<svg") && FAVICON_DATA_URI.contains("</svg>"),
        "well-formed SVG"
    );
    assert!(
        FAVICON_DATA_URI.contains("%23cc785c"),
        "the clay accent, %23-encoded in the URI"
    );
}

/// The palette is a never-shrink set of design tokens — the theme is reskinnable by
/// overriding a single `--token`, and a token can never be silently dropped.
#[test]
fn theme_defines_the_full_design_token_palette() {
    let required = [
        "--paper",
        "--surface",
        "--ink",
        "--muted",
        "--accent",
        "--accent-hover",
        "--accent-contrast",
        "--border",
        "--radius",
        "--shadow",
        "--font-sans",
        "--font-mono",
        "--measure",
        "--space",
    ];
    for token in required {
        assert!(
            DEFAULT_THEME_CSS.contains(&format!("{token}:")),
            "the theme must define the `{token}` design token"
        );
    }
    // Floor: at least this many token *definitions* (each on its own `--name: …;` line;
    // `var(--x)` usages are mid-line so they don't count). Light + dark overrides; grows only.
    let defs = DEFAULT_THEME_CSS
        .lines()
        .filter(|l| l.trim_start().starts_with("--"))
        .count();
    assert!(
        defs >= 22,
        "expected ≥22 token definitions (14 light + dark overrides), got {defs}"
    );
}

/// The theme styles the core HTML surface a LOGOS view produces — a never-shrink set of
/// element selectors so the component coverage can only grow.
#[test]
fn theme_styles_the_core_elements() {
    for selector in [
        "body", "button", "input", "a ", "h1", "ul", "li", "code", "pre", "table", ".card",
    ] {
        assert!(
            DEFAULT_THEME_CSS.contains(selector),
            "the theme must style `{}`",
            selector.trim()
        );
    }
    // A focus ring (accessibility) and a dark-mode block are non-negotiable.
    assert!(
        DEFAULT_THEME_CSS.contains(":focus-visible"),
        "keyboard focus is visible"
    );
    assert!(
        DEFAULT_THEME_CSS.contains("@media (prefers-color-scheme: dark)"),
        "dark mode present"
    );
    // Rule-block floor.
    let blocks = DEFAULT_THEME_CSS.matches('{').count();
    assert!(blocks >= 20, "expected ≥20 CSS rule blocks, got {blocks}");
}

/// The stylesheet is well-formed: balanced braces (a busted theme would break every page).
#[test]
fn theme_css_is_brace_balanced() {
    let opens = DEFAULT_THEME_CSS.matches('{').count();
    let closes = DEFAULT_THEME_CSS.matches('}').count();
    assert_eq!(
        opens, closes,
        "CSS braces must balance ({opens} open vs {closes} close)"
    );
}

/// Every `--name` a stylesheet DEFINES (a `--name:` at the start of a line).
fn defined_tokens(css: &str) -> std::collections::BTreeSet<String> {
    css.lines()
        .filter_map(|l| l.trim_start().strip_prefix("--"))
        .filter_map(|rest| {
            rest.split_once(':')
                .map(|(name, _)| format!("--{}", name.trim()))
        })
        .collect()
}

/// The theme `largo new --web` scaffolds (`static/theme.css`), read from the scaffold's own source.
fn scaffold_theme() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/logicaffeine_cli/src/commands/new.rs");
    let src = std::fs::read_to_string(&path).expect("the scaffold's source");
    let start = src
        .find("const WEB_THEME_CSS: &str = r#\"")
        .expect("the scaffold declares its theme as `WEB_THEME_CSS`");
    let body = &src[start + "const WEB_THEME_CSS: &str = r#\"".len()..];
    body[..body.find("\"#;").expect("the theme literal closes")].to_string()
}

/// ONE token vocabulary: every token the scaffold's theme defines is a token the built-in theme
/// defines, so a project's `theme.css` reskins the built-in styles instead of naming colours nothing
/// reads — and the built-in theme spells none of the scaffold's tokens differently.
#[test]
fn the_default_theme_and_the_scaffold_share_token_names() {
    let scaffold = defined_tokens(&scaffold_theme());
    assert!(
        scaffold.len() >= 7,
        "the scaffold's theme defines its tokens: {scaffold:?}"
    );
    let builtin = defined_tokens(DEFAULT_THEME_CSS);
    let missing: Vec<&String> = scaffold.iter().filter(|t| !builtin.contains(*t)).collect();
    assert!(
        missing.is_empty(),
        "tokens the scaffold defines and the built-in theme does not: {missing:?} — a project that \
         changes one changes nothing the built-in styles read"
    );
}

/// A site that brings its own design opts out of the built-in theme, and its shell then carries none
/// of it — no design tokens, no element rules — while the rest of the shell stays exactly the same.
#[test]
fn a_site_can_opt_out_of_the_default_theme() {
    use logicaffeine_compile::web_host::{browser_host_html_themed, ShellTheme};
    let themed = browser_host_html_themed("demo", None, None, ShellTheme::Default);
    assert_eq!(
        themed,
        browser_host_html("demo"),
        "the default is the theme every shell has always carried"
    );
    let own = browser_host_html_themed("demo", None, None, ShellTheme::None);
    assert!(
        !own.contains(DEFAULT_THEME_CSS) && !own.contains("<style>"),
        "an opted-out shell carries no theme:\n{own}"
    );
    assert_eq!(
        own,
        themed.replace(&format!("<style>\n{DEFAULT_THEME_CSS}</style>\n"), ""),
        "and everything else in the shell is unchanged"
    );
}
