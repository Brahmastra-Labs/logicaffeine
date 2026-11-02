//! The Studio's workbench: the toolbar, the Files tree, the editor and output columns, the Socratic
//! guide and the status bar, with every mode's handlers.
//!
//! Two pages draw it. The Studio page ([`Host::Page`]) wraps it with its head, its nav, the reader's own
//! store and the address bar, which the workbench never reaches. The front page ([`Host::Frame`])
//! frames it over a shelf of shipped examples. Its state is [`WorkbenchState`], which the host holds
//! ([`use_workbench`]), so both pages show one Studio: one markup, one stylesheet, one set of handlers.

use crate::ui::components::ast_tree::AstTree;
use crate::ui::components::code_editor::{CodeEditor, CodeView, Language};
use crate::ui::components::collapsible::Collapsible;
use crate::ui::components::context_view::ContextView;
use crate::ui::components::debug_drawer::{DebugDrawer, IC_BUG};
use crate::ui::components::file_browser::FileBrowser;
use crate::ui::components::indent::Indentation;
use crate::ui::components::logic_output::{LogicOutput, OutputFormat};
use crate::ui::components::proof_panel::{dock_prompt, ProofNode, ProofPanel, ProofStatus, Walkthrough};
use crate::ui::components::repl_output::ReplOutput;
use crate::ui::components::socratic_guide::{logic_guide, GuideMode, SocraticGuide};
use crate::ui::components::status_bar::{EditorStatus, StatusBar};
use crate::ui::components::symbol_dictionary::{symbols_in, SymbolGroups};
use crate::ui::examples::{compiled_runtime_notice, planted_bug_notice};
use crate::ui::fault::{contain, ActionFault};
use crate::ui::hooks::{use_keyboard_inset, use_media, use_viewport_width};
use crate::ui::pages::heal;
use crate::ui::pages::hardware_report::{
    analyze as analyze_hardware, compiled as compile_hardware, kind_of as hardware_kind_of,
    Compiled, HardwareKind, HardwareReport,
};
use crate::ui::pages::studio::{
    attempt, auto_view, formula_on_screen, kg_svg, parse_math_statements, run_logic, run_math,
    traffic_svg, waveform_svg, CodeOutputMode, LogicRun, LogicView, MathContext, TheoremView,
    COLLAPSE_FILES_SVG, FILES_SVG, STUDIO_STYLE,
};
use crate::ui::pages::studio_actions::{actions_for, Actions, Offer};
use crate::ui::pages::studio_files::example_text;
use crate::ui::pages::studio_layout::{first_pane, Columns, MobileTab, Resizer, Shown, DRAWER_INITIAL, PROOF_DRAWER_INITIAL};
use crate::ui::pages::studio_opening::StudioOpening;
use crate::ui::responsive::media;
use crate::ui::router::studio_file_url;
use crate::ui::state::{FileNode, ReplLine, StudioMode};
use dioxus::prelude::*;
#[cfg(target_arch = "wasm32")]
use logicaffeine_compile::interpret_for_ui_baseline;
use logicaffeine_compile::{
    extract_logic_rust, extract_math_rust, generate_rust_code, interpret_streaming_with_vfs,
    interpreter::InterpreterResult, CompileResult, TheoremCompileResult,
};
use logicaffeine_kernel::interface::Repl;
use logicaffeine_language::symbol_dict::SymbolEntry;
use logicaffeine_proof::ProofExpr;
use logicaffeine_system::fs::Vfs;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// The Web preview's name, for a screen reader.
pub const PREVIEW_TITLE: &str = "The page, built from the code";

/// Where a workbench is drawn, and so what it may reach beyond itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Host {
    /// The Studio page: the whole window, columns the reader drags, the debugger, and the reader's own
    /// files through the page.
    Page,
    /// A frame on another page: a fixed height, the shelf it is given, columns its stylesheet lays
    /// out until the reader drags a bar, nothing kept, and a link that opens the open file in the
    /// Studio.
    Frame,
}

/// What the reader's store says beside the files: an error reading it, whether it lasts only for the
/// browser session, and why New file cannot make a file yet.
#[derive(Clone, PartialEq, Default)]
pub struct StoreNote {
    pub error: Option<String>,
    pub private_mode: bool,
    pub new_file_waits: Option<String>,
}

/// The files a frame shows: shipped examples, in the Studio's own folders, and the one it opens on.
pub struct Shelf {
    pub opens: &'static str,
    pub files: &'static [&'static str],
}

/// The state of a frame over `shelf`, for a page to hold: opened on `shelf.opens`, its tree the
/// shelf's files. A page calls it once, as it calls any hook, and hands it to [`StudioFrame`].
pub fn use_shelf(shelf: &'static Shelf) -> WorkbenchState {
    let opening = use_hook(|| StudioOpening::for_route(Some(shelf.opens)));
    use_workbench(&opening, || FileNode::from_paths(shelf.files.iter().copied()))
}

/// Where the site serves a shipped Web page's prebuilt preview document: `/previews/<path>.html`
/// for a page under `/examples/`, written at release by `web_previews_lock`'s regenerator from
/// [`preview_document`]. `None` for anything that is not a shipped page, so a reader's own file or
/// an edited page is built in the browser as before.
pub fn prebuilt_preview_path(vfs_path: &str) -> Option<String> {
    let text = example_text(vfs_path)?;
    (StudioMode::for_file(vfs_path, text) == Some(StudioMode::Web))
        .then(|| format!("/previews{}.html", vfs_path.strip_prefix("/examples").unwrap_or(vfs_path)))
}

/// What the Studio adds to a page's preview document, in its head: the page stands in the middle of
/// the frame, across and down, however small it is, and from the top once it is taller than the
/// frame (`safe center`). A page served by largo gets none of this: it is the Studio's frame, not
/// the page, that centers.
pub const PREVIEW_CENTERING_STYLE: &str = "<style data-studio=\"preview-centering\">html{min-height:100%;display:flex;flex-direction:column;justify-content:safe center}</style>";

/// The document the Studio's Web preview frames for `source`: the page as `largo serve` would paint
/// it, first paint included, with [`PREVIEW_CENTERING_STYLE`] in its head; or, when the page does not
/// build, the themed overlay that says why.
pub fn preview_document(source: &str) -> String {
    match logicaffeine_compile::web_host::assemble_preview_painted(source) {
        Ok(doc) => match doc.find("</head>") {
            Some(at) => format!("{}{}{}", &doc[..at], PREVIEW_CENTERING_STYLE, &doc[at..]),
            None => doc,
        },
        Err(pe) => logicaffeine_compile::web_host::error_overlay_html(&pe, source, ""),
    }
}

/// The Studio in a frame on another page: the workbench with the Studio's own stylesheet, and
/// [`FRAME_STYLE`] for what a frame lays out itself.
/// The id of the frame's element on its page: what a Try it button names, and scrolls to.
pub const FRAME_ID: &str = "studio-frame";

#[component]
pub fn StudioFrame(state: WorkbenchState) -> Element {
    rsx! {
        div { id: FRAME_ID, class: "studio-frame",
            style { "{STUDIO_STYLE}" }
            style { "{crate::ui::drag::DRAG_STYLE}" }
            style { "{FRAME_STYLE}" }
            Workbench { state, host: Host::Frame }
        }
    }
}

/// What a frame lays out itself, every rule inside the frame: its box on the page and its height in
/// each band, its columns in the proportions the Studio first opens with, which hold until the
/// reader drags a column bar (from then on the frame draws the columns as the Studio page does, in
/// its own width), and the link into the Studio.
pub const FRAME_STYLE: &str = r#"
.studio-frame {
    position: relative;
    display: flex;
    flex-direction: column;
    width: calc(100% - 32px);
    max-width: 1600px;
    height: 720px;
    margin: 0 auto;
    background: var(--studio-bg);
    color: var(--studio-text);
    border: 1px solid var(--studio-border);
    border-radius: var(--radius-lg);
    overflow: hidden;
    scrollbar-color: var(--studio-border-hover) transparent;
}

.studio-frame .studio-content {
    --col-sidebar: 240px;
    --col-left: 35%;
    --col-right: 25%;
}

.studio-frame .studio-sidebar {
    border-right: 1px solid var(--studio-border);
}

.studio-frame .studio-main > .col-left,
.studio-frame .studio-main > .col-right {
    min-width: 200px;
}

/* The way into the Studio, at the toolbar's end, in the toolbar's quieter text. */
.studio-frame .studio-open {
    display: inline-flex;
    align-items: center;
    min-height: 32px;
    padding: 0 12px;
    border: 1px solid var(--accent-primary);
    border-radius: 6px;
    color: var(--studio-text);
    font-size: 13px;
    font-weight: 600;
    text-decoration: none;
    white-space: nowrap;
}

.studio-frame .studio-open:hover {
    background: var(--accent-primary);
    color: #fff;
}

@media (min-width: 720px) {
    .studio-frame {
        width: calc(100% - 48px);
    }
}

/* The way into the Studio is a finger wide wherever a finger may press it. */
@media (max-width: 768px), (pointer: coarse), (max-height: 500px) and (orientation: landscape) {
    .studio-frame .studio-open {
        min-height: 44px;
    }
}

/* The frame's height in each band, set by the stylesheet alone, so the server's render is the first
   frame at every width: some page always shows above or below it on a phone, where a swipe scrolls
   the page. */
@media (min-width: 769px) and (min-height: 501px) and (pointer: coarse) {
    .studio-frame {
        height: min(720px, 85svh);
    }
}

@media (min-width: 601px) and (max-width: 768px), (min-width: 769px) and (max-width: 1024px) and (orientation: portrait) and (pointer: coarse) {
    .studio-frame {
        height: min(960px, 85svh);
    }
}

@media (max-width: 600px) {
    .studio-frame {
        height: min(640px, 75svh);
    }
}

@media (max-height: 500px) and (orientation: landscape) {
    .studio-frame {
        height: 85svh;
    }
}
"#;

