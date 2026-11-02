//! The tokenizer (`model::tokenizer`) held equal to llama.cpp's — the code llama.cpp runs, not a description of it.
//!
//! Two oracle programs are compiled against the pinned llama.cpp (`external/competitors/llama.cpp`): one links its
//! `src/unicode.cpp` and answers codepoint flags, the lowercase map, `unicode_regex_split` for any pattern list,
//! libstdc++'s regex classes and `std::sort`; the other links the built `libllama` and loads a vocabulary
//! (`vocab_only`) to tokenize, render pieces, detokenize and report every token's attributes. Every locked fact is
//! compared over its whole domain (every codepoint) or over seeded random text drawn from every script class the
//! splitters branch on, malformed UTF-8 and the vocabulary's own special tokens included.
//!
//! The bundled vocabulary corpora (`models/ggml-vocab-*.gguf` with `.inp`/`.out`) carry Hugging Face's own
//! tokenizations; llama.cpp passes all of them, and so must we.

mod common;

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::tokenizer::pre::{pre_type, PreType};
use logicaffeine_compile::model::tokenizer::regex::{Regex, Width};
use logicaffeine_compile::model::tokenizer::split::Splitter;
use logicaffeine_compile::model::tokenizer::{attr, cxx, unicode, TokenizeError, Vocab, VocabError, VocabKind};

const UNICODE_ORACLE: &str = r#"
#include "unicode.h"
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <exception>
#include <regex>
#include <string>
#include <unordered_map>
#include <vector>

static bool rd(void * p, size_t n) { return fread(p, 1, n, stdin) == n; }
static void wr(const void * p, size_t n) { fwrite(p, 1, n, stdout); }
static void wr32(uint32_t v) { wr(&v, 4); }
static bool rdstr(std::string & s) {
    uint32_t n;
    if (!rd(&n, 4)) return false;
    s.resize(n);
    return n == 0 || rd(&s[0], n);
}

int main(int argc, char ** argv) {
    const std::string mode = argc > 1 ? argv[1] : "";
    if (mode == "flags") {
        for (uint32_t c = 0; c < 0x110000; c++) { uint16_t f = unicode_cpt_flags_from_cpt(c).as_uint(); wr(&f, 2); }
        return 0;
    }
    if (mode == "lower") {
        for (uint32_t c = 0; c < 0x110000; c++) wr32(unicode_tolower(c));
        return 0;
    }
    if (mode == "classes") {
        const char * pats[] = {"\\s", "\\S", "\\d", "\\D", "\\w", "\\W", "."};
        for (const char * p : pats) {
            std::regex re(p, std::regex_constants::optimize | std::regex_constants::nosubs);
            for (int c = 0; c < 256; c++) { std::string s(1, (char) c); uint8_t m = std::regex_match(s, re); wr(&m, 1); }
            std::wstring wp(p, p + std::char_traits<char>::length(p));
            std::wregex wre(wp, std::regex_constants::optimize | std::regex_constants::nosubs);
            for (uint32_t c = 0; c < 0x110000; c++) { std::wstring s(1, (wchar_t) c); uint8_t m = std::regex_match(s, wre); wr(&m, 1); }
        }
        return 0;
    }
    if (mode == "sort") {
        std::vector<uint32_t> lens;
        while (true) {
            uint32_t n;
            if (!rd(&n, 4)) break;
            lens.resize(n);
            if (n) rd(lens.data(), 4 * n);
            std::vector<uint32_t> ids(n);
            for (uint32_t i = 0; i < n; i++) ids[i] = i;
            std::sort(ids.begin(), ids.end(), [&](uint32_t a, uint32_t b) { return lens[a] > lens[b]; });
            wr(ids.data(), 4 * n);
        }
        return 0;
    }
    if (mode == "umap") {
        std::string key;
        while (true) {
            uint32_t n;
            if (!rd(&n, 4)) break;
            std::unordered_map<std::string, int> m;
            for (uint32_t i = 0; i < n; i++) { rdstr(key); m[key] = (int) i; }
            for (const auto & kv : m) wr32((uint32_t) kv.second);
        }
        return 0;
    }
    if (mode == "hash") {
        std::string key;
        while (rdstr(key)) { uint64_t h = std::hash<std::string>{}(key); wr(&h, 8); }
        return 0;
    }
    if (mode == "split") {
        uint32_t nre;
        if (!rd(&nre, 4)) return 3;
        std::vector<std::string> res(nre);
        for (auto & r : res) rdstr(r);
        uint8_t be;
        rd(&be, 1);
        std::string text;
        while (rdstr(text)) {
            try {
                auto words = unicode_regex_split(text, res, be != 0);
                uint8_t ok = 1; wr(&ok, 1);
                wr32(words.size());
                for (auto & w : words) { wr32(w.size()); wr(w.data(), w.size()); }
            } catch (const std::exception & e) {
                uint8_t ok = 0; wr(&ok, 1);
                std::string m = e.what(); wr32(m.size()); wr(m.data(), m.size());
            }
        }
        return 0;
    }
    return 2;
}
"#;

const VOCAB_ORACLE: &str = r#"
#include "llama.h"
#include <cstdint>
#include <cstdio>
#include <exception>
#include <string>
#include <vector>

static bool rd(void * p, size_t n) { return fread(p, 1, n, stdin) == n; }
static void wr(const void * p, size_t n) { fwrite(p, 1, n, stdout); }
static void wr32(uint32_t v) { wr(&v, 4); }
static void wrs(const std::string & s) { wr32(s.size()); wr(s.data(), s.size()); }
static bool rdstr(std::string & s) {
    uint32_t n;
    if (!rd(&n, 4)) return false;
    s.resize(n);
    return n == 0 || rd(&s[0], n);
}

