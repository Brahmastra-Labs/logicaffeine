//! A Hugging Face `tokenizer.json` byte-pair tokenizer, run as the `tokenizers` library's `encode` runs it: the added
//! tokens the normalizer leaves alone split out of the raw text, the normalizer over what remains, the normalized added
//! tokens split out of that, the pre-tokenizer's splits, the byte-pair model over each split, and the post-processor's
//! special tokens around the whole. Patterns run on [`onig`](super::onig) and the normal forms on the generated Unicode
//! data, as the library runs them on Oniguruma and the Unicode Character Database.
//!
//! A component the port does not reproduce is refused by name when the tokenizer is read, never approximated.

use std::collections::{BinaryHeap, HashMap};
use std::io;
use std::path::Path;

use serde_json::Value;

use super::invalid;
use super::onig::{self, Regex};
use super::unicode_data::{CANONICAL_DECOMPOSITIONS, COMBINING_CLASSES, PRIMARY_COMPOSITES};

/// What a pattern matches in a text: a literal, a regular expression, or one codepoint by a predicate.
enum Pattern {
    Text(Vec<char>),
    Re(Regex),
    Char(fn(char) -> bool),
}

/// Covering segments of a text, `(start, end, is_match)` in codepoint offsets: `find_matches` of `tokenizers`.
fn segments(pattern: &Pattern, s: &[char]) -> Vec<(usize, usize, bool)> {
    match pattern {
        Pattern::Text(t) if t.is_empty() => vec![(0, s.len(), false)],
        Pattern::Text(t) => {
            let mut out = Vec::new();
            let (mut prev, mut i) = (0, 0);
            while i + t.len() <= s.len() {
                if s[i..i + t.len()] == t[..] {
                    if prev != i {
                        out.push((prev, i, false));
                    }
                    out.push((i, i + t.len(), true));
                    i += t.len();
                    prev = i;
                } else {
                    i += 1;
                }
            }
            if prev != s.len() || s.is_empty() {
                out.push((prev, s.len(), false));
            }
            out
        }
        Pattern::Re(re) => {
            if s.is_empty() {
                return vec![(0, 0, false)];
            }
            let cps: Vec<u32> = s.iter().map(|&c| c as u32).collect();
            let mut out = Vec::new();
            let mut prev = 0;
            for (start, end) in re.find_iter(&cps) {
                if prev != start {
                    out.push((prev, start, false));
                }
                out.push((start, end, true));
                prev = end;
            }
            if prev != s.len() {
                out.push((prev, s.len(), false));
            }
            out
        }
        Pattern::Char(f) => {
            if s.is_empty() {
                return vec![(0, 0, false)];
            }
            let mut out = Vec::new();
            let mut last = 0;
            for (i, &c) in s.iter().enumerate() {
                if f(c) {
                    if last < i {
                        out.push((last, i, false));
                    }
                    out.push((i, i + 1, true));
                    last = i + 1;
                }
            }
            if s.len() > last {
                out.push((last, s.len(), false));
            }
            out
        }
    }
}

/// How a split treats the pieces a pattern matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Behavior {
    Removed,
    Isolated,
    MergedWithPrevious,
    MergedWithNext,
    Contiguous,
}

impl Behavior {
    fn of(v: &Value) -> io::Result<Behavior> {
        Ok(match v.as_str() {
            Some("Removed") => Behavior::Removed,
            Some("Isolated") => Behavior::Isolated,
            Some("MergedWithPrevious") => Behavior::MergedWithPrevious,
            Some("MergedWithNext") => Behavior::MergedWithNext,
            Some("Contiguous") => Behavior::Contiguous,
            other => return Err(invalid(format!("tokenizer.json: split behavior {other:?}"))),
        })
    }
}

