//! ★ LOCK — `docs/lgx-reference.md` is GENERATED, and the committed file is the generated text. ★
//!
//! The reference is a pure function of the compiler's own catalogues. Every row is produced by
//! PARSING its example with the real parser and reading the answer off the result:
//!
//! - template tags and directives: `lgx::template::parse` names the `TNode` / `TAttr` each example
//!   becomes, and the whole example parses inside a component's `View:`;
//! - event modifiers: the tables of `lgx::html` (`BEHAVIOUR_MODIFIERS`, `KEY_MODIFIERS`,
//!   `SYSTEM_KEY_MODIFIERS`, `KEY_EVENTS`), `classify_modifiers` for each row's verdict, and
//!   `compile_to_wasm` for whether a page carrying it builds for the browser;
//! - triggers: each sentence parses into a handler, and `Trigger::form()` is read off the parsed
//!   trigger; the selector words are every row of `SELECTOR_EVENTS`, each parsed to its DOM event;
//! - UI verbs: each sentence parses into a `UiStmt` in the position the row names, and
//!   `UiStmt::role()` is read off it;
//! - page queries: each parses into a `DomQuery` inside a `## Test`, and `DomQuery::noun()` is
//!   read off it;
//! - loop bindings: a parsed `{for each}` and `ast::view::LOOP_INDEX_BASE`;
//! - filters: every public `## To` of `ui.lg` through `teach::extract_literate_docs`, each filter
//!   form parsed back by the template parser.
//!
//! The descriptive column of each table is an exhaustive `match` over the parsed value, so a new
//! variant does not compile here until it is described; `every_catalogue_member_has_a_row` reads
//! the enums and tables from SOURCE and fails on a member with no row.
//!
//! Regenerate: `./scripts/release/lgx/gen-reference.sh`.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_compile::loader::prelude_module_sources;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_language::ast::component::Trigger;
use logicaffeine_language::ast::stmt::{DomQuery, Expr, Role, Stmt, TypeExpr, UiStmt};
use logicaffeine_language::ast::view::LOOP_INDEX_BASE;
use logicaffeine_language::lgx::html::{
    classify_modifiers, Behaviour, ModifierError, BEHAVIOUR_MODIFIERS, KEY_EVENTS, KEY_MODIFIERS,
    SYSTEM_KEY_MODIFIERS,
};
use logicaffeine_language::lgx::template::{self, TAttr, TNode, TemplateErrorKind, TAG_ALIASES};
use logicaffeine_language::teach::extract_literate_docs;
use logicaffeine_language::Interner;
use std::fmt::Write as _;
use std::path::PathBuf;

const REFERENCE: &str = "docs/lgx-reference.md";
const REGENERATE: &str = "./scripts/release/lgx/gen-reference.sh";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

// ── rows ────────────────────────────────────────────────────────────────────────────────────────

/// `(template, the TNode variant it parses to)`. The empty variant is a template that parses to no
/// node at all.
const TEMPLATE_ROWS: &[(&str, &str)] = &[
    ("<p class=\"note\">Saved</p>", "Element"),
    ("Saved", "Text"),
    ("{count}", "Hole"),
    ("{title | truncate 20}", "Hole"),
    ("{raw trustedHtml}", "Hole"),
    ("{if count is 0:}<p>none</p>{else if count is 1:}<p>one</p>{else:}<p>many</p>{end}", "If"),
    (
        "{for each todo in todos, keyed by todo's id, at index i:}<li>{i}. {todo's title}</li>{else:}<li>nothing yet</li>{end}",
        "For",
    ),
    ("{inspect found:}{when Some (name):}<p>{name}</p>{when None:}<p>not found</p>{end}", "Inspect"),
    ("{let left be length of todos}", "Let"),
    ("{extends Site}", "Extends"),
    ("{block main:}<p>nothing here yet</p>{end}", "Block"),
    ("{block main}", "Block"),
    ("{children}", "Children"),
    ("<Badge label={title}/>", "Component"),
    ("{# a note for the reader #}", ""),
];

/// `(element, the TAttr variant of its first attribute)`.
const DIRECTIVE_ROWS: &[(&str, &str)] = &[
    ("<button on-click={add}>Add</button>", "On"),
    ("<button on-click={remove(todo's id)}>Remove</button>", "On"),
    ("<form on-submit.prevent={add}></form>", "On"),
    ("<input on-keydown.enter={add}>", "On"),
    ("<input bind={draft}>", "Bind"),
    ("<li class:done={todo's done}>{todo's title}</li>", "Class"),
    ("<a href=\"/todos/{todo's id}\">open</a>", "Dynamic"),
    ("<img src={avatarUrl} alt=\"\">", "Dynamic"),
    ("<input type=\"text\">", "Static"),
    ("<input disabled>", "Static"),
];

/// `(sentence, the Trigger variant it parses to)`.
const TRIGGER_ROWS: &[(&str, &str)] = &[
    ("When the page loads:", "PageLoads"),
    ("When this component appears:", "Appears"),
    ("When this component disappears:", "Disappears"),
    ("When \"#save\" is clicked:", "Selector"),
    ("When the user types in \"#draft\":", "Selector"),
    ("When the user presses Enter in \"#draft\":", "Selector"),
    ("When the user presses Escape:", "Key"),
    ("When n changes:", "FieldChanges"),
    ("When Session's name changes:", "FieldChanges"),
    ("Every 60 seconds:", "Every"),
    ("Every 60 seconds as tick:", "Every"),
    ("After 5 seconds:", "After"),
    ("After 5 seconds as reminder:", "After"),
    (
        "When a message arrives on \"news\" (body: Text):",
        "Message",
    ),
    ("When an action fails (why: Text):", "ActionFails"),
    (
        "When a fetch fails (url: Text) and (why: Text):",
        "FetchFails",
    ),
    ("When the route changes:", "RouteChanges"),
    ("When the app goes offline:", "Offline"),
    ("When the app comes back online:", "Online"),
    ("When the window resizes:", "WindowResizes"),
    ("When the user scrolls to the bottom:", "ScrollsToBottom"),
    (
        "When the user scrolls to the bottom of \"#list\":",
        "ScrollsToBottom",
    ),
    (
        "When a file is dropped on \"#zone\" (file: File):",
        "FileDropped",
    ),
    (
        "When a file is dropped on \"#zone\" (name: Text):",
        "FileDropped",
    ),
];

/// Where a verb row is written: inside a component's `Actions:` or in a `## Test` body.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Position {
    Actions,
    Test,
}

