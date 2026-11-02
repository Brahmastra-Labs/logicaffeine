//! ★ LOCK — a page's module is entered by its host through exactly the entries it was built for.
//!
//! `compile::compile_to_wasm_with(source, &HostEntries)` builds a module whose host may also mount
//! the named components on demand: the browser page tier builds one for a `## Test` that writes
//! `Mount X.` at a statement of its own, and `runtime.js`'s `__logosMount` calls the `__mount_<X>`
//! the module then exports. A shipped page asks for nothing, so its module is the one
//! `compile_to_wasm` writes, byte for byte, with no mount export — and when entries ARE asked for,
//! the gate that refuses what a backend cannot link judges the code those entries reach, the same
//! roots the backend lowers.
//!
//! | test | property |
//! |---|---|
//! | `a_shipped_page_is_built_exactly_as_before` | default entries ≡ `compile_to_wasm`, byte for byte, over every example page; no `__mount_` export |
//! | `a_host_mount_is_exported_only_when_asked_for` | `__mount_<C>` is a function export iff `C` is in the entries |
//! | `a_component_only_the_host_mounts_is_gated_like_one_main_mounts` | an unlinkable construct only a host entry reaches is refused by the gate, by name |
//! | `every_import_that_hands_over_host_text_is_spelled_so` | a `runtime.js` import that calls `allocText(` ⇔ its row is spelled `HOST_TEXT` |
//! | `a_module_that_takes_host_text_carries_the_allocator_the_host_writes_it_through` | such a module exports `__logos_text_new`, interactive or not |
//! | `every_import_that_hands_over_a_host_built_list_is_spelled_so` | a `runtime.js` import that calls `allocSeq(` ⇔ its row is spelled `HOST_SEQ`, and every such row is implemented |
//! | `a_module_that_takes_a_host_built_list_carries_the_allocator_the_host_builds_it_through` | a module reading a dropped file's bytes exports `__logos_seq_new` |

use logicaffeine_compile::compile;
use logicaffeine_compile::vm::reach::HostEntries;

/// Every function the module exports, by name.
fn exported_functions(bytes: &[u8]) -> Vec<String> {
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, bytes)
        .unwrap_or_else(|e| panic!("the module must be valid WebAssembly: {e}"));
    module
        .exports()
        .filter(|e| matches!(e.ty(), wasmi::ExternType::Func(_)))
        .map(|e| e.name().to_string())
        .collect()
}

/// Every page file under `examples/*/pages/`, assembled through its project closure the way
/// `largo build --static` reads it.
fn example_pages() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut out = Vec::new();
    let mut pages: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("examples"))
        .unwrap()
        .flatten()
        .map(|e| e.path().join("pages"))
        .filter(|p| p.is_dir())
        .flat_map(|dir| {
            std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .collect::<Vec<_>>()
        })
        .filter(|p| p.extension().is_some_and(|x| x == "lg"))
        .collect();
    pages.sort();
    for page in pages {
        let source = logicaffeine_compile::page_host::assemble_stable(&page)
            .unwrap_or_else(|e| panic!("could not assemble {}: {e}", page.display()));
        out.push((
            page.strip_prefix(&root).unwrap().display().to_string(),
            source,
        ));
    }
    out
}

#[test]
fn a_shipped_page_is_built_exactly_as_before() {
    let pages = example_pages();
    assert!(
        pages.len() >= 10,
        "only {} example pages found — the reader is broken",
        pages.len()
    );
    let mut compiled = 0;
    for (id, source) in &pages {
        let shipped = compile::compile_to_wasm(source).map_err(|e| format!("{e:?}"));
        let default = compile::compile_to_wasm_with(source, &HostEntries::default())
            .map_err(|e| format!("{e:?}"));
        assert!(
            shipped == default,
            "{id}: a page built with no host entries must be the very module `compile_to_wasm` \
             ships — the entries are an addition for a host that asks, never a second build"
        );
        if let Ok(bytes) = &shipped {
            compiled += 1;
            let mounts: Vec<String> = exported_functions(bytes)
                .into_iter()
                .filter(|n| n.starts_with("__mount_"))
                .collect();
            assert!(
                mounts.is_empty(),
                "{id}: a shipped page exports no mount for its host: {mounts:?}"
            );
        }
    }
    assert!(
        compiled >= 10,
        "only {compiled} example pages compiled to WebAssembly — nothing was compared"
    );
}

