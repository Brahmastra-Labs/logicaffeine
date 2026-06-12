# Worklist — wikis/paul-corballis.txt

Actionable items only (gate `auto` or `investigate`). Design decisions and isolated noise are in `needs_human.md`. Fix a *cluster*, not a single line.

## Clusters (fix the class)

- **semantics:lossy** ×2 — SemanticLossy/Semantics, gate `Investigate` — e.g. "He completed a BSc in Psychology (1989) and an MSc in Psychology with First Class Honours (1991) at the University of Auckland." (sentences [4, 5])
- **lexicon:academic** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "Corballis is the son of academic psychologist Michael Corballis." (sentences [3])
- **lexicon:cognitive** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "Paul Michael Corballis is a New Zealand cognitive neuroscientist and professor of Psychology at the University of Auckland." (sentences [1])
- **lexicon:doctorate** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "After his doctorate, Corballis held a postdoctoral position in the Department of Psychological and Brain Sciences at Dartmouth College." (sentences [6])
- **lexicon:faculty** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "He joined the faculty at the University of Auckland School of Psychology, where he is a professor of Psychology." (sentences [7])
- **lexicon:neural** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "Corballis studies the neural mechanisms that support selective attention and visual perception, often using event-related potentials such as the N2pc, together with other EEG and neuroimaging techniques." (sentences [9])
- **lexicon:noninvasive** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "Early in his career he co-authored work demonstrating noninvasive optical imaging of human brain responses during visual stimulation." (sentences [10])
- **lexicon:publications** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "His later publications and collaborations include work on lateralized ERP components associated with attentional selection." (sentences [11])
- **lexicon:visual** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "His research focuses on visual perception and attention, and uses electrophysiology (EEG/ERPs) and neuroimaging methods." (sentences [2])
- **parser:expected_content_word** ×1 — ParserGap/Parser, gate `Investigate` — e.g. "In an inaugural lecture hosted by the Faculty of Science, he discussed \"brain mechanisms of constructive perception.\"" (sentences [8])

## Auto-eligible (lexicon, low risk)

### 01. ActionableLexiconGap (0.75)
- input: "Paul Michael Corballis is a New Zealand cognitive neuroscientist and professor of Psychology at the University of Auckland."
- suspect words: ["cognitive"]
- proposed lexicon entry: `Cognitive` as `Noun` (POS is a guess (most gaps are nouns); verify before adding)

### 02. ActionableLexiconGap (0.75)
- input: "His research focuses on visual perception and attention, and uses electrophysiology (EEG/ERPs) and neuroimaging methods."
- suspect words: ["visual", "perception", "attention", "electrophysiology"]
- proposed lexicon entry: `Visual` as `Noun` (POS is a guess (most gaps are nouns); verify before adding)

### 03. ActionableLexiconGap (0.75)
- input: "Corballis is the son of academic psychologist Michael Corballis."
- suspect words: ["academic"]
- proposed lexicon entry: `Academic` as `Noun` (POS is a guess (most gaps are nouns); verify before adding)

### 06. ActionableLexiconGap (0.75)
- input: "After his doctorate, Corballis held a postdoctoral position in the Department of Psychological and Brain Sciences at Dartmouth College."
- suspect words: ["doctorate", "postdoctoral"]
- proposed lexicon entry: `Doctorate` as `Noun` (POS is a guess (most gaps are nouns); verify before adding)

### 07. ActionableLexiconGap (0.75)
- input: "He joined the faculty at the University of Auckland School of Psychology, where he is a professor of Psychology."
- suspect words: ["faculty"]
- proposed lexicon entry: `Faculty` as `Noun` (POS is a guess (most gaps are nouns); verify before adding)

### 09. ActionableLexiconGap (0.75)
- input: "Corballis studies the neural mechanisms that support selective attention and visual perception, often using event-related potentials such as the N2pc, together with other EEG and neuroimaging techniques."
- suspect words: ["neural", "mechanisms", "selective", "attention", "visual", "perception", "often", "potentials", "techniques"]
- proposed lexicon entry: `Neural` as `Noun` (POS is a guess (most gaps are nouns); verify before adding)

### 10. ActionableLexiconGap (0.75)
- input: "Early in his career he co-authored work demonstrating noninvasive optical imaging of human brain responses during visual stimulation."
- suspect words: ["noninvasive", "optical", "human", "visual", "stimulation"]
- proposed lexicon entry: `Noninvasive` as `Noun` (POS is a guess (most gaps are nouns); verify before adding)

### 11. ActionableLexiconGap (0.75)
- input: "His later publications and collaborations include work on lateralized ERP components associated with attentional selection."
- suspect words: ["publications", "collaborations", "lateralized", "components", "associated", "attentional", "selection"]
- proposed lexicon entry: `Publications` as `Noun` (POS is a guess (most gaps are nouns); verify before adding)

## Investigate (agent + human judgment)

### 08. ParserGap (0.85)
- input: "In an inaugural lecture hosted by the Faculty of Science, he discussed \"brain mechanisms of constructive perception.\""
- error: `expected_content_word` at "an"
- suspect words: ["inaugural", "lecture"]
- oracle [pp_fronting_to_trailing]: paraphrase "he discussed \"brain mechanisms of constructive perception.\" in an inaugural lecture hosted by the Faculty of Science." parses ⇒ expected `∃e(Discuss(e) ∧ Agent(e, Him) ∧ Past(e))`
- proposed RED test:
```rust
#[test]
fn triage_08_paraphrase_equivalence() {
    // The failing form should compile like its parsing paraphrase.
    // input:    "In an inaugural lecture hosted by the Faculty of Science, he discussed \"brain mechanisms of constructive perception.\""
    // oracle:   "he discussed \"brain mechanisms of constructive perception.\" in an inaugural lecture hosted by the Faculty of Science."
    // expected: ∃e(Discuss(e) ∧ Agent(e, Him) ∧ Past(e))
    let fol = compile("In an inaugural lecture hosted by the Faculty of Science, he discussed \"brain mechanisms of constructive perception.\"").expect("should parse like its paraphrase");
    assert!(!fol.is_empty(), "got: {}", fol);
}
```

### 04. SemanticLossy (0.35)
- input: "He completed a BSc in Psychology (1989) and an MSc in Psychology with First Class Honours (1991) at the University of Auckland."

### 05. SemanticLossy (0.35)
- input: "He earned a PhD in Psychology from Columbia University in 1997."

