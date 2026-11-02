//! A tiktoken vocabulary (Kimi K2's `tiktoken.model`) as its tokenizer encodes with it and as the Hugging Face converter
//! reads it.
//!
//! `tiktoken.model` lists every mergeable token, base64, beside its rank. The tokenizer's code (`tokenization_kimi.py`)
//! names the split pattern and the count of special tokens reserved after the ranks, each named by
//! `tokenizer_config.json`'s `added_tokens_decoder` or `<|reserved_token_<id>|>`. Its encode cuts the text every
//! 400,000 characters and wherever a run of spaces or of non-spaces passes 25,000, then runs `tiktoken`'s: special
//! tokens' text read as those tokens, the rest split by the pattern, each piece the ranks hold one token and any other
//! merged pairwise, the lowest-ranked adjacent pair first, until no pair merges.
//!
//! The converter's Kimi path (`DeepseekV2Model.set_vocab`) writes each token in GPT-2's byte alphabet and each merge
//! as the pair byte-pair merging below the token's own rank leaves (`QwenModel.bpe`), and names the pre-tokenizer by
//! the digest of the tokenizer's encoding of its probe text.

use super::encode::byte_char;
use super::onig::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::path::Path;

fn invalid(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.into())
}

/// The characters per encode call Kimi's tokenizer cuts a text into (`TIKTOKEN_MAX_ENCODE_CHARS`).
const MAX_ENCODE_CHARS: usize = 400_000;
/// The longest run of spaces or of non-spaces it encodes whole (`MAX_NO_WHITESPACES_CHARS`).
const MAX_RUN_CHARS: usize = 25_000;

/// A tiktoken tokenizer read from its directory.
pub struct Tiktoken {
    /// The mergeable tokens in the file's order, with their ranks.
    tokens: Vec<(Vec<u8>, u32)>,
    ranks: HashMap<Vec<u8>, u32>,
    pattern: Regex,
    /// The special tokens' text and ids, in id order.
    specials: Vec<(String, u32)>,
}

impl Tiktoken {
    /// The tiktoken tokenizer of `dir`: `tiktoken.model`, the pattern and reserved count its tokenizer code states, and
    /// the special tokens `tokenizer_config.json` names.
    pub fn open(dir: &Path) -> io::Result<Tiktoken> {
        let config = read_json(&dir.join("tokenizer_config.json"))?
            .ok_or_else(|| invalid(format!("{}: no tokenizer_config.json", dir.display())))?;
        let module = config
            .get("auto_map")
            .and_then(|m| m.get("AutoTokenizer"))
            .and_then(|a| a.get(0).or(Some(a)))
            .and_then(Value::as_str)
            .and_then(|s| s.split_once('.'))
            .map(|(module, _)| module.to_string())
            .ok_or_else(|| invalid(format!("{}: tokenizer_config.json names no tokenizer code (auto_map)", dir.display())))?;
        let code_path = dir.join(format!("{module}.py"));
        let code = std::fs::read_to_string(&code_path).map_err(|e| invalid(format!("{}: {e}", code_path.display())))?;
        let (pattern, reserved) = pattern_of(&code).map_err(|e| invalid(format!("{}: {e}", code_path.display())))?;
        let pattern = Regex::tiktoken(&pattern).map_err(|e| invalid(format!("{}: {e}", code_path.display())))?;

        let model_path = dir.join("tiktoken.model");
        let text = std::fs::read_to_string(&model_path).map_err(|e| invalid(format!("{}: {e}", model_path.display())))?;
        let mut tokens = Vec::new();
        for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.is_empty()) {
            let mut fields = line.split_whitespace();
            let (Some(b64), Some(rank), None) = (fields.next(), fields.next(), fields.next()) else {
                return Err(invalid(format!("{}: line {} is not `<base64> <rank>`", model_path.display(), n + 1)));
            };
            let bytes = base64(b64).ok_or_else(|| invalid(format!("{}: line {}: bad base64", model_path.display(), n + 1)))?;
            let rank = rank.parse().map_err(|_| invalid(format!("{}: line {}: bad rank", model_path.display(), n + 1)))?;
            tokens.push((bytes, rank));
        }
        // `load_tiktoken_bpe` builds a dict: a token listed twice keeps its last rank, at its first place.
        let mut ranks: HashMap<Vec<u8>, u32> = HashMap::new();
        let mut order: Vec<Vec<u8>> = Vec::new();
        for (bytes, rank) in tokens {
            if ranks.insert(bytes.clone(), rank).is_none() {
                order.push(bytes);
            }
        }
        let tokens: Vec<(Vec<u8>, u32)> = order
            .into_iter()
            .map(|b| {
                let rank = ranks[&b];
                (b, rank)
            })
            .collect();

