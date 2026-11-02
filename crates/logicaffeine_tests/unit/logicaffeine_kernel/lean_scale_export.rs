//! The pinned Lean exports under `tests/fixtures/lean_scale`, read by the kernel's unit tests.
use std::collections::HashMap;

pub(crate) fn fixture(stem: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../logicaffeine_tests/tests/fixtures/lean_scale");
    std::fs::read_to_string(dir.join(format!("{stem}.ndjson"))).expect("pinned export")
}

/// The export without the declarations whose Lean names start with `prefix`: Lean's own environment
/// without the witness roots (nothing else refers to a root).
pub(crate) fn environment_without(export: &str, prefix: &str) -> String {
    let mut names: HashMap<u64, String> = HashMap::from([(0, String::new())]);
    let mut kept = String::new();
    for line in export.lines() {
        let row: serde_json::Value = serde_json::from_str(line).expect("an NDJSON record");
        if let Some(i) = row["in"].as_u64() {
            let part = row
                .get("str")
                .or_else(|| row.get("num"))
                .expect("a name part");
            let pre = names[&part["pre"].as_u64().unwrap()].clone();
            let component = part["str"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| part["i"].to_string());
            names.insert(
                i,
                if pre.is_empty() {
                    component
                } else {
                    format!("{pre}.{component}")
                },
            );
        }
        let declared = ["def", "thm", "axiom", "opaque"]
            .iter()
            .find_map(|kind| row.get(*kind))
            .and_then(|d| d["name"].as_u64());
        if declared.is_some_and(|n| names.get(&n).is_some_and(|name| name.starts_with(prefix))) {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    kept
}

/// The kernel spelling of a dotted Lean name.
pub(crate) fn spelled(lean: &str) -> String {
    crate::interface::lean::reference::kernel_name_of_components(
        &lean.split('.').collect::<Vec<_>>(),
    )
}

pub(crate) fn on_a_large_stack<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(1 << 28)
            .spawn_scoped(scope, f)
            .unwrap()
            .join()
            .unwrap()
    })
}