/// Everything the workbench shows and edits: what is open, each mode's editor and what each mode last
/// made, and how the columns are laid out. The host holds it ([`use_workbench`]), so the Studio page's
/// own effects, its store and its address bar, and the workbench's handlers work on one state; it is
/// signals, so it is `Copy`.
#[derive(Clone, Copy, PartialEq)]
pub struct WorkbenchState {
    // What is open
    /// The mode the open file is in: the Studio has no mode switch, opening a file is choosing it.
    pub(super) mode: Signal<StudioMode>,
    /// Whether the Files tree shows as a column, where it is one.
    pub(super) sidebar_open: Signal<bool>,
    /// Whether the Files tree is a drawer over the workbench on this screen (`media::FILES_DRAWER`).
    pub(super) drawer: Signal<bool>,
    /// Whether the reader opened that drawer. It starts folded, so no tree covers the editor.
    pub(super) drawer_open: Signal<bool>,
    /// Whether the panes show one at a time or in halves (`media::PANES_FIXED`), so a pane's header
    /// folds nothing.
    pub(super) panes_fixed: Signal<bool>,
    /// How much of the window an on-screen keyboard covers.
    pub(super) keyboard_inset: Signal<f64>,
    /// The Files tree.
    pub(super) file_tree: Signal<FileNode>,
    /// The file open in the editor, by path.
    pub(super) current_file: Signal<Option<String>>,
    /// The text the reader left each file at this visit, by path, so opening it again shows it. Nothing
    /// of it is written to a store.
    pub(super) edits: Signal<HashMap<String, String>>,
    // Logic
    pub(super) input: Signal<String>,
    pub(super) result: Signal<CompileResult>,
    pub(super) format: Signal<OutputFormat>,
    pub(super) proof_text: Signal<String>,
    /// The derivation, as the drawer's tree, once a proof has one.
    pub(super) proof_tree: Signal<Option<ProofNode>>,
    pub(super) proof_status: Signal<ProofStatus>,
    pub(super) proof_hint: Signal<Option<String>>,
    pub(super) current_proof_expr: Signal<Option<ProofExpr>>,
    pub(super) knowledge_base: Signal<Vec<ProofExpr>>,
    pub(super) logic_output_mode: Signal<LogicView>,
    pub(super) generated_logic_rust: Signal<String>,
    pub(super) dictionary_open: Signal<bool>,
    pub(super) tree_open: Signal<bool>,
    pub(super) dictionary: Memo<Option<Vec<SymbolEntry>>>,
    /// The proof drawer's height, which its grip changes, and whether the drawer is open.
    pub(super) proof_height: Signal<f64>,
    pub(super) proof_open: Signal<bool>,
    // Code
    /// Code and Web share one editor.
    pub(super) code_input: Signal<String>,
    pub(super) code_output_mode: Signal<CodeOutputMode>,
    pub(super) interpreter_result: Signal<InterpreterResult>,
    pub(super) generated_rust: Signal<String>,
    pub(super) debugging: Signal<bool>,
    pub(super) debug_output: Signal<InterpreterResult>,
    pub(super) drawer_height: Signal<f64>,
    /// What a program's file statements read and write: the reader's store on the Studio page, none in a frame.
    pub(super) program_store: Signal<Option<Arc<dyn Vfs>>>,
    // Math
    pub(super) math_input: Signal<String>,
    pub(super) math_context: Signal<MathContext>,
    pub(super) math_output: Signal<Vec<ReplLine>>,
    pub(super) math_output_mode: Signal<CodeOutputMode>,
    pub(super) generated_math_rust: Signal<String>,
    // Hardware
    pub(super) hw_input: Signal<String>,
    pub(super) hw_output_mode: Signal<CodeOutputMode>,
    pub(super) hw_report: Signal<Option<(HardwareKind, HardwareReport)>>,
    pub(super) generated_hw_rust: Signal<String>,
    pub(super) hw_compiled: Signal<Option<Compiled>>,
    // Web
    pub(super) debounced_code: Signal<String>,
    pub(super) web_edit_gen: Signal<u64>,
    pub(super) web_preview: Memo<String>,
    // Every mode
    pub(super) fault: Signal<Option<ActionFault>>,
    pub(super) actions: Memo<Actions>,
    pub(super) editor_status: Signal<EditorStatus>,
    /// LLM Heal: where the fairy is and what her card shows (work/LLM_HEAL.md).
    pub(super) heal: Signal<heal::Flight>,
    /// What each file's text was when it opened or was last kept, by path: a heal mends only the
    /// lines a person wrote since (the user, 2026-11-02).
    pub(super) heal_baseline: Signal<std::collections::HashMap<String, String>>,
    // Layout
    pub(super) column_pref: Signal<Option<Columns>>,
    pub(super) viewport_width: Signal<f64>,
    pub(super) active_tab: Signal<MobileTab>,
    pub(super) editor_expanded: Signal<bool>,
    pub(super) output_expanded: Signal<bool>,
    pub(super) mobile_split: Signal<f64>,
}

/// The workbench's state, for a host to hold: what `opening` opens on, over the files `tree` lists. The
/// host calls it once, as it calls any hook, and hands what it returns to [`Workbench`].
pub fn use_workbench(opening: &StudioOpening, tree: impl FnOnce() -> FileNode) -> WorkbenchState {
    // Mode state
    let mode = use_signal(|| opening.mode);

    // File browser state
    let sidebar_open = use_signal(|| true);
    let drawer = use_media(media::FILES_DRAWER);
    let drawer_open = use_signal(|| false);
    let panes_fixed = use_media(media::PANES_FIXED);
    let keyboard_inset = use_keyboard_inset();
    // The examples' part of the tree is the site's own, there from the first frame. The reader's
    // files join it once their store answers.
    let file_tree = use_signal(tree);
    let current_file = use_signal(|| opening.current_file());
    let edits = use_signal(HashMap::new);

    // Logic mode state
    let input = use_signal(|| opening.editor_text(StudioMode::Logic));
    let result = use_signal(|| CompileResult {
        logic: None,
        simple_logic: None,
        kripke_logic: None,
        ast: None,
        readings: Vec::new(),
        simple_readings: Vec::new(),
        kripke_readings: Vec::new(),
        tokens: Vec::new(),
        error: None,
    });
    let format = use_signal(|| OutputFormat::SimpleFOL);

    // Proof panel state for Logic mode
    let proof_text = use_signal(String::new);
    let proof_tree = use_signal(|| None::<ProofNode>);
    let proof_status = use_signal(|| ProofStatus::Idle);
    let proof_hint = use_signal(|| None::<String>);
    // The current ProofExpr for the proof engine
    let current_proof_expr = use_signal(|| None::<ProofExpr>);
    // Knowledge base (axioms/premises) for the proof engine
    let knowledge_base = use_signal(Vec::<ProofExpr>::new);
    // Logic mode output view: FOL (Logic) vs extracted Rust
    let logic_output_mode = use_signal(|| LogicView::Logic);
    let generated_logic_rust = use_signal(String::new);
    // The Logic column's blocks under the formula, open or closed as the reader left them — across
    // Executes, errors and files — until the page is left.
    let dictionary_open = use_signal(|| true);
    let tree_open = use_signal(|| false);
    // What the Symbol Dictionary lists: the symbols of the formula on screen, in its view.
    let dictionary = use_memo(move || {
        formula_on_screen(&result.read(), *format.read())
            .map(|(logic, view)| symbols_in(&logic, view))
            .filter(|symbols| !symbols.is_empty())
    });

    // Code mode state (imperative .logos)
    let code_input = use_signal(|| opening.editor_text(StudioMode::Code));
    let code_output_mode = use_signal(|| CodeOutputMode::Interpret);
    let interpreter_result = use_signal(|| InterpreterResult {
        lines: vec![],
        error: None,
    });
    let generated_rust = use_signal(String::new);
    // Code-mode debugger drawer (bottom-docked, additive — see `DebugDrawer`).
    let debugging = use_signal(|| false);

    // Math mode state (vernacular/theorem proving)
    let math_input = use_signal(|| opening.editor_text(StudioMode::Math));
    // The environment the last run built, as the Context panel lists it.
    let math_context = use_signal(MathContext::prelude);
    let math_output = use_signal(Vec::<ReplLine>::new);
    // Math mode output view: REPL output (Interpret) vs extracted Rust
    let math_output_mode = use_signal(|| CodeOutputMode::Interpret);
    let generated_math_rust = use_signal(String::new);

    // Hardware mode state (English hardware spec -> SVA + in-browser proving)
    let hw_input = use_signal(|| opening.editor_text(StudioMode::Hardware));
    // Output view: SVA (Interpret) vs extracted Rust runtime monitor.
    let hw_output_mode = use_signal(|| CodeOutputMode::Interpret);
    // What Hardware mode made of the editor's text, from one reading (`hardware_report::analyze`):
    // its kind, the result panel's sections and verdict, the trace, signals and knowledge graph.
    let mut hw_report = use_signal(|| None::<(HardwareKind, HardwareReport)>);
    let generated_hw_rust = use_signal(String::new);
    // What Compile said when it made no Rust: a design with nothing to compile, or one it could
    // not read. Cleared whenever the design is read again.
    let hw_compiled = use_signal(|| None::<Compiled>);

    // What stopped the last action, if it failed in a way nobody planned for: the toolbar's
    // orange note (see `attempt`).
    let fault = use_signal(|| None::<ActionFault>);

    // The editor each mode reads and writes: Code and Web share one.
    let editor = move |mode: StudioMode| match mode {
        StudioMode::Logic => input,
        StudioMode::Code | StudioMode::Web => code_input,
        StudioMode::Math => math_input,
        StudioMode::Hardware => hw_input,
    };

    // What each toolbar action can do for the file in the editor (studio_actions.rs): an action it
    // cannot take stays in place, disabled, and says why. Read behind the fault boundary — a fault
    // in reading the file leaves every action live, to report the fault itself when pressed.
    let actions = use_memo(move || {
        let mode = *mode.read();
        let path = current_file.read().clone();
        let content = editor(mode).read().clone();
        contain(|| actions_for(mode, path.as_deref(), &content)).unwrap_or(Actions {
            run: Offer::Ready,
            compile: Offer::Ready,
            debug: Offer::Ready,
        })
    });

    // The column layout the user has shaped by dragging — none until the first drag — drawn
    // fitted to the window's width (see studio_layout.rs).
    let column_pref = use_signal(|| None::<Columns>);
    let viewport_width = use_viewport_width();
    // The debugger drawer's height, kept across closing and reopening it.
    let drawer_height = use_signal(|| DRAWER_INITIAL);
    // What the debugged program has printed at the cursor, shown in the Output column while the
    // debugger is open.
    let debug_output = use_signal(|| InterpreterResult {
        lines: Vec::new(),
        error: None,
    });
    // Where the editor's caret is and what its language service found, for the status bar.
    let editor_status = use_signal(EditorStatus::default);
    let proof_height = use_signal(|| PROOF_DRAWER_INITIAL);
    let proof_open = use_signal(|| true);
    // The pane a phone shows, the one that shows the file first.
    let active_tab = use_signal(|| first_pane(opening.mode));

    // Mobile panel collapse state
    let editor_expanded = use_signal(|| true);
    let output_expanded = use_signal(|| true);

    // Mobile vertical split: Panel 1 height as % of studio-main
    let mobile_split = use_signal(|| 50.0f64);

    // The store a program's file statements read and write: the reader's, once the Studio page
    // has opened it. A frame has none.
    let program_store = use_signal(|| None::<Arc<dyn Vfs>>);

    // Debounce the Web preview: recompiling wasm on every keystroke makes the iframe
    // flicker and steals cycles while you type. We commit the editor buffer into
    // `debounced_code` only after ~300ms of quiet (a generation counter voids stale
    // spawns — the last edit wins), and the preview reads the settled buffer. A page the
    // workbench opens on is settled from the start, so the first frame, the server's render
    // included, already shows it.
    let mut debounced_code = use_signal(|| {
        if opening.mode == StudioMode::Web {
            opening.editor_text(StudioMode::Web)
        } else {
            String::new()
        }
    });
    let mut web_edit_gen = use_signal(|| 0u64);
    // The file the settled source came from: a source that arrives with another file is a file
    // opened, not a keystroke, and shows at once.
    let mut settled_file = use_signal(|| opening.current_file());
    use_effect(move || {
        if *mode.read() != StudioMode::Web {
            return;
        }
        let src = code_input.read().clone();
        let file = current_file.read().clone();
        if *debounced_code.peek() == src {
            return;
        }
        // A page opened after another mode shows instantly — nothing to debounce against
        // yet — and so does a file opened from the tree; only the *edits* wait for the quiet window.
        if debounced_code.peek().is_empty() || *settled_file.peek() != file {
            settled_file.set(file);
            debounced_code.set(src);
            return;
        }
        let my_gen = *web_edit_gen.peek() + 1;
        web_edit_gen.set(my_gen);
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(300).await;
            if *web_edit_gen.peek() == my_gen {
                debounced_code.set(src);
            }
        });
    });

    // The live "Web" preview: compile the LOGOS source to wasm in-browser and assemble a
    // self-contained iframe `srcdoc` that carries the page's first paint (a build error becomes
    // the themed overlay), so a rebuild never flashes blank. Recomputes only when the debounced
    // source settles and only in Web mode — the edit-and-see loop.
    // A page built once in this session is not built again: coming back to a shelf file shows its
    // preview at once (the build is the slow part, the LOGOS compiler running in the browser).
    let built = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::<String, String>::new())));
    // A shipped page's document comes prebuilt from the site (`/previews/...`, written at release
    // by `web_previews_lock`'s regenerator) rather than built in the browser on every open (the
    // user, 2026-11-02: the web files were slow to load). Keyed by the file's path, filled by the
    // fetch below, read by the memo; `last_doc` is what the frame keeps showing while a fetch is on
    // its way, so nothing is built that the fetch will bring.
    let prebuilt = use_signal(std::collections::HashMap::<String, String>::new);
    let last_doc = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(String::new())));
    // The server's render already holds the open page's document in the frame on the page: take
    // it before the first render, so hydration shows it instead of building it again (the user,
    // 2026-11-02: a web file took two seconds to open; that was the compiler running once more).
    // Whether the first render has a server render to match: a prerendered page has the frame
    // on the page already; the dev server and a route the site does not prerender have nothing,
    // so a shipped page there waits the few milliseconds for its prebuilt document instead of
    // building it (the dev build opened a web file a second slower than a code file).
    #[cfg(target_arch = "wasm32")]
    let hydrating = use_hook(|| {
        let served = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector(&format!("iframe[title=\"{PREVIEW_TITLE}\"]")).ok().flatten())
            .and_then(|frame| frame.get_attribute("srcdoc"))
            .filter(|d| !d.is_empty());
        let Some(doc) = served else { return false };
        if opening.mode == StudioMode::Web {
            built.borrow_mut().insert(opening.editor_text(StudioMode::Web), doc.clone());
            *last_doc.borrow_mut() = doc;
        }
        true
    });
    #[cfg(target_arch = "wasm32")]
    {
        let in_flight = use_signal(std::collections::HashSet::<String>::new);
        // Copies of the signals inside, so the closure is `Fn` and copies into each effect.
        let fetch = move |path: String, url: String| {
            let mut in_flight = in_flight;
            let mut prebuilt = prebuilt;
            if prebuilt.peek().contains_key(&path) || in_flight.peek().contains(&path) {
                return;
            }
            in_flight.write().insert(path.clone());
            spawn(async move {
                if let Ok(response) = gloo_net::http::Request::get(&url).send().await {
                    if response.ok() {
                        if let Ok(doc) = response.text().await {
                            prebuilt.write().insert(path.clone(), doc);
                        }
                    }
                }
                in_flight.write().remove(&path);
            });
        };
        // The open page first, then every shipped page once the app is up, so a click on a web
        // file finds its document already here.
        use_effect(move || {
            if *mode.read() != StudioMode::Web {
                return;
            }
            let Some(path) = current_file.read().clone() else { return };
            let Some(url) = prebuilt_preview_path(&path) else { return };
            fetch(path, url);
        });
        use_hook(|| {
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(1500).await;
                for spec in crate::ui::examples::seeded_examples() {
                    if let Some(url) = prebuilt_preview_path(spec.vfs_path) {
                        fetch(spec.vfs_path.to_string(), url);
                    }
                }
            });
        });
    }
    let web_preview = use_memo(move || {
        if *mode.read() != StudioMode::Web {
            return String::new();
        }
        let src = debounced_code.read().clone();
        if let Some(doc) = built.borrow().get(&src) {
            return doc.clone();
        }
        if let Some(path) = current_file.read().as_deref() {
            if example_text(path).is_some_and(|text| text == src) && prebuilt_preview_path(path).is_some() {
                if let Some(doc) = prebuilt.read().get(path) {
                    built.borrow_mut().insert(src, doc.clone());
                    *last_doc.borrow_mut() = doc.clone();
                    return doc.clone();
                }
                // In the browser the fetch brings it, and the frame keeps its last document until
                // then; with no document yet (the first render, which must match the server's,
                // or a cold open) the page is built here. The server always builds it.
                #[cfg(target_arch = "wasm32")]
                {
                    let last = last_doc.borrow().clone();
                    if !last.is_empty() || !hydrating {
                        return last;
                    }
                }
            }
        }
        let doc = preview_document(&src);
        built.borrow_mut().insert(src, doc.clone());
        *last_doc.borrow_mut() = doc.clone();
        doc
    });

    let heal = use_signal(|| heal::Flight::Idle);
    let heal_baseline = use_signal(|| {
        let mut baselines = std::collections::HashMap::new();
        if let Some(path) = opening.current_file() {
            baselines.insert(path, opening.editor_text(opening.mode));
        }
        baselines
    });

    let state = WorkbenchState {
        mode,
        sidebar_open,
        drawer,
        drawer_open,
        panes_fixed,
        keyboard_inset,
        file_tree,
        current_file,
        edits,
        input,
        result,
        format,
        proof_text,
        proof_tree,
        proof_status,
        proof_hint,
        current_proof_expr,
        knowledge_base,
        logic_output_mode,
        generated_logic_rust,
        dictionary_open,
        tree_open,
        dictionary,
        proof_height,
        proof_open,
        code_input,
        code_output_mode,
        interpreter_result,
        generated_rust,
        debugging,
        debug_output,
        drawer_height,
        program_store,
        math_input,
        math_context,
        math_output,
        math_output_mode,
        generated_math_rust,
        hw_input,
        hw_output_mode,
        hw_report,
        generated_hw_rust,
        hw_compiled,
        debounced_code,
        web_edit_gen,
        web_preview,
        fault,
        actions,
        editor_status,
        heal,
        heal_baseline,
        column_pref,
        viewport_width,
        active_tab,
        editor_expanded,
        output_expanded,
        mobile_split,
    };

    // The pigeonhole and register-allocation specs are read as they are typed, as their pictures
    // always were; the heavier designs wait for Execute.
    use_effect(move || {
        let content = hw_input.read().clone();
        if matches!(
            hardware_kind_of(&content),
            Some(HardwareKind::Pigeonhole | HardwareKind::RegisterAlloc)
        ) {
            hw_report.set(analyze_hardware(&content));
        }
    });

    // The file the workbench opened with runs at once when the site carries its text: Logic reads it,
    // a program runs, a hardware design is analysed.
    #[cfg(target_arch = "wasm32")]
    {
        let opening = opening.clone();
        let mut started = use_signal(|| false);
        use_effect(move || {
            if *started.read() {
                return;
            }
            started.set(true);
            if let Some(text) = opening.text {
                let (opened_mode, path) = (opening.mode, opening.path.clone());
                spawn(async move {
                    // Let the first frame paint before the work.
                    gloo_timers::future::TimeoutFuture::new(0).await;
                    state.run_opened(opened_mode, path, text.to_string()).await;
                });
            }
        });
    }

    state
}