/// `(sentence, the UiStmt variant it parses to, the position it is parsed in)`.
const VERB_ROWS: &[(&str, &str, Position)] = &[
    ("Focus \"#draft\".", "Focus", Position::Actions),
    ("Blur \"#draft\".", "Blur", Position::Actions),
    ("Hover \"#draft\".", "Hover", Position::Actions),
    ("Navigate to \"/todos\".", "Navigate", Position::Actions),
    ("Reload the page.", "Reload", Position::Actions),
    ("Scroll to \"#draft\".", "ScrollTo", Position::Actions),
    (
        "Add class \"shake\" to \"#draft\".",
        "AddClass",
        Position::Actions,
    ),
    (
        "Remove class \"shake\" from \"#draft\".",
        "RemoveClass",
        Position::Actions,
    ),
    (
        "Select the text in \"#draft\".",
        "SelectText",
        Position::Actions,
    ),
    (
        "Copy draft to the clipboard.",
        "CopyToClipboard",
        Position::Actions,
    ),
    (
        "Download draft as \"notes.txt\".",
        "Download",
        Position::Actions,
    ),
    (
        "Replace the URL with \"/todos?page=2\".",
        "ReplaceUrl",
        Position::Actions,
    ),
    ("Go back.", "GoBack", Position::Actions),
    ("Set the title to \"Todos\".", "SetTitle", Position::Actions),
    ("Save draft as \"draft\".", "Save", Position::Actions),
    ("Load \"draft\" into draft.", "Load", Position::Actions),
    ("Forget \"draft\".", "Forget", Position::Actions),
    ("Notify \"Saved.\".", "Notify", Position::Actions),
    ("Notify \"Saved.\" as success.", "Notify", Position::Actions),
    ("Publish draft on \"chat\".", "Publish", Position::Actions),
    ("Toggle done.", "Toggle", Position::Actions),
    (
        "Call Session's login with \"ada\".",
        "CallStore",
        Position::Actions,
    ),
    ("Stop the timer refresh.", "StopTimer", Position::Actions),
    ("Start the timer refresh.", "StartTimer", Position::Actions),
    ("Click \"#save\".", "Click", Position::Test),
    ("Submit \"#form\".", "Submit", Position::Test),
    ("Choose \"red\" in \"#colour\".", "Choose", Position::Test),
    ("Type \"milk\" into \"#draft\".", "TypeInto", Position::Test),
    ("Press \"Enter\" in \"#draft\".", "Press", Position::Test),
    ("Press \"Escape\".", "Press", Position::Test),
    ("Raise \"offline\".", "RaiseWindowEvent", Position::Test),
    ("Mount Badge.", "Mount", Position::Test),
];

/// `(query, what follows it in an `Expect that …` sentence, the DomQuery variant)`.
const QUERY_ROWS: &[(&str, &str, &str)] = &[
    ("the text of \"#n\"", " is equal to \"2\"", "Text"),
    ("the count of \".row\"", " is equal to 3", "Count"),
    ("the value of \"#draft\"", " is equal to \"milk\"", "Value"),
    ("\"#n\" is visible", "", "Visible"),
    ("\"#n\" is hidden", "", "Hidden"),
    ("\"#save\" is disabled", "", "Disabled"),
    ("\"#done\" is checked", "", "Checked"),
    ("\"#draft\" is focused", "", "Focused"),
    ("the journal", " contains \"increment\"", "Journal"),
    (
        "the journal shows increment then decrement",
        "",
        "JournalShows",
    ),
    ("the title", " is equal to \"Todos\"", "Title"),
    ("the URL", " is equal to \"/todos\"", "Url"),
];

/// `(event, qualifiers, the ModifierError variant classify_modifiers answers)`.
const MODIFIER_RULE_ROWS: &[(&str, &[&str], &str)] = &[
    ("click", &["stpo"], "Unknown"),
    ("click", &["enter"], "KeyOnNonKeyEvent"),
    ("keydown", &["enter", "escape"], "TwoKeys"),
    ("keydown", &["ctrl"], "SystemKeyNotDelivered"),
    ("click", &["prevent", "passive"], "PreventOnPassive"),
];

// ── parsing ─────────────────────────────────────────────────────────────────────────────────────

/// The name of an enum value's variant, as its `Debug` rendering starts.
fn variant_name(debug: &str) -> String {
    debug
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// The probe program every sentence row is parsed in: a store to call and watch, a component to
/// mount, a layout to extend, and one component whose `Actions:` and `## Test` carry the row.
fn probe(trigger: Option<&str>, verb: &str, test_body: Option<&str>) -> String {
    let mut src = String::from(
        "## A Todo has:\n\
        \x20   an id, which is Int.\n\
        \x20   a title, which is Text.\n\
        \x20   a done, which is Bool.\n\n\
         ## Store Session\n\
         State:\n\
        \x20   Let name be \"guest\".\n\
         Actions:\n\
        \x20   To login (who: Text):\n\
        \x20       Set name to who.\n\n\
         ## Component Badge\n\
         View:\n\
        \x20   <span>badge</span>\n\n\
         ## Component App\n\
         Use Session.\n\
         State:\n\
        \x20   Let n be 0.\n\
        \x20   Let draft be \"\".\n\
        \x20   Let done be false.\n\
         Actions:\n\
        \x20   To increment:\n\
        \x20       Set n to n + 1.\n\
        \x20   To decrement:\n\
        \x20       Set n to n - 1.\n\
        \x20   To go:\n",
    );
    let _ = writeln!(src, "        {verb}");
    src.push_str("    Every 60 seconds as refresh:\n        Set draft to draft.\n");
    if let Some(t) = trigger {
        let _ = writeln!(src, "    {t}\n        Set n to n + 1.");
    }
    src.push_str(
        "View:\n\
        \x20   <div><p id=\"n\">{n}</p><input id=\"draft\" type=\"text\"><button id=\"save\" on-click={go}>go</button></div>\n",
    );
    if let Some(body) = test_body {
        let _ = write!(src, "\n## Test: probe\n\n{body}\n");
    }
    src
}

/// Parse `src` and hand `f` its statements, or fail naming the program that did not parse.
fn parsed<R>(src: &str, f: impl for<'a> FnOnce(&'a [Stmt<'a>], &'a Interner) -> R) -> R {
    with_parsed_program(src, |p, interner| match p {
        Ok((stmts, _, _)) => f(stmts, interner),
        Err(e) => panic!("a reference probe does not parse:\n{e}\n\n{src}"),
    })
}

fn component<'s, 'a>(
    stmts: &'s [Stmt<'a>],
    interner: &Interner,
    name: &str,
) -> &'s logicaffeine_language::ast::component::ComponentDef<'a> {
    stmts
        .iter()
        .find_map(|s| match s {
            Stmt::ComponentDef(d) if interner.resolve(d.name) == name => Some(&**d),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the probe declares `## Component {name}`"))
}

fn test_body<'s, 'a>(stmts: &'s [Stmt<'a>]) -> &'s [Stmt<'a>] {
    stmts
        .iter()
        .find_map(|s| match s {
            Stmt::TestDef { body, .. } => Some(*body),
            _ => None,
        })
        .expect("the probe declares `## Test: probe`")
}

