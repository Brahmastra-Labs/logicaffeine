//! The Math editor sets its text in Logos Math Mono, whose ligatures draw the kernel's ASCII
//! spellings as symbols (`forall` as ∀, `->` as →). The font ships with the site, with the licence
//! its source face requires, and the editor's stylesheet sets it on both layers of the Math editor
//! and nowhere else. `scripts/web/build-math-font.py` builds it and shapes probes that prove each
//! ligature fires on whole words and operators only.

#![cfg(feature = "web-tests")]

use logicaffeine_web::ui::components::code_editor::CODE_EDITOR_STYLE;
use std::path::PathBuf;

fn fonts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_web/public/assets/fonts")
}

#[test]
fn the_font_ships_as_woff2() {
    let bytes = std::fs::read(fonts().join("LogosMathMono.woff2")).expect("the Math font");
    assert_eq!(&bytes[..4], b"wOF2", "a WOFF2 file");
    assert!(bytes.len() < 80_000, "the subset font is {} bytes", bytes.len());
}

#[test]
fn the_font_ships_with_its_licence() {
    let licence = std::fs::read_to_string(fonts().join("LogosMathMono-LICENSE.txt")).expect("the font's licence");
    for notice in [
        "Copyright (c) 2003 by Bitstream, Inc. All Rights Reserved.",
        "Bitstream Vera is\na trademark of Bitstream, Inc.",
        "Permission is hereby granted, free of charge",
        "DejaVu changes are in public domain.",
        "renamed",
    ] {
        assert!(licence.contains(notice), "the licence file lacks {notice:?}");
    }
}

#[test]
fn the_build_script_is_kept_beside_the_site() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/web/build-math-font.py");
    let text = std::fs::read_to_string(script).expect("scripts/web/build-math-font.py");
    assert!(text.contains("PROBES") && text.contains("Logos Math Mono"), "the build script shapes its probes");
}

#[test]
fn the_math_editor_sets_the_font_on_both_layers_and_only_there() {
    let style = CODE_EDITOR_STYLE;
    assert!(style.contains("@font-face") && style.contains("url('/assets/fonts/LogosMathMono.woff2')"), "the stylesheet loads the font");
    let rule = style
        .split('}')
        .find(|block| block.contains("font-family: 'Logos Math Mono'") && !block.contains("@font-face"))
        .expect("a rule that sets the Math font");
    for layer in [".code-editor.lang-vernacular .code-editor-textarea", ".code-editor.lang-vernacular .code-editor-highlight"] {
        assert!(rule.contains(layer), "the Math font rule misses {layer}: {rule}");
    }
    assert_eq!(style.matches("font-family: 'Logos Math Mono'").count(), 2, "set in the @font-face and the Math rule, nowhere else");
}
