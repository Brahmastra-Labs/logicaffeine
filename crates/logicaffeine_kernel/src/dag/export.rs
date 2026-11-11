//! Reading a `lean4export` NDJSON stream straight into the kernel core.
//!
//! The export is already a maximally shared DAG: every name, level and expression is a numbered
//! record defined before it is used. Each record becomes one interned node, so the export's
//! sharing is preserved exactly and nothing is ever expanded into a tree. Declarations are checked
//! the moment they arrive; nothing of the input is retained but the interned DAG.

use super::check::{Decl, DefnDecl, Header};
use super::env::{Const, CtorInfo, Hint, InductiveInfo, Kind, QuotKind, RecInfo, RecRule, Safety};
use super::error::{FailKind, KResult, KernelFailure};
use super::expr::{ExprId, Node};
use super::inductive::{IndType, InductiveDecl};
use super::json::{Tape, Tok};
use super::level::{LevelId, LevelNode};
use super::name::NameId;
use super::nat::Nat;
use super::tc::Kernel;
use rustc_hash::FxHashMap as FxMap;
use std::io::BufRead;
use std::rc::Rc;

const ABSENT: u32 = u32::MAX;

/// Export index → interned id; dense for the usual contiguous numbering, a map for the rest.
struct IdMap {
    dense: Vec<u32>,
    sparse: FxMap<u64, u32>,
}

impl IdMap {
    fn new() -> IdMap {
        IdMap {
            dense: Vec::new(),
            sparse: FxMap::default(),
        }
    }

    #[inline]
    fn insert(&mut self, k: u64, v: u32) -> bool {
        if k == self.dense.len() as u64 && k < u32::MAX as u64 {
            self.dense.push(v);
            return true;
        }
        if k < (self.dense.len() as u64) * 2 + 1024 && k < u32::MAX as u64 {
            let k = k as usize;
            if self.dense.len() <= k {
                self.dense.resize(k + 1, ABSENT);
            }
            let fresh = self.dense[k] == ABSENT;
            self.dense[k] = v;
            fresh
        } else {
            self.sparse.insert(k, v).is_none()
        }
    }

    fn get(&self, k: u64) -> Option<u32> {
        match self.dense.get(k as usize) {
            Some(&v) if v != ABSENT => Some(v),
            _ => self.sparse.get(&k).copied(),
        }
    }
}

pub struct Stats {
    /// The kernel's memory breakdown at the end of the run.
    pub memory: Vec<(&'static str, usize)>,
    pub records: u64,
    pub declarations: u64,
    /// Primitive `Nat` operations proven to meet their equations, and so computed natively.
    pub validated_primitives: Vec<String>,
}

/// Observes each declaration the reader hands to the kernel: `begin` just before the kernel
/// checks it and `end` just after, with its name, its record kind and whether it was admitted. A
/// measurement hook; checking never depends on it.
pub trait DeclMeter {
    fn begin(&mut self);
    fn end(&mut self, name: &str, kind: &'static str, admitted: bool);
}

/// How a run is observed, beside the kernel's own configuration.
#[derive(Default)]
pub struct Observe {
    /// Told about every declaration the kernel checks.
    pub meter: Option<Box<dyn DeclMeter>>,
    /// The declarations before this index are admitted as the export states them, unchecked —
    /// except the quotient's constants and any declaration or inductive block taking a primitive's
    /// or a quotient's name, which are checked so that the literal operations validated and the
    /// quotient installed match a full run. A diagnostic for
    /// measuring declarations deep in an export without re-checking its prefix: a run with a
    /// trusted prefix claims nothing, and always declines.
    pub trust_before: u64,
}

pub struct Reader<'k> {
    pub k: &'k mut Kernel,
    names: IdMap,
    levels: IdMap,
    exprs: IdMap,
    pending_mutual: Vec<(Rc<[NameId]>, DefnDecl)>,
    /// Named reference terms (`{"ref":{"key":…,"value":…}}`), for files of kernel-internal terms.
    pub refs: FxMap<String, ExprId>,
    /// Read and intern every record but check no declaration: a measurement of the reader alone.
    pub parse_only: bool,
    pub stats: Stats,
    pub meter: Option<Box<dyn DeclMeter>>,
    /// See [`Observe::trust_before`].
    pub trust_before: u64,
    /// Read the common record layouts byte for byte (the default). Off, every record goes
    /// through the general parser — which must build exactly the same store.
    pub fast_paths: bool,
    /// Admit leaf declarations (theorems, opaques, axioms) unchecked while the environment is built,
    /// collecting them in `deferred` to be re-checked over the finished environment. The arena entry
    /// sets this to spread the leaf checks across cores; off, every declaration is checked in order.
    pub defer_leaves: bool,
    /// Also defer non-primitive definitions (not only the leaves). A definition's body is δ-unfolded
    /// by later checks, but `admit` records the same env entry (body, height, type) a checked
    /// definition would, so a later unfold sees identical state and an invalid definition is still
    /// caught by its own re-check — only its self-check moves to a worker. Off by default; it widens
    /// the parallel share at the cost of re-checking definitions over private per-worker caches.
    pub defer_defns: bool,
    /// The leaves admitted unchecked under `defer_leaves`, in file order, for re-checking.
    pub deferred: Vec<Decl>,
}

enum NamePart<'a> {
    Str(&'a str),
    Num(u64),
}

/// Powers of ten up to one eight-digit chunk's scale.
const POW10: [u64; 9] = [
    1,
    10,
    100,
    1_000,
    10_000,
    100_000,
    1_000_000,
    10_000_000,
    100_000_000,
];

/// Eight input bytes, each XOR `'0'` (so a digit becomes its value, first byte lowest): how many
/// lead with digits, and their value — the whole run converted with three multiply-adds instead
/// of a loop over bytes.
#[inline(always)]
fn swar_digits(x: u64) -> (u32, u64) {
    // A byte is a digit iff it is now at most 9; `b + 0x76` sets bit 7 exactly for bytes above 9
    // (a carry out of a byte only happens above 0x89, which is already not a digit, and only
    // disturbs later bytes, past the first non-digit).
    let non_digit = (x.wrapping_add(0x7676_7676_7676_7676) | x) & 0x8080_8080_8080_8080;
    let n = if non_digit == 0 {
        8
    } else {
        non_digit.trailing_zeros() / 8
    };
    if n == 0 {
        return (0, 0);
    }
    // Keep the digits, moved to the high bytes: the leading positions become zeros, which are
    // leading zeros of the number.
    let mut d = x << (64 - 8 * n);
    d = (d * 10 + (d >> 8)) & 0x00FF_00FF_00FF_00FF;
    d = (d * 100 + (d >> 16)) & 0x0000_FFFF_0000_FFFF;
    d = (d * 10_000 + (d >> 32)) & 0x0000_0000_FFFF_FFFF;
    (n, d)
}

/// The window a fixed-layout record read may look at: every load of [`wide_app`] stays inside it,
/// so one length check covers them all.
const WIDE: usize = 64;

#[inline(always)]
fn word(w: &[u8; WIDE], at: usize) -> u64 {
    u64::from_le_bytes(*w[at..].first_chunk::<8>().unwrap())
}

/// An id of one to eight digits at `at`, without a leading zero: its value and its length. The
/// caller checks the byte after it, which rejects a ninth digit.
#[inline(always)]
fn wide_id(w: &[u8; WIDE], at: usize) -> Option<(u64, usize)> {
    let x = word(w, at) ^ 0x3030_3030_3030_3030;
    let (n, v) = swar_digits(x);
    if n == 0 || (n > 1 && x & 0xFF == 0) {
        return None;
    }
    Some((v, n as usize))
}

/// `{"app":{"arg":A,"fn":F},"ie":I}` and its newline, each id of at most eight digits, from the
/// front of a window: eight word loads and compares, no loop over bytes. Anything else — a longer
/// id, a leading zero, other spacing — is `None`, for the general record path. Returns the bytes
/// consumed and the ids `(I, F, A)`.
#[inline(always)]
fn wide_app(w: &[u8; WIDE]) -> Option<(usize, u64, u64, u64)> {
    const HEAD: u64 = u64::from_le_bytes(*b"{\"app\":{");
    const ARG: u64 = u64::from_le_bytes(*b":{\"arg\":");
    const FN: u64 = u64::from_le_bytes(*b",\"fn\":\0\0");
    const IE: u64 = u64::from_le_bytes(*b"},\"ie\":\0");
    if word(w, 0) != HEAD || word(w, 6) != ARG {
        return None;
    }
    let (a, n) = wide_id(w, 14)?;
    let p = 14 + n;
    if word(w, p) & 0xFFFF_FFFF_FFFF != FN {
        return None;
    }
    let (f, n) = wide_id(w, p + 6)?;
    let q = p + 6 + n;
    if word(w, q) & 0xFF_FFFF_FFFF_FFFF != IE {
        return None;
    }
    let (i, n) = wide_id(w, q + 7)?;
    let r = q + 7 + n;
    if w[r..r + 2] != *b"}\n" {
        return None;
    }
    Some((r + 2, i, f, a))
}

/// A cursor over one record's bytes for the fast path.
struct Cur<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Cur<'a> {
    /// The fixed literal `s` (its length known at compile time, so the comparison is inline).
    #[inline(always)]
    fn lit<const N: usize>(&mut self, s: &[u8; N]) -> Option<()> {
        let got: &[u8; N] = self.b.get(self.i..self.i + N)?.try_into().ok()?;
        if got == s {
            self.i += N;
            Some(())
        } else {
            None
        }
    }

