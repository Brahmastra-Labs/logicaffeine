//! The LGX HTML tokenizer — real markup for the `View:` section of a component.
//!
//! Flat and pure: a raw view body becomes start/end tags, text runs and `{…}` brace groups,
//! each with the byte span it came from. Nothing here knows the LOGOS expression grammar
//! (a brace group's inner text is handed to the template parser, which hands holes to the
//! real expression parser) and nothing here builds a tree (nesting, whitespace collapse and
//! template tags live in [`super::template`]). What it does own:
//!
//! - tag and attribute syntax: quoted, unquoted and valueless attributes; `/>`; the HTML
//!   void elements, which never take an end tag;
//! - `{…}` groups in text and inside attribute values — scanned STRING- and NESTING-aware,
//!   so a LOGOS interpolation inside a hole (`{greet("{x}")}`) stays one group and a quoted
//!   `"}"` cannot end one;
//! - character references (`&amp;`, `&#169;`, `&#x41;`) in text and static attribute values;
//! - `<!-- -->` comments (dropped);
//! - the refusals a component template must make: `<script>`, `<style>` and a doctype are
//!   not a view's business (styles live in `Style:`, the document shell in a layout).
//!
//! Errors carry the span of the offending construct and a message that names the fix.

use crate::token::Span;

/// One flat token of a view body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlToken {
    /// `<name attr…>` or `<name …/>`. `self_closing` is true for an explicit `/>` AND for
    /// every HTML void element (`<br>`, `<input>`, …), which never has an end tag.
    StartTag {
        name: String,
        attrs: Vec<HtmlAttr>,
        self_closing: bool,
        span: Span,
    },
    /// `</name>`.
    EndTag { name: String, span: Span },
    /// A text run with character references decoded and whitespace kept VERBATIM.
    Text { text: String, span: Span },
    /// A `{…}` group in text position: `inner` is the text between the braces.
    Brace { inner: String, span: Span },
}

/// One attribute of a start tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlAttr {
    pub name: String,
    /// `None` for a valueless attribute (`disabled`).
    pub value: Option<AttrValue>,
    pub span: Span,
}

/// An attribute's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrValue {
    /// No `{…}` inside — a plain string (character references decoded).
    Static(String),
    /// Text and brace groups interleaved (`"badge {tone}"`), or a bare `{expr}`.
    Parts(Vec<AttrPart>),
}

/// One piece of a dynamic attribute value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrPart {
    Text(String),
    Brace { inner: String, span: Span },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlErrorKind {
    /// A `<…` that never reaches its `>`.
    UnterminatedTag,
    /// A quoted attribute value that never closes.
    UnterminatedAttributeValue,
    /// A `{` with no matching `}`.
    UnterminatedBrace,
    /// A `}` with no `{` before it.
    StrayCloseBrace,
    /// A `<!--` that never reaches `-->`.
    UnterminatedComment,
    /// `<script>`, `<style>` or a doctype inside a view.
    ForbiddenElement(String),
    /// An attribute with no name (`<p =x>`), or a name containing a forbidden character.
    BadAttributeName(String),
    /// `<` followed by something that cannot start a tag (`<1>`, `< a>`, `<>`).
    BadTagName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlError {
    pub kind: HtmlErrorKind,
    pub span: Span,
}

impl HtmlError {
    /// The Socratic explanation: what went wrong and what to write instead.
    pub fn message(&self) -> String {
        match &self.kind {
            HtmlErrorKind::UnterminatedTag => "This tag never closes — I reached the end of the view still inside `<…`. Finish it with `>` (or `/>` for an element with no children).".into(),
            HtmlErrorKind::UnterminatedAttributeValue => "This attribute value opens a quote that never closes. Close it with the same quote, e.g. `class=\"card\"`.".into(),
            HtmlErrorKind::UnterminatedBrace => "This `{` opens a hole or template tag that never closes. Every `{…}` needs its `}` — inside it, LOGOS strings are quoted, so a `}` in a string does not count.".into(),
            HtmlErrorKind::StrayCloseBrace => "This `}` closes nothing. To show a literal brace, write it as a string hole: `{\"}\"}`.".into(),
            HtmlErrorKind::UnterminatedComment => "This `<!--` comment never reaches its `-->`.".into(),
            HtmlErrorKind::ForbiddenElement(name) => match name.as_str() {
                "script" => "A view cannot contain `<script>`: behavior belongs in `Actions:` (the compiler generates the browser code). Write the logic as an action and attach it with `on-click={…}`.".into(),
                "style" => "A view cannot contain `<style>`: put the rules in the component's `Style:` section — they are scoped to this component automatically.".into(),
                _ => "A view is a fragment, never a whole document: leave out `<!doctype>` and `<html>`. The page shell comes from the layout the page `{extends}`.".into(),
            },
            HtmlErrorKind::BadAttributeName(found) => format!("`{found}` cannot start an attribute. Attributes are `name`, `name=\"value\"`, `name={{expression}}`, or a directive like `on-click={{action}}`."),
            HtmlErrorKind::BadTagName => "A `<` must start a tag name (`<div>`, `<TodoRow>`), an end tag (`</div>`) or a comment (`<!-- -->`). To show a literal `<`, write `&lt;`.".into(),
        }
    }
}

/// The HTML void elements: they never take an end tag.
pub const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

pub fn is_void(name: &str) -> bool {
    VOID_ELEMENTS.iter().any(|v| v.eq_ignore_ascii_case(name))
}

/// The HTML elements, as the WHATWG HTML standard names them (plus the `svg` and `math` roots of
/// the foreign content it embeds).
pub const HTML_ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "bdi",
    "bdo",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "math",
    "menu",
    "meta",
    "meter",
    "nav",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "search",
    "section",
    "select",
    "slot",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "svg",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

pub fn is_html_element(name: &str) -> bool {
    HTML_ELEMENTS.iter().any(|e| e.eq_ignore_ascii_case(name))
}

/// Whether `target` reads as a CSS selector for a mount point rather than a file path: an id
/// (`#root`), a class (`.app`), an attribute test (`[data-app]`), or exactly an element name
/// (`main`, `body`).
pub fn is_selector_shaped(target: &str) -> bool {
    let t = target.trim();
    t.starts_with('#') || t.starts_with('.') || t.starts_with('[') || is_html_element(t)
}

/// What an `on-<event>.<modifier>` qualifier asks for. Every tier reads a handler's modifiers
/// through this type; the words themselves live only in the tables below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventModifier {
    /// How the browser treats the event once a listener has it.
    Behaviour(Behaviour),
    /// The one key a keyboard handler answers.
    Key(KeyName),
    /// A modifier key that must be held while the event happens.
    System(SystemKey),
}

