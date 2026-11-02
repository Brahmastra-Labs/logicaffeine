//! ★ LOCK — one declared list of the functions a HOST enters a compiled page by name. ★
//!
//! `lgx_lower::LGX_ENTRIES` is the single source: a page's wasm module exports every entry the
//! lowering defined under the entry's own name (and nothing else but what the module itself mints,
//! `vm::wasm::runtime_export_names`), the dead-code walk roots at them, `runtime.js` enters the
//! module only through them, and the in-process page host calls only them. Each census reads the
//! SOURCE of the side it checks, and each can fail — the last rows feed it a name nobody declared.

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::wasm::runtime_export_names;
use logicaffeine_language::ast::stmt::Stmt;
use logicaffeine_language::parser::lgx_lower::{self, host_entry_of, EntryHost, LGX_ENTRIES};

/// A page with a listener, a timer, a route trigger and a failure handler — every entry a browser
/// page's lowering defines by itself.
const PAGE: &str = "\
## Component App
State:
    Let n be 0.
    Let why be \"\".
Actions:
    To bump:
        Set n to n + 1.
    Every 1 second:
        Set n to n + 1.
    When the route changes:
        Set n to 0.
    When an action fails (e: Text):
        Set why to e.
View:
    <button id=\"b\" on-click={bump}>{n}</button><p>{why}</p>

## Main
Mount App.
";

fn repo_file(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Every function the lowering of `src` defines, by name.
fn lowered_functions(src: &str) -> Vec<String> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.unwrap_or_else(|e| panic!("the page parses: {e}"));
        stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, .. } => Some(interner.resolve(*name).to_string()),
                _ => None,
            })
            .collect()
    })
}

/// The export names of a wasm module.
fn exports(module: &[u8]) -> Vec<String> {
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, module).expect("a valid module");
    module.exports().map(|e| e.name().to_string()).collect()
}

/// Whether `name` is something a host may enter a page module by: a declared entry, or a name the
/// module mints for itself.
fn declared(name: &str) -> bool {
    host_entry_of(name).is_some() || runtime_export_names().contains(&name)
}

#[test]
fn every_page_entry_the_lowering_defines_is_exported_under_its_own_name() {
    let defined = lowered_functions(PAGE);
    let exported = exports(&compile_to_wasm(PAGE).expect("the page compiles to wasm"));
    let fixed: Vec<&str> = LGX_ENTRIES
        .iter()
        .filter(|e| !e.family && e.host == EntryHost::Page)
        .map(|e| e.name)
        .collect();
    for must in [
        lgx_lower::dispatch_node_fn_name(),
        lgx_lower::on_failure_fn_name(),
        lgx_lower::tick_fn_name(),
        lgx_lower::route_changed_fn_name(),
    ] {
        assert!(
            fixed.contains(&must),
            "`{must}` is a page entry, declared in LGX_ENTRIES"
        );
        assert!(
            defined.iter().any(|d| d == must),
            "the fixture's lowering defines `{must}`: {defined:?}"
        );
    }
    let missing: Vec<&&str> = fixed
        .iter()
        .filter(|e| defined.iter().any(|d| d == **e) && !exported.iter().any(|x| x == **e))
        .collect();
    assert!(missing.is_empty(), "page entries the lowering defined and the module does not export: {missing:?}\nexports: {exported:?}");
}

#[test]
fn a_page_module_exports_nothing_undeclared() {
    let exported = exports(&compile_to_wasm(PAGE).expect("the page compiles to wasm"));
    let stray: Vec<&String> = exported.iter().filter(|x| !declared(x)).collect();
    assert!(
        stray.is_empty(),
        "exports neither an LGX entry nor the module's own: {stray:?}"
    );
}

#[test]
fn a_test_only_entry_is_not_exported_to_a_browser() {
    let exported = exports(&compile_to_wasm(PAGE).expect("the page compiles to wasm"));
    let test_only: Vec<&String> = exported
        .iter()
        .filter(|x| host_entry_of(x).is_some_and(|e| e.host != EntryHost::Page))
        .collect();
    assert!(
        test_only.is_empty(),
        "a shipped page exports an entry only a test or a server enters: {test_only:?}"
    );
}