    /// The literal `s` and then the newline that ends the record.
    #[inline(always)]
    fn close<const N: usize>(&mut self, s: &[u8; N]) -> Option<()> {
        self.lit(s)?;
        if self.b.get(self.i) == Some(&b'\n') {
            self.i += 1;
            Some(())
        } else {
            None
        }
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    /// A plain JSON integer of at most 19 digits (so it cannot overflow `u64`), with no leading
    /// zero unless it is `0`. Longer numbers take the general path.
    #[inline(always)]
    fn num(&mut self) -> Option<u64> {
        let start = self.i;
        let Some(chunk) = self.b.get(start..start + 8) else {
            return self.num_tail(start, 0);
        };
        let x = u64::from_le_bytes(chunk.try_into().unwrap()) ^ 0x3030_3030_3030_3030;
        let (n, v) = swar_digits(x);
        // The first byte, XOR '0', is zero exactly for a leading zero.
        if n < 8 {
            if n == 0 || (n > 1 && x & 0xFF == 0) {
                return None;
            }
            self.i = start + n as usize;
            return Some(v);
        }
        // Eight digits — every export id from 10^7 to 10^8 — end here unless a ninth follows.
        match self.b.get(start + 8) {
            Some(c) if !c.is_ascii_digit() => {
                if x & 0xFF == 0 {
                    return None;
                }
                self.i = start + 8;
                Some(v)
            }
            _ => self.num_long(start, v),
        }
    }

    /// A number of eight or more digits, its first eight already read as `v`. At most 19 digits
    /// are accepted, checked before each multiply, so the value never overflows.
    #[cold]
    fn num_long(&mut self, start: usize, mut v: u64) -> Option<u64> {
        if self.b[start] == b'0' {
            return None;
        }
        self.i = start + 8;
        loop {
            let Some(chunk) = self.b.get(self.i..self.i + 8) else {
                return self.num_tail(start, v);
            };
            let x = u64::from_le_bytes(chunk.try_into().unwrap()) ^ 0x3030_3030_3030_3030;
            let (n, digits) = swar_digits(x);
            if self.i - start + n as usize > 19 {
                return None;
            }
            v = v * POW10[n as usize] + digits;
            self.i += n as usize;
            if n < 8 {
                return Some(v);
            }
        }
    }

    /// The digits left when fewer than eight bytes of input remain, one at a time, onto the `v`
    /// read from `start` so far.
    #[cold]
    fn num_tail(&mut self, start: usize, mut v: u64) -> Option<u64> {
        while let Some(&d) = self.b.get(self.i) {
            let d = d.wrapping_sub(b'0');
            if d > 9 {
                break;
            }
            if self.i - start == 19 {
                return None;
            }
            v = v * 10 + d as u64;
            self.i += 1;
        }
        let len = self.i - start;
        (len > 0 && (len == 1 || self.b[start] != b'0')).then_some(v)
    }

    /// The remainder of a binder-info string (after its opening quote), one of Lean's four.
    #[inline]
    fn binder_info(&mut self) -> Option<()> {
        if self.lit(b"default\"").is_some()
            || self.lit(b"implicit\"").is_some()
            || self.lit(b"instImplicit\"").is_some()
            || self.lit(b"strictImplicit\"").is_some()
        {
            Some(())
        } else {
            None
        }
    }

    /// A string body up to its closing quote, only when it contains no escape or control byte.
    #[inline]
    fn plain_string(&mut self) -> Option<&'a [u8]> {
        let start = self.i;
        while let Some(&c) = self.b.get(self.i) {
            match c {
                b'"' => {
                    let s = &self.b[start..self.i];
                    self.i += 1;
                    return Some(s);
                }
                b'\\' => return None,
                c if c < 0x20 => return None,
                _ => self.i += 1,
            }
        }
        None
    }
}

