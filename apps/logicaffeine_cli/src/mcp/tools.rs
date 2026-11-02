//! The MCP tool registry: schemas and handlers for the v1 tool surface.
//!
//! Every tool accepts LOGOS source inline (`source`) or by path (`file`).
//! A tool that *completed its job* — including a check that found errors —
//! returns `isError: false`; `isError: true` means the tool itself could not
//! do its work (unreadable file, unparseable input for a transformer, an
//! unknown target). Payloads ride `structuredContent`, mirrored as pretty
//! JSON in a text content block.

use serde_json::{json, Value};

use logicaffeine_language::diagnostic::diagnostic_for;
use logicaffeine_language::{CompileOptions, Interner, OutputFormat};

use super::protocol::{self, INVALID_PARAMS};

/// The `tools/list` result: the pinned v1 tool surface.
pub fn list() -> Value {
    let source_props = |extra: Value| -> Value {
        let mut props = json!({
            "source": {"type": "string", "description": "LOGOS source text, inline"},
            "file": {"type": "string", "description": "Path to a .lg/.md file (alternative to 'source')"}
        });
        if let (Some(base), Some(more)) = (props.as_object_mut(), extra.as_object()) {
            for (k, v) in more {
                base.insert(k.clone(), v.clone());
            }
        }
        props
    };
    let schema = |props: Value| json!({"type": "object", "properties": props});

    json!({"tools": [
        {
            "name": "logos_check",
            "description": "Parse and type-check a LOGOS program without running it. \
                Returns {ok, diagnostics: [logos-diagnostic/1]} — each diagnostic carries a \
                stable code, byte+line/col span, headline, Socratic explanation, and \
                suggestion. A check that finds errors is a SUCCESSFUL call (isError stays \
                false; read `ok`). Call this after writing or editing LOGOS code, before running it.",
            "inputSchema": schema(source_props(json!({})))
        },
        {
            "name": "logos_run",
            "description": "Compile and execute a LOGOS program in a sandboxed subprocess and \
                return its output lines. Deterministic: pass `seed` to reproduce concurrent \
                scheduling byte-for-byte. Always bounded: `timeout_ms` caps wall-clock \
                (default 5000, max 60000), and `step_budget` stops the engine at a \
                deterministic op (exit_code 122 with partial output — same op every run). \
                Engine labels are STRICT: the payload reports which engine ran \
                (`engine_ran`), and a program the requested engine cannot run is a typed \
                refusal (`refused: true`, `engine_ran: null`, the reason in `error`) — \
                never a silent substitution. Use this to verify a program behaves as intended.",
            "inputSchema": schema(source_props(json!({
                "engine": {"type": "string", "enum": ["tw", "vm"], "description": "Execution engine, strict: tree-walker (reference) or bytecode VM — a program the chosen engine cannot run is refused, never rerouted", "default": "tw"},
                "seed": {"type": "integer", "description": "Scheduler seed for deterministic concurrent runs"},
                "args": {"type": "array", "items": {"type": "string"}, "description": "Program arguments"},
                "timeout_ms": {"type": "integer", "description": "Wall-clock limit in milliseconds", "default": 5000, "maximum": 60000},
                "step_budget": {"type": "integer", "description": "Interpreter step budget (fuel); unlimited when omitted"}
            })))
        },
        {
            "name": "logos_logic",
            "description": "Compile English sentences to first-order logic. Modes: 'default' \
                (one reading), 'pragmatic' (scalar implicatures), 'kripke' (modals as world \
                quantification), 'all_scopes' (every quantifier-scope reading), 'discourse' \
                (multi-sentence with pronoun resolution; one sentence per line).",
            "inputSchema": schema(source_props(json!({
                "mode": {"type": "string", "enum": ["default", "pragmatic", "kripke", "all_scopes", "discourse"], "default": "default"},
                "format": {"type": "string", "enum": ["unicode", "ascii", "latex"], "default": "unicode"}
            })))
        },
        {
            "name": "logos_prove",
            "description": "Prove every theorem in the source, kernel-certified. Accepts \
                formal developments (## Theory blocks with Axiom/Theorem declarations) and \
                English theorems (## Theorem blocks with Given:/Prove: lines). Returns \
                {axioms, theorems: [{name, verified, error}], all_verified}.",
            "inputSchema": schema(source_props(json!({})))
        },
        {
            "name": "logos_emit",
            "description": "Compile a LOGOS program and return the generated code: 'rust' \
                (the AOT backend), 'c', 'isa' (the Silicon tier), or 'wasm' \
                (direct WebAssembly, returned base64-encoded).",
            "inputSchema": schema(source_props(json!({
                "target": {"type": "string", "enum": ["rust", "c", "isa", "wasm"], "default": "rust"}
            })))
        },
        {
            "name": "logos_opts",
            "description": "Report which optimizations fire for a program on each tier: \
                {aot, run, vm} lists of optimization keywords. Useful feedback when tuning \
                generated code for performance.",
            "inputSchema": schema(source_props(json!({})))
        }
    ]})
}

