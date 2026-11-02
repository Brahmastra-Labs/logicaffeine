//! The `.rodata` pool for the RISC-V backend: the fixed `printf`/`puts` format strings the emitter
//! needs (integer format, the two boolean words) plus interned user text constants (Phase 3). Labels
//! are stable and unique so `la` can address each from the code section.

/// Collects the read-only data a program's assembly needs, emitting a single `.rodata` block at the end.
#[derive(Default)]
pub(super) struct RoData {
    /// `.Lfmt_i64: .string "%ld\n"` — the Show(Int) format.
    pub need_fmt_i64: bool,
    /// `.Lstr_true` / `.Lstr_false` — the Show(Bool) words.
    pub need_bool: bool,
    /// The ApproxEq tolerance constants (`1e-9` relative, `1e-12` absolute floor).
    pub need_approx_consts: bool,
    /// Interned f64 constants (raw bits), label = `.Lflt_<index>`, emitted as `.dword`.
    floats: Vec<u64>,
    /// Interned user text constants (Phase 3), label = `.Lstr_<index>`.
    strings: Vec<String>,
    /// Labelled tables of 8-byte words (`.quad` operands — numbers or labels), each 8-byte aligned.
    tables: Vec<(String, Vec<String>)>,
}

impl RoData {
    /// Intern an f64 constant (by raw bits), returning its `.rodata` label.
    pub fn intern_float(&mut self, bits: u64) -> String {
        let idx = self.floats.len();
        self.floats.push(bits);
        format!(".Lflt_{idx}")
    }

    /// Intern a user text constant, returning its `.rodata` label. (Phase 3.)
    #[allow(dead_code)]
    pub fn intern_text(&mut self, s: &str) -> String {
        let idx = self.strings.len();
        self.strings.push(escape_asm_string(s));
        format!(".Lstr_{idx}")
    }

    /// Add a table of 8-byte words under `label`.
    pub fn table(&mut self, label: String, words: Vec<String>) {
        self.tables.push((label, words));
    }

    fn is_empty(&self) -> bool {
        !self.need_fmt_i64
            && !self.need_bool
            && !self.need_approx_consts
            && self.floats.is_empty()
            && self.strings.is_empty()
            && self.tables.is_empty()
    }

    /// Emit the collected read-only data as a `.rodata` section (empty string if nothing is needed).
    pub fn emit(&self, out: &mut String) {
        if self.is_empty() {
            return;
        }
        out.push_str("\n  .section .rodata\n  .align 3\n");
        if self.need_fmt_i64 {
            out.push_str("  .Lfmt_i64: .string \"%ld\\n\"\n");
        }
        if self.need_bool {
            out.push_str("  .Lstr_true: .string \"true\"\n");
            out.push_str("  .Lstr_false: .string \"false\"\n");
        }
        if self.need_approx_consts {
            // 1e-9 (relative tolerance) and 1e-12 (absolute floor) as raw f64 bits.
            out.push_str(&format!("  .Lapprox_rel: .dword {}\n", 1e-9_f64.to_bits()));
            out.push_str(&format!("  .Lapprox_abs: .dword {}\n", 1e-12_f64.to_bits()));
        }
        for (i, bits) in self.floats.iter().enumerate() {
            out.push_str(&format!("  .Lflt_{i}: .dword {bits}\n"));
        }
        for (i, s) in self.strings.iter().enumerate() {
            out.push_str(&format!("  .Lstr_{i}: .string \"{s}\"\n"));
        }
        for (label, words) in &self.tables {
            out.push_str(&format!("  .align 3\n{label}:\n"));
            for w in words {
                out.push_str(&format!("  .quad {w}\n"));
            }
        }
    }
}

/// Escape a Rust string into a GNU-as `.string` literal body (which itself re-interprets C escapes).
pub(super) fn escape_asm_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\r' => out.push_str("\\r"),
            0x20..=0x7e => out.push(b as char),
            other => out.push_str(&format!("\\{other:03o}")),
        }
    }
    out
}
