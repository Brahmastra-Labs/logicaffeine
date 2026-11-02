//! The front page as it is prerendered, held to the truth.
//!
//! The user (2026-10-01): "clean up the front page to make the demo a mini studio version with the
//! heading Code we can all read, logic and mathmatics built in, bring your own agent, with a button
//! below that says copy agent instructions with the button showing the copy symbol." Then
//! (2026-10-15): "The demo showing each example should be more modeled lke the file tree and IDE in
//! the Studio so when people get to the actual studio it looks basically the same."
//!
//! Below the heading, the copy button copies a five-line prompt, the one for the visitor's system:
//! who writes and who reads, the install that runs there, and the guide to read before writing,
//! served at /agent.txt. That guide is the instructions this suite has always held to the truth: every
//! command runs on the largo a reader installs (the latest GitHub release, whose own `--help` is
//! `fixtures/largo_release_help/`, recorded by `scripts/web/capture-release-help.sh`) and on this
//! tree's, every address is one the site serves, and every line that states a fact about LOGOS names
//! the suites that prove it. Below the prompt is the Studio itself in a frame, on a shelf of shipped
//! examples (`one_studio_lock` holds it to being the Studio's workbench). Where things are painted,
//! and how the copy button and the frame behave in a browser, is `scripts/web/test-site-e2e.mjs`.

#![cfg(feature = "web-tests")]

#[path = "web_common/page_html.rs"]
mod page_html;

#[path = "cli_common/mod.rs"]
mod common;

use clap::CommandFactory;
use logicaffeine_web::ui::components::icon::IconVariant;
use logicaffeine_web::ui::examples::{seeded_examples, Expected, ExampleSpec, Mode};
use logicaffeine_web::ui::pages::hardware_report::{analyze, HardwareKind};
use logicaffeine_web::ui::pages::landing::content::{self, Evidence, Install, Line, Status, System, Uses, AGENT_GUIDE_URL, COPY_LABEL, INSTRUCTIONS_ID};
use logicaffeine_web::ui::pages::contact::content::{free_answer, ASSISTANT_GUIDE_URL, INSTALL_UNIX, SOURCE_URL, INSTALL_WINDOWS};
use logicaffeine_web::ui::pages::studio_run::{run_logic, run_math, LogicRun};
use logicaffeine_web::ui::pages::studio_workbench::preview_document;
use logicaffeine_web::ui::router::{studio_file_url, Route};
use logicaffeine_web::ui::seo::pages as seo_pages;
use logicaffeine_web::ui::state::StudioMode;
use logicaffeine_compile::web_host::{assemble_preview, assemble_preview_painted};
use page_html::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------------------------
// The page, the instructions, the two CLIs
// ---------------------------------------------------------------------------------------------

/// The user's three sentences, in their words (with "mathematics" spelled).
const HEADING: [&str; 3] = ["Code we can all read.", "Logic and mathematics built in.", "Bring your own agent."];

fn landing() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| render_route("/"))
}

/// The page's own content: its `<main>`, without the site's nav and footer.
fn main_content() -> &'static str {
    elements(landing(), "main").into_iter().next().expect("the front page renders a <main>")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn public_dir() -> PathBuf {
    repo_root().join("apps/logicaffeine_web/public")
}

/// The instructions' lines that are commands to type, as they read.
fn run_lines() -> Vec<&'static str> {
    content::instructions().iter().filter_map(|line| if let Line::Run(command) = line { Some(*command) } else { None }).collect()
}

/// The command lines that run largo.
fn largo_lines() -> Vec<&'static str> {
    run_lines().into_iter().filter(|line| line.starts_with("largo ")).collect()
}

/// The steps' titles, in order.
fn steps() -> Vec<&'static str> {
    content::instructions().iter().filter_map(|line| if let Line::Step(title) = line { Some(*title) } else { None }).collect()
}

/// The lines of the step titled `title`, up to the next step.
fn step(title: &str) -> Vec<&'static Line> {
    content::instructions()
        .iter()
        .skip_while(|line| !matches!(line, Line::Step(t) if *t == title))
        .skip(1)
        .take_while(|line| !matches!(line, Line::Step(_)))
        .collect()
}

/// A command line split the way a POSIX shell splits it: on spaces, with double quotes holding a
/// word together.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    assert!(!quoted, "an unclosed quote in {line:?}");
    if started {
        words.push(word);
    }
    words
}

/// Every `largo <word>` the text names: the word after `largo ` when it is a word or a flag.
fn largo_mentions(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in text.match_indices("largo ") {
        if at > 0 && text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '/') {
            continue;
        }
        let word: String = text[at + "largo ".len()..].chars().take_while(|c| c.is_ascii_lowercase() || *c == '-').collect();
        if !word.is_empty() {
            found.push(word);
        }
    }
    found
}

/// Every web address in the text, without the punctuation that ends its sentence.
fn urls(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|word| word.contains("://"))
        .map(|word| word.trim_start_matches(|c| c == '"' || c == '(').trim_end_matches(|c: char| matches!(c, '.' | ',' | ':' | ')' | '"')).to_string())
        .collect()
}

/// The released largo, as its own `--help` describes it.
struct Release {
    /// `largo X.Y.Z`, as `largo --version` prints it.
    version: String,
    /// `largo --help`.
    top: String,
    /// Each command's `--help`, by command.
    commands: BTreeMap<String, String>,
}

fn release_dir() -> PathBuf {
    repo_root().join("crates/logicaffeine_tests/tests/fixtures/largo_release_help")
}

/// The commands a `--help` lists under `Commands:`.
fn listed_commands(help: &str) -> Vec<String> {
    help.lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| !line.trim().is_empty() && line.starts_with(' '))
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .filter(|name| name != "help")
        .collect()
}

fn release() -> &'static Release {
    static RELEASE: OnceLock<Release> = OnceLock::new();
    RELEASE.get_or_init(|| {
        let read = |name: &str| {
            let path = release_dir().join(name);
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        };
        let top = read("largo.txt");
        let commands = listed_commands(&top).into_iter().map(|name| {
            let help = read(&format!("{name}.txt"));
            (name, help)
        });
        Release { version: read("VERSION").trim().to_string(), commands: commands.collect(), top }
    })
}

/// The options a `--help` describes: every `-x` and `--long` that starts one of its option lines.
fn options(help: &str) -> BTreeSet<String> {
    help.lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with('-'))
        .flat_map(|line| line.split(", ").map(|part| part.split(|c: char| c.is_whitespace() || c == '=' || c == '<' || c == '[').next().unwrap_or("").to_string()))
        .filter(|option| option.starts_with('-'))
        .collect()
}

/// The commands this tree's largo defines, with their aliases.
fn tree_commands() -> BTreeSet<String> {
    logicaffeine_cli::cli::Cli::command()
        .get_subcommands()
        .flat_map(|command| std::iter::once(command.get_name().to_string()).chain(command.get_all_aliases().map(str::to_string)))
        .collect()
}

/// The top-level options this tree's largo takes.
fn tree_top_options() -> BTreeSet<String> {
    let cli = logicaffeine_cli::cli::Cli::command();
    let mut found: BTreeSet<String> = ["--help", "-h", "--version", "-V"].into_iter().map(str::to_string).collect();
    for arg in cli.get_arguments() {
        if let Some(long) = arg.get_long() {
            found.insert(format!("--{long}"));
        }
        if let Some(short) = arg.get_short() {
            found.insert(format!("-{short}"));
        }
    }
    found
}

/// The commands this tree has and the release does not.
fn unreleased_commands() -> Vec<String> {
    tree_commands().into_iter().filter(|command| !release().commands.contains_key(command)).collect()
}

/// Everything the front page publishes, lowercased: what its content says, its structured data,
/// and the title and description its head carries.
fn published_text() -> String {
    let data: Vec<String> = json_ld(landing()).iter().flat_map(json_strings).collect();
    format!(
        "{}\n{}\n{}\n{}",
        visible_text(main_content()),
        data.join("\n"),
        seo_pages::LANDING.title,
        seo_pages::LANDING.description
    )
    .to_lowercase()
}

/// The shipped Studio example at `path`.
fn example(path: &str) -> &'static ExampleSpec {
    seeded_examples().find(|spec| spec.vfs_path == path).unwrap_or_else(|| panic!("{path} is not a shipped Studio example"))
}

// ---------------------------------------------------------------------------------------------
// 1. The heading and the button below it
// ---------------------------------------------------------------------------------------------

#[test]
fn the_heading_is_the_users_three_sentences() {
    assert_eq!(content::HEADING, HEADING, "the heading's words");
    let h1s = elements(landing(), "h1");
    assert_eq!(h1s.len(), 1, "the front page has {} h1 elements", h1s.len());
    let lines: Vec<String> = elements(inner(h1s[0]), "span").into_iter().map(|span| visible_text(inner(span))).collect();
    assert_eq!(lines, HEADING, "the h1 is the three sentences, one per line");
    assert_eq!(visible_text(inner(h1s[0])), HEADING.join(" "), "the h1 says more than the three sentences");
    assert!(!published_text().contains("mathmatics"), "the page keeps the typo the user asked to fix");
    let own = format!("{}\n{}", visible_text(main_content()), seo_pages::LANDING.title).to_lowercase();
    assert!(!own.contains("debug your thoughts"), "the page or its title still carries the heading the user replaced");
}

