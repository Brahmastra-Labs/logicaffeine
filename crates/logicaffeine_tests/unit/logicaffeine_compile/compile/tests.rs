use super::*;

/// The staged runtime crates declare the workspace's own edition and minimum Rust, read from
/// the root manifest; cargo's view of this crate's inherited `rust-version` is the cross-check.
#[test]
fn a_staged_manifest_carries_the_workspace_edition_and_minimum_rust() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workspace =
        WorkspacePackage::read(&root).expect("the root manifest declares [workspace.package]");
    assert_eq!(
        workspace.rust_version,
        env!("CARGO_PKG_RUST_VERSION"),
        "read a different rust-version than cargo inherited"
    );

    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("Cargo.toml");
    fs::write(
        &manifest,
        "[package]\nname = \"staged\"\nedition.workspace = true\nrust-version.workspace = true\n",
    )
    .unwrap();
    deworkspace_cargo_toml(&manifest, &workspace).unwrap();
    let staged = fs::read_to_string(&manifest).unwrap();
    assert!(
        staged.contains(&format!("edition = \"{}\"", workspace.edition)),
        "{staged}"
    );
    assert!(
        staged.contains(&format!("rust-version = \"{}\"", workspace.rust_version)),
        "{staged}"
    );
    assert!(
        !staged.contains(".workspace = true"),
        "an inherited field survived staging: {staged}"
    );
}

#[test]
fn the_workspace_package_table_is_read_by_its_own_header_only() {
    let text = "[package]\nedition = \"2015\"\n\n[workspace.package]\n# comment\nedition = \"2021\"\nrust-version = \"1.88\" # measured\n\n[workspace.lints.rust]\nrust-version = \"9.99\"\n";
    let ws = WorkspacePackage::parse(text).expect("parses");
    assert_eq!(
        (ws.edition.as_str(), ws.rust_version.as_str()),
        ("2021", "1.88")
    );
    assert!(
        WorkspacePackage::parse("[package]\nedition = \"2021\"\n").is_none(),
        "no [workspace.package] must be an error, not a default"
    );
}

fn parse_error_of(kind: crate::error::ParseErrorKind) -> ParseError {
    ParseError {
        kind,
        span: crate::token::Span::default(),
    }
}

#[test]
fn describe_parse_error_never_prints_a_bare_kind_name() {
    let text = describe_parse_error(&parse_error_of(
        crate::error::ParseErrorKind::ExpectedStatement,
    ));
    assert!(
        !text.contains("ExpectedStatement"),
        "an imported module's mistake must read as a sentence, not a Rust enum variant: {text:?}"
    );
    assert!(
        text.contains("statement"),
        "the sentence names what was expected: {text:?}"
    );
}

#[test]
fn describe_parse_error_is_the_entry_files_own_explanation() {
    for kind in [
        crate::error::ParseErrorKind::ExpectedStatement,
        crate::error::ParseErrorKind::ExpectedKeyword {
            keyword: "be".to_string(),
        },
        crate::error::ParseErrorKind::Custom("a message the parser wrote".to_string()),
    ] {
        let pe = parse_error_of(kind);
        assert_eq!(
            describe_parse_error(&pe),
            crate::error::socratic_explanation(&pe, &Interner::new()),
            "a mistake reads the same whether it sits in the entry file or in an imported module"
        );
    }
}

#[test]
fn test_compile_let_statement() {
    let source = "## Main\nLet x be 5.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should compile: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("fn main()"));
    assert!(rust.contains("let x: i64 = 5;"));
}

#[test]
fn test_compile_return_statement() {
    let source = "## Main\nReturn 42.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should compile: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("return 42;"));
}

// --- standalone-install AOT: crates.io runtime deps -----------------------
// An installed `largo` has no monorepo checkout to stage runtime sources
// from, so the generated project must version-depend on the published
// crates.io releases at the compiler's OWN version. These lock that in.

#[test]
fn standalone_manifest_depends_on_crates_io_not_paths() {
    let source = RuntimeCrateSource::Registry {
        version: "0.10.1".to_string(),
    };
    let manifest = render_cargo_manifest("", &[], &source);
    assert!(
        manifest.contains("logicaffeine-data = \"=0.10.1\""),
        "data must be an EXACT crates.io version dep:\n{manifest}"
    );
    assert!(
        manifest.contains("logicaffeine-system = { version = \"=0.10.1\", features = [\"full\"] }"),
        "system must be a crates.io version dep carrying the full feature:\n{manifest}"
    );
    assert!(
        !manifest.contains("path = \"./crates"),
        "a standalone manifest must NOT reference staged ./crates path deps:\n{manifest}"
    );
    assert!(
        manifest.contains("[target.'cfg(target_os = \"linux\")'.dependencies]")
            && manifest.contains("features = [\"full\", \"io-uring\"]"),
        "the linux override must keep io-uring as a version dep:\n{manifest}"
    );
}

