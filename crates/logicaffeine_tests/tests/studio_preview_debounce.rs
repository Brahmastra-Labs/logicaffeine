//! Ratchet: the Studio "Web" live preview must stay **debounced**.
//!
//! Recompiling the LOGOS source to wasm on every keystroke makes the preview
//! iframe flicker and burns cycles while you type. The fix routes the preview
//! through a `debounced_code` buffer that only settles after a quiet window,
//! with a generation counter so the *last* edit wins. This lock keeps a future
//! edit from silently re-pointing the preview back at the raw editor buffer —
//! the exact papercut it removed.

const STUDIO_SRC: &str = include_str!("../../../apps/logicaffeine_web/src/ui/pages/studio_workbench.rs");

/// Slice out the body of the `web_preview` memo so we can assert what it reads.
fn web_preview_memo_body(src: &str) -> &str {
    let start = src
        .find("let web_preview = use_memo(")
        .expect("studio_workbench.rs defines the `web_preview` memo");
    let tail = &src[start..];
    // The memo body ends at the first `});` that closes `use_memo(move || { … })`.
    let end = tail.find("});").expect("web_preview memo is closed");
    &tail[..end]
}

#[test]
fn web_preview_reads_the_debounced_buffer_not_the_raw_editor() {
    let body = web_preview_memo_body(STUDIO_SRC);
    assert!(
        body.contains("debounced_code.read()"),
        "the Web preview must compile the *settled* source (`debounced_code`), \
         not the live editor — otherwise it recompiles + flickers on every keystroke.\n\
         memo body was:\n{body}"
    );
    assert!(
        !body.contains("code_input.read()"),
        "the Web preview must NOT read `code_input` directly — that reintroduces \
         per-keystroke recompiles. Read `debounced_code` instead.\nmemo body was:\n{body}"
    );
}

#[test]
fn a_debounce_effect_settles_the_editor_after_a_quiet_window() {
    // A ~300ms quiet window before committing the buffer — long enough to skip
    // mid-word recompiles, short enough to feel live.
    assert!(
        STUDIO_SRC.contains("gloo_timers::future::TimeoutFuture::new(300)"),
        "the preview debounce must wait a quiet window (300ms) before recompiling"
    );
    // A generation counter voids stale spawns so the LAST edit wins (no thrash).
    assert!(
        STUDIO_SRC.contains("web_edit_gen"),
        "the debounce must use a generation counter so the last edit supersedes \
         earlier pending spawns"
    );
    assert!(
        STUDIO_SRC.contains("debounced_code.set("),
        "the settled source is committed into `debounced_code`"
    );
}