/// The first statement of a body that is a sentence, past the parser's `Stmt::Site` markers.
fn first_sentence<'s, 'a>(body: &'s [Stmt<'a>]) -> Option<&'s Stmt<'a>> {
    body.iter().find(|s| !matches!(s, Stmt::Site(_)))
}

fn type_name(ty: &TypeExpr<'_>, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(s) | TypeExpr::Named(s) => interner.resolve(*s).to_string(),
        other => variant_name(&format!("{other:?}")),
    }
}

fn find_query<'s, 'a>(e: &'s Expr<'a>) -> Option<&'s DomQuery<'a>> {
    match e {
        Expr::Dom(q) => Some(q),
        Expr::BinaryOp { left, right, .. } => find_query(left).or_else(|| find_query(right)),
        Expr::Not { operand } => find_query(operand),
        Expr::Contains { collection, value } => {
            find_query(collection).or_else(|| find_query(value))
        }
        _ => None,
    }
}

// ── descriptions: exhaustive over each catalogue ────────────────────────────────────────────────

fn tnode_meaning(node: &TNode) -> String {
    match node {
        TNode::Element { .. } => {
            "An HTML element. It closes with its end tag; a void element (`input`, `br`, `img` …) takes none.".into()
        }
        TNode::Text { .. } => {
            "Text. A run that is only whitespace and holds a newline is formatting and is dropped; other runs collapse to one space.".into()
        }
        TNode::Hole { raw: true, .. } => "The value inserted as markup, unescaped — only for text already made safe.".into(),
        TNode::Hole { expr, filters, .. } if !filters.is_empty() => {
            let mut call = expr.text.trim().to_string();
            for f in filters {
                let mut args = vec![call];
                args.extend(f.args.iter().map(|a| a.text.trim().to_string()));
                call = format!("{}({})", f.name, args.join(", "));
            }
            format!("The value through a filter: this hole is `{call}`. Filters apply left to right.")
        }
        TNode::Hole { .. } => "The value, escaped.".into(),
        TNode::If { .. } => "The first arm whose condition holds; `{else:}` when none does.".into(),
        TNode::For { .. } => {
            "One body per item; the `{else:}` arm renders when the collection is empty. See Loop bindings.".into()
        }
        TNode::Inspect { .. } => "The arm whose pattern matches the value.".into(),
        TNode::Let { .. } => "A name bound for the rest of the enclosing scope.".into(),
        TNode::Extends { .. } => "This view fills the named layout's blocks. It is the first thing in the view, or absent.".into(),
        TNode::Block { default: Some(_), .. } => "A layout slot, with the content shown when the page fills nothing.".into(),
        TNode::Block { default: None, .. } => "A layout slot a page fills.".into(),
        TNode::Children { .. } => "Where the caller's markup lands inside a component.".into(),
        TNode::Component { .. } => "A placed component. A capitalised tag is a component; its attributes are its declared props.".into(),
    }
}

fn tattr_meaning(attr: &TAttr) -> String {
    match attr {
        TAttr::On {
            event,
            modifiers,
            expr,
            ..
        } => {
            let call = expr.text.trim();
            let what = if call.contains('(') {
                format!("calls `{call}`")
            } else {
                format!("runs the action `{call}`")
            };
            if modifiers.is_empty() {
                format!("On `{event}`, {what}.")
            } else {
                let quals: Vec<String> = modifiers
                    .iter()
                    .map(|m| format!("`.{}`", m.word()))
                    .collect();
                format!(
                    "On `{event}`, {what}, qualified by {} (see Event modifiers).",
                    quals.join(" ")
                )
            }
        }
        TAttr::Bind { field, .. } => {
            format!("Two-way binding between the control and the state field `{field}`.")
        }
        TAttr::Class { name, .. } => {
            format!("The class `{name}` is present while the condition holds.")
        }
        TAttr::Dynamic { name, .. } => format!("A computed `{name}`, escaped."),
        TAttr::Static {
            value: Some(_),
            name,
            ..
        } => format!("A literal `{name}`."),
        TAttr::Static {
            value: None, name, ..
        } => {
            format!("A boolean attribute: `{name}` is on while it is present.")
        }
    }
}

fn behaviour_meaning(b: Behaviour) -> &'static str {
    match b {
        Behaviour::Prevent => {
            "The browser's default action (a form posting, a link following) does not run."
        }
        Behaviour::Stop => "The event does not propagate to the element's ancestors.",
        Behaviour::Once => "The listener answers the first event and is then removed.",
        Behaviour::Capture => {
            "The listener runs in the capture phase, before the target's own listeners."
        }
        Behaviour::Passive => "The listener promises not to cancel the default action.",
    }
}

fn modifier_rule(e: &ModifierError) -> String {
    match e {
        ModifierError::Unknown { word, suggestion: Some(s) } => {
            format!("A word the catalogue does not know is refused, naming the nearest one: `.{word}` → `.{s}`.")
        }
        ModifierError::Unknown { word, suggestion: None } => {
            format!("A word the catalogue does not know is refused: `.{word}`.")
        }
        ModifierError::KeyOnNonKeyEvent { word, event } => format!(
            "A key qualifier on an event that carries no key is refused: `.{word}` on `{event}`. Keys belong on {}.",
            KEY_EVENTS.iter().map(|e| format!("`on-{e}`")).collect::<Vec<_>>().join(", ")
        ),
        ModifierError::TwoKeys { first, second } => format!(
            "One handler answers one key: `.{first}` with `.{second}` is refused; write a handler per key."
        ),
        ModifierError::SystemKeyNotDelivered { word } => format!(
            "A system-key qualifier (`.{word}`) is refused: no tier delivers modifier-key state to a handler."
        ),
        ModifierError::PreventOnPassive { .. } => {
            "`.prevent` and `.passive` on one handler are refused: a passive listener ignores \
             `.prevent`, so a browser would run the default action anyway. Write one or the other."
                .to_string()
        }
    }
}