#[test]
fn workspace_manifest_keeps_path_deps() {
    let source = RuntimeCrateSource::Workspace;
    let manifest = render_cargo_manifest("", &[], &source);
    assert!(
        manifest.contains("logicaffeine-data = { path = \"./crates/logicaffeine_data\" }"),
        "in-workspace data stays a path dep:\n{manifest}"
    );
    assert!(
        manifest.contains(
            "logicaffeine-system = { path = \"./crates/logicaffeine_system\", features = [\"full\"] }"
        ),
        "in-workspace system stays a path dep:\n{manifest}"
    );
    assert!(
        !manifest.contains("logicaffeine-data = \"="),
        "in-workspace mode must NOT emit crates.io version deps:\n{manifest}"
    );
}

#[test]
fn standalone_manifest_preserves_user_requires_deps_in_order() {
    let source = RuntimeCrateSource::Registry {
        version: "0.10.1".to_string(),
    };
    let deps = vec![CrateDependency {
        name: "rand".to_string(),
        version: "0.8".to_string(),
        features: vec![],
    }];
    let manifest = render_cargo_manifest("", &deps, &source);
    let dep_pos = manifest
        .find("rand = \"0.8\"")
        .expect("user `## Requires` dep must appear in the manifest");
    let target_pos = manifest
        .find("[target.")
        .expect("manifest must have the linux [target] override");
    assert!(
        dep_pos < target_pos,
        "user deps must stay inside [dependencies], before the [target] header:\n{manifest}"
    );
}

#[test]
fn registry_manifest_pins_the_compilers_own_version() {
    // The whole safety argument: the runtime crates the generated Rust links
    // against are pinned to the exact version that produced that Rust.
    let version = env!("CARGO_PKG_VERSION");
    let source = RuntimeCrateSource::Registry {
        version: version.to_string(),
    };
    let manifest = render_cargo_manifest("", &[], &source);
    assert!(
        manifest.contains(&format!("logicaffeine-data = \"={version}\"")),
        "runtime deps must pin the compiler's own version {version}:\n{manifest}"
    );
}

#[test]
fn manifest_declares_conditional_deps_the_generated_code_names() {
    // Deep check stages its own cargo project; the manifest must carry the
    // same on-demand deps the CLI build manifest injects, or a stdlib
    // splice (`indexOf`/`countSub` → memchr) drowns a clean program in
    // phantom findings.
    let source = RuntimeCrateSource::Workspace;
    let manifest = render_cargo_manifest("memchr::memmem::find_iter(a, b)", &[], &source);
    let dep_pos = manifest
        .find("memchr = \"2\"")
        .expect("memchr-using code must declare the memchr dep");
    assert!(
        dep_pos < manifest.find("[target.").unwrap(),
        "conditional deps stay inside [dependencies]:\n{manifest}"
    );

    let manifest = render_cargo_manifest("rayon::prelude::*;", &[], &source);
    assert!(manifest.contains("rayon = \"1\""), "{manifest}");

    // A `## Requires` for the same crate wins — no duplicate key.
    let deps = vec![CrateDependency {
        name: "memchr".to_string(),
        version: "2.7".to_string(),
        features: vec![],
    }];
    let manifest = render_cargo_manifest("memchr::memchr(b, h)", &deps, &source);
    assert_eq!(manifest.matches("memchr = ").count(), 1, "{manifest}");
}

/// `Simultaneously:` lowers to `rayon::join(…)` with no `use rayon::prelude::*;` anywhere, and a program
/// that names a crate its manifest does not declare does not build.
#[test]
fn a_parallel_block_declares_rayon() {
    let source = RuntimeCrateSource::Workspace;
    let manifest = render_cargo_manifest(
        "let (a, b) = rayon::join(|| first(), || second());",
        &[],
        &source,
    );
    assert!(manifest.contains("rayon = \"1\""), "{manifest}");
}

#[test]
fn manifest_carries_the_http_feature_for_serve_programs() {
    let source = RuntimeCrateSource::Workspace;
    let manifest = render_cargo_manifest(
        "logicaffeine_system::http::server::serve(addr, handler)",
        &[],
        &source,
    );
    assert!(
        manifest.contains("features = [\"full\", \"http\"]"),
        "a Serve program's check project needs the http runtime feature:\n{manifest}"
    );
    let manifest = render_cargo_manifest("", &[], &source);
    assert!(
        !manifest.contains("\"http\""),
        "no http reference → the feature stays off:\n{manifest}"
    );
}

#[test]
fn detect_uses_workspace_inside_the_monorepo() {
    // These tests run from within the checkout, so detection must choose
    // Workspace (staged path deps) and never accidentally reach crates.io.
    assert!(matches!(
        RuntimeCrateSource::detect(),
        RuntimeCrateSource::Workspace
    ));
}