/// `NormalizedString::split`: the pieces of `s` the pattern and behavior leave, empty ones dropped.
fn split(s: &str, pattern: &Pattern, behavior: Behavior, invert: bool) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let segs: Vec<(usize, usize, bool)> =
        segments(pattern, &chars).into_iter().map(|(a, b, m)| (a, b, m != invert)).collect();
    // `(start, end, removed)`.
    let pieces: Vec<(usize, usize, bool)> = match behavior {
        Behavior::Isolated => segs.iter().map(|&(a, b, _)| (a, b, false)).collect(),
        Behavior::Removed => segs.clone(),
        Behavior::Contiguous => {
            let mut acc: Vec<(usize, usize, bool)> = Vec::new();
            let mut previous = false;
            for &(a, b, m) in &segs {
                match acc.last_mut() {
                    Some(last) if m == previous => last.1 = b,
                    _ => acc.push((a, b, false)),
                }
                previous = m;
            }
            acc
        }
        Behavior::MergedWithPrevious => {
            let mut acc: Vec<(usize, usize, bool)> = Vec::new();
            let mut previous = false;
            for &(a, b, m) in &segs {
                match acc.last_mut() {
                    Some(last) if m && !previous => last.1 = b,
                    _ => acc.push((a, b, false)),
                }
                previous = m;
            }
            acc
        }
        Behavior::MergedWithNext => {
            let mut acc: Vec<(usize, usize, bool)> = Vec::new();
            let mut previous = false;
            for &(a, b, m) in segs.iter().rev() {
                match acc.last_mut() {
                    Some(last) if m && !previous => last.0 = a,
                    _ => acc.push((a, b, false)),
                }
                previous = m;
            }
            acc.reverse();
            acc
        }
    };
    pieces
        .into_iter()
        .filter(|&(a, b, removed)| !removed && a != b)
        .map(|(a, b, _)| chars[a..b].iter().collect())
        .collect()
}

/// `replace` of a normalized string: every match of the pattern replaced by `content`.
fn replace(s: &str, pattern: &Pattern, content: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (a, b, m) in segments(pattern, &chars) {
        if m {
            out.push_str(content);
        } else {
            out.extend(&chars[a..b]);
        }
    }
    out
}

fn pattern_of(v: &Value) -> io::Result<Pattern> {
    if let Some(s) = v.get("String").and_then(Value::as_str) {
        return Ok(Pattern::Text(s.chars().collect()));
    }
    if let Some(r) = v.get("Regex").and_then(Value::as_str) {
        return Ok(Pattern::Re(Regex::new(r).map_err(invalid)?));
    }
    Err(invalid(format!("tokenizer.json: a pattern {v}")))
}

fn ccc(c: u32) -> u8 {
    let at = COMBINING_CLASSES.partition_point(|&(first, _, _)| first <= c);
    match at.checked_sub(1).map(|i| COMBINING_CLASSES[i]) {
        Some((first, last, class)) if (first..=last).contains(&c) => class,
        _ => 0,
    }
}

const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

fn decompose_into(c: u32, out: &mut Vec<u32>) {
    if (S_BASE..S_BASE + S_COUNT).contains(&c) {
        let s = c - S_BASE;
        out.push(L_BASE + s / N_COUNT);
        out.push(V_BASE + (s % N_COUNT) / T_COUNT);
        if s % T_COUNT != 0 {
            out.push(T_BASE + s % T_COUNT);
        }
        return;
    }
    match CANONICAL_DECOMPOSITIONS.binary_search_by_key(&c, |&(cp, _, _)| cp) {
        Ok(i) => {
            let (_, a, b) = CANONICAL_DECOMPOSITIONS[i];
            decompose_into(a, out);
            if b != 0 {
                decompose_into(b, out);
            }
        }
        Err(_) => out.push(c),
    }
}

/// NFD: the full canonical decomposition, combining marks put in canonical order.
fn nfd(s: &str) -> Vec<u32> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        decompose_into(c as u32, &mut out);
    }
    let mut i = 0;
    while i < out.len() {
        if ccc(out[i]) == 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < out.len() && ccc(out[i]) != 0 {
            i += 1;
        }
        out[start..i].sort_by_key(|&c| ccc(c));
    }
    out
}

fn compose_pair(a: u32, b: u32) -> Option<u32> {
    if (L_BASE..L_BASE + L_COUNT).contains(&a) && (V_BASE..V_BASE + V_COUNT).contains(&b) {
        return Some(S_BASE + ((a - L_BASE) * V_COUNT + (b - V_BASE)) * T_COUNT);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&a) && (a - S_BASE) % T_COUNT == 0 && (T_BASE + 1..T_BASE + T_COUNT).contains(&b) {
        return Some(a + (b - T_BASE));
    }
    PRIMARY_COMPOSITES.binary_search_by_key(&(a, b), |&(x, y, _)| (x, y)).ok().map(|i| PRIMARY_COMPOSITES[i].2)
}

