//! The browser host for a LOGOS→WASM DOM app — shared by the CLI (`largo emit
//! wasm-dom` / `largo serve`) and the web Studio's live preview so both render a
//! LOGOS app identically. It supplies the real DOM behind the `dom_*` imports,
//! allocates host `Text` into linear memory, and fires event/fetch callbacks through
//! the exported `__logos_call_closure` trampoline (the production analog of the test
//! `MockDom`). It also carries the default theme, the favicon, and the build-error
//! overlay so a fresh LOGOS web app is beautiful — and legible when it breaks — with
//! zero CSS written.

/// A clay-λ favicon (an inline SVG data-URI) so a fresh LOGOS app is branded in the
/// tab bar with no asset file. `#` is `%23`-encoded (a `#` is a URI fragment marker).
pub const FAVICON_DATA_URI: &str = "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='7' fill='%23cc785c'/><text x='16' y='23' font-family='sans-serif' font-size='21' font-weight='700' text-anchor='middle' fill='white'>λ</text></svg>";

/// The default theme — a small, opinionated design system in Claude's language: a warm
/// cream surface, a clay accent, ink text, generous spacing, rounded controls, and an
/// automatic dark mode via `prefers-color-scheme`. Every value is a `--token` on
/// `:root`, so a user overrides one line to reskin — and the tokens are the ones `largo new
/// --web` scaffolds into `static/theme.css` (`--paper`, `--ink`, `--accent`, `--muted`,
/// `--border`, `--radius`, `--shadow`), so a project's theme file reskins these styles too.
/// Ships in every generated page so a LOGOS web app looks *designed* with zero CSS written,
/// unless the site opts out ([`ShellTheme::None`]).
pub const DEFAULT_THEME_CSS: &str = r#":root {
  --paper: #faf9f5;
  --surface: #ffffff;
  --ink: #1f1e1d;
  --muted: #6b6a67;
  --accent: #cc785c;
  --accent-hover: #b8613f;
  --accent-contrast: #ffffff;
  --border: #e8e6df;
  --radius: 10px;
  --shadow: 0 1px 3px rgba(31,30,29,.06), 0 8px 24px rgba(31,30,29,.05);
  --font-sans: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
  --font-mono: ui-monospace, "SF Mono", "JetBrains Mono", "Fira Code", Menlo, Consolas, monospace;
  --measure: 720px;
  --space: 1rem;
}
@media (prefers-color-scheme: dark) {
  :root {
    --paper: #1f1e1d;
    --surface: #262624;
    --ink: #f5f4ef;
    --muted: #a3a29e;
    --accent: #e0906f;
    --accent-hover: #ecab8f;
    --accent-contrast: #1f1e1d;
    --border: #3a3936;
    --shadow: 0 1px 3px rgba(0,0,0,.3), 0 8px 24px rgba(0,0,0,.25);
  }
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
  margin: 0 auto;
  max-width: min(var(--measure), 92vw);
  padding: 2.5rem 1.25rem 4rem;
  background: var(--paper);
  color: var(--ink);
  font-family: var(--font-sans);
  font-size: 16px;
  line-height: 1.6;
  -webkit-font-smoothing: antialiased;
}
#root { display: block; }
h1, h2, h3, h4, h5, h6 { line-height: 1.2; font-weight: 600; letter-spacing: -0.011em; margin: 1.6em 0 0.5em; }
h1 { font-size: 2rem; margin-top: 0; }
h2 { font-size: 1.5rem; }
h3 { font-size: 1.25rem; }
p { margin: 0 0 1em; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
small, .muted { color: var(--muted); }
ul, ol { padding-left: 1.4em; margin: 0 0 1em; }
li { margin: 0.25em 0; }
hr { border: none; border-top: 1px solid var(--border); margin: 2em 0; }
button {
  font: inherit;
  font-weight: 550;
  color: var(--accent-contrast);
  background: var(--accent);
  border: none;
  border-radius: var(--radius);
  padding: 0.6em 1.15em;
  cursor: pointer;
  transition: background .15s ease, transform .04s ease;
}
button:hover { background: var(--accent-hover); }
button:active { transform: translateY(1px); }
button:disabled { opacity: .5; cursor: default; }
button.secondary { background: transparent; color: var(--ink); border: 1px solid var(--border); }
input, textarea, select {
  font: inherit;
  color: var(--ink);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.55em 0.7em;
}
input:focus-visible, textarea:focus-visible, select:focus-visible, button:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.card, section {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
  box-shadow: var(--shadow);
}
code { font-family: var(--font-mono); font-size: 0.9em; background: rgba(31,30,29,.06); padding: 0.1em 0.35em; border-radius: 5px; }
pre { font-family: var(--font-mono); background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: 1rem; overflow: auto; }
pre code { background: none; padding: 0; }
table { border-collapse: collapse; width: 100%; }
th, td { text-align: left; padding: 0.5em 0.75em; border-bottom: 1px solid var(--border); }
"#;

/// The HTML shell that loads a LOGOS→WASM DOM app in a browser — carrying the default
/// theme, the clay-λ favicon, and a light/dark `theme-color` so a fresh app is
/// beautiful out of the box.
pub fn browser_host_html(name: &str) -> String {
    browser_host_html_with_head(name, None, None)
}

/// The tags the shell contributes when a page declares nothing — good defaults, in the
/// order a document wants them. Each is REPLACEABLE by a page's own tag of the same
/// identity (see [`head_identity`]); everything a page adds beyond these is appended.
fn default_head_tags(name: &str, favicon: Option<&str>) -> Vec<String> {
    vec![
        "<meta charset=\"utf-8\">".to_string(),
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">".to_string(),
        "<meta name=\"theme-color\" content=\"#faf9f5\" media=\"(prefers-color-scheme: light)\">"
            .to_string(),
        "<meta name=\"theme-color\" content=\"#1f1e1d\" media=\"(prefers-color-scheme: dark)\">"
            .to_string(),
        format!(
            "<link rel=\"icon\" href=\"{}\">",
            favicon.unwrap_or(FAVICON_DATA_URI)
        ),
        format!("<title>{name}</title>"),
    ]
}

/// What makes two head tags THE SAME TAG — the thing a page's own version replaces.
///
/// ★ IDENTITY, NOT POSITION. A document with two `<title>`s has whichever title a
/// browser read last, and two `viewport` metas is a page that renders differently on
/// two phones. A page that writes a viewport means *instead of*, never *as well as*.
///
/// A tag with no identity here (`<meta http-equiv=…>`, a second `<link rel="preload">`)
/// replaces nothing and is simply added: a list of what is REPLACEABLE can only fail by
/// being too small, which shows up as one extra tag. A rule that guessed could delete a
/// tag the author wrote.
///
/// The identity is the document head's own ([`crate::semantics::dom::HeadTag::identity`]), read
/// off the tag's parsed attributes, so the server's composition and the in-process document merge
/// a page's `Head:` by one rule — the one `runtime.js`'s `headIdentity` applies in a browser.
fn head_identity(tag: &str) -> Option<String> {
    crate::semantics::dom::parse_head(tag)
        .first()
        .and_then(crate::semantics::dom::HeadTag::identity)
}

/// Split a rendered head fragment into its top-level tags. The fragment comes from the
/// LGX renderer, so it is well-formed markup of `<title>`/`<meta>`/`<link>`/`<base>`
/// only — the parser refuses anything else in a `Head:`.
fn split_head_tags(fragment: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = fragment.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // `<title>…</title>` carries text; the void tags do not.
        let rest = &fragment[i..];
        let close = match rest.to_ascii_lowercase().starts_with("<title") {
            true => rest
                .to_ascii_lowercase()
                .find("</title>")
                .map(|e| e + "</title>".len()),
            false => rest.find('>').map(|e| e + 1),
        };
        match close {
            Some(end) => {
                out.push(rest[..end].trim().to_string());
                i += end;
            }
            None => break,
        }
    }
    out
}

