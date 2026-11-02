//! What the front page says, as data.
//!
//! The heading, the line on how for each system, the prompt for each system, the guide the prompt
//! sends an agent to (served at /agent.txt), the Studio frame's shelf, the comparison, what comes with
//! LOGOS and the FAQ live here rather than in the markup, so they read and review in one place and
//! every line that states a fact about LOGOS carries the suites that prove it. `landing_page` in logicaffeine_tests holds every suite to existing, every command in the guide
//! to the released largo and to this tree's, every address to a file or page the site serves, and the
//! page to showing exactly the text the copy button copies.

use std::sync::LazyLock;

use crate::ui::pages::contact::claims::learn_figures;
use crate::ui::pages::contact::content::{free_answer, ASSISTANT_GUIDE_URL, INSTALL_UNIX, INSTALL_WINDOWS, SOURCE_URL};
use crate::ui::pages::studio_workbench::Shelf;
use crate::ui::router::studio_file_url;

/// How largo installs: the shell script on macOS, Linux and every phone's agent, PowerShell on Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    Unix,
    Windows,
}

impl Install {
    pub const ALL: &'static [Install] = &[Install::Unix, Install::Windows];

    /// The name the page marks its prompts and `<html data-os>` with, and remembers.
    pub fn name(self) -> &'static str {
        match self {
            Install::Unix => "unix",
            Install::Windows => "windows",
        }
    }

    pub fn from_name(name: &str) -> Option<Install> {
        Install::ALL.iter().copied().find(|install| install.name() == name)
    }

    /// The switch's offer under this install's prompt: the other system.
    pub fn switch_label(self) -> &'static str {
        match self {
            Install::Unix => "Using Windows?",
            Install::Windows => "Using macOS or Linux?",
        }
    }

    /// The other install, which the switch shows.
    pub fn other(self) -> Install {
        match self {
            Install::Unix => Install::Windows,
            Install::Windows => Install::Unix,
        }
    }
}

/// The guide the prompt sends an agent to before it writes: the instructions, served whole
/// (`public/agent.txt`, which `landing_page` holds to [`instructions_text`] byte for byte).
pub const AGENT_GUIDE_URL: &str = "https://logicaffeine.com/agent.txt";

/// Where `largo serve` puts the site: its default port, on the reader's own machine.
pub const SERVE_URL: &str = "http://localhost:8080";

/// The page that lists the crates the language is built from, one per part.
pub const CRATES_URL: &str = "https://logicaffeine.com/crates";

/// The language on one page: the forms an agent writes, one construct a line, drawn from the
/// quickguide's tables (`teach_parity` holds the guide to the lessons, `doc_examples` runs the docs).
/// The guide carries it under step 4, and the heal worker's prompt carries the same lines
/// (`services/heal/src/primer.js`, held equal by `landing_page`), so the model that mends a program
/// knows the language it mends (the user, 2026-11-02).
pub const SYNTAX_MATRIX: &[&str] = &[
    "The language on one page. Every statement is one English sentence.",
    "Blocks: ## Main is the program. ## To add (a: Int, b: Int) -> Int: is a function.",
    "## A Point has: with An x: Int. below is a struct. ## A Shape is one of: is an enum.",
    "## Page /path is a web page. ## Route GET /api/x -> Seq of Todo: is an API route.",
    "## Test: name is a test of a page. ## Theorem: name is a theorem for the prover.",
    "Bind: Let x be 5. Let mutable n be 0. Let x: Int be 5. Change: Set n to n + 1.",
    "Also: Increase n by 1. Decrease n by 1. Set p's x to 5. Set item i of xs to v.",
    "Types: Int, Float, Text, Bool, Seq of Int, Map of Text to Int, Set of Int, some 30, none.",
    "Make: a new Seq of Int, [1, 2, 3], a new Map of Text to Int, {1, 2, 3}, a new Point.",
    "Read: item 1 of xs is the first item, length of xs, xs contains v, p's x, p.x.",
    "Slice: items 2 through 4 of xs. Change a list or set: Push v to xs. Pop from xs into y.",
    "Add v to s. Remove v from s. Arithmetic: x + y, x - y, x * y, x / y, x % y.",
    "Text: \"Hi, {name}!\" fills the hole with name, a combined with b joins two Texts.",
    "Compare: is equal to or ==, is not or !=, is less than or <, is greater than or >.",
    "Also: is at most or <=, is at least or >=, is between lo and hi, is divisible by n.",
    "Logic in a test: and, or, not. Choose: If c: then Otherwise If d: then Otherwise:.",
    "A block is the lines indented four spaces under the colon.",
    "Repeat: While c:, Repeat for x in xs:, Repeat for i from 1 to n:, Break.",
    "Match: Inspect t: then When V (a): and Otherwise:. Options: some 30, none.",
    "Filter: Let small be every n in xs where n is less than 4.",
    "In place: Remove every t from todos where t's done.",
    "Call: add(3, 7), or xs.f(a) for f(xs, a). A closure: (x: Int) -> x + 1. Return x.",
    "Show x. prints x. Contracts: Let x: Int where x > 0 be 5. Assert that x is equal to 42.",
    "Check that b is at least a. stops the program when it is not.",
    "Time: 500ms, 2s, 5min, 1h, 2026-05-20, 9am, 2 weeks, 2026-05-20 at 4pm.",
    "A page: State: holds Let draft be \"\". Actions: holds To add: and its steps.",
    "View: is HTML. {draft} is a hole, on-click={add}, bind={draft}, {if c:}...{else:}...{end}.",
    "Lists in a View: {for each t in todos, keyed by t's id:}...{end}. Style: is CSS, scoped.",
    "Derived state: Let left always be length of todos. It recomputes after every turn.",
    "A page test: Click \"#add\". Expect that the text of \"#n\" is equal to \"1\".",
    "Logic mode: one sentence per line. Every dog barks. Some cat sleeps. No cat barks.",
    "A theorem: Given: Every man is mortal. Given: Socrates is a man. Prove: He is mortal.",
    "Then Proof: Auto. and the kernel certifies it.",
];