/// NFC: the canonical decomposition, recomposed by the canonical composition algorithm.
pub fn nfc(s: &str) -> String {
    let d = nfd(s);
    let mut out: Vec<u32> = Vec::with_capacity(d.len());
    let mut starter: Option<usize> = None;
    let mut last_class: i32 = -1;
    for c in d {
        let class = ccc(c) as i32;
        if let Some(st) = starter {
            let blocked = last_class != -1 && (last_class == 0 || last_class >= class);
            if !blocked {
                if let Some(comp) = compose_pair(out[st], c) {
                    out[st] = comp;
                    continue;
                }
            }
        }
        if class == 0 {
            starter = Some(out.len());
            last_class = -1;
        } else {
            last_class = class;
        }
        out.push(c);
    }
    out.into_iter().filter_map(char::from_u32).collect()
}

/// A normalizer of `tokenizer.json`.
enum Normalizer {
    Nfc,
    Nfd,
    Lowercase,
    Replace(Pattern, String),
    Prepend(String),
    Strip(bool, bool),
    StripAccents,
    Sequence(Vec<Normalizer>),
}

impl Normalizer {
    fn of(v: &Value) -> io::Result<Option<Normalizer>> {
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some(match v["type"].as_str() {
            Some("NFC") => Normalizer::Nfc,
            Some("NFD") => Normalizer::Nfd,
            Some("Lowercase") => Normalizer::Lowercase,
            Some("Replace") => {
                Normalizer::Replace(pattern_of(&v["pattern"])?, v["content"].as_str().unwrap_or_default().to_string())
            }
            Some("Prepend") => Normalizer::Prepend(v["prepend"].as_str().unwrap_or_default().to_string()),
            Some("Strip") => Normalizer::Strip(
                v["strip_left"].as_bool().unwrap_or(true),
                v["strip_right"].as_bool().unwrap_or(true),
            ),
            Some("StripAccents") => Normalizer::StripAccents,
            Some("Sequence") => Normalizer::Sequence(
                v["normalizers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|n| Normalizer::of(n).map(|o| o.unwrap_or(Normalizer::Sequence(Vec::new()))))
                    .collect::<io::Result<_>>()?,
            ),
            other => return Err(invalid(format!("tokenizer.json: the {other:?} normalizer, which this port does not run"))),
        }))
    }

    fn run(&self, s: String) -> String {
        match self {
            Normalizer::Nfc => nfc(&s),
            Normalizer::Nfd => nfd(&s).into_iter().filter_map(char::from_u32).collect(),
            Normalizer::Lowercase => s.chars().flat_map(char::to_lowercase).collect(),
            Normalizer::Replace(p, content) => replace(&s, p, content),
            Normalizer::Prepend(p) if !s.is_empty() => format!("{p}{s}"),
            Normalizer::Prepend(_) => s,
            Normalizer::Strip(left, right) => {
                let t = if *left { s.trim_start() } else { &s };
                (if *right { t.trim_end() } else { t }).to_string()
            }
            Normalizer::StripAccents => s.chars().filter(|&c| !matches!(onig::category_name(c as u32), "Mn" | "Mc" | "Me")).collect(),
            Normalizer::Sequence(v) => v.iter().fold(s, |s, n| n.run(s)),
        }
    }
}

/// `Metaspace`'s scheme for the replacement it puts first.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PrependScheme {
    Always,
    First,
    Never,
}

/// A pre-tokenizer of `tokenizer.json`.
enum PreTokenizer {
    Sequence(Vec<PreTokenizer>),
    Split { pattern: Pattern, behavior: Behavior, invert: bool },
    ByteLevel { add_prefix_space: bool, use_regex: bool },
    Digits(bool),
    Whitespace,
    WhitespaceSplit,
    Punctuation(Behavior),
    Metaspace { replacement: char, scheme: PrependScheme, split: bool },
}

/// The pattern `tokenizers`' byte-level pre-tokenizer splits by when `use_regex` is set (GPT-2's).
const BYTE_LEVEL_PATTERN: &str = r"'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+(?!\S)|\s+";

/// The pattern `Whitespace` keeps.
const WHITESPACE_PATTERN: &str = r"\w+|[^\w\s]+";

/// A split of the text: its content and whether it begins at the original text's first codepoint.
struct Piece {
    text: String,
    at_origin: bool,
}