#[test]
fn dep_line_methods_lock_both_call_sites() {
    // The CLI project build (apps/logicaffeine_cli project/build.rs) renders
    // its manifest from these methods directly, so their exact strings are a
    // shared contract — lock them independently of render_cargo_manifest.
    let ws = RuntimeCrateSource::Workspace;
    assert_eq!(
        ws.data_dep(),
        "logicaffeine-data = { path = \"./crates/logicaffeine_data\" }"
    );
    assert_eq!(
        ws.system_dep(&["full"]),
        "logicaffeine-system = { path = \"./crates/logicaffeine_system\", features = [\"full\"] }"
    );

    let reg = RuntimeCrateSource::Registry {
        version: "0.10.1".to_string(),
    };
    assert_eq!(reg.data_dep(), "logicaffeine-data = \"=0.10.1\"");
    assert_eq!(
        reg.system_dep(&["full"]),
        "logicaffeine-system = { version = \"=0.10.1\", features = [\"full\"] }"
    );
    assert!(reg
        .system_dep(&["full", "io-uring"])
        .contains("features = [\"full\", \"io-uring\"]"));
}

#[test]
fn both_manifests_end_with_the_one_linux_engine_section() {
    // `largo build` (apps/logicaffeine_cli project/build.rs) appends this section after its own
    // dependencies; `served_address_lock` proves the binary it builds serves on io_uring.
    let reg = RuntimeCrateSource::Registry {
        version: "0.10.1".to_string(),
    };
    assert_eq!(
        reg.linux_engine_section(),
        "[target.'cfg(target_os = \"linux\")'.dependencies]\n\
         logicaffeine-system = { version = \"=0.10.1\", features = [\"full\", \"io-uring\"] }\n"
    );
    let ws = RuntimeCrateSource::Workspace;
    assert_eq!(
        ws.linux_engine_section(),
        "[target.'cfg(target_os = \"linux\")'.dependencies]\n\
         logicaffeine-system = { path = \"./crates/logicaffeine_system\", features = [\"full\", \"io-uring\"] }\n"
    );
    for source in [reg, ws] {
        let manifest = render_cargo_manifest("", &[], &source);
        assert!(
            manifest.contains(&source.linux_engine_section()),
            "the single-file manifest carries the shared section:\n{manifest}"
        );
    }
}

/// Run `src` through the RUN-PATH optimizer (`optimize_for_run`, which
/// includes the indexed-load LICM) and then the pure-bytecode VM (no native
/// tier → no forge dependency), capturing the outcome.
fn optimized_vm_outcome_no_tier(src: &str, argv: &[String]) -> (String, Option<String>) {
    crate::ui_bridge::with_optimized_program(src, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => crate::vm::run_to_outcome_with_args(
            stmts,
            interner,
            Some(types),
            Some(&policies),
            argv,
            None,
        ),
        Err(advice) => (String::new(), Some(advice)),
    })
}

fn norm_lines(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Task C SOUNDNESS, in-process: an inner loop reading a loop-invariant
/// `item i of arr` (i = OUTER counter, arr never mutated in the loop) mixed
/// with a variant `item j of arr` must produce the EXACT outcome the
/// tree-walker produces once the invariant read is hoisted. The guarded
/// hoist preserves WHEN the read runs, so the optimized VM matches the
/// oracle bit-for-bit on a real, in-bounds nbody-shaped loop.
#[test]
fn licm_indexed_load_nbody_shape_matches_oracle_inprocess() {
    let src = "## Main\n\
                   Let arr be [10, 20, 30, 40, 50].\n\
                   Let mutable acc be 0.\n\
                   Let mutable i be 1.\n\
                   While i is at most 5:\n\
                   \x20   Let mutable j be 1.\n\
                   \x20   While j is at most 5:\n\
                   \x20       Let d be item i of arr - item j of arr.\n\
                   \x20       Set acc to acc + d * d.\n\
                   \x20       Set j to j + 1.\n\
                   \x20   Set i to i + 1.\n\
                   Show acc.\n";
    let argv: [String; 0] = [];
    let tw = tw_outcome_with_args(src, &argv);
    let (out, err) = optimized_vm_outcome_no_tier(src, &argv);
    assert_eq!(
        (norm_lines(&tw.output), &tw.error),
        ("10000".to_string(), &None),
        "oracle: sum of (a_i - a_j)^2"
    );
    assert_eq!(
        (norm_lines(&out), &err),
        (norm_lines(&tw.output), &tw.error),
        "optimized VM (LICM) must match the tree-walker on the nbody-shaped loop"
    );
}

/// SOUNDNESS GATE (forge-free): the OPTIMIZED bytecode VM — exercising the
/// constant-pool sharing (Task B) and the indexed-load LICM (Task C) — must
/// produce BIT-IDENTICAL output to the raw tree-walker on the whole
/// benchmark corpus. This mirrors the `vm_opt_differential` integration
/// test but runs without a native tier, so it stays inside this crate.
#[test]
fn corpus_optimized_vm_matches_treewalker_no_tier() {
    const CORPUS: &[(&str, &str)] = &[
        ("ackermann", "3"),
        ("array_fill", "2000"),
        ("array_reverse", "2000"),
        ("binary_trees", "6"),
        ("bubble_sort", "60"),
        ("coins", "500"),
        ("collatz", "300"),
        ("collect", "300"),
        ("counting_sort", "2000"),
        ("fannkuch", "5"),
        ("fib", "12"),
        ("fib_iterative", "500"),
        ("gcd", "60"),
        ("graph_bfs", "200"),
        ("heap_sort", "300"),
        ("histogram", "2000"),
        ("knapsack", "30"),
        ("loop_sum", "2000"),
        ("mandelbrot", "20"),
        ("matrix_mult", "8"),
        ("mergesort", "300"),
        ("nbody", "100"),
        ("nqueens", "5"),
        ("pi_leibniz", "2000"),
        ("prefix_sum", "2000"),
        ("primes", "500"),
        ("quicksort", "300"),
        ("sieve", "2000"),
        ("spectral_norm", "20"),
        ("string_search", "1200"),
        ("strings", "200"),
        ("two_sum", "300"),
    ];
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            for &(name, size) in CORPUS {
                let path = format!(
                    "{}/../../benchmarks/programs/{}/main.lg",
                    env!("CARGO_MANIFEST_DIR"),
                    name
                );
                let src = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
                let argv = vec!["bench".to_string(), size.to_string()];
                let (out, err) = optimized_vm_outcome_no_tier(&src, &argv);
                let tw = tw_outcome_with_args(&src, &argv);
                assert_eq!(
                    (norm_lines(&out), &err),
                    (norm_lines(&tw.output), &tw.error),
                    "OPTIMIZED VM (no tier) diverged from the tree-walker on '{name}' at {size}"
                );
            }
        })
        .expect("spawn")
        .join()
        .expect("corpus thread panicked");
}