/// Below the heading stand the one line on how (the user, 2026-10-31: the bare minimum), the box
/// for a reader who installs by hand, and three copy buttons, one per system with its mark, that
/// copy the prompt for that system (the user, 2026-11-01: a professional release).
#[test]
fn the_heading_leads_to_the_line_on_how_the_install_box_and_a_copy_button_per_system() {
    assert_eq!(COPY_LABEL, "Copy agent instructions");
    let top = element_by_id(landing(), "top");
    let after = &top[top.find("</h1>").expect("the hero holds the heading") + "</h1>".len()..];
    let how = elements(after, "p").into_iter().find(|p| has_class(&own_attributes(p), "landing-how")).expect("no line on how below the heading");
    assert_eq!(visible_text(inner(how)), content::how().text, "the line on how is not the content's");
    assert!(content::how().text.split(". ").all(|sentence| sentence.split_whitespace().count() <= 16), "the line on how runs long: {:?}", content::how().text);
    let between = &after[..after.find(how).unwrap()];
    assert_eq!(visible_text(between), "", "something stands between the heading and the line on how");

    let install = elements(after, "div").into_iter().find(|d| has_class(&own_attributes(d), "landing-install")).expect("no install box");
    assert!(visible_text(install).contains(content::INSTALL_HEADING), "the install box has no heading");
    let commands: Vec<String> = elements(install, "code").into_iter().map(|c| visible_text(inner(c))).collect();
    assert_eq!(commands, content::install_commands().map(|(_, command)| command.to_string()), "the install box does not show the install commands");
    assert_eq!(elements(install, "button").len(), commands.len(), "the install box has a copy button for each command");
    assert!(after.find(how).unwrap() < after.find(install).unwrap(), "the install box stands above the line on how");

    let buttons: Vec<&str> = elements(after, "button").into_iter().filter(|b| has_class(&own_attributes(b), "landing-copy")).collect();
    assert_eq!(buttons.len(), System::ALL.len(), "the hero has {} copy buttons, not one per system", buttons.len());
    assert!(after.find(install).unwrap() < after.find(buttons[0]).unwrap(), "the copy buttons stand above the install box");
    for (system, button) in System::ALL.iter().zip(&buttons) {
        let attrs = own_attributes(button);
        assert_eq!(attrs.get("type").map(String::as_str), Some("button"), "the {} button is not a plain button", system.name());
        assert_eq!(attrs.get("data-system").map(String::as_str), Some(system.slug()), "the buttons are not in the systems' order");
        assert_eq!(attrs.get("aria-controls").map(String::as_str), Some(INSTRUCTIONS_ID), "the {} button does not name the text it copies", system.name());
        assert_eq!(visible_text(inner(button)), system.copy_label(), "the {} button's label", system.name());
        assert!(visible_text(inner(button)).starts_with(COPY_LABEL), "the {} button's label does not start with the copy label", system.name());
        let svgs = elements(button, "svg");
        assert_eq!(svgs.len(), 1, "the {} button carries {} marks", system.name(), svgs.len());
        assert!(svgs[0].contains(&format!("d=\"{}\"", system.mark())), "the {} button's mark is not the system's", system.name());
        assert!(button.find("<svg").unwrap() < button.find(COPY_LABEL).unwrap(), "the {} mark does not lead the label", system.name());
    }
    assert_eq!(System::MacOs.install(), Install::Unix);
    assert_eq!(System::Linux.install(), Install::Unix);
    assert_eq!(System::Windows.install(), Install::Windows);
}

/// The page holds a prompt for each install, one preformatted block each, hidden until a browser
/// refuses the copy (the user, 2026-10-31: the instructions only copy), and the copy button copies
/// the one for the visitor's system, character for character.
#[test]
fn the_page_holds_exactly_the_text_the_button_copies_hidden_until_a_copy_is_refused() {
    let panel = element_by_id(landing(), INSTRUCTIONS_ID);
    assert!(own_attributes(panel).contains_key("hidden"), "the prompt shows on the page; it should only copy");
    assert!(landing().contains(".landing-prompt[hidden] { display: none; }"), "the hidden prompt has no rule keeping it off the page");
    assert!(landing().contains(".landing-instructions[hidden] { display: none; }"), "a hidden prompt block has no rule keeping it off the page");
    let blocks = elements(panel, "pre");
    assert_eq!(blocks.len(), Install::ALL.len(), "the instructions hold {} preformatted blocks, not one per install", blocks.len());
    for (install, block) in Install::ALL.iter().zip(blocks) {
        let attrs = own_attributes(block);
        assert_eq!(attrs.get("data-install").map(String::as_str), Some(install.name()), "the blocks are not marked by their install, in order");
        assert_eq!(preformatted_text(block), content::prompt_text(*install), "the {} block is not the text the button copies", install.name());
        let lines: Vec<String> = content::prompt(*install).iter().map(Line::text).collect();
        assert_eq!(content::prompt_text(*install), lines.join("\n"), "the copied text is not the prompt's lines");
        assert_eq!(attrs.get("tabindex").map(String::as_str), Some("0"), "the block cannot be reached by keyboard");
        assert_eq!(attrs.get("aria-label").map(String::as_str), Some("Agent instructions"), "the block has no name");
    }
}

// ---------------------------------------------------------------------------------------------
// 2. The instructions: their shape, their commands, their addresses
// ---------------------------------------------------------------------------------------------

#[test]
fn the_instructions_are_forty_to_eighty_short_lines() {
    let text = content::instructions_text();
    let n = text.lines().count();
    assert!((40..=140).contains(&n), "the instructions are {n} lines, not 40 to 140");
    for line in text.lines() {
        assert!(line.chars().count() <= 90, "a line runs past 90 characters: {line:?}");
        assert_eq!(line, line.trim_end(), "a line ends in spaces: {line:?}");
    }
    assert!(!text.contains("\n\n\n"), "the instructions hold two blank lines in a row");
}

/// The order a person and their agent need them in, from installing to working together.
/// The guide opens with what it is, plainly, for any agent (the user, 2026-11-01): no "pair with
/// me", no "you write, I read".
#[test]
fn the_guide_opens_plainly_for_any_agent() {
    let text = content::instructions_text();
    assert!(text.starts_with("LOGOS programmer instructions, for any coding agent"), "the guide opens with {:?}", text.lines().next());
    let opening: String = text.lines().take(3).collect::<Vec<_>>().join(" ");
    for word in ["easy to install", "lightweight", "modular", "any coding agent", "bring your own"] {
        assert!(opening.contains(word), "the guide's opening does not say {word:?}: {opening}");
    }
    // No "cannot" in agent text (the user, 2026-11-01): say can't, or leave the sentence out.
    for phrase in ["Pair with me", "You write the LOGOS", "I read it", "show me", "Show me", "tell me", "I mean", " me.", "cannot", "Cannot"] {
        assert!(!text.contains(phrase), "the guide still says {phrase:?}");
    }
    assert!(text.contains(content::CRATES_URL), "the guide does not point at the crates");
    for install in Install::ALL {
        let prompt = content::prompt_text(*install);
        assert!(prompt.starts_with("LOGOS programmer instructions, for any coding agent"), "the {} prompt opens with {:?}", install.name(), prompt.lines().next());
        for word in ["easy to install", "lightweight", "bring your own"] {
            assert!(prompt.contains(word), "the {} prompt does not say {word:?}: {prompt}", install.name());
        }
        assert!(!prompt.contains("Pair with me") && !prompt.contains("I read it") && !prompt.contains("cannot"), "the {} prompt is not plain: {prompt}", install.name());
    }
}

/// The language on one page (the user, 2026-11-02): every line short, every construct it names a
/// form the quickguide lists as working today, and the heal worker's primer the same lines.
#[test]
fn the_syntax_matrix_is_the_guides_and_the_heal_workers() {
    use logicaffeine_web::ui::pages::landing::content::SYNTAX_MATRIX;
    assert!(SYNTAX_MATRIX.len() >= 25, "the matrix is {} lines", SYNTAX_MATRIX.len());
    let text = content::instructions_text();
    for line in SYNTAX_MATRIX.iter().skip(1) {
        assert!(line.chars().count() <= 90, "a matrix line runs past 90 characters: {line:?}");
        assert!(text.contains(line), "the guide does not carry the matrix line {line:?}");
    }
    let guide = std::fs::read_to_string(repo_root().join("LOGOS_QUICKGUIDE.md")).unwrap();
    for form in ["## Main", "## To add", "## A Point has:", "## A Shape is one of:", "## Page", "## Route", "## Test:", "## Theorem:", "Let mutable", "Increase", "Inspect", "When ", "Push ", "Pop from", "items ", "through", "combined with", "is divisible by", "is between", "always be", "on-click=", "bind=", "keyed by", "Style:", "Proof: Auto."] {
        assert!(guide.contains(form), "the matrix names {form:?}, which the quickguide does not list");
    }
    let primer = std::fs::read_to_string(repo_root().join("services/heal/src/primer.js")).expect("the heal worker's primer");
    let start = primer.find("PRIMER_LINES = [").expect("the primer's lines") + "PRIMER_LINES = ".len();
    let end = primer[start..].find("];").expect("the primer's lines end") + start + 1;
    let array = primer[start..end].trim_end_matches(']').trim_end().trim_end_matches(',').to_string() + "]";
    let lines: Vec<String> = serde_json::from_str(&array).expect("the primer's lines are JSON strings");
    assert_eq!(lines, SYNTAX_MATRIX.iter().map(|l| l.to_string()).collect::<Vec<_>>(), "the heal worker's primer is not the guide's matrix");
}

#[test]
fn the_steps_run_from_installing_to_working_together() {
    assert_eq!(
        steps(),
        [
            "1. Get the source",
            "2. Install largo, the LOGOS command line tool",
            "3. Start a website",
            "4. Learn the language before you write it",
            "5. Build the site",
            "6. Programs",
            "7. Logic",
            "8. Math",
            "9. Hardware",
            "10. Working with the user",
        ],
        "the steps, in order"
    );
}

#[test]
fn every_command_line_runs_a_program_a_reader_has() {
    let runs = run_lines();
    assert!(runs.len() >= 10, "the instructions give only {} commands", runs.len());
    for line in runs {
        let program = shell_words(line).into_iter().next().unwrap_or_default();
        assert!(["largo", "cd", "curl", "powershell", "irm", "git"].contains(&program.as_str()), "{line:?} runs {program:?}");
    }
}