/// Whether a page's shell carries the built-in theme ([`DEFAULT_THEME_CSS`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellTheme {
    /// The built-in design tokens and element styles, under everything the site adds.
    #[default]
    Default,
    /// None of it: the site brings its whole design itself (`Largo.toml`: `[web]` `theme = false`).
    None,
}

/// The HTML shell that loads a LOGOS→WASM DOM app, with a page's own `Head:` merged
/// over the defaults, carrying the built-in theme — [`browser_host_html_themed`] with
/// [`ShellTheme::Default`].
pub fn browser_host_html_with_head(
    name: &str,
    declared: Option<&str>,
    favicon: Option<&str>,
) -> String {
    browser_host_html_themed(name, declared, favicon, ShellTheme::Default)
}

/// The HTML shell that loads a LOGOS→WASM DOM app, with a page's own `Head:` merged
/// over the defaults.
///
/// The one merge every document builder reads — `largo build`'s pre-render, `largo serve`'s dev
/// response and `largo emit` — so development and production serve the same document.
/// `favicon` is the SITE's own icon when it ships one (`static/favicon.svg`), which
/// stands in for the built-in clay-λ data URI as the DEFAULT — so a page that declares
/// its own `<link rel="icon">` still wins over both. `theme` is whether the built-in theme
/// comes with it.
pub fn browser_host_html_themed(
    name: &str,
    declared: Option<&str>,
    favicon: Option<&str>,
    theme: ShellTheme,
) -> String {
    let declared: Vec<String> = declared.map(split_head_tags).unwrap_or_default();
    // A page declares its language in `Head:` as `<html lang="…">` (default `en`). `lang` is what
    // a screen reader picks a voice from and what translation, hyphenation and quotation rules
    // key off. The tag is LIFTED onto the document element rather than rendered, since an `<html>`
    // inside `<head>` is malformed.
    let (lang, declared): (String, Vec<String>) = {
        let mut lang = "en".to_string();
        let mut rest = Vec::with_capacity(declared.len());
        for tag in declared {
            match html_lang_of(&tag) {
                Some(l) => lang = l,
                None => rest.push(tag),
            }
        }
        (lang, rest)
    };
    let claimed: std::collections::HashSet<String> =
        declared.iter().filter_map(|t| head_identity(t)).collect();
    let mut head: Vec<String> = default_head_tags(name, favicon)
        .into_iter()
        .filter(|d| head_identity(d).is_none_or(|k| !claimed.contains(&k)))
        .collect();
    head.extend(declared);
    // The module script sits in the HEAD (module scripts run after parsing, so placement does not
    // delay it), which leaves the body holding exactly the page: a pre-render replaces the
    // `<body>\n` line with the body and its markup, and the host adopts every child of the body as
    // the page's first paint.
    let style = match theme {
        ShellTheme::Default => format!("<style>\n{DEFAULT_THEME_CSS}</style>\n"),
        ShellTheme::None => String::new(),
    };
    format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n{head}\n{style}\
         <script type=\"module\" src=\"./{name}.js\"></script>\n</head>\n<body>\n</body>\n</html>\n",
        lang = escape_html(&lang),
        head = head.join("\n"),
    )
}

/// `document` carrying the visitor's CSRF `token` in its head, as `<meta name="csrf-token">` — what
/// the server writes into every page it composes under a `## Session`, and what the browser host
/// (`runtime.js`'s `requestHeaders`) sends back as `x-csrf-token` on the page's state-changing
/// requests to its own server. The meta opens the head; a page's own `Head:` merges over it by
/// identity like every other head element. A document with no `<head>` is returned as it is.
pub fn with_csrf_meta(document: &str, token: &str) -> String {
    logicaffeine_system::csrf::with_csrf_meta(document, token)
}