int main(int argc, char ** argv) {
    llama_log_set([](enum ggml_log_level, const char *, void *) {}, nullptr);
    auto mp = llama_model_default_params();
    mp.vocab_only = true;
    llama_model * model = llama_model_load_from_file(argv[1], mp);
    if (!model) { fprintf(stderr, "the vocabulary does not load\n"); return 3; }
    const llama_vocab * v = llama_model_get_vocab(model);
    uint8_t op;
    while (rd(&op, 1)) {
        if (op == 1) {
            uint8_t add_special, parse_special;
            rd(&add_special, 1); rd(&parse_special, 1);
            std::string text; rdstr(text);
            try {
                int n = llama_tokenize(v, text.data(), (int) text.size(), nullptr, 0, add_special, parse_special);
                std::vector<llama_token> toks(n < 0 ? -n : n);
                if (n < 0) n = llama_tokenize(v, text.data(), (int) text.size(), toks.data(), (int) toks.size(), add_special, parse_special);
                uint8_t ok = 1; wr(&ok, 1);
                wr32(toks.size()); wr(toks.data(), 4 * toks.size());
            } catch (const std::exception & e) {
                uint8_t ok = 0; wr(&ok, 1); wrs(e.what());
            }
        } else if (op == 2) {
            int32_t tok, lstrip; uint8_t special;
            rd(&tok, 4); rd(&lstrip, 4); rd(&special, 1);
            std::vector<char> buf(256);
            int n = llama_token_to_piece(v, tok, buf.data(), (int) buf.size(), lstrip, special);
            if (n < 0) { buf.resize(-n); n = llama_token_to_piece(v, tok, buf.data(), (int) buf.size(), lstrip, special); }
            wrs(std::string(buf.data(), n));
        } else if (op == 3) {
            uint8_t remove_special, unparse_special; uint32_t n;
            rd(&remove_special, 1); rd(&unparse_special, 1); rd(&n, 4);
            std::vector<llama_token> toks(n);
            if (n) rd(toks.data(), 4 * n);
            std::vector<char> buf(1024);
            int m = llama_detokenize(v, toks.data(), (int) n, buf.data(), (int) buf.size(), remove_special, unparse_special);
            if (m < 0) { buf.resize(-m); m = llama_detokenize(v, toks.data(), (int) n, buf.data(), (int) buf.size(), remove_special, unparse_special); }
            wrs(std::string(buf.data(), m));
        } else if (op == 4) {
            const int32_t n = llama_vocab_n_tokens(v);
            wr32(n);
            const int32_t ids[] = {llama_vocab_bos(v), llama_vocab_eos(v), llama_vocab_eot(v), llama_vocab_sep(v),
                llama_vocab_nl(v), llama_vocab_pad(v), llama_vocab_mask(v), llama_vocab_fim_pre(v),
                llama_vocab_fim_suf(v), llama_vocab_fim_mid(v), llama_vocab_fim_pad(v), llama_vocab_fim_rep(v),
                llama_vocab_fim_sep(v)};
            wr(ids, sizeof ids);
            uint8_t flags[2] = {(uint8_t) llama_vocab_get_add_bos(v), (uint8_t) llama_vocab_get_add_eos(v)};
            wr(flags, 2);
            for (int32_t i = 0; i < n; i++) {
                uint32_t a = llama_vocab_get_attr(v, i);
                float s = llama_vocab_get_score(v, i);
                uint8_t eog = llama_vocab_is_eog(v, i);
                wr(&a, 4); wr(&s, 4); wr(&eog, 1);
                wrs(llama_vocab_get_text(v, i));
            }
        }
        fflush(stdout);
    }
    llama_model_free(model);
    return 0;
}
"#;

fn pin_commit() -> String {
    let pins = std::fs::read_to_string(common::llama_cpp_root().join("../PINS.tsv")).expect("PINS.tsv");
    pins.lines()
        .find(|l| l.starts_with("llama.cpp\t"))
        .and_then(|l| l.split('\t').nth(3))
        .expect("llama.cpp is pinned")
        .to_string()
}

/// Compile an oracle once per (source, flags, pin) into the test scratch directory, installing it atomically.
fn build_oracle(name: &str, source: &str, args: &[String]) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (source, args, pin_commit()).hash(&mut h);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tokenizer-oracles");
    std::fs::create_dir_all(&dir).expect("oracle directory");
    let bin = dir.join(format!("{name}-{:016x}", h.finish()));
    if bin.is_file() {
        return bin;
    }
    let pid = std::process::id();
    let src = dir.join(format!("{name}-{pid}.cpp"));
    std::fs::write(&src, source).expect("oracle source");
    let tmp = dir.join(format!("{name}-{pid}.bin"));
    let out = Command::new("g++")
        .args(["-O2", "-std=c++17"])
        .arg(&src)
        .args(args)
        .arg("-o")
        .arg(&tmp)
        .output()
        .expect("g++ runs");
    assert!(out.status.success(), "the {name} oracle does not compile:\n{}", String::from_utf8_lossy(&out.stderr));
    std::fs::rename(&tmp, &bin).expect("install the oracle");
    let _ = std::fs::remove_file(&src);
    bin
}

fn unicode_oracle() -> PathBuf {
    let src = common::llama_cpp_root().join("src");
    build_oracle(
        "unicode",
        UNICODE_ORACLE,
        &[
            format!("-I{}", src.display()),
            src.join("unicode.cpp").display().to_string(),
            src.join("unicode-data.cpp").display().to_string(),
        ],
    )
}

fn vocab_oracle() -> PathBuf {
    let root = common::llama_cpp_root();
    let lib = root.join("build/bin");
    build_oracle(
        "vocab",
        VOCAB_ORACLE,
        &[
            format!("-I{}", root.join("include").display()),
            format!("-I{}", root.join("ggml/include").display()),
            format!("-L{}", lib.display()),
            "-lllama".into(),
            format!("-Wl,-rpath,{}", lib.display()),
        ],
    )
}

/// Run an oracle over `input`, feeding stdin from a thread so a large answer cannot deadlock the pipe.
fn run_oracle(bin: &Path, args: &[&str], input: Vec<u8>) -> Vec<u8> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the oracle starts");
    let mut stdin = child.stdin.take().expect("stdin");
    let feeder = std::thread::spawn(move || {
        stdin.write_all(&input).expect("feed the oracle");
    });
    let mut out = Vec::new();
    child.stdout.take().expect("stdout").read_to_end(&mut out).expect("read the oracle");
    feeder.join().expect("the feeder");
    let status = child.wait().expect("the oracle exits");
    let mut err = String::new();
    child.stderr.take().map(|mut e| e.read_to_string(&mut err));
    assert!(status.success(), "the oracle {} {args:?} failed: {err}", bin.display());
    out
}

struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn new(b: &'a [u8]) -> Self {
        Cursor { b, i: 0 }
    }
    fn u8(&mut self) -> u8 {
        self.i += 1;
        self.b[self.i - 1]
    }
    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.b[self.i..self.i + 4].try_into().unwrap());
        self.i += 4;
        v
    }
    fn i32(&mut self) -> i32 {
        self.u32() as i32
    }
    fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }
    fn bytes(&mut self) -> &'a [u8] {
        let n = self.u32() as usize;
        self.i += n;
        &self.b[self.i - n..self.i]
    }
    fn done(&self) -> bool {
        self.i == self.b.len()
    }
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_u32(out, b.len() as u32);
    out.extend_from_slice(b);
}