#[test]
fn every_largo_command_line_parses_with_this_trees_cli() {
    let lines = largo_lines();
    assert!(lines.len() >= 8, "the instructions run largo only {} times", lines.len());
    for line in lines {
        let words = shell_words(line);
        match logicaffeine_cli::cli::Cli::command().try_get_matches_from(&words) {
            Ok(_) => {}
            Err(e) if matches!(e.kind(), clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp) => {}
            Err(e) => panic!("the instructions teach a command this tree's largo rejects: {line:?}\n{e}"),
        }
    }
}

#[test]
fn every_largo_command_line_runs_on_the_release_the_install_script_installs() {
    let release = release();
    let lines = largo_lines();
    assert!(lines.len() >= 8, "the instructions run largo only {} times", lines.len());
    for line in lines {
        let words = shell_words(line);
        let word = &words[1];
        if word.starts_with('-') {
            assert!(options(&release.top).contains(word), "{} takes no top-level {word}: {line:?}", release.version);
            continue;
        }
        let help = release.commands.get(word).unwrap_or_else(|| panic!("{} has no `largo {word}` ({line:?})", release.version));
        for flag in words[2..].iter().filter(|w| w.starts_with('-')) {
            assert!(options(help).contains(flag), "{}'s `largo {word}` takes no {flag}: {line:?}", release.version);
        }
    }
}

#[test]
fn every_largo_command_the_text_names_is_in_the_release_and_the_tree() {
    let text = content::instructions_text();
    let mentions = largo_mentions(text);
    assert!(mentions.len() >= 14, "the instructions name largo commands only {} times", mentions.len());
    let release = release();
    let (top_release, top_tree) = (options(&release.top), tree_top_options());
    let tree = tree_commands();
    for word in mentions {
        if word.starts_with('-') {
            assert!(top_release.contains(&word) && top_tree.contains(&word), "`largo {word}` is not a top-level option of both the release and the tree");
        } else {
            assert!(release.commands.contains_key(&word), "`largo {word}` is not in {}", release.version);
            assert!(tree.contains(&word), "`largo {word}` is not in this tree's CLI");
        }
    }
}

#[test]
fn no_command_the_release_lacks_reaches_the_page() {
    let unreleased = unreleased_commands();
    let text = published_text();
    for command in &unreleased {
        assert!(!text.contains(&format!("largo {command}")), "the page names `largo {command}`, which {} does not have", release().version);
    }
}