/// Parses `source` and decompiles its first `## Component`/`## Page` back to LGX text —
/// `decompile_component`'s own entry point, exercised directly rather than through
/// `projection1_source`'s Futamura path (which runs the projection-safe optimizer over the
/// program first and is not guaranteed to leave a `ComponentDef` statement in the residual
/// untouched).
fn decompiled_component(source: &str) -> String {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    let (type_registry, _policies) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };
    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();
    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program().expect("source must parse");
    let def = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::ComponentDef(def) => Some(def),
            _ => None,
        })
        .expect("source must declare a component");
    let mut out = String::new();
    decompile_component(def, &interner, &mut out);
    out
}

/// G25: `When a file is dropped on … (file: File):` decompiles naming `File` — never a hardcoded
/// literal blind to what the author actually declared. Closes the class the decompiler's own
/// FileDropped arm regressed into before it read `param`'s DECLARED type via `decompile_type`
/// instead of a fixed `": File"` suffix.
#[test]
fn a_file_typed_drop_handler_decompiles_naming_file() {
    let src = "## Component App\nState:\n    Let got be \"-\".\nActions:\n    When a file is dropped on \"#zone\" (file: File):\n        Set got to file's name.\nView:\n    <div id=\"zone\">drop here</div><p id=\"g\">{got}</p>\n\n## Main\nMount App.\n";
    let out = decompiled_component(src);
    assert!(
        out.contains("When a file is dropped on \"#zone\" (file: File):"),
        "expected the declared `File` type in the decompiled trigger, got:\n{out}"
    );
}

/// The sibling case: `(name: Text)` decompiles naming `Text`, not `File` — the same call,
/// `decompile_type(param.1, interner)`, answering truthfully for the OTHER declared type instead
/// of a hardcoded guess.
#[test]
fn a_text_typed_drop_handler_decompiles_naming_text() {
    let src = "## Component App\nState:\n    Let got be \"-\".\nActions:\n    When a file is dropped on \"#zone\" (name: Text):\n        Set got to name.\nView:\n    <div id=\"zone\">drop here</div><p id=\"g\">{got}</p>\n\n## Main\nMount App.\n";
    let out = decompiled_component(src);
    assert!(
        out.contains("When a file is dropped on \"#zone\" (name: Text):"),
        "expected the declared `Text` type in the decompiled trigger, got:\n{out}"
    );
}

/// An operand of a binary operator that is a constructor ending in its type prints in parentheses: bare, the type takes
/// the operator that follows (`a new Seq of Int and true` reads as a list).
#[test]
fn a_typed_constructor_operand_prints_in_parentheses() {
    crate::ui_bridge::with_parsed_program(
        "## Main\nShow (a new Seq of Int) and true.\nShow (a new Set of Int) or false.\n",
        |parsed, interner| {
            let (stmts, _, _) = parsed.expect("the program parses");
            let printed: Vec<String> = stmts
                .iter()
                .filter_map(|s| match s {
                    Stmt::Show { object, .. } => Some(decompile_expr(object, interner)),
                    _ => None,
                })
                .collect();
            assert_eq!(printed, ["(a new Seq of Int) and true", "(a new Set of Int) or false"]);
        },
    );
}