fn trigger_runs(t: &Trigger<'_>, interner: &Interner) -> String {
    match t {
        Trigger::PageLoads => "Once, when the page mounts, before its first paint.".into(),
        Trigger::Appears => "When this component enters the page.".into(),
        Trigger::Disappears => "When this component leaves the page.".into(),
        Trigger::RouteChanges => "After every navigation.".into(),
        Trigger::Online => "When the network comes back.".into(),
        Trigger::Offline => "When the network goes.".into(),
        Trigger::Selector { event, key: Some(k), .. } => format!(
            "On `{}` at that element, for the `{}` key.",
            interner.resolve(*event),
            interner.resolve(*k)
        ),
        Trigger::Selector { event, key: None, .. } => {
            format!("On `{}` at that element.", interner.resolve(*event))
        }
        Trigger::Key { key } => format!("On the `{}` key, anywhere on the page.", interner.resolve(*key)),
        Trigger::FieldChanges { store: None, field } => {
            format!("After a turn changes this component's `{}`.", interner.resolve(*field))
        }
        Trigger::FieldChanges { store: Some(s), field } => format!(
            "After a turn changes the store's field `{}'s {}`, in every component that uses the store.",
            interner.resolve(*s),
            interner.resolve(*field)
        ),
        Trigger::Every { name: None, .. } => "Repeatedly, once per period.".into(),
        Trigger::Every { name: Some(n), .. } => format!(
            "Repeatedly, once per period; named, so `Stop the timer {}.` cancels it.",
            interner.resolve(*n)
        ),
        Trigger::After { name: None, .. } => "Once, after the delay.".into(),
        Trigger::After { name: Some(n), .. } => format!(
            "Once, after the delay; named, so `Stop the timer {}.` cancels it before it fires.",
            interner.resolve(*n)
        ),
        Trigger::Message { param, .. } => format!(
            "On each message published on the topic; `{}` binds its payload as `{}`.",
            interner.resolve(param.0),
            type_name(param.1, interner)
        ),
        Trigger::ActionFails { param } => format!(
            "After a turn fails and is rolled back; `{}` binds the failure message.",
            interner.resolve(*param)
        ),
        Trigger::FetchFails { url, error } => format!(
            "After any request fails; `{}` binds its URL and `{}` the reason.",
            interner.resolve(*url),
            interner.resolve(*error)
        ),
        Trigger::WindowResizes => "When the window is resized.".into(),
        Trigger::ScrollsToBottom { selector: None } => "When the page is scrolled to its bottom.".into(),
        Trigger::ScrollsToBottom { selector: Some(_) } => "When that element is scrolled to its bottom.".into(),
        Trigger::FileDropped { param, .. } => {
            let ty = type_name(param.1, interner);
            let binds = if ty == "File" { "the whole `File` record" } else { "the file's name" };
            format!("When a file is dropped on that element; `{}: {ty}` binds {binds}.", interner.resolve(param.0))
        }
    }
}

fn verb_does(u: &UiStmt<'_>) -> &'static str {
    match u {
        UiStmt::Focus { .. } => "Puts focus on the element and raises `focus`.",
        UiStmt::Blur { .. } => "Takes focus off the element and raises `blur`.",
        UiStmt::Hover { .. } => "Raises `mouseover` on the element.",
        UiStmt::ScrollTo { .. } => "Scrolls the element into view.",
        UiStmt::AddClass { .. } => "Adds one class word to the element.",
        UiStmt::RemoveClass { .. } => "Removes one class word from the element.",
        UiStmt::SelectText { .. } => "Selects the text in the element.",
        UiStmt::CopyToClipboard { .. } => "Writes the text to the clipboard.",
        UiStmt::Download { .. } => "Hands the visitor the bytes as a file with that name.",
        UiStmt::Navigate { .. } => "Adds a history entry and routes to it.",
        UiStmt::ReplaceUrl { .. } => "Replaces the current history entry; no new entry.",
        UiStmt::GoBack => "Goes one history entry back.",
        UiStmt::SetTitle { .. } => "Sets the document title.",
        UiStmt::Save { .. } => "Writes the value to browser storage under the key.",
        UiStmt::Load { .. } => "Reads the key from browser storage into a state field.",
        UiStmt::Forget { .. } => "Removes the key from browser storage.",
        UiStmt::Reload => "Tears down every instance, listener and mounted root; browser storage stays.",
        UiStmt::Notify { tone: None, .. } => "Shows a notification.",
        UiStmt::Notify { tone: Some(_), .. } => "Shows a notification in a tone: `success` or `error`.",
        UiStmt::Publish { .. } => "Sends the message on a relay topic; this page's own subscribers hear it once.",
        UiStmt::Toggle { .. } => "Flips a `Bool` state field.",
        UiStmt::CallStore { .. } => "Runs a store's action, with arguments after `with`.",
        UiStmt::StopTimer { .. } => "Cancels a named timer.",
        UiStmt::StartTimer { .. } => "Arms a named timer again from now, reading its period afresh.",
        UiStmt::RaiseWindowEvent { .. } => "Causes an event that happens to the page, not to an element (`offline`, `online`, `resize`).",
        UiStmt::Click { .. } => "Clicks the element.",
        UiStmt::Submit { .. } => "Submits the form, as its submit button would.",
        UiStmt::Choose { .. } => "Picks the option with that value in a `<select>`.",
        UiStmt::Press { selector: Some(_), .. } => "Presses the key at the element.",
        UiStmt::Press { selector: None, .. } => "Presses the key on the page.",
        UiStmt::TypeInto { .. } => "Types into the input, as a visitor would.",
        UiStmt::Mount { .. } => "Creates the component's root instance and renders it.",
    }
}

fn role_where(r: Role) -> &'static str {
    match r {
        Role::Drive => "`## Test`",
        Role::Action => "`Actions:`",
        Role::Both => "`Actions:` and `## Test`",
    }
}

fn query_asks(q: &DomQuery<'_>) -> &'static str {
    match q {
        DomQuery::Text { .. } => "The element's text content, its descendants' included.",
        DomQuery::Count { .. } => "How many elements match.",
        DomQuery::Value { .. } => "An input's current value.",
        DomQuery::Visible { .. } => "Whether anything matches.",
        DomQuery::Hidden { .. } => "Whether nothing matches — the complement of `visible`.",
        DomQuery::Disabled { .. } => "Whether the match carries `disabled`.",
        DomQuery::Checked { .. } => "A checkbox's or radio's live checked state.",
        DomQuery::Focused { .. } => "Whether focus is on the match.",
        DomQuery::Journal => "The actions that ran, in order, as a `Seq of Text`.",
        DomQuery::JournalShows { .. } => {
            "Whether the first action ran and the second ran after it, not necessarily next."
        }
        DomQuery::Title => "The document title.",
        DomQuery::Url => "The current path.",
    }
}

// ── the document ────────────────────────────────────────────────────────────────────────────────

/// A table cell: a `|` inside it would end the cell, so it is escaped.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn code(text: &str) -> String {
    cell(&format!("`{text}`"))
}

