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
