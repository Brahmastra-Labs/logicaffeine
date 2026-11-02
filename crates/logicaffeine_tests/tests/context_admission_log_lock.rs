//! Every change to the registries a declaration lands in is logged: the context's admission log is
//! how the core's mirror (`dag::mirror`) learns what a context changed since the one it mirrors,
//! and a change the log misses is a context the core silently disagrees with.
//!
//! The registries are fields private to the kernel crate, so the lock reads the kernel's sources:
//! - an insertion goes through `Context::record` (which logs it), never `Arc::make_mut` on a
//!   registry field;
//! - a removal (`remove_shared_entry` on a registry field) happens only in `displace` and
//!   `invalidate_certificates`, which log what they remove;
//! - a registry field is never assigned, taken, replaced or swapped whole;
//! - a certificate (the standard quotient's, the observational equality's) is installed only by
//!   its own `certify_*` door and revoked only by `invalidate_certificates`, which log the names it
//!   covers.

use std::path::{Path, PathBuf};

const REGISTRIES: [&str; 9] = [
    "inductives",
    "constructors",
    "declarations",
    "definitions",
    "universe_polys",
    "poly_types",
    "inductive_params",
    "inductive_inferred_params",
    "quotient_primitives",
];

const REMOVERS: [&str; 2] = ["displace", "invalidate_certificates"];

/// Each certificate field, with the one door that installs it.
const CERTIFICATES: [(&str, &str); 2] = [
    ("prelude_quotient", "certify_prelude_quotient"),
    ("observational", "certify_observational"),
];

/// The function a line sits in: the nearest `fn` declared above it.
fn enclosing_fn(lines: &[&str], at: usize) -> Option<String> {
    lines[..=at].iter().rev().find_map(|line| {
        let code = line.trim_start();
        let rest = code.split_once("fn ").filter(|(head, _)| {
            head.split_whitespace().all(|w| {
                [
                    "pub",
                    "pub(crate)",
                    "pub(super)",
                    "const",
                    "unsafe",
                    "async",
                ]
                .contains(&w)
            })
        })?;
        Some(
            rest.1
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()?
                .to_string(),
        )
    })
}