/// The command that clones the source, with its examples and docs, for an agent to read.
pub const CLONE_SOURCE: &str = "git clone https://github.com/Brahmastra-Labs/logicaffeine";

/// The five lines a person pastes into their coding agent, for `install`: what this is, the source on
/// GitHub first, the install that runs on the reader's system, and the guide to follow, which ends
/// with a site running. Plain and agent agnostic (Tristen and the user, 2026-11-01, DAT-105).
pub fn prompt(install: Install) -> &'static [Line] {
    static PROMPTS: LazyLock<[Vec<Line>; 2]> = LazyLock::new(|| [Install::Unix, Install::Windows].map(prompt_lines));
    &PROMPTS[install as usize]
}

fn prompt_lines(install: Install) -> Vec<Line> {
    use Line::*;
    let (how, command) = match install {
        Install::Unix => ("Install largo, the LOGOS command line tool, easy to install and lightweight:", INSTALL_UNIX),
        Install::Windows => ("Install largo, the LOGOS command line tool, easy to install, lightweight, in PowerShell:", INSTALL_WINDOWS),
    };
    vec![
        Say(agreed("LOGOS programmer instructions, for any coding agent (bring your own).")),
        Say(agreed(format!("Source, examples and docs: {SOURCE_URL}"))),
        Say(agreed(how)),
        Run(command),
        Say(agreed(format!("Then follow {AGENT_GUIDE_URL}, every step. It ends with a site running."))),
    ]
}

/// The prompt as one text: what the page shows and what the copy button copies.
pub fn prompt_text(install: Install) -> &'static str {
    static TEXTS: LazyLock<[String; 2]> =
        LazyLock::new(|| [Install::Unix, Install::Windows].map(|install| prompt(install).iter().map(Line::text).collect::<Vec<_>>().join("\n")));
    &TEXTS[install as usize]
}

/// The one line below the heading on how to use LOGOS: the whole of what the page tells a reader to
/// do, since the prompt itself only copies (the user, 2026-10-31).
pub fn how() -> &'static Claim {
    static HOW: LazyLock<Claim> = LazyLock::new(|| {
        proven("Copy the prompt for your system into your coding agent. It installs largo and puts a LOGOS website on your screen.", &["landing_page", "llms_lock"])
    });
    &HOW
}

/// A system a reader copies the prompt for: one button each, with the system's mark. macOS and
/// Linux share the shell install; Windows has PowerShell's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    MacOs,
    Linux,
    Windows,
}

impl System {
    pub const ALL: [System; 3] = [System::MacOs, System::Linux, System::Windows];

    pub fn name(self) -> &'static str {
        match self {
            System::MacOs => "macOS",
            System::Linux => "Linux",
            System::Windows => "Windows",
        }
    }

    /// The name the page marks the button with.
    pub fn slug(self) -> &'static str {
        match self {
            System::MacOs => "macos",
            System::Linux => "linux",
            System::Windows => "windows",
        }
    }

    /// The prompt the button copies: the install that runs on the system.
    pub fn install(self) -> Install {
        match self {
            System::Windows => Install::Windows,
            System::MacOs | System::Linux => Install::Unix,
        }
    }

    /// The button's words.
    pub fn copy_label(self) -> String {
        format!("{COPY_LABEL} for {}", self.name())
    }

    /// The system's mark, as an SVG path on a 24-unit square (Apple's and Ubuntu's from Simple Icons,
    /// CC0; Windows's four panes drawn here).
    pub fn mark(self) -> &'static str {
        match self {
            System::MacOs => APPLE_MARK,
            System::Linux => UBUNTU_MARK,
            System::Windows => WINDOWS_MARK,
        }
    }
}

