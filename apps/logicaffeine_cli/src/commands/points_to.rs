//! `largo points-to` — what each variable of a program may point to.
//!
//! Reads the program's statements as Datalog facts and evaluates Whaley and Lam's points-to rules over them
//! (`logicaffeine_compile::analysis::points_to`), without contexts or, with `--contexts`, every call path its own. It
//! prints each function's variables with the allocation sites each may hold, the heap's field edges, and what each
//! site allocates; `--json` prints the same as data.

use std::path::PathBuf;

use crate::ui::CliError;

/// Handle `largo points-to FILE`.
pub(crate) fn cmd_points_to(file: PathBuf, contexts: bool, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let closure = crate::closure::assemble_closure(&file)
        .map_err(|e| CliError::new(format!("could not assemble {}: {e}", file.display())))?;
    let analysis = logicaffeine_compile::analysis::points_to::points_to(&closure.source, contexts)
        .map_err(|e| CliError::new(format!("{}: {e}", file.display())))?;
    // A compiler temporary (`function.%n`) holds what an expression made on its way into a variable: it is the
    // analysis's, not the program's.
    let named = |v: &str| !v.contains(".%");
    if json {
        let variables: serde_json::Map<String, serde_json::Value> = analysis
            .variables
            .iter()
            .filter(|(v, _)| named(v))
            .map(|(v, sites)| (v.clone(), serde_json::json!(sites)))
            .collect();
        let report = serde_json::json!({
            "file": file.display().to_string(),
            "contexts": analysis.contexts.map(|n| u64::try_from(n).map_or_else(|_| serde_json::json!(n.to_string()), |n| serde_json::json!(n))),
            "variables": variables,
            "heap": analysis.heap.iter().map(|(a, f, b)| serde_json::json!([a, f, b])).collect::<Vec<_>>(),
            "sites": analysis.sites.iter().map(|(site, function, what)| {
                serde_json::json!({ "site": site, "function": function, "allocates": what })
            }).collect::<Vec<_>>(),
        });
        println!("{report}");
        return Ok(());
    }
    if let Some(n) = analysis.contexts {
        println!("{n} contexts, every call path its own");
    }
    let mut current = "";
    for (variable, sites) in analysis.variables.iter().filter(|(v, _)| named(v)) {
        let (function, name) = variable.split_once('.').unwrap_or(("", variable));
        if function != current {
            println!("{function}:");
            current = function;
        }
        println!("    {name} → {}", sites.iter().cloned().collect::<Vec<_>>().join(", "));
    }
    if !analysis.heap.is_empty() {
        println!("heap:");
        for (from, field, to) in &analysis.heap {
            println!("    {from}.{field} → {to}");
        }
    }
    println!("sites:");
    for (site, function, what) in &analysis.sites {
        println!("    {site}: {what} in {function}");
    }
    Ok(())
}