/// The behavioural qualifiers: they shape what happens to the event, not which event it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// `.prevent` — the browser's default action (a form posting, a link following) does not run.
    Prevent,
    /// `.stop` — the event does not propagate to the element's ancestors.
    Stop,
    /// `.once` — the listener answers the first event and is then removed.
    Once,
    /// `.capture` — the listener runs in the capture phase, before the target's own listeners.
    Capture,
    /// `.passive` — the listener promises not to prevent the default action.
    Passive,
}

/// A key a keyboard qualifier names: one from [`KEY_MODIFIERS`], or a single printable character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyName {
    /// A named key, held as its `KeyboardEvent.key` spelling (`"Enter"`, `" "` for space).
    Named(&'static str),
    /// A printable character — `on-keydown.a` answers the `a` key.
    Char(char),
}

/// The system keys a qualifier may require to be held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemKey {
    Ctrl,
    Alt,
    Shift,
    Meta,
}

/// `(qualifier, behaviour)` — the behavioural modifiers.
pub const BEHAVIOUR_MODIFIERS: &[(&str, Behaviour)] = &[
    ("prevent", Behaviour::Prevent),
    ("stop", Behaviour::Stop),
    ("once", Behaviour::Once),
    ("capture", Behaviour::Capture),
    ("passive", Behaviour::Passive),
];

/// `(qualifier, KeyboardEvent.key)` — the named keys a keyboard handler may wait for. The second
/// column is what a browser reports for the key and what a driving `Press` names; the two are
/// compared case-insensitively.
pub const KEY_MODIFIERS: &[(&str, &str)] = &[
    ("enter", "Enter"),
    ("escape", "Escape"),
    ("space", " "),
    ("tab", "Tab"),
    ("backspace", "Backspace"),
    ("delete", "Delete"),
    ("arrowup", "ArrowUp"),
    ("arrowdown", "ArrowDown"),
    ("arrowleft", "ArrowLeft"),
    ("arrowright", "ArrowRight"),
    ("home", "Home"),
    ("end", "End"),
    ("pageup", "PageUp"),
    ("pagedown", "PageDown"),
];

/// `(qualifier, key)` — the system keys.
pub const SYSTEM_KEY_MODIFIERS: &[(&str, SystemKey)] = &[
    ("ctrl", SystemKey::Ctrl),
    ("alt", SystemKey::Alt),
    ("shift", SystemKey::Shift),
    ("meta", SystemKey::Meta),
];

/// The events that carry a key. A key qualifier on any other event could never match.
pub const KEY_EVENTS: &[&str] = &["keydown", "keyup", "keypress"];

/// Whether `event` is one of [`KEY_EVENTS`].
pub fn is_key_event(event: &str) -> bool {
    KEY_EVENTS.iter().any(|e| e.eq_ignore_ascii_case(event))
}