fn oracles_ready() -> bool {
    common::require_tool("competitor-sources") && common::require_tool("g++")
}

fn vocab_oracle_ready() -> bool {
    oracles_ready() && common::require_tool("llama-cpp-build")
}

// ---------------------------------------------------------------- seeded text

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + self.below((hi - lo + 1) as u64) as u32
    }
}

/// UTF-8 of any codepoint below 0x110000, surrogates included (they encode as their three bytes).
fn push_cpt(out: &mut Vec<u8>, c: u32) {
    unicode::push_utf8(c, out).expect("a codepoint below 0x110000");
}

/// Random text drawn from every class the splitters branch on, the vocabulary's special tokens among them.
fn seeded_text(rng: &mut Rng, specials: &[Vec<u8>]) -> Vec<u8> {
    const WS: [u32; 12] = [0x20, 0x09, 0x0A, 0x0D, 0x0B, 0x0C, 0xA0, 0x85, 0x2028, 0x3000, 0x2009, 0x1680];
    const CONTRACTIONS: [&str; 14] =
        ["'s", "'t", "'re", "'ve", "'m", "'ll", "'d", "'S", "'LL", "'Re", "'\u{17F}", "'x", "'", "''"];
    let len = rng.below(40);
    let mut out = Vec::new();
    for _ in 0..len {
        match rng.below(22) {
            0..=3 => out.push(rng.range(0x21, 0x7E) as u8),
            4 => {
                for _ in 0..rng.range(1, 8) {
                    out.push(if rng.below(2) == 0 { rng.range(b'a' as u32, b'z' as u32) } else { rng.range(b'A' as u32, b'Z' as u32) } as u8);
                }
            }
            5 => push_cpt(&mut out, WS[rng.below(WS.len() as u64) as usize]),
            6 => {
                for _ in 0..rng.range(1, 3) {
                    out.push(b' ');
                }
            }
            7 => {
                for _ in 0..rng.range(1, 11) {
                    out.push(rng.range(b'0' as u32, b'9' as u32) as u8);
                }
            }
            8 => out.extend_from_slice(CONTRACTIONS[rng.below(CONTRACTIONS.len() as u64) as usize].as_bytes()),
            9 => push_cpt(&mut out, rng.range(0xC0, 0x24F)),
            10 => push_cpt(&mut out, rng.range(0x300, 0x36F)),
            11 => push_cpt(&mut out, if rng.below(2) == 0 { rng.range(0x391, 0x3C9) } else { rng.range(0x400, 0x4FF) }),
            12 => push_cpt(
                &mut out,
                match rng.below(5) {
                    0 => rng.range(0x4E00, 0x9FFF),
                    1 => rng.range(0x3040, 0x30FF),
                    2 => rng.range(0xAC00, 0xD7A3),
                    3 => rng.range(0x900, 0x97F),
                    _ => rng.range(0xE00, 0xE7F),
                },
            ),
            13 => {
                push_cpt(&mut out, rng.range(0x1F300, 0x1FAFF));
                match rng.below(4) {
                    0 => push_cpt(&mut out, 0x200D),
                    1 => push_cpt(&mut out, 0xFE0F),
                    2 => push_cpt(&mut out, 0x200C),
                    _ => {}
                }
            }
            14 => push_cpt(
                &mut out,
                match rng.below(4) {
                    0 => rng.range(0x660, 0x669),
                    1 => rng.range(0xFF10, 0xFF19),
                    2 => [0xB2, 0xB3, 0xB9, 0xBC, 0x2153][rng.below(5) as usize],
                    _ => rng.range(0x2160, 0x2188),
                },
            ),
            15 => push_cpt(
                &mut out,
                match rng.below(4) {
                    0 => rng.range(0x2000, 0x206F),
                    1 => rng.range(0x20A0, 0x20C0),
                    2 => rng.range(0x2200, 0x22FF),
                    _ => rng.range(0xFF01, 0xFF0F),
                },
            ),
            16 if !specials.is_empty() => out.extend_from_slice(&specials[rng.below(specials.len() as u64) as usize]),
            17 => match rng.below(5) {
                0 => out.push(rng.range(0x80, 0xFF) as u8),
                1 => out.extend_from_slice(&[0xC3]),
                2 => out.extend_from_slice(&[0xE2, 0x82]),
                3 => out.extend_from_slice(&[0xF0, 0x9F, 0x98]),
                _ => out.extend_from_slice(&[0xC0, 0x80]),
            },
            18 => out.extend_from_slice([b"\r\n".as_slice(), b"\n\n", b" \n", b"\n ", b"\t\n", b"  \r"][rng.below(6) as usize]),
            19 => {
                let mut c = rng.range(0, 0x10FFFF);
                if c == 0 {
                    c = 1;
                }
                push_cpt(&mut out, c);
            }
            20 => push_cpt(&mut out, rng.range(0x2C00, 0x2DFF)),
            _ => out.extend_from_slice(b"<|"),
        }
    }
    out
}

// ---------------------------------------------------------------- always-on engine facts

#[test]
fn the_regex_engine_takes_the_first_match_in_priority_order() {
    let find = |p: &str, t: &str| {
        let re = Regex::new(p, Width::Wide).unwrap();
        let units: Vec<u32> = t.chars().map(|c| c as u32).collect();
        re.matches(&units)
    };
    assert_eq!(find("a|ab", "ab"), vec![(0, 1)], "the left alternative wins, not the longest");
    assert_eq!(find("ab|a", "ab"), vec![(0, 2)]);
    assert_eq!(find("a+", "aaa"), vec![(0, 3)], "greedy");
    assert_eq!(find("a+?", "aaa"), vec![(0, 1), (1, 1), (2, 1)], "lazy");
    assert_eq!(find("\\s+(?!\\S)|\\s+", "a   b"), vec![(1, 2), (3, 1)], "a whitespace run leaves its last space to the word");
    assert_eq!(find("\\s+$", "ab  "), vec![(2, 2)]);
    assert_eq!(find("\\d{1,3}", "12345"), vec![(0, 3), (3, 2)]);
    assert_eq!(find("x*", "ab"), vec![(0, 0), (1, 0), (2, 0)], "an empty match advances one unit");
    assert_eq!(find("(?=(\\d{3})+(?!\\d))", "1234567"), vec![(1, 0), (4, 0)], "lookahead-only matches");
    assert_eq!(find("\\bab", "ab ab xab"), vec![(0, 2), (3, 2)]);
    assert_eq!(find("[^a-z]+", "AB+cd"), vec![(0, 3)]);
    assert_eq!(find("[\\-.]+", "a-.-b"), vec![(1, 3)], "an escaped hyphen is a member, not a range");
    assert_eq!(find("[\\[-\\]]+", "x[\\]y"), vec![(1, 3)], "a range between escaped brackets");
}

