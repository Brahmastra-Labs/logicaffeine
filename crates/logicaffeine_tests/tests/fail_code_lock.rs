//! ★ LOCK — the browser reports a failure in the reference tier's own words. ★
//!
//! Most runtime failures carry a message the compiler already wrote, so the wasm backend hands
//! that Text to the host and there is nothing to drift. An INLINE check cannot: its sentence
//! needs a value that only exists while running (`Index 9 out of bounds`), so the module reports
//! a CODE and the VALUE, and the host renders the sentence.
//!
//! ★ WHICH PUTS THE WORDING IN TWO LANGUAGES — `semantics::failcode::message` in Rust, and a
//! `switch` in `assets/web/runtime.js`. That is the shape of every drift this campaign has paid
//! for: two implementations that agree on the day they are written. A comparison makes drift
//! DETECTABLE; that is the most this seam allows, since a browser host cannot call Rust. So this
//! lock does the comparison, and does it against the REAL file rather than a copy of it.
//!
//! It reads the cases out of `runtime.js`, renders each one the way JavaScript would, and holds
//! it to `failcode::message` for the same code. A new code the host does not render fails here —
//! loudly, in a test — instead of reaching a visitor as a sentence nobody wrote.

use logicaffeine_compile::semantics::failcode::{self, ALL};

const RUNTIME_JS: &str = include_str!("../../logicaffeine_compile/assets/web/runtime.js");

/// Every host that renders a failure code, as `(what to call it, the JavaScript it runs)`: the browser's
/// `runtime.js` as written, and the Node shim exactly as `largo emit wasm` writes it beside a module. The
/// shim is read as generated, not from the Rust source that generates it, so what is held here is what ships.
fn hosts() -> Vec<(&'static str, String)> {
    vec![
        ("assets/web/runtime.js", RUNTIME_JS.to_string()),
        (
            "the wasm Node shim (web_host::wasm_host_shim)",
            logicaffeine_compile::web_host::wasm_host_shim("probe"),
        ),
    ]
}

/// A sample value that is unmistakable in the rendered text — no code's wording should contain
/// it by accident, so a template that ignores its value is still compared honestly.
const SAMPLE: i64 = 4242;

/// Render one `case` body from `runtime.js` the way JavaScript would, with `v` bound to `SAMPLE`.
///
/// The bodies are deliberately a flat concatenation of single-quoted literals and `v`; anything
/// else is refused rather than guessed at, because a case this cannot read is a case this cannot
/// hold to the Rust wording — and silently skipping it is how a lock stops covering things.
fn render_case(body: &str) -> Result<String, String> {
    let mut out = String::new();
    for piece in body.split('+') {
        let piece = piece.trim();
        if piece == "v" {
            out.push_str(&SAMPLE.to_string());
        } else if piece.len() >= 2 && piece.starts_with('\'') && piece.ends_with('\'') {
            out.push_str(&piece[1..piece.len() - 1]);
        } else {
            return Err(format!(
                "this lock reads a case body as literals joined by `+` with `v` for the value, and \
                 met `{piece}`. Either write the case that way, or teach this reader the new form — \
                 do not leave a case it cannot check."
            ));
        }
    }
    Ok(out)
}

/// Pull `case <n>: failMessage = <expr>; break;` out of the `logos_rt_fail_at` switch.
/// The body of the `logos_rt_fail_at` host function, and nothing else.
///
/// ★ SCOPED ON PURPOSE. `runtime.js` also holds the patch-stream decoder, whose switch numbers
/// its opcodes from 0 — so a reader that searched the whole file for `case 1:` would compare the
/// failure wording against `createElement` and report a drift that does not exist. It did
/// exactly that on its first run.
fn fail_at_body<'s>(host: &str, src: &'s str) -> &'s str {
    let start = src.find("logos_rt_fail_at:").unwrap_or_else(|| {
        panic!(
            "`{host}` no longer defines `logos_rt_fail_at` — the host stopped rendering failures"
        )
    });
    let rest = &src[start..];
    // Both hosts close the property with `\n  },`.
    let end = rest
        .find("\n  },")
        .unwrap_or_else(|| panic!("could not find the end of `{host}`'s `logos_rt_fail_at`"));
    &rest[..end]
}

fn case_body(body: &str, code: i32) -> Option<String> {
    let needle = format!("case {code}:");
    let start = body.find(&needle)? + needle.len();
    let rest = &body[start..];
    let assign = rest.find("failMessage =")? + "failMessage =".len();
    let rest = &rest[assign..];
    let end = rest.find(';')?;
    Some(rest[..end].trim().to_string())
}

/// ★ THE LOAD-BEARING TEST — every code renders identically in both languages.
#[test]
fn the_browser_renders_every_failure_code_exactly_as_the_reference_tier_does() {
    for (host, src) in hosts() {
        let body = fail_at_body(host, &src);
        for &code in ALL {
            let c = code.code();
            let expected = failcode::message(c, SAMPLE);
            let case = case_body(body, c).unwrap_or_else(|| {
                panic!(
                    "`{host}` has no `case {c}:` for {code:?}. A module built today reports that \
                     code and the browser would fall through to the unrecognized-code branch — a \
                     failure the visitor sees worded by nobody. Add the case beside the others."
                )
            });
            let rendered = render_case(&case).unwrap_or_else(|e| panic!("{host} / {code:?}: {e}"));
            assert_eq!(
                rendered, expected,
                "{code:?} (code {c}) reads differently in `{host}` than on the reference tier. The \
                 wording belongs to `semantics::failcode::message`; change it there and mirror it here."
            );
        }
    }
}

/// The host must not carry a case the Rust side does not know — a stale code renders a sentence
/// for a failure that can no longer happen, which is worse than no sentence at all.
#[test]
fn the_browser_carries_no_case_the_compiler_does_not_emit() {
    let known: Vec<i32> = ALL.iter().map(|c| c.code()).collect();
    for (host, src) in hosts() {
        let mut seen = Vec::new();
        let mut rest = fail_at_body(host, &src);
        while let Some(i) = rest.find("case ") {
            rest = &rest[i + "case ".len()..];
            let end = rest.find(':').unwrap_or(0);
            if let Ok(n) = rest[..end].trim().parse::<i32>() {
                seen.push(n);
            }
        }
        assert!(
            !seen.is_empty(),
            "the reader found no cases at all in `{host}` — it has stopped covering the switch"
        );
        for n in seen {
            assert!(
                known.contains(&n),
                "`{host}` renders a message for code {n}, which no `FailCode` emits. Either it was \
                 removed from Rust and left here, or the numbering drifted — and the codes are \
                 append-only precisely so a module built earlier is never re-labelled."
            );
        }
    }
}