const WIDGET: &str = "\
## Component Widget
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p><button id=\"b\" on-click={bump}>b</button>

## Component Other
View:
    <p>other</p>
";

#[test]
fn a_host_mount_is_exported_only_when_asked_for() {
    let shipped = compile::compile_to_wasm(WIDGET).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        !exported_functions(&shipped)
            .iter()
            .any(|n| n.starts_with("__mount_")),
        "nothing mounts `Widget` in the program itself, and nothing asked for a host mount"
    );
    let hosted = compile::compile_to_wasm_with(WIDGET, &HostEntries::mounting(["Widget"]))
        .unwrap_or_else(|e| panic!("{e:?}"));
    let exports = exported_functions(&hosted);
    assert!(
        exports.iter().any(|n| n == "__mount_Widget"),
        "a module built for a host that mounts `Widget` exports its mount under that name: {exports:?}"
    );
    assert!(
        !exports.iter().any(|n| n == "__mount_Other"),
        "and only the mounts it was asked for: {exports:?}"
    );
}

const WIDGET_WITH_A_KERNEL: &str = "\
## Component Widget
State:
    Let n be 0.
Actions:
    To shape:
        Let z be zerosW16(2).
        Set n to length of z.
View:
    <p>{n}</p><button id=\"b\" on-click={shape}>b</button>
";

#[test]
fn a_component_only_the_host_mounts_is_gated_like_one_main_mounts() {
    assert!(
        compile::compile_to_wasm(WIDGET_WITH_A_KERNEL).is_ok(),
        "nothing in the program mounts `Widget`, so its native-kernel call cannot run and refuses \
         nothing: {:?}",
        compile::compile_to_wasm(WIDGET_WITH_A_KERNEL).err()
    );
    let hosted =
        compile::compile_to_wasm_with(WIDGET_WITH_A_KERNEL, &HostEntries::mounting(["Widget"]));
    let why = match hosted {
        Err(e) => format!("{e:?}"),
        Ok(_) => panic!(
            "a host that mounts `Widget` reaches its `zerosW16` call — the module must be refused, \
             not lowered around a kernel it cannot link"
        ),
    };
    assert!(
        why.contains("`zerosW16` is a standard-library native kernel (crypto.lg) implemented in Rust — the WebAssembly backend cannot link it"),
        "the refusal is the gate's, naming the kernel — the gate judges the roots the backend lowers: {why}"
    );
}