        let named: HashMap<u32, String> = match config.get("added_tokens_decoder") {
            Some(Value::Object(d)) => d
                .iter()
                .filter_map(|(id, t)| Some((id.parse().ok()?, t.get("content")?.as_str()?.to_string())))
                .collect(),
            _ => HashMap::new(),
        };
        let base = tokens.len() as u32;
        let specials: Vec<(String, u32)> = (base..base + reserved)
            .map(|id| (named.get(&id).cloned().unwrap_or_else(|| format!("<|reserved_token_{id}|>")), id))
            .collect();
        Ok(Tiktoken { tokens, ranks, pattern, specials })
    }

    /// The mergeable tokens, in the file's order, with their ranks.
    pub fn tokens(&self) -> &[(Vec<u8>, u32)] {
        &self.tokens
    }

    /// The special tokens' text and ids, in id order.
    pub fn specials(&self) -> &[(String, u32)] {
        &self.specials
    }

    /// `text` encoded as the tokenizer's `encode(text)` encodes it, its special tokens allowed.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        for chunk in chars.chunks(MAX_ENCODE_CHARS) {
            for piece in runs_split(chunk, MAX_RUN_CHARS) {
                self.encode_with_specials(&piece.iter().collect::<String>(), &mut out);
            }
        }
        out
    }

    /// `tiktoken`'s `encode(text, allowed_special="all")`: each special token's text, leftmost first (the longest of
    /// several at one place), read as the token; the text between encoded ordinarily.
    fn encode_with_specials(&self, text: &str, out: &mut Vec<u32>) {
        let mut start = 0;
        while start <= text.len() {
            let next = self
                .specials
                .iter()
                .filter_map(|(s, id)| text[start..].find(s.as_str()).map(|at| (start + at, s.len(), *id)))
                .min_by_key(|&(at, len, _)| (at, std::cmp::Reverse(len)));
            let end = next.map_or(text.len(), |(at, _, _)| at);
            self.encode_ordinary(&text[start..end], out);
            match next {
                Some((at, len, id)) => {
                    out.push(id);
                    start = at + len;
                }
                None => break,
            }
        }
    }

    /// The text split by the pattern, each piece a token or byte-pair merged.
    fn encode_ordinary(&self, text: &str, out: &mut Vec<u32>) {
        let cps: Vec<u32> = text.chars().map(u32::from).collect();
        for (lo, hi) in self.pattern.find_iter(&cps) {
            let piece: String = cps[lo..hi].iter().filter_map(|&c| char::from_u32(c)).collect();
            match self.ranks.get(piece.as_bytes()) {
                Some(&rank) => out.push(rank),
                None => out.extend(byte_pair_encode(piece.as_bytes(), &self.ranks)),
            }
        }
    }

    /// The pair byte-pair merging below `rank` leaves of `token` (`QwenModel.bpe(ranks, token, max_rank=rank)`), when it
    /// leaves two parts.
    pub fn merge_of(&self, token: &[u8], rank: u32) -> Option<(Vec<u8>, Vec<u8>)> {
        let mut parts: Vec<Vec<u8>> = token.iter().map(|&b| vec![b]).collect();
        loop {
            let mut best: Option<(usize, u32)> = None;
            for i in 0..parts.len().saturating_sub(1) {
                let pair = [parts[i].as_slice(), parts[i + 1].as_slice()].concat();
                if let Some(&r) = self.ranks.get(&pair) {
                    if best.is_none_or(|(_, b)| r < b) {
                        best = Some((i, r));
                    }
                }
            }
            match best {
                Some((i, r)) if r < rank => {
                    let right = parts.remove(i + 1);
                    parts[i].extend(right);
                }
                _ => break,
            }
        }
        (parts.len() == 2).then(|| (parts[0].clone(), parts[1].clone()))
    }
}

/// The pieces `tiktoken`'s pattern `pattern` finds in `text`, in order.
pub fn split(pattern: &str, text: &str) -> Result<Vec<String>, String> {
    let re = Regex::tiktoken(pattern)?;
    let cps: Vec<u32> = text.chars().map(u32::from).collect();
    Ok(re.find_iter(&cps).into_iter().map(|(lo, hi)| cps[lo..hi].iter().filter_map(|&c| char::from_u32(c)).collect()).collect())
}

/// `bytes` in GPT-2's byte alphabet (`QwenModel.token_bytes_to_string`).
pub fn byte_level(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| byte_char(b)).collect()
}