impl EventModifier {
    /// Classify one qualifier word, or `None` when the catalogue does not know it.
    ///
    /// Named qualifiers match case-insensitively, as HTML attribute names do. Any other single
    /// printable character is the key of that character.
    pub fn classify(word: &str) -> Option<EventModifier> {
        fn row<T: Copy>(table: &[(&'static str, T)], word: &str) -> Option<T> {
            table
                .iter()
                .find(|(w, _)| w.eq_ignore_ascii_case(word))
                .map(|(_, v)| *v)
        }
        if let Some(b) = row(BEHAVIOUR_MODIFIERS, word) {
            return Some(EventModifier::Behaviour(b));
        }
        if let Some(k) = row(KEY_MODIFIERS, word) {
            return Some(EventModifier::Key(KeyName::Named(k)));
        }
        if let Some(s) = row(SYSTEM_KEY_MODIFIERS, word) {
            return Some(EventModifier::System(s));
        }
        let mut chars = word.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if !c.is_control() && !c.is_whitespace() => {
                Some(EventModifier::Key(KeyName::Char(c)))
            }
            _ => None,
        }
    }

    /// The qualifier as it is written after the `.`.
    pub fn word(&self) -> String {
        match self {
            EventModifier::Behaviour(b) => b.word().to_string(),
            EventModifier::Key(KeyName::Named(key)) => KEY_MODIFIERS
                .iter()
                .find(|(_, k)| k == key)
                .map(|(w, _)| w.to_string())
                .unwrap_or_else(|| key.to_ascii_lowercase()),
            EventModifier::Key(KeyName::Char(c)) => c.to_string(),
            EventModifier::System(s) => s.word().to_string(),
        }
    }

    /// The key a handler with these modifiers waits for, as a browser reports it — `None` for a
    /// handler that answers every key.
    pub fn key_of(modifiers: &[EventModifier]) -> Option<String> {
        modifiers.iter().find_map(|m| match m {
            EventModifier::Key(k) => Some(k.event_key()),
            _ => None,
        })
    }

    /// The bits a host's listen registration carries for these modifiers (`Behaviour::listen_bit`)
    /// — every behaviour a host dispatches by. `.once` is not among them: a host never retires a
    /// listener on its own, and the lowering keeps `.once` as compiled LOGOS.
    pub fn host_listen_bits(modifiers: &[EventModifier]) -> u8 {
        modifiers
            .iter()
            .filter_map(|m| match m {
                EventModifier::Behaviour(Behaviour::Once) => None,
                EventModifier::Behaviour(b) => Some(b.listen_bit()),
                EventModifier::Key(_) | EventModifier::System(_) => None,
            })
            .fold(0, |bits, bit| bits | bit)
    }

    /// Every qualifier the catalogue names, for a did-you-mean.
    pub fn every_word() -> impl Iterator<Item = &'static str> {
        BEHAVIOUR_MODIFIERS
            .iter()
            .map(|(w, _)| *w)
            .chain(KEY_MODIFIERS.iter().map(|(w, _)| *w))
            .chain(SYSTEM_KEY_MODIFIERS.iter().map(|(w, _)| *w))
    }
}

impl Behaviour {
    pub fn word(self) -> &'static str {
        BEHAVIOUR_MODIFIERS
            .iter()
            .find(|(_, b)| *b == self)
            .map(|(w, _)| *w)
            .unwrap_or("")
    }

    /// The bit a host's listen registration carries for this behaviour — the order of the table
    /// above, one bit each, the same bits `semantics::dom`'s `LISTEN_*` names.
    pub fn listen_bit(self) -> u8 {
        match self {
            Behaviour::Prevent => 1 << 0,
            Behaviour::Stop => 1 << 1,
            Behaviour::Once => 1 << 2,
            Behaviour::Capture => 1 << 3,
            Behaviour::Passive => 1 << 4,
        }
    }
}

impl SystemKey {
    pub fn word(self) -> &'static str {
        SYSTEM_KEY_MODIFIERS
            .iter()
            .find(|(_, k)| *k == self)
            .map(|(w, _)| *w)
            .unwrap_or("")
    }
}

impl KeyName {
    /// The key as `KeyboardEvent.key` spells it.
    pub fn event_key(&self) -> String {
        match self {
            KeyName::Named(k) => k.to_string(),
            KeyName::Char(c) => c.to_string(),
        }
    }
}

/// Why a handler's qualifiers cannot be accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModifierError {
    /// A word the catalogue does not name, with the closest one it does.
    Unknown {
        word: String,
        suggestion: Option<&'static str>,
    },
    /// A key or system-key qualifier on an event that carries no key.
    KeyOnNonKeyEvent { word: String, event: String },
    /// Two different keys on one handler, which waits for one.
    TwoKeys { first: String, second: String },
    /// A system-key qualifier. No tier delivers modifier-key state, so the handler could never
    /// be told the key was held.
    SystemKeyNotDelivered { word: String },
    /// `.prevent` and `.passive` on one handler. A passive listener promises never to cancel the
    /// event, so a browser silently ignores its `.prevent` and runs the default action anyway.
    PreventOnPassive { event: String },
}