/// The `Show` operands of `program`, each as the Rust decompiler prints it.
fn printed_operands(program: &str) -> Vec<String> {
    crate::ui_bridge::with_parsed_program(program, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::Show { object, .. } => Some(decompile_expr(object, interner)),
                _ => None,
            })
            .collect()
    })
}

#[test]
fn the_rust_decompiler_spells_each_literal_as_the_lexer_reads_it() {
    let printed = printed_operands(
        "## Main\nShow `a`.\nShow `\\n`.\nShow 2026-05-20.\nShow 2026-05-20 at 9:30am.\nShow 1 year and 3 days.\nShow 4pm.\nShow 500ms.\n",
    );
    assert_eq!(
        printed,
        ["`a`", "`\\n`", "2026-05-20", "2026-05-20 at 9:30am", "12 months and 3 days", "4:00pm", "500000000ns"]
    );
}

#[test]
fn a_literal_spelled_in_words_prints_as_an_operand_in_parentheses() {
    let printed = printed_operands(
        "## Main\nShow 2026-05-20 at 9:30am is equal to 2026-05-20 at 9:30am.\nShow 1 year and 3 days is equal to 15 months.\n",
    );
    assert!(printed[0].starts_with("(2026-05-20 at 9:30am) ") && printed[0].ends_with(" (2026-05-20 at 9:30am)"), "{printed:?}");
    assert!(printed[1].starts_with("(12 months and 3 days) ") && printed[1].ends_with(" (15 months)"), "{printed:?}");
    let reprinted = printed_operands(&format!("## Main\nShow {}.\nShow {}.\n", printed[0], printed[1]));
    assert_eq!(reprinted, printed, "what the decompiler prints reads back as the same expressions");
}

#[test]
fn a_construction_with_fields_prints_as_an_operand_in_parentheses() {
    let program = "## A Point has:\n    an x: Int.\n    a label: Text.\n\n## Main\nShow (a new Point with x 1 and label \"a\") equals (a new Point with x 1 and label \"a\").\n";
    let printed = printed_operands(program);
    assert!(
        printed[0].starts_with("(a new Point with ") && printed[0].contains(") equals (a new Point with "),
        "a construction an operator follows would take the operator into its last field: {printed:?}"
    );
    let reprinted = printed_operands(&format!("## A Point has:\n    an x: Int.\n    a label: Text.\n\n## Main\nShow {}.\n", printed[0]));
    assert_eq!(reprinted, printed, "what the decompiler prints reads back as the same expression");
}

/// Every encoder's text for `program`: the source encoder's and the compact one's.
fn encodings(program: &str) -> [String; 2] {
    [
        encode_program_source(program).expect("the program encodes"),
        encode_program_source_compact(program).expect("the program encodes compactly"),
    ]
}

#[test]
fn a_temporal_or_character_literal_is_held_as_the_languages_own_value() {
    for (source, held) in [
        ("`a`", "a new CChar with value (`a`)"),
        ("2026-05-20", "a new CDate with value (2026-05-20)"),
        ("2026-05-20 at 9:30am", "a new CMoment with value (2026-05-20 at 9:30am)"),
        ("1 year and 3 days", "a new CSpan with value (12 months and 3 days)"),
        ("4pm", "a new CTime with value (4:00pm)"),
        ("90 seconds", "a new CDuration with value (90000000000ns)"),
    ] {
        let program = format!("## Main\nLet v be {source}.\nShow v.\n");
        for encoded in encodings(&program) {
            assert!(encoded.contains(held), "`{source}` is held as `{held}`\n{encoded}");
            assert!(!encoded.contains("CSpelled"), "`{source}` is a value the self-interpreter carries\n{encoded}");
        }
    }
}

#[test]
fn an_expression_the_self_interpreter_carries_no_value_of_is_held_as_spelled() {
    let program = "## Main\nLet v be the title.\nShow v.\n";
    for encoded in encodings(program) {
        let held = format!("a new CSpelled with code {} and awaits {}", text_literal("the title"), text_literal(PAGE_AWAITS));
        assert!(encoded.contains(&held), "a question about the page is held as `{held}`\n{encoded}");
        assert!(!encoded.contains("unsupported"), "it is never held as the text `unsupported`\n{encoded}");
    }
}

