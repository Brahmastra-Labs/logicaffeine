# Worklist — wikis/nipponanthemum.txt

Actionable items only (gate `auto` or `investigate`). Design decisions and isolated noise are in `needs_human.md`. Fix a *cluster*, not a single line.

## Clusters (fix the class)

- **lexicon:nipponicum** ×2 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "Nipponanthemum nipponicum, commonly called \"Nippon daisy\" or \"Montauk daisy,\" is a species of flowering plant in the family Asteraceae." (sentences [1, 5])
- **parser:expected_content_word** ×2 — ParserGap/Parser, gate `Investigate` — e.g. "It is native to coastal regions of Japan but cultivated as an ornamental in other regions." (sentences [2, 6])
- **lexicon:disc** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "Ray flowers are white, disc flowers usually yellow but sometimes red or purple." (sentences [8])
- **lexicon:genus** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "It is the only species in the genus Nipponanthemum, formerly considered part of Chrysanthemum." (sentences [4])
- **lexicon:now** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "It is now naturalized as an escapee along seashores in New York and New Jersey." (sentences [3])
- **lexicon:up** ×1 — ActionableLexiconGap/Lexicon, gate `Auto` — e.g. "Flower heads are up to 8 cm (3 inches) across and are borne singly." (sentences [7])

## Auto-eligible (lexicon, low risk)

### 01. ActionableLexiconGap (0.75)
- input: "Nipponanthemum nipponicum, commonly called \"Nippon daisy\" or \"Montauk daisy,\" is a species of flowering plant in the family Asteraceae."
- suspect words: ["nipponicum", "commonly"]
- proposed lexicon entry: `Nipponicum` as `Noun` (POS inferred from suffix; verify before adding)

### 03. ActionableLexiconGap (0.75)
- input: "It is now naturalized as an escapee along seashores in New York and New Jersey."
- error: `expected_content_word` at "naturalized"
- suspect words: ["now", "naturalized", "seashores"]
- proposed lexicon entry: `Now` as `Noun` (POS inferred from suffix; verify before adding)

### 04. ActionableLexiconGap (0.75)
- input: "It is the only species in the genus Nipponanthemum, formerly considered part of Chrysanthemum."
- suspect words: ["genus", "formerly"]
- proposed lexicon entry: `Genus` as `Noun` (POS inferred from suffix; verify before adding)

### 05. ActionableLexiconGap (0.75)
- input: "Nipponanthemum nipponicum is a shrub up to 100 cm (40 inches) tall."
- suspect words: ["nipponicum", "shrub", "up", "cm"]
- proposed lexicon entry: `Nipponicum` as `Noun` (POS inferred from suffix; verify before adding)

### 07. ActionableLexiconGap (0.75)
- input: "Flower heads are up to 8 cm (3 inches) across and are borne singly."
- suspect words: ["up", "cm", "borne", "singly"]
- proposed lexicon entry: `Up` as `Noun` (POS inferred from suffix; verify before adding)

### 08. ActionableLexiconGap (0.75)
- input: "Ray flowers are white, disc flowers usually yellow but sometimes red or purple."
- suspect words: ["disc", "usually", "sometimes"]
- proposed lexicon entry: `Disc` as `Noun` (POS inferred from suffix; verify before adding)

## Investigate (agent + human judgment)

### 02. ParserGap (0.50)
- input: "It is native to coastal regions of Japan but cultivated as an ornamental in other regions."
- error: `expected_content_word` at "to"
- suspect words: ["coastal", "cultivated", "ornamental"]

### 06. ParserGap (0.50)
- input: "Most of the alternate leaves are clustered near the top of the stem."
- error: `expected_content_word` at "the"
- suspect words: ["alternate", "clustered", "stem"]