impl PreTokenizer {
    fn of(v: &Value) -> io::Result<Option<PreTokenizer>> {
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some(match v["type"].as_str() {
            Some("Sequence") => PreTokenizer::Sequence(
                v["pretokenizers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| PreTokenizer::of(p).transpose())
                    .collect::<io::Result<_>>()?,
            ),
            Some("Split") => PreTokenizer::Split {
                pattern: pattern_of(&v["pattern"])?,
                behavior: Behavior::of(&v["behavior"])?,
                invert: v["invert"].as_bool().unwrap_or(false),
            },
            Some("ByteLevel") => PreTokenizer::ByteLevel {
                add_prefix_space: v["add_prefix_space"].as_bool().unwrap_or(true),
                use_regex: v["use_regex"].as_bool().unwrap_or(true),
            },
            Some("Digits") => PreTokenizer::Digits(v["individual_digits"].as_bool().unwrap_or(false)),
            Some("Whitespace") => PreTokenizer::Whitespace,
            Some("WhitespaceSplit") => PreTokenizer::WhitespaceSplit,
            Some("Punctuation") => {
                PreTokenizer::Punctuation(if v["behavior"].is_null() { Behavior::Isolated } else { Behavior::of(&v["behavior"])? })
            }
            Some("Metaspace") => PreTokenizer::Metaspace {
                replacement: v["replacement"].as_str().and_then(|s| s.chars().next()).unwrap_or('\u{2581}'),
                scheme: match v["prepend_scheme"].as_str() {
                    Some("always") | None => PrependScheme::Always,
                    Some("first") => PrependScheme::First,
                    Some("never") => PrependScheme::Never,
                    Some(other) => return Err(invalid(format!("tokenizer.json: Metaspace prepend scheme `{other}`"))),
                },
                split: v["split"].as_bool().unwrap_or(true),
            },
            other => return Err(invalid(format!("tokenizer.json: the {other:?} pre-tokenizer, which this port does not run"))),
        }))
    }

    /// Each piece split further, as `PreTokenizedString::split` does: empty pieces dropped, the first sub-piece of a
    /// piece keeping its place at the origin.
    fn split_each(pieces: Vec<Piece>, f: impl Fn(&Piece) -> Vec<String>) -> Vec<Piece> {
        let mut out = Vec::new();
        for p in pieces {
            for (i, text) in f(&p).into_iter().enumerate() {
                if !text.is_empty() {
                    out.push(Piece { text, at_origin: p.at_origin && i == 0 });
                }
            }
        }
        out
    }

    fn run(&self, pieces: Vec<Piece>) -> Vec<Piece> {
        match self {
            PreTokenizer::Sequence(v) => v.iter().fold(pieces, |p, t| t.run(p)),
            PreTokenizer::Split { pattern, behavior, invert } => {
                Self::split_each(pieces, |p| split(&p.text, pattern, *behavior, *invert))
            }
            PreTokenizer::ByteLevel { add_prefix_space, use_regex } => {
                let re = use_regex.then(|| Pattern::Re(Regex::new(BYTE_LEVEL_PATTERN).expect("the byte-level pattern compiles")));
                let split = Self::split_each(pieces, |p| {
                    let text = if *add_prefix_space && !p.text.starts_with(' ') { format!(" {}", p.text) } else { p.text.clone() };
                    match &re {
                        Some(re) => split(&text, re, Behavior::Isolated, false),
                        None => vec![text],
                    }
                });
                split.into_iter().map(|p| Piece { text: byte_level(&p.text), at_origin: p.at_origin }).collect()
            }
            PreTokenizer::Digits(individual) => {
                let b = if *individual { Behavior::Isolated } else { Behavior::Contiguous };
                Self::split_each(pieces, |p| split(&p.text, &Pattern::Char(char::is_numeric), b, false))
            }
            PreTokenizer::Whitespace => {
                let re = Pattern::Re(Regex::new(WHITESPACE_PATTERN).expect("the whitespace pattern compiles"));
                Self::split_each(pieces, |p| split(&p.text, &re, Behavior::Removed, true))
            }
            PreTokenizer::WhitespaceSplit => {
                Self::split_each(pieces, |p| split(&p.text, &Pattern::Char(char::is_whitespace), Behavior::Removed, false))
            }
            PreTokenizer::Punctuation(b) => Self::split_each(pieces, |p| {
                split(&p.text, &Pattern::Char(|c| c.is_ascii_punctuation() || onig::category_name(c as u32).starts_with('P')), *b, false)
            }),
            PreTokenizer::Metaspace { replacement, scheme, split: do_split } => {
                let rep = replacement.to_string();
                Self::split_each(pieces, |p| {
                    let mut text = p.text.replace(' ', &rep);
                    let prepend = match scheme {
                        PrependScheme::Always => !text.starts_with(*replacement),
                        PrependScheme::First => !text.starts_with(*replacement) && p.at_origin,
                        PrependScheme::Never => false,
                    };
                    if prepend {
                        text = format!("{rep}{text}");
                    }
                    if *do_split {
                        split(&text, &Pattern::Text(vec![*replacement]), Behavior::MergedWithNext, false)
                    } else {
                        vec![text]
                    }
                })
            }
        }
    }
}