impl ModifierError {
    pub fn message(&self) -> String {
        let taught = "The qualifiers are `.prevent`, `.stop`, `.once`, `.capture`, `.passive`, and on a \
                      key event one key: `.enter`, `.escape`, `.space`, `.tab`, `.backspace`, `.delete`, \
                      `.arrowup`/`.arrowdown`/`.arrowleft`/`.arrowright`, `.home`, `.end`, `.pageup`, \
                      `.pagedown`, or a single character (`.a`).";
        match self {
            ModifierError::Unknown { word, suggestion: Some(s) } => format!(
                "`.{word}` is not an event qualifier — did you mean `.{s}`? {taught}"
            ),
            ModifierError::Unknown { word, suggestion: None } => {
                format!("`.{word}` is not an event qualifier. {taught}")
            }
            ModifierError::KeyOnNonKeyEvent { word, event } => format!(
                "`.{word}` names a key, and `{event}` is not a key event, so the handler could \
                 never match. Put key qualifiers on `on-keydown`, `on-keyup` or `on-keypress`: \
                 `on-keydown.{word}={{…}}`."
            ),
            ModifierError::TwoKeys { first, second } => format!(
                "`.{first}` and `.{second}` both name a key, and a handler answers one key. Write \
                 one handler per key: `on-keydown.{first}={{…}}` and `on-keydown.{second}={{…}}`."
            ),
            ModifierError::SystemKeyNotDelivered { word } => format!(
                "`.{word}` waits for a modifier key to be held, and no tier delivers modifier-key \
                 state to a handler yet, so this handler could never be told the key was down. \
                 Handle the key on its own (`on-keydown.enter={{…}}`)."
            ),
            ModifierError::PreventOnPassive { event } => format!(
                "`.prevent` and `.passive` contradict each other on one handler: a passive listener \
                 promises never to cancel the event, so a browser ignores `.prevent` on it and the \
                 default action runs anyway. Did you mean `on-{event}.prevent={{…}}` (cancel the \
                 default action) or `on-{event}.passive={{…}}` (never cancel, so the browser acts \
                 without waiting for the handler)?"
            ),
        }
    }
}

/// Classify every qualifier of one `on-<event>.<…>` directive against the catalogue.
pub fn classify_modifiers(
    event: &str,
    words: &[&str],
) -> Result<Vec<EventModifier>, (usize, ModifierError)> {
    let mut out: Vec<EventModifier> = Vec::with_capacity(words.len());
    for (at, word) in words.iter().enumerate() {
        let Some(m) = EventModifier::classify(word) else {
            let suggestion = crate::suggest::did_you_mean(word, EventModifier::every_word());
            return Err((
                at,
                ModifierError::Unknown {
                    word: word.to_string(),
                    suggestion,
                },
            ));
        };
        match m {
            EventModifier::System(_) => {
                return Err((
                    at,
                    ModifierError::SystemKeyNotDelivered {
                        word: word.to_string(),
                    },
                ))
            }
            EventModifier::Key(_) if !is_key_event(event) => {
                return Err((
                    at,
                    ModifierError::KeyOnNonKeyEvent {
                        word: word.to_string(),
                        event: event.to_string(),
                    },
                ))
            }
            EventModifier::Key(k) => {
                if let Some(EventModifier::Key(prior)) =
                    out.iter().find(|p| matches!(p, EventModifier::Key(_)))
                {
                    if *prior != k {
                        return Err((
                            at,
                            ModifierError::TwoKeys {
                                first: EventModifier::Key(*prior).word(),
                                second: word.to_string(),
                            },
                        ));
                    }
                }
            }
            EventModifier::Behaviour(b @ (Behaviour::Prevent | Behaviour::Passive)) => {
                let other = if b == Behaviour::Prevent {
                    Behaviour::Passive
                } else {
                    Behaviour::Prevent
                };
                if out.contains(&EventModifier::Behaviour(other)) {
                    return Err((
                        at,
                        ModifierError::PreventOnPassive {
                            event: event.to_string(),
                        },
                    ));
                }
            }
            EventModifier::Behaviour(Behaviour::Stop | Behaviour::Once | Behaviour::Capture) => {}
        }
        if !out.contains(&m) {
            out.push(m);
        }
    }
    Ok(out)
}