#[test]
fn the_regex_engine_compares_narrow_units_as_signed_chars() {
    let re = Regex::from_units(&[b'[' as u32, 0xD2, b'A' as u32, b'-' as u32, b'Z' as u32, b']' as u32], "[\u{D2}A-Z]", Width::Narrow)
        .unwrap();
    assert_eq!(re.matches(&[0xD2, b'Q' as u32, 0xD1]), vec![(0, 1), (1, 1)], "0xD2 is a member; 0xD1 sorts below 'A'");
    let err = Regex::from_units(&[b'[' as u32, b'A' as u32, b'-' as u32, 0xD2, b']' as u32], "[A-\u{D2}]", Width::Narrow)
        .unwrap_err();
    assert_eq!(err.why, "a range whose start is above its end", "signed 0xD2 is below 'A'");
    assert!(Regex::new("[A-\u{D2}]", Width::Wide).is_ok(), "the same range is ascending over codepoints");
}

#[test]
fn the_regex_engine_refuses_what_libstdcxx_and_ecmascript_disagree_on() {
    for (p, why) in [
        ("(a*)*", "a quantified group that can match the empty string"),
        ("(a|)+", "a quantified group that can match the empty string"),
        ("(a)\\1", "a back-reference"),
        ("(?i:a)", "a group modifier other than (?: (?= (?!"),
        ("a{3,2}", "an interval's upper bound is below its lower bound"),
        ("[]", "an empty bracket expression"),
        ("*a", "a quantifier with nothing to repeat"),
    ] {
        assert_eq!(Regex::new(p, Width::Wide).unwrap_err().why, why, "{p}");
    }
}

#[test]
fn every_pre_tokenizer_compiles() {
    for pt in PreType::ALL {
        Splitter::new(pt.regexes(), pt.byte_encode()).unwrap_or_else(|e| panic!("{}: {e}", pt.llama_name()));
    }
    assert_eq!(PreType::ALL.iter().map(|p| p.llama_name()).collect::<BTreeSet<_>>().len(), PreType::ALL.len());
}

#[test]
fn the_libstdcxx_sort_port_sorts() {
    let mut rng = Rng(0x5eed_0001);
    for n in [0usize, 1, 2, 15, 16, 17, 33, 200, 1000] {
        let lens: Vec<u32> = (0..n).map(|_| rng.range(1, 6)).collect();
        let mut ids: Vec<u32> = (0..n as u32).collect();
        cxx::sort(&mut ids, &|&a: &u32, &b: &u32| lens[a as usize] > lens[b as usize]);
        assert!(ids.windows(2).all(|w| lens[w[0] as usize] >= lens[w[1] as usize]));
        assert_eq!(ids.iter().copied().collect::<BTreeSet<_>>(), (0..n as u32).collect());
    }
}

// ---------------------------------------------------------------- against llama.cpp's unicode.cpp

#[test]
fn every_codepoints_flags_equal_llama_cpps() {
    if !oracles_ready() {
        return;
    }
    let out = run_oracle(&unicode_oracle(), &["flags"], Vec::new());
    assert_eq!(out.len(), 0x110000 * 2);
    let mut wrong = Vec::new();
    for c in 0..0x110000u32 {
        let theirs = u16::from_le_bytes([out[2 * c as usize], out[2 * c as usize + 1]]);
        if unicode::flags(c).0 != theirs & unicode::Flags::KEPT || theirs & unicode::Flags::MASK_CATEGORIES == 0 {
            wrong.push((c, unicode::flags(c).0, theirs));
        }
    }
    assert!(wrong.is_empty(), "{} codepoints differ, first {:x?}", wrong.len(), &wrong[..wrong.len().min(8)]);
    assert_eq!(unicode::flags(0x110000).0, 1, "past the table every codepoint is undefined");
}

#[test]
fn every_codepoints_lowercase_equals_llama_cpps() {
    if !oracles_ready() {
        return;
    }
    let out = run_oracle(&unicode_oracle(), &["lower"], Vec::new());
    let wrong: Vec<u32> = (0..0x110000u32)
        .filter(|&c| unicode::to_lower(c) != u32::from_le_bytes(out[4 * c as usize..4 * c as usize + 4].try_into().unwrap()))
        .collect();
    assert!(wrong.is_empty(), "{} codepoints lowercase differently, first {:x?}", wrong.len(), &wrong[..wrong.len().min(8)]);
}

#[test]
fn the_engines_character_classes_are_libstdcxxs_for_every_unit() {
    if !oracles_ready() {
        return;
    }
    let out = run_oracle(&unicode_oracle(), &["classes"], Vec::new());
    let pats = ["\\s", "\\S", "\\d", "\\D", "\\w", "\\W", "."];
    let mut at = 0;
    for p in pats {
        let narrow = Regex::new(p, Width::Narrow).unwrap();
        for c in 0..256u32 {
            let ours = narrow.matches(&[c]) == vec![(0, 1)];
            assert_eq!(ours, out[at] != 0, "narrow {p} on 0x{c:02x}");
            at += 1;
        }
        let wide = Regex::new(p, Width::Wide).unwrap();
        for c in 0..0x110000u32 {
            let ours = wide.matches(&[c]) == vec![(0, 1)];
            assert_eq!(ours, out[at] != 0, "wide {p} on U+{c:04X}");
            at += 1;
        }
    }
    assert_eq!(at, out.len());
}

#[test]
fn the_sort_port_orders_ties_as_libstdcxx_does() {
    if !oracles_ready() {
        return;
    }
    let mut rng = Rng(0x5eed_0002);
    let mut cases = Vec::new();
    let mut input = Vec::new();
    for k in 0..400 {
        let n = match k % 4 {
            0 => rng.range(0, 40) as usize,
            1 => rng.range(17, 300) as usize,
            2 => rng.range(300, 3000) as usize,
            _ => rng.range(1, 17) as usize,
        };
        let distinct = rng.range(1, 12);
        let lens: Vec<u32> = (0..n).map(|_| rng.range(1, distinct)).collect();
        put_u32(&mut input, n as u32);
        for &l in &lens {
            put_u32(&mut input, l);
        }
        cases.push(lens);
    }
    let out = run_oracle(&unicode_oracle(), &["sort"], input);
    let mut c = Cursor::new(&out);
    for lens in cases {
        let theirs: Vec<u32> = (0..lens.len()).map(|_| c.u32()).collect();
        let mut ours: Vec<u32> = (0..lens.len() as u32).collect();
        cxx::sort(&mut ours, &|&a: &u32, &b: &u32| lens[a as usize] > lens[b as usize]);
        assert_eq!(ours, theirs, "n = {}", lens.len());
    }
    assert!(c.done());
}