#[test]
fn every_atom_spells_an_expression_that_evaluates_to_exactly_it() {
    let atoms = [
        CoreAtom::Duration(0),
        CoreAtom::Duration(1_500_000),
        CoreAtom::Duration(-7),
        CoreAtom::Duration(i64::MIN),
        CoreAtom::Duration(i64::MAX),
        CoreAtom::Date(0),
        CoreAtom::Date(20_000),
        CoreAtom::Date(-719_529),
        CoreAtom::Date(3_000_000),
        CoreAtom::Moment(0),
        CoreAtom::Moment(1_778_232_600_000_000_000),
        CoreAtom::Moment(1_778_232_615_500_000_001),
        CoreAtom::Moment(-1),
        CoreAtom::Moment(-9_000_000_000_000_000_001),
        CoreAtom::Moment(i64::MAX),
        CoreAtom::Span { months: 0, days: 0 },
        CoreAtom::Span { months: 14, days: 3 },
        CoreAtom::Span { months: 0, days: 40 },
        CoreAtom::Time(0),
        CoreAtom::Time(34_200_000_000_000),
        CoreAtom::Time(34_215_250_000_001),
        CoreAtom::Time(86_399_999_999_999),
        CoreAtom::Char('a'),
        CoreAtom::Char('\n'),
        CoreAtom::Char('\0'),
        CoreAtom::Char('`'),
        CoreAtom::Char('\\'),
        CoreAtom::Char('é'),
    ];
    for atom in atoms {
        let spelled = atom.expression();
        let bytes = crate::concurrency::marshal::encode_value_raw(&atom.runtime_value()).expect("an atom encodes");
        let exact = format!("[{}]", bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", "));
        let program = format!("## Main\nLet v be {spelled}.\nShow wireBytes(v).\n");
        let shown = crate::compile::interpret_program(&program).unwrap_or_else(|e| panic!("{atom:?} spelled `{spelled}`: {e:?}"));
        assert_eq!(shown.trim_end_matches('\n'), exact, "{atom:?} spelled `{spelled}` evaluates to another value");
    }
}

#[test]
fn a_construction_holds_its_left_out_slots_among_its_fields_by_name() {
    let program = "## An Inner has:\n    A v: Int.\n\n## A Record has:\n    A share: Real.\n    A label: Text.\n    An inner: Inner.\n    A count: Int.\n\n## Main\nLet r be a new Record with label \"x\".\nShow r's label.\n";
    for encoded in encodings(program) {
        let names: Vec<&str> = encoded
            .lines()
            .filter_map(|l| l.strip_prefix("Push \"").and_then(|rest| rest.split_once("\" to nvNames_")).map(|(n, _)| n))
            .collect();
        assert_eq!(names, ["count", "inner", "label", "share"], "the record's fields, by name\n{encoded}");
        for slot in ["Inner", "Real"] {
            assert!(
                encoded.contains(&format!("CDeclaredDefault with typeName \"{slot}\"")),
                "the `{slot}` slot no expression spells is held as its declared default\n{encoded}"
            );
        }
    }
}

#[test]
fn the_rust_decompiler_keeps_each_holes_format_spec() {
    let printed = printed_operands("## Main\nLet x be 3.14159.\nLet n be 7.\nShow \"{x:.2}|{n:>4}|{n:$}\".\nShow \"{n=}\".\nShow \"{x=:.3}\".\n");
    assert_eq!(printed, ["\"{x:.2}|{n:>4}|{n:$}\"", "\"{n=}\"", "\"{x=:.3}\""]);
}

#[test]
fn a_holes_format_spec_is_held_in_the_core_ir() {
    let program = "## Main\nLet x be 3.14159.\nLet n be 7.\nShow \"{x:.2} and {n:>4}\".\nShow \"{n=:$}\".\n";
    for encoded in encodings(program) {
        for spec in [".2", ">4", "$"] {
            assert!(
                encoded.contains("a new CFormattedPart with expr") && encoded.contains(&format!("and spec {} and form", text_literal(spec))),
                "the hole formatted by `{spec}` is held as a CFormattedPart\n{encoded}"
            );
        }
        assert!(
            encoded.contains("a new CLiteralPart with value \"n=\""),
            "a debug hole is labelled with the name the program spelled, as text\n{encoded}"
        );
    }
}

#[test]
fn a_check_is_held_with_its_policy_conditions() {
    let program = "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n    a verified: Bool.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User is trusted if the user's verified equals true.\nA User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet u be a new User with name \"A\" and role \"guest\" and verified true.\nLet d be a new Document with owner \"A\".\nCheck that u can edit d.\nCheck that u is trusted.\nShow \"ok\".\n";
    for encoded in encodings(program) {
        assert!(!encoded.contains("CCheck with predicate"), "a Check is never held as a predicate that always holds\n{encoded}");
        assert!(
            encoded.contains("a new CCheckPolicy with subject \"u\" and object \"d\" and capability true and rule \"Edit\""),
            "the capability check names its subject, object and action\n{encoded}"
        );
        assert!(encoded.contains("a new CPolicyCase with typeName \"User\""), "the check holds the User type's condition\n{encoded}");
        assert!(
            encoded.contains("a new PFieldEquals with field \"role\" and expected \"admin\" and literal true"),
            "the predicate the capability names is resolved to its own condition\n{encoded}"
        );
        assert!(
            encoded.contains("a new PFieldsEqual with subjectField \"name\" and objectField \"owner\""),
            "a subject field compared with an object field\n{encoded}"
        );
        assert!(
            encoded.contains("a new PFieldEquals with field \"verified\" and expected \"true\" and literal false and asInt 0 and isInt false and asBool true and isBool true"),
            "a policy value written as a word is read as the Bool it spells\n{encoded}"
        );
    }
}