/// Every name `js` enters the module by: `exports.NAME`, and `exports['PREFIX' + …]` as the family
/// `PREFIX*`.
fn js_entries(js: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = js;
    while let Some(at) = rest.find("exports") {
        rest = &rest[at + "exports".len()..];
        let name: String = if let Some(tail) = rest.strip_prefix('.') {
            tail.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect()
        } else if let Some(tail) = rest.strip_prefix("['") {
            let prefix: String = tail.chars().take_while(|c| *c != '\'').collect();
            format!("{prefix}*")
        } else {
            continue;
        };
        if !name.is_empty() {
            out.push(name);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The names in `js_entries(js)` that are not declared: a name no row answers, or a prefix no
/// family is.
fn undeclared_js_entries(js: &str) -> Vec<String> {
    js_entries(js)
        .into_iter()
        .filter(|n| match n.strip_suffix('*') {
            Some(prefix) => !LGX_ENTRIES.iter().any(|e| e.family && e.name == prefix),
            None => !declared(n),
        })
        .collect()
}

#[test]
fn runtime_js_enters_the_module_only_by_declared_names() {
    let js = repo_file("crates/logicaffeine_compile/assets/web/runtime.js");
    let names = js_entries(&js);
    assert!(
        names.len() >= 6,
        "read only {names:?} out of runtime.js — the reader stopped seeing its calls"
    );
    assert!(
        names.iter().any(|n| n == "__mount_*"),
        "the family prefix is read: {names:?}"
    );
    let stray = undeclared_js_entries(&js);
    assert!(
        stray.is_empty(),
        "runtime.js enters the module by names nothing declares: {stray:?}"
    );
}

/// Every name `rs` (the in-process page host's source) passes to `.call(`: a string literal, a
/// `const` of this file, or a local bound from an `lgx_lower` naming function — each resolved to the
/// name it produces. `Err` names a naming function this census cannot resolve.
fn host_calls(rs: &str) -> Result<Vec<String>, String> {
    let resolve = |f: &str| -> Option<String> {
        Some(match f {
            "dispatch_node_fn_name" => lgx_lower::dispatch_node_fn_name().to_string(),
            "on_failure_fn_name" => lgx_lower::on_failure_fn_name().to_string(),
            "failure_context_fn_name" => lgx_lower::failure_context_fn_name().to_string(),
            "tick_fn_name" => lgx_lower::tick_fn_name().to_string(),
            "route_changed_fn_name" => lgx_lower::route_changed_fn_name().to_string(),
            "raise_fn_name" => lgx_lower::raise_fn_name().to_string(),
            "mount_fn_name" => lgx_lower::mount_fn_name("Probe"),
            "action_fn_name" => lgx_lower::action_fn_name("Probe", "probe"),
            _ => return None,
        })
    };
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(off) = rs[from..].find(".call(") {
        let at = from + off;
        from = at + ".call(".len();
        let arg: String = rs[from..]
            .chars()
            .take_while(|c| *c != ',' && *c != ')')
            .collect();
        let arg = arg.trim().trim_start_matches('&');
        if let Some(lit) = arg.strip_prefix('"') {
            out.push(lit.trim_end_matches('"').to_string());
            continue;
        }
        let before = &rs[..at];
        if let Some(c) = before.rfind(&format!("const {arg}: &str = \"")) {
            let v = &before[c + format!("const {arg}: &str = \"").len()..];
            out.push(v[..v.find('"').unwrap()].to_string());
            continue;
        }
        let binding = format!("let {arg} = lgx_lower::");
        let Some(b) = before.rfind(&binding) else {
            return Err(format!(
                "`.call({arg}, …)` passes a name this census cannot trace to a declaration"
            ));
        };
        let f: String = before[b + binding.len()..]
            .chars()
            .take_while(|c| *c != '(')
            .collect();
        out.push(resolve(&f).ok_or_else(|| format!("the page host enters the page through `lgx_lower::{f}`, which no LGX_ENTRIES row answers for"))?);
    }
    Ok(out)
}

#[test]
fn the_page_host_enters_the_page_only_by_declared_entries() {
    let rs = repo_file("crates/logicaffeine_compile/src/page_host/lowered.rs");
    let calls = host_calls(&rs).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        calls.len() >= 6,
        "read only {calls:?} out of lowered.rs — the reader stopped seeing its calls"
    );
    let stray: Vec<&String> = calls
        .iter()
        .filter(|c| host_entry_of(c).is_none())
        .collect();
    assert!(
        stray.is_empty(),
        "the page host calls names nothing declares: {stray:?}"
    );
}

#[test]
fn every_entry_row_says_why_a_host_enters_it() {
    for e in LGX_ENTRIES {
        assert!(
            !e.why.trim().is_empty() && e.why.ends_with('.'),
            "`{}` needs its reason, a sentence: {:?}",
            e.name,
            e.why
        );
        assert_eq!(
            LGX_ENTRIES.iter().filter(|o| o.name == e.name).count(),
            1,
            "`{}` is declared once",
            e.name
        );
    }
    assert!(
        host_entry_of(&lgx_lower::mount_fn_name("Counter")).is_some_and(|e| e.family),
        "a family answers for its members"
    );
    assert!(
        host_entry_of("__mount_").is_none(),
        "a family's bare prefix is no member"
    );
}

#[test]
fn the_censuses_can_fail() {
    let js =
        "inst.exports.main(); inst.exports.__lgx_undeclared(1); inst.exports['__mount_' + c](p); \
              inst.exports['__nope_' + c](p);";
    assert_eq!(
        undeclared_js_entries(js),
        vec!["__lgx_undeclared".to_string(), "__nope_*".to_string()]
    );
    let rs = "let f = lgx_lower::render_fn_name(\"C\");\nself.guarded(|c| c.call(f, args));\n";
    assert!(
        host_calls(rs).is_err(),
        "a call through an unresolvable naming function is refused"
    );
    let rs = "self.guarded(|c| c.call(\"undeclaredEntry\", args));\n";
    let calls = host_calls(rs).expect("a literal resolves");
    assert!(
        calls.iter().any(|c| host_entry_of(c).is_none()),
        "an undeclared literal is seen"
    );
}

/// Two components, only `Shown` mounted: `Hidden`'s action is compiled but nothing draws it.
const ONE_OF_TWO: &str = "\
## Component Shown
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <button id=\"s\" on-click={bump}>{n}</button>

## Component Hidden
State:
    Let m be 0.
Actions:
    To poke:
        Set m to m + 1.
View:
    <button id=\"h\" on-click={poke}>{m}</button>

## Main
Mount Shown.
";

/// The names of the functions `src` compiles to that the dead-code walk reaches from Main and
/// the page entries, plus the host mounts `mounts`.
fn reached(src: &str, mounts: &[&str]) -> Vec<String> {
    use logicaffeine_compile::vm::reach::{code_layout, reach_with, HostEntries};
    logicaffeine_compile::ui_bridge::with_monomorphic_program_checked(
        src,
        logicaffeine_compile::monomorphize::Options {
            lower_option: false,
        },
        |parsed, interner| {
            let (stmts, types, _) = parsed?;
            let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
            let program = logicaffeine_compile::vm::Compiler::compile_with_oracle_located(
                stmts,
                interner,
                Some(types),
                Some(oracle),
            )?;
            let layout = code_layout(&program).expect("a resolvable layout");
            let r = reach_with(
                &program,
                &layout,
                interner,
                &HostEntries::mounting(mounts.iter().copied()),
            );
            Ok(program
                .functions
                .iter()
                .enumerate()
                .filter(|(fi, _)| r.functions[*fi])
                .map(|(_, f)| interner.resolve(f.name).to_string())
                .collect::<Vec<_>>())
        },
    )
    .unwrap_or_else(|e| panic!("the page compiles: {e:?}"))
}

#[test]
fn a_rooted_dispatcher_reaches_only_the_actions_of_components_something_draws() {
    let shipped = reached(ONE_OF_TWO, &[]);
    assert!(
        shipped
            .iter()
            .any(|n| n == lgx_lower::dispatch_node_fn_name()),
        "the dispatcher is a root: {shipped:?}"
    );
    assert!(
        shipped
            .iter()
            .any(|n| *n == lgx_lower::action_fn_name("Shown", "bump")),
        "the mounted component's action is reached through the dispatcher: {shipped:?}"
    );
    assert!(
        !shipped.iter().any(|n| n.starts_with(&lgx_lower::action_fn_name("Hidden", ""))),
        "nothing draws `Hidden`, so the dispatcher's arm for its action must not reach it: {shipped:?}"
    );
    let hosted = reached(ONE_OF_TWO, &["Hidden"]);
    assert!(
        hosted
            .iter()
            .any(|n| *n == lgx_lower::action_fn_name("Hidden", "poke")),
        "a host that may mount `Hidden` draws it, so its action is reached: {hosted:?}"
    );
}

#[test]
fn a_dispatcher_arm_for_a_component_nothing_draws_lowers_to_a_trap_and_the_module_validates() {
    // The dispatcher is live and its arm for `Hidden` is live code; `Hidden`'s action is not
    // lowered, so the arm's call is the trap the reach analysis proved never runs — and the module
    // is valid WebAssembly.
    let module =
        compile_to_wasm(ONE_OF_TWO).unwrap_or_else(|e| panic!("the page compiles to wasm: {e:?}"));
    let exported = exports(&module);
    assert!(
        exported
            .iter()
            .any(|n| n == lgx_lower::dispatch_node_fn_name()),
        "the dispatcher is exported: {exported:?}"
    );
}