/// GPT-2's byte alphabet: each byte of `s` written as the codepoint that stands for it.
fn byte_level(s: &str) -> String {
    s.bytes().map(byte_char).collect()
}

pub(super) fn byte_char(b: u8) -> char {
    let printable = |b: u8| matches!(b, 33..=126 | 161..=172 | 174..=255);
    if printable(b) {
        return b as char;
    }
    let n = (0..b).filter(|&x| !printable(x)).count() as u32;
    char::from_u32(256 + n).unwrap()
}

/// The byte-pair model of `tokenizer.json`.
struct Bpe {
    vocab: HashMap<String, u32>,
    /// `(left, right) -> (rank, merged)`.
    merges: HashMap<(u32, u32), (u32, u32)>,
    unk: Option<u32>,
    fuse_unk: bool,
    byte_fallback: bool,
    prefix: Option<String>,
    suffix: Option<String>,
    ignore_merges: bool,
}

#[derive(Clone, Copy)]
struct Symbol {
    id: u32,
    prev: isize,
    next: isize,
    len: usize,
}

#[derive(PartialEq, Eq)]
struct Merge {
    pos: usize,
    rank: u32,
    new_id: u32,
}

impl Ord for Merge {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.rank.cmp(&self.rank).then(o.pos.cmp(&self.pos))
    }
}

impl PartialOrd for Merge {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Bpe {
    fn of(m: &Value) -> io::Result<Bpe> {
        if m["type"].as_str() != Some("BPE") {
            return Err(invalid(format!("tokenizer.json: a `{}` model, which this port does not run", m["type"])));
        }
        if m["dropout"].as_f64().is_some_and(|d| d > 0.0) {
            return Err(invalid("tokenizer.json: a byte-pair model with dropout, whose output is random"));
        }
        let mut vocab = HashMap::new();
        for (t, id) in m["vocab"].as_object().into_iter().flatten() {
            vocab.insert(t.clone(), id.as_u64().ok_or_else(|| invalid(format!("tokenizer.json: `{t}` has no id")))? as u32);
        }
        let prefix = m["continuing_subword_prefix"].as_str().map(str::to_string);
        let prefix_len = prefix.as_ref().map_or(0, String::len);
        let mut merges = HashMap::new();
        for (rank, entry) in m["merges"].as_array().into_iter().flatten().enumerate() {
            let (a, b) = match entry {
                Value::String(s) => {
                    let parts: Vec<&str> = s.split(' ').collect();
                    if parts.len() != 2 {
                        return Err(invalid(format!("tokenizer.json: merge `{s}` is not two parts")));
                    }
                    (parts[0].to_string(), parts[1].to_string())
                }
                Value::Array(p) if p.len() == 2 => (
                    p[0].as_str().unwrap_or_default().to_string(),
                    p[1].as_str().unwrap_or_default().to_string(),
                ),
                other => return Err(invalid(format!("tokenizer.json: merge {other}"))),
            };
            let id = |t: &str| vocab.get(t).copied().ok_or_else(|| invalid(format!("tokenizer.json: merge token `{t}` is not in the vocabulary")));
            let merged = format!("{a}{}", b.get(prefix_len..).unwrap_or_default());
            merges.insert((id(&a)?, id(&b)?), (rank as u32, id(&merged)?));
        }
        let unk = match m["unk_token"].as_str() {
            Some(u) => Some(*vocab.get(u).ok_or_else(|| invalid(format!("tokenizer.json: unknown token `{u}` is not in the vocabulary")))?),
            None => None,
        };
        Ok(Bpe {
            vocab,
            merges,
            unk,
            fuse_unk: m["fuse_unk"].as_bool().unwrap_or(false),
            byte_fallback: m["byte_fallback"].as_bool().unwrap_or(false),
            prefix,
            suffix: m["end_of_word_suffix"].as_str().map(str::to_string),
            ignore_merges: m["ignore_merges"].as_bool().unwrap_or(false),
        })
    }