const APPLE_MARK: &str = "M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701";
const UBUNTU_MARK: &str = "M17.61.455a3.41 3.41 0 0 0-3.41 3.41 3.41 3.41 0 0 0 3.41 3.41 3.41 3.41 0 0 0 3.41-3.41 3.41 3.41 0 0 0-3.41-3.41zM12.92.8C8.923.777 5.137 2.941 3.148 6.451a4.5 4.5 0 0 1 .26-.007 4.92 4.92 0 0 1 2.585.737A8.316 8.316 0 0 1 12.688 3.6 4.944 4.944 0 0 1 13.723.834 11.008 11.008 0 0 0 12.92.8zm9.226 4.994a4.915 4.915 0 0 1-1.918 2.246 8.36 8.36 0 0 1-.273 8.303 4.89 4.89 0 0 1 1.632 2.54 11.156 11.156 0 0 0 .559-13.089zM3.41 7.932A3.41 3.41 0 0 0 0 11.342a3.41 3.41 0 0 0 3.41 3.409 3.41 3.41 0 0 0 3.41-3.41 3.41 3.41 0 0 0-3.41-3.41zm2.027 7.866a4.908 4.908 0 0 1-2.915.358 11.1 11.1 0 0 0 7.991 6.698 11.234 11.234 0 0 0 2.422.249 4.879 4.879 0 0 1-.999-2.85 8.484 8.484 0 0 1-.836-.136 8.304 8.304 0 0 1-5.663-4.32zm11.405.928a3.41 3.41 0 0 0-3.41 3.41 3.41 3.41 0 0 0 3.41 3.41 3.41 3.41 0 0 0 3.41-3.41 3.41 3.41 0 0 0-3.41-3.41z";
const WINDOWS_MARK: &str = "M3 3h8.5v8.5H3zM12.5 3H21v8.5h-8.5zM3 12.5h8.5V21H3zM12.5 12.5H21V21h-8.5z";

/// The box above the buttons for a reader who installs largo by hand: its heading and the command
/// for each kind of system, as the pricing page states them.
pub const INSTALL_HEADING: &str = "Install largo yourself";

pub fn install_commands() -> [(&'static str, &'static str); 2] {
    [("macOS and Linux", INSTALL_UNIX), ("Windows", INSTALL_WINDOWS)]
}

/// The heading: the promise, then the two things that come with it. The user's words (2026-10-01).
pub const HEADING: [&str; 3] = ["Code we can all read.", "Logic and mathematics built in.", "Bring your own agent."];

/// The button below the heading, which copies the instructions.
pub const COPY_LABEL: &str = "Copy agent instructions";

/// The element that shows the instructions, which the copy button names.
pub const INSTRUCTIONS_ID: &str = "agent-instructions";

/// What the page says only once the reader acts.
pub mod said {
    /// The copy button's label while the instructions are on the clipboard.
    pub const COPIED: &str = "Copied";
    /// What a screen reader hears once the instructions are copied.
    pub const COPIED_STATUS: &str = "Copied the agent instructions.";
    /// What the page says when the browser refuses the copy, having selected the instructions.
    pub const COPY_REFUSED: &str = "This browser blocked copying. The instructions are selected, so press Ctrl+C or Command+C.";
    /// An install command's copy button, while the command is on the clipboard.
    pub const COMMAND_COPIED: &str = "Copied";

    /// Every one of them, with where it shows.
    pub fn all() -> Vec<(&'static str, &'static str)> {
        vec![
            ("the copy button once pressed", COPIED),
            ("the copy status", COPIED_STATUS),
            ("the copy status when the browser refuses", COPY_REFUSED),
            ("an install command's copy button once pressed", COMMAND_COPIED),
        ]
    }
}

/// What backs a line of the page./// What backs a line of the page.
pub enum Evidence {
    /// The suites under `crates/logicaffeine_tests/tests/` that prove it.
    Suites(&'static [&'static str]),
    /// It states nothing a suite could prove: a label, a direction, how the pair works.
    Agreement,
}

/// One sentence the page states, and what backs it.
pub struct Claim {
    pub text: String,
    pub evidence: Evidence,
}

fn proven(text: impl Into<String>, suites: &'static [&'static str]) -> Claim {
    Claim { text: text.into(), evidence: Evidence::Suites(suites) }
}

fn agreed(text: impl Into<String>) -> Claim {
    Claim { text: text.into(), evidence: Evidence::Agreement }
}

