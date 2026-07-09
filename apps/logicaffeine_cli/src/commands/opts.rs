//! `largo opts` — report which optimizations actually fire for a program.

use std::fs;

/// Handle `largo opts <file>`: print the fired/blocker/dependency optimization graph.
pub(crate) fn cmd_opts(file: &std::path::Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    use logicaffeine_compile::optimization::REGISTRY;
    let source = fs::read_to_string(file)?;

    // The complete per-program optimization graph from one all-on evaluation: what
    // FIRED, the BLOCKERS (precedence preemptions that occurred), and the emergent
    // DEPENDENCIES (one optimization only fired because another was on). All on the
    // AOT codegen path, so the three views agree with the generated Rust.
    let (fired, blockers, dependencies) = crate::compile::optimization_graph(&source);

    if json {
        let arr = |v: &[&str]| -> String {
            v.iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(",")
        };
        let pairs = |v: &[(&str, &str)]| -> String {
            v.iter().map(|(a, b)| format!("[\"{a}\",\"{b}\"]")).collect::<Vec<_>>().join(",")
        };
        println!(
            "{{\"fired\":[{}],\"blockers\":[{}],\"dependencies\":[{}]}}",
            arr(&fired),
            pairs(&blockers),
            pairs(&dependencies)
        );
        return Ok(());
    }

    if fired.is_empty() {
        println!("No optimizations fired for {}.", file.display());
        return Ok(());
    }

    let fired_set: std::collections::BTreeSet<&str> = fired.iter().copied().collect();
    println!(
        "Optimizations that fired for {} ({} of {}):",
        file.display(),
        fired.len(),
        REGISTRY.len()
    );
    // List in registry order, grouped by category.
    let mut last_group = "";
    for m in REGISTRY {
        if !fired_set.contains(m.keyword) {
            continue;
        }
        if m.group != last_group {
            println!("  {}", m.group);
            last_group = m.group;
        }
        println!("    {:<14} {}", m.keyword, m.label);
    }
    if !dependencies.is_empty() {
        println!("Dependencies (one fired only because another was on):");
        for (dependent, dep) in &dependencies {
            println!("    {dependent:<14} depends on {dep}");
        }
    }
    if !blockers.is_empty() {
        println!("Blockers (one took precedence, skipping another):");
        for (winner, loser) in &blockers {
            println!("    {winner:<14} blocks    {loser}");
        }
    }
    Ok(())
}