    /// The ids of one pre-tokenized split (`BPE::tokenize`).
    fn tokenize(&self, w: &str, out: &mut Vec<u32>) {
        if w.is_empty() {
            return;
        }
        if self.ignore_merges {
            if let Some(&id) = self.vocab.get(w) {
                out.push(id);
                return;
            }
        }
        let mut syms: Vec<Symbol> = Vec::new();
        let add = |syms: &mut Vec<Symbol>, id: u32, len: usize| {
            let n = syms.len() as isize;
            if let Some(last) = syms.last_mut() {
                last.next = n;
            }
            syms.push(Symbol { id, prev: n - 1, next: -1, len });
        };
        let mut unk: Option<(u32, usize)> = None;
        let chars: Vec<(usize, char)> = w.char_indices().collect();
        for (k, &(_, c)) in chars.iter().enumerate() {
            let (first, last) = (k == 0, k + 1 == chars.len());
            let byte_len = c.len_utf8();
            let mut s = c.to_string();
            if !first {
                if let Some(p) = &self.prefix {
                    s = format!("{p}{s}");
                }
            }
            if last {
                if let Some(x) = &self.suffix {
                    s = format!("{s}{x}");
                }
            }
            if let Some(&id) = self.vocab.get(&s) {
                if let Some((u, l)) = unk.take() {
                    add(&mut syms, u, l);
                }
                add(&mut syms, id, byte_len);
                continue;
            }
            if self.byte_fallback {
                let ids: Option<Vec<u32>> = s.bytes().map(|b| self.vocab.get(&format!("<0x{b:02X}>")).copied()).collect();
                if let Some(ids) = ids {
                    for id in ids {
                        add(&mut syms, id, 1);
                    }
                    continue;
                }
            }
            if let Some(u) = self.unk {
                unk = match (unk, self.fuse_unk) {
                    (Some((id, l)), true) => Some((id, l + byte_len)),
                    (Some((id, l)), false) => {
                        add(&mut syms, id, l);
                        Some((u, byte_len))
                    }
                    (None, _) => Some((u, byte_len)),
                };
            }
        }
        if let Some((u, l)) = unk {
            add(&mut syms, u, l);
        }
        let mut queue: BinaryHeap<Merge> = syms
            .windows(2)
            .enumerate()
            .filter_map(|(pos, w)| self.merges.get(&(w[0].id, w[1].id)).map(|&(rank, new_id)| Merge { pos, rank, new_id }))
            .collect();
        while let Some(top) = queue.pop() {
            if syms[top.pos].len == 0 || syms[top.pos].next == -1 {
                continue;
            }
            let next = syms[top.pos].next as usize;
            let right = syms[next];
            if self.merges.get(&(syms[top.pos].id, right.id)).is_none_or(|&(_, id)| id != top.new_id) {
                continue;
            }
            syms[top.pos].id = top.new_id;
            syms[top.pos].len += right.len;
            syms[top.pos].next = right.next;
            syms[next].len = 0;
            if right.next > -1 && (right.next as usize) < syms.len() {
                syms[right.next as usize].prev = top.pos as isize;
            }
            let cur = syms[top.pos];
            if cur.prev >= 0 {
                let prev = cur.prev as usize;
                if let Some(&(rank, new_id)) = self.merges.get(&(syms[prev].id, cur.id)) {
                    queue.push(Merge { pos: prev, rank, new_id });
                }
            }
            if cur.next >= 0 && (cur.next as usize) < syms.len() {
                if let Some(&(rank, new_id)) = self.merges.get(&(cur.id, syms[cur.next as usize].id)) {
                    queue.push(Merge { pos: top.pos, rank, new_id });
                }
            }
        }
        out.extend(syms.iter().filter(|s| s.len != 0).map(|s| s.id));
    }
}

/// An added token: its id, its text (normalized, for a token matched on the normalized text) and its flags.
struct AddedToken {
    id: u32,
    content: Vec<char>,
    single_word: bool,
    lstrip: bool,
    rstrip: bool,
}

/// One pass of the added-token split: the tokens matched leftmost-longest.
struct AddedSet {
    tokens: Vec<AddedToken>,
    /// Token indices by first codepoint, longest first.
    by_first: HashMap<char, Vec<usize>>,
}

impl AddedSet {
    fn new(tokens: Vec<AddedToken>) -> AddedSet {
        let mut by_first: HashMap<char, Vec<usize>> = HashMap::new();
        for (i, t) in tokens.iter().enumerate() {
            if let Some(&c) = t.content.first() {
                by_first.entry(c).or_default().push(i);
            }
        }
        for v in by_first.values_mut() {
            v.sort_by(|&a, &b| tokens[b].content.len().cmp(&tokens[a].content.len()).then(a.cmp(&b)));
        }
        AddedSet { tokens, by_first }
    }