#[test]
fn a_predicate_naming_itself_is_held_as_recursive() {
    let program = "## Definition\nA User has:\n    a role: Text.\n\n## Policy\nA User is odd if the user is odd.\n\n## Main\nLet u be a new User with role \"x\".\nCheck that u is odd.\n";
    for encoded in encodings(program) {
        assert!(
            encoded.contains("a new PRecursive with predicate \"odd\""),
            "a predicate met again in its own condition stops the resolution\n{encoded}"
        );
    }
}

/// A loop only a `Break` under a test of its input ends.
const COUNT_UP_TO: &str = "## To count (d: Int) -> Int:\n    Let mutable i be 0.\n    While true:\n        If i is greater than d:\n            Break.\n        Set i to i + 1.\n    Return i.\n\n## Main\nShow count(d).\n";

/// A loop only a `Break` in an arm of an `Inspect` on a value its input decides ends.
const STEP_UNTIL_STOP: &str = "## A Step is one of:\n    A Go.\n    A Stop.\n\n## To next (i: Int) and (d: Int) -> Step:\n    If i is greater than d:\n        Return a new Stop.\n    Return a new Go.\n\n## To run (d: Int) -> Int:\n    Let mutable i be 0.\n    While true:\n        Inspect next(i, d):\n            When Stop:\n                Break.\n            Otherwise:\n                Set i to i + 1.\n    Return i.\n\n## Main\nShow run(d).\n";

/// A loop its input bounds, which a `Break` under a test of its static argument also ends.
const SCAN_UNLESS_STOPPED: &str = "## To scan (d: Int) and (s: Int) -> Int:\n    Let mutable n be 0.\n    While n is less than d:\n        If s is greater than 0:\n            Break.\n        Set n to n + 1.\n    Return n.\n\n## Main\nShow scan(d, s).\n";

/// A loop over a static list, which a `Break` under a test of its own variable ends, inside a branch its input decides.
const STOP_AT_TWO_IF_ASKED: &str = "## To shown (d: Int) -> Int:\n    If d is greater than 0:\n        Repeat for x in [1, 2, 3]:\n            If x is 2:\n                Break.\n            Show x.\n    Return d.\n\n## Main\nShow shown(d).\n";

/// A function that reads the clock, called beside an input.
const STAMPED: &str = "## To stamp () -> Int:\n    Return now().\n\n## Main\nShow stamp() is greater than d.\n";

/// A function that reads the clock, called with a static argument beside an input.
const LATER_BY_FIVE: &str = "## To later (by: Int) -> Int:\n    Return now() + by.\n\n## Main\nShow later(5) is greater than d.\n";

