# Worklist — wikis/chad-brook.txt

Actionable items only (gate `auto` or `investigate`). Design decisions and isolated noise are in `needs_human.md`. Fix a *cluster*, not a single line.

## Clusters (fix the class)

- **parser:expected_content_word** ×3 — ParserGap/Parser, gate `Investigate` — e.g. "From there, water flows into the Rivers Rea, Tame and Trent, then the Humber, and eventually the North Sea." (sentences [4, 5, 7])
- **lexicon:derived** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "It may refer to Chad of Mercia, or be derived from the medieval term shadwell, a 'shallow boundary brook'." (sentences [9])
- **lexicon:district** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "It rises in the district of Harborne (formerly in Worcestershire), giving its name to the area known as Chad Valley (and thus indirectly to Chad Valley toys), and runs through the suburb of Edgbaston." (sentences [2])
- **lexicon:mill** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "A water mill, called 'Over Mill' operated on the brook from the 16th to 19th centuries." (sentences [6])
- **lexicon:origins** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "The origins of the name are not recorded." (sentences [8])
- **lexicon:roughly** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "Its course follows a roughly south-easterly direction, passing through the grounds of Lordswood Boys' School and then Harborne Nature Reserve and the Grade II listed Westbourne Road Town Gardens, underneath the former Harborne Railway (now a walkway), crosses the campus of the University of Birmingham and the grounds of Edgbaston Hall where it feeds Edgbaston Pool, then leading to its confluence with the Bourn Brook." (sentences [3])
- **lexicon:stream** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "The Chad Brook is a stream, or brook, wholly within Birmingham, England." (sentences [1])

## Auto-eligible (lexicon, low risk)

### 01. ActionableLexiconGap (0.75)
- input: "The Chad Brook is a stream, or brook, wholly within Birmingham, England."
- suspect words: ["stream", "brook", "wholly"]
- proposed lexicon entry: `Stream` as `Noun` (POS inferred from suffix; verify before adding)

### 02. ActionableLexiconGap (0.75)
- input: "It rises in the district of Harborne (formerly in Worcestershire), giving its name to the area known as Chad Valley (and thus indirectly to Chad Valley toys), and runs through the suburb of Edgbaston."
- suspect words: ["district", "suburb"]
- proposed lexicon entry: `District` as `Noun` (POS inferred from suffix; verify before adding)

### 03. ActionableLexiconGap (0.75)
- input: "Its course follows a roughly south-easterly direction, passing through the grounds of Lordswood Boys' School and then Harborne Nature Reserve and the Grade II listed Westbourne Road Town Gardens, underneath the former Harborne Railway (now a walkway), crosses the campus of the University of Birmingham and the grounds of Edgbaston Hall where it feeds Edgbaston Pool, then leading to its confluence with the Bourn Brook."
- suspect words: ["roughly", "underneath", "crosses", "confluence"]
- proposed lexicon entry: `Roughly` as `Noun` (POS inferred from suffix; verify before adding)

### 06. ActionableLexiconGap (0.75)
- input: "A water mill, called 'Over Mill' operated on the brook from the 16th to 19th centuries."
- suspect words: ["mill", "brook"]
- proposed lexicon entry: `Mill` as `Noun` (POS inferred from suffix; verify before adding)

### 08. ActionableLexiconGap (0.75)
- input: "The origins of the name are not recorded."
- suspect words: ["origins"]
- proposed lexicon entry: `Origins` as `Noun` (POS inferred from suffix; verify before adding)

### 09. ActionableLexiconGap (0.75)
- input: "It may refer to Chad of Mercia, or be derived from the medieval term shadwell, a 'shallow boundary brook'."
- suspect words: ["derived", "medieval", "term", "shadwell", "shallow", "boundary", "brook"]
- proposed lexicon entry: `Derived` as `Adjective` (POS inferred from suffix; verify before adding)

## Investigate (agent + human judgment)

### 05. ParserGap (0.85)
- input: "At one time, The Chad formed the boundary between the counties of Worcestershire and Staffordshire."
- error: `expected_content_word` at "one"
- suspect words: ["boundary", "counties"]
- oracle [pp_fronting_to_trailing]: paraphrase "The Chad formed the boundary between the counties of Worcestershire and Staffordshire at one time." parses ⇒ expected `1) ∃x(((Chad(x) ∧ ∀y((Chad(y) → y = x))) ∧ (∃e(Form(e) ∧ Agent(e, x) ∧ Theme(e, Boundary) ∧ Past(e)) ∧ Between(e, Poss(Worcestershire, Counties)))))
2) Staffordshire`
- proposed RED test:
```rust
#[test]
fn triage_05_paraphrase_equivalence() {
    // The failing form should compile like its parsing paraphrase.
    // input:    "At one time, The Chad formed the boundary between the counties of Worcestershire and Staffordshire."
    // oracle:   "The Chad formed the boundary between the counties of Worcestershire and Staffordshire at one time."
    // expected: 1) ∃x(((Chad(x) ∧ ∀y((Chad(y) → y = x))) ∧ (∃e(Form(e) ∧ Agent(e, x) ∧ Theme(e, Boundary) ∧ Past(e)) ∧ Between(e, Poss(Worcestershire, Counties)))))
2) Staffordshire
    let fol = compile("At one time, The Chad formed the boundary between the counties of Worcestershire and Staffordshire.").expect("should parse like its paraphrase");
    assert!(!fol.is_empty(), "got: {}", fol);
}
```

### 04. ParserGap (0.50)
- input: "From there, water flows into the Rivers Rea, Tame and Trent, then the Humber, and eventually the North Sea."
- error: `expected_content_word` at "there"
- suspect words: ["eventually"]

### 07. ParserGap (0.50)
- input: "The remains of some of its buildings are extant."
- error: `expected_content_word` at "of"
- suspect words: ["extant"]