/// The HTML RAW TEXT elements: their content is not markup, and escaping it CORRUPTS it.
///
/// ★ A `<style>` HOLDING `&gt;` IS A STYLESHEET THE BROWSER THROWS AWAY. HTML's parser reads the
/// content of `<style>` and `<script>` verbatim — character references are NOT decoded there — so
/// a serializer that escapes text the way it escapes everything else turns the child combinator
/// `.list > li` into `.list &gt; li`, which is not a selector. The browser discards that rule and
/// every declaration in it, silently: the page renders, the styling is simply absent, and nothing
/// anywhere says why.
///
/// Same shape as [`is_void`] one function up — a rule HTML keys on the element NAME, which a
/// serializer cannot infer from the content it is holding, held once so both renderers ask it.
pub fn is_raw_text(name: &str) -> bool {
    name.eq_ignore_ascii_case("style") || name.eq_ignore_ascii_case("script")
}

/// The HTML BOOLEAN attributes: their state is their PRESENCE, and the value is ignored.
///
/// `disabled="false"` disables the control in every browser there is — so a page that renders a
/// Bool `false` into one of these as a string ships the opposite of what its author wrote, and
/// nothing about the page looks wrong. Held as a list rather than guessed from the value,
/// because HTML keys the rule on the attribute NAME.
pub const BOOLEAN_ATTRIBUTES: &[&str] = &[
    "allowfullscreen",
    "async",
    "autofocus",
    "autoplay",
    "checked",
    "controls",
    "default",
    "defer",
    "disabled",
    "formnovalidate",
    "hidden",
    "inert",
    "ismap",
    "itemscope",
    "loop",
    "multiple",
    "muted",
    "novalidate",
    "open",
    "playsinline",
    "readonly",
    "required",
    "reversed",
    "selected",
];

pub fn is_boolean_attribute(name: &str) -> bool {
    BOOLEAN_ATTRIBUTES
        .iter()
        .any(|v| v.eq_ignore_ascii_case(name))
}

/// The attributes whose value is a URL the BROWSER will follow, fetch or submit to.
///
/// These are the positions where a string stops being data and becomes a navigation the browser
/// performs on the author's behalf — so they are the positions where a scheme matters. Held as a
/// list for the same reason [`BOOLEAN_ATTRIBUTES`] is: HTML keys the rule on the attribute NAME,
/// and guessing from the value is how `disabled="false"` disabled a control.
/// The characters that mean something to an HTML parser in TEXT content.
///
/// ★ THREE, NOT FOUR — the quote is not one of them. A `"` between tags is an ordinary character;
/// only inside an attribute value, where a quote is what ENDS the value, does it have to go.
pub const TEXT_ESCAPES: &[(char, &str)] = &[('&', "&amp;"), ('<', "&lt;"), ('>', "&gt;")];

/// The same, plus the quote that delimits an attribute value.
///
/// `&` is first in both tables and must stay first: replacing `<` before `&` would re-escape the
/// `&` of the `&lt;` just written.
pub const ATTR_ESCAPES: &[(char, &str)] = &[
    ('&', "&amp;"),
    ('<', "&lt;"),
    ('>', "&gt;"),
    ('"', "&quot;"),
];

/// Escape `s` for HTML TEXT content.
///
/// ★ THE ONE DEFINITION. There were FOUR — `escape_text` and `escape_attr` in `semantics::dom`,
/// `htmlEscape` in `web.lg`, and `html_escape_literal` in the lowering — and they disagreed about
/// the quote, so a text hole holding one **served `&quot;` and painted `"`**. Both look identical
/// in a browser, which is exactly why it survived: the difference is in the bytes, and the bytes
/// are what every cross-tier comparison in this codebase rests on. An escaper is also not a place
/// to hold four opinions; the next disagreement would not be a cosmetic one.
///
/// It lives here beside `VOID_ELEMENTS` and `URL_ATTRIBUTES` for the same reason they do: this is
/// the module every renderer already asks about HTML's own rules.
pub fn escape_text(s: &str) -> String {
    escape_with(s, TEXT_ESCAPES)
}

/// Escape `s` for an HTML ATTRIBUTE value.
pub fn escape_attr(s: &str) -> String {
    escape_with(s, ATTR_ESCAPES)
}

fn escape_with(s: &str, table: &[(char, &str)]) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match table.iter().find(|(k, _)| *k == c) {
            Some((_, replacement)) => out.push_str(replacement),
            None => out.push(c),
        }
    }
    out
}

pub const URL_ATTRIBUTES: &[&str] = &[
    "href",
    "src",
    "action",
    "formaction",
    "poster",
    "data",
    "cite",
    "ping",
    "background",
];

pub fn is_url_attribute(name: &str) -> bool {
    URL_ATTRIBUTES.iter().any(|v| v.eq_ignore_ascii_case(name))
}

/// Schemes a browser EXECUTES rather than fetches. A URL carrying one is script, not a location.
///
/// `data:` is here because a `data:text/html` document runs in the origin that opened it, which is
/// the same capability `javascript:` gives with more steps.
pub const EXECUTABLE_SCHEMES: &[&str] = &["javascript:", "data:", "vbscript:"];