#[test]
fn the_string_hash_is_libstdcxxs() {
    assert_eq!(cxx::hash_string(b""), 0x553e93901e462a6e);
    assert_eq!(cxx::hash_string(b"abc"), 0x32d82bf8ed3dba39);
    if !oracles_ready() {
        return;
    }
    let mut rng = Rng(0x5eed_0003);
    let keys: Vec<Vec<u8>> = (0..5000).map(|k| (0..(k % 41)).map(|_| rng.next() as u8).collect()).collect();
    let mut input = Vec::new();
    for k in &keys {
        put_bytes(&mut input, k);
    }
    let out = run_oracle(&unicode_oracle(), &["hash"], input);
    for (k, theirs) in keys.iter().zip(out.chunks_exact(8)) {
        assert_eq!(cxx::hash_string(k), u64::from_le_bytes(theirs.try_into().unwrap()), "{k:?}");
    }
}

#[test]
fn the_map_order_is_libstdcxxs_for_every_size_and_every_bundled_vocabulary() {
    if !oracles_ready() {
        return;
    }
    let mut rng = Rng(0x5eed_0004);
    let mut sets: Vec<Vec<Vec<u8>>> = Vec::new();
    for n in (0..60).chain([97, 211, 1000, 4096, 30000]) {
        let mut seen = BTreeSet::new();
        let mut keys = Vec::new();
        while keys.len() < n {
            let k: Vec<u8> = (0..rng.range(1, 12)).map(|_| rng.range(b'a' as u32, b'h' as u32) as u8).collect();
            if seen.insert(k.clone()) {
                keys.push(k);
            }
        }
        sets.push(keys);
    }
    if common::require_tool("llama-cpp-build") {
        for path in vocab_files() {
            if let Ok(v) = load(&path) {
                sets.push((0..v.n_tokens() as u32).map(|id| v.token(id).text.clone()).collect());
            }
        }
    }
    let mut input = Vec::new();
    for keys in &sets {
        put_u32(&mut input, keys.len() as u32);
        for k in keys {
            put_bytes(&mut input, k);
        }
    }
    let out = run_oracle(&unicode_oracle(), &["umap"], input);
    let mut c = Cursor::new(&out);
    for keys in &sets {
        let theirs: Vec<u32> = (0..keys.len()).map(|_| c.u32()).collect();
        let hashes: Vec<u64> = keys.iter().map(|k| cxx::hash_string(k)).collect();
        assert_eq!(cxx::unordered_map_order(&hashes), theirs, "{} keys", keys.len());
    }
    assert!(c.done());
}

/// Split a corpus with llama.cpp's `unicode_regex_split` and with ours.
fn compare_splits(name: &str, patterns: &[&str], byte_encode: bool, texts: &[Vec<u8>]) {
    let splitter = Splitter::new(patterns, byte_encode).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut input = Vec::new();
    put_u32(&mut input, patterns.len() as u32);
    for p in patterns {
        put_bytes(&mut input, p.as_bytes());
    }
    input.push(byte_encode as u8);
    for t in texts {
        put_bytes(&mut input, t);
    }
    let out = run_oracle(&unicode_oracle(), &["split"], input);
    let mut c = Cursor::new(&out);
    for t in texts {
        let ok = c.u8() == 1;
        let theirs: Result<Vec<Vec<u8>>, String> = if ok {
            Ok((0..c.u32()).map(|_| c.bytes().to_vec()).collect())
        } else {
            Err(String::from_utf8_lossy(c.bytes()).into_owned())
        };
        let ours = splitter.split(t);
        match (&ours, &theirs) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "{name} splits {:?} differently", String::from_utf8_lossy(t)),
            (Err(_), Err(_)) => {}
            _ => panic!("{name} on {:?}: ours {ours:?}, llama.cpp {theirs:?}", String::from_utf8_lossy(t)),
        }
    }
    assert!(c.done());
}

#[test]
fn every_pre_tokenizer_splits_seeded_text_as_llama_cpp_does() {
    if !oracles_ready() {
        return;
    }
    let specials: Vec<Vec<u8>> = ["<|im_start|>", "<|endoftext|>", "<s>", "</s>", "<|begin_of_text|>"]
        .iter()
        .map(|s| s.as_bytes().to_vec())
        .collect();
    for (k, pt) in PreType::ALL.iter().enumerate() {
        let mut rng = Rng(0x5eed_1000 + k as u64);
        let texts: Vec<Vec<u8>> = (0..600).map(|_| seeded_text(&mut rng, &specials)).collect();
        compare_splits(pt.llama_name(), pt.regexes(), pt.byte_encode(), &texts);
    }
}