/// `src` specialized by the offline specializer on the tree-walker, each of `statics` bound and each of `dynamic` left
/// to the run, as the program the residual is.
fn mix_with_inputs(src: &str, statics: &[(&'static str, i64)], dynamic: &[&str]) -> String {
    use crate::optimize::mix_bta::{annotate, StoreEntry};
    let tree = program_to_core_tree(src).expect("the program is in the native builder's coverage");
    let names: Vec<&str> = statics.iter().map(|(n, _)| *n).collect();
    let two_level = annotate(&tree, &mix_catalog(), &names, dynamic).expect("the program annotates");
    let store: MixStore = statics.iter().map(|(n, v)| (*n, StoreEntry::Value(CoreNode::Int(*v)))).collect();
    let residual = mix_apply(&two_level, &store, MixEngine::TreeWalker).expect("the specializer specializes the program");
    prepend_type_definitions(src, residual)
}

/// `program`'s run on the tree-walker with each of `inputs` bound ahead of its main.
fn run_with_inputs(program: &str, inputs: &[(&str, i64)]) -> String {
    let (functions, main) = program.split_once("## Main\n").expect("the program has a main");
    let bound: String = inputs.iter().map(|(n, v)| format!("Let {n} be {v}.\n")).collect();
    let run = format!("{functions}\n## Main\n{bound}{main}");
    interpret_bootstrap(&run).unwrap_or_else(|e| panic!("the run fails: {}\n{run}", describe_parse_error(&e)))
}

#[test]
fn a_residual_loop_breaks_where_the_program_breaks() {
    let cases: [(&str, &str, &[(&'static str, i64)]); 5] = [
        ("count", COUNT_UP_TO, &[]),
        ("step", STEP_UNTIL_STOP, &[]),
        ("scan, stopped", SCAN_UNLESS_STOPPED, &[("s", 1)]),
        ("scan, not stopped", SCAN_UNLESS_STOPPED, &[("s", 0)]),
        ("stop at two", STOP_AT_TWO_IF_ASKED, &[]),
    ];
    let mut differ = Vec::new();
    for (label, src, statics) in cases {
        let residual = mix_with_inputs(src, statics, &["d"]);
        for d in [-1, 0, 3, 7] {
            let specialized = run_with_inputs(&residual, &[("d", d)]);
            let mut inputs: Vec<(&str, i64)> = statics.to_vec();
            inputs.push(("d", d));
            let original = run_with_inputs(src, &inputs);
            if specialized != original {
                differ.push(format!("{label}, d = {d}: the residual printed {specialized:?}, the program {original:?}\n{residual}"));
            }
        }
    }
    assert!(differ.is_empty(), "{} runs differ:\n{}", differ.len(), differ.join("\n═══\n"));
}

#[test]
fn a_call_with_static_arguments_compiles_to_a_function_of_none() {
    let residual = mix_with_inputs(LATER_BY_FIVE, &[], &["d"]);
    assert!(residual.contains("Return (now() + 5)."), "the specialized body holds the static argument:\n{residual}");
    assert!(!residual.contains("later__0(5)"), "the static argument is never passed at run time:\n{residual}");
    for d in [0, i64::MAX] {
        assert_eq!(run_with_inputs(&residual, &[("d", d)]), run_with_inputs(LATER_BY_FIVE, &[("d", d)]), "d = {d}");
    }
}

#[test]
fn a_specialization_never_reads_the_clock() {
    let first = mix_with_inputs(STAMPED, &[], &["d"]);
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = mix_with_inputs(STAMPED, &[], &["d"]);
    assert_eq!(first, second, "two specializations of one program differ: the specializer read the clock");
    assert!(first.contains("now()"), "the residual reads the clock where the program does:\n{first}");
    for d in [0, i64::MAX] {
        assert_eq!(run_with_inputs(&first, &[("d", d)]), run_with_inputs(STAMPED, &[("d", d)]), "d = {d}");
    }
}

/// Whether `node` holds an `If` on the literal `true` anywhere inside it.
fn holds_an_always_taken_if(node: &CoreNode) -> bool {
    match node {
        CoreNode::Ind(name, fields) => {
            (name == "CIf"
                && matches!(fields.first(), Some(CoreNode::Ind(c, v)) if c == "CBool" && v.as_slice() == [CoreNode::Bool(true)]))
                || fields.iter().any(holds_an_always_taken_if)
        }
        CoreNode::List(items) => items.iter().any(holds_an_always_taken_if),
        _ => false,
    }
}

/// A desugaring's statements (a comprehension's, a destructuring `Let`'s, a tensor declaration's) and a `Reversibly:`
/// block run in the block that holds them, which an engine that scopes its blocks must not scope: no encoder writes
/// them as a block of their own, an `If` on the literal `true`, in `Main`, a function, a loop or a closure's block.
/// None of the programs writes `true` itself; the `If`s a comprehension tests its items with stay.
#[test]
fn a_desugarings_statements_encode_where_they_stand() {
    for program in [
        "## Main\nLet xs be [1, 5, 2, 8, 3].\nLet small be every n in xs where n is less than 4.\nShow small.",
        "## To pair (n: Int) -> (Int, Int):\n    Return (n, n * 2).\n\n## Main\nLet (a, b) be pair(3).\nShow a + b.",
        "## To spread (xs: Seq of Int) -> Int:\n    Let big be every x in xs where x is greater than 2.\n    Let (lo, hi) be (item 1 of big, item 2 of big).\n    Return lo + hi.\n\n## Main\nLet mutable total be 0.\nRepeat for i from 1 to 3:\n    Let picked be every x in [1, 2, 3, 4] where x is greater than i.\n    Set total to total + length of picked.\nShow total.\nShow spread([1, 3, 5, 7]).\nLet count be (n: Int) ->:\n    Let above be every x in [1, 2, 3, 4, 5] where x is greater than n.\n    Return length of above.\nShow count(2).",
        "## Main\nLet mutable x be 5.\nReversibly:\n    Let y be x + 3.\n    Set x to y xor 1.\nShow x.\nShow y.",
        "## Main\nLet A be a 2 by 2 tensor of Int.\nSet item 1 of A to 7.\nSet item 4 of A to 9.\nShow item 1 of A + item 4 of A.",
    ] {
        let text = encode_program_source(program).expect("the text encoder covers the program");
        assert!(!text.contains("CBool with value true"), "the text encoder wrote a block for:\n{program}\n{text}");
        let compact = encode_program_source_compact(program).expect("the compact encoder covers the program");
        assert!(!compact.contains("CBool with value true"), "the compact encoder wrote a block for:\n{program}\n{compact}");
        let tree = program_to_core_tree(program).expect("the tree encoder covers the program");
        assert!(!holds_an_always_taken_if(&tree), "the tree encoder wrote a block for:\n{program}\n{tree:?}");
    }
    let comprehension = encode_program_source("## Main\nLet xs be [1, 5, 2, 8, 3].\nLet small be every n in xs where n is less than 4.\nShow small.").expect("the text encoder covers the program");
    assert!(comprehension.contains("new CIf"), "a comprehension tests each item with an `If`:\n{comprehension}");
}