/// Whether `url` would execute if a browser followed it.
///
/// ★ IT NORMALISES THE WAY A BROWSER DOES, WHICH IS THE WHOLE DIFFICULTY. A browser strips leading
/// whitespace and control characters before reading the scheme, and compares it case-insensitively
/// — so `JaVaScRiPt:`, a leading space, and a tab or newline spliced INSIDE the word all reach the
/// same place as the plain spelling. A check that compared the raw text would pass every one of
/// them while looking correct, which is worse than no check at all.
pub fn is_executable_url(url: &str) -> bool {
    let flat: String = url
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    EXECUTABLE_SCHEMES.iter().any(|s| flat.starts_with(s))
}

/// Whether `name` names a component use (`<TodoRow>`) rather than an HTML element: LGX
/// components are Capitalized, HTML tags are lowercase.
pub fn is_component_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

/// Tokenize a raw view body.
pub fn tokenize(src: &str) -> Result<Vec<HtmlToken>, HtmlError> {
    let mut out = Vec::new();
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'<' => {
                if src[i..].starts_with("<!--") {
                    match src[i + 4..].find("-->") {
                        Some(k) => i = i + 4 + k + 3,
                        None => {
                            return Err(HtmlError {
                                kind: HtmlErrorKind::UnterminatedComment,
                                span: Span::new(i, src.len()),
                            })
                        }
                    }
                } else if src[i..].starts_with("<!") {
                    let name: String = src[i + 2..]
                        .chars()
                        .take_while(|c| c.is_ascii_alphabetic())
                        .collect::<String>()
                        .to_ascii_lowercase();
                    let name = if name.is_empty() {
                        "doctype".to_string()
                    } else {
                        name
                    };
                    let span = Span::new(i, (i + 2 + name.len()).min(src.len()));
                    return Err(HtmlError {
                        kind: HtmlErrorKind::ForbiddenElement(name),
                        span,
                    });
                } else if src[i..].starts_with("</") {
                    let (tok, next) = end_tag(src, i)?;
                    out.push(tok);
                    i = next;
                } else {
                    let (tok, next) = start_tag(src, i)?;
                    out.push(tok);
                    i = next;
                }
            }
            b'{' => {
                let end = scan_brace(src, i)?;
                out.push(HtmlToken::Brace {
                    inner: src[i + 1..end - 1].to_string(),
                    span: Span::new(i, end),
                });
                i = end;
            }
            b'}' => {
                return Err(HtmlError {
                    kind: HtmlErrorKind::StrayCloseBrace,
                    span: Span::new(i, i + 1),
                })
            }
            _ => {
                let start = i;
                while i < b.len() && b[i] != b'<' && b[i] != b'{' && b[i] != b'}' {
                    i += 1;
                }
                out.push(HtmlToken::Text {
                    text: decode_entities(&src[start..i]),
                    span: Span::new(start, i),
                });
            }
        }
    }
    Ok(out)
}

