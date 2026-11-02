//! The tokenizer a GGUF checkpoint carries, read from its `tokenizer.ggml.*` metadata and run the way llama.cpp runs
//! it (`src/llama-vocab.cpp` at the pinned commit): text to token ids, ids to text pieces, and the special tokens
//! that end generation.
//!
//! Two vocabularies are tokenized: byte-pair (`gpt2` and its `gemma4`, `hybriddna` and `whitespace` forms — Qwen,
//! Llama 3, gpt-oss, DeepSeek) and SentencePiece (`llama` — Gemma 3, Llama 2, Mistral). WordPiece, Unigram, RWKV and
//! PLaMo vocabularies are refused by name. `model_tokenizer_lock` holds every result equal to llama.cpp's own, on its
//! bundled vocabulary corpora and on seeded random text.
//!
//! Text is bytes: a fragment that is not UTF-8 tokenizes as llama.cpp tokenizes it (an invalid byte reads as
//! U+FFFD in byte-pair splitting, and SentencePiece falls back to byte tokens).

pub mod cxx;
pub mod pre;
pub mod regex;
pub mod split;
pub mod unicode;
mod unicode_tables;

use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};
use std::fmt;

use super::gguf::{Gguf, MetaValue};
use pre::{PreSettings, PreType};
use regex::RegexError;
use split::Splitter;
use unicode::InvalidCodepoint;

/// A token attribute bit, as llama.cpp's `llama_token_attr`.
pub mod attr {
    pub const UNDEFINED: u32 = 0;
    pub const UNKNOWN: u32 = 1 << 0;
    pub const UNUSED: u32 = 1 << 1;
    pub const NORMAL: u32 = 1 << 2;
    pub const CONTROL: u32 = 1 << 3;
    pub const USER_DEFINED: u32 = 1 << 4;
    pub const BYTE: u32 = 1 << 5;
    pub const NORMALIZED: u32 = 1 << 6;
    pub const LSTRIP: u32 = 1 << 7;
    pub const RSTRIP: u32 = 1 << 8;
    pub const SINGLE_WORD: u32 = 1 << 9;
}

/// The SentencePiece escaped space, U+2581.
const ESCAPED_SPACE: &[u8] = "\u{2581}".as_bytes();
/// The suffix a `hybriddna` vocabulary marks its k-mer tokens with.
const DNA_KMER_MARKER: &[u8] = b"\xee\x80\x80";

/// The tokenizer family a vocabulary runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VocabKind {
    /// SentencePiece byte-fallback BPE (`tokenizer.ggml.model = "llama"`).
    Spm,
    /// Byte-pair merges over pre-tokenized words.
    Bpe(PreType),
}

/// How a byte-pair vocabulary's fragments are cut before the generic merge loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BpeSession {
    Plain,
    HybridDna,
    Whitespace,
}

/// A vocabulary that cannot be read, by name.
#[derive(Clone, Debug, PartialEq)]
pub enum VocabError {
    Missing(&'static str),
    WrongType(&'static str),
    UnknownModel(String),
    UnsupportedModel(String),
    UnknownPre(String),
    TooFewScores { scores: usize, tokens: usize },
    TooFewTypes { types: usize, tokens: usize },
    DuplicateToken(Vec<u8>),
    Regex(RegexError),
    NoNewline,
}

impl fmt::Display for VocabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VocabError::Missing(k) => write!(f, "the vocabulary has no `{k}`"),
            VocabError::WrongType(k) => write!(f, "`{k}` has the wrong GGUF type"),
            VocabError::UnknownModel(m) => write!(f, "unknown tokenizer model `{m}`"),
            VocabError::UnsupportedModel(m) => {
                write!(f, "the `{m}` tokenizer is not run here (byte-pair and SentencePiece vocabularies are)")
            }
            VocabError::UnknownPre(p) => write!(f, "unknown pre-tokenizer `{p}`"),
            VocabError::TooFewScores { scores, tokens } => write!(f, "{scores} scores for {tokens} tokens"),
            VocabError::TooFewTypes { types, tokens } => write!(f, "{types} token types for {tokens} tokens"),
            VocabError::DuplicateToken(t) => write!(f, "the token {:?} appears twice", String::from_utf8_lossy(t)),
            VocabError::Regex(e) => write!(f, "{e}"),
            VocabError::NoNewline => write!(f, "the SentencePiece vocabulary has no newline token"),
        }
    }
}

impl std::error::Error for VocabError {}

/// A text llama.cpp would abort on, by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenizeError {
    /// A codepoint above U+10FFFF, which no UTF-8 sequence encodes.
    InvalidCodepoint(u32),
    /// A byte with no byte token in a byte-fallback vocabulary.
    MissingByteToken(u8),
    /// `add_special` asked for a beginning- or end-of-sequence token the vocabulary does not have.
    MissingSpecial(&'static str),
}

impl fmt::Display for TokenizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenizeError::InvalidCodepoint(c) => write!(f, "the text decodes to U+{c:X}, past U+10FFFF"),
            TokenizeError::MissingByteToken(b) => write!(f, "the vocabulary has no token for byte 0x{b:02X}"),
            TokenizeError::MissingSpecial(s) => write!(f, "the vocabulary has no {s} token"),
        }
    }
}

impl std::error::Error for TokenizeError {}

impl From<InvalidCodepoint> for TokenizeError {
    fn from(e: InvalidCodepoint) -> Self {
        TokenizeError::InvalidCodepoint(e.0)
    }
}

/// One vocabulary entry.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenData {
    pub text: Vec<u8>,
    pub score: f32,
    pub attr: u32,
}

/// The special token ids, each `None` when the vocabulary has none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Specials {
    pub bos: Option<u32>,
    pub eos: Option<u32>,
    pub eot: Option<u32>,
    pub eom: Option<u32>,
    pub unk: Option<u32>,
    pub sep: Option<u32>,
    pub pad: Option<u32>,
    pub mask: Option<u32>,
    pub fim_pre: Option<u32>,
    pub fim_suf: Option<u32>,
    pub fim_mid: Option<u32>,
    pub fim_pad: Option<u32>,
    pub fim_rep: Option<u32>,
    pub fim_sep: Option<u32>,
}

