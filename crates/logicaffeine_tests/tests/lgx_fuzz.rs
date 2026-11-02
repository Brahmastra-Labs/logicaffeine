//! ★ LOCK — LGX round-trips and never panics: the template tree prints back to source
//! that parses to the same tree, and hostile input never crashes the tokenizer, the tree
//! builder or the CSS scoper. ★
//!
//! The light corpus runs in every suite; the seeded generators are the `#[ignore]`d
//! heavies (`./scripts/run-all-tests-fast.sh` runs them; `--no-ignored` skips them).

use logicaffeine_language::lgx::css;
use logicaffeine_language::lgx::html;
use logicaffeine_language::lgx::template::{parse, print};

/// A tiny deterministic PRNG (xorshift64*) so the heavies replay bit-for-bit.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

const ROUND_TRIP_CORPUS: &[&str] = &[
    "<p>Hi {name | upper}!</p>",
    "{if a:}<b>1</b>{else if c:}<i>2</i>{else:}3{end}",
    "{for each todo in todos, keyed by todo's id, at index i:}<li>{todo's text}</li>{else:}<li>none</li>{end}",
    "{inspect r:}{when Ok (v):}<p>{v}</p>{when Err (e):}<p>{e}</p>{end}",
    "{let total be sum(prices)}<p>{total}</p>",
    "{extends Site}{block title:}Todos{end}{block content:}<p>x</p>{end}",
    "<div>{children}{raw html}</div>",
    "<Card title=\"Todos\" count={n}><p>body</p></Card>",
    "<form on-submit.prevent={add}><input bind={draft} class:done={ok} tooltip={t} disabled={busy}></form>",
    "<ul><li>a</li><li>b</li></ul>",
    "<pre>  a\n   b</pre>",
    "<p title=\"a &amp; b\">&lt;x&gt;</p>",
    "<TodoRow todo={t} remove={remove}/>",
];

#[test]
fn printing_a_tree_and_parsing_it_back_gives_the_same_tree() {
    for src in ROUND_TRIP_CORPUS {
        let tree = parse(src).unwrap_or_else(|e| panic!("{src:?}: {e:?}"));
        let printed = print(&tree);
        let again = parse(&printed)
            .unwrap_or_else(|e| panic!("printed {printed:?} of {src:?} must parse: {e:?}"));
        assert_eq!(
            format!("{again:?}").replace(char::is_numeric, ""),
            format!("{tree:?}").replace(char::is_numeric, ""),
            "round trip changed the tree for {src:?}\nprinted: {printed}"
        );
        let twice = print(&again);
        assert_eq!(twice, printed, "printing is idempotent for {src:?}");
    }
}

/// Build a random well-formed template from a small grammar.
fn gen_template(rng: &mut Rng, depth: usize, out: &mut String) {
    let n = 1 + rng.below(3);
    for _ in 0..n {
        match rng.below(if depth == 0 { 4 } else { 9 }) {
            0 => out.push_str(rng.pick(&["hi", "a b", "x &amp; y", " "])),
            1 => out.push_str(&format!(
                "{{{}}}",
                rng.pick(&[
                    "name",
                    "todo's text",
                    "count of xs",
                    "price | money",
                    "f(\"{x}\")"
                ])
            )),
            2 => out.push_str(&format!(
                "<{0}>{1}</{0}>",
                rng.pick(&["p", "span", "b"]),
                rng.pick(&["x", "{y}", ""])
            )),
            3 => out.push_str(rng.pick(&[
                "<br>",
                "<input bind={draft}>",
                "<img src=\"a.png\" alt=\"a\">",
                "<TodoRow todo={t}/>",
            ])),
            4 => {
                out.push_str("<div class=\"c\">");
                gen_template(rng, depth - 1, out);
                out.push_str("</div>");
            }
            5 => {
                out.push_str("{if ok:}");
                gen_template(rng, depth - 1, out);
                if rng.below(2) == 0 {
                    out.push_str("{else:}");
                    gen_template(rng, depth - 1, out);
                }
                out.push_str("{end}");
            }
            6 => {
                out.push_str("{for each x in xs:}");
                gen_template(rng, depth - 1, out);
                out.push_str("{end}");
            }
            7 => {
                out.push_str("<Card title=\"t\">");
                gen_template(rng, depth - 1, out);
                out.push_str("</Card>");
            }
            _ => {
                out.push_str("{inspect r:}{when Ok (v):}");
                gen_template(rng, depth - 1, out);
                out.push_str("{end}");
            }
        }
    }
}

#[test]
#[ignore = "seeded generator heavy — run by the full suite"]
fn generated_templates_round_trip_and_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..20_000 {
        let mut src = String::new();
        gen_template(&mut rng, 3, &mut src);
        let tree =
            parse(&src).unwrap_or_else(|e| panic!("generated template must parse: {e:?}\n{src}"));
        let printed = print(&tree);
        let again = parse(&printed)
            .unwrap_or_else(|e| panic!("printed template must parse: {e:?}\n{printed}"));
        assert_eq!(print(&again), printed, "idempotent print\n{src}");
    }
}

#[test]
#[ignore = "random-bytes heavy — run by the full suite"]
fn hostile_bytes_never_panic_any_stage() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    // ★ THE ALPHABET CARRIES NON-ASCII, AND THAT IS THE POINT. It was 33 ASCII characters, so
    // 50,000 generated cases could not reach a single one of the byte-wise scanners that slice
    // a `&str` at the byte they have reached — and two of them PANICKED on any multi-byte
    // character (`lgx_non_ascii_lock`). A fuzzer for a language people write prose in has to be
    // given the bytes prose is made of, or it only ever proves the easy half.
    //
    // One character from each UTF-8 width (2, 3 and 4 bytes) plus a combining mark, so a
    // continuation byte lands in every position the generator can produce.
    let alphabet: Vec<char> =
        "<>/{}:;|\"'=&#. \n\tabcxyz-_()[]!@%*\u{e9}\u{fc}\u{2014}\u{65e5}\u{1f600}\u{301}"
            .chars()
            .collect();
    for _ in 0..50_000 {
        let len = rng.below(48);
        let s: String = (0..len)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        let _ = html::tokenize(&s);
        let _ = parse(&s);
        let _ = css::scope(&s, "lgx-t");
    }
}