/// Scan the `{…}` group opening at byte `open`; returns the index just past its `}`.
/// String-aware (a `"…"` may contain braces; `\"` is an escaped quote) and nesting-aware.
pub fn scan_brace(src: &str, open: usize) -> Result<usize, HtmlError> {
    debug_assert_eq!(src.as_bytes().get(open), Some(&b'{'));
    let b = src.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i >= b.len() {
                    return Err(HtmlError {
                        kind: HtmlErrorKind::UnterminatedBrace,
                        span: Span::new(open, open + 1),
                    });
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err(HtmlError {
        kind: HtmlErrorKind::UnterminatedBrace,
        span: Span::new(open, open + 1),
    })
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic()
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':' || c == '.'
}

fn end_tag(src: &str, at: usize) -> Result<(HtmlToken, usize), HtmlError> {
    // `</` name ws* `>`
    let mut i = at + 2;
    let name_start = i;
    let mut chars = src[i..].char_indices().peekable();
    if !chars.peek().is_some_and(|(_, c)| is_name_start(*c)) {
        return Err(HtmlError {
            kind: HtmlErrorKind::BadTagName,
            span: Span::new(at, (at + 2).min(src.len())),
        });
    }
    for (k, c) in src[i..].char_indices() {
        if is_name_char(c) {
            i = name_start + k + c.len_utf8();
        } else {
            break;
        }
    }
    let name = src[name_start..i].to_string();
    while i < src.len() && src.as_bytes()[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < src.len() && src.as_bytes()[i] == b'>' {
        Ok((
            HtmlToken::EndTag {
                name,
                span: Span::new(at, i + 1),
            },
            i + 1,
        ))
    } else {
        Err(HtmlError {
            kind: HtmlErrorKind::UnterminatedTag,
            span: Span::new(at, src.len().min(i.max(at + 2))),
        })
    }
}

fn start_tag(src: &str, at: usize) -> Result<(HtmlToken, usize), HtmlError> {
    let b = src.as_bytes();
    let mut i = at + 1;
    let name_start = i;
    if !src[i..].chars().next().is_some_and(is_name_start) {
        return Err(HtmlError {
            kind: HtmlErrorKind::BadTagName,
            span: Span::new(at, (at + 1).min(src.len())),
        });
    }
    for (k, c) in src[i..].char_indices() {
        if is_name_char(c) {
            i = name_start + k + c.len_utf8();
        } else {
            break;
        }
    }
    let name = src[name_start..i].to_string();
    let lower = name.to_ascii_lowercase();
    if lower == "script" || lower == "style" {
        return Err(HtmlError {
            kind: HtmlErrorKind::ForbiddenElement(lower),
            span: Span::new(at, i),
        });
    }
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            return Err(HtmlError {
                kind: HtmlErrorKind::UnterminatedTag,
                span: Span::new(at, src.len()),
            });
        }
        if src[i..].starts_with("/>") {
            self_closing = true;
            i += 2;
            break;
        }
        if b[i] == b'>' {
            i += 1;
            break;
        }
        if b[i] == b'/' {
            // A lone `/` inside a tag (`<a//>`): skip it like browsers do.
            i += 1;
            continue;
        }
        // Attribute name.
        let attr_start = i;
        while i < b.len()
            && !b[i].is_ascii_whitespace()
            && b[i] != b'='
            && b[i] != b'>'
            && b[i] != b'/'
            && b[i] != b'<'
            && b[i] != b'{'
            && b[i] != b'}'
            && b[i] != b'"'
            && b[i] != b'\''
        {
            i += 1;
        }
        if i == attr_start {
            let bad: String = src[i..].chars().take(1).collect();
            return Err(HtmlError {
                kind: HtmlErrorKind::BadAttributeName(bad),
                span: Span::new(i, (i + 1).min(src.len())),
            });
        }
        let attr_name = src[attr_start..i].to_string();
        // Optional `= value`.
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let value = if j < b.len() && b[j] == b'=' {
            j += 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= b.len() {
                return Err(HtmlError {
                    kind: HtmlErrorKind::UnterminatedTag,
                    span: Span::new(at, src.len()),
                });
            }
            let (value, next) = attr_value(src, j)?;
            i = next;
            Some(value)
        } else {
            None
        };
        attrs.push(HtmlAttr {
            name: attr_name,
            value,
            span: Span::new(attr_start, i),
        });
    }
    // ★ A COMPONENT IS NOT AN HTML ELEMENT, and the void check was the one place that did not ask.
    // `Link` lowercases to `link`, a real void element, so `<Link href={u}>Home</Link>` was marked
    // self-closing: the children were dropped and the author was then told that `<Link>` is a void
    // element — about a component they declared themselves. `Link` is what every router calls its
    // navigation component; `Input`, `Img`, `Meta`, `Source`, `Track`, `Area`, `Col`, `Embed`,
    // `Base`, `Param`, `Br`, `Hr` and `Wbr` collide the same way.
    //
    // `is_component_name` is the line the rest of the tokenizer already draws — it is how
    // `<TodoRow/>` composes instead of emitting a `<todorow>` tag.
    let void = !is_component_name(&name) && is_void(&name);
    Ok((
        HtmlToken::StartTag {
            name,
            attrs,
            self_closing: self_closing || void,
            span: Span::new(at, i),
        },
        i,
    ))
}

