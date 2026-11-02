//! The multiplier a build elaborates: a module's header default for a parameter, and the variable-by-variable
//! multiplies yosys elaborates under a top. Shared by the locks that hold each target to its multiplier
//! (csa_multiplier_lock for the engines, coproc_pipeline_lock for the coprocessor).

#![allow(dead_code)]

/// The default `module`'s header parameter list gives `param`.
pub fn header_default(text: &str, module: &str, param: &str) -> String {
    let at = text
        .find(&format!("module {module} #("))
        .unwrap_or_else(|| panic!("no module {module} with a header parameter list"));
    let header = &text[at..at + text[at..].find(") (").expect("the header's end")];
    let value = [format!("parameter {param} = "), format!("parameter integer {param} = ")]
        .iter()
        .find_map(|key| header.find(key.as_str()).map(|i| &header[i + key.len()..]))
        .unwrap_or_else(|| panic!("{module}'s header declares no {param}"));
    value.split([',', ')']).next().unwrap().trim().to_string()
}

/// The variable-by-variable multiplies yosys elaborates under `top`, as sorted (A width, B width) pairs. A multiply
/// by a constant (an index scaled by three) is no multiplier and is left out.
pub fn variable_multiplies(files: &[(&str, String)], top: &str) -> Vec<(u32, u32)> {
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let names: Vec<&str> = files.iter().map(|(n, _)| *n).collect();
    let run = std::process::Command::new("yosys")
        .current_dir(dir.path())
        .args([
            "-q",
            "-p",
            &format!(
                "read_verilog -sv {}; hierarchy -top {top}; proc; flatten; opt_expr; opt_clean; tee -q -o mul.txt dump t:$mul",
                names.join(" ")
            ),
        ])
        .output()
        .expect("run yosys");
    assert!(run.status.success(), "yosys failed on {top}: {}", String::from_utf8_lossy(&run.stderr));
    let dump = std::fs::read_to_string(dir.path().join("mul.txt")).expect("the multiply dump");
    let constant = |sig: &str| {
        let sig = sig.trim();
        sig.split_once('\'').is_some_and(|(w, bits)| {
            !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()) && !bits.is_empty() && bits.bytes().all(|b| b"01xz".contains(&b))
        })
    };
    let mut found = Vec::new();
    for cell in dump.split("  cell ").skip(1) {
        let field = |key: &str| {
            cell.lines()
                .find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim().to_string()))
                .unwrap_or_else(|| panic!("a multiply without {key}: {cell}"))
        };
        if constant(&field("connect \\A ")) || constant(&field("connect \\B ")) {
            continue;
        }
        found.push((field("parameter \\A_WIDTH ").parse().unwrap(), field("parameter \\B_WIDTH ").parse().unwrap()));
    }
    found.sort();
    found
}