/// One line of the agent instructions.
pub enum Line {
    /// A step's title, numbered.
    Step(&'static str),
    /// A sentence.
    Say(Claim),
    /// A rule of the language, in the list under step 2.
    Rule(Claim),
    /// How the pair works, in the list under the last step.
    Agree(&'static str),
    /// A command to type, on a line of its own.
    Run(&'static str),
    /// A page to open, on a line of its own.
    Open(String),
    Blank,
}

/// A command or an address sits four spaces in, where an agent and a person both read it as one to
/// type or open.
const INDENT: &str = "    ";

impl Line {
    /// The line as the copied text reads it.
    pub fn text(&self) -> String {
        match self {
            Line::Step(title) => title.to_string(),
            Line::Say(claim) => claim.text.clone(),
            Line::Rule(claim) => format!("- {}", claim.text),
            Line::Agree(text) => format!("- {text}"),
            Line::Run(command) => format!("{INDENT}{command}"),
            Line::Open(url) => format!("{INDENT}{url}"),
            Line::Blank => String::new(),
        }
    }
}

/// The site's address for a shipped Studio example.
fn studio_address(example: &str) -> String {
    format!("https://logicaffeine.com{}", studio_file_url(example))
}

/// The guide the prompt sends an agent through: install, a website on the reader's screen inside
/// a few minutes, then the language, the site, programs, logic, math and hardware, each where it
/// lives in the released largo, and how the two of them work (Tristen, 2026-11-01, DAT-105: get
/// them up, running and building, with a starter site, and point them at the source). Every command
/// runs on the release the install script installs.
pub fn instructions() -> &'static [Line] {
    static LINES: LazyLock<Vec<Line>> = LazyLock::new(|| {
        use Line::*;
        vec![
            Say(proven("LOGOS programmer instructions, for any coding agent (bring your own).", &["landing_page", "llms_lock"])),
            Say(proven("LOGOS is the English programming language from logicaffeine.com.", &["doc_examples", "code_examples_all_tiers"])),
            Say(proven("LOGOS is lightweight and modular, and largo, its command line tool, is easy to install.", &["installer_docs_lock", "landing_page"])),
            Blank,
            Step("1. Get the source"),
            Say(agreed("Source, examples and docs are on GitHub. Clone it, and read it when a question comes up:")),
            Run(CLONE_SOURCE),
            Say(proven(format!("Its crates, one per part of the language, are listed at {CRATES_URL}."), &["crates_page_lock", "landing_page"])),
            Blank,
            Step("2. Install largo, the LOGOS command line tool"),
            Say(agreed("On macOS and Linux:")),
            Run(INSTALL_UNIX),
            Say(agreed("On Windows, in PowerShell:")),
            Run(INSTALL_WINDOWS),
            Say(proven("If the shell can't find largo, open a new terminal or add its folder to PATH.", &["installer_docs_lock"])),
            Say(agreed("Then check the install:")),
            Run("largo --version"),
            Run("largo doctor"),
            Blank,
            Step("3. Start a website"),
            Run("largo new mysite --site"),
            Run("cd mysite"),
            Run("largo test"),
            Run("largo serve"),
            Say(proven("The front page is pages/app.lg: a counter with a button, written in English.", &["landing_page"])),
            Say(proven("largo test ran the ## Test at the end of the page: it clicks the button and counts.", &["landing_page"])),
            Say(proven("largo serve keeps running, so run it in a second terminal or in the background.", &["landing_page"])),
            Say(proven(format!("The site is at {SERVE_URL}. Each save rebuilds the page and reloads it."), &["landing_page"])),
            Say(agreed("Report that the site is up before changing anything.")),
            Blank,
            Step("4. Learn the language before you write it"),
            Say(proven(format!("Read {ASSISTANT_GUIDE_URL}, every LOGOS guide in one file."), &["llms_lock"])),
            Say(proven("largo new also wrote AGENTS.md in the project, the language taught to an agent. Read it.", &["landing_page"])),
            Say(proven("Use a command only if largo --help lists it, and say so when one is missing.", &["landing_page"])),
            Say(agreed("Four rules to start with:")),
            Rule(proven("A statement is one English sentence, as in Let x be 5.", &["doc_examples", "landing_page"])),
            Rule(proven("Let names a value and Set changes it, as in Set x to 6.", &["phase_interpret_mode", "landing_page"])),
            Rule(proven("Lists count from 1: item 1 of xs is the first item.", &["phase22_index", "landing_page"])),
            Rule(proven("Compare with is equal to, as in If name is equal to \"Ada\":", &["correctness_is_equality", "landing_page"])),
            Blank,
            Say(agreed("The language on one page:")),
            Say(proven("Blocks: ## Main is the program. ## To add (a: Int, b: Int) -> Int: is a function.", &["teach_parity", "doc_examples"])),
            Say(proven("## A Point has: with An x: Int. below is a struct. ## A Shape is one of: is an enum.", &["teach_parity", "doc_examples"])),
            Say(proven("## Page /path is a web page. ## Route GET /api/x -> Seq of Todo: is an API route.", &["teach_parity", "doc_examples"])),
            Say(proven("## Test: name is a test of a page. ## Theorem: name is a theorem for the prover.", &["teach_parity", "doc_examples"])),
            Say(proven("Bind: Let x be 5. Let mutable n be 0. Let x: Int be 5. Change: Set n to n + 1.", &["teach_parity", "doc_examples"])),
            Say(proven("Also: Increase n by 1. Decrease n by 1. Set p's x to 5. Set item i of xs to v.", &["teach_parity", "doc_examples"])),
            Say(proven("Types: Int, Float, Text, Bool, Seq of Int, Map of Text to Int, Set of Int, some 30, none.", &["teach_parity", "doc_examples"])),
            Say(proven("Make: a new Seq of Int, [1, 2, 3], a new Map of Text to Int, {1, 2, 3}, a new Point.", &["teach_parity", "doc_examples"])),
            Say(proven("Read: item 1 of xs is the first item, length of xs, xs contains v, p's x, p.x.", &["teach_parity", "doc_examples"])),
            Say(proven("Slice: items 2 through 4 of xs. Change a list or set: Push v to xs. Pop from xs into y.", &["teach_parity", "doc_examples"])),
            Say(proven("Add v to s. Remove v from s. Arithmetic: x + y, x - y, x * y, x / y, x % y.", &["teach_parity", "doc_examples"])),
            Say(proven("Text: \"Hi, {name}!\" fills the hole with name, a combined with b joins two Texts.", &["teach_parity", "doc_examples"])),
            Say(proven("Compare: is equal to or ==, is not or !=, is less than or <, is greater than or >.", &["teach_parity", "doc_examples"])),
            Say(proven("Also: is at most or <=, is at least or >=, is between lo and hi, is divisible by n.", &["teach_parity", "doc_examples"])),
            Say(proven("Logic in a test: and, or, not. Choose: If c: then Otherwise If d: then Otherwise:.", &["teach_parity", "doc_examples"])),
            Say(proven("A block is the lines indented four spaces under the colon.", &["teach_parity", "doc_examples"])),
            Say(proven("Repeat: While c:, Repeat for x in xs:, Repeat for i from 1 to n:, Break.", &["teach_parity", "doc_examples"])),
            Say(proven("Match: Inspect t: then When V (a): and Otherwise:. Options: some 30, none.", &["teach_parity", "doc_examples"])),
            Say(proven("Filter: Let small be every n in xs where n is less than 4.", &["teach_parity", "doc_examples"])),
            Say(proven("In place: Remove every t from todos where t's done.", &["teach_parity", "doc_examples"])),
            Say(proven("Call: add(3, 7), or xs.f(a) for f(xs, a). A closure: (x: Int) -> x + 1. Return x.", &["teach_parity", "doc_examples"])),
            Say(proven("Show x. prints x. Contracts: Let x: Int where x > 0 be 5. Assert that x is equal to 42.", &["teach_parity", "doc_examples"])),
            Say(proven("Check that b is at least a. stops the program when it is not.", &["teach_parity", "doc_examples"])),
            Say(proven("Time: 500ms, 2s, 5min, 1h, 2026-05-20, 9am, 2 weeks, 2026-05-20 at 4pm.", &["teach_parity", "doc_examples"])),
            Say(proven("A page: State: holds Let draft be \"\". Actions: holds To add: and its steps.", &["teach_parity", "doc_examples"])),
            Say(proven("View: is HTML. {draft} is a hole, on-click={add}, bind={draft}, {if c:}...{else:}...{end}.", &["teach_parity", "doc_examples"])),
            Say(proven("Lists in a View: {for each t in todos, keyed by t's id:}...{end}. Style: is CSS, scoped.", &["teach_parity", "doc_examples"])),
            Say(proven("Derived state: Let left always be length of todos. It recomputes after every turn.", &["teach_parity", "doc_examples"])),
            Say(proven("A page test: Click \"#add\". Expect that the text of \"#n\" is equal to \"1\".", &["teach_parity", "doc_examples"])),
            Say(proven("Logic mode: one sentence per line. Every dog barks. Some cat sleeps. No cat barks.", &["teach_parity", "doc_examples"])),
            Say(proven("A theorem: Given: Every man is mortal. Given: Socrates is a man. Prove: He is mortal.", &["teach_parity", "doc_examples"])),
            Say(proven("Then Proof: Auto. and the kernel certifies it.", &["teach_parity", "doc_examples"])),
            Blank,
            Step("5. Build the site"),
            Say(agreed("Change pages/app.lg one thing at a time. After every change, run:")),
            Run("largo check"),
            Run("largo test"),
            Say(proven("largo check explains each mistake it finds in plain English, with its line.", &["check_json", "diagnostic_surfaces_lock", "landing_page"])),
            Say(proven("largo fmt sets the source in the standard layout.", &["fmt"])),
            Say(proven("A page is its State, its Actions, its View in HTML with {holes}, and its Style in CSS.", &["landing_page"])),
            Say(proven("For a backend of ## Route blocks under the pages, scaffold with largo new mysite --web.", &["landing_page"])),
            Blank,
            Step("6. Programs"),
            Run("largo new hello"),
            Run("cd hello"),
            Run("largo run --interpret"),
            Say(proven("It prints Hello, world! The program is src/main.lg and the manifest is Largo.toml.", &["landing_page", "phase_interpret_mode"])),
            Say(proven("--interpret runs it in the interpreter, which needs no Rust toolchain.", &["landing_page", "phase_interpret_mode"])),
            Say(proven("largo new also wrote a .gitignore, so git init makes the folder a repository.", &["landing_page"])),
            Blank,
            Step("7. Logic"),
            Run("largo logic \"Every student read a book.\" --all-readings"),
            Say(proven("prints every reading of the sentence in first-order logic.", &["logic", "phase12_ambiguity", "landing_page"])),
            Say(proven("largo prove proves each ## Theorem block in a file (Given, Prove and Proof lines),", &["prove", "landing_page"])),
            Say(proven("and the type theory kernel certifies every proof it reports:", &["prove", "phase69_kernel_coc"])),
            Run("largo prove theorem.lg"),
            Blank,
            Step("8. Math"),
            Say(proven("largo prove also proves the theorems of a ## Theory block from its axioms.", &["prove", "landing_page"])),
            Say(proven("Inductive types, Definition, Check and Eval run in the Studio's Math mode:", &["e2e_studio_examples", "studio_example_contract"])),
            Open(studio_address("/examples/math/natural-numbers.logos")),
            Blank,
            Step("9. Hardware"),
            Say(proven("The Studio's Hardware mode turns an English spec into a SystemVerilog assertion", &["hardware_examples", "phase_hw_sva_2023"])),
            Say(proven("and checks that the assertion matches the spec:", &["hardware_examples", "landing_page"])),
            Open(studio_address("/examples/hardware/handshake.hw")),
            Say(proven("The command line release has no hardware command yet.", &["landing_page"])),
            Blank,
            Step("10. Working with the user"),
            Agree("Take small steps. Change one thing, run it, and report the output."),
            Agree("Never report that something works before running it."),
            Agree("Show the LOGOS written. The user reads it as English, so keep it plain."),
            Agree("When a sentence can mean two things, ask which one is meant."),
            Agree("Commit each step that runs, so there is always a way back."),
        ]
    });
    &LINES
}

/// The instructions as one text: what the page shows and what the copy button copies.
pub fn instructions_text() -> &'static str {
    static TEXT: LazyLock<String> = LazyLock::new(|| instructions().iter().map(Line::text).collect::<Vec<_>>().join("\n"));
    &TEXT
}

/// The front page's Studio frame: one shipped example for each kind of work the comparison names, in
/// the Studio's own folders, opened on the website in a website (the user, 2026-10-01). The Logic
/// example is a theorem with its givens, so a first visit's Prove has something to prove from (the
/// user, 2026-11-01).
pub fn shelf() -> &'static Shelf {
    static SHELF: Shelf = Shelf {
        opens: "/examples/code/web/website.logos",
        files: &[
            "/examples/code/web/website.logos",
            "/examples/code/basics/control-flow.logos",
            "/examples/code/verification/refinement.logos",
            "/examples/code/distributed/merge.logos",
            "/examples/logic/prover-demo.logic",
            "/examples/math/auto-tactic.logos",
            "/examples/hardware/handshake.hw",
        ],
    };
    &SHELF
}