/// The `lang` an author declared with `<html lang="…">` in a `Head:` block, if this tag is one.
///
/// Returns `None` for every ordinary head tag, so the caller can partition in one pass. An
/// `<html>` with no `lang` is still lifted (and answers `""`), because rendering it into the head
/// would be malformed either way — the author plainly meant it as a statement about the document.
fn html_lang_of(tag: &str) -> Option<String> {
    let t = tag.trim();
    let rest = t.strip_prefix("<html")?;
    if !rest.starts_with(|c: char| c.is_whitespace() || c == '>') {
        return None; // `<htmlfoo…>` is not the document element
    }
    let Some(at) = rest.find("lang") else {
        return Some(String::new());
    };
    let after = rest[at + 4..].trim_start();
    let Some(after) = after.strip_prefix('=') else {
        return Some(String::new());
    };
    let after = after.trim_start();
    let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    after[1..].split(quote).next().map(str::to_string)
}

/// Escape the HTML-significant characters so a source line / message embeds safely.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Overlay-specific styling, layered on the shared theme tokens.
const OVERLAY_CSS: &str = r#"
.err-wrap { max-width: min(820px, 94vw); margin: 3rem auto; }
.err-card { background: var(--surface); border: 1px solid var(--border); border-left: 4px solid var(--accent); border-radius: var(--radius); padding: 1.5rem 1.75rem; box-shadow: var(--shadow); }
.err-title { color: var(--accent); font-weight: 650; font-size: 1.05rem; margin: 0 0 .4rem; }
.err-msg { color: var(--ink); margin: 0 0 1.1rem; line-height: 1.55; }
.err-src { font-family: var(--font-mono); font-size: .9rem; line-height: 1.5; background: var(--paper); border: 1px solid var(--border); border-radius: 8px; padding: .8rem 1rem; overflow-x: auto; white-space: pre; }
.err-src .gutter { color: var(--muted); }
.err-src .caret { color: #cf5a4e; font-weight: 700; }
.err-hint { color: #2f7d8a; margin-top: .9rem; font-size: .95rem; }
"#;

/// Render a build error as a themed HTML overlay — the SAME source line + caret +
/// Socratic message + spelling suggestion the terminal shows, in the Claude palette,
/// with `reload_client` embedded so fixing the source snaps the page back to green.
/// Reuses [`ParseError::diagnostic_parts`] so the terminal and browser never drift.
///
/// [`ParseError::diagnostic_parts`]: logicaffeine_language::error::ParseError::diagnostic_parts
pub fn error_overlay_html(
    err: &logicaffeine_language::error::ParseError,
    source: &str,
    reload_client: &str,
) -> String {
    error_overlay_html_in(err, source, reload_client, None)
}

/// The same overlay, told WHICH FILE it is drawing.
///
/// A dev server serves many pages and each may import several modules, so "line 24" on its own
/// sends a reader to look in whichever file they had open. The name costs one line and removes
/// the guess.
pub fn error_overlay_html_in(
    err: &logicaffeine_language::error::ParseError,
    source: &str,
    reload_client: &str,
    file: Option<&str>,
) -> String {
    let p = err.diagnostic_parts(source);
    // Carets align by GLYPH: the browser renders each escaped entity (`&lt;`) as one
    // character in a `white-space: pre` block, so the offset is the unescaped prefix's
    // char count — not its byte length nor the escaped length.
    let col_chars = p
        .line
        .get(..p.column)
        .map_or(p.column, |s| s.chars().count());
    let caret = format!("{}{}", " ".repeat(col_chars), "^".repeat(p.caret_len));
    let suggestion = match p.suggestion {
        Some(s) => format!(
            "<p class=\"err-hint\">help: did you mean <b>{}</b>?</p>",
            escape_html(s)
        ),
        None => String::new(),
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>build error</title><style>{theme}{overlay}</style></head><body>\
<div class=\"err-wrap\"><div class=\"err-card\">\
<p class=\"err-title\">LOGOS build error — {where_}line {line_num}</p>\
<p class=\"err-msg\">{message}</p>\
<div class=\"err-src\"><span class=\"gutter\">{line_num:>4} | </span>{line}\n\
<span class=\"gutter\">     | </span><span class=\"caret\">{caret}</span></div>\
{suggestion}</div></div><script>{reload}</script></body></html>",
        theme = DEFAULT_THEME_CSS,
        overlay = OVERLAY_CSS,
        line_num = p.line_num,
        where_ = file.map(|f| format!("{f}, ")).unwrap_or_default(),
        message = escape_html(&p.message),
        line = escape_html(&p.line),
        caret = caret,
        suggestion = suggestion,
        reload = reload_client,
    )
}

/// The browser host: the production analog of the test `MockDom`. It supplies the
/// real DOM behind the `dom_*` imports, allocates host Text into linear memory, and
/// fires event/fetch callbacks through the exported `__logos_call_closure`
/// trampoline. Loading the emitted `.wasm` and calling `main()` builds the page.
pub fn browser_host_js(name: &str) -> String {
    let wasm_ref = serde_json::to_string(&format!("./{name}.wasm")).expect("json");
    // The runtime is a real file (`assets/web/runtime.js`), read at build time. The only thing
    // that varies per page is where the module lives, so that is all the caller supplies.
    format!("const WASM_URL = {wasm_ref};\n{}", BROWSER_RUNTIME_JS)
}

/// The browser host runtime, verbatim. Kept as a file so it can be executed by a test rather
/// than only string-matched — see `runtime_js_lock`.
pub const BROWSER_RUNTIME_JS: &str = include_str!("../assets/web/runtime.js");

/// Assemble a self-contained, sandboxable iframe `srcdoc` that COMPILES-and-RUNS an LGX
/// app entirely in the browser — the wasm module is embedded inline (no file to fetch), the
/// shared DOM runtime drives the iframe's own `document`, and the default theme makes it
/// beautiful. Powers the Studio's live "Web" preview. On a build failure returns the
/// `ParseError` (render it with [`error_overlay_html`]).
pub fn assemble_preview(source: &str) -> Result<String, logicaffeine_language::error::ParseError> {
    let wasm = crate::compile::compile_to_wasm(source)?;
    let bytes_csv = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    // The browser host itself, with its one fixed loader line swapped for the inline bytes, so the
    // module needs no network. The script sits in the head, leaving the body to the page.
    // (`assemble_preview_*` fails if the loader line's shape drifts.)
    let from = "const bytes = await (await fetch(WASM_URL)).arrayBuffer();";
    let to = format!("const bytes = new Uint8Array([{bytes_csv}]);");
    let js = browser_host_js("__preview__").replace(from, &to);
    Ok(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>preview</title><style>{css}</style><script type=\"module\">{js}</script></head>\
<body></body></html>",
        css = DEFAULT_THEME_CSS,
        js = js,
    ))
}

/// A float as LOGOS shows it, in the JavaScript every WebAssembly host runs: the display
/// [`logicaffeine_data::fmt::fmt_f64`] defines, byte for byte. ECMAScript's `Number.prototype.toString` gives the
/// shortest digits that read back to the same bits, as Rust's `{}` does, except where the double lies exactly halfway
/// between two shortest candidates: ECMAScript takes the even one and Rust the larger, so an exact tie (screened by
/// parsing, settled with the double's exact binary value in `BigInt`) moves up. The digits are spelled without
/// exponent notation, an integral value bare, and `NaN`, `inf`, `-inf` and `-0` as Rust spells them. The node shim
/// interpolates it and `runtime.js` carries the same text (`wasm_float_display_lock`).
pub const FMT_F64_JS: &str = "const fmtF = (v) => {
  if (Number.isNaN(v)) return 'NaN';
  if (v === Infinity) return 'inf';
  if (v === -Infinity) return '-inf';
  if (v === 0) return Object.is(v, -0) ? '-0' : '0';
  const sign = v < 0 ? '-' : '';
  const a = Math.abs(v);
  const s = a.toString();
  const e = s.indexOf('e');
  const mantissa = e < 0 ? s : s.slice(0, e);
  const dot = mantissa.indexOf('.');
  let digits = dot < 0 ? mantissa : mantissa.slice(0, dot) + mantissa.slice(dot + 1);
  let point = (dot < 0 ? mantissa.length : dot) + (e < 0 ? 0 : Number(s.slice(e + 1)));
  const lead = digits.length - digits.replace(/^0+/, '').length;
  digits = digits.slice(lead).replace(/0+$/, '');
  point -= lead;
  const spell = (d, p) => p <= 0 ? '0.' + '0'.repeat(-p) + d : p >= d.length ? d + '0'.repeat(p - d.length) : d.slice(0, p) + '.' + d.slice(p);
  if (Number(spell(digits + '5', point)) === a) {
    const view = new DataView(new ArrayBuffer(8));
    view.setFloat64(0, a);
    const bits = view.getBigUint64(0);
    const biased = Number(bits >> 52n);
    const m = biased === 0 ? bits & 0xfffffffffffffn : (bits & 0xfffffffffffffn) | (1n << 52n);
    const b = biased === 0 ? -1074 : biased - 1075;
    const q = point - digits.length - 1;
    const half = BigInt(digits + '5') * 10n ** BigInt(Math.max(q, 0)) * 2n ** BigInt(Math.max(-b, 0));
    const exact = m * 2n ** BigInt(Math.max(b, 0)) * 10n ** BigInt(Math.max(-q, 0));
    if (half === exact) {
      const up = (BigInt(digits) + 1n).toString();
      point += up.length - digits.length;
      digits = up.replace(/0+$/, '');
    }
  }
  return sign + spell(digits, point);
};";