impl WorkbenchState {
    /// The editor `mode` reads and writes: Code and Web share one.
    pub fn editor(self, mode: StudioMode) -> Signal<String> {
        match mode {
            StudioMode::Logic => self.input,
            StudioMode::Code | StudioMode::Web => self.code_input,
            StudioMode::Math => self.math_input,
            StudioMode::Hardware => self.hw_input,
        }
    }

    /// A compiled `## Theorem:` block, shown the one way opening its file and Execute share: its goal
    /// and premises loaded for the tactics and its proof panel filled, or why it did not compile.
    pub(super) fn show_theorem(self, theorem: Result<(TheoremCompileResult, TheoremView), String>) {
        let WorkbenchState { mut result, mut proof_text, mut proof_tree, mut proof_status, mut proof_hint, mut current_proof_expr, mut knowledge_base, .. } = self;
        match theorem {
            Err(err) => {
                result.set(CompileResult {
                    logic: None,
                    simple_logic: None,
                    kripke_logic: None,
                    ast: None,
                    readings: Vec::new(),
                    simple_readings: Vec::new(),
                    kripke_readings: Vec::new(),
                    tokens: Vec::new(),
                    error: Some(err.clone()),
                });
                proof_status.set(ProofStatus::Failed(err));
                current_proof_expr.set(None);
                knowledge_base.write().clear();
            }
            Ok((theorem, view)) => {
                result.set(CompileResult {
                    logic: theorem.goal_string.clone(),
                    simple_logic: theorem.goal_string.clone(),
                    kripke_logic: None,
                    ast: None,
                    readings: Vec::new(),
                    simple_readings: Vec::new(),
                    kripke_readings: Vec::new(),
                    tokens: Vec::new(),
                    error: None,
                });
                knowledge_base.set(theorem.premises);
                if let Some(goal) = theorem.goal {
                    current_proof_expr.set(Some(goal));
                }
                proof_text.set(view.html);
                proof_tree.set(view.tree);
                proof_status.set(view.status);
                proof_hint.set(Some(view.hint));
            }
        }
    }

    /// What opening a file does once its text is in the editor: Logic reads its sentences or theorem,
    /// a program runs, a hardware design is analysed. Math waits for Execute, and a Web page's preview
    /// effect builds it from the editor.
    #[cfg(target_arch = "wasm32")]
    async fn run_opened(self, file_mode: StudioMode, path: String, content: String) {
        let WorkbenchState { mut result, mut current_proof_expr, mut interpreter_result, mut hw_report, mut hw_compiled, fault, .. } = self;
        match file_mode {
            StudioMode::Logic => match attempt(fault, "Execute", || run_logic(&content)) {
                Some(LogicRun::Theorem(theorem)) => self.show_theorem(theorem),
                Some(LogicRun::Sentences(compiled, proof)) => {
                    result.set(compiled);
                    if let Some(expr) = proof.proof_expr {
                        current_proof_expr.set(Some(expr));
                    }
                }
                Some(LogicRun::Empty) | None => {}
            },
            // A native-only example opens to its notice: run here, it would only pretend.
            StudioMode::Code => {
                interpreter_result.set(if compiled_runtime_notice(&path).is_some() {
                    InterpreterResult {
                        lines: Vec::new(),
                        error: None,
                    }
                } else {
                    interpret_for_ui_baseline(&content).await
                })
            }
            StudioMode::Hardware => {
                hw_report.set(analyze_hardware(&content));
                hw_compiled.set(None);
            }
            StudioMode::Math | StudioMode::Web => {}
        }
    }

    /// Opening a file is choosing its mode: the Studio has no mode switch. The file picks the mode
    /// (`StudioMode::for_file`; a file with no LOGOS extension keeps the current one), its content loads
    /// into that mode's editor (or the text the reader left it at, this visit), and it runs. The store's answer on mount, a choice in the Files tree and
    /// a host opening a file all come through here, so they can never disagree about a file.
    #[cfg(target_arch = "wasm32")]
    pub async fn open(self, path: String, content: String) {
        let WorkbenchState { mut mode, mut fault, .. } = self;
        let file_mode = StudioMode::for_file(&path, &content).unwrap_or(*mode.peek());
        mode.set(file_mode);
        // A note about the last file's run says nothing about this one.
        if fault.peek().is_some() {
            fault.set(None);
        }
        let content = self.opening_text(&path, content);
        self.editor(file_mode).set(content.clone());
        // A heal mends what the reader writes from here.
        let mut baseline = self.heal_baseline;
        baseline.write().insert(path.clone(), content.clone());
        self.run_opened(file_mode, path, content).await;
    }

    /// The text a file opens with: what the reader left it at this visit, or else `content`.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(super) fn opening_text(self, path: &str, content: String) -> String {
        self.edits.peek().get(path).cloned().unwrap_or(content)
    }

    /// Keep the open file's text as the reader left it, for when they come back to it.
    fn keep_edit(self) {
        let WorkbenchState { mode, current_file, mut edits, .. } = self;
        let open = current_file.peek().clone();
        if let Some(path) = open {
            let text = self.editor(*mode.peek()).peek().clone();
            edits.write().insert(path, text);
        }
    }

    /// A file chosen in the Files tree, or by the host: it becomes the open file, the tree folds away on
    /// a phone, and a shipped example opens at once, from the site itself, even before any store has
    /// answered. A reader's own file is the host's to read.
    pub fn choose(self, path: String) {
        let mut current_file = self.current_file;
        let mut drawer_open = self.drawer_open;
        if *self.drawer.peek() && *drawer_open.peek() {
            drawer_open.set(false);
        }
        self.keep_edit();
        current_file.set(Some(path.clone()));
        #[cfg(target_arch = "wasm32")]
        if let Some(text) = example_text(&path) {
            let mut active_tab = self.active_tab;
            let file_mode = StudioMode::for_file(&path, text).unwrap_or(*self.mode.peek());
            spawn(async move {
                active_tab.set(first_pane(file_mode));
                self.open(path, text.to_string()).await;
            });
        }
    }
}

/// The closest element matching `selector` to the one `evt` pressed: a drag works on the bars and panels
/// of the workbench it began in, wherever else the page holds a copy of them (the prerendered copy,
/// while the app takes over from it).
#[cfg(target_arch = "wasm32")]
fn pressed(evt: &PointerEvent, selector: &str) -> Option<web_sys::Element> {
    use wasm_bindgen::JsCast;
    let target = evt.data().downcast::<web_sys::PointerEvent>()?.target()?;
    target.dyn_into::<web_sys::Element>().ok()?.closest(selector).ok().flatten()
}