#[test]
fn the_pre_tokenizer_table_is_llama_cpps() {
    if !common::require_tool("competitor-sources") {
        return;
    }
    let src = std::fs::read_to_string(common::llama_cpp_root().join("src/llama-vocab.cpp")).unwrap();
    let ctor = &src[src.find("llm_tokenizer_bpe(const llama_vocab & vocab) {").unwrap()..];
    let ctor = &ctor[..ctor.find("std::vector<std::string> regex_exprs;").unwrap()];
    let mut theirs: Vec<(Vec<String>, Vec<String>, bool)> = Vec::new();
    let mut cases: Vec<String> = Vec::new();
    for block in ctor.split("break;") {
        for line in block.lines() {
            let l = line.trim();
            if let Some(c) = l.strip_prefix("case LLAMA_VOCAB_PRE_TYPE_").and_then(|c| c.strip_suffix(':')) {
                cases.push(c.to_string());
            } else if l == "default:" {
                cases.push("DEFAULT".into());
            }
        }
        let Some(start) = block.find("regex_exprs = {") else { continue };
        let body = &block[start..block[start..].find("};").unwrap() + start];
        let mut pats = Vec::new();
        for line in body.lines().skip(1) {
            let l = line.trim();
            if l.starts_with("//") || !l.starts_with('"') {
                continue;
            }
            pats.push(decode_c_string(l));
        }
        theirs.push((std::mem::take(&mut cases), pats, !block.contains("byte_encode = false")));
    }
    let mut seen = BTreeSet::new();
    for (names, pats, byte_encode) in &theirs {
        for name in names {
            let pt = PreType::ALL.iter().find(|p| p.llama_name() == name).unwrap_or_else(|| panic!("no PreType for {name}"));
            assert_eq!(pt.regexes(), pats.iter().map(String::as_str).collect::<Vec<_>>().as_slice(), "{name}");
            assert_eq!(pt.byte_encode(), *byte_encode, "{name} byte encoding");
            seen.insert(name.clone());
        }
    }
    let ours: BTreeSet<String> = PreType::ALL.iter().map(|p| p.llama_name().to_string()).collect();
    assert_eq!(seen, ours, "every type llama.cpp splits for, and no other");

    let load = &src[src.find("for now, only BPE models have pre-tokenizers").unwrap()..];
    let load = &load[..load.find("} else if (type == LLAMA_VOCAB_TYPE_SPM)").unwrap()];
    let mut names = 0;
    for branch in load.split("} else if (").skip(1) {
        let quoted: Vec<&str> = branch.split("tokenizer_pre == \"").skip(1).map(|s| &s[..s.find('"').unwrap()]).collect();
        let ty = branch.split("pre_type = LLAMA_VOCAB_PRE_TYPE_").nth(1).map(|s| &s[..s.find(';').unwrap()]).unwrap();
        let flag = |f: &str| -> Option<bool> {
            branch.split(&format!("{f} = ")).nth(1).map(|s| s.starts_with("true"))
        };
        for name in quoted {
            let (pt, set) = pre_type(name).unwrap_or_else(|| panic!("{name} is unknown to us"));
            assert_eq!(pt.llama_name(), ty, "{name}");
            assert_eq!(set.clean_spaces, flag("clean_spaces"), "{name} clean_spaces");
            assert_eq!(set.ignore_merges, flag("ignore_merges"), "{name} ignore_merges");
            assert_eq!(set.add_bos, flag("add_bos"), "{name} add_bos");
            assert_eq!(set.escape_whitespaces, flag("escape_whitespaces"), "{name} escape_whitespaces");
            assert_eq!(set.no_bos_token, branch.contains("special_bos_id = LLAMA_TOKEN_NULL"), "{name} bos");
            assert_eq!(set.no_lowercase, branch.contains("normalizer_opts.lowercase = false"), "{name} lowercase");
            names += 1;
        }
    }
    assert!(names > 90, "the name table read {names} names");
    assert!(pre_type("no-such-pre").is_none());
}

/// A C string literal's value.
fn decode_c_string(lit: &str) -> String {
    let s = &lit[1..lit.rfind('"').unwrap()];
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next().unwrap() {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            'r' => out.push('\r'),
            'n' => out.push('\n'),
            't' => out.push('\t'),
            other => panic!("an escape \\{other} in a pattern literal"),
        }
    }
    out
}

// ---------------------------------------------------------------- the generated tables

const TABLES: &str = "../logicaffeine_compile/src/model/tokenizer/unicode_tables.rs";

/// The tables file the generator writes from the oracle's dumps, formatted as `cargo fmt` leaves it.
fn generated_tables() -> String {
    let flags = run_oracle(&unicode_oracle(), &["flags"], Vec::new());
    let lower = run_oracle(&unicode_oracle(), &["lower"], Vec::new());
    let mut ranges: Vec<(u32, u16)> = Vec::new();
    for c in 0..0x110000u32 {
        let f = u16::from_le_bytes([flags[2 * c as usize], flags[2 * c as usize + 1]]) & unicode::Flags::KEPT;
        if ranges.last().map(|r| r.1) != Some(f) {
            ranges.push((c, f));
        }
    }
    let pairs: Vec<(u32, u32)> = (0..0x110000u32)
        .filter_map(|c| {
            let l = u32::from_le_bytes(lower[4 * c as usize..4 * c as usize + 4].try_into().unwrap());
            (l != c).then_some((c, l))
        })
        .collect();
    let mut runs: Vec<(u32, u32, u32, i64)> = Vec::new();
    let mut k = 0;
    while k < pairs.len() {
        let (first, l) = pairs[k];
        let delta = l as i64 - first as i64;
        let mut best = (1u32, 1u32);
        for stride in [1u32, 2] {
            let mut count = 1;
            while k + (count as usize) < pairs.len() {
                let (c, l) = pairs[k + count as usize];
                if c != first + count * stride || l as i64 - c as i64 != delta {
                    break;
                }
                count += 1;
            }
            if count > best.0 {
                best = (count, stride);
            }
        }
        runs.push((first, best.0, best.1, delta));
        k += best.0 as usize;
    }
    let mut s = String::new();
    s.push_str(&format!(
        "//! Codepoint tables generated by `model_tokenizer_lock::regenerate_unicode_tables` from llama.cpp's running\n\
         //! `unicode_cpt_flags_from_cpt` and `unicode_tolower` at commit {}; do not edit.\n\n",
        pin_commit()
    ));
    s.push_str("/// `first << 9 | flags`: the category and whitespace flags every codepoint holds up to the next entry's first.\n");
    s.push_str(&format!("pub(super) static FLAG_RANGES: [u32; {}] = [\n", ranges.len()));
    for (c, f) in &ranges {
        s.push_str(&format!("    0x{:08X},\n", (c << 9) | *f as u32));
    }
    s.push_str("];\n\n/// `(first, count, stride, delta)`: `first + k·stride` lowercases to itself plus `delta`, for `k < count`.\n");
    s.push_str(&format!("pub(super) static LOWERCASE_RUNS: [(u32, u32, u32, i32); {}] = [\n", runs.len()));
    for (c, n, st, d) in &runs {
        s.push_str(&format!("    (0x{c:05X}, {n}, {st}, {d}),\n"));
    }
    s.push_str("];\n");
    let mut fmt = Command::new("rustfmt")
        .args(["--edition", "2021", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("rustfmt runs");
    fmt.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    let out = fmt.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn the_unicode_tables_are_what_the_generator_writes() {
    if !(oracles_ready() && common::require_tool("rustfmt")) {
        return;
    }
    let checked_in = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TABLES)).unwrap();
    assert!(
        checked_in == generated_tables(),
        "unicode_tables.rs is not what the generator writes from the pinned llama.cpp; run \
         `cargo nextest run -p logicaffeine-tests --test model_tokenizer_lock --run-ignored only -E 'test(regenerate_unicode_tables)'`"
    );
}

#[test]
#[ignore = "rewrites unicode_tables.rs from the pinned llama.cpp"]
fn regenerate_unicode_tables() {
    assert!(oracles_ready() && common::require_tool("rustfmt"), "the generator needs llama.cpp's sources, g++ and rustfmt");
    std::fs::write(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TABLES), generated_tables()).unwrap();
}