fn table(out: &mut String, header: &[&str], rows: &[Vec<String>]) {
    let _ = writeln!(out, "| {} |", header.join(" | "));
    let _ = writeln!(
        out,
        "|{}",
        header.iter().map(|_| "---|").collect::<String>()
    );
    for r in rows {
        let _ = writeln!(out, "| {} |", r.join(" | "));
    }
    out.push('\n');
}

/// The template parse of one example, or a failure naming it.
fn template_of(example: &str) -> Vec<TNode> {
    template::parse(example).unwrap_or_else(|e| {
        panic!(
            "reference template `{example}` does not parse: {}",
            e.message()
        )
    })
}

/// Every template example must also parse as a component's `View:`, holes and patterns included.
/// A view opens with markup, so the example sits inside an element — except `{extends}`, which
/// must come first.
fn assert_view_parses(example: &str) {
    let body = if example.starts_with("{extends") {
        example.to_string()
    } else {
        format!("<div>{example}</div>")
    };
    let src = format!(
        "## A Todo has:\n\x20   an id, which is Int.\n\x20   a title, which is Text.\n\x20   a done, which is Bool.\n\n\
         ## Component Site\nView:\n\x20   <main>{{block main}}</main>\n\n\
         ## Component Badge (label: Text)\nView:\n\x20   <span>{{label}}</span>\n\n\
         ## Component App\n\
         State:\n\
        \x20   Let count be 0.\n\
        \x20   Let title be \"Todos\".\n\
        \x20   Let draft be \"\".\n\
        \x20   Let avatarUrl be \"/a.png\".\n\
        \x20   Let trustedHtml be \"<b>hi</b>\".\n\
        \x20   Let todos be a new Seq of Todo.\n\
        \x20   Let found be some \"milk\".\n\
        \x20   Let todo be a new Todo with id 1 and title \"milk\" and done false.\n\
         Actions:\n\
        \x20   To add:\n\
        \x20       Set count to count + 1.\n\
        \x20   To remove (id: Int):\n\
        \x20       Set count to count - id.\n\
         View:\n\x20   {body}\n"
    );
    with_parsed_program(&src, |p, _| {
        if let Err(e) = p {
            panic!("reference template `{example}` does not parse inside a `View:`:\n{e}\n\n{src}");
        }
    });
}

fn template_section(out: &mut String) {
    out.push_str("## Template tags\n\nWhat a `View:` holds, one row per construct.\n\n");
    let mut rows = Vec::new();
    for (example, variant) in TEMPLATE_ROWS {
        let nodes = template_of(example);
        assert_view_parses(example);
        let meaning = match nodes.first() {
            None => {
                assert!(
                    variant.is_empty(),
                    "`{example}` parses to no node, and its row names `{variant}`"
                );
                "A comment; dropped.".to_string()
            }
            Some(node) => {
                let got = variant_name(&format!("{node:?}"));
                assert_eq!(
                    &got, variant,
                    "`{example}` parses to `TNode::{got}`, and its row names `{variant}`"
                );
                assert_eq!(nodes.len(), 1, "`{example}` is one construct");
                tnode_meaning(node)
            }
        };
        rows.push(vec![code(example), cell(&meaning)]);
    }
    table(out, &["Template", "Meaning"], &rows);

    out.push_str(
        "Spellings from other template languages are refused with the LGX spelling, rather than read as a hole naming a variable:\n\n",
    );
    let mut rows = Vec::new();
    for (alias, canonical) in TAG_ALIASES {
        let probe = format!("{{{alias}}}");
        match template::parse(&probe).map_err(|e| e.kind) {
            Err(TemplateErrorKind::NonCanonicalTag { .. }) => {}
            other => panic!("`{probe}` must be refused as a non-canonical tag, got {other:?}"),
        }
        rows.push(vec![code(alias), code(canonical)]);
    }
    table(out, &["Written", "LGX spelling"], &rows);
}

fn directive_section(out: &mut String) {
    out.push_str("## Directives on elements\n\n");
    let mut rows = Vec::new();
    for (example, variant) in DIRECTIVE_ROWS {
        let nodes = template_of(example);
        assert_view_parses(example);
        let Some(TNode::Element { attrs, .. }) = nodes.first() else {
            panic!("`{example}` is one element");
        };
        let attr = attrs
            .first()
            .unwrap_or_else(|| panic!("`{example}` carries an attribute"));
        let got = variant_name(&format!("{attr:?}"));
        assert_eq!(
            &got, variant,
            "`{example}`'s first attribute is `TAttr::{got}`, and its row names `{variant}`"
        );
        rows.push(vec![code(example), cell(&tattr_meaning(attr))]);
    }
    table(out, &["Element", "Meaning"], &rows);
}

/// A page with one input per `(event, qualifier)` handler, compiled for the browser.
fn browser_build(handlers: &[(&str, &str)]) -> Result<(), String> {
    let mut page = String::from(
        "## Page /\n\nState:\n\x20   Let n be 0.\n\nActions:\n\x20   To add:\n\x20       Set n to n + 1.\n\nView:\n\x20   <main><p id=\"n\">{n}</p>",
    );
    for (k, (event, word)) in handlers.iter().enumerate() {
        let _ = write!(
            page,
            "<input id=\"d{k}\" aria-label=\"d{k}\" on-{event}.{word}={{add}}>"
        );
    }
    page.push_str("</main>\n");
    compile_to_wasm(&page)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

/// The Browser build cell of one qualifier: `yes`, or `refused by name` when the browser build
/// refuses the page with a message quoting the qualifier. Any other failure is a broken probe.
fn browser_build_cell(event: &str, word: &str) -> &'static str {
    match browser_build(&[(event, word)]) {
        Ok(()) => "yes",
        Err(e) if e.contains(&format!("`.{word}`")) => "refused by name",
        Err(e) => {
            panic!("`on-{event}.{word}` fails to build for a reason that does not name it: {e}")
        }
    }
}