/// A loaded vocabulary and its tokenizer.
#[derive(Clone, Debug)]
pub struct Vocab {
    pub kind: VocabKind,
    pub model: String,
    pub specials: Specials,
    pub linefeed: Option<u32>,
    pub add_space_prefix: bool,
    pub add_bos: bool,
    pub add_eos: bool,
    pub ignore_merges: bool,
    pub clean_spaces: bool,
    pub escape_whitespaces: bool,
    pub treat_whitespace_as_suffix: bool,
    pub lowercase: bool,
    tokens: Vec<TokenData>,
    token_to_id: HashMap<Vec<u8>, u32>,
    bpe_ranks: HashMap<Vec<u8>, u32>,
    eog: BTreeSet<u32>,
    special_cache: Vec<u32>,
    piece_cache: Vec<Vec<u8>>,
    splitter: Option<Splitter>,
    session: BpeSession,
}

/// The key a merge `(first, second)` is ranked under.
fn pair_key(first: &[u8], second: &[u8]) -> Vec<u8> {
    let mut k = Vec::with_capacity(4 + first.len() + second.len());
    k.extend_from_slice(&(first.len() as u32).to_le_bytes());
    k.extend_from_slice(first);
    k.extend_from_slice(second);
    k
}

fn meta_str<'a>(meta: &'a BTreeMap<String, MetaValue>, key: &str) -> Option<&'a str> {
    meta.get(key).and_then(MetaValue::as_str)
}

/// `std::string::find(needle, from)` over bytes.
fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return (from <= hay.len()).then_some(from);
    }
    if from > hay.len() || hay.len() - from < needle.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// `replace_all(s, search, replace)`.
fn replace_all(s: &[u8], search: &[u8], replace: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut last = 0;
    while let Some(p) = find(s, search, last) {
        out.extend_from_slice(&s[last..p]);
        out.extend_from_slice(replace);
        last = p + search.len();
    }
    out.extend_from_slice(&s[last..]);
    out
}

fn c_isspace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// A fragment of the text being tokenized: raw bytes still to split, or a special token already matched.
#[derive(Clone, Copy, Debug)]
enum Fragment {
    Token(u32),
    Raw { offset: usize, len: usize },
}

impl Vocab {
    /// Read the vocabulary of `g`.
    pub fn from_gguf(g: &Gguf) -> Result<Vocab, VocabError> {
        Vocab::from_meta(&g.meta)
    }

    /// Read the vocabulary a checkpoint's metadata describes: its `tokenizer.*` keys, and the `general.*` keys whose
    /// names llama.cpp keys token attributes by.
    pub fn from_meta(meta: &BTreeMap<String, MetaValue>) -> Result<Vocab, VocabError> {
        let model = meta_str(meta, "tokenizer.ggml.model")
            .ok_or(VocabError::Missing("tokenizer.ggml.model"))?
            .to_string();
        let mut pre_name = meta_str(meta, "tokenizer.ggml.pre").map(str::to_string);
        let mut specials = Specials::default();
        let mut kind_spm = false;
        let mut bpe_ranks = HashMap::new();
        let mut session = BpeSession::Plain;
        let read_merges = |ranks: &mut HashMap<Vec<u8>, u32>| -> Result<(), VocabError> {
            let merges = meta
                .get("tokenizer.ggml.merges")
                .ok_or(VocabError::Missing("tokenizer.ggml.merges"))?
                .as_array()
                .ok_or(VocabError::WrongType("tokenizer.ggml.merges"))?;
            for (i, m) in merges.iter().enumerate() {
                let word = m.as_bytes().ok_or(VocabError::WrongType("tokenizer.ggml.merges"))?;
                let (first, second): (&[u8], &[u8]) = match find(word, b" ", 1) {
                    Some(p) => (&word[..p], &word[p + 1..]),
                    None => (b"", b""),
                };
                ranks.entry(pair_key(first, second)).or_insert(i as u32);
            }
            Ok(())
        };
        match model.as_str() {
            "llama" => {
                kind_spm = true;
                specials.bos = Some(1);
                specials.eos = Some(2);
                specials.unk = Some(0);
            }
            "gpt2" | "hybriddna" | "whitespace" => {
                if pre_name.as_deref() != Some("kimi-k2") || meta.contains_key("tokenizer.ggml.merges") {
                    read_merges(&mut bpe_ranks)?;
                }
                specials.bos = Some(11);
                specials.eos = Some(11);
                session = match model.as_str() {
                    "hybriddna" => BpeSession::HybridDna,
                    "whitespace" => BpeSession::Whitespace,
                    _ => BpeSession::Plain,
                };
            }
            "gemma4" => {
                read_merges(&mut bpe_ranks)?;
                pre_name = Some("gemma4".to_string());
            }
            "bert" | "t5" | "rwkv" | "plamo2" | "plamo3" | "test" | "no_vocab" | "none" => {
                return Err(VocabError::UnsupportedModel(model));
            }
            other => return Err(VocabError::UnknownModel(other.to_string())),
        }

        let (kind, mut add_space_prefix, add_bos, add_eos, ignore_merges, clean_spaces, escape_whitespaces, lowercase_cleared) =
            if kind_spm {
                (VocabKind::Spm, true, true, false, false, false, true, false)
            } else {
                let (pt, set): (PreType, PreSettings) = match pre_name.as_deref() {
                    None => (PreType::Default, PreSettings::default()),
                    Some(p) => pre::pre_type(p).ok_or_else(|| VocabError::UnknownPre(p.to_string()))?,
                };
                if set.no_bos_token {
                    specials.bos = None;
                }
                (
                    VocabKind::Bpe(pt),
                    false,
                    set.add_bos.unwrap_or(false),
                    false,
                    set.ignore_merges.unwrap_or(false),
                    set.clean_spaces.unwrap_or(true),
                    set.escape_whitespaces.unwrap_or(false),
                    set.no_lowercase,
                )
            };
        let get_bool = |k: &'static str| -> Result<Option<bool>, VocabError> {
            match meta.get(k) {
                None => Ok(None),
                Some(v) => v.as_bool().map(Some).ok_or(VocabError::WrongType(k)),
            }
        };
        if let Some(v) = get_bool("tokenizer.ggml.add_space_prefix")? {
            add_space_prefix = v;
        }