/// A registry field written as a place: `.name` not followed by more of an identifier.
fn field_at(code: &str, field: &str) -> bool {
    let needle = format!(".{field}");
    code.match_indices(&needle).any(|(i, _)| {
        !code[i + needle.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    })
}

/// Every line of `source` that writes a registry, or a certificate, outside its door.
fn violations(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let at = || format!("line {}: {}", i + 1, line.trim());
        for field in REGISTRIES {
            if !field_at(code, field) {
                continue;
            }
            let place = format!(".{field})");
            if code.contains("Arc::make_mut(") && code.contains(&place) {
                out.push(format!("{} — insert through `Context::record`", at()));
            }
            if code.contains("remove_shared_entry(") && code.contains(&format!(".{field},")) {
                let inside = enclosing_fn(&lines, i).unwrap_or_default();
                if !REMOVERS.contains(&inside.as_str()) {
                    out.push(format!(
                        "{} — removed in `{inside}`, which does not log it",
                        at()
                    ));
                }
            }
            if ["mem::take(", "mem::replace(", "mem::swap("]
                .iter()
                .any(|m| code.contains(m))
            {
                out.push(format!("{} — a registry replaced whole", at()));
            }
            let assigned = code.match_indices(&format!(".{field}")).any(|(j, _)| {
                let after = code[j + field.len() + 1..].trim_start();
                after.starts_with('=') && !after.starts_with("==") && !after.starts_with("=>")
            });
            if assigned {
                out.push(format!("{} — a registry assigned whole", at()));
            }
        }
        for (field, door) in CERTIFICATES {
            if !field_at(code, field) {
                continue;
            }
            let inside = enclosing_fn(&lines, i).unwrap_or_default();
            let place = format!(".{field}");
            let assigned = code.match_indices(&place).any(|(j, _)| {
                let after = code[j + place.len()..].trim_start();
                after.starts_with('=') && !after.starts_with("==")
            });
            if assigned && inside != door {
                out.push(format!("{} — certify through `{door}`", at()));
            }
            if code.contains(&format!("{place}.take()")) && inside != "invalidate_certificates" {
                out.push(format!(
                    "{} — revoked in `{inside}`, which does not log it",
                    at()
                ));
            }
        }
    }
    out
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_registry_write_in_the_kernel_goes_through_a_logging_door() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_kernel/src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    assert!(
        files.len() > 100,
        "found only {} kernel sources under {}",
        files.len(),
        src.display()
    );
    let context = std::fs::read_to_string(src.join("context.rs")).expect("context.rs");
    assert!(
        context.contains("fn record<V: Clone>(")
            && CERTIFICATES
                .iter()
                .all(|(_, door)| context.contains(&format!("fn {door}(")))
            && context.contains("fn invalidate_certificates("),
        "the logging doors this lock routes writes to exist"
    );
    let mut offenders = Vec::new();
    for file in files {
        let source =
            std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        for v in violations(&source) {
            offenders.push(format!("{}: {v}", file.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "registry writes the admission log does not see:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_scan_refuses_each_unlogged_write() {
    let cases = [
        "    fn add(&mut self) {\n        Arc::make_mut(&mut self.declarations).insert(n, t);\n    }",
        "    fn add(&mut self) {\n        Arc::make_mut(&mut self.constructors)\n            .insert(n, t);\n    }",
        "    fn add(ctx: &mut Context) {\n        Arc::make_mut(&mut ctx.quotient_primitives).insert(n, c);\n    }",
        "    fn forget(&mut self, n: &str) {\n        remove_shared_entry(&mut self.definitions, n);\n    }",
        "    fn reset(&mut self) {\n        self.universe_polys = Arc::new(HashMap::new());\n    }",
        "    fn reset(&mut self) {\n        let _ = std::mem::take(&mut self.inductives);\n    }",
        "pub(super) fn register(ctx: &mut Context) {\n    ctx.prelude_quotient = Some(Arc::new(c));\n}",
        "    fn drop_quotient(&mut self) {\n        let _ = self.prelude_quotient.take();\n    }",
        "pub fn register(ctx: &mut Context) {\n    ctx.observational = Some(Arc::new(c));\n}",
        "    fn drop_observational(&mut self) {\n        let _ = self.observational.take();\n    }",
        "    fn certify_prelude_quotient(&mut self, c: Certificate) {\n        self.observational = Some(Arc::new(c));\n    }",
    ];
    for case in cases {
        assert_eq!(
            violations(case).len(),
            1,
            "refused exactly once:\n{case}\n→ {:?}",
            violations(case)
        );
    }
}

#[test]
fn the_scan_admits_the_logging_doors_and_reads() {
    let source = "\
    pub(crate) fn record<V: Clone>(&mut self, table: fn(&mut Context) -> &mut Arc<HashMap<String, V>>, name: String, value: V) {
        Arc::make_mut(table(self)).insert(name.clone(), value);
    }
    fn displace(&mut self, name: &str) {
        remove_shared_entry(&mut self.inductives, name);
        if let Some(order) = Arc::make_mut(&mut self.constructor_order).get_mut(&owner) {}
    }
    fn invalidate_certificates(&mut self, name: &str) {
        let certificate = self.prelude_quotient.take();
        remove_shared_entry(&mut self.quotient_primitives, &primitive);
        let observational = self.observational.take();
    }
    pub(crate) fn certify_prelude_quotient(&mut self, c: PreludeCertificate) {
        self.prelude_quotient = Some(Arc::new(c));
    }
    pub(crate) fn certify_observational(&mut self, c: Certificate) {
        self.observational = Some(Arc::new(c));
    }
    fn read(&self) -> bool {
        self.declarations.get(name).is_some() && self.definitions_seen == 0 && self.inductives.len() == 1
    }
    fn add(&mut self) {
        self.record(|c| &mut c.declarations, name.to_string(), ty);
        // Arc::make_mut(&mut self.declarations).insert(n, t) in a comment writes nothing.
    }";
    assert_eq!(violations(source), Vec::<String>::new());
}