fn bad(m: impl Into<String>) -> KernelFailure {
    KernelFailure::reject(format!("malformed export: {}", m.into()))
}

fn recycle<'b>(mut v: Vec<Tok<'_>>) -> Vec<Tok<'b>> {
    v.clear();
    v.into_iter().map(|_| unreachable!()).collect()
}

impl<'k> Reader<'k> {
    pub fn new(k: &'k mut Kernel) -> Reader<'k> {
        let mut names = IdMap::new();
        names.insert(0, NameId::ANON.0);
        let mut levels = IdMap::new();
        levels.insert(0, LevelId::ZERO.0);
        Reader {
            k,
            names,
            levels,
            exprs: IdMap::new(),
            pending_mutual: Vec::new(),
            refs: FxMap::default(),
            parse_only: false,
            stats: Stats {
                memory: Vec::new(),
                records: 0,
                declarations: 0,
                validated_primitives: Vec::new(),
            },
            meter: None,
            trust_before: 0,
            fast_paths: true,
            defer_leaves: false,
            defer_defns: false,
            deferred: Vec::new(),
        }
    }

    /// The expressions an export of `bytes` bytes holds, at most: one per 56 bytes, the densest of
    /// the Arena's large exports (56 to 61 bytes an expression). The persistent table is sized once
    /// for this count, so an estimate a quarter too high doubles the table for nothing.
    pub(crate) fn expected_expressions(bytes: u64) -> usize {
        (bytes / 56).min(u32::MAX as u64 / 2) as usize
    }

    /// Prepares the stores for an export of `bytes` bytes ([`Reader::expected_expressions`]).
    pub fn size_hint(&mut self, bytes: u64) {
        let nodes = Self::expected_expressions(bytes);
        self.k.s.reserve_persistent(nodes);
        self.exprs.dense.reserve(nodes);
    }

    /// Reads and checks a whole export. `Ok` means every declaration was admitted.
    pub fn run(&mut self, mut input: impl BufRead) -> KResult<()> {
        let mut buf: Vec<u8> = vec![0; 1 << 22];
        let (mut start, mut end, mut eof) = (0usize, 0usize, false);
        let mut toks: Vec<Tok<'static>> = Vec::new();
        let mut line_no = 0u64;
        let at_line = |mut e: KernelFailure, line_no: u64| {
            if e.detail.is_none() {
                e.detail = Some(format!("at export line {line_no}"));
            }
            e
        };
        loop {
            if self.fast_paths && start < end {
                let (used, read, r) = self.fast_records(&buf[start..end]);
                line_no += read;
                self.stats.records += read;
                start += used;
                r.map_err(|e| at_line(e, line_no + 1))?;
            }
            if start < end {
                let rest = &buf[start..end];
                let line_end = match rest.iter().position(|&b| b == b'\n') {
                    Some(nl) => Some(nl),
                    None if eof => Some(rest.len()),
                    None => None,
                };
                if let Some(len) = line_end {
                    line_no += 1;
                    let mut line = &buf[start..start + len];
                    while let [head @ .., b'\r'] = line {
                        line = head;
                    }
                    start = (start + len + 1).min(end);
                    if line.is_empty() {
                        continue;
                    }
                    let tape = match Tape::parse(line, recycle(toks)) {
                        Ok(t) => t,
                        Err(_) => return Err(bad(format!("line {line_no} is not valid JSON"))),
                    };
                    let r = self.record(&tape);
                    toks = recycle(tape.toks);
                    r.map_err(|e| at_line(e, line_no))?;
                    self.stats.records += 1;
                    continue;
                }
            } else if eof {
                break;
            }
            if start > 0 {
                buf.copy_within(start..end, 0);
                end -= start;
                start = 0;
            }
            if end == buf.len() {
                buf.resize(buf.len() * 2, 0);
            }
            let n = input
                .read(&mut buf[end..])
                .map_err(|e| KernelFailure::resource(format!("cannot read input: {e}")))?;
            if n == 0 {
                eof = true;
            } else {
                end += n;
            }
        }
        if let Some((all, _)) = self.pending_mutual.first() {
            let missing: Vec<String> = all.iter().map(|n| self.k.s.names.display(*n)).collect();
            return Err(bad(format!(
                "incomplete mutual block {{{}}}",
                missing.join(", ")
            )));
        }
        Ok(())
    }

    /// Reads records from the front of `buf` for as long as each is one of the layouts
    /// [`Reader::fast_record`] recognises, keeping the loop's state in registers. Returns the
    /// bytes consumed, the records read, and the failure of the record after them, if one failed;
    /// otherwise the record there (if any) is for the general parser, or not yet whole.
    fn fast_records(&mut self, buf: &[u8]) -> (usize, u64, KResult<()>) {
        let (mut at, mut read) = (0usize, 0u64);
        loop {
            let rest = &buf[at..];
            // Applications are three records in four: read them from a fixed window when one fits.
            if let Some(w) = rest.first_chunk::<WIDE>() {
                if let Some((used, i, f, a)) = wide_app(w) {
                    if let Err(e) = self.fast_app(i, f, a) {
                        return (at, read, Err(e));
                    }
                    at += used;
                    read += 1;
                    continue;
                }
            }
            match self.fast_record(rest) {
                Some((used, Ok(()))) => {
                    at += used;
                    read += 1;
                }
                Some((_, Err(e))) => return (at, read, Err(e)),
                None => return (at, read, Ok(())),
            }
        }
    }

    /// The common record layouts `lean4export` writes, recognised byte for byte in the input
    /// buffer, through the newline that ends the record, without building a token tape. Anything
    /// that is not exactly one of these layouts (another key order, spacing, an escaped string, a
    /// number with a sign or exponent, a record not yet wholly in the buffer) returns `None` and
    /// goes through the general parser, so this path accepts only what the general path would
    /// read identically. Returns the bytes consumed.
    ///
    /// [`Reader::fast_records`] reads runs of these; this is one record, from any window.
    fn fast_record(&mut self, line: &[u8]) -> Option<(usize, KResult<()>)> {
        let mut c = Cur { b: line, i: 0 };
        c.lit(b"{\"")?;
        match *line.get(2)? {
            b'a' => {
                c.lit(b"app\":{\"arg\":")?;
                let a = c.num()?;
                c.lit(b",\"fn\":")?;
                let f = c.num()?;
                c.lit(b"},\"ie\":")?;
                let i = c.num()?;
                c.close(b"}")?;
                Some((c.i, self.fast_app(i, f, a)))
            }
            b'f' => {
                c.lit(b"forallE\":{\"binderInfo\":\"")?;
                c.binder_info()?;
                c.lit(b",\"body\":")?;
                let b = c.num()?;
                c.lit(b",\"name\":")?;
                c.num()?;
                c.lit(b",\"type\":")?;
                let t = c.num()?;
                c.lit(b"},\"ie\":")?;
                let i = c.num()?;
                c.close(b"}")?;
                Some((c.i, self.fast_binder(i, t, b, true)))
            }
            b'b' => {
                c.lit(b"bvar\":")?;
                let k = c.num()?;
                c.lit(b",\"ie\":")?;
                let i = c.num()?;
                c.close(b"}")?;
                let k = u32::try_from(k).ok()?;
                let e = self.k.s.bvar(k);
                Some((c.i, self.put_expr(i, e)))
            }
            b'c' => {
                c.lit(b"const\":{\"name\":")?;
                let n = c.num()?;
                c.lit(b",\"us\":[")?;
                let mut us: smallvec::SmallVec<[u64; 4]> = smallvec::SmallVec::new();
                if c.peek() != Some(b']') {
                    us.push(c.num()?);
                    while c.peek() == Some(b',') {
                        c.i += 1;
                        us.push(c.num()?);
                    }
                }
                c.lit(b"]},\"ie\":")?;
                let i = c.num()?;
                c.close(b"}")?;
                Some((c.i, self.fast_const(i, n, &us)))
            }
            b'i' => {
                c.lit(b"i")?;
                match c.peek()? {
                    b'e' => {
                        c.lit(b"e\":")?;
                        let i = c.num()?;
                        c.lit(b",\"")?;
                        match c.peek()? {
                            b'l' => {
                                if c.lit(b"lam\":{\"binderInfo\":\"").is_some() {
                                    c.binder_info()?;
                                    c.lit(b",\"body\":")?;
                                    let b = c.num()?;
                                    c.lit(b",\"name\":")?;
                                    c.num()?;
                                    c.lit(b",\"type\":")?;
                                    let t = c.num()?;
                                    c.close(b"}}")?;
                                    Some((c.i, self.fast_binder(i, t, b, false)))
                                } else {
                                    None
                                }
                            }
                            b's' => {
                                c.lit(b"sort\":")?;
                                let l = c.num()?;
                                c.close(b"}")?;
                                Some((c.i, self.fast_sort(i, l)))
                            }
                            _ => None,
                        }
                    }
                    b'n' => {
                        c.lit(b"n\":")?;
                        let i = c.num()?;
                        c.lit(b",\"")?;
                        match c.peek()? {
                            b's' => {
                                c.lit(b"str\":{\"pre\":")?;
                                let p = c.num()?;
                                c.lit(b",\"str\":\"")?;
                                let text = c.plain_string()?;
                                c.close(b"}}")?;
                                let text = std::str::from_utf8(text).ok()?;
                                Some((c.i, self.fast_name(i, p, NamePart::Str(text))))
                            }
                            b'n' => {
                                c.lit(b"num\":{\"i\":")?;
                                let k = c.num()?;
                                c.lit(b",\"pre\":")?;
                                let p = c.num()?;
                                c.close(b"}}")?;
                                Some((c.i, self.fast_name(i, p, NamePart::Num(k))))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn put_expr(&mut self, idx: u64, e: ExprId) -> KResult<()> {
        if !self.exprs.insert(idx, e.0) {
            return Err(bad(format!("expression #{idx} is defined twice")));
        }
        Ok(())
    }

    fn expr_ix(&self, idx: u64) -> KResult<ExprId> {
        self.exprs
            .get(idx)
            .map(ExprId)
            .ok_or_else(|| bad(format!("undefined expression #{idx}")))
    }

    fn level_ix(&self, idx: u64) -> KResult<LevelId> {
        self.levels
            .get(idx)
            .map(LevelId)
            .ok_or_else(|| bad(format!("undefined level #{idx}")))
    }

    fn name_ix(&self, idx: u64) -> KResult<NameId> {
        self.names
            .get(idx)
            .map(NameId)
            .ok_or_else(|| bad(format!("undefined name #{idx}")))
    }

    fn fast_app(&mut self, i: u64, f: u64, a: u64) -> KResult<()> {
        let (f, a) = (self.expr_ix(f)?, self.expr_ix(a)?);
        let e = self.k.s.app(f, a);
        self.put_expr(i, e)
    }

    fn fast_binder(&mut self, i: u64, t: u64, b: u64, pi: bool) -> KResult<()> {
        let (t, b) = (self.expr_ix(t)?, self.expr_ix(b)?);
        let e = if pi {
            self.k.s.pi(t, b)
        } else {
            self.k.s.lam(t, b)
        };
        self.put_expr(i, e)
    }

    fn fast_sort(&mut self, i: u64, l: u64) -> KResult<()> {
        let l = self.level_ix(l)?;
        let e = self.k.s.sort(l);
        self.put_expr(i, e)
    }

    fn fast_const(&mut self, i: u64, n: u64, us: &[u64]) -> KResult<()> {
        let n = self.name_ix(n)?;
        let us: KResult<smallvec::SmallVec<[LevelId; 4]>> =
            us.iter().map(|u| self.level_ix(*u)).collect();
        let e = self.k.s.cnst_levels(n, &us?);
        self.put_expr(i, e)
    }

    fn fast_name(&mut self, i: u64, p: u64, part: NamePart<'_>) -> KResult<()> {
        let pre = self.name_ix(p)?;
        let id = match part {
            NamePart::Str(s) => self.k.s.names.str(pre, s),
            NamePart::Num(k) => self.k.s.names.num(pre, k),
        };
        if i == 0 || !self.names.insert(i, id.0) {
            return Err(bad(format!("name #{i} is defined twice")));
        }
        Ok(())
    }

    fn name(&self, t: &Tape, i: usize) -> KResult<NameId> {
        let idx = t.u64(i).ok_or_else(|| bad("name index expected"))?;
        self.names
            .get(idx)
            .map(NameId)
            .ok_or_else(|| bad(format!("undefined name #{idx}")))
    }

    fn level(&self, t: &Tape, i: usize) -> KResult<LevelId> {
        let idx = t.u64(i).ok_or_else(|| bad("level index expected"))?;
        self.levels
            .get(idx)
            .map(LevelId)
            .ok_or_else(|| bad(format!("undefined level #{idx}")))
    }

    fn expr(&self, t: &Tape, i: usize) -> KResult<ExprId> {
        let idx = t.u64(i).ok_or_else(|| bad("expression index expected"))?;
        self.exprs
            .get(idx)
            .map(ExprId)
            .ok_or_else(|| bad(format!("undefined expression #{idx}")))
    }

    fn field(&self, t: &Tape, obj: usize, key: &str) -> KResult<usize> {
        t.get(obj, key)
            .ok_or_else(|| bad(format!("missing field '{key}'")))
    }

    fn name_field(&self, t: &Tape, obj: usize, key: &str) -> KResult<NameId> {
        let i = self.field(t, obj, key)?;
        self.name(t, i)
    }

    fn expr_field(&self, t: &Tape, obj: usize, key: &str) -> KResult<ExprId> {
        let i = self.field(t, obj, key)?;
        self.expr(t, i)
    }

    fn u64_field(&self, t: &Tape, obj: usize, key: &str) -> KResult<u64> {
        let i = self.field(t, obj, key)?;
        t.u64(i)
            .ok_or_else(|| bad(format!("field '{key}' is not a number")))
    }

    fn u32_field(&self, t: &Tape, obj: usize, key: &str) -> KResult<u32> {
        let v = self.u64_field(t, obj, key)?;
        u32::try_from(v).map_err(|_| bad(format!("field '{key}' is out of range")))
    }

    fn bool_field(&self, t: &Tape, obj: usize, key: &str) -> KResult<bool> {
        let i = self.field(t, obj, key)?;
        t.bool(i)
            .ok_or_else(|| bad(format!("field '{key}' is not a boolean")))
    }

    fn names_field(&self, t: &Tape, obj: usize, key: &str) -> KResult<Rc<[NameId]>> {
        let i = self.field(t, obj, key)?;
        let v: KResult<Vec<NameId>> = t.elems(i).map(|e| self.name(t, e)).collect();
        Ok(v?.into())
    }

    fn record(&mut self, t: &Tape) -> KResult<()> {
        if !matches!(t.toks.first(), Some(Tok::Obj { .. })) {
            return Err(bad("record is not an object"));
        }
        let mut id_key: Option<(&[u8], u64)> = None;
        let mut kind: Option<(&[u8], usize)> = None;
        for (k, v) in t.members(0) {
            match k {
                b"in" | b"il" | b"ie" => {
                    id_key = Some((
                        k,
                        t.u64(v)
                            .ok_or_else(|| bad("record index is not a number"))?,
                    ));
                }
                _ => {
                    if kind.is_some() {
                        return Err(bad("record has more than one payload"));
                    }
                    kind = Some((k, v));
                }
            }
        }
        let Some((kind, v)) = kind else {
            return Err(bad("record has no payload"));
        };
        match id_key {
            Some((b"in", idx)) => self.name_record(t, idx, kind, v),
            Some((b"il", idx)) => self.level_record(t, idx, kind, v),
            Some((b"ie", idx)) => self.expr_record(t, idx, kind, v),
            Some(_) => unreachable!(),
            None => self.decl_record(t, kind, v),
        }
    }

    fn name_record(&mut self, t: &Tape, idx: u64, kind: &[u8], v: usize) -> KResult<()> {
        let pre = self.name_field(t, v, "pre")?;
        let id = match kind {
            b"str" => {
                let si = self.field(t, v, "str")?;
                let s = t
                    .str(si)
                    .ok_or_else(|| bad("name component is not a string"))?;
                self.k.s.names.str(pre, &s)
            }
            b"num" => {
                let n = self.u64_field(t, v, "i")?;
                self.k.s.names.num(pre, n)
            }
            _ => return Err(bad("unknown name record")),
        };
        if idx == 0 || !self.names.insert(idx, id.0) {
            return Err(bad(format!("name #{idx} is defined twice")));
        }
        Ok(())
    }

    fn level_record(&mut self, t: &Tape, idx: u64, kind: &[u8], v: usize) -> KResult<()> {
        let pair = |r: &Reader| -> KResult<(LevelId, LevelId)> {
            let mut it = t.elems(v);
            let a = it.next().ok_or_else(|| bad("level pair expected"))?;
            let b = it.next().ok_or_else(|| bad("level pair expected"))?;
            if it.next().is_some() {
                return Err(bad("level pair expected"));
            }
            Ok((r.level(t, a)?, r.level(t, b)?))
        };
        let node = match kind {
            b"succ" => LevelNode::Succ(self.level(t, v)?),
            b"max" => {
                let (a, b) = pair(self)?;
                LevelNode::Max(a, b)
            }
            b"imax" => {
                let (a, b) = pair(self)?;
                LevelNode::IMax(a, b)
            }
            b"param" => LevelNode::Param(self.name(t, v)?),
            _ => return Err(bad("unknown level record")),
        };
        let id = self.k.s.levels.intern(node);
        if idx == 0 || !self.levels.insert(idx, id.0) {
            return Err(bad(format!("level #{idx} is defined twice")));
        }
        Ok(())
    }

    fn expr_record(&mut self, t: &Tape, idx: u64, kind: &[u8], v: usize) -> KResult<()> {
        let s = &mut self.k.s;
        let e = match kind {
            b"bvar" => {
                let i = t.u64(v).ok_or_else(|| bad("bvar index"))?;
                s.bvar(u32::try_from(i).map_err(|_| bad("bvar index out of range"))?)
            }
            b"sort" => {
                let l = self.level(t, v)?;
                self.k.s.sort(l)
            }
            b"const" => {
                let n = self.name_field(t, v, "name")?;
                let us_i = self.field(t, v, "us")?;
                let us: KResult<Vec<LevelId>> = t.elems(us_i).map(|x| self.level(t, x)).collect();
                let us = us?;
                self.k.s.cnst_levels(n, &us)
            }
            b"app" => {
                let f = self.expr_field(t, v, "fn")?;
                let a = self.expr_field(t, v, "arg")?;
                self.k.s.app(f, a)
            }
            b"lam" | b"forallE" => {
                let ty = self.expr_field(t, v, "type")?;
                let body = self.expr_field(t, v, "body")?;
                if kind == b"lam" {
                    self.k.s.lam(ty, body)
                } else {
                    self.k.s.pi(ty, body)
                }
            }
            b"letE" => {
                let ty = self.expr_field(t, v, "type")?;
                let val = self.expr_field(t, v, "value")?;
                let body = self.expr_field(t, v, "body")?;
                self.k.s.let_(ty, val, body)
            }
            b"proj" => {
                let sn = self.name_field(t, v, "typeName")?;
                let i = self.u32_field(t, v, "idx")?;
                let x = self.expr_field(t, v, "struct")?;
                self.k.s.proj(sn, i, x)
            }
            b"natVal" => {
                let digits = t.str(v).ok_or_else(|| bad("natVal is not a string"))?;
                let n = Nat::parse_decimal(&digits)
                    .ok_or_else(|| bad("natVal is not a natural number"))?;
                self.k.s.nat_lit(n)
            }
            b"strVal" => {
                let st = t.str(v).ok_or_else(|| bad("strVal is not a string"))?;
                self.k.s.str_lit(&st)
            }
            b"mdata" => self.expr_field(t, v, "expr")?,
            _ => {
                return Err(KernelFailure::resource(format!(
                    "unsupported expression record '{}'",
                    String::from_utf8_lossy(kind)
                )))
            }
        };
        if !self.exprs.insert(idx, e.0) {
            return Err(bad(format!("expression #{idx} is defined twice")));
        }
        Ok(())
    }

    fn header(&self, t: &Tape, v: usize) -> KResult<Header> {
        Ok(Header {
            name: self.name_field(t, v, "name")?,
            lparams: self.names_field(t, v, "levelParams")?,
            ty: self.expr_field(t, v, "type")?,
        })
    }

    fn decl_record(&mut self, t: &Tape, kind: &[u8], v: usize) -> KResult<()> {
        if self.parse_only && kind != b"meta" && kind != b"ref" {
            return Ok(());
        }
        match kind {
            b"meta" => {
                let fmt = t
                    .get(v, "format")
                    .and_then(|f| t.get(f, "version"))
                    .and_then(|x| t.str(x));
                match fmt {
                    Some(ver) if ver.starts_with("3.") => Ok(()),
                    _ => Err(KernelFailure::resource("unsupported export format version")),
                }
            }
            b"axiom" => {
                let header = self.header(t, v)?;
                let is_unsafe = self.bool_field(t, v, "isUnsafe")?;
                self.decl(Decl::Axiom { header, is_unsafe })
            }
            b"thm" => {
                let header = self.header(t, v)?;
                let value = self.expr_field(t, v, "value")?;
                self.decl(Decl::Thm { header, value })
            }
            b"opaque" => {
                let header = self.header(t, v)?;
                let value = self.expr_field(t, v, "value")?;
                let is_unsafe = self.bool_field(t, v, "isUnsafe")?;
                self.decl(Decl::Opaque {
                    header,
                    value,
                    is_unsafe,
                })
            }
            b"def" => {
                let header = self.header(t, v)?;
                let value = self.expr_field(t, v, "value")?;
                let hi = self.field(t, v, "hints")?;
                let hint = match t.str(hi).as_deref() {
                    Some("abbrev") => Hint::Abbrev,
                    Some("opaque") => Hint::Opaque,
                    Some(_) => return Err(bad("unknown reducibility hint")),
                    None => {
                        let r = t
                            .get(hi, "regular")
                            .ok_or_else(|| bad("unknown reducibility hint"))?;
                        Hint::Regular(
                            t.u64(r)
                                .and_then(|x| u32::try_from(x).ok())
                                .ok_or_else(|| bad("height"))?,
                        )
                    }
                };
                let si = self.field(t, v, "safety")?;
                let safety = match t.str(si).as_deref() {
                    Some("safe") => Safety::Safe,
                    Some("unsafe") => Safety::Unsafe,
                    Some("partial") => Safety::Partial,
                    _ => return Err(bad("unknown safety")),
                };
                let all = self.names_field(t, v, "all")?;
                let d = DefnDecl {
                    header,
                    value,
                    hint,
                    safety,
                };
                if safety != Safety::Safe && all.len() > 1 {
                    self.pending_mutual.push((all.clone(), d));
                    self.flush_mutual(&all)
                } else {
                    self.decl(Decl::Defn(d))
                }
            }
            b"quot" => {
                let header = self.header(t, v)?;
                let ki = self.field(t, v, "kind")?;
                let kind = match t.str(ki).as_deref() {
                    Some("type") => QuotKind::Type,
                    Some("ctor") => QuotKind::Ctor,
                    Some("lift") => QuotKind::Lift,
                    Some("ind") => QuotKind::Ind,
                    _ => return Err(bad("unknown quotient kind")),
                };
                self.decl(Decl::Quot { header, kind })
            }
            b"inductive" => self.inductive_record(t, v),
            b"ref" => {
                let key = self.field(t, v, "key")?;
                let key = t
                    .str(key)
                    .ok_or_else(|| bad("ref key is not a string"))?
                    .into_owned();
                let value = self.expr_field(t, v, "value")?;
                self.refs.insert(key, value);
                Ok(())
            }
            _ => Err(KernelFailure::resource(format!(
                "unsupported declaration record '{}'",
                String::from_utf8_lossy(kind)
            ))),
        }
    }

    fn decl(&mut self, d: Decl) -> KResult<()> {
        let index = self.stats.declarations;
        self.stats.declarations += 1;
        if index < self.trust_before && self.k.may_trust(&d) {
            self.k.admit_trusted(d);
            return Ok(());
        }
        // A leaf (theorem, opaque, axiom) is never unfolded by a later check — a theorem's proof is
        // irrelevant, an opaque is opaque, an axiom has no body — so admitting it unchecked and
        // re-checking it over the finished environment is a sequential replay. Only its type, which
        // the re-check verifies, is ever referenced meanwhile.
        // A definition's body IS unfolded later, but `admit` records the same env entry a checked
        // definition would, so a worker's re-check (which an invalid definition never survives) is
        // still a sequential replay. Only non-primitive definitions, never a quotient or an env-
        // mutating kind: `may_trust` is exactly that predicate.
        let deferrable = matches!(d, Decl::Thm { .. } | Decl::Opaque { .. } | Decl::Axiom { .. })
            || (self.defer_defns && matches!(d, Decl::Defn { .. }) && self.k.may_trust(&d));
        if self.defer_leaves && deferrable {
            self.k.admit_trusted(d.clone());
            self.deferred.push(d);
            return Ok(());
        }
        if self.meter.is_none() {
            return self.k.add_decl(d);
        }
        let (name, kind) = (d.name(), d.kind_name());
        self.metered(name, kind, |k| k.add_decl(d))
    }

    /// `check` on the kernel, reported to the meter.
    fn metered(
        &mut self,
        name: NameId,
        kind: &'static str,
        check: impl FnOnce(&mut Kernel) -> KResult<()>,
    ) -> KResult<()> {
        if let Some(m) = self.meter.as_mut() {
            m.begin();
        }
        let r = check(self.k);
        if let Some(m) = self.meter.as_mut() {
            m.end(&self.k.s.names.display(name), kind, r.is_ok());
        }
        r
    }

    fn flush_mutual(&mut self, all: &Rc<[NameId]>) -> KResult<()> {
        let have: Vec<NameId> = self
            .pending_mutual
            .iter()
            .filter(|(a, _)| a == all)
            .map(|(_, d)| d.header.name)
            .collect();
        if !all.iter().all(|n| have.contains(n)) {
            return Ok(());
        }
        let mut block = Vec::new();
        for n in all.iter() {
            let pos = self
                .pending_mutual
                .iter()
                .position(|(a, d)| a == all && d.header.name == *n)
                .unwrap();
            block.push(self.pending_mutual.remove(pos).1);
        }
        self.decl(Decl::Mutual(block))
    }

    fn inductive_record(&mut self, t: &Tape, v: usize) -> KResult<()> {
        let types_i = self.field(t, v, "types")?;
        let ctors_i = self.field(t, v, "ctors")?;
        let recs_i = self.field(t, v, "recs")?;
        let mut claimed = Vec::new();
        let mut ctor_types: FxMap<NameId, ExprId> = FxMap::default();
        for c in t.elems(ctors_i) {
            let h = self.header(t, c)?;
            ctor_types.insert(h.name, h.ty);
            claimed.push(Const {
                name: h.name,
                lparams: h.lparams,
                ty: h.ty,
                kind: Kind::Ctor(CtorInfo {
                    induct: self.name_field(t, c, "induct")?,
                    cidx: self.u32_field(t, c, "cidx")?,
                    num_params: self.u32_field(t, c, "numParams")?,
                    num_fields: self.u32_field(t, c, "numFields")?,
                    is_unsafe: self.bool_field(t, c, "isUnsafe")?,
                }),
            });
        }
        let mut types = Vec::new();
        let mut first: Option<(Rc<[NameId]>, u32, bool)> = None;
        for ti in t.elems(types_i) {
            let h = self.header(t, ti)?;
            let ctors = self.names_field(t, ti, "ctors")?;
            let num_params = self.u32_field(t, ti, "numParams")?;
            let is_unsafe = self.bool_field(t, ti, "isUnsafe")?;
            if first.is_none() {
                first = Some((h.lparams.clone(), num_params, is_unsafe));
            }
            let mut cs = Vec::new();
            for c in ctors.iter() {
                let ty = *ctor_types.get(c).ok_or_else(|| {
                    KernelFailure::reject(format!(
                        "inductive '{}' lists constructor '{}' but the block does not declare it",
                        self.k.s.names.display(h.name),
                        self.k.s.names.display(*c)
                    ))
                })?;
                cs.push((*c, ty));
            }
            types.push(IndType {
                name: h.name,
                ty: h.ty,
                ctors: cs,
            });
            claimed.push(Const {
                name: h.name,
                lparams: h.lparams,
                ty: h.ty,
                kind: Kind::Induct(InductiveInfo {
                    num_params,
                    num_indices: self.u32_field(t, ti, "numIndices")?,
                    all: self.names_field(t, ti, "all")?,
                    ctors,
                    num_nested: self.u32_field(t, ti, "numNested")?,
                    is_rec: self.bool_field(t, ti, "isRec")?,
                    is_reflexive: self.bool_field(t, ti, "isReflexive")?,
                    is_unsafe,
                    rec: self.k.rec_name(h.name),
                }),
            });
        }
        for ri in t.elems(recs_i) {
            let h = self.header(t, ri)?;
            let rules_i = self.field(t, ri, "rules")?;
            let mut rules = Vec::new();
            for r in t.elems(rules_i) {
                rules.push(RecRule {
                    ctor: self.name_field(t, r, "ctor")?,
                    nfields: self.u32_field(t, r, "nfields")?,
                    rhs: self.expr_field(t, r, "rhs")?,
                });
            }
            let mut info = RecInfo {
                all: self.names_field(t, ri, "all")?,
                num_params: self.u32_field(t, ri, "numParams")?,
                num_indices: self.u32_field(t, ri, "numIndices")?,
                num_motives: self.u32_field(t, ri, "numMotives")?,
                num_minors: self.u32_field(t, ri, "numMinors")?,
                rules: rules.into(),
                k: self.bool_field(t, ri, "k")?,
                is_unsafe: self.bool_field(t, ri, "isUnsafe")?,
                major_induct: NameId::ANON,
            };
            info.major_induct = self.major_induct(h.ty, info.major_idx());
            claimed.push(Const {
                name: h.name,
                lparams: h.lparams,
                ty: h.ty,
                kind: Kind::Rec(info),
            });
        }
        let Some((lparams, nparams, is_unsafe)) = first else {
            return Err(KernelFailure::reject("inductive block declares no types"));
        };
        for c in &claimed {
            if self.k.env.contains(c.name) {
                return Err(KernelFailure::reject(format!(
                    "'{}' has already been declared",
                    self.k.s.names.display(c.name)
                )));
            }
        }
        let index = self.stats.declarations;
        self.stats.declarations += 1;
        if index < self.trust_before && self.k.may_trust_block(&claimed) {
            self.k.admit_trusted_block(claimed);
            return Ok(());
        }
        let name = types.first().map_or(NameId::ANON, |t| t.name);
        let decl = InductiveDecl {
            lparams,
            nparams,
            types,
            is_unsafe,
        };
        if self.meter.is_none() {
            return self.k.add_inductive(decl, claimed);
        }
        self.metered(name, "inductive", |k| k.add_inductive(decl, claimed))
    }

    fn major_induct(&self, mut ty: ExprId, idx: usize) -> NameId {
        for _ in 0..idx {
            match self.k.s.node(ty) {
                Node::Pi(_, b) => ty = b,
                _ => return NameId::ANON,
            }
        }
        match self.k.s.node(ty) {
            Node::Pi(d, _) => self.k.s.const_head(d).map_or(NameId::ANON, |(n, _)| n),
            _ => NameId::ANON,
        }
    }
}

/// The verdict of checking one export, in the Arena's exit-code protocol.
pub enum Verdict {
    Accept,
    Reject(KernelFailure),
    Decline(KernelFailure),
}

impl Verdict {
    pub fn exit_code(&self) -> i32 {
        match self {
            Verdict::Accept => 0,
            Verdict::Reject(_) => 1,
            Verdict::Decline(_) => 2,
        }
    }
}

/// Checks a whole export stream.
pub fn check_export(input: impl BufRead) -> (Verdict, Stats) {
    check_export_with(input, |_| {})
}

/// Checks a whole export with a kernel configured by `configure` first.
pub fn check_export_with(
    input: impl BufRead,
    configure: impl FnOnce(&mut Kernel),
) -> (Verdict, Stats) {
    check_export_sized(input, None, configure)
}

/// [`check_export_with`] for an input whose size is known, so the stores are sized once.
pub fn check_export_sized(
    input: impl BufRead,
    bytes: Option<u64>,
    configure: impl FnOnce(&mut Kernel),
) -> (Verdict, Stats) {
    check_export_observed(input, bytes, configure, Observe::default())
}

/// [`check_export_sized`], observed as `observe` asks.
pub fn check_export_observed(
    input: impl BufRead,
    bytes: Option<u64>,
    configure: impl FnOnce(&mut Kernel),
    observe: Observe,
) -> (Verdict, Stats) {
    let mut k = Kernel::new();
    configure(&mut k);
    check_export_in(&mut k, input, bytes, observe)
}

/// Checks a whole export into the caller's kernel, which keeps everything admitted — a command
/// that exits once it has reported need never pay for tearing the kernel down.
pub fn check_export_in(
    k: &mut Kernel,
    input: impl BufRead,
    bytes: Option<u64>,
    observe: Observe,
) -> (Verdict, Stats) {
    let mut r = Reader::new(k);
    r.meter = observe.meter;
    r.trust_before = observe.trust_before;
    if let Some(b) = bytes {
        r.size_hint(b);
    }
    let outcome = r.run(input);
    let v = if observe.trust_before > 0 {
        let prefix = format!(
            "trusted-prefix run: the declarations before #{} were admitted unchecked",
            observe.trust_before
        );
        Verdict::Decline(match outcome {
            Ok(()) => {
                KernelFailure::resource(format!("{prefix}; every checked declaration passed"))
            }
            Err(e) => {
                KernelFailure::resource(format!("{prefix}; a checked declaration failed: {e}"))
            }
        })
    } else {
        match outcome {
            Ok(()) => Verdict::Accept,
            Err(e) if e.kind == FailKind::Reject => Verdict::Reject(e),
            Err(e) => Verdict::Decline(e),
        }
    };
    let mut stats = r.stats;
    let mut prims: Vec<String> = k
        .validated_primitives
        .iter()
        .map(|n| k.s.names.display(*n))
        .collect();
    prims.sort();
    stats.validated_primitives = prims;
    stats.memory = k.memory_report();
    (v, stats)
}

/// Check an export the arena's way: build the whole environment, admitting leaf declarations
/// (theorems, opaques, axioms) unchecked, then re-check those leaves over the finished environment.
/// The verdict is identical to [`check_export`] — a sequential replay — but the re-checks are
/// independent, which is what the arena entry spreads across cores (THREADS > 1). Here they run on
/// one worker, in order.
pub fn check_export_deferred(input: impl BufRead) -> (Verdict, Stats) {
    let mut k = Kernel::new();
    check_export_deferred_in(&mut k, input, None, Observe::default())
}

/// Build the whole environment from `input`, admitting leaf declarations (theorems, opaques, axioms)
/// unchecked and returning them to be re-checked, with the build outcome and stats. The environment
/// is left in `k`. The arena entry re-checks the returned leaves across forked children; the serial
/// path ([`check_export_deferred_in`]) re-checks them here. A non-leaf failure stops the build and is
/// the returned `Err`.
pub fn check_export_build_only(
    k: &mut Kernel,
    input: impl BufRead,
    bytes: Option<u64>,
    observe: Observe,
) -> (KResult<()>, Vec<Decl>, Stats) {
    let mut r = Reader::new(k);
    r.meter = observe.meter;
    r.trust_before = observe.trust_before;
    r.defer_leaves = true;
    r.defer_defns = std::env::var_os("LOGOS_DEFER_DEFNS").is_some();
    if let Some(b) = bytes {
        r.size_hint(b);
    }
    let built = r.run(input);
    let deferred = std::mem::take(&mut r.deferred);
    let mut stats = r.stats;
    let mut prims: Vec<String> = k.validated_primitives.iter().map(|n| k.s.names.display(*n)).collect();
    prims.sort();
    stats.validated_primitives = prims;
    stats.memory = k.memory_report();
    (built, deferred, stats)
}

/// [`check_export_deferred`] against a caller's kernel and size hint.
pub fn check_export_deferred_in(
    k: &mut Kernel,
    input: impl BufRead,
    bytes: Option<u64>,
    observe: Observe,
) -> (Verdict, Stats) {
    let (built, deferred, stats) = check_export_build_only(k, input, bytes, observe);
    let outcome = match built {
        Ok(()) => {
            let mut res = Ok(());
            for d in &deferred {
                if let Err(e) = k.recheck_decl(d) {
                    res = Err(e);
                    break;
                }
            }
            res
        }
        Err(e) => Err(e),
    };
    let v = match outcome {
        Ok(()) => Verdict::Accept,
        Err(e) if e.kind == FailKind::Reject => Verdict::Reject(e),
        Err(e) => Verdict::Decline(e),
    };
    (v, stats)
}

/// Loads a file of named reference terms into `k`'s store.
pub fn load_refs(k: &mut Kernel, input: impl BufRead) -> KResult<FxMap<String, ExprId>> {
    let mut r = Reader::new(k);
    r.run(input)?;
    Ok(std::mem::take(&mut r.refs))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/export/number_tests.rs"]
mod number_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/export/size_controls.rs"]
mod size_controls;