        let list = meta
            .get("tokenizer.ggml.tokens")
            .ok_or(VocabError::Missing("tokenizer.ggml.tokens"))?
            .as_array()
            .ok_or(VocabError::WrongType("tokenizer.ggml.tokens"))?;
        let n = list.len();
        let scores: Option<Vec<f32>> = match meta.get("tokenizer.ggml.scores") {
            None => None,
            Some(v) => {
                let a = v.as_array().ok_or(VocabError::WrongType("tokenizer.ggml.scores"))?;
                if a.len() < n {
                    return Err(VocabError::TooFewScores { scores: a.len(), tokens: n });
                }
                Some(
                    a.iter()
                        .map(|x| match *x {
                            MetaValue::F32(f) => Ok(f),
                            MetaValue::I32(i) => Ok(i as f32),
                            _ => Err(VocabError::WrongType("tokenizer.ggml.scores")),
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
        };
        let types: Option<Vec<i32>> = match meta.get("tokenizer.ggml.token_type") {
            None => None,
            Some(v) => {
                let a = v.as_array().ok_or(VocabError::WrongType("tokenizer.ggml.token_type"))?;
                if a.len() < n {
                    return Err(VocabError::TooFewTypes { types: a.len(), tokens: n });
                }
                Some(
                    a.iter()
                        .map(|x| match *x {
                            MetaValue::I32(i) => Ok(i),
                            _ => Err(VocabError::WrongType("tokenizer.ggml.token_type")),
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
        };
        let mut tokens = Vec::with_capacity(n);
        let mut token_to_id = HashMap::with_capacity(n);
        for (i, t) in list.iter().enumerate() {
            let mut text = t.as_bytes().ok_or(VocabError::WrongType("tokenizer.ggml.tokens"))?.to_vec();
            if text.is_empty() {
                text = format!("[EMPTY_{i}]").into_bytes();
            }
            if token_to_id.insert(text.clone(), i as u32).is_some() {
                return Err(VocabError::DuplicateToken(text));
            }
            let attr = match types.as_ref().map(|t| t[i]) {
                None => attr::NORMAL,
                Some(0) => attr::UNDEFINED,
                Some(1) => attr::NORMAL,
                Some(2) => attr::UNKNOWN,
                Some(3) => attr::CONTROL,
                Some(4) => attr::USER_DEFINED,
                Some(5) => attr::UNUSED,
                Some(6) => attr::BYTE,
                Some(_) => attr::UNDEFINED,
            };
            tokens.push(TokenData { text, score: scores.as_ref().map_or(0.0, |s| s[i]), attr });
        }
        let map_hashes: Vec<u64> = tokens.iter().map(|t: &TokenData| cxx::hash_string(&t.text)).collect();
        if model == "hybriddna" {
            if let Some(&oov) = token_to_id.get(b"<oov>".as_slice()) {
                for t in tokens.iter_mut().skip(oov as usize + 1) {
                    if t.text.len() > DNA_KMER_MARKER.len() && t.text.ends_with(DNA_KMER_MARKER) {
                        let keep = t.text.len() - DNA_KMER_MARKER.len();
                        t.text.truncate(keep);
                    }
                }
            }
        }

        let splitter = match kind {
            VocabKind::Bpe(pt) => Some(Splitter::new(pt.regexes(), pt.byte_encode()).map_err(VocabError::Regex)?),
            VocabKind::Spm => None,
        };
        let mut v = Vocab {
            kind,
            model,
            specials,
            linefeed: None,
            add_space_prefix,
            add_bos,
            add_eos,
            ignore_merges,
            clean_spaces,
            escape_whitespaces,
            treat_whitespace_as_suffix: false,
            lowercase: true,
            tokens,
            token_to_id,
            bpe_ranks,
            eog: BTreeSet::new(),
            special_cache: Vec::new(),
            piece_cache: Vec::new(),
            splitter,
            session,
        };

        v.linefeed = match kind {
            VocabKind::Spm => Some(v.byte_to_token(b'\n').map_err(|_| VocabError::NoNewline)?),
            VocabKind::Bpe(_) => v.tokenize(b"\n", false, false).ok().and_then(|ids| ids.first().copied()),
        };

        let n_tokens = v.tokens.len();
        let in_range = |id: Option<u32>| id.filter(|&i| (i as usize) < n_tokens);
        for slot in [&mut v.specials.bos, &mut v.specials.eos, &mut v.specials.unk] {
            *slot = in_range(*slot);
        }
        let keys: [(&'static str, fn(&mut Specials) -> &mut Option<u32>); 17] = [
            ("tokenizer.ggml.bos_token_id", |s| &mut s.bos),
            ("tokenizer.ggml.eos_token_id", |s| &mut s.eos),
            ("tokenizer.ggml.eot_token_id", |s| &mut s.eot),
            ("tokenizer.ggml.eom_token_id", |s| &mut s.eom),
            ("tokenizer.ggml.unknown_token_id", |s| &mut s.unk),
            ("tokenizer.ggml.seperator_token_id", |s| &mut s.sep),
            ("tokenizer.ggml.padding_token_id", |s| &mut s.pad),
            ("tokenizer.ggml.mask_token_id", |s| &mut s.mask),
            ("tokenizer.ggml.fim_pre_token_id", |s| &mut s.fim_pre),
            ("tokenizer.ggml.fim_suf_token_id", |s| &mut s.fim_suf),
            ("tokenizer.ggml.fim_mid_token_id", |s| &mut s.fim_mid),
            ("tokenizer.ggml.fim_pad_token_id", |s| &mut s.fim_pad),
            ("tokenizer.ggml.fim_rep_token_id", |s| &mut s.fim_rep),
            ("tokenizer.ggml.fim_sep_token_id", |s| &mut s.fim_sep),
            ("tokenizer.ggml.prefix_token_id", |s| &mut s.fim_pre),
            ("tokenizer.ggml.suffix_token_id", |s| &mut s.fim_suf),
            ("tokenizer.ggml.middle_token_id", |s| &mut s.fim_mid),
        ];
        for (key, slot) in keys {
            let Some(val) = meta.get(key) else { continue };
            let MetaValue::U32(id) = *val else { return Err(VocabError::WrongType(key)) };
            if (id as usize) < n_tokens {
                *slot(&mut v.specials) = Some(id);
            }
        }
        if let Some(b) = get_bool("tokenizer.ggml.add_bos_token")? {
            v.add_bos = b;
        }
        if let Some(b) = get_bool("tokenizer.ggml.add_eos_token")? {
            v.add_eos = b;
        }
        if v.kind == VocabKind::Bpe(PreType::Gemma4) && !v.add_bos {
            v.add_bos = true;
        }
        v.lowercase = !lowercase_cleared;
        if let Some(b) = get_bool("tokenizer.ggml.normalizer.lowercase")? {
            v.lowercase = b;
        }

        v.detect_specials(&map_hashes);
        v.build_special_cache();
        v.piece_cache = (0..n_tokens as u32).map(|id| v.piece_uncached(id)).collect();
        v.apply_name_attrs(meta);
        Ok(v)
    }

    /// The text-matched special tokens, the end-of-generation set, and the attribute fixes llama.cpp applies after
    /// reading the vocabulary. A slot takes the first matching token in the order llama.cpp's `token_to_id` map
    /// iterates ([`cxx::unordered_map_order`] over `map_hashes`, the hashes of the texts as the map was keyed).
    fn detect_specials(&mut self, map_hashes: &[u64]) {
        type Slot = fn(&mut Specials) -> &mut Option<u32>;
        let rules: [(Slot, &[&str]); 8] = [
            (
                |s| &mut s.eot,
                &[
                    "<|eot_id|>",
                    "<|im_end|>",
                    "<|end|>",
                    "<end_of_turn>",
                    "<|endoftext|>",
                    "<|end_of_text|>",
                    "<EOT>",
                    "_<EOT>",
                    "[EOT]",
                    "<｜end▁of▁sentence｜>",
                    "<end_of_utterance>",
                ],
            ),
            (|s| &mut s.eom, &["<|eom_id|>"]),
            (
                |s| &mut s.fim_pre,
                &[
                    "<|fim_prefix|>",
                    "<fim-prefix>",
                    "<fim_prefix>",
                    "<｜fim▁begin｜>",
                    "<PRE>",
                    "▁<PRE>",
                    "<|code_prefix|>",
                    "<|prefix|>",
                    "<|plamo:fim_prefix|>",
                ],
            ),
            (
                |s| &mut s.fim_suf,
                &[
                    "<|fim_suffix|>",
                    "<fim-suffix>",
                    "<fim_suffix>",
                    "<｜fim▁hole｜>",
                    "<SUF>",
                    "▁<SUF>",
                    "<|code_suffix|>",
                    "<|suffix|>",
                    "<|plamo:fim_suffix|>",
                ],
            ),
            (
                |s| &mut s.fim_mid,
                &[
                    "<|fim_middle|>",
                    "<fim-middle>",
                    "<fim_middle>",
                    "<｜fim▁end｜>",
                    "<MID>",
                    "▁<MID>",
                    "<|code_middle|>",
                    "<|middle|>",
                    "<|plamo:fim_middle|>",
                ],
            ),
            (|s| &mut s.fim_pad, &["<|fim_pad|>", "<fim-pad>", "<fim_pad>", "<PAD>", "[PAD]"]),
            (|s| &mut s.fim_rep, &["<|fim_repo|>", "<|repo_name|>", "<fim-repo>", "<REPO>", "<reponame>"]),
            (|s| &mut s.fim_sep, &["<|file_sep|>", "<|plamo:file_separator|>"]),
        ];
        let mut rank = vec![0u32; map_hashes.len()];
        for (pos, id) in cxx::unordered_map_order(map_hashes).into_iter().enumerate() {
            rank[id as usize] = pos as u32;
        }
        for (slot, texts) in rules {
            if slot(&mut self.specials).is_some() {
                continue;
            }
            let found = texts
                .iter()
                .filter_map(|t| self.token_to_id.get(t.as_bytes()).copied())
                .min_by_key(|&id| rank[id as usize]);
            if let Some(id) = found {
                *slot(&mut self.specials) = Some(id);
                self.tokens[id as usize].attr |= attr::CONTROL;
            }
        }
        for t in &mut self.tokens {
            if t.attr & attr::CONTROL != 0 && t.attr & attr::UNUSED == 0 && find(&t.text, b"unused", 0).is_some() {
                t.attr |= attr::UNUSED;
            }
        }
        let mut eog = BTreeSet::new();
        for id in [self.specials.fim_pad, self.specials.fim_rep, self.specials.fim_sep].into_iter().flatten() {
            eog.insert(id);
        }
        const EOG_TEXTS: [&str; 23] = [
            "<|eot_id|>",
            "<|im_end|>",
            "<|end|>",
            "<|return|>",
            "<|call|>",
            "<|flush|>",
            "<|calls|>",
            "<end_of_turn>",
            "<|endoftext|>",
            "</s>",
            "<|eom_id|>",
            "<EOT>",
            "_<EOT>",
            "[EOT]",
            "[EOS]",
            "<|end_of_text|>",
            "<end_of_utterance>",
            "<eos>",
            "<turn|>",
            "<|tool_response>",
            "<｜end▁of▁sentence｜>",
            "[e~[",
            "<|ifm|im_end|>",
        ];
        for text in EOG_TEXTS {
            if let Some(&id) = self.token_to_id.get(text.as_bytes()) {
                eog.insert(id);
                self.tokens[id as usize].attr |= attr::CONTROL;
            }
        }
        for text in ["<|channel|>", "<|message|>", "<|start|>", "<|constrain|>"] {
            if let Some(&id) = self.token_to_id.get(text.as_bytes()) {
                self.tokens[id as usize].attr = attr::USER_DEFINED;
            }
        }
        for id in [self.specials.eos, self.specials.eot, self.specials.eom].into_iter().flatten() {
            eog.insert(id);
        }
        let find_eog = |tokens: &[TokenData], eog: &BTreeSet<u32>, text: &[u8]| {
            eog.iter().copied().find(|&id| tokens.get(id as usize).is_some_and(|t| t.text == text))
        };
        if let Some(end_id) = find_eog(&self.tokens, &eog, b"<|end|>") {
            let has = |t: &[u8]| find_eog(&self.tokens, &eog, t).is_some();
            let has_call = has(b"<|call|>") || has(b"<|calls|>");
            if (has(b"<|return|>") && has_call) || (has_call && has(b"<|flush|>")) {
                eog.remove(&end_id);
                self.tokens[end_id as usize].attr = attr::USER_DEFINED;
            }
        }
        let has_marker = find_eog(&self.tokens, &eog, b"<|tool_response>").is_some()
            || find_eog(&self.tokens, &eog, b"<|plamo:eos|>").is_some();
        if let (true, Some(s_id)) = (has_marker, find_eog(&self.tokens, &eog, b"</s>")) {
            eog.remove(&s_id);
            self.tokens[s_id as usize].attr = attr::NORMAL;
        }
        self.eog = eog;
    }

    fn build_special_cache(&mut self) {
        let mut cache: Vec<u32> = (0..self.tokens.len() as u32)
            .filter(|&id| self.tokens[id as usize].attr & (attr::CONTROL | attr::USER_DEFINED | attr::UNKNOWN) != 0)
            .collect();
        let tokens = &self.tokens;
        cxx::sort(&mut cache, &|&a: &u32, &b: &u32| tokens[a as usize].text.len() > tokens[b as usize].text.len());
        self.special_cache = cache;
    }

    /// The per-token attributes llama.cpp sets by model, pre-tokenizer or architecture name.
    fn apply_name_attrs(&mut self, meta: &BTreeMap<String, MetaValue>) {
        let name = meta_str(meta, "general.name").unwrap_or("").to_ascii_lowercase();
        let pre = meta_str(meta, "tokenizer.ggml.pre").unwrap_or("");
        let arch = meta_str(meta, "general.architecture").unwrap_or("");
        let any = |s: &str, subs: &[&str]| subs.iter().any(|x| s.contains(x));
        let set = |v: &mut Vocab, text: &[u8], a: u32, on: bool| {
            if let Some(&id) = v.token_to_id.get(text) {
                let t = &mut v.tokens[id as usize];
                t.attr = if on { t.attr | a } else { t.attr & !a };
            }
        };
        if any(pre, &["jina-v2-de", "jina-v2-es", "jina-v2-code", "mmbert"]) || any(arch, &["nomic-bert-moe", "jina-bert-v3"]) {
            set(self, b"<mask>", attr::LSTRIP, true);
        } else if any(&name, &["phi-3", "phi3"]) {
            for id in self.special_cache.clone() {
                self.tokens[id as usize].attr |= attr::RSTRIP;
            }
            set(self, b"</s>", attr::RSTRIP, true);
            for t in [b"<unk>".as_slice(), b"<s>", b"<|endoftext|>"] {
                set(self, t, attr::RSTRIP, false);
            }
        } else if any(&name, &["modern-bert"]) {
            set(self, b"[MASK]", attr::LSTRIP, true);
        }
    }

    pub fn n_tokens(&self) -> usize {
        self.tokens.len()
    }

    pub fn token(&self, id: u32) -> &TokenData {
        &self.tokens[id as usize]
    }

    pub fn text_to_token(&self, text: &[u8]) -> Option<u32> {
        self.token_to_id.get(text).copied()
    }

    /// Whether `id` ends generation.
    pub fn is_eog(&self, id: u32) -> bool {
        self.eog.contains(&id)
    }

    /// The end-of-generation tokens, ascending.
    pub fn eog(&self) -> impl Iterator<Item = u32> + '_ {
        self.eog.iter().copied()
    }

    /// The special tokens in the order fragments are split on them.
    pub fn special_order(&self) -> &[u32] {
        &self.special_cache
    }

    fn bpe_rank(&self, first: &[u8], second: &[u8]) -> Option<u32> {
        self.bpe_ranks.get(&pair_key(first, second)).copied()
    }

    /// `byte_to_token`: the token standing for one byte.
    pub fn byte_to_token(&self, b: u8) -> Result<u32, TokenizeError> {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let found = match self.kind {
            VocabKind::Spm => {
                let name = [b'<', b'0', b'x', HEX[(b >> 4) as usize], HEX[(b & 15) as usize], b'>'];
                self.text_to_token(&name).or_else(|| {
                    let raw: &[u8] = if b == 0 { b"" } else { std::slice::from_ref(&b) };
                    self.text_to_token(raw)
                })
            }
            VocabKind::Bpe(_) => {
                let mut s = Vec::new();
                unicode::byte_to_utf8(b, &mut s);
                self.text_to_token(&s)
            }
        };
        found.ok_or(TokenizeError::MissingByteToken(b))
    }

    /// `token_to_byte`: the byte a byte token stands for (`<0xXX>`, the hex read as `strtol` reads it).
    fn token_to_byte(&self, id: u32) -> u8 {
        let t = &self.tokens[id as usize].text;
        let hex = t.get(3..t.len().min(5)).unwrap_or(&[]);
        let mut v: u32 = 0;
        for &c in hex {
            match (c as char).to_digit(16) {
                Some(d) => v = v * 16 + d,
                None => break,
            }
        }
        v as u8
    }

    /// `tokenizer_st_partition`: split the raw fragments of `text` at every special token, longest first.
    fn partition(&self, text: &[u8], parse_special: bool) -> Vec<Fragment> {
        let mut frags = vec![Fragment::Raw { offset: 0, len: text.len() }];
        for &id in &self.special_cache {
            let data = &self.tokens[id as usize];
            if !parse_special && data.attr & (attr::CONTROL | attr::UNKNOWN) != 0 {
                continue;
            }
            let needle = &data.text;
            let mut next = Vec::with_capacity(frags.len());
            for f in frags {
                let Fragment::Raw { offset, len } = f else {
                    next.push(f);
                    continue;
                };
                let (mut base, mut blen) = (offset, len);
                loop {
                    let Some(m) = find(&text[..base + blen], needle, base) else {
                        next.push(Fragment::Raw { offset: base, len: blen });
                        break;
                    };
                    if m > base {
                        let mut left = m - base;
                        if data.attr & attr::LSTRIP != 0 {
                            while left > 0 && c_isspace(text[base + left - 1]) {
                                left -= 1;
                            }
                        }
                        if left > 0 {
                            next.push(Fragment::Raw { offset: base, len: left });
                        }
                    }
                    next.push(Fragment::Token(id));
                    if m + needle.len() < base + blen {
                        let mut r_off = m + needle.len();
                        let mut r_len = blen - ((m - base) + needle.len());
                        if data.attr & attr::RSTRIP != 0 {
                            while r_len > 0 && c_isspace(text[r_off]) {
                                r_off += 1;
                                r_len -= 1;
                            }
                        }
                        if r_len == 0 {
                            break;
                        }
                        base = r_off;
                        blen = r_len;
                    } else {
                        break;
                    }
                }
            }
            frags = next;
        }
        frags
    }

    /// `llama_tokenize`: the token ids of `text`. `add_special` adds the vocabulary's beginning- and end-of-sequence
    /// tokens as it asks; `parse_special` lets control tokens written in the text match as themselves.
    pub fn tokenize(&self, text: &[u8], add_special: bool, parse_special: bool) -> Result<Vec<u32>, TokenizeError> {
        let mut out = Vec::new();
        let frags = if text.is_empty() { Vec::new() } else { self.partition(text, parse_special) };
        match self.kind {
            VocabKind::Spm => {
                let mut prev_special = true;
                if add_special && self.add_bos {
                    out.push(self.specials.bos.ok_or(TokenizeError::MissingSpecial("beginning-of-sequence"))?);
                    prev_special = true;
                }
                for f in &frags {
                    match *f {
                        Fragment::Raw { offset, len } => {
                            let mut t = Vec::with_capacity(len + 1);
                            if self.add_space_prefix && prev_special {
                                t.push(b' ');
                            }
                            t.extend_from_slice(&text[offset..offset + len]);
                            let t = replace_all(&t, b" ", ESCAPED_SPACE);
                            self.spm(&t, &mut out)?;
                            prev_special = false;
                        }
                        Fragment::Token(id) => {
                            out.push(id);
                            prev_special = true;
                        }
                    }
                }
                if add_special && self.add_eos {
                    out.push(self.specials.eos.ok_or(TokenizeError::MissingSpecial("end-of-sequence"))?);
                }
            }
            VocabKind::Bpe(_) => {
                if add_special && self.add_bos {
                    out.push(self.specials.bos.ok_or(TokenizeError::MissingSpecial("beginning-of-sequence"))?);
                }
                for f in &frags {
                    match *f {
                        Fragment::Raw { offset, len } => {
                            let raw = &text[offset..offset + len];
                            let t = if self.escape_whitespaces { replace_all(raw, b" ", ESCAPED_SPACE) } else { raw.to_vec() };
                            match self.session {
                                BpeSession::Plain => self.bpe(&t, &mut out)?,
                                BpeSession::HybridDna => self.bpe_dna(&t, &mut out)?,
                                BpeSession::Whitespace => self.bpe_whitespace(&t, &mut out)?,
                            }
                        }
                        Fragment::Token(id) => out.push(id),
                    }
                }
                if add_special && self.add_eos {
                    out.push(self.specials.eos.ok_or(TokenizeError::MissingSpecial("end-of-sequence"))?);
                }
            }
        }
        Ok(out)
    }

    /// The SentencePiece merge loop over one escaped fragment (`llm_tokenizer_spm_session`).
    fn spm(&self, text: &[u8], out: &mut Vec<u32>) -> Result<(), TokenizeError> {
        #[derive(Clone, Copy)]
        struct Sym {
            prev: isize,
            next: isize,
            start: usize,
            n: usize,
        }
        #[derive(PartialEq)]
        struct Bigram {
            score: f32,
            left: usize,
            right: usize,
            size: usize,
        }
        impl Eq for Bigram {}
        impl PartialOrd for Bigram {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Bigram {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                let less = |l: &Bigram, r: &Bigram| l.score < r.score || (l.score == r.score && l.left > r.left);
                if less(self, o) {
                    std::cmp::Ordering::Less
                } else if less(o, self) {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            }
        }
        let mut syms: Vec<Sym> = Vec::new();
        let mut offs = 0;
        while offs < text.len() {
            let n = unicode::len_utf8(text[offs]).min(text.len() - offs);
            let idx = syms.len() as isize;
            offs += n;
            syms.push(Sym { prev: idx - 1, next: if offs == text.len() { -1 } else { idx + 1 }, start: offs - n, n });
        }
        let mut queue = BinaryHeap::new();
        let try_add = |syms: &[Sym], queue: &mut BinaryHeap<Bigram>, left: isize, right: isize| {
            if left < 0 || right < 0 {
                return;
            }
            let (l, r) = (syms[left as usize], syms[right as usize]);
            let piece = &text[l.start..l.start + l.n + r.n];
            if let Some(id) = self.text_to_token(piece) {
                queue.push(Bigram { score: self.tokens[id as usize].score, left: left as usize, right: right as usize, size: piece.len() });
            }
        };
        for i in 1..syms.len() {
            try_add(&syms, &mut queue, i as isize - 1, i as isize);
        }
        while let Some(b) = queue.pop() {
            let (l, r) = (syms[b.left], syms[b.right]);
            if l.n == 0 || r.n == 0 || l.n + r.n != b.size {
                continue;
            }
            syms[b.left].n += r.n;
            syms[b.right].n = 0;
            syms[b.left].next = r.next;
            if r.next >= 0 {
                syms[r.next as usize].prev = b.left as isize;
            }
            let prev = syms[b.left].prev;
            let next = syms[b.left].next;
            try_add(&syms, &mut queue, prev, b.left as isize);
            try_add(&syms, &mut queue, b.left as isize, next);
        }
        let mut i: isize = if syms.is_empty() { -1 } else { 0 };
        while i != -1 {
            let s = syms[i as usize];
            let piece = &text[s.start..s.start + s.n];
            // A merged symbol is always a token, so a symbol that is not one is a single unmerged character.
            match self.text_to_token(piece) {
                Some(id) => out.push(id),
                None => {
                    for &b in piece {
                        out.push(self.byte_to_token(b)?);
                    }
                }
            }
            i = s.next;
        }
        Ok(())
    }

    /// The byte-pair merge loop over one fragment (`llm_tokenizer_bpe_session::tokenize`).
    fn bpe(&self, text: &[u8], out: &mut Vec<u32>) -> Result<(), TokenizeError> {
        let VocabKind::Bpe(pre) = self.kind else { unreachable!("a byte-pair session runs a byte-pair vocabulary") };
        let splitter = self.splitter.as_ref().expect("a byte-pair vocabulary has a splitter");
        let mut words = splitter.split(text)?;
        if pre == PreType::Mmbert {
            let mut split = Vec::new();
            for mut w in words {
                if w.iter().all(|&c| c == b'\n') {
                    split.push(w);
                    continue;
                }
                if !w.starts_with(ESCAPED_SPACE) {
                    let mut p = ESCAPED_SPACE.to_vec();
                    p.extend_from_slice(&w);
                    w = p;
                }
                let mut start = 0;
                while start < w.len() {
                    let end = find(&w, ESCAPED_SPACE, start + ESCAPED_SPACE.len()).unwrap_or(w.len());
                    split.push(w[start..end].to_vec());
                    start = end;
                }
            }
            words = split;
        }
        let byte_encode = pre.byte_encode();
        for word in &words {
            for piece in self.bpe_word(word, pre) {
                match self.text_to_token(piece) {
                    Some(id) => out.push(id),
                    None => {
                        for &b in piece {
                            let id = if byte_encode {
                                self.text_to_token(std::slice::from_ref(&b))
                            } else {
                                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                                self.text_to_token(&[b'<', b'0', b'x', HEX[(b >> 4) as usize], HEX[(b & 15) as usize], b'>'])
                            };
                            if let Some(id) = id {
                                out.push(id);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The pieces one word merges into.
    fn bpe_word<'w>(&self, word: &'w [u8], pre: PreType) -> Vec<&'w [u8]> {
        #[derive(Clone, Copy)]
        struct Sym {
            prev: isize,
            next: isize,
            start: usize,
            n: usize,
        }
        #[derive(PartialEq, Eq)]
        struct Bigram {
            rank: u32,
            left: usize,
            right: usize,
            text: Vec<u8>,
        }
        impl PartialOrd for Bigram {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Bigram {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                (o.rank, o.left).cmp(&(self.rank, self.left))
            }
        }
        let mut syms: Vec<Sym> = Vec::new();
        let mut offset = 0;
        if self.ignore_merges && self.text_to_token(word).is_some() {
            syms.push(Sym { prev: -1, next: -1, start: 0, n: word.len() });
            offset = word.len();
        } else if matches!(pre, PreType::Gemma4 | PreType::Mmbert)
            && word.iter().all(|&c| c == b'\n')
            && self.text_to_token(word).is_some()
        {
            syms.push(Sym { prev: -1, next: -1, start: 0, n: word.len() });
            offset = word.len();
        }
        while offset < word.len() {
            let n = (word.len() - offset).min(unicode::len_utf8(word[offset]));
            let idx = syms.len() as isize;
            offset += n;
            syms.push(Sym { prev: idx - 1, next: if offset == word.len() { -1 } else { idx + 1 }, start: offset - n, n });
        }
        let mut queue: BinaryHeap<Bigram> = BinaryHeap::new();
        let add = |syms: &[Sym], queue: &mut BinaryHeap<Bigram>, left: isize, right: isize| {
            if left < 0 || right < 0 {
                return;
            }
            let (l, r) = (syms[left as usize], syms[right as usize]);
            let (lt, rt) = (&word[l.start..l.start + l.n], &word[r.start..r.start + r.n]);
            if let Some(rank) = self.bpe_rank(lt, rt) {
                let mut text = lt.to_vec();
                text.extend_from_slice(rt);
                queue.push(Bigram { rank, left: left as usize, right: right as usize, text });
            }
        };
        for i in 1..syms.len() {
            add(&syms, &mut queue, i as isize - 1, i as isize);
        }
        while let Some(b) = queue.pop() {
            let (l, r) = (syms[b.left], syms[b.right]);
            if l.n == 0 || r.n == 0 {
                continue;
            }
            let joined_now = &word[l.start..l.start + l.n].iter().chain(&word[r.start..r.start + r.n]).copied().collect::<Vec<u8>>();
            if *joined_now != b.text {
                continue;
            }
            syms[b.left].n += r.n;
            syms[b.right].n = 0;
            syms[b.left].next = r.next;
            if r.next >= 0 {
                syms[r.next as usize].prev = b.left as isize;
            }
            let prev = syms[b.left].prev;
            let next = syms[b.left].next;
            add(&syms, &mut queue, prev, b.left as isize);
            add(&syms, &mut queue, b.left as isize, next);
        }
        syms.iter().filter(|s| s.n > 0).map(|s| &word[s.start..s.start + s.n]).collect()
    }

    /// The `hybriddna` session: `<dna>…</dna>` spans become k-mer tokens, the rest merges as byte pairs.
    fn bpe_dna(&self, text: &[u8], out: &mut Vec<u32>) -> Result<(), TokenizeError> {
        let (Some(open), Some(close), Some(oov)) =
            (self.text_to_token(b"<dna>"), self.text_to_token(b"</dna>"), self.text_to_token(b"<oov>"))
        else {
            return self.bpe(text, out);
        };
        const K: usize = 6;
        let mut pos = 0;
        while pos < text.len() {
            let Some(start) = find(text, b"<dna>", pos) else {
                self.bpe(&text[pos..], out)?;
                break;
            };
            if start > pos {
                self.bpe(&text[pos..start], out)?;
            }
            out.push(open);
            let content = start + 5;
            let end = find(text, b"</dna>", content);
            let seq: Vec<u8> = text[content..end.unwrap_or(text.len())].iter().map(|c| c.to_ascii_uppercase()).collect();
            let kmer = |k: &[u8]| {
                let mut t = k.to_vec();
                t.extend_from_slice(DNA_KMER_MARKER);
                self.text_to_token(&t).unwrap_or(oov)
            };
            let mut i = 0;
            while i + K <= seq.len() {
                out.push(kmer(&seq[i..i + K]));
                i += K;
            }
            if i < seq.len() {
                let mut tail = seq[i..].to_vec();
                tail.resize(K, b'A');
                out.push(kmer(&tail));
            }
            let Some(end) = end else { break };
            out.push(close);
            pos = end + 6;
        }
        Ok(())
    }

    /// The `whitespace` session: whitespace-separated segments, lowercased when the normalizer says so.
    fn bpe_whitespace(&self, text: &[u8], out: &mut Vec<u32>) -> Result<(), TokenizeError> {
        let mut seg = Vec::new();
        for c in unicode::cpts_from_utf8(text) {
            if unicode::flags(c).is_whitespace() {
                if !seg.is_empty() {
                    self.bpe(&seg, out)?;
                    seg.clear();
                }
            } else {
                unicode::push_utf8(if self.lowercase { unicode::to_lower(c) } else { c }, &mut seg)?;
            }
        }
        if !seg.is_empty() {
            self.bpe(&seg, out)?;
        }
        Ok(())
    }

    /// The text of a token as `token_to_piece` renders it with `special` on and no strip (the cached piece).
    fn piece_uncached(&self, id: u32) -> Vec<u8> {
        let t = &self.tokens[id as usize];
        let a = t.attr;
        if a & (attr::UNKNOWN | attr::CONTROL | attr::USER_DEFINED) != 0 {
            return t.text.clone();
        }
        if a & attr::NORMAL != 0 {
            return match self.kind {
                VocabKind::Spm => replace_all(&t.text, ESCAPED_SPACE, b" "),
                VocabKind::Bpe(_) if self.escape_whitespaces => replace_all(&t.text, ESCAPED_SPACE, b" "),
                VocabKind::Bpe(_) => decode_text(&t.text),
            };
        }
        if a & attr::BYTE != 0 {
            return vec![self.token_to_byte(id)];
        }
        Vec::new()
    }

    /// `llama_token_to_piece`: the text of `id`, up to `lstrip` leading spaces removed; a control or unknown token
    /// renders only when `special` is on.
    pub fn token_to_piece(&self, id: u32, lstrip: usize, special: bool) -> &[u8] {
        if !special && self.tokens[id as usize].attr & (attr::UNKNOWN | attr::CONTROL) != 0 {
            return &[];
        }
        let p = &self.piece_cache[id as usize];
        let strip = p.iter().take(lstrip).take_while(|&&c| c == b' ').count();
        &p[strip..]
    }

    /// `llama_detokenize`: the text of `tokens`. `remove_special` drops the leading BOS and trailing EOS the
    /// vocabulary adds; `unparse_special` renders control tokens.
    pub fn detokenize(&self, tokens: &[u32], remove_special: bool, unparse_special: bool) -> Vec<u8> {
        let mut toks = tokens;
        let mut remove_space = self.add_space_prefix;
        if remove_special && self.add_bos && !toks.is_empty() && Some(toks[0]) == self.specials.bos {
            remove_space = false;
            toks = &toks[1..];
        }
        if remove_special && self.add_eos && !toks.is_empty() && Some(toks[toks.len() - 1]) == self.specials.eos {
            toks = &toks[..toks.len() - 1];
        }
        let mut text = Vec::new();
        for &t in toks {
            text.extend_from_slice(self.token_to_piece(t, remove_space as usize, unparse_special));
            remove_space = false;
        }
        if self.clean_spaces {
            clean_spaces(&mut text);
        }
        text
    }
}

/// `llama_decode_text`: a byte-level token back to its bytes; a codepoint outside the byte alphabet is written as
/// llama.cpp writes it, `[UNK_BYTE_0x…<token>]`.
fn decode_text(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for c in unicode::cpts_from_utf8(text) {
        match unicode::utf8_to_byte(c) {
            Some(b) => out.push(b),
            None => {
                let mut u = Vec::new();
                if unicode::push_utf8(c, &mut u).is_err() {
                    continue;
                }
                out.extend_from_slice(b"[UNK_BYTE_0x");
                for b in &u {
                    out.extend_from_slice(format!("{b:02x}").as_bytes());
                }
                out.extend_from_slice(text);
                out.push(b']');
            }
        }
    }
    out
}

/// The three `clean_up_tokenization_spaces` passes of `llama_detokenize`, run in place over the buffer exactly as
/// llama.cpp runs them (a pass reads characters an earlier step of the same pass rewrote).
fn clean_spaces(text: &mut Vec<u8>) {
    let mut total = text.len();
    let total1 = total;
    total = if total > 0 { 1 } else { 0 };
    for i in 1..total1 {
        let x = text[i];
        if text[i - 1] == b' ' && matches!(x, b'?' | b'!' | b'.' | b',') {
            total -= 1;
        }
        text[total] = x;
        total += 1;
    }
    let total2 = total;
    total = if total > 0 { 1 } else { 0 };
    let mut i = 1;
    while i < total2 {
        let x = text[i];
        if x == b'\'' && i + 1 < total2 && text[i - 1] == b' ' && text[i + 1] == b' ' {
            total -= 1;
            i += 1;
            text[i] = 0;
        }
        text[total] = x;
        total += 1;
        i += 1;
    }
    let total3 = total;
    total = if total > 0 { 1 } else { 0 };
    for i in 1..total3 {
        let x = text[i];
        if text[i - 1] == b' ' && x == b'\'' && i + 1 < total3 {
            let x1 = text[i + 1];
            if x1 == b't' || x1 == b'd' {
            } else if x1 == b's' || x1 == b'm' {
                total -= 1;
            } else if i + 2 < total3 {
                let x2 = text[i + 2];
                if (x1 == b'r' && x2 == b'e') || (x1 == b'v' && x2 == b'e') {
                    total -= 1;
                }
            }
        }
        text[total] = x;
        total += 1;
    }
    text.truncate(total);
}