/// A tool a kind of work usually reaches for, named as its own site names it, and the page where it
/// says so (shown nowhere on the page).
pub struct Usually {
    pub name: &'static str,
    pub source: &'static str,
}

/// Where what a card says stands: in the released largo, in the Studio, in part, or in the next
/// release (this tree, ahead of the release a reader installs today).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ships,
    InTheStudio,
    Partial,
    NextRelease,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Ships => "In largo",
            Status::InTheStudio => "In the Studio",
            Status::Partial => "In part",
            Status::NextRelease => "Next release",
        }
    }
}

/// What a card's line leans on in largo: a command, or a target of `largo emit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Uses {
    Command(&'static str),
    Emit(&'static str),
}

/// One card of the comparison: a kind of work, what it is usually done with, and what LOGOS does there.
pub struct Card {
    pub kind: &'static str,
    pub usually: &'static [Usually],
    pub logos: Claim,
    pub tags: &'static [Status],
    pub uses: &'static [Uses],
    /// The shelf file the card's Try it opens in the frame above.
    pub try_it: Option<&'static str>,
}

impl Card {
    /// The tools the kind of work is usually done with, as the card lists them.
    pub fn usually_names(&self) -> String {
        self.usually.iter().map(|tool| tool.name).collect::<Vec<_>>().join(", ")
    }
}

pub const COMPARE_HEADING: &str = "How LOGOS compares";

/// The line under the comparison's heading: what the cards are.
pub fn compare_lead() -> &'static Claim {
    static LEAD: LazyLock<Claim> =
        LazyLock::new(|| proven("Eight kinds of work, what each usually takes, and what LOGOS does there.", &["landing_page"]));
    &LEAD
}