/// Each `env` entry of the browser host `runtime.js` defines, by name, with its source — the lines
/// from `  name:` up to the next entry, comment lines left out.
fn runtime_env_entries() -> Vec<(String, String)> {
    let runtime = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS;
    let start = runtime
        .find("\nconst env = {\n")
        .expect("runtime.js defines its `env` imports as `const env = {`");
    let end = start
        + runtime[start..]
            .find("\n};\n")
            .expect("and closes them with `};`");
    let mut entries: Vec<(String, String)> = Vec::new();
    for line in runtime[start..end].lines().skip(2) {
        let entry_name = line
            .strip_prefix("  ")
            .filter(|l| !l.starts_with(' '))
            .and_then(|l| l.split_once(':'))
            .map(|(name, _)| name)
            .filter(|name| {
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        match entry_name {
            Some(name) => entries.push((name.to_string(), line.to_string())),
            None if line.trim_start().starts_with("//") => {}
            None => {
                if let Some((_, source)) = entries.last_mut() {
                    source.push('\n');
                    source.push_str(line);
                }
            }
        }
    }
    entries
}

#[test]
fn every_import_that_hands_over_host_text_is_spelled_so() {
    let spelled: Vec<&str> = logicaffeine_compile::vm::wasm::host_text_import_names();
    let entries = runtime_env_entries();
    assert!(
        entries.len() >= 40,
        "only {} `env` entries read from runtime.js — the reader is broken",
        entries.len()
    );
    let mut wrong = Vec::new();
    for name in logicaffeine_compile::vm::wasm::host_import_names() {
        let Some((_, source)) = entries.iter().find(|(n, _)| n == name) else {
            continue;
        };
        let allocates = source.contains("allocText(");
        let flagged = spelled.contains(&name);
        if allocates != flagged {
            wrong.push(format!(
                "`{name}`: runtime.js {} a Text through `allocText(`, but its row in the host-import \
                 table is {}spelled `HOST_TEXT`",
                if allocates { "hands over" } else { "never hands over" },
                if flagged { "" } else { "not " }
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "an import that answers with a Text the host allocates makes the module carry \
         `__logos_text_new`; the table must say which ones do:\n{}",
        wrong.join("\n")
    );
    assert!(
        spelled.contains(&"dom_route_path"),
        "the census must see the rows it checks: {spelled:?}"
    );
}

#[test]
fn every_import_that_hands_over_a_host_built_list_is_spelled_so() {
    let spelled: Vec<&str> = logicaffeine_compile::vm::wasm::host_seq_import_names();
    let entries = runtime_env_entries();
    assert!(
        entries.len() >= 40,
        "only {} `env` entries read from runtime.js — the reader is broken",
        entries.len()
    );
    let mut wrong = Vec::new();
    for name in logicaffeine_compile::vm::wasm::host_import_names() {
        let flagged = spelled.contains(&name);
        let Some((_, source)) = entries.iter().find(|(n, _)| n == name) else {
            if flagged {
                wrong.push(format!(
                    "`{name}` is spelled `HOST_SEQ`, and runtime.js does not implement it"
                ));
            }
            continue;
        };
        let builds = source.contains("allocSeq(");
        if builds != flagged {
            wrong.push(format!(
                "`{name}`: runtime.js {} a list through `allocSeq(`, but its row in the host-import \
                 table is {}spelled `HOST_SEQ`",
                if builds { "hands over" } else { "never hands over" },
                if flagged { "" } else { "not " }
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "an import that answers with a list the host builds makes the module carry \
         `__logos_seq_new`; the table must say which ones do, and the browser must build each through \
         the module's own allocator:\n{}",
        wrong.join("\n")
    );
    assert!(
        spelled.contains(&"dom_event_file_bytes"),
        "the census must see the rows it checks: {spelled:?}"
    );
}

const READS_A_DROPPED_FILE: &str = "## Page /\n\nState:\n    Let n be 0.\nActions:\n    When a file is dropped on \"#zone\" (file: File):\n        Set n to length of file's bytes.\nView:\n    <div id=\"zone\">{n}</div>\n";

#[test]
fn a_module_that_takes_a_host_built_list_carries_the_allocator_the_host_builds_it_through() {
    let module = compile::compile_to_wasm(READS_A_DROPPED_FILE).unwrap_or_else(|e| panic!("{e:?}"));
    let exports = exported_functions(&module);
    assert!(
        exports.iter().any(|n| n == "__logos_seq_new"),
        "a dropped file's bytes are a list the host builds in the module's heap — a module that takes \
         one must export `__logos_seq_new`: {exports:?}"
    );
}

const ONLY_THE_ROUTE: &str = "## Page /\n\nView:\n    <p id=\"where\">{routePath()}</p>\n";

#[test]
fn a_module_that_takes_host_text_carries_the_allocator_the_host_writes_it_through() {
    let module = compile::compile_to_wasm(ONLY_THE_ROUTE).unwrap_or_else(|e| panic!("{e:?}"));
    let exports = exported_functions(&module);
    assert!(
        exports.iter().any(|n| n == "__logos_text_new"),
        "`routePath()` is a Text the host writes into the module's heap — a module that takes one \
         must export `__logos_text_new`: {exports:?}"
    );
}