    /// `find_matches` of the added vocabulary: `(token, start, end)` and gaps (`None`), over `s`.
    fn split(&self, s: &[char]) -> Vec<(Option<u32>, usize, usize)> {
        if s.is_empty() {
            return vec![(None, 0, 0)];
        }
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut out = Vec::new();
        let mut start_offset = 0;
        let mut i = 0;
        while i < s.len() {
            let hit = self.by_first.get(&s[i]).and_then(|cands| {
                cands.iter().copied().find(|&t| s[i..].starts_with(&self.tokens[t].content))
            });
            let Some(t) = hit else {
                i += 1;
                continue;
            };
            let tok = &self.tokens[t];
            let (mut start, mut stop) = (i, i + tok.content.len());
            i = stop;
            if tok.single_word {
                let start_space = start == 0 || !word(s[start - 1]);
                let stop_space = stop == s.len() || !word(s[stop]);
                if !start_space || !stop_space {
                    continue;
                }
            }
            if tok.lstrip {
                let mut l = start;
                while l > 0 && s[l - 1].is_whitespace() {
                    l -= 1;
                }
                start = l.max(start_offset);
            }
            if tok.rstrip {
                while stop < s.len() && s[stop].is_whitespace() {
                    stop += 1;
                }
            }
            if start_offset < start {
                out.push((None, start_offset, start));
            }
            out.push((Some(tok.id), start, stop));
            start_offset = stop;
        }
        if start_offset != s.len() {
            out.push((None, start_offset, s.len()));
        }
        out
    }
}

/// The special tokens a post-processor puts around a single sequence.
fn post_processor_wrap(pp: &Value, before: &mut Vec<u32>, after: &mut Vec<u32>) -> io::Result<()> {
    let id_of = |v: &Value, what: &str| -> io::Result<u32> {
        v.as_u64().map(|x| x as u32).ok_or_else(|| invalid(format!("tokenizer.json: the post-processor's {what} has no id")))
    };
    match pp["type"].as_str() {
        None if pp.is_null() => {}
        Some("Sequence") => {
            for p in pp["processors"].as_array().into_iter().flatten() {
                post_processor_wrap(p, before, after)?;
            }
        }
        Some("ByteLevel") => {}
        Some("TemplateProcessing") => {
            let single = pp["single"].as_array().cloned().unwrap_or_default();
            let at = single.iter().position(|i| i.get("Sequence").is_some()).unwrap_or(single.len());
            let ids = |items: &[Value]| -> io::Result<Vec<u32>> {
                let mut out = Vec::new();
                for i in items {
                    if let Some(name) = i["SpecialToken"]["id"].as_str() {
                        for id in pp["special_tokens"][name]["ids"].as_array().into_iter().flatten() {
                            out.push(id_of(id, name)?);
                        }
                    }
                }
                Ok(out)
            };
            let mut head = ids(&single[..at])?;
            head.append(before);
            *before = head;
            after.extend(ids(&single[(at + 1).min(single.len())..])?);
        }
        Some("RobertaProcessing" | "BertProcessing") => {
            before.insert(0, id_of(&pp["cls"][1], "cls")?);
            after.push(id_of(&pp["sep"][1], "sep")?);
        }
        Some(other) => return Err(invalid(format!("tokenizer.json: the `{other}` post-processor, which this port does not run"))),
        None => return Err(invalid("tokenizer.json: a post-processor without a type")),
    }
    Ok(())
}

/// A `tokenizer.json` pre-tokenizer alone, as `tokenizers`' `pre_tokenize_str` runs one.
pub struct HfPreTokenizer(PreTokenizer);

impl HfPreTokenizer {
    pub fn from_json(v: &Value) -> io::Result<HfPreTokenizer> {
        PreTokenizer::of(v)?.map(HfPreTokenizer).ok_or_else(|| invalid("tokenizer.json: no pre-tokenizer"))
    }