fn modifier_section(out: &mut String) {
    out.push_str(
        "## Event modifiers\n\n\
         An `on-<event>` directive takes qualifiers after dots. Every tier reads them through one \
         catalogue; a word it does not know is a compile error.\n\n",
    );
    out.push_str("Behaviour — how the event is treated, on any event:\n\n");
    let mut rows = Vec::new();
    for (word, b) in BEHAVIOUR_MODIFIERS {
        assert!(
            classify_modifiers("click", &[word]).is_ok(),
            "`.{word}` is accepted on `click`"
        );
        rows.push(vec![
            code(&format!(".{word}")),
            cell(behaviour_meaning(*b)),
            browser_build_cell("click", word).to_string(),
        ]);
    }
    table(out, &["Qualifier", "Meaning", "Browser build"], &rows);

    let key_events: Vec<String> = KEY_EVENTS.iter().map(|e| format!("`on-{e}`")).collect();
    let _ = writeln!(
        out,
        "Keys — one per handler, on {} only. The handler answers when the browser's \
         `KeyboardEvent.key` is the value shown, compared without case.\n",
        key_events.join(", ")
    );
    let key_event = KEY_EVENTS[0];
    let mut key_words: Vec<&str> = KEY_MODIFIERS.iter().map(|(w, _)| *w).collect();
    key_words.push("a");
    let all_build = browser_build(
        &key_words
            .iter()
            .map(|w| (key_event, *w))
            .collect::<Vec<_>>(),
    )
    .is_ok();
    let key_cell = |word: &str| {
        if all_build {
            "yes"
        } else {
            browser_build_cell(key_event, word)
        }
    };
    let mut rows = Vec::new();
    for (word, key) in KEY_MODIFIERS {
        assert!(
            classify_modifiers(key_event, &[word]).is_ok(),
            "`.{word}` is accepted on a key event"
        );
        rows.push(vec![
            code(&format!(".{word}")),
            code(&format!("\"{key}\"")),
            key_cell(word).to_string(),
        ]);
    }
    assert!(
        classify_modifiers(key_event, &["a"]).is_ok(),
        "a single character names its key"
    );
    rows.push(vec![
        "any single printable character, such as `.a`".to_string(),
        "that character".to_string(),
        key_cell("a").to_string(),
    ]);
    table(
        out,
        &["Qualifier", "`KeyboardEvent.key`", "Browser build"],
        &rows,
    );

    out.push_str("System keys — named by the catalogue and refused on every tier:\n\n");
    let mut rows = Vec::new();
    for (word, _) in SYSTEM_KEY_MODIFIERS {
        let Err((_, e)) = classify_modifiers(KEY_EVENTS[0], &[word]) else {
            panic!("`.{word}` is refused");
        };
        rows.push(vec![code(&format!(".{word}")), cell(&modifier_rule(&e))]);
    }
    table(out, &["Qualifier", "Verdict"], &rows);

    out.push_str("The refusals:\n\n");
    for (event, words, variant) in MODIFIER_RULE_ROWS {
        let Err((_, e)) = classify_modifiers(event, words) else {
            panic!("`on-{event}.{}` is refused", words.join("."));
        };
        let got = variant_name(&format!("{e:?}"));
        assert_eq!(
            &got,
            variant,
            "`on-{event}.{}` is refused as `{got}`, and its row names `{variant}`",
            words.join(".")
        );
        let _ = writeln!(out, "- {}", modifier_rule(&e));
    }
    out.push('\n');
}

fn trigger_section(out: &mut String) {
    out.push_str(
        "## Triggers\n\n\
         A sentence in a component's `Actions:` that runs its indented body when something happens. \
         The form is how a compiler message quotes the sentence.\n\n",
    );
    let mut rows = Vec::new();
    for (sentence, variant) in TRIGGER_ROWS {
        let row = parsed(
            &probe(Some(sentence), "Set n to n + 1.", None),
            |stmts, interner| {
                let app = component(stmts, interner, "App");
                let handler = app
                    .events
                    .last()
                    .unwrap_or_else(|| panic!("`{sentence}` parses to a handler"));
                let got = variant_name(&format!("{:?}", handler.trigger));
                assert_eq!(
                    &got, variant,
                    "`{sentence}` parses to `Trigger::{got}`, and its row names `{variant}`"
                );
                vec![
                    code(sentence),
                    code(handler.trigger.form()),
                    cell(&trigger_runs(&handler.trigger, interner)),
                ]
            },
        );
        rows.push(row);
    }
    table(out, &["Sentence", "Form", "Runs"], &rows);

    out.push_str("`When \"#sel\" is <word>:` takes these words:\n\n");
    let mut rows = Vec::new();
    for word in selector_event_words() {
        let sentence = format!("When \"#save\" is {word}:");
        let row = parsed(
            &probe(Some(&sentence), "Set n to n + 1.", None),
            |stmts, interner| {
                let app = component(stmts, interner, "App");
                match app.events.last().map(|h| &h.trigger) {
                    Some(Trigger::Selector {
                        event, key: None, ..
                    }) => {
                        vec![code(&word), code(interner.resolve(*event))]
                    }
                    other => panic!("`{sentence}` parses to {other:?}"),
                }
            },
        );
        rows.push(row);
    }
    table(out, &["Word", "DOM event"], &rows);
}

fn verb_section(out: &mut String) {
    out.push_str(
        "## UI verbs\n\n\
         A verb is written in a component's `Actions:`, in a `## Test` that drives a page, or in \
         both — the Where column.\n\n",
    );
    let mut rows = Vec::new();
    for (sentence, variant, position) in VERB_ROWS {
        let src = match position {
            Position::Actions => probe(None, sentence, None),
            Position::Test => probe(None, "Set n to n + 1.", Some(sentence)),
        };
        let row = parsed(&src, |stmts, interner| {
            let stmt = match position {
                Position::Actions => {
                    let app = component(stmts, interner, "App");
                    let go = app
                        .actions
                        .iter()
                        .find(|a| interner.resolve(a.name) == "go")
                        .expect("the probe declares `To go:`");
                    first_sentence(go.body)
                }
                Position::Test => first_sentence(test_body(stmts)),
            };
            let Some(Stmt::Ui(u)) = stmt else {
                panic!("`{sentence}` parses to {stmt:?}, not a UI verb");
            };
            let got = variant_name(&format!("{u:?}"));
            assert_eq!(
                &got, variant,
                "`{sentence}` parses to `UiStmt::{got}`, and its row names `{variant}`"
            );
            let role = u.role();
            let consistent = match position {
                Position::Actions => role != Role::Drive,
                Position::Test => role != Role::Action,
            };
            assert!(
                consistent,
                "`{sentence}` parsed in {position:?} and its role is {role:?}"
            );
            vec![
                code(sentence),
                role_where(role).to_string(),
                cell(verb_does(u)),
            ]
        });
        rows.push(row);
    }
    table(out, &["Sentence", "Where", "Does"], &rows);
}

fn query_section(out: &mut String) {
    out.push_str(
        "## Page queries\n\n\
         What a `## Test` asks the page, inside `Expect that …`.\n\n",
    );
    let mut rows = Vec::new();
    for (query, tail, variant) in QUERY_ROWS {
        let sentence = format!("Expect that {query}{tail}.");
        let row = parsed(
            &probe(None, "Set n to n + 1.", Some(&sentence)),
            |stmts, _| {
                let Some(Stmt::Expect { condition }) = first_sentence(test_body(stmts)) else {
                    panic!("`{sentence}` parses to an expectation");
                };
                let q = find_query(condition)
                    .unwrap_or_else(|| panic!("`{sentence}` asks the page nothing"));
                let got = variant_name(&format!("{q:?}"));
                assert_eq!(
                    &got, variant,
                    "`{query}` parses to `DomQuery::{got}`, and its row names `{variant}`"
                );
                vec![code(query), code(q.noun()), cell(query_asks(q))]
            },
        );
        rows.push(row);
    }
    table(out, &["Query", "Noun", "Asks"], &rows);
}