/// The host's half of a module's exact integers (`exact_*`, `vm::wasm::module::exact`), as JavaScript: a
/// tagged word is `BigInt`, an even one the integer `t >> 1`, an odd one a slot of `exactHeld` that a dropped
/// handle gives back to `exactFree`. The arithmetic is `BigInt`'s, which truncates a quotient and gives a
/// remainder the dividend's sign as the reference tier does; a floored quotient corrects the truncated one.
/// `Number` of a `BigInt` is the nearest binary64, ties to even, as an `Int` widens to a Real. The node shim
/// interpolates it and `runtime.js` carries the same text (`exact_int_every_tier_lock`).
pub const EXACT_INT_JS: &str = "const exactHeld = [], exactFree = [];
const exactValue = (t) => ((t & 1n) === 0n ? t >> 1n : exactHeld[Number(t >> 1n)]);
const exactWord = (v) => {
  if (v >= -(1n << 62n) && v < (1n << 62n)) return v << 1n;
  const h = exactFree.length > 0 ? exactFree.pop() : exactHeld.length;
  exactHeld[h] = v;
  return (BigInt(h) << 1n) | 1n;
};
const exactImports = {
  exact_box: (v) => exactWord(v),
  exact_narrow: (t) => { const v = exactValue(t); return v >= -(1n << 63n) && v < (1n << 63n) ? v : 0n; },
  exact_add: (a, b) => exactWord(exactValue(a) + exactValue(b)),
  exact_sub: (a, b) => exactWord(exactValue(a) - exactValue(b)),
  exact_mul: (a, b) => exactWord(exactValue(a) * exactValue(b)),
  exact_div: (a, b) => exactWord(exactValue(a) / exactValue(b)),
  exact_mod: (a, b) => exactWord(exactValue(a) % exactValue(b)),
  exact_floordiv: (a, b) => { const x = exactValue(a), y = exactValue(b), q = x / y; return exactWord(x % y !== 0n && (x < 0n) !== (y < 0n) ? q - 1n : q); },
  exact_pow: (a, e) => exactWord(exactValue(a) ** e),
  exact_cmp: (a, b) => { const x = exactValue(a), y = exactValue(b); return x < y ? -1 : x > y ? 1 : 0; },
  exact_f64: (t) => Number(exactValue(t)),
  exact_clone: (t) => exactWord(exactValue(t)),
  exact_drop: (t) => { const h = Number(t >> 1n); exactHeld[h] = undefined; exactFree.push(h); },
};";