const PYTHON: Usually = Usually { name: "Python", source: "https://docs.python.org/3/" };
const RUST: Usually = Usually { name: "Rust", source: "https://www.rust-lang.org/" };
const C: Usually = Usually { name: "C", source: "https://www.open-std.org/jtc1/sc22/wg14/" };
const LEAN: Usually = Usually { name: "Lean 4", source: "https://lean-lang.org/" };
const ROCQ: Usually = Usually { name: "Rocq", source: "https://rocq-prover.org/" };
const DAFNY: Usually = Usually { name: "Dafny", source: "https://dafny.org/" };
const TLA: Usually = Usually { name: "TLA+", source: "https://lamport.azurewebsites.net/tla/tla.html" };
const PROLOG: Usually = Usually { name: "Prolog", source: "https://www.iso.org/standard/21413.html" };
const ACE: Usually = Usually { name: "Attempto Controlled English", source: "https://attempto.ifi.uzh.ch/site/" };
const SYSTEMVERILOG: Usually = Usually { name: "SystemVerilog", source: "https://standards.ieee.org/ieee/1800/7743/" };
const CHISEL: Usually = Usually { name: "Chisel", source: "https://www.chisel-lang.org/" };
const CUDA: Usually = Usually { name: "CUDA", source: "https://docs.nvidia.com/cuda/" };
const TYPESCRIPT: Usually = Usually { name: "TypeScript", source: "https://www.typescriptlang.org/" };
const JAVASCRIPT: Usually = Usually { name: "JavaScript", source: "https://tc39.es/ecma262/" };
const ELIXIR: Usually = Usually { name: "Elixir", source: "https://elixir-lang.org/" };
const ERLANG: Usually = Usually { name: "Erlang", source: "https://www.erlang.org/" };
const AUTOMERGE: Usually = Usually { name: "Automerge", source: "https://automerge.org/" };