fn loop_section(out: &mut String) {
    let example = "{for each todo in todos, keyed by todo's id, at index i:}<li>{i}. {todo's title}</li>{else:}<li>nothing yet</li>{end}";
    let nodes = template_of(example);
    let Some(TNode::For {
        pattern,
        key: Some(key),
        index: Some(index),
        empty: Some(_),
        ..
    }) = nodes.first()
    else {
        panic!("`{example}` parses to a `{{for each}}` with a key, an index and an empty arm");
    };
    let _ = writeln!(out, "## Loop bindings\n\n{}\n", code(example));
    let rows = vec![
        vec![code(pattern), "Each item of the collection in turn.".to_string()],
        vec![
            code(key.text.trim()),
            "The row's identity: a row whose key survives a re-render keeps its element and its state.".to_string(),
        ],
        vec![
            code(index),
            format!("The item's position. The first item is {LOOP_INDEX_BASE}, the position `item {index} of todos` reads."),
        ],
        vec!["`{else:}`".to_string(), "Renders instead when the collection is empty.".to_string()],
    ];
    table(out, &["Binding", "Value"], &rows);
}

/// A `## To` header's parameters and return type: `([(name, type)], return)`.
fn signature_parts(signature: &str) -> (Vec<(String, String)>, String) {
    let (params_part, ret) = match signature.split_once("->") {
        Some((p, r)) => (p, r.trim().trim_end_matches(':').trim().to_string()),
        None => (signature, String::new()),
    };
    let mut params = Vec::new();
    let mut rest = params_part;
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else {
            break;
        };
        let group = &rest[open + 1..open + close];
        if let Some((n, t)) = group.split_once(':') {
            params.push((n.trim().to_string(), t.trim().to_string()));
        }
        rest = &rest[open + close + 1..];
    }
    (params, ret)
}