// ---------------------------------------------------------------- against libllama

fn vocab_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(common::llama_cpp_root().join("models"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "gguf") && p.file_name().unwrap().to_string_lossy().starts_with("ggml-vocab-"))
        .collect();
    v.sort();
    v
}

fn load(path: &Path) -> Result<Vocab, VocabError> {
    Vocab::from_gguf(&Gguf::open(path).expect("the GGUF opens"))
}

/// Hugging Face's tokenizations of llama.cpp's vocabulary corpus: every input with its ids.
fn corpus(path: &Path) -> Option<Vec<(Vec<u8>, Vec<u32>)>> {
    let inp = std::fs::read(format!("{}.inp", path.display())).ok()?;
    let out = std::fs::read_to_string(format!("{}.out", path.display())).ok()?;
    let sep = b"\n__ggml_vocab_test__\n";
    let mut texts = Vec::new();
    let mut pos = 0;
    while pos < inp.len() {
        match inp[pos..].windows(sep.len()).position(|w| w == sep) {
            Some(p) => {
                texts.push(inp[pos..pos + p].to_vec());
                pos += p + sep.len();
            }
            None => {
                texts.push(inp[pos..].to_vec());
                break;
            }
        }
    }
    let ids: Vec<Vec<u32>> =
        out.lines().map(|l| l.split_whitespace().map(|t| t.parse().unwrap()).collect()).collect();
    assert_eq!(texts.len(), ids.len(), "{}", path.display());
    Some(texts.into_iter().zip(ids).collect())
}

#[test]
fn every_bundled_vocabulary_tokenizes_its_corpus_as_hugging_face_does() {
    if !common::require_tool("llama-cpp-build") {
        return;
    }
    let mut checked = 0;
    for path in vocab_files() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let v = match load(&path) {
            Ok(v) => v,
            Err(VocabError::UnsupportedModel(m)) => {
                assert!(name.contains("bert") || m == "bert", "{name} is refused as `{m}`");
                continue;
            }
            Err(e) => panic!("{name}: {e}"),
        };
        let Some(cases) = corpus(&path) else { continue };
        for (text, ids) in &cases {
            let got = v.tokenize(text, false, false).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(&got, ids, "{name} tokenizes {:?}", String::from_utf8_lossy(text));
        }
        checked += 1;
    }
    assert!(checked >= 13, "{checked} corpora checked");
}

/// llama.cpp's view of a vocabulary: its special ids, add flags, and every token's attributes, score, text and
/// end-of-generation bit.
struct TheirVocab {
    ids: [i32; 13],
    add_bos: bool,
    add_eos: bool,
    tokens: Vec<(u32, f32, bool, Vec<u8>)>,
}

fn their_vocab(oracle: &Path, path: &Path) -> TheirVocab {
    let out = run_oracle(oracle, &[path.to_str().unwrap()], vec![4]);
    let mut c = Cursor::new(&out);
    let n = c.u32();
    let ids: [i32; 13] = std::array::from_fn(|_| c.i32());
    let add_bos = c.u8() != 0;
    let add_eos = c.u8() != 0;
    let tokens = (0..n).map(|_| (c.u32(), c.f32(), c.u8() != 0, c.bytes().to_vec())).collect();
    assert!(c.done());
    TheirVocab { ids, add_bos, add_eos, tokens }
}

fn opt(i: i32) -> Option<u32> {
    (i >= 0).then_some(i as u32)
}

/// Compare one vocabulary with llama.cpp: its facts, every token's pieces, and tokenization and detokenization of
/// `n` seeded texts under every `add_special` × `parse_special`.
fn compare_vocab(path: &Path, v: &Vocab, n: usize, seed: u64) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let oracle = vocab_oracle();
    let theirs = their_vocab(&oracle, path);
    assert_eq!(v.n_tokens(), theirs.tokens.len(), "{name} size");
    let s = &v.specials;
    let ours = [s.bos, s.eos, s.eot, s.sep, v.linefeed, s.pad, s.mask, s.fim_pre, s.fim_suf, s.fim_mid, s.fim_pad, s.fim_rep, s.fim_sep];
    let labels = ["bos", "eos", "eot", "sep", "nl", "pad", "mask", "fim_pre", "fim_suf", "fim_mid", "fim_pad", "fim_rep", "fim_sep"];
    for k in 0..13 {
        assert_eq!(ours[k], opt(theirs.ids[k]), "{name} {}", labels[k]);
    }
    assert_eq!((v.add_bos, v.add_eos), (theirs.add_bos, theirs.add_eos), "{name} add flags");
    for (id, (a, score, eog, text)) in theirs.tokens.iter().enumerate() {
        let t = v.token(id as u32);
        assert_eq!(t.attr, *a, "{name} token {id} attributes");
        assert_eq!(t.score.to_bits(), score.to_bits(), "{name} token {id} score");
        assert_eq!(v.is_eog(id as u32), *eog, "{name} token {id} end-of-generation");
        let ours_text = t.text.split(|&b| b == 0).next().unwrap();
        assert_eq!(ours_text, text.as_slice(), "{name} token {id} text");
    }

    let specials: Vec<Vec<u8>> = v.special_order().iter().map(|&id| v.token(id).text.clone()).collect();
    let mut rng = Rng(seed);
    let mut texts: Vec<Vec<u8>> = (0..n).map(|_| seeded_text(&mut rng, &specials)).collect();
    texts.push(Vec::new());
    texts.push(b" ".to_vec());
    texts.push(b"\n".to_vec());

    let mut input = Vec::new();
    let mut asked = Vec::new();
    for t in &texts {
        for (add, parse) in [(false, false), (true, false), (false, true), (true, true)] {
            input.push(1);
            input.push(add as u8);
            input.push(parse as u8);
            put_bytes(&mut input, t);
            asked.push((t.clone(), add, parse));
        }
    }
    for id in 0..v.n_tokens() as u32 {
        for (lstrip, special) in [(0, false), (0, true), (1, true)] {
            input.push(2);
            input.extend_from_slice(&(id as i32).to_le_bytes());
            input.extend_from_slice(&(lstrip as i32).to_le_bytes());
            input.push(special as u8);
        }
    }
    let out = run_oracle(&oracle, &[path.to_str().unwrap()], input);
    let mut c = Cursor::new(&out);
    let mut tokenized = Vec::new();
    for (t, add, parse) in &asked {
        let theirs: Result<Vec<u32>, String> = if c.u8() == 1 {
            Ok((0..c.u32()).map(|_| c.u32()).collect())
        } else {
            Err(String::from_utf8_lossy(c.bytes()).into_owned())
        };
        let ours = v.tokenize(t, *add, *parse);
        match (&ours, &theirs) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "{name} add={add} parse={parse} tokenizes {:?}", String::from_utf8_lossy(t)),
            (Err(TokenizeError::InvalidCodepoint(_)), Err(_)) => {}
            _ => panic!("{name} add={add} parse={parse} on {:?}: ours {ours:?}, llama.cpp {theirs:?}", String::from_utf8_lossy(t)),
        }
        if let Ok(ids) = ours {
            tokenized.push(ids);
        }
    }
    for id in 0..v.n_tokens() as u32 {
        for (lstrip, special) in [(0usize, false), (0, true), (1, true)] {
            assert_eq!(v.token_to_piece(id, lstrip, special), c.bytes(), "{name} piece of {id} lstrip={lstrip} special={special}");
        }
    }
    assert!(c.done());

    let mut input = Vec::new();
    let mut asked = Vec::new();
    for ids in tokenized.iter().take(4 * n) {
        for (remove, unparse) in [(false, false), (true, false), (false, true), (true, true)] {
            input.push(3);
            input.push(remove as u8);
            input.push(unparse as u8);
            put_u32(&mut input, ids.len() as u32);
            for &i in ids {
                input.extend_from_slice(&i.to_le_bytes());
            }
            asked.push((ids.clone(), remove, unparse));
        }
    }
    let out = run_oracle(&oracle, &[path.to_str().unwrap()], input);
    let mut c = Cursor::new(&out);
    for (ids, remove, unparse) in &asked {
        assert_eq!(v.detokenize(ids, *remove, *unparse), c.bytes(), "{name} detokenizes {ids:?} remove={remove} unparse={unparse}");
    }
    assert!(c.done());
}