/// The page's markup: the first paint of the page's own lowered program, the one its module runs,
/// drawn in process, or the reason it cannot be rendered without a browser. `largo build` writes it
/// into the document it ships, and [`assemble_preview_painted`] into a preview's frame; either way
/// the page's mount adopts it (`data-logos-prerendered`) rather than drawing it again.
pub fn first_paint(source: &str) -> Result<String, String> {
    crate::semantics::dom::reset();
    // The markup a reader without scripts sees is the page at its own URL: `routePath()` answers
    // the `## Page` path during the render, as it does on every tier a `## Test` runs.
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        if let Ok((stmts, ..)) = parsed {
            crate::page_host::boot_at_declared_route(stmts, interner);
        }
    });
    let out = crate::compile::vm_outcome(source);
    if let Some(e) = out.error {
        return Err(e);
    }
    let rendered = crate::semantics::dom::render_served();
    // `render_served()` wraps the page in the recorder's `<body>`; the document already has one.
    let inner = rendered
        .strip_prefix("<body>")
        .and_then(|r| r.strip_suffix("</body>"))
        .ok_or_else(|| "the page's render did not produce a document body".to_string())?;
    if inner.is_empty() {
        // The commonest shape by far, and the one that used to look like success: nothing was
        // mounted, so there was nothing to draw.
        return Err(
            "it renders nothing — no `## Page` and no `Mount`, so the document is empty"
                .to_string(),
        );
    }
    Ok(inner.to_string())
}

/// [`assemble_preview`] with the page's [`first_paint`] already in the frame's body, marked for the
/// page's mount to adopt, so the frame shows the page the moment its document parses rather than
/// once its module has run, and a frame that reloads with the same page shows it throughout. A
/// program with no first paint is previewed as [`assemble_preview`] previews it.
pub fn assemble_preview_painted(
    source: &str,
) -> Result<String, logicaffeine_language::error::ParseError> {
    let document = assemble_preview(source)?;
    let Ok(markup) = first_paint(source) else {
        return Ok(document);
    };
    // The body closes the document; the runtime in the head may spell the same tags in its text.
    let body = "<body></body>";
    let Some(at) = document.rfind(body) else {
        return Ok(document);
    };
    let mark = logicaffeine_language::parser::lgx_lower::PRERENDERED_ATTR;
    Ok(format!(
        "{}<body {mark}=\"1\">\n{markup}\n</body>{}",
        &document[..at],
        &document[at + body.len()..]
    ))
}