/// `tiktoken`'s `byte_pair_encode`: a piece of one byte its rank; any other merged at its lowest-ranked adjacent pair
/// (the leftmost of equals) until none of its pairs is ranked.
fn byte_pair_encode(piece: &[u8], ranks: &HashMap<Vec<u8>, u32>) -> Vec<u32> {
    if piece.len() == 1 {
        return vec![ranks[piece]];
    }
    let rank_of = |a: usize, b: usize| ranks.get(&piece[a..b]).copied().unwrap_or(u32::MAX);
    // `(start, rank of the pair starting here)`, the last two sentinels.
    let mut parts: Vec<(usize, u32)> = (0..piece.len() - 1).map(|i| (i, rank_of(i, i + 2))).collect();
    parts.push((piece.len() - 1, u32::MAX));
    parts.push((piece.len(), u32::MAX));
    loop {
        let (mut min, mut at) = (u32::MAX, usize::MAX);
        for (i, &(_, r)) in parts[..parts.len() - 1].iter().enumerate() {
            if r < min {
                (min, at) = (r, i);
            }
        }
        if min == u32::MAX {
            break;
        }
        let pair_rank = |parts: &[(usize, u32)], i: usize| {
            if i + 3 < parts.len() {
                rank_of(parts[i].0, parts[i + 3].0)
            } else {
                u32::MAX
            }
        };
        if at > 0 {
            parts[at - 1].1 = pair_rank(&parts, at - 1);
        }
        parts[at].1 = pair_rank(&parts, at);
        parts.remove(at + 1);
    }
    parts.windows(2).map(|w| ranks[&piece[w[0].0..w[1].0]]).collect()
}

/// Kimi's `_split_whitespaces_or_nonwhitespaces`: `s` cut wherever a run of spaces or of non-spaces (Python's
/// `str.isspace`) would pass `max` characters.
fn runs_split(s: &[char], max: usize) -> Vec<&[char]> {
    let mut out = Vec::new();
    let mut run = 0;
    let mut in_space = s.first().is_some_and(|&c| python_space(c));
    let mut start = 0;
    for (i, &c) in s.iter().enumerate() {
        let space = python_space(c);
        if in_space != space {
            run = 1;
            in_space = space;
        } else {
            run += 1;
            if run > max {
                out.push(&s[start..i]);
                start = i;
                run = 1;
            }
        }
    }
    out.push(&s[start..]);
    out
}

/// Python's `str.isspace` of one character: bidirectional class WS, B or S, or general category Zs.
fn python_space(c: char) -> bool {
    matches!(
        c as u32,
        0x09..=0x0D | 0x1C..=0x20 | 0x85 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000
    )
}

/// The split pattern (`pat_str = "|".join([r"""…""", …])`) and the reserved special-token count
/// (`num_reserved_special_tokens = <n>`) a tiktoken tokenizer's code states.
fn pattern_of(code: &str) -> Result<(String, u32), String> {
    let start = code.find("pat_str = \"|\".join(").ok_or("no `pat_str = \"|\".join(` in the tokenizer code")?;
    let body = &code[start..];
    let end = body.find("])").ok_or("the pattern's list is not closed")?;
    let mut alternatives = Vec::new();
    let mut rest = &body[..end];
    while let Some(open) = rest.find("r\"\"\"") {
        let after = &rest[open + 4..];
        let close = after.find("\"\"\"").ok_or("an unclosed raw string in the pattern")?;
        alternatives.push(&after[..close]);
        rest = &after[close + 3..];
    }
    if alternatives.is_empty() {
        return Err("the pattern's list holds no raw strings".into());
    }
    let reserved = code
        .lines()
        .find_map(|l| l.trim().strip_prefix("num_reserved_special_tokens = "))
        .ok_or("no `num_reserved_special_tokens` in the tokenizer code")?
        .trim()
        .parse()
        .map_err(|_| "num_reserved_special_tokens is not a count")?;
    Ok((alternatives.join("|"), reserved))
}

/// Standard base64 (RFC 4648), its padding optional.
fn base64(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let digits: Vec<u8> = s.bytes().filter(|&c| c != b'=').collect();
    if digits.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for group in digits.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in group.iter().enumerate() {
            acc |= value(c)? << (18 - 6 * i);
        }
        out.extend_from_slice(&acc.to_be_bytes()[1..group.len()]);
    }
    Some(out)
}

fn read_json(path: &Path) -> io::Result<Option<Value>> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map(Some).map_err(|e| invalid(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