/// The Studio's working area for `state`, drawn for `host`.
#[component]
pub fn Workbench(
    state: WorkbenchState,
    host: Host,
    /// What the reader's store says beside the files. A frame has no store.
    #[props(default)]
    store: StoreNote,
    /// New file, where the host keeps the reader's files. With none, the Files header offers no New
    /// file.
    #[props(default)]
    on_new_file: Option<EventHandler<()>>,
    /// A file the reader chose in the Files tree, after the workbench has opened it when the site
    /// carries it. The Studio page keeps the address bar on it and reads a reader's own file.
    #[props(default)]
    on_choose: Option<EventHandler<String>>,
) -> Element {
    let WorkbenchState { mode, mut sidebar_open, drawer, mut drawer_open, panes_fixed, keyboard_inset, mut file_tree, current_file, mut input, mut result, mut format, mut proof_text, mut proof_tree, mut proof_status, mut proof_hint, mut current_proof_expr, mut knowledge_base, mut logic_output_mode, mut generated_logic_rust, dictionary_open, tree_open, dictionary, proof_height, mut proof_open, mut code_input, mut code_output_mode, mut interpreter_result, mut generated_rust, mut debugging, debug_output, drawer_height, program_store, mut math_input, mut math_context, mut math_output, mut math_output_mode, mut generated_math_rust, mut hw_input, mut hw_output_mode, mut hw_report, mut generated_hw_rust, mut hw_compiled, web_preview, fault, actions, editor_status, mut heal, mut heal_baseline, column_pref, viewport_width, mut active_tab, mut editor_expanded, mut output_expanded, mobile_split, .. } = state;

    // The width of a frame's panel row, measured in the browser once the frame is on the page and
    // again whenever the window changes size; `None` on the server and before the first measure.
    let mut frame_row_width = use_signal(|| None::<f64>);
    use_effect(move || {
        let _ = viewport_width.read();
        if host != Host::Frame {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let measured = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.query_selector(".studio-frame .studio-content").ok().flatten())
                .map(|row| row.client_width() as f64)
                .filter(|w| *w > 0.0);
            if measured.is_some() {
                frame_row_width.set(measured);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = &mut frame_row_width;
        }
    });
    let show_theorem = move |theorem| state.show_theorem(theorem);
    // Whether a tap on the fault note opened its whole story.
    let mut fault_open = use_signal(|| false);


    // Logic mode input handler - compiles for both UI and proof engine
    // Logic mode keystroke handler: just store the text. The heavy compile/prove/grid runs
    // on the Execute button (`handle_logic_execute`), mirroring Code mode's Run — so typing
    // never triggers a per-keystroke solve.
    let handle_logic_input = move |new_value: String| {
        input.set(new_value);
    };

    // Code mode: Run button handler (interpret) with streaming output
    let handle_code_run = move |_| {
        let code = code_input.read().clone();
        // Switch to Output tab (Panel2) on mobile and switch to Output mode
        active_tab.set(MobileTab::Panel2);
        code_output_mode.set(CodeOutputMode::Interpret);
        // Clear previous output
        interpreter_result.set(InterpreterResult {
            lines: vec![],
            error: None,
        });
        spawn(async move {
            // Create streaming callback that updates the signal as output arrives
            let callback = Rc::new(RefCell::new(move |line: String| {
                // Update the signal with the new line
                interpreter_result.write().lines.push(line);
            }));

            // Route the interpreter's file I/O to the store the host gave the workbench (the
            // Studio page's OPFS/IndexedDB), so the standard-library I/O vocabulary works in the
            // browser. A frame, and the native build, have none.
            let vfs = program_store.read().clone();
            let result = interpret_streaming_with_vfs(&code, callback, vfs).await;
            // Set final result (includes any error)
            interpreter_result.set(result);
        });
    };

    // Code mode: Compile button handler (generate Rust)
    // Uses generate_rust_code which works on WASM
    let handle_code_compile = move |_| {
        let code = code_input.read().clone();
        // Switch to Output tab (Panel2) on mobile
        active_tab.set(MobileTab::Panel2);
        spawn(async move {
            // Yield once so the switch to the output renders before the work.
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(0).await;
            match generate_rust_code(&code) {
                Ok(rust_code) => {
                    generated_rust.set(rust_code);
                    code_output_mode.set(CodeOutputMode::Rust);
                }
                Err(e) => {
                    interpreter_result.set(InterpreterResult {
                        lines: vec![],
                        error: Some(format!("Compile error: {:?}", e)),
                    });
                    code_output_mode.set(CodeOutputMode::Interpret);
                }
            }
        });
    };

    // Math mode execute handler (vernacular REPL).
    // Re-runs from a fresh kernel each press so output is REPLACED, not appended
    // (mirrors Code mode's Run — no manual Clear needed). The editor holds the
    // full program, so a fresh run is idempotent — and the Context panel, which
    // lists the same environment in the same order, holds still.
    let handle_math_execute = move |_| {
        active_tab.set(MobileTab::Panel2);
        math_output_mode.set(CodeOutputMode::Interpret);
        let code = math_input.read().clone();
        spawn(async move {
            // Yield once so the switch to the output renders before the work.
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(0).await;
            let Some(run) = attempt(fault, "Execute", || run_math(&code)) else {
                return;
            };
            if let Some(context) = run.context {
                if *math_context.peek() != context {
                    math_context.set(context);
                }
            }
            math_output.set(run.lines);
        });
    };

    // Math mode compile handler: rebuild the kernel context from the editor, then
    // extract every user-defined Definition/Inductive into one Rust module.
    let handle_math_compile = move |_| {
        active_tab.set(MobileTab::Panel2);
        let code = math_input.read().clone();
        spawn(async move {
            // Yield once so the switch to the output renders before the work.
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(0).await;
            let compiled = attempt(fault, "Compile", || {
                let mut repl = Repl::new();
                for stmt in parse_math_statements(&code) {
                    let _ = repl.execute(&stmt);
                }
                let rust = match extract_math_rust(repl.context()) {
                    Ok(rust) => rust,
                    Err(e) => format!("// extraction error: {e}"),
                };
                (MathContext::of(repl.context()), rust)
            });
            let Some((context, rust)) = compiled else {
                return;
            };
            if *math_context.peek() != context {
                math_context.set(context);
            }
            generated_math_rust.set(rust);
            math_output_mode.set(CodeOutputMode::Rust);
        });
    };

    // Logic mode execute handler: compile the theorem (or sentences), prove it, and render
    // the result — the solved grid, the wh-question answer, or the certified derivation.
    // Like Code mode's Run, the heavy work happens on this button, not on every keystroke.
    let handle_logic_execute = move |_| {
        active_tab.set(MobileTab::Panel2);
        logic_output_mode.set(LogicView::Logic);
        let text = input.read().clone();
        match attempt(fault, "Execute", || run_logic(&text)) {
            Some(LogicRun::Theorem(theorem)) => show_theorem(theorem),
            Some(LogicRun::Sentences(compiled, proof)) => {
                result.set(compiled);
                knowledge_base.write().clear();
                if let Some(expr) = proof.proof_expr {
                    current_proof_expr.set(Some(expr));
                    proof_status.set(ProofStatus::Idle);
                    proof_hint.set(Some("Press Prove to check the first sentence, or give it Given lines in a ## Theorem: block.".to_string()));
                } else if let Some(err) = proof.error {
                    current_proof_expr.set(None);
                    proof_status.set(ProofStatus::Failed(err));
                }
            }
            Some(LogicRun::Empty) => {
                result.set(CompileResult {
                    logic: None,
                    simple_logic: None,
                    kripke_logic: None,
                    ast: None,
                    readings: Vec::new(),
                    simple_readings: Vec::new(),
                    kripke_readings: Vec::new(),
                    tokens: Vec::new(),
                    error: None,
                });
                current_proof_expr.set(None);
                knowledge_base.write().clear();
                proof_text.set(String::new());
                proof_tree.set(None);
                proof_status.set(ProofStatus::Idle);
            }
            None => {}
        }
    };

    // Logic mode compile handler: extract Rust where there's constructive content
    // (a `## Theorem:` block / definitions); honest note otherwise.
    let handle_logic_compile = move |_| {
        active_tab.set(MobileTab::Panel2);
        let text = input.read().clone();
        spawn(async move {
            // Yield once so the switch to the output renders before the work.
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(0).await;
            let extracted = attempt(fault, "Compile", || match extract_logic_rust(&text) {
                Ok(rust) => rust,
                Err(e) => format!("// extraction error: {e}"),
            });
            let Some(rust) = extracted else { return };
            generated_logic_rust.set(rust);
            logic_output_mode.set(LogicView::Rust);
        });
    };

    // Logic mode: Prove asks the kernel for a derivation of the goal from the knowledge base
    // (`auto_view`), opens the proof drawer, and shows what came of it. The prover searches on its
    // own, so there is no tactic for a reader to choose.
    let handle_prove = use_callback(move |_: ()| {
        let maybe_goal = current_proof_expr.read().clone();
        let kb = knowledge_base.read().clone();
        // Prove checks the first sentence, so the goal it reports is that sentence's formula, not
        // the numbered list of every sentence's.
        let logic_str = result.read().simple_logic.as_deref().and_then(|all| all.lines().next()).map(|first| {
            first.trim_start_matches(|c: char| c.is_ascii_digit() || c == ')' || c == ' ').to_string()
        });
        proof_open.set(true);
        match maybe_goal {
            Some(goal) => {
                let before = proof_status.peek().clone();
                proof_status.set(ProofStatus::Proving);
                let shown = attempt(fault, "Prove", || auto_view(&kb, &goal, logic_str.as_deref().unwrap_or("(no expression)")));
                match shown {
                    Some(view) => {
                        // A sentence with no givens proves only if it is a logical truth: say what
                        // was missing, not just that the search failed.
                        let hint = if kb.is_empty() && matches!(view.status, ProofStatus::Failed(_)) {
                            "No givens to prove from. Add Given lines in a ## Theorem: block, as prover-demo.logic does.".to_string()
                        } else {
                            view.hint
                        };
                        proof_text.set(view.html);
                        proof_tree.set(view.tree);
                        proof_status.set(view.status);
                        proof_hint.set(Some(hint));
                    }
                    // Prove faulted: the proof drawer keeps what it showed before.
                    None => proof_status.set(before),
                }
            }
            None => {
                proof_status.set(ProofStatus::Failed("No logic expression to prove. Enter a sentence first.".to_string()));
                proof_text.set(String::new());
                proof_tree.set(None);
                proof_hint.set(Some("Enter an English sentence above to generate a logical formula.".to_string()));
            }
        }
    });

    // Hardware mode handlers: Execute reads the design again; Compile makes its Rust monitor, or
    // says why the design has none.
    let handle_hardware_execute = move |_| {
        hw_report.set(analyze_hardware(&hw_input.read()));
        hw_compiled.set(None);
        hw_output_mode.set(CodeOutputMode::Interpret);
    };

    let handle_hardware_compile = move |_| match compile_hardware(&hw_input.read()) {
        Compiled::Rust(rust) => {
            generated_hw_rust.set(rust);
            hw_compiled.set(None);
            hw_output_mode.set(CodeOutputMode::Rust);
        }
        said => {
            generated_hw_rust.set(String::new());
            hw_compiled.set(Some(said));
            hw_output_mode.set(CodeOutputMode::Interpret);
        }
    };

    // Logic mode guide
    let current_result = result.read();
    let guide_mode = if *mode.read() == StudioMode::Logic {
        logic_guide(
            current_result.error.as_deref(),
            current_result.logic.as_deref(),
            current_result.readings.len(),
            &input.read(),
            &proof_status.read(),
        )
    } else {
        GuideMode::Idle
    };

    // Determine if Panel 3 should be shown based on mode and content. Hardware mode's panels all
    // follow the one reading in `hw_report`.
    let hw_kind = hw_report.read().as_ref().map(|(kind, _)| *kind);
    let show_panel3 = match *mode.read() {
        StudioMode::Code => interpreter_result.read().error.is_some(),
        StudioMode::Math => !math_context.read().is_empty(),
        StudioMode::Hardware => match hw_report.read().as_ref() {
            Some((HardwareKind::Pigeonhole | HardwareKind::RegisterAlloc, report)) => {
                report.error.is_none()
            }
            Some((_, report)) => {
                !report.signals.is_empty()
                    || !report.trace.is_empty()
                    || !report.kg.nodes.is_empty()
            }
            None => false,
        },
        // Logic keeps its Syntax Tree in a block of the Logic Output column, and Web mode is two
        // panels — source + live preview — so neither has a third panel.
        StudioMode::Logic | StudioMode::Web => false,
    };

    let shown = Shown {
        sidebar: *sidebar_open.read() && !*drawer.read(),
        right: show_panel3,
    };
    // A frame's row is as wide as the frame, not the window: measured once it is on the page and
    // again on every resize (`frame_row_width`); on the server, and until then, the window stands in.
    let viewport = match (host, *frame_row_width.read()) {
        (Host::Frame, Some(width)) => width,
        _ => *viewport_width.read(),
    };
    let columns = column_pref
        .read()
        .unwrap_or_else(|| Columns::initial(viewport, shown))
        .fit(viewport, shown);
    // The Studio page draws the columns the reader shaped, fitted to the window. A frame lays them out
    // in its own stylesheet until the reader drags a bar, so its first paint depends on nothing
    // measured; from that drag on it draws them as the page does, fitted to its own width.
    let columns_style = (host == Host::Page || column_pref.read().is_some()).then(|| {
        format!(
            "--col-sidebar: {}px; --col-left: {}px; --col-right: {}px;",
            columns.sidebar, columns.left, columns.right
        )
    });

    // A column bar paints each frame of its drag straight onto the row's custom properties and
    // commits the layout once, on release (see ui::drag) — one render per drag, not one per
    // pointer event.
    let start_column_drag = move |evt: PointerEvent, resizer: Resizer| {
        #[cfg(target_arch = "wasm32")]
        {
            use dioxus::html::input_data::MouseButton;
            use wasm_bindgen::JsCast;
            let mut column_pref = column_pref;
            if evt
                .data()
                .trigger_button()
                .is_some_and(|b| b != MouseButton::Primary)
            {
                return;
            }
            evt.prevent_default();
            let Some(document) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let find = |selector: &str| document.query_selector(selector).ok().flatten();
            let Some(handle) = pressed(&evt, "[data-resizer]")
                .or_else(|| find(&format!("[data-resizer=\"{}\"]", resizer.attr())))
            else {
                return;
            };
            let Some(row) = handle
                .closest(".studio-content")
                .ok()
                .flatten()
                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
            else {
                return;
            };
            let point = evt.data().client_coordinates();
            let _ = handle.class_list().add_1("active");
            let released = handle.clone();
            crate::ui::drag::begin(
                handle,
                evt.data().pointer_id(),
                (point.x, point.y),
                crate::ui::drag::Axis::Columns,
                move |dx, _| {
                    let c = columns.dragged(resizer, dx, viewport, shown);
                    let style = row.style();
                    let _ = style.set_property("--col-sidebar", &format!("{}px", c.sidebar));
                    let _ = style.set_property("--col-left", &format!("{}px", c.left));
                    let _ = style.set_property("--col-right", &format!("{}px", c.right));
                },
                move |dx, _| {
                    let _ = released.class_list().remove_1("active");
                    column_pref.set(Some(columns.dragged(resizer, dx, viewport, shown)));
                },
            );
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (evt, resizer);
    };

    // On a phone the panels stack, and the bar between them drags the same way, one axis over.
    let start_split_drag = move |evt: PointerEvent| {
        #[cfg(target_arch = "wasm32")]
        {
            use crate::ui::pages::studio_layout::split_percent;
            use wasm_bindgen::JsCast;
            let mut mobile_split = mobile_split;
            evt.prevent_default();
            let Some(document) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let find = |selector: &str| document.query_selector(selector).ok().flatten();
            let handle = pressed(&evt, ".mobile-panel-resizer").or_else(|| find(".mobile-panel-resizer"));
            let stack = handle.as_ref().and_then(|handle| handle.closest(".studio-main").ok().flatten());
            let panel = |selector: &str| {
                stack
                    .as_ref()
                    .and_then(|stack| stack.query_selector(selector).ok().flatten())
                    .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
            };
            let (Some(handle), Some(upper), Some(lower)) =
                (handle, panel(":scope > .col-left"), panel(":scope > .col-center"))
            else {
                return;
            };
            let Some(stack) = stack else { return };
            let rect = stack.get_bounding_client_rect();
            let (top, height) = (rect.top(), rect.height());
            let point = evt.data().client_coordinates();
            let _ = handle.class_list().add_1("active");
            let released = handle.clone();
            crate::ui::drag::begin(
                handle,
                evt.data().pointer_id(),
                (point.x, point.y),
                crate::ui::drag::Axis::Rows,
                move |_, dy| {
                    let pct = split_percent(point.y + dy, top, height);
                    let _ = upper
                        .style()
                        .set_property("--panel-flex", &format!("{}", pct / 50.0));
                    let _ = lower
                        .style()
                        .set_property("--panel-flex", &format!("{}", (100.0 - pct) / 50.0));
                },
                move |_, dy| {
                    let _ = released.class_list().remove_1("active");
                    mobile_split.set(split_percent(point.y + dy, top, height));
                },
            );
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = evt;
    };

    let current_format = *format.read();
    let current_mode = *mode.read();
    // The pane a phone shows: the one its switch names, the output when it names a third panel the
    // file does not have.
    let current_tab = match *active_tab.read() {
        MobileTab::Panel3 if !show_panel3 => MobileTab::Panel2,
        tab => tab,
    };
    let on_phone = |tab: MobileTab| if current_tab == tab { " phone-shown" } else { "" };
    // Where the panes share the screen with the editor (a phone on its side, an upright tablet), the
    // other half shows the output or, when the switch names it, the third panel.
    let right_half = if current_tab == MobileTab::Panel3 { MobileTab::Panel3 } else { MobileTab::Panel2 };
    let on_right = |tab: MobileTab| if right_half == tab { " phone-right" } else { "" };

    // Panel classes: mobile-expanded/collapsed for the stacked layout, phone-shown for the one pane a
    // phone shows; the desktop ignores them.
    let editor_exp = *editor_expanded.read();
    let output_exp = *output_expanded.read();
    let panel1_class = format!("studio-panel {}{}", if editor_exp { "mobile-expanded" } else { "mobile-collapsed" }, on_phone(MobileTab::Panel1));
    let output_fold = if output_exp { "mobile-expanded" } else { "mobile-collapsed" };
    let panel2_class = format!("studio-panel {output_fold}{}{}", on_phone(MobileTab::Panel2), on_right(MobileTab::Panel2));
    let panel3_class = format!("studio-panel {output_fold}{}{}", on_phone(MobileTab::Panel3), on_right(MobileTab::Panel3));

    // Mobile vertical split: compute panel flex proportions
    let mobile_pct = *mobile_split.read();
    let panel1_flex = mobile_pct / 50.0;
    let panel2_flex = (100.0 - mobile_pct) / 50.0;
    let both_expanded = editor_exp && output_exp;

    let panel1_style = if both_expanded {
        format!("--panel-flex: {panel1_flex};")
    } else {
        String::new()
    };
    let panel2_style = if both_expanded {
        format!("--panel-flex: {panel2_flex};")
    } else {
        String::new()
    };

    // Read output view modes for rendering
    let current_code_output_mode = *code_output_mode.read();
    let current_math_output_mode = *math_output_mode.read();
    let current_logic_output_mode = *logic_output_mode.read();
    let current_hw_output_mode = *hw_output_mode.read();

    // Mobile tab labels based on mode
    let (tab1_icon, tab1_label, tab2_icon, tab2_label, tab3_icon, tab3_label) = match current_mode {
        StudioMode::Logic => ("\u{270F}", "Input", "\u{2200}", "Logic", "", ""),
        StudioMode::Code => (
            "\u{03BB}",
            "Editor",
            "\u{276F}",
            "Output",
            "\u{1F4CB}",
            "Console",
        ),
        StudioMode::Math => (
            "\u{2200}",
            "Editor",
            "\u{276F}",
            "Output",
            "\u{1F4CB}",
            "Context",
        ),
        StudioMode::Hardware => ("\u{270F}", "Spec", "\u{22A8}", "SVA", "\u{25A6}", "Graph"),
        StudioMode::Web => ("\u{03BB}", "Source", "\u{25C9}", "Preview", "", ""),
    };

    // The status bar names the editor's language and says how it indents; Hardware by what its
    // text is (Verilog or a spec).
    let (status_language, status_source) = match current_mode {
        StudioMode::Logic => (Language::Logic, String::new()),
        StudioMode::Code | StudioMode::Web => (Language::Logos, String::new()),
        StudioMode::Math => (Language::Vernacular, String::new()),
        StudioMode::Hardware => (Language::Hardware, hw_input.read().clone()),
    };

    // Where a first visit stands, for the proof drawer's walkthrough and the dock bar's line.
    let walkthrough = Walkthrough { wrote: !input.read().trim().is_empty(), executed: result.read().logic.is_some() };
    let dock_text = match current_mode {
        StudioMode::Logic => dock_prompt(walkthrough, proof_hint.read().as_deref()),
        StudioMode::Code if debugging() => "Stepping through the program, one instruction at a time.".to_string(),
        StudioMode::Code => "Debug runs the program one instruction at a time, every variable in view.".to_string(),
        _ => String::new(),
    };
    // LLM Heal: the fairy mends the editor's English (work/LLM_HEAL.md). Code and Logic only; the
    // worker answers, the card shows both texts, and nothing lands in the editor until Keep.
    let heal_source: Signal<String> = if current_mode == StudioMode::Logic { input } else { code_input };
    let heal_mode = if current_mode == StudioMode::Logic { heal::Mode::Logic } else { heal::Mode::Code };
    let handle_heal = use_callback(move |_: ()| {
        let text = heal_source.read().clone();
        let begun = heal::Flight::begin(&text);
        let flying = begun.busy();
        heal.set(begun);
        if !flying {
            return;
        }
        // Only what the reader wrote since the file opened, or since the last Keep.
        let baseline = current_file.peek().as_deref().and_then(|path| heal_baseline.peek().get(path).cloned()).unwrap_or_default();
        let regions = heal::edited_regions(&baseline, &text);
        if regions.is_empty() {
            heal.set(heal::Flight::Failed(heal::NOTHING_NEW.to_string()));
            return;
        }
        heal.set(heal::Flight::Flying { to: regions[0].start });
        spawn(async move {
            let mut mended = Vec::with_capacity(regions.len());
            for region in &regions {
                let fragment = heal::region_text(&text, region);
                match heal::send(heal_mode, &fragment, true).await {
                    Ok(lines) => mended.push(heal::reindent(&fragment, &lines)),
                    Err(reason) => {
                        heal.set(heal::Flight::Failed(reason));
                        return;
                    }
                }
            }
            let after = heal::splice(&text, &regions, &mended);
            heal.set(heal::Flight::answered(text, Ok(after)));
        });
    });
    let heal_button = move || {
        let busy = heal.read().busy();
        rsx! {
            button {
                class: "execute-btn heal-btn",
                disabled: busy,
                title: "Mend the English in the editor, then keep or undo the mend",
                onclick: move |_| handle_heal.call(()),
                img { class: "heal-btn-icon", src: heal::FAIRY, alt: "" }
                span { class: "desktop-label", "Heal" }
                span { class: "mobile-label", "Heal" }
            }
        }
    };

    let dock_text = heal::narration(&heal.read()).unwrap_or(dock_text);

    let panels = rsx! {
        // Panel 1
        section {
            class: "{panel1_class} col-left",
            style: "{panel1_style}",

            // LLM Heal: the fairy and her card, over the editor, while a heal is under way.
            if matches!(current_mode, StudioMode::Code | StudioMode::Logic | StudioMode::Web) {
                HealOverlay { flight: heal, editor: heal_source, file: current_file, baseline: heal_baseline }
            }

            match current_mode {
                StudioMode::Logic => rsx! {
                    div {
                        class: "panel-header",
                        onclick: move |_| {
                            if !*panes_fixed.peek() {
                                let v = *editor_expanded.read();
                                editor_expanded.set(!v);
                            }
                        },
                        span { "English Input" }
                    }
                    div { class: "panel-content",
                        CodeEditor {
                            value: input.read().clone(),
                            on_change: handle_logic_input,
                            language: Language::Logic,
                            language_server: true,
                            status: editor_status,
                            placeholder: "Type an English sentence...".to_string(),
                        }
                    }
                },
                // Web mode edits LOGOS source in the same editor as Code mode.
                StudioMode::Code | StudioMode::Web => rsx! {
                    div {
                        class: "panel-header",
                        onclick: move |_| {
                            if !*panes_fixed.peek() {
                                let v = *editor_expanded.read();
                                editor_expanded.set(!v);
                            }
                        },
                        span { "Code Editor" }
                    }
                    div { class: "panel-content",
                        CodeEditor {
                            value: code_input.read().clone(),
                            on_change: move |v| code_input.set(v),
                            language: Language::Logos,
                            language_server: true,
                            status: editor_status,
                            placeholder: "-- Imperative LOGOS code\n\n## Main\n\nLet x be 1.\nLet y be 2.\nShow x + y.".to_string(),
                        }
                    }
                },
                StudioMode::Math => rsx! {
                    div {
                        class: "panel-header",
                        onclick: move |_| {
                            if !*panes_fixed.peek() {
                                let v = *editor_expanded.read();
                                editor_expanded.set(!v);
                            }
                        },
                        span { "Theorem Editor" }
                    }
                    div { class: "panel-content",
                        CodeEditor {
                            value: math_input.read().clone(),
                            on_change: move |v| math_input.set(v),
                            language: Language::Vernacular,
                            status: editor_status,
                            placeholder: "-- Define natural numbers\nInductive Nat := Zero : Nat | Succ : Nat -> Nat.\n\nDefinition one : Nat := Succ Zero.\n\nCheck one.".to_string(),
                        }
                    }
                },
                StudioMode::Hardware => rsx! {
                    div {
                        class: "panel-header",
                        onclick: move |_| {
                            if !*panes_fixed.peek() {
                                let v = *editor_expanded.read();
                                editor_expanded.set(!v);
                            }
                        },
                        span { "Hardware Spec" }
                    }
                    div { class: "panel-content",
                        CodeEditor {
                            value: hw_input.read().clone(),
                            on_change: move |v| hw_input.set(v),
                            language: Language::Hardware,
                            language_server: true,
                            status: editor_status,
                            placeholder: "Describe hardware in English (\"Always, if request is high, then acknowledge is high.\"), design a signal plan (\"NS-left conflicts with the EW crossing.\"), or paste Verilog (module \u{2026} endmodule) to model-check.".to_string(),
                        }
                    }
                },
            }
        }

        // Mobile vertical resizer between Panel 1 and Panel 2
        if both_expanded {
            div {
                class: "mobile-panel-resizer",
                onpointerdown: move |evt| start_split_drag(evt),
            }
        }

        div {
            class: "panel-resizer",
            "data-resizer": Resizer::LeftCenter.attr(),
            onpointerdown: move |evt| start_column_drag(evt, Resizer::LeftCenter),
        }

        // Panel 2
        section {
            class: "{panel2_class} col-center",
            style: "{panel2_style}",

            match current_mode {
                StudioMode::Logic => rsx! {
                    div {
                        class: "panel-header",
                        onclick: move |_| {
                            if !*panes_fixed.peek() {
                                let v = *output_expanded.read();
                                output_expanded.set(!v);
                            }
                        },
                        span { "Logic Output" }
                        div {
                            class: "output-mode-toggle",
                            onclick: move |evt| evt.stop_propagation(),
                            button {
                                class: if current_logic_output_mode == LogicView::Logic { "output-mode-btn active" } else { "output-mode-btn" },
                                onclick: move |_| logic_output_mode.set(LogicView::Logic),
                                "Logic"
                            }
                            button {
                                class: if current_logic_output_mode == LogicView::Rust { "output-mode-btn active" } else { "output-mode-btn" },
                                onclick: move |_| logic_output_mode.set(LogicView::Rust),
                                "Rust"
                            }
                        }
                        if current_logic_output_mode == LogicView::Logic {
                            div {
                                class: "format-toggle",
                                onclick: move |evt| evt.stop_propagation(),
                                button {
                                    class: if current_format == OutputFormat::SimpleFOL { "format-btn active" } else { "format-btn" },
                                    onclick: move |_| format.set(OutputFormat::SimpleFOL),
                                    "Simple"
                                }
                                button {
                                    class: if current_format == OutputFormat::Unicode { "format-btn active" } else { "format-btn" },
                                    onclick: move |_| format.set(OutputFormat::Unicode),
                                    "Full"
                                }
                                button {
                                    class: if current_format == OutputFormat::LaTeX { "format-btn active" } else { "format-btn" },
                                    onclick: move |_| format.set(OutputFormat::LaTeX),
                                    "LaTeX"
                                }
                                button {
                                    class: if current_format == OutputFormat::Kripke { "format-btn active" } else { "format-btn" },
                                    onclick: move |_| format.set(OutputFormat::Kripke),
                                    "Deep"
                                }
                            }
                        }
                    }
                    div { class: "panel-content",
                        if current_logic_output_mode == LogicView::Logic {
                            {
                                rsx! {
                                    LogicOutput {
                                        logic: current_result.logic.clone(),
                                        simple_logic: current_result.simple_logic.clone(),
                                        kripke_logic: current_result.kripke_logic.clone(),
                                        readings: current_result.readings.clone(),
                                        simple_readings: current_result.simple_readings.clone(),
                                        kripke_readings: current_result.kripke_readings.clone(),
                                        error: current_result.error.clone(),
                                        format: current_format,
                                    }
                                    // Under the formula, what it is made of: its symbols, then its tree.
                                    if let Some(symbols) = dictionary() {
                                        Collapsible {
                                            title: "Symbol Dictionary",
                                            open: dictionary_open,
                                            SymbolGroups { symbols }
                                        }
                                    }
                                    if let Some(ast) = current_result.ast.clone() {
                                        Collapsible {
                                            title: "Syntax Tree",
                                            open: tree_open,
                                            AstTree { ast }
                                        }
                                    }
                                }
                            }
                        } else {
                            {
                                let rust_code = generated_logic_rust.read().clone();
                                rsx! {
                                    if rust_code.is_empty() {
                                        div { class: "interpreter-empty",
                                            "Compile to see generated Rust"
                                        }
                                    } else {
                                        CodeView {
                                            code: rust_code,
                                            language: Language::Rust,
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                StudioMode::Code => rsx! {
                    div {
                        class: "panel-header",
                        onclick: move |_| {
                            if !*panes_fixed.peek() {
                                let v = *output_expanded.read();
                                output_expanded.set(!v);
                            }
                        },
                        span { "Output" }
                        div {
                            class: "output-mode-toggle",
                            onclick: move |evt| evt.stop_propagation(),
                            button {
                                class: if current_code_output_mode == CodeOutputMode::Interpret { "output-mode-btn active" } else { "output-mode-btn" },
                                onclick: move |_| code_output_mode.set(CodeOutputMode::Interpret),
                                "Output"
                            }
                            button {
                                class: if current_code_output_mode == CodeOutputMode::Rust { "output-mode-btn active" } else { "output-mode-btn" },
                                onclick: move |_| code_output_mode.set(CodeOutputMode::Rust),
                                "Rust"
                            }
                        }
                    }
                    div { class: "panel-content",
                        if current_code_output_mode == CodeOutputMode::Interpret {
                            {
                                // While the debugger is open the column follows it: what the program
                                // has printed at the cursor, not the last Run.
                                let stepping = debugging();
                                let result = if stepping { debug_output.read() } else { interpreter_result.read() };
                                let runtime_notice = current_file.read().as_deref().and_then(compiled_runtime_notice);
                                rsx! {
                                    div { class: "interpreter-output",
                                        if let Some(notice) = &runtime_notice {
                                            div { class: "panel-verdict", "{notice}" }
                                        }
                                        if result.lines.is_empty() && result.error.is_none() {
                                            if stepping {
                                                div { class: "interpreter-empty",
                                                    "Nothing printed yet. What the program shows appears here."
                                                }
                                            } else if runtime_notice.is_none() {
                                                div { class: "interpreter-empty",
                                                    "Run your code to see output"
                                                }
                                            }
                                        } else {
                                            for line in result.lines.iter() {
                                                div { class: "interpreter-line", "{line}" }
                                            }
                                            if let Some(ref err) = result.error {
                                                div { class: "interpreter-error", "{err}" }
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            {
                                let rust_code = generated_rust.read().clone();
                                rsx! {
                                    if rust_code.is_empty() {
                                        div { class: "interpreter-empty",
                                            "Compile your code to see generated Rust"
                                        }
                                    } else {
                                        CodeView {
                                            code: rust_code,
                                            language: Language::Rust,
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                StudioMode::Math => rsx! {
                    div {
                        class: "panel-header",
                        onclick: move |_| {
                            if !*panes_fixed.peek() {
                                let v = *output_expanded.read();
                                output_expanded.set(!v);
                            }
                        },
                        span { "Output" }
                        div {
                            class: "panel-header-actions",
                            onclick: move |evt| evt.stop_propagation(),
                            if current_math_output_mode == CodeOutputMode::Interpret && !math_output.read().is_empty() {
                                div { class: "output-mode-toggle",
                                    button {
                                        class: "output-mode-btn",
                                        onclick: move |_| {
                                            math_output.write().clear();
                                            // The Context panel goes back to the bare standard library too.
                                            math_context.set(MathContext::prelude());
                                        },
                                        "Clear"
                                    }
                                }
                            }
                            div { class: "output-mode-toggle",
                                button {
                                    class: if current_math_output_mode == CodeOutputMode::Interpret { "output-mode-btn active" } else { "output-mode-btn" },
                                    onclick: move |_| math_output_mode.set(CodeOutputMode::Interpret),
                                    "Output"
                                }
                                button {
                                    class: if current_math_output_mode == CodeOutputMode::Rust { "output-mode-btn active" } else { "output-mode-btn" },
                                    onclick: move |_| math_output_mode.set(CodeOutputMode::Rust),
                                    "Rust"
                                }
                            }
                        }
                    }
                    div { class: "panel-content",
                        if current_math_output_mode == CodeOutputMode::Interpret {
                            ReplOutput {
                                lines: math_output.read().clone(),
                            }
                        } else {
                            {
                                let rust_code = generated_math_rust.read().clone();
                                rsx! {
                                    if rust_code.is_empty() {
                                        div { class: "interpreter-empty",
                                            "Compile to see generated Rust"
                                        }
                                    } else {
                                        CodeView {
                                            code: rust_code,
                                            language: Language::Rust,
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                StudioMode::Hardware => {
                    let report = hw_report.read().clone();
                    let title = hw_kind.map(HardwareKind::result_title).unwrap_or("SVA");
                    let rust_view = hw_kind.map_or(true, HardwareKind::compiles);
                    let planted = current_file.read().as_deref().and_then(planted_bug_notice);
                    let compiled = hw_compiled.read().clone();
                    rsx! {
                        div {
                            class: "panel-header",
                            onclick: move |_| {
                                if !*panes_fixed.peek() {
                                    let v = *output_expanded.read();
                                    output_expanded.set(!v);
                                }
                            },
                            span { "{title}" }
                            if rust_view {
                                div {
                                    class: "output-mode-toggle",
                                    onclick: move |evt| evt.stop_propagation(),
                                    button {
                                        class: if current_hw_output_mode == CodeOutputMode::Interpret { "output-mode-btn active" } else { "output-mode-btn" },
                                        onclick: move |_| hw_output_mode.set(CodeOutputMode::Interpret),
                                        "SVA"
                                    }
                                    button {
                                        class: if current_hw_output_mode == CodeOutputMode::Rust { "output-mode-btn active" } else { "output-mode-btn" },
                                        onclick: move |_| hw_output_mode.set(CodeOutputMode::Rust),
                                        "Rust"
                                    }
                                }
                            }
                        }
                        div { class: "panel-content",
                            if current_hw_output_mode == CodeOutputMode::Rust && rust_view {
                                {
                                    let rust_code = generated_hw_rust.read().clone();
                                    rsx! {
                                        if rust_code.is_empty() {
                                            div { class: "interpreter-empty",
                                                "Compile to see the Rust runtime monitor"
                                            }
                                        } else {
                                            CodeView {
                                                code: rust_code,
                                                language: Language::Rust,
                                            }
                                        }
                                    }
                                }
                            } else {
                                div { class: "interpreter-output",
                                    match compiled {
                                        Some(Compiled::Nothing(note)) => rsx! { div { class: "panel-verdict", "{note}" } },
                                        Some(Compiled::Failed(e)) => rsx! { div { class: "interpreter-error", "{e}" } },
                                        Some(Compiled::Rust(_)) | None => rsx! {},
                                    }
                                    if let Some(notice) = &planted {
                                        div { class: "panel-verdict", "{notice}" }
                                    }
                                    match report {
                                        None => rsx! {
                                            div { class: "interpreter-empty",
                                                "Execute to analyze: synthesize an assertion, design a signal plan, or model-check Verilog"
                                            }
                                        },
                                        Some((_, report)) => rsx! {
                                            if let Some(e) = &report.error {
                                                div { class: "interpreter-error", "{e}" }
                                            }
                                            for section in report.sections.iter() {
                                                div { class: "panel-section-label", "{section.label}" }
                                                pre { class: "panel-code", "{section.body}" }
                                            }
                                            if let Some((text, ok)) = &report.verdict {
                                                div {
                                                    class: match ok {
                                                        Some(true) => "panel-verdict proved",
                                                        Some(false) => "panel-verdict refuted",
                                                        None => "panel-verdict",
                                                    },
                                                    "{text}"
                                                }
                                            }
                                        },
                                    }
                                }
                            }
                        }
                    }
                },
                // Web mode: the LIVE PREVIEW — a sandboxed iframe rendering the
                // in-browser-compiled reactive app (or the themed build overlay).
                StudioMode::Web => {
                    let doc = web_preview.read().clone();
                    rsx! {
                        div { class: "panel-header", span { "Preview" } }
                        div { class: "panel-content", style: "padding: 0; overflow: hidden;",
                            iframe {
                                title: PREVIEW_TITLE,
                                srcdoc: "{doc}",
                                "sandbox": "allow-scripts",
                                style: "width: 100%; height: 100%; border: 0; background: #faf9f5;",
                            }
                        }
                    }
                },
            }
        }

        // Right resizer and Panel 3 (only shown when there's content)
        if show_panel3 {
            div {
                class: "panel-resizer",
                "data-resizer": Resizer::CenterRight.attr(),
                onpointerdown: move |evt| start_column_drag(evt, Resizer::CenterRight),
            }

            aside {
                class: "{panel3_class} col-right",
                style: "{panel2_style}",

                match current_mode {
                    StudioMode::Code => rsx! {
                        div { class: "panel-header",
                            span { "Console" }
                        }
                        div { class: "panel-content",
                            div { class: "interpreter-output",
                                {
                                    let result = interpreter_result.read();
                                    rsx! {
                                        if let Some(ref err) = result.error {
                                            div { class: "interpreter-error", "{err}" }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    StudioMode::Math => rsx! {
                        div { class: "panel-header",
                            span { "Context" }
                        }
                        div { class: "panel-content",
                            ContextView {
                                definitions: math_context.read().definitions.clone(),
                                inductives: math_context.read().inductives.clone(),
                            }
                        }
                    },
                    // Logic and Web have no third panel (`show_panel3` is false).
                    StudioMode::Logic | StudioMode::Web => rsx! {},
                    StudioMode::Hardware => {
                        let report = hw_report.read().clone().map(|(_, report)| report).unwrap_or_default();
                        let violated = report.verdict.as_ref().is_some_and(|(_, ok)| *ok == Some(false));
                        if let (Some(HardwareKind::Pigeonhole), Some(pspec)) = (hw_kind, crate::ui::pages::pigeonhole_viz::parse_pigeonhole_spec(&hw_input.read())) {
                            // Pigeonhole easter egg: n pigeons fly into n-1 holes, one left
                            // out — solved live (matching Hall witness + certified symmetry
                            // breaking) by our prover in the browser, no Z3.
                            let (svg, caption) = crate::ui::pages::pigeonhole_viz::render(&pspec);
                            rsx! {
                                div { class: "panel-header", span { "Pigeonhole" } }
                                div { class: "panel-content",
                                    div { class: "panel-body wide",
                                        div { dangerous_inner_html: "{svg}" }
                                        div { class: "panel-caption", "{caption}" }
                                    }
                                }
                            }
                        } else if let (Some(HardwareKind::RegisterAlloc), Some(spec)) = (hw_kind, crate::ui::pages::register_alloc_viz::parse_register_spec(&hw_input.read())) {
                            // Register-allocation easter egg: live-range timeline coloured
                            // by the certified linear-scan allocation (spill clique in red).
                            let (svg, _) = crate::ui::pages::register_alloc_viz::render(&spec);
                            rsx! {
                                div { class: "panel-header", span { "Register Allocation" } }
                                div { class: "panel-content",
                                    div { class: "panel-body wide",
                                        div { dangerous_inner_html: "{svg}" }
                                        div { class: "panel-caption",
                                            "The sweep line is the program counter. Each register lane lights up while it holds a value and goes dark when it's freed. The pressure histogram counts how many values are live at each instruction, and the bars above the dashed budget line turn red: the exact points where spilling is forced and a value drops to the MEM lane. The interference graph is what allocation really colours: an edge joins two values whose lifetimes overlap, nodes are coloured by their register, and a red clique is a set that mutually conflicts and provably cannot share."
                                        }
                                    }
                                }
                            }
                        } else if !report.trace.is_empty() {
                            // A counterexample (or a proven design's witness run), drawn as a waveform.
                            let wf = logicaffeine_compile::codegen_sva::hw_pipeline::counterexample_waveform(&report.trace);
                            let traffic = traffic_svg(&wf);
                            let osc = waveform_svg(&wf);
                            let header = match (traffic.is_some(), violated) {
                                (true, true) => "Intersection: Conflict Found",
                                (true, false) => "Intersection: Proven Safe (witness run)",
                                (false, true) => "Counterexample Waveform",
                                (false, false) => "Witness Waveform",
                            };
                            rsx! {
                                div { class: "panel-header", span { "{header}" } }
                                div { class: "panel-content",
                                    div { class: "panel-body wide",
                                        if let Some(t) = traffic.as_ref() {
                                            div { dangerous_inner_html: "{t}" }
                                        }
                                        div { dangerous_inner_html: "{osc}" }
                                    }
                                }
                            }
                        } else {
                            let svg = kg_svg(&report.kg);
                            rsx! {
                                div { class: "panel-header", span { if svg.is_empty() { "Signals" } else { "Knowledge Graph" } } }
                                div { class: "panel-content",
                                    if !svg.is_empty() {
                                        div { class: "panel-body",
                                            div { dangerous_inner_html: "{svg}" }
                                            div { style: "display:flex; gap:12px; flex-wrap:wrap; justify-content:center; margin-top:4px; font-size:10px;",
                                                span { style: "color:#60a5fa;", "● input" }
                                                span { style: "color:#4ade80;", "● output" }
                                                span { style: "color:#a78bfa;", "● internal" }
                                                span { style: "color:#fbbf24;", "● clock" }
                                            }
                                        }
                                    } else if !report.signals.is_empty() {
                                        div { class: "panel-body",
                                            for s in report.signals.iter() {
                                                div { style: "font-family: ui-monospace, monospace; font-size: 13px; color: #e5e7eb; padding: 2px 0;", "{s}" }
                                            }
                                        }
                                    } else {
                                        div { class: "interpreter-empty",
                                            "Execute to extract the hardware knowledge graph"
                                        }
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    };

    // What a touch screen cannot hover to read: each action the toolbar shows disabled, with why.
    let withheld: Vec<(&'static str, String)> = {
        let actions = actions();
        let run = if current_mode == StudioMode::Code { "Run" } else { "Execute" };
        let mut shown = vec![(run, &actions.run), ("Compile", &actions.compile)];
        if current_mode == StudioMode::Code && host == Host::Page {
            shown.push(("Debug", &actions.debug));
        }
        if current_mode == StudioMode::Web {
            shown.clear();
        }
        shown.into_iter().filter_map(|(name, offer)| offer.reason().map(|why| (name, why.to_string()))).collect()
    };
    let inset = *keyboard_inset.read();

    rsx! {
        div { class: "studio-workbench", style: "--keyboard-inset: {inset}px;",
        div { class: "studio-toolbar",
            div { class: "studio-toolbar-left",
                {
                    let open = if *drawer.read() { *drawer_open.read() } else { *sidebar_open.read() };
                    let label = if open { "Hide files" } else { "Show files" };
                    rsx! {
                        button {
                            class: "sidebar-toggle-btn",
                            onclick: move |_| if *drawer.peek() { drawer_open.set(!open) } else { sidebar_open.set(!open) },
                            title: "{label}",
                            aria_label: "{label}",
                            aria_expanded: "{open}",
                            span {
                                class: "toggle-icon",
                                dangerous_inner_html: if open { COLLAPSE_FILES_SVG } else { FILES_SVG },
                            }
                        }
                        // Folded, the square button says what it opens.
                        if !open {
                            span { class: "files-label", "Files" }
                        }
                    }
                }
            }
            div { class: "studio-toolbar-right",
                if let Some(stopped) = fault() {
                    span {
                        class: "studio-fault",
                        role: "alert",
                        title: "{stopped.detail()}",
                        onclick: move |_| fault_open.set(!fault_open()),
                        "\u{26A0} {stopped.headline()}"
                    }
                }
                if current_mode == StudioMode::Code {
                    button {
                        class: "execute-btn",
                        disabled: !actions().run.is_ready(),
                        title: actions().run.reason().map(str::to_string),
                        onclick: handle_code_run,
                        span { class: "desktop-label", "\u{25B6} Run" }
                        span { class: "mobile-label", "\u{25B6} Run" }
                    }
                    {heal_button()}
                    button {
                        class: "execute-btn compile-btn",
                        style: "background: linear-gradient(135deg, #56b6c2 0%, #61afef 100%);",
                        disabled: !actions().compile.is_ready(),
                        title: actions().compile.reason().map(str::to_string),
                        onclick: handle_code_compile,
                        span { class: "desktop-label", "\u{1F980} Compile" }
                        span { class: "mobile-label", "\u{1F980} Compile to Rust" }
                    }
                    if host == Host::Page {
                        button {
                            class: "execute-btn",
                            style: "background: linear-gradient(135deg, #f59e0b 0%, #ef4444 100%); display:inline-flex; align-items:center; gap:6px;",
                            disabled: !actions().debug.is_ready(),
                            title: actions().debug.reason().map(str::to_string),
                            onclick: move |_| {
                                code_output_mode.set(CodeOutputMode::Interpret);
                                debugging.set(true);
                            },
                            span { style: "display:inline-flex;", dangerous_inner_html: IC_BUG }
                            span { class: "desktop-label", "Debug" }
                            span { class: "mobile-label", "Debug" }
                        }
                    }
                }
                if current_mode == StudioMode::Math {
                    button {
                        class: "execute-btn",
                        disabled: !actions().run.is_ready(),
                        title: actions().run.reason().map(str::to_string),
                        onclick: handle_math_execute,
                        span { class: "desktop-label", "\u{25B6} Execute" }
                        span { class: "mobile-label", "\u{25B6} Execute" }
                    }
                    button {
                        class: "execute-btn compile-btn",
                        style: "background: linear-gradient(135deg, #56b6c2 0%, #61afef 100%);",
                        disabled: !actions().compile.is_ready(),
                        title: actions().compile.reason().map(str::to_string),
                        onclick: handle_math_compile,
                        span { class: "desktop-label", "\u{1F980} Compile" }
                        span { class: "mobile-label", "\u{1F980} Compile to Rust" }
                    }
                }
                if current_mode == StudioMode::Logic {
                    button {
                        class: "execute-btn",
                        disabled: !actions().run.is_ready(),
                        title: actions().run.reason().map(str::to_string),
                        onclick: handle_logic_execute,
                        span { class: "desktop-label", "\u{25B6} Execute" }
                        span { class: "mobile-label", "\u{25B6} Execute" }
                    }
                    {heal_button()}
                    button {
                        class: "execute-btn compile-btn",
                        style: "background: linear-gradient(135deg, #56b6c2 0%, #61afef 100%);",
                        disabled: !actions().compile.is_ready(),
                        title: actions().compile.reason().map(str::to_string),
                        onclick: handle_logic_compile,
                        span { class: "desktop-label", "\u{1F980} Compile" }
                        span { class: "mobile-label", "\u{1F980} Compile to Rust" }
                    }
                }
                if current_mode == StudioMode::Hardware {
                    button {
                        class: "execute-btn",
                        disabled: !actions().run.is_ready(),
                        title: actions().run.reason().map(str::to_string),
                        onclick: handle_hardware_execute,
                        span { class: "desktop-label", "\u{25B6} Execute" }
                        span { class: "mobile-label", "\u{25B6} Execute" }
                    }
                    button {
                        class: "execute-btn compile-btn",
                        style: "background: linear-gradient(135deg, #56b6c2 0%, #61afef 100%);",
                        disabled: !actions().compile.is_ready(),
                        title: actions().compile.reason().map(str::to_string),
                        onclick: handle_hardware_compile,
                        span { class: "desktop-label", "\u{1F980} Compile" }
                        span { class: "mobile-label", "\u{1F980} Compile to Rust" }
                    }
                }
                // A web page is LOGOS code with no Run of its own: Heal is its one action.
                if current_mode == StudioMode::Web {
                    {heal_button()}
                }
                // A frame on another page leads into the Studio, on the file it shows.
                if host == Host::Frame {
                    if let Some(open) = current_file.read().clone() {
                        Link { to: studio_file_url(&open), class: "studio-open", "Open in Studio" }
                    }
                }
            }
        }

        // Where nothing hovers, a disabled action's reason and a fault's whole story are lines of their
        // own under the toolbar.
        if let Some(stopped) = fault().filter(|_| fault_open()) {
            div { class: "studio-fault-detail", "{stopped.detail()}" }
        }
        if !withheld.is_empty() {
            div { class: "studio-reasons",
                for (name, why) in withheld {
                    span { key: "{name}", "{name}: {why}" }
                }
            }
        }

        // A phone's switch between its panes, one shown at a time.
        nav { class: if show_panel3 { "mobile-tabs has-third" } else { "mobile-tabs" }, role: "tablist", "aria-label": "Panes",
            for (tab, icon, label) in [(MobileTab::Panel1, tab1_icon, tab1_label), (MobileTab::Panel2, tab2_icon, tab2_label), (MobileTab::Panel3, tab3_icon, tab3_label)] {
                if tab != MobileTab::Panel3 || show_panel3 {
                    button {
                        class: if current_tab == tab { "mobile-tab active" } else { "mobile-tab" },
                        role: "tab",
                        "aria-selected": if current_tab == tab { "true" } else { "false" },
                        onclick: move |_| active_tab.set(tab),
                        span { class: "mobile-tab-icon", "aria-hidden": "true", "{icon}" }
                        span { class: "mobile-tab-label", "{label}" }
                    }
                }
            }
        }

        // Socratic Guide (Logic mode only)
        if current_mode == StudioMode::Logic {
            div { class: "studio-guide",
                SocraticGuide {
                    mode: guide_mode.clone(),
                    on_hint_request: None,
                }
            }
        }

        // Main content with optional sidebar
        div { class: "studio-content", style: columns_style,
            // File browser sidebar
            // Mobile overlay to close sidebar when clicking outside
            if *sidebar_open.read() || *drawer_open.read() {
                div {
                    class: if *drawer_open.read() { "sidebar-overlay drawer-open" } else { "sidebar-overlay" },
                    onclick: move |_| drawer_open.set(false),
                }
            }

            if *sidebar_open.read() || *drawer_open.read() {
                div {
                    class: if *drawer_open.read() { "studio-sidebar drawer-open" } else { "studio-sidebar" },

                    // Show VFS error if any (critical errors only)
                    if let Some(err) = store.error.as_ref() {
                        div {
                            style: "background: #ff4444; color: white; padding: 6px 12px; font-size: 11px; border-radius: 0;",
                            "VFS Error: {err}"
                        }
                    }

                    FileBrowser {
                    show_private_mode: store.private_mode,
                    new_file_waits: store.new_file_waits.clone(),
                    tree: file_tree.read().clone(),
                    selected_path: current_file.read().clone(),
                    on_select: EventHandler::new(move |path: String| {
                        // Opening a shipped example is the workbench's, at once; the host keeps
                        // the address bar on it and reads a reader's own file from their store.
                        state.choose(path.clone());
                        if let Some(chosen) = on_choose {
                            chosen.call(path);
                        }
                    }),
                    on_toggle_dir: EventHandler::new(move |path: String| {
                        if let Some(node) = file_tree.write().find_mut(&path) {
                            node.toggle_expanded();
                        }
                    }),
                    on_new_file,
                }
                }
                if *sidebar_open.read() {
                    div {
                        class: "panel-resizer",
                        "data-resizer": Resizer::SidebarLeft.attr(),
                        onpointerdown: move |evt| start_column_drag(evt, Resizer::SidebarLeft),
                    }
                }
            }

            // The panels, content by mode. The panel row is the Studio page's main landmark; on another page it sits in that page's.
            if host == Host::Page {
                main { class: "studio-main", {panels} }
            } else {
                div { class: "studio-main", {panels} }
            }
        }

        // The drawers rise from the dock bar below them, on either host: Logic's proof and Code's
        // debugger, each bottom-docked and dragged by its grip.
        if current_mode == StudioMode::Logic && *proof_open.read() {
            ProofPanel {
                proof_text: proof_text.read().clone(),
                tree: proof_tree.read().clone(),
                status: proof_status.read().clone(),
                steps: walkthrough,
                height: proof_height,
                on_prove: move |_| handle_prove.call(()),
                on_close: move |_| proof_open.set(false),
            }
        }
        if debugging() && current_mode == StudioMode::Code {
            DebugDrawer {
                source: code_input.read().clone(),
                height: drawer_height,
                output: debug_output,
                on_close: move |_| debugging.set(false),
            }
        }
        // The dock bar: the one amber line along the bottom that says what to do next and holds
        // the way into the drawer above it.
        if current_mode == StudioMode::Logic {
            div { class: "studio-dock",
                span { class: "studio-dock-icon", "aria-hidden": "true", "\u{1F4A1}" }
                span { class: "studio-dock-text", role: "status", "{dock_text}" }
                button {
                    class: "studio-dock-btn primary",
                    title: "Ask the kernel whether the first sentence follows",
                    onclick: move |_| handle_prove.call(()),
                    "\u{25B6} Prove"
                }
                button {
                    class: "studio-dock-btn",
                    aria_expanded: if *proof_open.read() { "true" } else { "false" },
                    onclick: move |_| {
                        let open = *proof_open.read();
                        proof_open.set(!open);
                    },
                    if *proof_open.read() { "Hide proof" } else { "Show proof" }
                }
            }
        }
        if current_mode == StudioMode::Code {
            div { class: "studio-dock",
                span { class: "studio-dock-icon", "aria-hidden": "true", dangerous_inner_html: IC_BUG }
                span { class: "studio-dock-text", role: "status", "{dock_text}" }
                if debugging() {
                    button { class: "studio-dock-btn", onclick: move |_| debugging.set(false), "Stop debugging" }
                } else {
                    button {
                        class: "studio-dock-btn primary",
                        disabled: !actions().debug.is_ready(),
                        title: actions().debug.reason().map(str::to_string),
                        onclick: move |_| {
                            code_output_mode.set(CodeOutputMode::Interpret);
                            debugging.set(true);
                        },
                        "Debug"
                    }
                }
            }
        }

        StatusBar {
            status: editor_status,
            language: status_language.name(&status_source).to_string(),
            indent: Indentation::of(status_language, &status_source).step.len(),
            problems: current_mode != StudioMode::Math,
        }
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio_workbench/tests.rs"]
mod tests;

/// The fairy and her card (work/LLM_HEAL.md): nothing while she is home; the fairy alone, flying
/// to the line, while the worker thinks and mends; the card beside her with Before, After, Keep
/// and Undo once the mend is back, or the reason there is none with a way to close it. Keep writes
/// the mend into the editor; Undo leaves the editor as it was.
#[component]
fn HealOverlay(flight: Signal<heal::Flight>, editor: Signal<String>, file: Signal<Option<String>>, baseline: Signal<std::collections::HashMap<String, String>>) -> Element {
    let state = flight.read().clone();
    if state == heal::Flight::Idle {
        return rsx! {};
    }
    let line = state.line();
    let mended = match &state {
        heal::Flight::Review { lines, .. } => format!("{} place{} mended. Nothing changes until Keep.", lines.len(), if lines.len() == 1 { "" } else { "s" }),
        _ => String::new(),
    };
    let phase = match &state {
        heal::Flight::Flying { .. } => "flying",
        heal::Flight::Mending => "mending",
        heal::Flight::Review { .. } => "review",
        heal::Flight::Failed(_) => "failed",
        heal::Flight::Idle => "idle",
    };
    // Escape closes the card, as a dialog should: the same as Undo, the editor untouched.
    #[cfg(target_arch = "wasm32")]
    {
        let closable = matches!(state, heal::Flight::Review { .. } | heal::Flight::Failed(_));
        use_effect(move || {
            if !closable {
                return;
            }
            use wasm_bindgen::{closure::Closure, JsCast};
            let Some(window) = web_sys::window() else { return };
            let on_key = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
                if event.key() == "Escape" {
                    let next = flight.peek().clone().undo();
                    flight.set(next);
                }
            });
            let _ = window.add_event_listener_with_callback("keydown", on_key.as_ref().unchecked_ref());
            on_key.forget();
        });
    }
    rsx! {
        div { class: "heal-overlay", style: "--heal-line: {line};",
            img { class: "heal-fairy {phase}", src: heal::FAIRY, alt: "The fairy", "aria-hidden": "true" }
            match &state {
                heal::Flight::Review { before, after, diff, .. } => rsx! {
                    div { class: "heal-card", role: "dialog", "aria-label": "Review the mend",
                        div { class: "heal-card-cols",
                            section { class: "heal-card-col",
                                h3 { "Before" }
                                pre { class: "heal-before", {marked_lines(before, after, &diff.before, &diff.after, "replaced")} }
                            }
                            section { class: "heal-card-col",
                                h3 { "After" }
                                pre { class: "heal-after", {marked_lines(after, before, &diff.after, &diff.before, "mended")} }
                            }
                        }
                        div { class: "heal-card-actions",
                            span { class: "heal-card-note", "{mended}" }
                            button {
                                class: "studio-dock-btn",
                                onclick: move |_| { let next = flight.peek().clone().undo(); flight.set(next); },
                                "Undo"
                            }
                            button {
                                class: "studio-dock-btn primary heal-keep",
                                onclick: move |_| {
                                    let (next, kept) = flight.peek().clone().keep();
                                    if let Some(text) = kept {
                                        editor.set(text.clone());
                                        if let Some(path) = file.peek().clone() {
                                            baseline.write().insert(path, text);
                                        }
                                    }
                                    flight.set(next);
                                },
                                "Keep"
                            }
                        }
                    }
                },
                heal::Flight::Failed(reason) => rsx! {
                    div { class: "heal-card heal-card-failed", role: "alert",
                        p { "{reason}" }
                        button {
                            class: "studio-dock-btn",
                            onclick: move |_| { let next = flight.peek().clone().undo(); flight.set(next); },
                            "Close"
                        }
                    }
                },
                _ => rsx! {},
            }
        }
    }
}

/// `text` line by line for the card, the lines in `own` marked with `class`, and within a marked
/// line that replaces one line of the other side (its counterpart in `other`), the words that
/// differ marked stronger. So the reader sees the word that changed, not only the line.
fn marked_lines(text: &str, other_text: &str, own: &[std::ops::Range<usize>], other: &[std::ops::Range<usize>], class: &'static str) -> Element {
    let other_lines: Vec<&str> = other_text.lines().collect();
    let counterpart = |i: usize| -> Option<&str> {
        let (k, r) = own.iter().enumerate().find(|(_, r)| r.contains(&i))?;
        let o = other.get(k)?;
        (r.len() == 1 && o.len() == 1).then(|| other_lines.get(o.start).copied()).flatten()
    };
    rsx! {
        for (i, line) in text.lines().enumerate() {
            if own.iter().any(|r| r.contains(&i)) {
                span { class: "heal-line {class}",
                    if let Some(pair) = counterpart(i) {
                        for (piece, differs) in heal::word_marks(line, pair) {
                            span { class: if differs { "heal-word" } else { "" }, "{piece}" }
                        }
                    } else {
                        "{line}"
                    }
                    "\n"
                }
            } else {
                span { class: "heal-line", "{line}\n" }
            }
        }
    }
}