    /// The pieces the pre-tokenizer splits `text` into.
    pub fn pre_tokenize_str(&self, text: &str) -> Vec<String> {
        self.0.run(vec![Piece { text: text.to_string(), at_origin: true }]).into_iter().map(|p| p.text).collect()
    }
}

/// A `tokenizer.json` byte-pair tokenizer.
pub struct HfTokenizer {
    raw: AddedSet,
    normalized: AddedSet,
    normalizer: Option<Normalizer>,
    pre: Option<PreTokenizer>,
    model: Bpe,
    before: Vec<u32>,
    after: Vec<u32>,
}

impl HfTokenizer {
    pub fn from_file(path: &Path) -> io::Result<HfTokenizer> {
        let text = std::fs::read_to_string(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
        HfTokenizer::from_json(&json).map_err(|e| invalid(format!("{}: {e}", path.display())))
    }

    pub fn from_json(tj: &Value) -> io::Result<HfTokenizer> {
        let normalizer = Normalizer::of(&tj["normalizer"])?;
        let mut raw = Vec::new();
        let mut normalized = Vec::new();
        for a in tj["added_tokens"].as_array().into_iter().flatten() {
            let content = a["content"].as_str().unwrap_or_default().to_string();
            let id = a["id"].as_u64().ok_or_else(|| invalid(format!("tokenizer.json: added token `{content}` has no id")))? as u32;
            let flag = |k: &str| a[k].as_bool().unwrap_or(false);
            let is_normalized = flag("normalized");
            let text = match (&normalizer, is_normalized) {
                (Some(n), true) => n.run(content),
                _ => content,
            };
            let t = AddedToken { id, content: text.chars().collect(), single_word: flag("single_word"), lstrip: flag("lstrip"), rstrip: flag("rstrip") };
            if is_normalized {
                normalized.push(t);
            } else {
                raw.push(t);
            }
        }
        let (mut before, mut after) = (Vec::new(), Vec::new());
        post_processor_wrap(&tj["post_processor"], &mut before, &mut after)?;
        Ok(HfTokenizer {
            raw: AddedSet::new(raw),
            normalized: AddedSet::new(normalized),
            normalizer,
            pre: PreTokenizer::of(&tj["pre_tokenizer"])?,
            model: Bpe::of(&tj["model"])?,
            before,
            after,
        })
    }

    /// The ids of `text` (`Tokenizer::encode`), with the post-processor's special tokens when `add_special_tokens`.
    pub fn encode(&self, text: &str, add_special_tokens: bool) -> Vec<u32> {
        // A split is an added token's id, or text with whether it begins at the original text's first codepoint.
        enum Part {
            Token(u32),
            Text(String, bool),
        }
        let chars: Vec<char> = text.chars().collect();
        let mut parts: Vec<Part> = self
            .raw
            .split(&chars)
            .into_iter()
            .filter(|&(id, a, b)| id.is_some() || a != b)
            .map(|(id, a, b)| match id {
                Some(id) => Part::Token(id),
                None => Part::Text(chars[a..b].iter().collect(), a == 0),
            })
            .collect();
        let mut second = Vec::new();
        for p in parts.drain(..) {
            match p {
                Part::Token(id) => second.push(Part::Token(id)),
                Part::Text(s, at_origin) => {
                    let s = match &self.normalizer {
                        Some(n) => n.run(s),
                        None => s,
                    };
                    let cs: Vec<char> = s.chars().collect();
                    for (id, a, b) in self.normalized.split(&cs) {
                        match id {
                            Some(id) => second.push(Part::Token(id)),
                            None if a != b => second.push(Part::Text(cs[a..b].iter().collect(), at_origin && a == 0)),
                            None => {}
                        }
                    }
                }
            }
        }
        let mut ids = Vec::new();
        if add_special_tokens {
            ids.extend(&self.before);
        }
        for p in second {
            match p {
                Part::Token(id) => ids.push(id),
                Part::Text(text, at_origin) => {
                    let pieces = vec![Piece { text, at_origin }];
                    let pieces = match &self.pre {
                        Some(pre) => pre.run(pieces),
                        None => pieces,
                    };
                    for piece in pieces {
                        self.model.tokenize(&piece.text, &mut ids);
                    }
                }
            }
        }
        if add_special_tokens {
            ids.extend(&self.after);
        }
        ids
    }
}