#[test]
fn every_bundled_vocabulary_matches_llama_cpp_on_seeded_text() {
    if !vocab_oracle_ready() {
        return;
    }
    for (k, path) in vocab_files().iter().enumerate() {
        match load(path) {
            Ok(v) => compare_vocab(path, &v, 300, 0x5eed_2000 + k as u64),
            Err(VocabError::UnsupportedModel(_)) => {}
            Err(e) => panic!("{}: {e}", path.display()),
        }
    }
}

#[test]
fn every_bundled_vocabulary_is_read_as_llama_cpp_reads_it() {
    if !common::require_tool("llama-cpp-build") {
        return;
    }
    let kinds: Vec<(String, Result<VocabKind, String>)> = vocab_files()
        .iter()
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), load(p).map(|v| v.kind).map_err(|e| e.to_string())))
        .collect();
    let kind = |n: &str| kinds.iter().find(|(f, _)| f == n).unwrap_or_else(|| panic!("{n} is bundled")).1.clone();
    assert_eq!(kind("ggml-vocab-qwen2.gguf"), Ok(VocabKind::Bpe(PreType::Qwen2)));
    assert_eq!(kind("ggml-vocab-qwen35.gguf"), Ok(VocabKind::Bpe(PreType::Qwen35)));
    assert_eq!(kind("ggml-vocab-llama-bpe.gguf"), Ok(VocabKind::Bpe(PreType::Llama3)));
    assert_eq!(kind("ggml-vocab-llama-spm.gguf"), Ok(VocabKind::Spm));
    assert_eq!(kind("ggml-vocab-deepseek-llm.gguf"), Ok(VocabKind::Bpe(PreType::DeepseekLlm)));
    assert_eq!(kind("ggml-vocab-gemma-4.gguf"), Ok(VocabKind::Bpe(PreType::Gemma4)));
    assert_eq!(
        kind("ggml-vocab-bert-bge.gguf"),
        Err("the `bert` tokenizer is not run here (byte-pair and SentencePiece vocabularies are)".into())
    );
}

#[test]
fn the_qwen3_8b_checkpoint_tokenizes_as_llama_cpp_does() {
    if !(vocab_oracle_ready() && common::require_tool("qwen3-8b-checkpoint")) {
        return;
    }
    let path = common::qwen3_8b_checkpoint();
    let v = load(&path).unwrap();
    assert_eq!(v.kind, VocabKind::Bpe(PreType::Qwen2));
    let gold: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/qwen_golden.json"))
            .unwrap(),
    )
    .unwrap();
    let ids = |k: &str| -> Vec<u32> { gold[k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect() };
    assert_eq!(v.tokenize(gold["prompt"].as_str().unwrap().as_bytes(), true, false).unwrap(), ids("prompt_tokens"));
    assert_eq!(v.detokenize(&ids("generated_tokens"), false, false), gold["generated_text"].as_str().unwrap().as_bytes());
    assert!(v.is_eog(151645) && v.is_eog(151643), "<|im_end|> and <|endoftext|> end generation");
    compare_vocab(&path, &v, 400, 0x5eed_3000);
}

/// A downloaded checkpoint's vocabulary against llama.cpp's reading of the same file.
fn checkpoint_matches_llama_cpp(model: &str, seed: u64) {
    if !(vocab_oracle_ready() && common::require_tool(&format!("checkpoint:{model}"))) {
        return;
    }
    let path = common::checkpoint(model).unwrap();
    compare_vocab(&path, &load(&path).unwrap(), 300, seed);
}

#[test]
fn the_qwen3_5_vocabulary_tokenizes_as_llama_cpp_does() {
    checkpoint_matches_llama_cpp("qwen3.5-0.8b", 0x5eed_4001);
}

#[test]
fn the_llama3_2_vocabulary_tokenizes_as_llama_cpp_does() {
    checkpoint_matches_llama_cpp("llama3.2-1b", 0x5eed_4002);
}

#[test]
fn the_gemma3_vocabulary_tokenizes_as_llama_cpp_does() {
    checkpoint_matches_llama_cpp("gemma3-1b", 0x5eed_4003);
}

#[test]
fn the_gpt_oss_vocabulary_tokenizes_as_llama_cpp_does() {
    checkpoint_matches_llama_cpp("gpt-oss-20b", 0x5eed_4004);
}

#[test]
fn the_deepseek_v2_vocabulary_tokenizes_as_llama_cpp_does() {
    checkpoint_matches_llama_cpp("deepseek-v2-lite", 0x5eed_4005);
}

#[test]
fn a_token_type_bit_is_read_as_its_attribute() {
    assert_eq!(attr::NORMAL, 4);
    assert_eq!(attr::CONTROL, 8);
    assert_eq!(attr::USER_DEFINED, 16);
    assert_eq!(attr::BYTE, 32);
}