/// Parse an attribute value starting at `at` (just past `=` and whitespace).
fn attr_value(src: &str, at: usize) -> Result<(AttrValue, usize), HtmlError> {
    let b = src.as_bytes();
    match b[at] {
        q @ (b'"' | b'\'') => {
            let mut i = at + 1;
            let mut parts: Vec<AttrPart> = Vec::new();
            let mut text_start = i;
            loop {
                if i >= b.len() {
                    return Err(HtmlError {
                        kind: HtmlErrorKind::UnterminatedAttributeValue,
                        span: Span::new(at, src.len()),
                    });
                }
                if b[i] == q {
                    if i > text_start {
                        parts.push(AttrPart::Text(decode_entities(&src[text_start..i])));
                    }
                    let is_static = parts.iter().all(|p| matches!(p, AttrPart::Text(_)));
                    let value = if is_static {
                        let s: String = parts
                            .into_iter()
                            .map(|p| match p {
                                AttrPart::Text(t) => t,
                                _ => unreachable!(),
                            })
                            .collect();
                        AttrValue::Static(s)
                    } else {
                        AttrValue::Parts(parts)
                    };
                    return Ok((value, i + 1));
                }
                if b[i] == b'{' {
                    if i > text_start {
                        parts.push(AttrPart::Text(decode_entities(&src[text_start..i])));
                    }
                    let end = scan_brace(src, i)?;
                    parts.push(AttrPart::Brace {
                        inner: src[i + 1..end - 1].to_string(),
                        span: Span::new(i, end),
                    });
                    i = end;
                    text_start = i;
                    continue;
                }
                if b[i] == b'\n' && src[at + 1..i].contains('>') {
                    // A quote that swallowed the rest of the tag and the line: report precisely.
                    return Err(HtmlError {
                        kind: HtmlErrorKind::UnterminatedAttributeValue,
                        span: Span::new(at, i),
                    });
                }
                i += 1;
            }
        }
        b'{' => {
            let end = scan_brace(src, at)?;
            Ok((
                AttrValue::Parts(vec![AttrPart::Brace {
                    inner: src[at + 1..end - 1].to_string(),
                    span: Span::new(at, end),
                }]),
                end,
            ))
        }
        _ => {
            let mut i = at;
            // ★ `/>` IS COMPARED AS BYTES: `i` advances byte by byte, so `src[i..]` panicked
            // inside any multi-byte character — `<p class=café>` took the whole compiler down.
            // Both bytes are ASCII, so the byte comparison is exact. See `lgx_non_ascii_lock`.
            while i < b.len()
                && !b[i].is_ascii_whitespace()
                && b[i] != b'>'
                && !b[i..].starts_with(b"/>")
            {
                i += 1;
            }
            Ok((AttrValue::Static(decode_entities(&src[at..i])), i))
        }
    }
}

/// Decode HTML character references. Unknown or malformed references are kept verbatim.
/// The HTML numeric-character-reference replacement table for `0x80..=0x9F`, indexed by
/// `code - 0x80`.
///
/// ★ THE NUMBER IS NOT THE CODE POINT IN THIS RANGE. `&#128;` is `€` (U+20AC), not U+0080, because
/// the range is a legacy of documents that declared windows-1252 and wrote the raw byte — and it is
/// not an exotic corner: `&#147;`/`&#148;` (curly quotes) and `&#151;` (an em dash) are what a word
/// processor pastes, so this arrives in ordinary page copy.
///
/// Handing the number straight to `char::from_u32` SUCCEEDS here and yields an invisible C1
/// control, so the page rendered a character nobody typed and nothing anywhere reported it. The
/// five slots windows-1252 leaves unassigned are `U+FFFD` rather than the control.
const C1_REPLACEMENTS: [char; 32] = [
    '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}', '\u{017D}', '\u{FFFD}',
    '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
];

/// The value of a numeric reference's digits, or `None` when they are not digits at all.
///
/// ★ THE TWO FAILURES ARE NOT THE SAME FAILURE, and collapsing them is how `&#99999999999999;`
/// came to render as its own markup. Digits that are not digits (`&#xZZ;`) mean the author never
/// wrote a reference, so the text stands as typed; digits that OVERFLOW mean they wrote one that
/// denotes nothing, which is `U+FFFD`. Saturating rather than failing keeps that distinction, since
/// every value past the last code point already answers `U+FFFD`.
fn numeric_digits(digits: &str, radix: u32) -> Option<u32> {
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    Some(u32::from_str_radix(digits, radix).unwrap_or(u32::MAX))
}

/// The character a NUMERIC reference denotes, by the HTML standard's rules rather than by
/// `char::from_u32` alone.
///
/// Every failure here answers `U+FFFD` — a zero, a surrogate, a value past the last code point —
/// because the author DID write a reference and the standard says what a malformed one renders as.
/// Answering `None` and falling through to "not an entity" showed the reader the markup itself.
fn numeric_reference(code: u32) -> char {
    match code {
        0 => '\u{FFFD}',
        0x80..=0x9F => C1_REPLACEMENTS[(code - 0x80) as usize],
        0xD800..=0xDFFF => '\u{FFFD}',
        c if c > 0x10FFFF => '\u{FFFD}',
        c => char::from_u32(c).unwrap_or('\u{FFFD}'),
    }
}

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        match after.find(';') {
            Some(semi) if semi <= super::entities::LONGEST_NAME + 2 => {
                let name = &after[..semi];
                // A NAMED reference resolves against the standard's own list; a NUMERIC one is
                // computed. Both answer text, because 93 of the named ones stand for more than a
                // single code point.
                let decoded: Option<String> = if let Some(hex) =
                    name.strip_prefix("#x").or_else(|| name.strip_prefix("#X"))
                {
                    numeric_digits(hex, 16).map(|c| numeric_reference(c).to_string())
                } else if let Some(dec) = name.strip_prefix('#') {
                    numeric_digits(dec, 10).map(|c| numeric_reference(c).to_string())
                } else {
                    super::entities::named(name).map(str::to_string)
                };
                match decoded {
                    Some(text) => {
                        out.push_str(&text);
                        rest = &after[semi + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = after;
                    }
                }
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}