#[test]
fn the_release_fixture_is_the_whole_help_of_one_release() {
    let release = release();
    let version = release.version.strip_prefix("largo ").unwrap_or_else(|| panic!("VERSION reads {:?}", release.version));
    assert!(version.split('.').count() == 3 && version.split('.').all(|n| n.parse::<u32>().is_ok()), "VERSION is not X.Y.Z: {version:?}");
    assert!(release.top.starts_with("The LOGOS build tool") && release.top.contains("Usage: largo"), "largo.txt is not `largo --help`");
    assert!(release.commands.len() >= 20, "the release lists only {} commands", release.commands.len());
    for (name, help) in &release.commands {
        assert!(help.contains(&format!("Usage: largo {name}")), "{name}.txt is not `largo {name} --help`");
    }
    let mut files: Vec<String> = std::fs::read_dir(release_dir()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    files.sort();
    let mut want: Vec<String> = release.commands.keys().map(|name| format!("{name}.txt")).chain(["VERSION".to_string(), "largo.txt".to_string()]).collect();
    want.sort();
    assert_eq!(files, want, "the fixture holds files that are not the release's help");
    let script = repo_root().join("scripts/web/capture-release-help.sh");
    assert!(std::fs::read_to_string(&script).is_ok_and(|s| s.contains("largo_release_help")), "nothing rerecords the fixture");
}

#[test]
fn every_address_in_the_instructions_is_one_the_site_serves() {
    let text = content::instructions_text();
    let found = urls(text);
    assert!(found.len() >= 5, "the instructions give only {} addresses", found.len());
    let routes = logicaffeine_web::sitemap::prerender_routes();
    for url in found {
        // The site `largo serve` puts on the reader's machine, and the source on GitHub.
        if url == content::SERVE_URL || url == SOURCE_URL {
            continue;
        }
        let rest = url.strip_prefix("https://logicaffeine.com").unwrap_or_else(|| panic!("{url} is not on logicaffeine.com"));
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        if path == "/studio" {
            let file = query.strip_prefix("file=").unwrap_or_else(|| panic!("{url} opens the Studio on no file"));
            let spec = example(&format!("/{file}"));
            assert_eq!(rest, studio_file_url(spec.vfs_path), "{url} is not how the site links that example");
        } else if public_dir().join(path.trim_start_matches('/')).is_file() {
        } else {
            assert!(routes.iter().any(|route| route == path), "{url} is neither a file in public/ nor a page the site prerenders");
        }
    }
}

#[test]
fn the_install_commands_are_the_sites() {
    let runs = run_lines();
    assert!(runs.contains(&INSTALL_UNIX), "the instructions do not give {INSTALL_UNIX:?}");
    assert!(runs.contains(&INSTALL_WINDOWS), "the instructions do not give {INSTALL_WINDOWS:?}");
    for script in ["install.sh", "install.ps1"] {
        assert!(public_dir().join(script).is_file(), "the site does not serve {script}");
    }
    let install = step("2. Install largo, the LOGOS command line tool");
    assert!(install.iter().any(|line| matches!(line, Line::Run(c) if *c == INSTALL_UNIX)), "the Unix install is not in step 2");
}

#[test]
fn the_agent_reads_the_whole_guide_before_it_writes() {
    let text = content::instructions_text();
    assert_eq!(ASSISTANT_GUIDE_URL, "https://logicaffeine.com/llms-full.txt");
    let guide = text.find(ASSISTANT_GUIDE_URL).expect("the instructions name the guide");
    let first_change = text.find("5. Build the site").expect("the instructions have the agent build the site");
    assert!(guide < first_change, "the agent changes the site before it reads the guide");
    assert!(public_dir().join("llms-full.txt").is_file(), "the site does not serve llms-full.txt");
    let clone = text.find(content::CLONE_SOURCE).expect("the instructions clone the source");
    assert!(clone < first_change, "the agent changes the site before it has the source to read");
    let source_step = text.find("1. Get the source").expect("the source is step 1");
    let install_step = text.find("2. Install largo").expect("the install is step 2");
    assert!(source_step < clone && clone < install_step, "the clone is not in step 1, before the install");
}

/// "It also covers features newer than the release you installed": true while the guide names a
/// command the release lacks. When a release catches the guide up, this goes red and the line goes.
#[test]
fn the_guide_covers_commands_newer_than_the_release() {
    let claim = "It also covers features newer than the release you installed.";
    let guide = std::fs::read_to_string(public_dir().join("llms-full.txt")).unwrap();
    let newer: Vec<String> = unreleased_commands().into_iter().filter(|command| guide.contains(&format!("largo {command}"))).collect();
    if newer.is_empty() {
        assert!(!content::instructions_text().contains(claim), "the instructions warn that the guide runs ahead of {}, and it does not", release().version);
    } else {
        assert!(content::instructions_text().contains(claim), "the guide names {newer:?}, newer than {}, and the instructions do not say so", release().version);
    }
}

/// Hardware lives in the Studio, because the release has no hardware command.
#[test]
fn hardware_points_at_the_studio_because_the_release_has_no_hardware_command() {
    let release = release();
    for help in std::iter::once(&release.top).chain(release.commands.values()) {
        for word in ["SystemVerilog", "SVA", "hardware", "Verilog"] {
            assert!(!help.contains(word), "{} now mentions {word}: the instructions' hardware step should name its command", release.version);
        }
    }
    let lines = step("9. Hardware");
    let opens: Vec<&String> = lines.iter().filter_map(|line| if let Line::Open(url) = line { Some(url) } else { None }).collect();
    assert_eq!(opens.len(), 1, "the hardware step opens {} pages", opens.len());
    let file = opens[0].split("file=").nth(1).expect("the hardware step opens the Studio on a file");
    let spec = example(&format!("/{file}"));
    assert!(spec.mode == Mode::Hardware && matches!(spec.expected, Expected::SvaSynthesizes), "{} is not an English spec the Studio turns into an assertion", spec.vfs_path);
}

#[test]
fn math_points_at_the_studio_for_the_kernel() {
    let lines = step("8. Math");
    let opens: Vec<&String> = lines.iter().filter_map(|line| if let Line::Open(url) = line { Some(url) } else { None }).collect();
    assert_eq!(opens.len(), 1, "the math step opens {} pages", opens.len());
    let file = opens[0].split("file=").nth(1).expect("the math step opens the Studio on a file");
    let spec = example(&format!("/{file}"));
    assert!(spec.mode == Mode::Math && matches!(spec.expected, Expected::KernelAllStatementsOk), "{} is not a kernel document that checks clean", spec.vfs_path);
}

#[test]
fn every_claim_names_a_suite_that_exists() {
    let tests = repo_root().join("crates/logicaffeine_tests/tests");
    let claims = content::every_claim();
    assert!(claims.len() >= 20, "the page states only {} claims; the content lost its claims", claims.len());
    let mut proven = 0;
    for (text, evidence) in claims {
        if let Evidence::Suites(suites) = evidence {
            proven += 1;
            assert!(!suites.is_empty(), "{text:?} names no suite");
            for suite in *suites {
                assert!(tests.join(format!("{suite}.rs")).is_file(), "{text:?} cites the suite {suite}, which does not exist");
            }
        }
    }
    assert!(proven >= 15, "only {proven} lines name the suites that prove them");
}

/// A rule of the language is a fact about LOGOS, never a working agreement.
#[test]
fn every_rule_and_every_line_about_logos_is_proven() {
    let rules = content::instructions().iter().filter(|line| matches!(line, Line::Rule(_))).count();
    assert!(rules >= 4, "the instructions give {rules} rules of the language");
    for line in content::instructions() {
        if let Line::Rule(claim) = line {
            assert!(matches!(claim.evidence, Evidence::Suites(_)), "the rule {:?} names no suite", claim.text);
        }
    }
    let agreements: Vec<&str> = content::instructions()
        .iter()
        .filter_map(|line| match line {
            Line::Say(claim) | Line::Rule(claim) if matches!(claim.evidence, Evidence::Agreement) => Some(claim.text.as_str()),
            _ => None,
        })
        .collect();
    for text in agreements {
        for word in ["largo ", "LOGOS ", "kernel", "prints", "proves", "Studio"] {
            assert!(!text.contains(word), "{text:?} is a working agreement but talks about the product ({word:?}); name its suites");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// 3. The instructions' steps, run on this tree's largo as a reader runs them
// ---------------------------------------------------------------------------------------------

/// Run a step's command lines in order from `dir`, with nothing on PATH: `cd` moves, every other
/// line runs this tree's largo. The output of each largo command, in order.
fn run_step(title: &str, dir: &Path) -> Vec<(String, std::process::Output)> {
    let nothing = tempfile::tempdir().unwrap();
    let mut here = dir.to_path_buf();
    let mut outputs = Vec::new();
    for line in step(title) {
        let Line::Run(command) = line else { continue };
        let words = shell_words(command);
        match words[0].as_str() {
            "cd" => here = here.join(&words[1]),
            // The clone reaches GitHub; a test runs nothing that needs the network.
            "git" => {}
            "largo" => {
                let output = common::largo().args(&words[1..]).current_dir(&here).env("PATH", nothing.path()).output().expect("largo runs");
                outputs.push((command.to_string(), output));
            }
            other => panic!("step {title:?} runs {other:?}"),
        }
    }
    outputs
}

#[test]
fn step_five_starts_a_program_that_prints_hello_world_with_no_rust_toolchain() {
    let dir = tempfile::tempdir().unwrap();
    let outputs = run_step("6. Programs", dir.path());
    assert!(outputs.len() >= 2, "step 6 runs largo {} times", outputs.len());
    for (command, output) in &outputs {
        assert!(output.status.success(), "`{command}` failed:\n{}{}", common::stdout(output), common::stderr(output));
    }
    let said = content::instructions_text();
    assert!(said.contains("It prints Hello, world!"), "step 6 does not say what the program prints");
    let (last, output) = outputs.last().unwrap();
    assert_eq!(last, "largo run --interpret", "step 6 does not end by running the program");
    assert_eq!(common::stdout(output).trim(), "Hello, world!", "the new project prints something else");
    let project = dir.path().join("hello");
    for file in ["Largo.toml", "src/main.lg", ".gitignore"] {
        assert!(project.join(file).is_file(), "largo new did not write {file}, which step 6 names");
        assert!(said.contains(file), "step 6 does not name {file}");
    }
}

/// Step 2 is the point of the guide (Tristen, 2026-11-01, DAT-105): a site scaffolded, its test
/// green and `largo serve` the last thing run, with the page, the project's own guide and the serve
/// address named, before the agent reads anything else.
#[test]
fn step_two_scaffolds_a_site_whose_test_passes_and_ends_serving_it() {
    let lines = step("3. Start a website");
    let runs: Vec<&str> = lines.iter().filter_map(|line| if let Line::Run(c) = line { Some(*c) } else { None }).collect();
    assert_eq!(runs, ["largo new mysite --site", "cd mysite", "largo test", "largo serve"], "step 3 runs {runs:?}");
    let dir = tempfile::tempdir().unwrap();
    let nothing = tempfile::tempdir().unwrap();
    let new = common::largo().args(["new", "mysite", "--site"]).current_dir(dir.path()).env("PATH", nothing.path()).output().expect("largo runs");
    assert!(new.status.success(), "`largo new mysite --site` failed:\n{}{}", common::stdout(&new), common::stderr(&new));
    let site = dir.path().join("mysite");
    for file in ["pages/app.lg", "AGENTS.md", "Largo.toml"] {
        assert!(site.join(file).is_file(), "largo new --site did not write {file}");
    }
    let test = common::largo().args(["test"]).current_dir(&site).env("PATH", nothing.path()).output().expect("largo runs");
    assert!(test.status.success(), "`largo test` on the new site failed:\n{}{}", common::stdout(&test), common::stderr(&test));
    let page = std::fs::read_to_string(site.join("pages/app.lg")).unwrap();
    assert!(page.contains("## Test") && page.contains("Click"), "the scaffold's page has no test that clicks");
    let said = content::instructions_text();
    for word in ["pages/app.lg", "AGENTS.md", content::SERVE_URL] {
        assert!(said.contains(word), "the instructions do not name {word}");
    }
    let text: Vec<String> = lines.iter().map(|line| line.text()).collect();
    assert!(text.iter().any(|l| l.contains("second terminal") || l.contains("background")), "step 3 does not say largo serve keeps running");
}

#[test]
fn step_five_reads_every_reading_and_proves_a_theorem() {
    let lines = step("7. Logic");
    let runs: Vec<&str> = lines.iter().filter_map(|line| if let Line::Run(c) = line { Some(*c) } else { None }).collect();
    let logic = runs.iter().find(|c| c.starts_with("largo logic ")).expect("step 7 runs no largo logic");
    let dir = tempfile::tempdir().unwrap();
    let words = shell_words(logic);
    let out = common::largo_in(dir.path(), &words[1..].iter().map(String::as_str).collect::<Vec<_>>());
    assert!(out.status.success(), "`{logic}` failed: {}", common::stderr(&out));
    let printed = common::stdout(&out);
    let readings: Vec<&str> = printed.lines().filter(|l| l.split_once(". ").is_some_and(|(n, _)| n.parse::<u32>().is_ok())).collect();
    assert!(readings.len() >= 2, "the sentence step 5 reads has {} readings, so it shows no ambiguity:\n{printed}", readings.len());
    assert!(readings.iter().all(|r| r.contains('∀') || r.contains('∃')), "a reading is not first-order logic:\n{printed}");

    let prove = runs.iter().find(|c| c.starts_with("largo prove ")).expect("step 5 runs no largo prove");
    let file = shell_words(prove)[2].clone();
    let theorem = seeded_examples()
        .find(|spec| spec.mode == Mode::Logic && matches!(spec.expected, Expected::Proves) && spec.source.contains("Given:"))
        .expect("a shipped Logic theorem");
    std::fs::write(dir.path().join(&file), theorem.source).unwrap();
    let out = common::largo_in(dir.path(), &["prove", &file]);
    assert!(out.status.success(), "`{prove}` on {} failed:\n{}{}", theorem.vfs_path, common::stdout(&out), common::stderr(&out));
    assert!(common::stdout(&out).contains('✓'), "`{prove}` proved nothing:\n{}", common::stdout(&out));
}

#[test]
fn step_six_proves_a_theory_from_its_axioms() {
    assert!(content::instructions_text().contains("## Theory"), "step 6 does not say largo prove proves a ## Theory");
    let theory = seeded_examples().find(|spec| matches!(spec.expected, Expected::TheoryAllVerified)).expect("a shipped ## Theory");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("theory.lg"), theory.source).unwrap();
    let out = common::largo_in(dir.path(), &["prove", "theory.lg"]);
    let printed = common::stdout(&out);
    assert!(out.status.success(), "largo prove on {} failed:\n{printed}{}", theory.vfs_path, common::stderr(&out));
    let proved = printed.lines().filter(|l| l.starts_with('✓')).count();
    assert!(proved >= 3 && !printed.contains('✗'), "largo prove did not prove every theorem of {}:\n{printed}", theory.vfs_path);
}

#[test]
fn largo_check_explains_a_mistake_with_its_line() {
    let said = content::instructions_text();
    assert!(said.contains("largo check explains each mistake it finds in plain English"), "step 4 does not say what largo check does");
    let dir = tempfile::tempdir().unwrap();
    common::scaffold(dir.path(), "mistake");
    std::fs::write(dir.path().join("src/main.lg"), "## Main\n\nFrobnicate the x.\n").unwrap();
    let out = common::largo_in(dir.path(), &["check"]);
    let said = format!("{}{}", common::stdout(&out), common::stderr(&out));
    assert!(!out.status.success(), "largo check passed a program with no statement verb:\n{said}");
    assert!(said.contains("Frobnicate the x.") && said.contains('3'), "largo check does not show the line it stopped on:\n{said}");
}

/// The four rules the instructions start with, each run as written.
#[test]
fn the_rules_to_start_with_hold() {
    let rules: Vec<&str> = content::instructions()
        .iter()
        .filter_map(|line| if let Line::Rule(claim) = line { Some(claim.text.as_str()) } else { None })
        .collect();
    assert_eq!(rules.len(), 4, "the instructions give {} rules", rules.len());
    let all = rules.join("\n");
    for phrase in ["one English sentence", "Let x be 5.", "Set x to 6.", "item 1 of xs", "If name is equal to \"Ada\":"] {
        assert!(all.contains(phrase), "no rule says {phrase:?}:\n{all}");
    }
    let programs = [
        ("## Main\n\nLet x be 5.\nSet x to 6.\nShow x.\n", "6"),
        ("## Main\n\nLet xs be [10, 20, 30].\nShow item 1 of xs.\n", "10"),
        ("## Main\n\nLet name be \"Ada\".\nIf name is equal to \"Ada\":\n    Show \"same\".\n", "same"),
    ];
    for (program, shows) in programs {
        let dir = tempfile::tempdir().unwrap();
        common::scaffold(dir.path(), "rule");
        std::fs::write(dir.path().join("src/main.lg"), program).unwrap();
        let out = common::largo_in(dir.path(), &["run", "--interpret"]);
        assert!(out.status.success(), "the rule's program failed:\n{program}\n{}", common::stderr(&out));
        assert_eq!(common::stdout(&out).trim(), shows, "the rule's program printed something else:\n{program}");
    }
}

// ---------------------------------------------------------------------------------------------
// 4. The mini Studio
// ---------------------------------------------------------------------------------------------

/// Whether the shipped `spec` is an example of what the tab's Studio mode opens: a web page for Web,
/// an example of the same mode otherwise.
fn opens_in(spec: &ExampleSpec, mode: StudioMode) -> bool {
    match mode {
        StudioMode::Web => spec.mode == Mode::Code && matches!(spec.expected, Expected::WebPage),
        StudioMode::Logic => spec.mode == Mode::Logic,
        StudioMode::Math => spec.mode == Mode::Math,
        StudioMode::Hardware => spec.mode == Mode::Hardware,
        StudioMode::Code => spec.mode == Mode::Code && !matches!(spec.expected, Expected::WebPage),
    }
}

/// The front page's Studio frame shelves one shipped example for each kind of work, in the Studio's
/// own folders, and opens on the website in a website (the user, 2026-10-01: "The front hero can have
/// a section with the studio that displays the code like the studio and the web preview that changes
/// live as the code changs. Then we can make a cool example to show people a website within a
/// website.").
#[test]
fn the_front_page_studio_shelves_a_file_of_every_mode() {
    let shelf = content::shelf();
    assert_eq!(shelf.files.len(), 7, "the shelf holds {} files", shelf.files.len());
    assert_eq!(shelf.files.iter().collect::<BTreeSet<_>>().len(), shelf.files.len(), "the shelf repeats a file");
    assert_eq!(shelf.files.first(), Some(&shelf.opens), "the frame does not open on the shelf's first file");
    assert_eq!(shelf.opens, "/examples/code/web/website.logos", "the frame's first file");
    let modes: Vec<StudioMode> = shelf.files.iter().map(|file| StudioMode::for_file(file, example(file).source).unwrap_or_else(|| panic!("{file} opens in no mode"))).collect();
    for mode in [StudioMode::Web, StudioMode::Code, StudioMode::Logic, StudioMode::Math, StudioMode::Hardware] {
        assert!(modes.contains(&mode), "the shelf has no {mode:?} file: {modes:?}");
    }
    for file in shelf.files {
        assert!(file.starts_with("/examples/"), "{file} is not in the Studio's examples folder");
    }
}

/// The website in a website is a real shipped example: the whole page, with its own tests, so the
/// Studio and the example registry hold it to working.
#[test]
fn the_web_tab_is_the_website_in_a_website() {
    let opens = content::shelf().opens;
    assert_eq!(opens, "/examples/code/web/website.logos", "the frame's first file");
    let file = repo_root().join("apps/logicaffeine_web/src/ui/examples/website.logos");
    let page = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
    assert!(page.contains("## Page /") && page.contains("## Test:"), "the example is not a page with its own tests");
    let wasm = logicaffeine_compile::compile::compile_to_wasm(&page).unwrap_or_else(|e| panic!("the page does not compile to WebAssembly: {e:?}"));
    assert!(wasm.starts_with(b"\0asm"), "the page compiled to something other than a WebAssembly module");
    let spec = example(opens);
    assert!(opens_in(spec, StudioMode::Web), "the registry ships {opens} as something other than a web page");
    assert_eq!(spec.source, page, "the registry ships another text at {opens}");
    assert_eq!(StudioMode::for_file(opens, &page), Some(StudioMode::Web), "{opens} does not open in Web");
}

/// A Code file's run through the Studio's own interpreter entry, in a runtime that drives its
/// scheduler for real, as the example health suite runs them.
fn run_code(source: &str) -> logicaffeine_compile::interpreter::InterpreterResult {
    let runtime = logicaffeine_system::tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a Tokio runtime");
    runtime.block_on(logicaffeine_compile::interpret_for_ui_baseline_with_args(source, &[]))
}

/// Each shelf file does in the Studio's own runners what the frame shows it doing.
#[test]
fn each_shelf_file_runs_in_the_studios_own_runner() {
    for file in content::shelf().files {
        let spec = example(file);
        let source = spec.source;
        match StudioMode::for_file(file, source).expect("a shelf file opens in a mode") {
            StudioMode::Web => {
                let doc = assemble_preview_painted(source).unwrap_or_else(|e| panic!("{file} does not build: {e:?}"));
                let plain = assemble_preview(source).expect("the plain preview builds");
                assert_ne!(doc, plain, "{file}'s preview carries no first paint");
                assert!(doc.contains("<body data-logos-prerendered=\"1\">"), "{file}'s preview is not marked for the page to adopt");
                assert!(doc.contains("A website in a website"), "{file}'s preview does not paint the page's heading");
            }
            StudioMode::Code => {
                let run = run_code(source);
                assert!(run.error.is_none(), "{file} fails: {:?}", run.error);
                match spec.expected {
                    Expected::OutputExact(want) => assert_eq!(run.lines.join("\n").trim_end(), want.trim_end(), "{file} prints other than its verified output"),
                    Expected::OutputContains(needles) => {
                        for needle in needles {
                            assert!(run.lines.iter().any(|line| line.contains(needle)), "{file} does not print {needle:?}");
                        }
                    }
                    _ => panic!("{file} has no verified output"),
                }
            }
            StudioMode::Logic => match run_logic(source) {
                LogicRun::Sentences(result, _) => {
                    assert!(result.error.is_none(), "{file} fails: {:?}", result.error);
                    let logic = result.simple_logic.or(result.logic).unwrap_or_else(|| panic!("{file} reads no logic"));
                    if let Expected::CompilesToFol(predicates) = spec.expected {
                        for predicate in predicates.iter() {
                            assert!(logic.contains(predicate) || result.readings.iter().any(|r| r.contains(predicate)), "{file}'s logic lacks {predicate}:\n{logic}");
                        }
                    }
                }
                LogicRun::Theorem(Ok(theorem)) => assert!(theorem.verified, "{file}'s theorem is not proved and kernel-checked"),
                LogicRun::Theorem(Err(e)) => panic!("{file}'s theorem does not compile: {e}"),
                LogicRun::Empty => panic!("{file} holds no sentence"),
            },
            StudioMode::Math => {
                let run = run_math(source);
                assert!(!run.lines.is_empty(), "{file} runs no statement");
                for line in &run.lines {
                    assert!(line.success, "{file}'s statement fails: {:?} -> {:?}", line.input, line.output);
                }
            }
            StudioMode::Hardware => {
                let (kind, report) = analyze(source).unwrap_or_else(|| panic!("{file} holds no design"));
                assert_eq!(kind, HardwareKind::EnglishSpec, "{file} is not an English spec");
                assert!(report.error.is_none(), "{file} fails: {:?}", report.error);
                assert!(report.sections.iter().any(|s| s.label == "SystemVerilog Assertion" && s.body.contains("assert")), "{file} makes no assertion");
                assert_eq!(report.verdict.as_ref().map(|(_, ok)| *ok), Some(Some(true)), "{file}'s assertion is not certified against its spec");
            }
        }
    }
}

/// The prerender is the app's first frame: the Studio itself, opened on the website, its page in the
/// editor and its preview already painted, the same document the app builds.
#[test]
fn the_front_page_studio_prerenders_its_page_and_its_preview() {
    let studio = element_by_id(landing(), "studio-frame");
    let opens = content::shelf().opens;
    let source = example(opens).source;
    let tablists: Vec<_> = tags(studio, "nav").into_iter().chain(tags(studio, "div")).filter(|t| t.get("role").is_some_and(|r| r == "tablist")).collect();
    assert!(tablists.iter().all(|t| has_class(t, "mobile-tabs")), "the frame has a tab list of its own beside the phone's switch");
    let selected: Vec<String> = tags(studio, "div").into_iter().filter(|t| has_class(t, "tree-item") && has_class(t, "selected")).map(|t| t.get("title").cloned().unwrap_or_default()).collect();
    assert_eq!(selected, [opens], "the frame's selected file");
    let titles: Vec<String> = elements(studio, "div").into_iter().filter(|d| has_class(&own_attributes(d), "panel-header")).map(|d| visible_text(inner(elements(d, "span")[0]))).collect();
    assert_eq!(titles, ["Code Editor", "Preview"], "the frame's panels");

    let highlight = elements(studio, "div").into_iter().find(|d| has_class(&own_attributes(d), "code-editor-highlight")).expect("the frame prerenders no editor");
    assert_eq!(preformatted_text(highlight), source, "the prerendered editor does not show the page");
    let textarea = tags(studio, "textarea");
    assert_eq!(textarea.len(), 1, "the frame has {} editors", textarea.len());
    assert_eq!(textarea[0].get("value").map(String::as_str), Some(source), "the editor's text is not the page");

    let frames = tags(studio, "iframe");
    assert_eq!(frames.len(), 1, "the frame has {} previews", frames.len());
    let frame = &frames[0];
    assert_eq!(frame.get("sandbox").map(String::as_str), Some("allow-scripts"), "the preview is not sandboxed as the Studio sandboxes it");
    assert!(frame.get("title").is_some_and(|t| !t.is_empty()), "the preview has no title for a screen reader");
    let want = preview_document(source);
    assert_eq!(frame.get("srcdoc"), Some(&want), "the prerendered preview is not the document the app builds");
    let painted = assemble_preview_painted(source).expect("the page builds");
    assert!(want.ends_with(&painted[painted.find("<body").unwrap()..]), "the prerendered preview does not carry the page's first paint");

    let toolbar = elements(studio, "div").into_iter().find(|d| has_class(&own_attributes(d), "studio-toolbar")).expect("the frame has no toolbar");
    for action in ["Run", "Execute", "Compile"] {
        assert!(elements(toolbar, "button").iter().all(|b| !visible_text(inner(b)).contains(action)), "the Web page's toolbar offers {action}; its preview is live");
    }
    assert!(links(studio).iter().any(|(attrs, text)| text == "Open in Studio" && attrs.get("href") == Some(&studio_file_url(opens))), "the frame does not open {opens} in the Studio");
}

/// The mini Studio drives the Studio's runners. It keeps no runner of its own, so the front page can
/// never show a result the Studio would not.
#[test]
fn the_front_page_keeps_no_runner_of_its_own() {
    let dir = repo_root().join("apps/logicaffeine_web/src/ui/pages/landing");
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        let source = std::fs::read_to_string(&path).unwrap();
        for copy in ["fn parse_math_statements", "Repl::new", "compile_theorem_for_ui", "compile_for_ui(", "fn execute_math"] {
            assert!(!source.contains(copy), "{} carries {copy:?}; run through studio_run and the Studio's own calls", path.display());
        }
    }
}

// ---------------------------------------------------------------------------------------------
// 5. The words: plain punctuation, no unbacked claim
// ---------------------------------------------------------------------------------------------

/// The page's prose as a reader sees it: code (the editor, the instructions' block, program output)
/// is not prose, so it is left out here and its words are read where they are written.
fn prose(html: &str) -> String {
    let mut out = html.to_string();
    for editor in elements(html, "div").into_iter().filter(|d| has_class(&own_attributes(d), "code-editor")) {
        out = out.replace(editor, " ");
    }
    for tag in ["pre", "code", "samp", "textarea"] {
        for element in elements(&out.clone(), tag) {
            out = out.replace(element, " ");
        }
    }
    visible_text(&out)
}

/// The user (2026-10-01): no em dashes and no semicolons in the front page's visible text, title,
/// description or JSON-LD. The prompts and the guide they send an agent to are prose to the person who
/// reads them, so they are held to it too; a program's code is not prose, and keeps the punctuation
/// its language needs, but no shelf file carries an em dash.
#[test]
fn the_front_page_uses_no_em_dash_or_semicolon() {
    let data: Vec<String> = json_ld(landing()).iter().flat_map(json_strings).collect();
    let mut surfaces = vec![
        ("the page's prose".to_string(), prose(main_content())),
        ("the guide at /agent.txt".to_string(), content::instructions_text().to_string()),
        ("the title".to_string(), seo_pages::LANDING.title.to_string()),
        ("the description".to_string(), seo_pages::LANDING.description.to_string()),
        ("the structured data".to_string(), data.join("\n")),
    ];
    for install in Install::ALL {
        surfaces.push((format!("the {} prompt", install.name()), content::prompt_text(*install).to_string()));
    }
    for file in content::shelf().files {
        assert!(!example(file).source.contains('\u{2014}'), "{file} carries an em dash");
    }
    assert!(surfaces[0].1.contains(HEADING[0]) && surfaces[0].1.len() > 100, "the prose reader read nothing of the page: {:?}", surfaces[0].1);
    for (what, text) in surfaces {
        for (mark, name) in [('\u{2014}', "an em dash"), (';', "a semicolon")] {
            assert!(!text.contains(mark), "{what} carries {name}: {:?}", text.split('\n').find(|l| l.contains(mark)).unwrap_or(""));
        }
        // The language on one page quotes forms (`x - y`, `-> Int`): code, not prose.
        let prose: String = text.lines().filter(|l| !logicaffeine_web::ui::pages::landing::content::SYNTAX_MATRIX.contains(l)).collect::<Vec<_>>().join("\n");
        assert!(!prose.contains(" \u{2013} ") && !prose.contains(" - "), "{what} uses a dash between words");
    }
}

/// Claims the product does not back, each with why, matched without regard to case against
/// everything the front page publishes.
const BANNED_CLAIMS: &[(&str, &str)] = &[
    ("via LLVM", "LOGOS emits Rust source, which rustc compiles; the claim ignores the other tiers"),
    ("zero interpreter overhead", "the interpreter is one of five tiers"),
    ("zero runtime overhead", "no suite measures the claim"),
    ("tutor prompts", "there is no such feature"),
    ("capability-based", "access control is ## Policy predicates and Check that guards"),
    ("largo verify", "it verified nothing in the released 0.10.1"),
    ("Z3", "Z3 verification is off the page until a fixed release ships"),
    ("static verification", "Z3 verification is off the page until a fixed release ships"),
    ("no coding", "a person still reads the program; that is the point of the page"),
    ("no code", "a person still reads the program; that is the point of the page"),
    ("no symbols required", "the logic it prints is symbols"),
    ("bug-free", "no test proves a program free of bugs"),
    ("agent pack", "the agent pack is on main only, not in the installable release"),
    ("AGENTS.md", "largo new writes it on main only, not in the installable release"),
    ("MCP", "the MCP server is on main only, not in the installable release"),
    ("Futamura", "the hardware synthesis is fol_to_sva; the Futamura piece is a cached generator"),
    ("marketplace", "the VS Code extension ships as a VSIX; the marketplace listing is disabled"),
    ("2.6×", "the 2.62× headline counts five algorithm collapses"),
    ("maximum performance", "no suite measures the claim"),
    ("Markdown (.md)", "LOGOS sources are .lg files"),
];

#[test]
fn no_unbacked_claim_reaches_the_front_page() {
    let text = published_text();
    let found: Vec<String> = BANNED_CLAIMS
        .iter()
        .filter(|(claim, _)| text.contains(&claim.to_lowercase()))
        .map(|(claim, why)| format!("{claim:?}: {why}"))
        .collect();
    assert!(found.is_empty(), "the front page makes claims the product does not back:\n{}", found.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// 6. The page's parts
// ---------------------------------------------------------------------------------------------

/// The sections that are the page's own parts, by id: those inside no other section (the Studio's
/// panels are sections too, inside the hero).
fn part_ids() -> Vec<String> {
    let main = main_content();
    let sections = elements(main, "section");
    let span = |section: &str| {
        let start = section.as_ptr() as usize - main.as_ptr() as usize;
        start..start + section.len()
    };
    sections
        .iter()
        .filter(|section| {
            let own = span(section);
            !sections.iter().any(|other| {
                let outer = span(other);
                outer != own && outer.start <= own.start && own.end <= outer.end
            })
        })
        .map(|section| own_attributes(section).get("id").cloned().unwrap_or_default())
        .collect()
}

/// The page leads with the heading, the line on how, the install box, the copy buttons and the
/// hidden prompt, then the Studio frame (the user, 2026-11-01: the heading above the Studio again),
/// then how LOGOS compares, what comes with it and the FAQ, and nothing else.
#[test]
fn the_page_is_the_words_then_the_studio_then_the_comparison_the_library_and_the_faq() {
    assert_eq!(part_ids(), ["top", "compare", "library", "faq"], "the page's parts, in order");
    let top = element_by_id(landing(), "top");
    let at = |needle: &str| top.find(needle).unwrap_or_else(|| panic!("the hero holds no {needle}"));
    let order = [at("<h1"), at("class=\"landing-how\""), at("class=\"landing-install\""), at("landing-copy\""), at(&format!("id=\"{INSTRUCTIONS_ID}\"")), at("id=\"studio-frame\"")];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "the hero is not heading, the line on how, install box, copy buttons, prompt, Studio frame");
    for gone in ["mode-stories", "security-demo", "hello-world", "tech-stack", "bg-orb", "mini-studio", "grid3", "What you get", "id=\"next\"", "landing-switch", "data-os"] {
        assert!(!landing().contains(gone), "the page still renders {gone:?}, which the cleanup removed");
    }
}

/// Every link the front page makes into the Studio opens a file the Studio ships. The old page
/// linked `examples/code/fibonacci.logos` long after that example had moved, and the Studio opened
/// something else.
#[test]
fn every_studio_link_on_the_front_page_opens_a_shipped_example() {
    let studio = Route::Studio { file: None }.to_string();
    let mut files = Vec::new();
    for (attrs, text) in links(landing()) {
        let Some(href) = attrs.get("href") else { continue };
        // A bare `/studio?` (the nav's link) opens the Studio on its own first file.
        let Some(file) = href.strip_prefix("/studio?").and_then(|query| query.strip_prefix("file=")) else { continue };
        let spec = seeded_examples().find(|spec| spec.vfs_path.trim_start_matches('/') == file);
        assert!(spec.is_some(), "{text:?} opens {file} in the Studio, which the Studio does not ship");
        assert_eq!(href, &studio_file_url(spec.unwrap().vfs_path), "{text:?} is not how the site links {file}");
        files.push(file.to_string());
    }
    assert!(!files.is_empty(), "the front page links no Studio file (bare links go to {studio})");
}

#[test]
fn the_hero_keeps_the_way_to_the_studio_to_learn_and_to_contact() {
    let top = element_by_id(landing(), "top");
    let hero_links = links(top);
    let studio = Route::Studio { file: None }.to_string();
    let learn = Route::Learn { module: String::new() }.to_string();
    assert!(hero_links.iter().any(|(a, t)| t == "Open Studio" && a.get("href") == Some(&studio)), "the hero does not open the Studio");
    assert!(hero_links.iter().any(|(a, t)| t == "Learn" && a.get("href") == Some(&learn)), "the hero does not start Learn");
    assert!(hero_links.iter().any(|(a, t)| t == "Contact" && a.get("href").is_some_and(|h| h == "/contact")), "the hero does not lead to Contact");
}

/// The visible FAQ, as (question, answer) pairs in page order.
fn visible_faq() -> Vec<(String, String)> {
    let faq = element_by_id(landing(), "faq");
    let questions = element_texts(faq, "dt");
    let answers = element_texts(faq, "dd");
    assert_eq!(questions.len(), answers.len(), "the FAQ has {} questions and {} answers", questions.len(), answers.len());
    questions.into_iter().zip(answers).collect()
}

#[test]
fn the_faq_structured_data_is_the_visible_faq() {
    let faq = json_ld(landing()).into_iter().find(|v| v["@type"] == "FAQPage").expect("the front page carries an FAQPage");
    let data: Vec<(String, String)> = faq["mainEntity"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| (q["name"].as_str().unwrap().to_string(), q["acceptedAnswer"]["text"].as_str().unwrap().to_string()))
        .collect();
    let shown = visible_faq();
    assert_eq!(data, shown, "the FAQPage structured data and the visible FAQ disagree");
    let source: Vec<(String, String)> = content::faq().iter().map(|f| (f.question.to_string(), f.answer.text.clone())).collect();
    assert_eq!(source, shown, "the visible FAQ is not the FAQ data");
    assert!((3..=5).contains(&shown.len()), "the FAQ asks {} questions", shown.len());
    assert_eq!(shown[0], ("Is it really free?".to_string(), free_answer()), "the FAQ's first answer is not the pricing page's");
}

#[test]
fn the_title_and_description_follow_the_heading() {
    assert_eq!(seo_pages::LANDING.title, "LOGICAFFEINE | Code we can all read", "the front page's title");
    assert_eq!(seo_pages::LANDING.canonical_path, "/");
    let description = seo_pages::LANDING.description;
    for word in ["English", "logic", "math", "agent"] {
        assert!(description.contains(word), "the description does not mention {word}: {description:?}");
    }
    assert!((80..=170).contains(&description.chars().count()), "the description is {} characters", description.chars().count());
}

#[test]
fn the_page_carries_no_theme_attribute() {
    assert!(!landing().contains("data-theme"), "the front page carries a data-theme attribute");
}

// ---------------------------------------------------------------------------------------------
// The five-line prompt, for the visitor's system
// ---------------------------------------------------------------------------------------------

/// The prompt for `install`, a line a row.
fn prompt_lines(install: Install) -> Vec<String> {
    content::prompt(install).iter().map(Line::text).collect()
}

/// The box shows five lines a reader takes in at once, none wider than the PowerShell install with its
/// indent.
#[test]
fn the_prompt_is_five_short_lines() {
    for install in Install::ALL {
        let lines = prompt_lines(*install);
        assert_eq!(lines.len(), 5, "the {} prompt is {} lines", install.name(), lines.len());
        for line in &lines {
            assert!(!line.trim().is_empty(), "the {} prompt has a blank line", install.name());
            assert!(line.chars().count() <= 90, "a line of the {} prompt is {} characters: {line:?}", install.name(), line.chars().count());
        }
        assert_eq!(content::prompt_text(*install), lines.join("\n"), "the {} prompt's text is not its lines", install.name());
    }
}

/// Each prompt installs largo the way its system does, and only that way.
#[test]
fn the_prompt_installs_on_its_system() {
    for (install, own, other) in [(Install::Unix, INSTALL_UNIX, INSTALL_WINDOWS), (Install::Windows, INSTALL_WINDOWS, INSTALL_UNIX)] {
        let runs: Vec<&str> = content::prompt(install).iter().filter_map(|line| if let Line::Run(command) = line { Some(*command) } else { None }).collect();
        assert_eq!(runs, [own], "the {} prompt runs {runs:?}", install.name());
        assert!(!content::prompt_text(install).contains(other), "the {} prompt carries the other install", install.name());
    }
}

/// The prompt sends the agent to the served guide before it writes a line of LOGOS, and the guide is
/// on the site.
#[test]
fn the_prompt_sends_the_agent_to_the_guide_before_it_writes() {
    assert_eq!(AGENT_GUIDE_URL, "https://logicaffeine.com/agent.txt");
    assert!(public_dir().join("agent.txt").is_file(), "the site does not serve agent.txt");
    for install in Install::ALL {
        let lines = prompt_lines(*install);
        assert!(lines[1].contains(SOURCE_URL), "the {} prompt's second line is not the source: {:?}", install.name(), lines[1]);
        let guide = lines.iter().position(|line| line.contains(AGENT_GUIDE_URL)).unwrap_or_else(|| panic!("the {} prompt does not name the guide", install.name()));
        assert_eq!(guide, lines.len() - 1, "the {} prompt does not end by sending the agent through the guide", install.name());
        assert!(lines[guide].starts_with("Then follow "), "the {} prompt does not send the agent through the guide: {:?}", install.name(), lines[guide]);
        assert!(lines[guide].contains("site running"), "the {} prompt does not say where the guide ends: {:?}", install.name(), lines[guide]);
        let install_line = lines.iter().position(|line| line.starts_with("Install largo")).expect("the prompt installs largo");
        assert!(install_line < guide, "the {} prompt reads the guide before installing", install.name());
    }
}

#[test]
fn every_address_in_the_prompt_is_one_the_site_serves() {
    let routes = logicaffeine_web::sitemap::prerender_routes();
    let mut seen = 0;
    for install in Install::ALL {
        for url in urls(&content::prompt_text(*install)) {
            seen += 1;
            if url == SOURCE_URL {
                continue;
            }
            let rest = url.strip_prefix("https://logicaffeine.com").unwrap_or_else(|| panic!("{url} is not on logicaffeine.com"));
            let path = rest.split('?').next().unwrap_or(rest);
            assert!(public_dir().join(path.trim_start_matches('/')).is_file() || routes.iter().any(|route| route == path), "{url} is neither a file in public/ nor a page the site prerenders");
        }
    }
    assert!(seen >= 4, "the prompts give only {seen} addresses");
}

/// The long guide the prompt sends the agent to is the instructions, served byte for byte at
/// /agent.txt. `LOGOS_REGEN=1 cargo nextest run -p logicaffeine-tests --features web-tests --test
/// landing_page --run-ignored all -E 'test(regenerate_agent_txt)'` rewrites it.
#[test]
fn the_served_agent_guide_is_the_instructions() {
    let served = std::fs::read_to_string(public_dir().join("agent.txt")).unwrap_or_else(|e| panic!("read public/agent.txt ({e}); regenerate it"));
    assert!(served == format!("{}\n", content::instructions_text()), "public/agent.txt is not the instructions; regenerate it");
}

#[test]
#[ignore = "regenerates public/agent.txt; run on purpose with LOGOS_REGEN=1"]
fn regenerate_agent_txt() {
    if std::env::var_os("LOGOS_REGEN").is_some() {
        std::fs::write(public_dir().join("agent.txt"), format!("{}\n", content::instructions_text())).unwrap();
    }
}

// ---------------------------------------------------------------------------------------------
// 7. How LOGOS compares
// ---------------------------------------------------------------------------------------------

/// The kinds of work the comparison sets LOGOS beside, in order.
const KINDS: [&str; 8] = ["General purpose", "Proof assistants", "Verification", "Logic and controlled English", "Hardware", "GPU", "Web", "Concurrency and distributed"];

#[test]
fn the_comparison_is_one_card_per_kind() {
    let cards = content::compare();
    let kinds: Vec<&str> = cards.iter().map(|card| card.kind).collect();
    assert_eq!(kinds, KINDS, "the comparison's kinds, in order");
    for card in cards {
        assert!(!card.usually.is_empty(), "the {} card names no tool it is usually done with", card.kind);
        assert!(!card.logos.text.trim().is_empty(), "the {} card says nothing of LOGOS", card.kind);
        assert!(!card.tags.is_empty(), "the {} card carries no status", card.kind);
    }
    let compare = element_by_id(landing(), "compare");
    assert_eq!(element_texts(compare, "h2"), [content::COMPARE_HEADING], "the comparison's heading");
    assert_eq!(element_texts(compare, "h3"), KINDS, "the cards on the page");
}

#[test]
fn every_logos_line_in_the_comparison_is_proven() {
    let mut claims: Vec<&content::Claim> = content::compare().iter().map(|card| &card.logos).collect();
    claims.push(content::compare_lead());
    for claim in claims {
        assert!(matches!(&claim.evidence, Evidence::Suites(suites) if !suites.is_empty()), "{:?} names no suite that proves it", claim.text);
        assert!(content::every_claim().iter().any(|(text, _)| *text == claim.text), "{:?} is missing from every_claim, so no suite check reads it", claim.text);
    }
}

/// Each tool a card names is named as its own site names it: the source is on the tool's own domain.
#[test]
fn every_tool_the_comparison_names_has_its_own_source() {
    let homes: BTreeMap<&str, &[&str]> = BTreeMap::from([
        ("Python", &["docs.python.org"][..]),
        ("Rust", &["doc.rust-lang.org", "www.rust-lang.org"][..]),
        ("C", &["www.open-std.org"][..]),
        ("Lean 4", &["lean-lang.org"][..]),
        ("Rocq", &["rocq-prover.org"][..]),
        ("Dafny", &["dafny.org"][..]),
        ("TLA+", &["lamport.azurewebsites.net"][..]),
        ("Prolog", &["www.iso.org"][..]),
        ("Attempto Controlled English", &["attempto.ifi.uzh.ch"][..]),
        ("SystemVerilog", &["standards.ieee.org"][..]),
        ("Chisel", &["www.chisel-lang.org"][..]),
        ("CUDA", &["docs.nvidia.com"][..]),
        ("TypeScript", &["www.typescriptlang.org"][..]),
        ("JavaScript", &["tc39.es"][..]),
        ("Elixir", &["elixir-lang.org"][..]),
        ("Erlang", &["www.erlang.org"][..]),
        ("Automerge", &["automerge.org"][..]),
    ]);
    for card in content::compare() {
        for tool in card.usually {
            let host = tool.source.strip_prefix("https://").and_then(|rest| rest.split('/').next()).unwrap_or_else(|| panic!("{}'s source {:?} is not an https address", tool.name, tool.source));
            let own = homes.get(tool.name).unwrap_or_else(|| panic!("{} is named on the {} card with no known home", tool.name, card.kind));
            assert!(own.contains(&host), "{}'s source is on {host}, not its own site ({own:?})", tool.name);
        }
    }
}

/// The comparison names other tools and says what LOGOS does; it claims no win over them.
#[test]
fn the_comparison_claims_no_win() {
    let text = format!(" {} ", visible_text(element_by_id(landing(), "compare")).to_lowercase());
    for word in [" faster", " fastest", " better", " best ", " beats", " only ", " unlike ", "×", " times "] {
        assert!(!text.contains(word), "the comparison says {word:?}");
    }
}

/// The emit targets the released largo lists.
fn release_targets() -> Vec<String> {
    release().commands.get("emit").expect("the release has emit").lines().filter_map(|line| line.trim().strip_prefix("- ")).filter_map(|item| item.split(':').next()).map(str::to_string).collect()
}

/// Whether the released largo has what a card leans on.
fn in_release(uses: &Uses) -> bool {
    match uses {
        Uses::Command(command) => release().commands.contains_key(*command),
        Uses::Emit(target) => release_targets().iter().any(|t| t == target),
    }
}

#[test]
fn a_card_in_largo_names_only_what_the_release_has() {
    assert!(release_targets().contains(&"wasm".to_string()), "the release's emit targets read as {:?}", release_targets());
    for card in content::compare().iter().filter(|card| !card.tags.contains(&Status::NextRelease)) {
        for uses in card.uses {
            assert!(in_release(uses), "the {} card leans on {uses:?}, which the released largo lacks; tag it for the next release", card.kind);
        }
    }
}

/// A card is tagged for the next release exactly when it leans on something the released largo
/// lacks. When a release catches up, this goes red and the tag comes off; when the tree moves
/// ahead, it goes red and the tag goes on.
#[test]
fn a_card_waits_for_the_next_release_exactly_when_the_release_lacks_what_it_uses() {
    for card in content::compare() {
        let waits: Vec<&Uses> = card.uses.iter().filter(|uses| !in_release(uses)).collect();
        assert_eq!(
            card.tags.contains(&Status::NextRelease),
            !waits.is_empty(),
            "the {} card is tagged {:?} and the released largo lacks {waits:?}",
            card.kind,
            card.tags
        );
    }
}

/// Each card's Try it opens a file the frame above shelves, and the page says which.
#[test]
fn every_try_it_opens_a_shelf_file() {
    let compare = element_by_id(landing(), "compare");
    let buttons: Vec<BTreeMap<String, String>> = tags(compare, "button").into_iter().filter(|b| b.contains_key("data-file")).collect();
    let tried: Vec<&str> = content::compare().iter().filter_map(|card| card.try_it).collect();
    assert!(tried.len() >= 6, "only {} cards offer Try it", tried.len());
    for file in &tried {
        assert!(content::shelf().files.contains(file), "a card's Try it opens {file}, which the frame does not shelve");
        assert!(buttons.iter().any(|b| b.get("data-file").map(String::as_str) == Some(*file) && b.get("aria-controls").map(String::as_str) == Some("studio-frame")), "no Try it button on the page opens {file} in the frame");
    }
    assert_eq!(buttons.len(), tried.len(), "the page shows {} Try it buttons for {} cards that offer one", buttons.len(), tried.len());
}

// ---------------------------------------------------------------------------------------------
// 8. What comes with LOGOS
// ---------------------------------------------------------------------------------------------

/// The standard library's figures as the ledger states them, read here by a second, plain reader.
struct Ledger {
    green: usize,
    in_scope: usize,
    languages: Vec<String>,
}

fn ledger() -> Ledger {
    let text = std::fs::read_to_string(repo_root().join("benchmarks/parity/PARITY_LEDGER.md")).expect("the parity ledger");
    let parity = text.lines().find(|line| line.contains("**Correctness parity:**")).expect("the ledger states its correctness parity");
    let inner = &parity[parity.find('(').unwrap() + 1..parity.find(')').unwrap()];
    let numbers: Vec<usize> = inner.split(|c: char| !c.is_ascii_digit()).filter(|w| !w.is_empty()).filter_map(|w| w.parse().ok()).collect();
    let provenance = text.lines().find(|line| line.starts_with("Pinned upstream sources")).expect("the ledger names its upstream sources");
    let listed = &provenance[provenance.find(':').unwrap() + 1..];
    let languages = listed.split(", ").filter_map(|part| part.split(" = ").next()).map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).collect();
    Ledger { green: numbers[0], in_scope: numbers[1], languages }
}

#[test]
fn the_library_figures_are_the_ledgers() {
    let ledger = ledger();
    assert!(ledger.green > 0 && ledger.in_scope >= ledger.green, "the ledger reads as {} of {}", ledger.green, ledger.in_scope);
    assert_eq!(ledger.languages, ["C", "Python", "Rust", "Go", "Java"], "the ledger's upstream languages");
    let library = visible_text(element_by_id(landing(), "library"));
    assert!(library.contains(&format!("{} of {} capabilities", ledger.green, ledger.in_scope)), "the library does not quote the ledger's {} of {}: {library}", ledger.green, ledger.in_scope);
    assert!(library.contains("the C, Python, Rust, Go and Java standard libraries"), "the library does not name the ledger's languages: {library}");
}

#[test]
fn the_module_count_is_the_preludes() {
    let modules = logicaffeine_compile::loader::prelude_module_sources().len();
    assert!(modules >= 10, "the prelude has only {modules} modules");
    let library = visible_text(element_by_id(landing(), "library"));
    assert!(library.contains(&format!("{modules} modules written in LOGOS")), "the library does not count the prelude's {modules} modules: {library}");
}

/// The library says from which release largo has it only while the release a reader installs is
/// older than this tree; the release fixture has caught up, so the tag is gone.
#[test]
fn the_library_says_when_largo_has_it() {
    let released = std::fs::read_to_string(release_dir().join("VERSION")).expect("the release fixture's version");
    let released = released.trim().trim_start_matches("largo ").to_string();
    let tree = env!("CARGO_PKG_VERSION");
    let library = visible_text(element_by_id(landing(), "library"));
    let parse = |v: &str| v.split('.').map(|n| n.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    if parse(&released) < parse(tree) {
        assert!(library.contains(&format!("In largo from {tree}.")), "the library does not say largo has it from {tree}: {library}");
    } else {
        assert!(!library.contains("In largo from"), "the release has caught up with {tree}; the library's tag can go");
    }
}

#[test]
fn every_feature_line_is_proven() {
    let lines: Vec<&content::Claim> = content::features().iter().chain(content::library().iter()).collect();
    assert!(lines.len() >= 8, "the library section holds only {} lines", lines.len());
    for claim in lines {
        assert!(matches!(&claim.evidence, Evidence::Suites(suites) if !suites.is_empty()), "{:?} names no suite that proves it", claim.text);
        assert!(content::every_claim().iter().any(|(text, _)| *text == claim.text), "{:?} is missing from every_claim", claim.text);
    }
}

// ---------------------------------------------------------------------------------------------
// 9. Words: each part's budget
// ---------------------------------------------------------------------------------------------

fn words(text: &str) -> usize {
    text.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

/// `html` without the elements `ids` name, and without the Windows prompt and the switch's label for
/// it, which a page that knows no system hides.
fn without(html: &str, ids: &[&str]) -> String {
    let mut out = html.to_string();
    for id in ids {
        if out.contains(&format!(" id=\"{id}\"")) {
            let element = element_by_id(&out, id).to_string();
            out = out.replacen(&element, " ", 1);
        }
    }
    for tag in ["span", "pre"] {
        for element in elements(&out.clone(), tag) {
            if own_attributes(element).get("data-install").map(String::as_str) == Some("windows") {
                out = out.replacen(element, " ", 1);
            }
        }
    }
    out
}

/// The page's words keep to their budget, counted as a reader sees them (the Studio and the hidden
/// prompt aside): the heading, the line on how, the install box, three copy buttons and three ways on
/// (the user, 2026-10-31: way less words).
#[test]
fn every_section_keeps_its_word_budget() {
    let top = without(element_by_id(landing(), "top"), &["studio-frame", INSTRUCTIONS_ID]);
    for (id, budget, text) in [
        ("top", 70, prose(&top)),
        ("compare", 200, prose(element_by_id(landing(), "compare"))),
        ("library", 100, prose(element_by_id(landing(), "library"))),
        ("faq", 150, prose(element_by_id(landing(), "faq"))),
    ] {
        assert!(words(&text) > 0, "#{id} says nothing");
        assert!(words(&text) <= budget, "#{id} says {} words, over its {budget}: {text}", words(&text));
    }
}

/// The same budgets, line by line, so a failure names the line that grew.
#[test]
fn every_line_keeps_its_word_budget() {
    let mut over = Vec::new();
    let mut check = |what: String, text: &str, budget: usize| {
        if words(text) > budget {
            over.push(format!("{what} is {} words, over {budget}: {text:?}", words(text)));
        }
    };
    check("the comparison's lead".to_string(), &content::compare_lead().text, 14);
    for card in content::compare() {
        let names = card.usually.iter().map(|tool| tool.name).collect::<Vec<_>>().join(", ");
        check(format!("the {} card's line", card.kind), &card.logos.text, 12);
        check(format!("the {} card's tools", card.kind), &names, 6);
        let tags = card.tags.iter().map(|tag| tag.label()).collect::<Vec<_>>().join(" ");
        let all = format!("{} Usually: {names} {} {tags} {}", card.kind, card.logos.text, if card.try_it.is_some() { "Try it" } else { "" });
        check(format!("the {} card", card.kind), &all, 24);
    }
    for line in content::features() {
        check("a feature line".to_string(), &line.text, 10);
    }
    for line in content::library() {
        check("a library line".to_string(), &line.text, 20);
    }
    for item in content::faq() {
        check(format!("the question {:?}", item.question), item.question, 8);
        check(format!("the answer to {:?}", item.question), &item.answer.text, 35);
    }
    check("the line on how".to_string(), &content::how().text, 24);
    assert!(over.is_empty(), "lines over their budget:\n{}", over.join("\n"));
}