/// Dispatch one `tools/call`. Returns the full response line.
pub fn call(id: &Value, params: &Value) -> String {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return protocol::error(id, INVALID_PARAMS, "tools/call needs a tool 'name'");
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let outcome = match name {
        "logos_check" => check(&args),
        "logos_run" => super::sandbox::run(&args),
        "logos_logic" => logic(&args),
        "logos_prove" => prove(&args),
        "logos_emit" => emit(&args),
        "logos_opts" => opts(&args),
        other => return protocol::error(id, INVALID_PARAMS, &format!("unknown tool '{other}'")),
    };

    let result = match outcome {
        Ok(payload) => tool_result(payload, false),
        Err(message) => tool_result(json!({"error": message}), true),
    };
    protocol::ok(id, result)
}

/// Wrap a payload as an MCP tool result: structured content plus a mirrored
/// pretty-JSON text block.
fn tool_result(payload: Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": payload,
        "isError": is_error,
    })
}

/// Resolve the program text: inline `source` wins, else read `file`.
pub(crate) fn resolve_source(args: &Value) -> Result<(String, Option<String>), String> {
    if let Some(source) = args.get("source").and_then(Value::as_str) {
        return Ok((source.to_string(), None));
    }
    if let Some(file) = args.get("file").and_then(Value::as_str) {
        let text =
            std::fs::read_to_string(file).map_err(|e| format!("cannot read '{file}': {e}"))?;
        return Ok((text, Some(file.to_string())));
    }
    Err("provide LOGOS source via 'source' (inline text) or 'file' (path)".to_string())
}

/// `logos_check`: full-pipeline parse + type-check, structured diagnostics.
fn check(args: &Value) -> Result<Value, String> {
    let (source, file) = resolve_source(args)?;
    match crate::compile::compile_program_full(&source) {
        Ok(_) => Ok(json!({"ok": true, "diagnostics": []})),
        Err(parse_error) => {
            let interner = Interner::new();
            let diagnostic = diagnostic_for(&parse_error, &source, file.as_deref(), &interner);
            Ok(json!({"ok": false, "diagnostics": [diagnostic]}))
        }
    }
}

/// The Socratic rendering of a parse error, for transformer tools where the
/// input failing to parse means the tool itself cannot do its job.
fn socratic(parse_error: &logicaffeine_language::ParseError) -> String {
    let interner = Interner::new();
    logicaffeine_language::socratic_explanation(parse_error, &interner)
}