/// The comparison: eight kinds of work, each with what it is usually done with and what LOGOS does
/// there as of 0.11.0, every line proven by the suites it names, and no claim of a win over anything.
/// A card tagged for the next release leans on a command or an emit target the released largo lacks;
/// as of 0.11.0, none does.
pub fn compare() -> &'static [Card] {
    static CARDS: LazyLock<Vec<Card>> = LazyLock::new(|| {
        vec![
            Card {
                kind: "General purpose",
                usually: &[PYTHON, RUST, C],
                logos: proven(
                    "English programs, checked before they run, built through Rust, C or WebAssembly.",
                    &["code_examples_all_tiers", "benchmark_logos_tiers_lock", "largo_build_emit_c"],
                ),
                tags: &[Status::Ships],
                uses: &[Uses::Command("build"), Uses::Emit("c"), Uses::Emit("wasm")],
                try_it: Some("/examples/code/basics/control-flow.logos"),
            },
            Card {
                kind: "Proof assistants",
                usually: &[LEAN, ROCQ],
                logos: proven(
                    "Theorems in English, checked by a kernel whose integers are Lean's.",
                    &["dag_dual_agreement", "axiom_footprint_lean_parity", "definition_kernel"],
                ),
                tags: &[Status::InTheStudio],
                uses: &[Uses::Command("lean")],
                try_it: Some("/examples/math/auto-tactic.logos"),
            },
            Card {
                kind: "Verification",
                usually: &[DAFNY, TLA],
                logos: proven(
                    "Refinements and assertions checked as the program is built.",
                    &["e2e_refinement", "phase25_assertions", "phase43_refinement"],
                ),
                tags: &[Status::Ships],
                uses: &[Uses::Command("check"), Uses::Command("verify")],
                try_it: Some("/examples/code/verification/refinement.logos"),
            },
            Card {
                kind: "Logic and controlled English",
                usually: &[PROLOG, ACE],
                logos: proven(
                    "Sentences become first-order logic, with scope and every reading.",
                    &["logic", "logic_determiner_readings", "modal_scope_tests"],
                ),
                tags: &[Status::InTheStudio],
                uses: &[Uses::Command("logic")],
                try_it: Some("/examples/logic/prover-demo.logic"),
            },
            Card {
                kind: "Hardware",
                usually: &[SYSTEMVERILOG, CHISEL],
                logos: proven(
                    "English specs become SystemVerilog assertions, programs become circuits.",
                    &["phase_hw_fol_to_sva", "phase_hw_sva_roundtrip", "caravel_signoff_lock", "sky130_deep_signoff_lock"],
                ),
                tags: &[Status::InTheStudio],
                uses: &[Uses::Command("bringup")],
                try_it: Some("/examples/hardware/handshake.hw"),
            },
            Card {
                kind: "GPU",
                usually: &[CUDA],
                logos: proven(
                    "Transformers written as tensor sentences, lowered to CUDA, FPGAs and Tenstorrent.",
                    &["cuda_aot_lock", "cuda_attention_gqa_lock", "int8_decode_lock", "decode_board_lock"],
                ),
                tags: &[Status::Ships],
                uses: &[Uses::Emit("cuda"), Uses::Emit("tenstorrent")],
                try_it: None,
            },
            Card {
                kind: "Web",
                usually: &[TYPESCRIPT, JAVASCRIPT],
                logos: proven(
                    "Pages, components and typed routes in English, tested where they run.",
                    &["lgx_check_lock", "lgx_constructs_lock", "serve_fullstack"],
                ),
                tags: &[Status::InTheStudio],
                uses: &[Uses::Command("serve"), Uses::Command("test")],
                try_it: Some("/examples/code/web/website.logos"),
            },
            Card {
                kind: "Concurrency and distributed",
                usually: &[ELIXIR, ERLANG, AUTOMERGE],
                logos: proven(
                    "Tasks and channels in English, collections that merge without conflict.",
                    &["concurrency_aot", "e2e_crdt", "converge_crdt_lock"],
                ),
                tags: &[Status::Ships],
                uses: &[Uses::Command("run")],
                try_it: Some("/examples/code/distributed/merge.logos"),
            },
        ]
    });
    &CARDS
}

pub const LIBRARY_HEADING: &str = "What comes with LOGOS";

