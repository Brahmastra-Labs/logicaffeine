//! The C oracle that travels with a bundle: the same GEMM over the same fills in exact integer arithmetic,
//! printing the checksum the host prints — so a bundle can be checked without the LOGOS toolchain.

use super::SpatialLowerError;
use crate::spatial::ir::*;

fn fill_c(t: &TensorDecl, var: &str) -> Result<String, SpatialLowerError> {
    let n = t.elems.rows * t.elems.cols;
    Ok(match &t.init {
        TensorInit::Zero => String::new(),
        TensorInit::Pattern { modulus, shift } => format!("    for (size_t p = 0; p < {n}; ++p) {var}[p] = (int64_t)(p % {modulus}) + ({shift});\n"),
        TensorInit::Values(v) => {
            let lits: Vec<String> = v.iter().map(|x| x.to_string()).collect();
            format!("    {{ static const int64_t vals[{}] = {{{}}}; for (size_t p = 0; p < {n}; ++p) {var}[p] = vals[p]; }}\n", v.len().max(1), lits.join(", "))
        }
        TensorInit::FloatValues(_) | TensorInit::ConstF(_) => return Err(SpatialLowerError::Unsupported("an integer oracle for a float fill")),
        TensorInit::RandomBf16 { .. } => return Err(SpatialLowerError::Unsupported("an integer oracle for a random bf16 fill")),
        TensorInit::External => return Err(SpatialLowerError::Unsupported("an integer oracle for external data")),
    })
}

/// Why a program has no exact integer reference — the text the oracle prints instead of a checksum.
///
/// ★ THIS IS A REFUSAL TO LIE, NOT A GAP. The oracle's contract is *exact integer arithmetic*; a
/// float program has no exact integer answer, so emitting one would be a checksum that disagrees
/// with the hardware for reasons that are correct. The program still ships an oracle file — it just
/// says, in the bundle, which reference actually governs.
fn no_integer_reference(
    p: &SpatialProgram,
    inputs: &[&TensorDecl],
    gemm: bool,
) -> Option<&'static str> {
    if !gemm {
        return Some("not an integer GEMM: no integer reference");
    }
    if inputs
        .iter()
        .any(|t| matches!(t.init, TensorInit::RandomBf16 { .. }))
    {
        return Some("random-bf16 fill: no integer reference");
    }
    if inputs
        .iter()
        .any(|t| matches!(t.init, TensorInit::FloatValues(_)))
    {
        return Some("float fill: no integer reference");
    }
    let _ = p;
    None
}

pub fn print_oracle(p: &SpatialProgram) -> Result<String, SpatialLowerError> {
    let inputs: Vec<&TensorDecl> = p
        .tensors
        .iter()
        .filter(|t| t.role != TensorRole::Output)
        .collect();
    let out = p.output_tensor().ok_or(SpatialLowerError::Unsupported(
        "an oracle for a program with no output",
    ))?;
    if inputs.is_empty() {
        return Err(SpatialLowerError::Unsupported(
            "an oracle for a program with at least one input",
        ));
    }
    // The same GEMM test the host uses, so the two never disagree about which shape a program is.
    let a = inputs[0];
    let gemm = inputs.len() == 2
        && inputs[1].elems.rows == a.elems.cols
        && out.elems.rows == a.elems.rows
        && out.elems.cols == inputs[1].elems.cols;
    let mut c = String::new();
    c.push_str(
        "// logos_oracle.c — the bundle's exact integer reference (printed from the spatial IR).\n",
    );
    c.push_str("#include <stdio.h>\n#include <stdint.h>\n#include <stdlib.h>\n\n");
    c.push_str("int main(void) {\n");
    if let Some(why) = no_integer_reference(p, &inputs, gemm) {
        c.push_str(&format!("    printf(\"LOGOS_ORACLE {why} (the host's fp32 golden is the reference)\\n\");\n    return 0;\n}}\n"));
        return Ok(c);
    }
    let b = inputs[1];
    let (m, k, n) = (a.elems.rows, a.elems.cols, b.elems.cols);
    c.push_str(&format!(
        "    int64_t* a = (int64_t*)calloc({}, sizeof(int64_t));\n",
        m * k
    ));
    c.push_str(&format!(
        "    int64_t* b = (int64_t*)calloc({}, sizeof(int64_t));\n",
        k * n
    ));
    c.push_str(&fill_c(a, "a")?);
    c.push_str(&fill_c(b, "b")?);
    c.push_str("    long long checksum = 0;\n");
    c.push_str(&format!("    for (size_t i = 0; i < {m}; ++i) for (size_t j = 0; j < {n}; ++j) {{\n        int64_t acc = 0;\n        for (size_t kk = 0; kk < {k}; ++kk) acc += a[i * {k} + kk] * b[kk * {n} + j];\n        checksum += acc;\n    }}\n"));
    c.push_str("    printf(\"LOGOS_CHECKSUM %lld\\n\", checksum);\n    free(a); free(b);\n    return 0;\n}\n");
    Ok(c)
}