/// The JavaScript host shim written beside an emitted `.wasm` — supplies every `env.*` import the
/// backend can emit (the display sinks, the float display [`FMT_F64_JS`],
/// the scalar→Text formatters, `parse_int`, `pow`, and an `args` host that materializes `process.argv`
/// as a `Seq of Text` in the module's linear memory). Running it needs only a wasm engine (node/V8).
pub fn wasm_host_shim(name: &str) -> String {
    // The module filename lands inside generated JavaScript — JSON-encode it
    // so apostrophes/backslashes in a file stem can't corrupt the shim.
    let wasm_ref =
        serde_json::to_string(&format!("./{name}.wasm")).expect("a string always JSON-encodes");
    let fail_cases = crate::semantics::failcode::js_cases("failMessage", "v", "code");
    let fmt_f64_js = FMT_F64_JS;
    let exact_int_js = EXACT_INT_JS;
    format!(
        r#"// Auto-generated by `largo build --emit wasm`. Run: node <this file> [args…]
import {{ readFileSync }} from 'node:fs';
const bytes = readFileSync(new URL({wasm_ref}, import.meta.url));
let inst;
const dv = () => new DataView(inst.exports.memory.buffer);
const u8 = () => new Uint8Array(inst.exports.memory.buffer);
const dec = new TextDecoder(), enc = new TextEncoder();
const readText = (h) => {{ const d = dv(); const len = d.getInt32(h, true), dp = d.getInt32(h + 8, true); return dec.decode(u8().subarray(dp, dp + len)); }};
const writeStr = (buf, s) => {{ const b = enc.encode(s); u8().set(b, buf); return b.length; }};
const allocText = (s) => {{ const by = enc.encode(s); const t = inst.exports.__logos_text_new(by.length); u8().set(by, dv().getInt32(t + 8, true)); return t; }};
{exact_int_js}
{fmt_f64_js}
// A uniform draw from lo..=hi (`randomInt`): 64-bit words from `crypto.getRandomValues`, a word at
// or above the largest multiple of the span drawn again, so no value is likelier than another.
const randomWord = () => {{ const w = new BigUint64Array(1); crypto.getRandomValues(w); return w[0]; }};
const randomIntIn = (lo, hi) => {{
  if (lo > hi) throw new Error(`randomInt(${{lo}}, ${{hi}}): the low bound is above the high bound`);
  const span = hi - lo + 1n;
  if (span === 1n << 64n) return BigInt.asIntN(64, randomWord());
  const limit = (1n << 64n) - (1n << 64n) % span;
  for (;;) {{ const r = randomWord(); if (r < limit) return lo + r % span; }}
}};
const seqInts = (h, fmt) => {{ const d = dv(); const len = d.getInt32(h, true), dp = d.getInt32(h + 8, true); const e = []; for (let i = 0; i < len; i++) e.push(fmt(d, dp + i * 8)); return e; }};
// Calendar arithmetic — Howard Hinnant's `days_from_civil`/`civil_from_days` over BigInt, the
// same algorithm `logicaffeine_base::temporal` runs, so `temporal_component`/`temporal_component_date`
// answer bit-identically to the reference tier.
const daysFromCivil = (year, month, day) => {{
  const y = month <= 2n ? year - 1n : year;
  const era = (y >= 0n ? y : y - 399n) / 400n;
  const yoe = y - era * 400n;
  const doy = (153n * (month > 2n ? month - 3n : month + 9n) + 2n) / 5n + day - 1n;
  const doe = yoe * 365n + yoe / 4n - yoe / 100n + doy;
  return era * 146097n + doe - 719468n;
}};
const civilFromDays = (z) => {{
  const zz = z + 719468n;
  const era = (zz >= 0n ? zz : zz - 146096n) / 146097n;
  const doe = zz - era * 146097n;
  const yoe = (doe - doe / 1460n + doe / 36524n - doe / 146096n) / 365n;
  const y = yoe + era * 400n;
  const doy = doe - (365n * yoe + yoe / 4n - yoe / 100n);
  const mp = (5n * doy + 2n) / 153n;
  const day = doy - (153n * mp + 2n) / 5n + 1n;
  const month = mp < 10n ? mp + 3n : mp - 9n;
  const year = month <= 2n ? y + 1n : y;
  return [year, month, day];
}};
const isoWeekdayFromDays = (z) => ((((z + 3n) % 7n) + 7n) % 7n) + 1n;
const isoWeekFromDays = (z) => {{
  const isoDow = isoWeekdayFromDays(z);
  const thursday = z + (4n - isoDow);
  const [isoYear] = civilFromDays(thursday);
  const jan4 = daysFromCivil(isoYear, 1n, 4n);
  const week1Monday = jan4 - (isoWeekdayFromDays(jan4) - 1n);
  return (z - week1Monday) / 7n + 1n;
}};
const NANOS_PER_DAY = 86400000000000n;
const NANOS_PER_SECOND = 1000000000n;
const divEuclid = (a, b) => {{ const q = a / b; return a % b < 0n ? q - 1n : q; }};
const remEuclid = (a, b) => {{ const r = a % b; return r < 0n ? r + b : r; }};
const isGregorianLeap = (year) => (year % 4n === 0n && year % 100n !== 0n) || year % 400n === 0n;
const lastDayOfMonth = (year, month) => {{
  if ([1n, 3n, 5n, 7n, 8n, 10n, 12n].includes(month)) return 31n;
  if ([4n, 6n, 9n, 11n].includes(month)) return 30n;
  if (month === 2n) return isGregorianLeap(year) ? 29n : 28n;
  return 0n;
}};
const parseRfc3339 = (s) => {{
  const bad = () => new Error('invalid RFC 3339 timestamp: ' + s);
  if (s.length < 20) throw bad();
  const digits = (start, len) => {{
    let v = 0n;
    for (let i = start; i < start + len; i++) {{
      const c = s.charCodeAt(i);
      if (c < 48 || c > 57) throw bad();
      v = v * 10n + BigInt(c - 48);
    }}
    return v;
  }};
  if (s[4] !== '-' || s[7] !== '-' || s[10] !== 'T' || s[13] !== ':' || s[16] !== ':') throw bad();
  const year = digits(0, 4), month = digits(5, 2), day = digits(8, 2);
  const hour = digits(11, 2), minute = digits(14, 2), second = digits(17, 2);
  if (month < 1n || month > 12n || day < 1n || day > lastDayOfMonth(year, month)) throw bad();
  if (hour > 23n || minute > 59n || second > 59n) throw bad();
  let pos = 19;
  let nanosecond = 0n;
  if (s[pos] === '.') {{
    pos += 1;
    let frac = 0n, count = 0;
    while (pos < s.length && s[pos] >= '0' && s[pos] <= '9') {{
      if (count < 9) {{ frac = frac * 10n + BigInt(s.charCodeAt(pos) - 48); count += 1; }}
      pos += 1;
    }}
    if (count === 0) throw bad();
    for (let i = count; i < 9; i++) frac *= 10n;
    nanosecond = frac;
  }}
  let offsetSeconds = 0n;
  if (s[pos] === 'Z') {{
    pos += 1;
  }} else if (s[pos] === '+' || s[pos] === '-') {{
    const sign = s[pos] === '+' ? 1n : -1n;
    const oh = digits(pos + 1, 2);
    if (s[pos + 3] !== ':') throw bad();
    const om = digits(pos + 4, 2);
    if (oh > 23n || om > 59n) throw bad();
    pos += 6;
    offsetSeconds = sign * (oh * 3600n + om * 60n);
  }} else {{
    throw bad();
  }}
  if (pos !== s.length) throw bad();
  const days = daysFromCivil(year, month, day);
  const nanos = days * NANOS_PER_DAY + (hour * 3600n + minute * 60n + second) * NANOS_PER_SECOND + nanosecond;
  return nanos - offsetSeconds * NANOS_PER_SECOND;
}};
const temporalComponent = (nanos, which) => {{
  const day = divEuclid(nanos, NANOS_PER_DAY);
  const rem = remEuclid(nanos, NANOS_PER_DAY);
  const [year, month, d] = civilFromDays(day);
  const secs = rem / NANOS_PER_SECOND;
  switch (Number(which)) {{
    case 0: return year;
    case 1: return month;
    case 2: return d;
    case 3: return secs / 3600n;
    case 4: return (secs % 3600n) / 60n;
    case 5: return secs % 60n;
    case 6: return isoWeekdayFromDays(day);
    case 7: return isoWeekFromDays(day);
    case 8: return (month - 1n) / 3n + 1n;
    default: throw new Error('temporal_component: unknown component selector ' + which);
  }}
}};
const temporalComponentDate = (days, which) => {{
  const z = BigInt(days);
  const [year, month, d] = civilFromDays(z);
  switch (Number(which)) {{
    case 0: return year;
    case 1: return month;
    case 2: return d;
    case 6: return isoWeekdayFromDays(z);
    case 7: return isoWeekFromDays(z);
    case 8: return (month - 1n) / 3n + 1n;
    default: throw new Error('temporal_component_date: unknown component selector ' + which);
  }}
}};
const padSigned = (n, w) => {{
  const neg = n < 0;
  const digits = String(neg ? -n : n).padStart(neg ? w - 1 : w, '0');
  return neg ? '-' + digits : digits;
}};
const pad2 = (n) => String(n).padStart(2, '0');
const printDate = (days) => {{
  const [y, m, d] = civilFromDays(BigInt(days));
  return padSigned(Number(y), 4) + '-' + pad2(m) + '-' + pad2(d);
}};
const printMoment = (nanos) => {{
  const totalSeconds = divEuclid(nanos, NANOS_PER_SECOND);
  const days = divEuclid(totalSeconds, 86400n);
  const daySeconds = remEuclid(totalSeconds, 86400n);
  const hours = daySeconds / 3600n, minutes = (daySeconds % 3600n) / 60n;
  const [y, m, d] = civilFromDays(days);
  return padSigned(Number(y), 4) + '-' + pad2(m) + '-' + pad2(d) + ' ' + pad2(hours) + ':' + pad2(minutes);
}};
const printTime = (nanosFromMidnight) => {{
  const rem = remEuclid(nanosFromMidnight, NANOS_PER_DAY);
  const secs = rem / NANOS_PER_SECOND;
  let s = pad2(secs / 3600n) + ':' + pad2((secs % 3600n) / 60n) + ':' + pad2(secs % 60n);
  const nanos = rem % NANOS_PER_SECOND;
  if (nanos !== 0n) {{
    let frac = nanos.toString().padStart(9, '0');
    while (frac.endsWith('0')) frac = frac.slice(0, -1);
    s += '.' + frac;
  }}
  return s;
}};
const printDuration = (nanos) => {{
  const abs = nanos < 0n ? -nanos : nanos;
  const sign = nanos < 0n ? '-' : '';
  if (abs >= 3600000000000n) return sign + (abs / 3600000000000n) + 'h';
  if (abs >= 60000000000n) return sign + (abs / 60000000000n) + 'min';
  if (abs >= 1000000000n) return sign + (abs / 1000000000n) + 's';
  if (abs >= 1000000n) return sign + (abs / 1000000n) + 'ms';
  if (abs >= 1000n) return sign + (abs / 1000n) + 'μs';
  return sign + abs + 'ns';
}};
const printSpan = (packed) => {{
  const months = Number(BigInt.asIntN(32, packed >> 32n));
  const days = Number(BigInt.asIntN(32, packed));
  const parts = [];
  const years = Math.trunc(months / 12), remMonths = months % 12;
  if (years !== 0) parts.push(Math.abs(years) === 1 ? (years + ' year') : (years + ' years'));
  if (remMonths !== 0) parts.push(Math.abs(remMonths) === 1 ? (remMonths + ' month') : (remMonths + ' months'));
  if (days !== 0 || parts.length === 0) parts.push(Math.abs(days) === 1 ? (days + ' day') : (days + ' days'));
  return parts.join(' and ');
}};
const alignPad = (s, width, align) => {{
  const chars = Array.from(s);
  const pad = width - chars.length;
  if (pad <= 0) return s;
  if (align === 1) return s + ' '.repeat(pad);
  if (align === 2) {{ const l = Math.floor(pad / 2); return ' '.repeat(l) + s + ' '.repeat(pad - l); }}
  return ' '.repeat(pad) + s;
}};
const out = [];
let failMessage = '';
const env = {{
  ...exactImports,
  print_exact: (t) => out.push(exactValue(t).toString()),
  exact_text: (t) => allocText(exactValue(t).toString()),
  print_i64: (v) => out.push(v.toString()),
  // A runtime failure hands its reason over just before the module traps. Without these two,
  // a module that CAN fail does not instantiate at all — the program never starts, and the
  // error reads nothing like the mistake that caused it.
  logos_rt_fail: (h) => {{ failMessage = readText(h); }},
  logos_rt_fail_at: (code, value) => {{
    const v = typeof value === 'bigint' ? value.toString() : String(value);
    switch (code) {{ {fail_cases} }}
  }},
  print_bool: (v) => out.push(v ? 'true' : 'false'),
  print_f64: (v) => out.push(fmtF(v)),
  print_char: (v) => out.push(String.fromCodePoint(Number(v))),
  print_word: (v) => out.push(BigInt.asUintN(64, v).toString()),
  print_text: (h) => out.push(readText(h)),
  print_date: (d) => out.push(printDate(d)),
  print_moment: (n) => out.push(printMoment(n)),
  print_duration: (n) => out.push(printDuration(n)),
  print_time: (n) => out.push(printTime(n)),
  print_span: (packed) => out.push(printSpan(packed)),
  print_rational: (num, den) => out.push(den === 1n ? num.toString() : (num + '/' + den)),
  print_nothing: () => out.push('nothing'),
  print_seq_i64: (h) => out.push('[' + seqInts(h, (d, a) => d.getBigInt64(a, true).toString()).join(', ') + ']'),
  print_seq_bool: (h) => out.push('[' + seqInts(h, (d, a) => (d.getBigInt64(a, true) !== 0n).toString()).join(', ') + ']'),
  print_seq_word32: (h) => out.push('[' + seqInts(h, (d, a) => d.getUint32(a, true).toString()).join(', ') + ']'),
  print_seq_word64: (h) => out.push('[' + seqInts(h, (d, a) => d.getBigUint64(a, true).toString()).join(', ') + ']'),
  print_seq_f64: (h) => out.push('[' + seqInts(h, (d, a) => fmtF(d.getFloat64(a, true))).join(', ') + ']'),
  print_seq_text: (h) => out.push('[' + seqInts(h, (d, a) => readText(d.getInt32(a, true))).join(', ') + ']'),
  print_set_i64: (h) => out.push('{{' + seqInts(h, (d, a) => d.getBigInt64(a, true).toString()).join(', ') + '}}'),
  print_set_text: (h) => out.push('{{' + seqInts(h, (d, a) => readText(d.getInt32(a, true))).join(', ') + '}}'),
  fmt_i64_into: (buf, v) => writeStr(buf, v.toString()),
  fmt_f64_into: (buf, v) => writeStr(buf, fmtF(v)),
  fmt_bool_into: (buf, v) => writeStr(buf, v ? 'true' : 'false'),
  fmt_f64_prec_into: (buf, v, prec) => writeStr(buf, v.toFixed(prec)),
  fmt_align_into: (buf, text, width, align) => writeStr(buf, alignPad(readText(text), Number(width), Number(align))),
  fmt_seq_i64_into: (buf, h) => writeStr(buf, '[' + seqInts(h, (d, a) => d.getBigInt64(a, true).toString()).join(', ') + ']'),
  fmt_seq_bool_into: (buf, h) => writeStr(buf, '[' + seqInts(h, (d, a) => (d.getBigInt64(a, true) !== 0n).toString()).join(', ') + ']'),
  fmt_set_i64_into: (buf, h) => writeStr(buf, '{{' + seqInts(h, (d, a) => d.getBigInt64(a, true).toString()).join(', ') + '}}'),
  parse_int: (h) => BigInt(parseInt(readText(h).trim(), 10) || 0),
  parse_float: (h) => parseFloat(readText(h).trim()),
  read_reals: (h) => {{
    // The file's binary64 values, little-endian as the module's memory is, on a page the shim grows (the
    // module's own allocator never grows memory), refused as `logicaffeine_system::file::read_reals` refuses.
    const path = readText(h);
    let bytes;
    try {{ bytes = readFileSync(path); }} catch (e) {{ throw new Error(`readReals: cannot read '${{path}}': ${{e.message}}`); }}
    if (bytes.length % 8 !== 0) throw new Error(`readReals: '${{path}}' holds ${{bytes.length}} bytes, not a whole number of 8-byte Reals`);
    const n = bytes.length / 8;
    const seq = inst.exports.memory.grow(Math.ceil((16 + bytes.length) / 65536)) * 65536;
    const d = dv();
    d.setInt32(seq, n, true); d.setInt32(seq + 4, n, true); d.setInt32(seq + 8, seq + 16, true);
    u8().set(bytes, seq + 16);
    return seq;
  }},
  parse_timestamp: (h) => parseRfc3339(readText(h)),
  text_contains: (haystack, needle) => (readText(haystack).includes(readText(needle)) ? 1n : 0n),
  random_int: (lo, hi) => randomIntIn(lo, hi),
  env_len: (h) => {{ const v = process.env[readText(h)]; return v === undefined ? -1 : enc.encode(v).length; }},
  env_into: (buf, h) => writeStr(buf, process.env[readText(h)] ?? ''),
  time_now: () => BigInt(Date.now()),
  pow_ff: (a, b) => Math.pow(a, b),
  pow_fi: (a, b) => Math.pow(a, Number(b)),
  // Real wall-clock time: a compiled module running under `node` has no test-fixed-clock concept
  // (only the Rust `wasmi` test host, reading the tree-walker's own thread-local fixed clock,
  // needs that determinism).
  today: () => Math.floor(Date.now() / 86400000),
  now: () => BigInt(Date.now()) * 1000000n,
  // `write_wire_residual(data_ptr, len) -> the byte count` — the raw bytes written straight to
  // real stdout, exactly as the tree-walker's own `BuiltinId::WriteWireResidual` does.
  write_wire_residual: (dataPtr, len) => {{
    const bytes = u8().slice(Number(dataPtr), Number(dataPtr) + Number(len));
    process.stdout.write(Buffer.from(bytes));
    return BigInt(len);
  }},
  // Wave 13 observability: `Op::Obs` is populated only by the OBSERVED compile profile, which
  // `largo build --emit wasm` never uses — declared for ABI completeness, never actually reached.
  obs: (_site, _p0, _p1, _p2) => {{}},
  temporal_component: (nanos, which) => temporalComponent(nanos, which),
  temporal_component_date: (days, which) => temporalComponentDate(days, which),
  args: () => {{
    // argv lives on a page the SHIM grows: the program's bump allocator and
    // iterator stack are confined to the module's original pages (the
    // emitter never calls memory.grow), so a fresh page can never collide
    // with live program data. Cached: repeated args() calls share one page.
    if (argvSeq) return argvSeq;
    const argv = process.argv.slice(1); // [scriptPath, ...userArgs] — Logos `item 1` is the program
    const seq = inst.exports.memory.grow(1) * 65536;
    const d = dv(), b = u8(); const n = argv.length, seqData = seq + 16; let p = seqData + n * 8; const hs = [];
    for (const s of argv) {{ const t = p, by = enc.encode(s), td = t + 16; d.setInt32(t, by.length, true); d.setInt32(t + 4, by.length, true); d.setInt32(t + 8, td, true); b.set(by, td); hs.push(t); p = (td + by.length + 7) & ~7; }}
    d.setInt32(seq, n, true); d.setInt32(seq + 4, n, true); d.setInt32(seq + 8, seqData, true);
    for (let i = 0; i < n; i++) d.setBigInt64(seqData + i * 8, BigInt(hs[i]), true);
    argvSeq = seq;
    return seq;
  }},
}};
let argvSeq = 0;
const {{ instance }} = await WebAssembly.instantiate(bytes, {{ env }});
inst = instance;
// Flush whatever the program produced even when it traps (overflow etc.);
// a trap still fails the run loudly after the flush.
let trapped = null;
try {{
  instance.exports.main();
}} catch (e) {{
  trapped = e;
}}
process.stdout.write(out.join('\n') + (out.length ? '\n' : ''));
if (trapped) {{
  // A failed run ends as it ends on every other tier: its reason and a newline on stderr, exit status 1.
  // `failMessage` is the reference tier's sentence; a trap the module gave no reason for prints the engine's.
  process.stderr.write((failMessage || String(trapped && trapped.message ? trapped.message : trapped)) + '\n');
  process.exit(1);
}}
"#
    )
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/web_host/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/web_host/first_paint_tests.rs"]
mod first_paint_tests;