/// The standard library's figures: the prelude's module count, read from the loader, and the parity
/// ledger's correctness figures and upstream languages, read from the ledger as it is checked in
/// (`benchmarks/parity/PARITY_LEDGER.md`), so the page can never say a number the ledger does not.
pub struct LibraryFigures {
    pub modules: usize,
    pub green: usize,
    pub in_scope: usize,
}

const PARITY_LEDGER: &str = include_str!("../../../../../../benchmarks/parity/PARITY_LEDGER.md");

pub fn library_figures() -> LibraryFigures {
    let parity = PARITY_LEDGER.lines().find(|line| line.contains("**Correctness parity:**")).expect("the ledger states its correctness parity");
    let inner = &parity[parity.find('(').unwrap() + 1..parity.find(')').unwrap()];
    let numbers: Vec<usize> = inner.split(|c: char| !c.is_ascii_digit()).filter(|w| !w.is_empty()).filter_map(|w| w.parse().ok()).collect();
    LibraryFigures { modules: logicaffeine_compile::loader::prelude_module_sources().len(), green: numbers[0], in_scope: numbers[1] }
}


/// The standard library in two lines, each with the figures it quotes.
pub fn library() -> &'static [Claim] {
    static LINES: LazyLock<Vec<Claim>> = LazyLock::new(|| {
        let figures = library_figures();
        vec![
            proven(
                format!("{} modules written in LOGOS, auto-imported as a program uses them.", figures.modules),
                &["import_scan_lock", "phase36_modules", "crypto_stdlib"],
            ),
            proven(
                format!(
                    "{} of {} capabilities of the C, Python, Rust, Go and Java standard libraries, checked against their sources.",
                    figures.green, figures.in_scope
                ),
                &["blas_stdlib", "crypto_stdlib", "http_stdlib"],
            ),
        ]
    });
    &LINES
}

/// What 0.11.0 brings, a line each, ten words or fewer, each proven by the suites it names.
pub fn features() -> &'static [Claim] {
    static LINES: LazyLock<Vec<Claim>> = LazyLock::new(|| {
        vec![
            proven("Tests, properties and replays, written in the language.", &["property_test_lock", "tests_index_lock", "replay_lock"]),
            proven("A failing run replays, and the debugger steps backward.", &["replay_lock", "e2e_deterministic_replay"]),
            proven("Import joins files. The type checker runs on every join.", &["import_hygiene", "phase36_modules", "lgx_check_lock"]),
            proven("Infinitesimals, hyperreals and exact reals, compiled to Rust.", &["hyperreal_closure", "hyperreal_functions", "exact_by_context"]),
            proven("Theorems check in a kernel whose integers are Lean's.", &["dag_dual_agreement", "axiom_footprint_lean_parity"]),
            proven("HTTP served natively, with verified TLS.", &["http_lifecycle_lock", "tcp_tls_lock"]),
            proven("One scheduler runs tasks on C, RISC-V and GPU.", &["c_family_scheduler_conformance_lock", "c_concurrency_scheduler_lock"]),
        ]
    });
    &LINES
}

/// A question the front page answers.
pub struct Faq {
    pub question: &'static str,
    pub answer: Claim,
    /// A run of the answer shown as a link, and where it goes.
    pub link: Option<(&'static str, &'static str)>,
}

pub fn faq() -> &'static [Faq] {
    static FAQ: LazyLock<Vec<Faq>> = LazyLock::new(|| {
        let learn = learn_figures();
        vec![
            Faq { question: "Is it really free?", answer: proven(free_answer(), &["contact_page", "version_lock"]), link: None },
            Faq {
                question: "Do I need to know logic already?",
                answer: proven(
                    format!(
                        "No. Learn teaches it step by step, in {} modules and {} exercises, free in the browser.",
                        learn.modules, learn.exercises
                    ),
                    &["learn_curriculum"],
                ),
                link: None,
            },
            Faq {
                question: "Which coding agents can use the instructions?",
                answer: agreed(
                    "Any agent that can read a web page and run commands in a terminal, such as Claude Code, Cursor or \
                     Codex. The instructions are plain text, so they paste into any of them.",
                ),
                link: None,
            },
            Faq {
                question: "Where is the source?",
                answer: agreed("On GitHub: github.com/Brahmastra-Labs/logicaffeine"),
                link: Some(("github.com/Brahmastra-Labs/logicaffeine", SOURCE_URL)),
            },
        ]
    });
    &FAQ
}

/// Every line the page states with its evidence: the guide's sentences and rules, the line on how
/// for each system, the comparison's lead and cards, the library's lines and features, and the FAQ's
/// answers.
pub fn every_claim() -> Vec<(String, &'static Evidence)> {
    let lines = instructions().iter().filter_map(|line| match line {
        Line::Say(claim) | Line::Rule(claim) => Some(claim),
        _ => None,
    });
    let hows = std::iter::once(how());
    let cards = std::iter::once(compare_lead()).chain(compare().iter().map(|card| &card.logos));
    let library = library().iter().chain(features().iter());
    let answers = faq().iter().map(|faq| &faq.answer);
    lines.chain(hows).chain(cards).chain(library).chain(answers).map(|claim| (claim.text.clone(), &claim.evidence)).collect()
}