/// `logos_logic`: English → FOL in every mode.
fn logic(args: &Value) -> Result<Value, String> {
    use logicaffeine_language::compile::{compile_discourse_with_options, compile_with_options};

    let (source, _) = resolve_source(args)?;
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("unicode");

    let output_format = match (mode, format) {
        ("kripke", _) => OutputFormat::Kripke,
        (_, "latex") => OutputFormat::LaTeX,
        (_, "ascii") => OutputFormat::SimpleFOL,
        _ => OutputFormat::Unicode,
    };
    let options = CompileOptions {
        format: output_format,
        pragmatic: mode == "pragmatic",
    };

    match mode {
        "all_scopes" => {
            let mut readings = crate::commands::logic::collect_readings(&source, options);
            if readings.is_empty() {
                // No enumerated reading: the direct path's success is reading 1.
                readings = vec![compile_with_options(&source, options).map_err(|e| socratic(&e))?];
            }
            Ok(json!({"fol": Value::Null, "readings": readings}))
        }
        "discourse" => {
            let sentences: Vec<&str> = source
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            let fol =
                compile_discourse_with_options(&sentences, options).map_err(|e| socratic(&e))?;
            Ok(json!({"fol": fol, "readings": []}))
        }
        "default" | "pragmatic" | "kripke" => {
            let fol = compile_with_options(&source, options).map_err(|e| socratic(&e))?;
            Ok(json!({"fol": fol, "readings": []}))
        }
        other => Err(format!(
            "unknown mode '{other}' (default | pragmatic | kripke | all_scopes | discourse)"
        )),
    }
}

/// `logos_prove`: every theorem in the source, kernel-certified.
fn prove(args: &Value) -> Result<Value, String> {
    let (source, file) = resolve_source(args)?;
    let label = file.as_deref().unwrap_or("<inline>");
    let locate = |offset: usize| {
        let (line, col) = logicaffeine_language::diagnostic::line_col(&source, offset);
        format!("{label}:{line}:{col}")
    };
    let report = crate::commands::prove::prove_outcomes(&source, label, &locate)
        .map_err(|e| e.to_string())?;

    let theorems: Vec<Value> = report
        .outcomes
        .iter()
        .map(|o| json!({"name": o.name, "verified": o.verified, "error": o.error}))
        .collect();
    let all_verified = report.outcomes.iter().all(|o| o.verified);
    Ok(json!({
        "file": label,
        "axioms": report.axiom_count,
        "theorems": theorems,
        "all_verified": all_verified,
    }))
}

/// `logos_emit`: generated code for one backend.
fn emit(args: &Value) -> Result<Value, String> {
    let (source, _) = resolve_source(args)?;
    let target = args.get("target").and_then(Value::as_str).unwrap_or("rust");
    match target {
        "rust" => {
            let code = crate::compile::compile_to_rust(&source).map_err(|e| socratic(&e))?;
            Ok(json!({"target": "rust", "code": code}))
        }
        "c" => {
            let code = crate::compile::compile_to_c(&source).map_err(|e| socratic(&e))?;
            Ok(json!({"target": "c", "code": code}))
        }
        "isa" => {
            let code = crate::compile::compile_to_isa(&source).map_err(|e| socratic(&e))?;
            Ok(json!({"target": "isa", "code": code}))
        }
        "wasm" => {
            let bytes = crate::compile::compile_to_wasm(&source).map_err(|e| socratic(&e))?;
            Ok(json!({
                "target": "wasm",
                "wasm_base64": base64(&bytes),
                "bytes": bytes.len(),
            }))
        }
        other => Err(format!("unknown target '{other}' (rust | c | isa | wasm)")),
    }
}

/// `logos_opts`: fired optimizations per tier.
fn opts(args: &Value) -> Result<Value, String> {
    let (source, _) = resolve_source(args)?;
    // Surface a parse failure as a tool error instead of three empty lists.
    crate::compile::compile_program_full(&source).map_err(|e| socratic(&e))?;
    // The all-on evaluation `largo opts` prints: what actually FIRED on the
    // AOT path, plus the interpreter/VM tier reports.
    let (fired, _blockers, _dependencies) = crate::compile::optimization_graph(&source);
    Ok(json!({
        "aot": fired,
        "run": crate::compile::optimizations_fired_run(&source),
        "vm": crate::compile::optimizations_fired_vm(&source),
    }))
}

/// Standard padded base64 — the one place largo needs it does not justify a
/// dependency.
pub(crate) fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = u32::from_be_bytes([
            0,
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ]);
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/mcp/tools/tests.rs"]
mod tests;