fn filter_section(out: &mut String) {
    let ui = prelude_module_sources()
        .iter()
        .find(|src| src.starts_with("# UI\n"))
        .expect("the prelude carries ui.lg");
    out.push_str(
        "## Filters\n\n\
         A filter is any function whose first parameter takes the hole's value: `{x | f a b}` is \
         `f(x, a, b)`. The list-view filters come from `ui.lg`:\n\n",
    );
    let mut rows = Vec::new();
    for doc in extract_literate_docs(ui) {
        if !doc.signature.starts_with("## To ") || doc.signature.starts_with("## To private ") {
            continue;
        }
        let (params, ret) = signature_parts(&doc.signature);
        let (first, rest) = params
            .split_first()
            .unwrap_or_else(|| panic!("`{}` takes the value it filters", doc.name));
        let mut hole = format!("{{{} | {}", first.0, doc.name);
        for (p, _) in rest {
            let _ = write!(hole, " {p}");
        }
        hole.push('}');
        let nodes = template_of(&hole);
        match nodes.first() {
            Some(TNode::Hole { filters, .. })
                if filters.len() == 1
                    && filters[0].name == doc.name
                    && filters[0].args.len() == rest.len() => {}
            other => panic!(
                "`{hole}` must parse to one filter `{}` with {} argument(s), got {other:?}",
                doc.name,
                rest.len()
            ),
        }
        let call = format!(
            "{}({}) -> {ret}",
            doc.name,
            params
                .iter()
                .map(|(n, t)| format!("{n}: {t}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let note = doc
            .doc
            .as_deref()
            .unwrap_or_else(|| panic!("`{}` carries a `## Note`", doc.name));
        let first_paragraph = note
            .split("\n\n")
            .next()
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        rows.push(vec![code(&hole), code(&call), cell(&first_paragraph)]);
    }
    assert!(!rows.is_empty(), "ui.lg declares no public function");
    table(out, &["Filter", "Function", "Answers"], &rows);
}

/// The reference, as generated from the catalogues.
fn reference() -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "# LGX reference\n\n\
         Generated from the compiler's catalogues by `lgx_reference_lock`; every example below was \
         parsed by the compiler to produce its row. Do not edit this file: run `{REGENERATE}`. \
         [Frontend](frontend.md) teaches the same constructs in order, with pages you can run.\n\n"
    );
    let t = std::time::Instant::now();
    template_section(&mut out);
    eprintln!("TIMING template {:?}", t.elapsed());
    directive_section(&mut out);
    eprintln!("TIMING directive {:?}", t.elapsed());
    modifier_section(&mut out);
    eprintln!("TIMING modifier {:?}", t.elapsed());
    trigger_section(&mut out);
    eprintln!("TIMING trigger {:?}", t.elapsed());
    verb_section(&mut out);
    eprintln!("TIMING verb {:?}", t.elapsed());
    query_section(&mut out);
    eprintln!("TIMING query {:?}", t.elapsed());
    loop_section(&mut out);
    filter_section(&mut out);
    out.push_str("---\n[Docs index](README.md) · [Frontend](frontend.md) · [Web](web.md)\n");
    out
}

// ── the census: every catalogue member, read from source, has a row ────────────────────────────

/// The variant names an enum declares, read from its source text.
fn declared_variants(source: &str, opener: &str) -> Vec<String> {
    let body = source
        .split(opener)
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .unwrap_or_else(|| panic!("`{opener}` is not declared in the source read"));
    body.lines()
        .filter_map(|l| {
            let t = l.trim();
            let head = t
                .split(|c: char| c == '{' || c == ',' || c == '(')
                .next()?
                .trim();
            (!head.is_empty()
                && head.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && head.chars().all(|c| c.is_ascii_alphanumeric()))
            .then(|| head.to_string())
        })
        .collect()
}

/// The quoted first column of a `&[(&str, …)]` / `&[&str]` table, read from its source text.
fn declared_words(source: &str, opener: &str) -> Vec<String> {
    let body = source
        .split(opener)
        .nth(1)
        .and_then(|s| s.split("];").next())
        .unwrap_or_else(|| panic!("`{opener}` is not declared in the source read"));
    let mut words = Vec::new();
    for line in body.lines() {
        let code = line.split("//").next().unwrap_or("");
        let mut rest = code;
        while let Some(open) = rest.find('"') {
            let Some(close) = rest[open + 1..].find('"') else {
                break;
            };
            words.push(rest[open + 1..open + 1 + close].to_string());
            rest = &rest[open + 1 + close + 1..];
            if code.trim_start().starts_with('(') {
                break;
            }
        }
    }
    words
}

fn selector_event_words() -> Vec<String> {
    declared_words(
        &read("crates/logicaffeine_language/src/parser/component.rs"),
        "pub(super) const SELECTOR_EVENTS: &[(&str, &str)] = &[",
    )
}

fn missing(declared: &[String], covered: &[&str]) -> Vec<String> {
    declared
        .iter()
        .filter(|d| !covered.contains(&d.as_str()))
        .cloned()
        .collect()
}

/// With `LOGOS_REGEN=1` (`./scripts/release/lgx/gen-reference.sh`) it writes the generated reference first.
#[test]
fn the_committed_reference_is_the_generated_reference() {
    let generated = reference();
    if std::env::var_os("LOGOS_REGEN").is_some() {
        std::fs::write(repo_root().join(REFERENCE), &generated).expect("write the reference");
    }
    let committed = std::fs::read_to_string(repo_root().join(REFERENCE)).unwrap_or_default();
    if committed != generated {
        let at = committed
            .lines()
            .zip(generated.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| committed.lines().count().min(generated.lines().count()));
        panic!(
            "{REFERENCE} is not the generated reference — the catalogues changed, or the file was edited \
             by hand. Regenerate it:\n\n    {REGENERATE}\n\nFirst difference at line {}:\n  committed: {:?}\n  generated: {:?}",
            at + 1,
            committed.lines().nth(at).unwrap_or("<end of file>"),
            generated.lines().nth(at).unwrap_or("<end of file>"),
        );
    }
}

#[test]
fn every_catalogue_member_has_a_row() {
    let template_src = read("crates/logicaffeine_language/src/lgx/template.rs");
    let html_src = read("crates/logicaffeine_language/src/lgx/html.rs");
    let component_src = read("crates/logicaffeine_language/src/ast/component.rs");
    let stmt_src = read("crates/logicaffeine_language/src/ast/stmt.rs");

    let template_rows: Vec<&str> = TEMPLATE_ROWS.iter().map(|(_, v)| *v).collect();
    let directive_rows: Vec<&str> = DIRECTIVE_ROWS.iter().map(|(_, v)| *v).collect();
    let trigger_rows: Vec<&str> = TRIGGER_ROWS.iter().map(|(_, v)| *v).collect();
    let verb_rows: Vec<&str> = VERB_ROWS.iter().map(|(_, v, _)| *v).collect();
    let query_rows: Vec<&str> = QUERY_ROWS.iter().map(|(_, _, v)| *v).collect();
    let rule_rows: Vec<&str> = MODIFIER_RULE_ROWS.iter().map(|(_, _, v)| *v).collect();

    let censuses: Vec<(&str, Vec<String>, &Vec<&str>)> = vec![
        (
            "TNode (lgx/template.rs)",
            declared_variants(&template_src, "pub enum TNode {"),
            &template_rows,
        ),
        (
            "TAttr (lgx/template.rs)",
            declared_variants(&template_src, "pub enum TAttr {"),
            &directive_rows,
        ),
        (
            "Trigger (ast/component.rs)",
            declared_variants(&component_src, "pub enum Trigger<'a> {"),
            &trigger_rows,
        ),
        (
            "UiStmt (ast/stmt.rs)",
            declared_variants(&stmt_src, "pub enum UiStmt<'a> {"),
            &verb_rows,
        ),
        (
            "DomQuery (ast/stmt.rs)",
            declared_variants(&stmt_src, "pub enum DomQuery<'a> {"),
            &query_rows,
        ),
        (
            "ModifierError (lgx/html.rs)",
            declared_variants(&html_src, "pub enum ModifierError {"),
            &rule_rows,
        ),
    ];
    let mut gaps = Vec::new();
    for (name, declared, rows) in &censuses {
        assert!(
            declared.len() >= 4,
            "the {name} scan found only {declared:?} — it stopped matching the source"
        );
        let absent = missing(declared, rows);
        if !absent.is_empty() {
            gaps.push(format!("{name}: {absent:?}"));
        }
        let stale: Vec<&&str> = rows
            .iter()
            .filter(|r| !r.is_empty() && !declared.iter().any(|d| d == **r))
            .collect();
        if !stale.is_empty() {
            gaps.push(format!(
                "{name}: rows name variants that no longer exist: {stale:?}"
            ));
        }
    }

    // The tag words the template parser recognises. Private to the parser, so read from source;
    // each must open some template row.
    let tag_words = declared_words(&template_src, "const TAG_WORDS: &[&str] = &[");
    assert!(
        tag_words.len() >= 8,
        "the TAG_WORDS scan found only {tag_words:?}"
    );
    let unshown: Vec<&String> = tag_words
        .iter()
        .filter(|w| {
            !TEMPLATE_ROWS.iter().any(|(t, _)| {
                t.contains(&format!("{{{w} "))
                    || t.contains(&format!("{{{w}:}}"))
                    || t.contains(&format!("{{{w}}}"))
            })
        })
        .collect();
    if !unshown.is_empty() {
        gaps.push(format!(
            "TAG_WORDS (lgx/template.rs): no template row opens with {unshown:?}"
        ));
    }

    let words = selector_event_words();
    assert!(
        words.len() >= 6,
        "the SELECTOR_EVENTS scan found only {words:?}"
    );

    assert!(
        gaps.is_empty(),
        "the reference has no row for:\n  {}\nAdd a row (an example the compiler parses to that member) in \
         crates/logicaffeine_tests/tests/lgx_reference_lock.rs, then run {REGENERATE}.",
        gaps.join("\n  ")
    );
}

#[test]
fn the_source_scanners_read_what_they_should() {
    let src = "pub enum Shape<'a> {\n    /// A doc line.\n    Circle { r: &'a Expr<'a> },\n    Square(Symbol),\n    Dot,\n    Pair { a: Symbol,\n        b: Symbol },\n}\n";
    assert_eq!(
        declared_variants(src, "pub enum Shape<'a> {"),
        ["Circle", "Square", "Dot", "Pair"]
    );
    let table = "const T: &[(&str, &str)] = &[\n    (\"clicked\", \"click\"),\n    // (\"gone\", \"x\"),\n    (\"typed\", \"input\"),\n];\n";
    assert_eq!(
        declared_words(table, "const T: &[(&str, &str)] = &["),
        ["clicked", "typed"]
    );
    let list = "const W: &[&str] = &[\"if\", \"else\", \"end\"];\n";
    assert_eq!(
        declared_words(list, "const W: &[&str] = &["),
        ["if", "else", "end"]
    );
}
