# LogicAffeine Internationalization (i18n) — Implementation Plan

## Overview

A pre-lexer normalization layer that maps foreign-language surface syntax to English equivalents, enabling LogicAffeine programs to be written in any human language while keeping English as the canonical internal representation.

The type system (`VerbClass`, `Sort`, `Feature`, `Time`, `Aspect`, `Gender`, `Case`, `Number`) is already language-agnostic. Only the surface syntax is English. A normalizer that runs before the lexer lets the entire pipeline (lexer, parser, analysis, codegen) remain unchanged.

```
Foreign Source (.logic)
       |
       v
 +-----------------+
 | Normalizer      |  maps foreign words -> English equivalents
 +--------+--------+
          |  (English text + source map)
          v
 +-----------------+
 | Lexer           |  (unchanged)
 +--------+--------+
          v
 +-----------------+
 | Parser          |  (unchanged)
 +--------+--------+
          v
 +-----------------+
 | Codegen         |  (unchanged)
 +--------+--------+
          v
     Rust Source
```

Zero changes to lexer/parser/codegen. Zero cost for English (normalizer is a no-op). Translation files are word-to-word JSON mappings.

---

## Phase 1: Crate Scaffold + Core Types

### New Crate: `crates/logicaffeine_i18n/`

```
crates/logicaffeine_i18n/
  Cargo.toml
  src/
    lib.rs              # Public API
    normalizer.rs       # Pre-lexer text normalization engine
    overlay.rs          # Overlay loading, parsing, validation
    source_map.rs       # Byte-offset mapping (normalized <-> original)
    registry.rs         # Language registry + discovery
    validate.rs         # Coverage validation
```

### `Cargo.toml`

```toml
[package]
name = "logicaffeine_i18n"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
description = "Internationalization layer for LogicAffeine — pre-lexer normalization"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }

[dev-dependencies]
```

### Workspace Registration

File: `Cargo.toml` (workspace root, line 3)

Add `"crates/logicaffeine_i18n"` to the `members` array.

---

### `src/lib.rs` — Public API

```rust
//! # logicaffeine_i18n
//!
//! Pre-lexer internationalization for LogicAffeine.
//!
//! Normalizes foreign-language source code to English before the lexer runs,
//! enabling programs to be written in any human language.
//!
//! ## Architecture
//!
//! ```text
//! Foreign Source -> Normalizer -> English Source + SourceMap
//!                                      |
//!                                      v
//!                              Existing Pipeline (unchanged)
//! ```
//!
//! ## Usage
//!
//! ```
//! use logicaffeine_i18n::Normalizer;
//!
//! // English: zero-cost no-op
//! let norm = Normalizer::english();
//! let result = norm.normalize("## Main\nLet x be 5.");
//! assert_eq!(result.text, "## Main\nLet x be 5.");
//!
//! // Spanish (with overlay loaded)
//! let norm = Normalizer::for_language("es").unwrap();
//! let result = norm.normalize("## Principal\nSea x igual a 5.");
//! assert_eq!(result.text, "## Main\nLet x equal to 5.");
//! ```

mod normalizer;
mod overlay;
mod source_map;
mod registry;
mod validate;

pub use normalizer::{Normalizer, NormalizeResult};
pub use overlay::{TranslationOverlay, OverlayMeta, OverlayError};
pub use source_map::I18nSourceMap;
pub use registry::{LanguageRegistry, LangId};
pub use validate::{ValidationReport, CoverageReport};
```

---

### `src/overlay.rs` — Translation Overlay

```rust
//! Translation overlay loading, parsing, and validation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// ISO 639-1 language code.
pub type LangId = String;

/// Error loading or validating a translation overlay.
#[derive(Debug)]
pub enum OverlayError {
    /// File not found or unreadable.
    Io(std::io::Error),
    /// JSON parse/schema error.
    Json(serde_json::Error),
    /// Semantic validation error (e.g., target word not in English lexicon).
    Validation(Vec<String>),
    /// Language not found in registry.
    NotFound(String),
}

/// Metadata about a translation overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayMeta {
    pub language: String,
    pub iso_639_1: String,
    pub name: String,
    pub native_name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub text_direction: TextDirection,
    pub coverage: CoverageFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextDirection {
    Ltr,
    Rtl,
}

/// Tracks which sections of the overlay are populated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageFlags {
    pub keywords: bool,
    pub articles: bool,
    pub pronouns: bool,
    pub block_headers: bool,
    pub type_names: bool,
    pub imperative_keywords: bool,
    #[serde(default)]
    pub prepositions: bool,
    #[serde(default)]
    pub auxiliaries: bool,
    #[serde(default)]
    pub morphology: bool,
    #[serde(default)]
    pub vocabulary: bool,
}

/// A deserialized translation overlay JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationOverlayFile {
    pub meta: OverlayMeta,

    #[serde(default)]
    pub keywords: HashMap<String, String>,
    #[serde(default)]
    pub articles: HashMap<String, String>,
    #[serde(default)]
    pub pronouns: HashMap<String, String>,
    #[serde(default)]
    pub block_headers: HashMap<String, String>,
    #[serde(default)]
    pub type_names: HashMap<String, String>,
    #[serde(default)]
    pub imperative_keywords: HashMap<String, String>,
    #[serde(default)]
    pub prepositions: HashMap<String, String>,
    #[serde(default)]
    pub auxiliaries: HashMap<String, String>,
    #[serde(default)]
    pub number_words: HashMap<String, String>,

    /// Multi-word phrases: "cada uno" -> "each other"
    #[serde(default)]
    pub multi_word: HashMap<String, String>,

    #[serde(default)]
    pub morphology: MorphologySection,

    #[serde(default)]
    pub vocabulary: VocabularySection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MorphologySection {
    #[serde(default)]
    pub contractions: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub verb_forms: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VocabularySection {
    #[serde(default)]
    pub nouns: HashMap<String, String>,
    #[serde(default)]
    pub verbs: HashMap<String, String>,
    #[serde(default)]
    pub adjectives: HashMap<String, String>,
}

/// The compiled (ready-to-use) translation overlay.
///
/// Built from a `TranslationOverlayFile` by merging all section maps
/// into fast lookup structures.
pub struct TranslationOverlay {
    pub meta: OverlayMeta,
    /// Flat word -> English word lookup (merged from all sections).
    /// Case-insensitive keys (stored lowercase).
    lookup: HashMap<String, String>,
    /// Multi-word phrases sorted by descending word count (longest match first).
    multi_word: Vec<(Vec<String>, String)>,
    /// Case-sensitive type name lookup (Int, Nat, Bool, etc.).
    type_names: HashMap<String, String>,
    /// Block header lookup (case-insensitive).
    block_headers: HashMap<String, String>,
    /// Escape block trigger in the target language (e.g., "Escapar a Rust:").
    escape_trigger: Option<String>,
}

impl TranslationOverlay {
    /// Load and compile an overlay from a JSON file path.
    pub fn load(path: &Path) -> Result<Self, OverlayError> { todo!() }

    /// Load and compile from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, OverlayError> { todo!() }

    /// Compile a deserialized overlay file into the fast lookup structures.
    fn compile(file: TranslationOverlayFile) -> Self { todo!() }

    /// Look up a single word (case-insensitive). Returns None if not found.
    pub fn lookup_word(&self, word: &str) -> Option<&str> { todo!() }

    /// Look up a block header word (case-insensitive).
    pub fn lookup_block_header(&self, word: &str) -> Option<&str> { todo!() }

    /// Look up a type name (case-sensitive).
    pub fn lookup_type_name(&self, word: &str) -> Option<&str> { todo!() }

    /// Get multi-word phrases sorted longest-first for greedy matching.
    pub fn multi_word_phrases(&self) -> &[(Vec<String>, String)] { todo!() }
}
```

---

### `src/source_map.rs` — Byte-Offset Source Map

Note: this is distinct from `logicaffeine_compile::sourcemap::SourceMap` which maps generated Rust lines back to LOGOS source. This maps normalized English byte offsets back to original foreign-language byte offsets.

```rust
//! Byte-offset mapping between normalized English source and original foreign source.
//!
//! When the normalizer replaces "Sea" (3 bytes) with "Let" (3 bytes), positions
//! align trivially. When it replaces "a" (1 byte) with "to" (2 bytes), all
//! subsequent positions shift. The source map tracks these deltas so error
//! positions can be translated back to the original source.

/// Maps byte offsets between normalized (English) text and original (foreign) text.
#[derive(Debug, Clone)]
pub struct I18nSourceMap {
    /// Sorted list of (normalized_start, original_start, length_in_normalized, length_in_original).
    /// Each entry represents one replaced span.
    entries: Vec<SourceMapEntry>,
}

#[derive(Debug, Clone, Copy)]
struct SourceMapEntry {
    /// Byte offset in the normalized (English) text.
    normalized_start: usize,
    /// Byte offset in the original (foreign) text.
    original_start: usize,
    /// Length of this segment in the normalized text.
    normalized_len: usize,
    /// Length of the corresponding segment in the original text.
    original_len: usize,
}

impl I18nSourceMap {
    /// Create an identity source map (no transformations).
    pub fn identity() -> Self { todo!() }

    /// Create a source map from a list of replacement entries.
    pub fn new(entries: Vec<(usize, usize, usize, usize)>) -> Self { todo!() }

    /// Translate a byte offset in the normalized text to the original text.
    pub fn to_original(&self, normalized_offset: usize) -> usize { todo!() }

    /// Translate a byte range in the normalized text to the original text.
    pub fn range_to_original(&self, normalized_start: usize, normalized_end: usize) -> (usize, usize) { todo!() }

    /// Returns true if this is an identity map (no transformations were applied).
    pub fn is_identity(&self) -> bool { todo!() }
}
```

---

### `src/normalizer.rs` — Normalization Engine

```rust
//! Pre-lexer text normalization engine.
//!
//! Transforms foreign-language LogicAffeine source into canonical English
//! source that the existing lexer can process. The normalizer:
//!
//! - Replaces foreign keywords with English equivalents
//! - Handles multi-word phrases (longest match first)
//! - Preserves user-defined identifiers unchanged
//! - Preserves escape block contents unchanged
//! - Tracks byte-offset deltas for error position translation
//! - Is a zero-cost no-op for English

use crate::overlay::TranslationOverlay;
use crate::source_map::I18nSourceMap;

/// Result of normalizing source code.
#[derive(Debug)]
pub struct NormalizeResult {
    /// The normalized English source text.
    pub text: String,
    /// Maps byte offsets from the normalized text back to the original.
    pub source_map: I18nSourceMap,
}

/// Pre-lexer normalizer that translates foreign surface syntax to English.
pub struct Normalizer {
    overlay: Option<TranslationOverlay>,
}

impl Normalizer {
    /// Create a no-op normalizer for English source.
    /// This has zero overhead: `normalize()` returns the input unchanged.
    pub fn english() -> Self { todo!() }

    /// Create a normalizer for the given language.
    ///
    /// Looks up the language overlay in the registry (built-in translations
    /// first, then `~/.logicaffeine/translations/`).
    ///
    /// Returns `OverlayError::NotFound` if no overlay exists for the language.
    pub fn for_language(lang: &str) -> Result<Self, crate::overlay::OverlayError> { todo!() }

    /// Create a normalizer from a pre-loaded overlay.
    pub fn with_overlay(overlay: TranslationOverlay) -> Self { todo!() }

    /// Normalize source code, replacing foreign words with English equivalents.
    ///
    /// ## Algorithm
    ///
    /// ```text
    /// for each line in source:
    ///   if line starts with "##":
    ///     normalize block header using block_headers table
    ///   else if inside escape block:
    ///     pass through unchanged
    ///   else:
    ///     for each word (split on whitespace/punctuation):
    ///       1. check multi_word table (longest match first)
    ///       2. check lookup table (case-insensitive)
    ///       3. check type_names table (case-sensitive)
    ///       4. if no match: pass through unchanged
    ///     update source_map with offset deltas
    /// ```
    ///
    /// ## Behaviors
    ///
    /// - **User identifiers pass through**: if "perro" isn't in the overlay, it
    ///   stays "perro" and becomes a valid variable name
    /// - **Case preservation**: overlay maps "si"->"if", so "Si"->"If"
    /// - **Escape blocks are sacred**: content after `Escape to Rust:` (or its
    ///   translated equivalent) is never touched
    /// - **Multi-word greedy**: "cada uno" matches before "cada" alone
    /// - **English no-op**: when overlay is None, returns input unchanged
    pub fn normalize(&self, source: &str) -> NormalizeResult { todo!() }

    /// Returns true if this normalizer is a no-op (English).
    pub fn is_english(&self) -> bool { todo!() }

    /// Returns the language code, or "en" for English.
    pub fn lang_id(&self) -> &str { todo!() }
}
```

---

### `src/registry.rs` — Language Registry

```rust
//! Language registry and discovery.
//!
//! Searches for translation overlays in two locations:
//! 1. Built-in: `assets/translations/` (bundled with the compiler)
//! 2. User-installed: `~/.logicaffeine/translations/` (runtime packs)

use crate::overlay::{TranslationOverlay, OverlayError, OverlayMeta};
use std::path::PathBuf;

/// ISO 639-1 language identifier.
pub type LangId = String;

/// Discovers and loads translation overlays.
pub struct LanguageRegistry {
    /// Built-in translation directory (assets/translations/).
    builtin_dir: Option<PathBuf>,
    /// User-installed translation directory (~/.logicaffeine/translations/).
    user_dir: Option<PathBuf>,
}

impl LanguageRegistry {
    /// Create a registry that searches the default directories.
    pub fn new() -> Self { todo!() }

    /// Create a registry with explicit directories (for testing).
    pub fn with_dirs(builtin_dir: Option<PathBuf>, user_dir: Option<PathBuf>) -> Self { todo!() }

    /// Load the overlay for a language. Checks built-in first, then user-installed.
    pub fn load(&self, lang: &str) -> Result<TranslationOverlay, OverlayError> { todo!() }

    /// List all available languages (built-in + user-installed).
    pub fn available_languages(&self) -> Vec<(LangId, OverlayMeta)> { todo!() }

    /// Check if a language has an overlay available.
    pub fn has_language(&self, lang: &str) -> bool { todo!() }
}
```

---

### `src/validate.rs` — Coverage Validation

```rust
//! Translation overlay coverage validation.
//!
//! Validates that:
//! - Every English target word exists in the compiler's keyword set
//! - No ambiguous mappings exist
//! - Reports coverage percentage per tier

use crate::overlay::TranslationOverlayFile;

/// Validation result for a translation overlay.
#[derive(Debug)]
pub struct ValidationReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub coverage: CoverageReport,
}

/// Coverage percentages by tier.
#[derive(Debug)]
pub struct CoverageReport {
    /// Tier 1: Structural keywords (block headers, quantifiers, imperatives, comparisons).
    pub tier1_structural: f64,
    /// Tier 2: Grammar words (articles, pronouns, auxiliaries, prepositions).
    pub tier2_grammar: f64,
    /// Tier 3: Type system (type names, definition words).
    pub tier3_types: f64,
    /// Tier 4: Domain-specific (CRDT, calendar, time, escape).
    pub tier4_domain: f64,
    /// Tier 5: Morphology (verb forms, contractions, plurals).
    pub tier5_morphology: f64,
}

/// Validate a translation overlay file.
pub fn validate_overlay(overlay: &TranslationOverlayFile) -> ValidationReport { todo!() }
```

---

## Phase 2: Spanish Pilot Overlay

### File: `assets/translations/es.json`

The first translation overlay. Spanish was chosen because:
- Latin alphabet (no CJK tokenization needed yet)
- LTR (no RTL handling needed yet)
- Rich morphology (tests contraction/verb-form handling)
- Large speaker population

```json
{
  "meta": {
    "language": "es",
    "iso_639_1": "es",
    "name": "Spanish",
    "native_name": "Espanol",
    "version": "0.1.0",
    "authors": [],
    "text_direction": "ltr",
    "coverage": {
      "keywords": true,
      "articles": true,
      "pronouns": true,
      "block_headers": true,
      "type_names": true,
      "imperative_keywords": true,
      "prepositions": false,
      "auxiliaries": false,
      "morphology": false,
      "vocabulary": false
    }
  },

  "block_headers": {
    "principal": "main",
    "teorema": "theorem",
    "definicion": "definition",
    "prueba": "proof",
    "ejemplo": "example",
    "logica": "logic",
    "nota": "note",
    "para": "to",
    "un": "a",
    "una": "a",
    "politica": "policy",
    "requiere": "requires"
  },

  "keywords": {
    "todo": "all",
    "todos": "all",
    "toda": "all",
    "todas": "all",
    "cada": "every",
    "ningun": "no",
    "ninguno": "no",
    "ninguna": "no",
    "algun": "some",
    "alguno": "some",
    "alguna": "some",
    "cualquier": "any",
    "ambos": "both",
    "y": "and",
    "pero": "but",
    "o": "or",
    "si": "if",
    "entonces": "then",
    "no": "not",
    "verdadero": "true",
    "falso": "false"
  },

  "imperative_keywords": {
    "sea": "let",
    "establecer": "set",
    "devolver": "return",
    "dar": "give",
    "mostrar": "show",
    "mientras": "while",
    "repetir": "repeat",
    "empujar": "push",
    "sacar": "pop",
    "leer": "read",
    "escribir": "write",
    "llamar": "call",
    "afirmar": "assert",
    "verificar": "check",
    "esperar": "sleep",
    "escuchar": "listen",
    "conectar": "connect",
    "sincronizar": "sync",
    "agregar": "append",
    "eliminar": "remove",
    "resolver": "resolve"
  },

  "articles": {
    "el": "the",
    "la": "the",
    "los": "the",
    "las": "the",
    "un": "a",
    "una": "a",
    "este": "this",
    "esta": "this",
    "estos": "these",
    "estas": "these",
    "ese": "that",
    "esa": "that",
    "esos": "those",
    "esas": "those"
  },

  "pronouns": {
    "yo": "i",
    "el": "he",
    "ella": "she",
    "ello": "it",
    "ellos": "they",
    "ellas": "they",
    "tu": "you",
    "lo": "him",
    "su": "his",
    "mi": "my",
    "quien": "who",
    "que": "what",
    "donde": "where",
    "cuando": "when",
    "por que": "why"
  },

  "type_names": {
    "Ent": "Int",
    "Entero": "Int",
    "Texto": "Text",
    "Bool": "Bool",
    "Booleano": "Boolean",
    "Real": "Real",
    "Unidad": "Unit",
    "Sec": "Seq",
    "Lista": "List",
    "Conjunto": "Set",
    "Mapa": "Map",
    "Pila": "Stack"
  },

  "multi_word": {
    "cada uno": "each other",
    "si y solo si": "if and only if",
    "al menos": "at least",
    "a lo sumo": "at most",
    "es igual a": "is equal to"
  },

  "number_words": {},
  "morphology": {
    "contractions": {},
    "verb_forms": {}
  },
  "vocabulary": {
    "nouns": {},
    "verbs": {},
    "adjectives": {}
  }
}
```

---

## Phase 3: Integration Points

### 3a. `compile.rs` — New entry point

File: `crates/logicaffeine_compile/src/compile.rs`

Add alongside existing `compile_program_full`:

```rust
/// Compile LOGOS source written in a foreign language.
///
/// Normalizes the source from the given language to English before
/// running the standard compilation pipeline.
///
/// For English (`lang = "en"`), this is equivalent to `compile_program_full()`.
pub fn compile_program_with_lang(source: &str, lang: &str) -> Result<CompileOutput, ParseError> {
    if lang == "en" {
        return compile_program_full(source);
    }

    let normalizer = logicaffeine_i18n::Normalizer::for_language(lang)
        .map_err(|e| ParseError {
            kind: crate::error::ParseErrorKind::Custom(format!("i18n: {}", e)),
            span: crate::token::Span::default(),
        })?;

    let result = normalizer.normalize(source);
    // TODO: Thread result.source_map through for error position remapping
    compile_program_full(&result.text)
}
```

New dependency in `crates/logicaffeine_compile/Cargo.toml`:

```toml
logicaffeine_i18n = { path = "../logicaffeine_i18n", optional = true }
```

Feature flag in `crates/logicaffeine_compile/Cargo.toml`:

```toml
[features]
i18n = ["dep:logicaffeine_i18n"]
```

### 3b. `lib.rs` — Re-export

File: `crates/logicaffeine_compile/src/lib.rs`

```rust
#[cfg(feature = "i18n")]
pub use compile::compile_program_with_lang;
```

### 3c. LSP Pipeline

File: `crates/logicaffeine_lsp/src/pipeline.rs`

The `analyze()` function currently takes `source: &str`. With i18n:

```rust
pub fn analyze(source: &str) -> AnalysisResult {
    analyze_with_lang(source, "en")
}

pub fn analyze_with_lang(source: &str, lang: &str) -> AnalysisResult {
    // If non-English, normalize first
    let (effective_source, _source_map) = if lang != "en" {
        #[cfg(feature = "i18n")]
        {
            let normalizer = logicaffeine_i18n::Normalizer::for_language(lang)
                .unwrap_or_else(|_| logicaffeine_i18n::Normalizer::english());
            let result = normalizer.normalize(source);
            (result.text, Some(result.source_map))
        }
        #[cfg(not(feature = "i18n"))]
        { (source.to_string(), None) }
    } else {
        (source.to_string(), None)
    };

    // ... existing pipeline using &effective_source instead of source
}
```

Language detection from document: look for `# lang: xx` on first line.

### 3d. CLI

File: `apps/logicaffeine_cli/src/cli.rs`

Add `--lang` flag to `Build` and `Run` commands:

```rust
Build {
    // ... existing flags ...

    /// Source language (ISO 639-1 code). Default: auto-detect from file header.
    #[arg(long)]
    lang: Option<String>,
},
```

### 3e. Language Detection

File: `crates/logicaffeine_i18n/src/lib.rs` (or a new `detect.rs`)

```rust
/// Detect language from a `# lang: xx` header on the first line.
/// Returns None if no header is found (defaults to English).
pub fn detect_language(source: &str) -> Option<&str> {
    let first_line = source.lines().next()?;
    let trimmed = first_line.trim();
    if trimmed.starts_with("# lang:") {
        Some(trimmed["# lang:".len()..].trim())
    } else {
        None
    }
}
```

---

## Phase 4: Test Strategy

### Unit Tests (in `logicaffeine_i18n`)

```rust
// overlay.rs tests
#[test] fn load_spanish_overlay() { ... }
#[test] fn overlay_lookup_case_insensitive() { ... }
#[test] fn overlay_multi_word_longest_match() { ... }
#[test] fn overlay_type_names_case_sensitive() { ... }

// source_map.rs tests
#[test] fn identity_map_passthrough() { ... }
#[test] fn single_replacement_offset() { ... }
#[test] fn multiple_replacements_accumulate() { ... }
#[test] fn range_translation() { ... }

// normalizer.rs tests
#[test] fn english_noop() { ... }
#[test] fn spanish_block_header() { ... }
#[test] fn spanish_let_statement() { ... }
#[test] fn escape_block_preserved() { ... }
#[test] fn user_identifiers_passthrough() { ... }
#[test] fn case_preservation() { ... }
#[test] fn multi_word_greedy_match() { ... }
#[test] fn mixed_foreign_and_english() { ... }

// registry.rs tests
#[test] fn registry_finds_builtin_spanish() { ... }
#[test] fn registry_not_found() { ... }

// validate.rs tests
#[test] fn validate_good_overlay() { ... }
#[test] fn validate_missing_tier1() { ... }
```

### Integration Tests (in `logicaffeine_tests`)

```rust
// Spanish source compiles to identical Rust as English equivalent
#[test]
fn spanish_hello_world() {
    let english = "## Main\nShow \"Hola Mundo\".";
    let spanish = "## Principal\nMostrar \"Hola Mundo\".";

    let en_result = compile_program_full(english).unwrap();
    let es_result = compile_program_with_lang(spanish, "es").unwrap();
    assert_eq!(en_result.rust_code, es_result.rust_code);
}

#[test]
fn spanish_let_and_show() {
    let spanish = "## Principal\nSea x igual a 5.\nMostrar x.";
    let result = compile_program_with_lang(spanish, "es").unwrap();
    assert!(result.rust_code.contains("let x = 5;"));
}

#[test]
fn english_unchanged_with_lang_en() {
    let source = "## Main\nLet x be 5.";
    let a = compile_program_full(source).unwrap();
    let b = compile_program_with_lang(source, "en").unwrap();
    assert_eq!(a.rust_code, b.rust_code);
}
```

---

## Phase 5: Translation Overlay JSON Schema

### File: `assets/translations/schema.json`

JSON Schema for overlay validation. Enables editor autocompletion when writing overlays.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "LogicAffeine Translation Overlay",
  "description": "Maps foreign-language keywords to English equivalents for the LogicAffeine compiler.",
  "type": "object",
  "required": ["meta"],
  "properties": {
    "meta": {
      "type": "object",
      "required": ["language", "iso_639_1", "name", "native_name", "version", "text_direction", "coverage"],
      "properties": {
        "language": { "type": "string" },
        "iso_639_1": { "type": "string", "pattern": "^[a-z]{2}$" },
        "name": { "type": "string" },
        "native_name": { "type": "string" },
        "version": { "type": "string" },
        "authors": { "type": "array", "items": { "type": "string" } },
        "text_direction": { "type": "string", "enum": ["ltr", "rtl"] },
        "coverage": {
          "type": "object",
          "properties": {
            "keywords": { "type": "boolean" },
            "articles": { "type": "boolean" },
            "pronouns": { "type": "boolean" },
            "block_headers": { "type": "boolean" },
            "type_names": { "type": "boolean" },
            "imperative_keywords": { "type": "boolean" },
            "prepositions": { "type": "boolean" },
            "auxiliaries": { "type": "boolean" },
            "morphology": { "type": "boolean" },
            "vocabulary": { "type": "boolean" }
          }
        }
      }
    },
    "keywords": { "$ref": "#/$defs/wordMap" },
    "articles": { "$ref": "#/$defs/wordMap" },
    "pronouns": { "$ref": "#/$defs/wordMap" },
    "block_headers": { "$ref": "#/$defs/wordMap" },
    "type_names": { "$ref": "#/$defs/wordMap" },
    "imperative_keywords": { "$ref": "#/$defs/wordMap" },
    "prepositions": { "$ref": "#/$defs/wordMap" },
    "auxiliaries": { "$ref": "#/$defs/wordMap" },
    "number_words": { "$ref": "#/$defs/wordMap" },
    "multi_word": { "$ref": "#/$defs/wordMap" },
    "morphology": {
      "type": "object",
      "properties": {
        "contractions": {
          "type": "object",
          "additionalProperties": { "type": "array", "items": { "type": "string" } }
        },
        "verb_forms": { "$ref": "#/$defs/wordMap" }
      }
    },
    "vocabulary": {
      "type": "object",
      "properties": {
        "nouns": { "$ref": "#/$defs/wordMap" },
        "verbs": { "$ref": "#/$defs/wordMap" },
        "adjectives": { "$ref": "#/$defs/wordMap" }
      }
    }
  },
  "$defs": {
    "wordMap": {
      "type": "object",
      "additionalProperties": { "type": "string" }
    }
  }
}
```

---

## Phase 6: Future Work (Not in Initial Implementation)

### Phase 6a: Error Position Remapping
Thread the `I18nSourceMap` through the pipeline so parser errors point at the original foreign-language source, not the normalized English.

### Phase 6b: RTL Support
For Arabic/Hebrew overlays, ensure the normalizer processes text logically (Unicode bidi). The compiler itself needs no changes — RTL is a rendering concern.

### Phase 6c: CJK Tokenization
For Chinese/Japanese/Korean, the normalizer uses the overlay's word list as a tokenizer (longest-match on the character stream) since these languages have no whitespace word boundaries.

### Phase 6d: CLI `validate-translation` Command
```bash
largo validate-translation path/to/overlay.json
```

### Phase 6e: Runtime Language Packs
```bash
largo install-language path/to/custom.json
# Copies to ~/.logicaffeine/translations/
```

### Phase 6f: Largo.toml Language Config
```toml
[project]
language = "es"
```

---

## Complete English Surface Area Reference

Everything a translation overlay must cover for full structural coverage.

### Tier 1: Structural Keywords (~80 words, required for any program)

**Block headers** (lexer.rs:~1702-1714):
`theorem`, `main`, `definition`, `proof`, `example`, `logic`, `note`, `to` (function), `a`/`an` (typedef), `policy`, `requires`

**Quantifiers/connectives** (lexicon keywords):
`all`, `every`, `no`, `some`, `any`, `both`, `most`, `few`, `many`, `and`, `but`, `or`, `if`, `then`, `not`

**Imperative keywords** (lexer.rs:~1934-2068):
`let`, `set`, `return`, `while`, `repeat`, `for`, `from`, `push`, `pop`, `give`, `show`, `read`, `write`, `call`, `before`, `assert`, `check`, `sleep`, `listen`, `connect`, `sync`, `append`, `remove`, `resolve`

**Comparison/logic**:
`is`, `are`, `was`, `were`, `equals`, `than`, `less`, `more`, `greater`, `equal`, `at least`, `at most`

### Tier 2: Grammar Words (~60 words)

**Articles**: `the`, `a`, `an`, `this`, `these`, `that`, `those`
**Pronouns**: `i`, `he`, `she`, `it`, `they`, `you`, `him`, `her`, `his`, `its`, `my`, `their`, `them`, `who`, `whom`, `what`, `where`, `when`, `why`
**Auxiliaries**: `will`, `did`, `does`, `do`, `must`, `shall`, `should`, `can`, `may`, `cannot`, `would`, `could`, `might`
**Prepositions**: `in`, `on`, `at`, `by`, `with`, `for`, `to`, `from`, `of`, `into`, `through`, `toward`, `towards`

### Tier 3: Type System (~20 words)

**Type names** (parser/mod.rs:~519-564):
`Int`, `Nat`, `Text`, `Bool`, `Boolean`, `Real`, `Unit`, `Seq`, `List`, `Vec`, `Set`, `HashSet`, `Map`, `HashMap`, `Stack`

**Type definition words** (discovery.rs):
`has`, `with`, `which`, `generic`, `record`, `struct`, `structure`, `sum`, `enum`, `choice`, `either`, `one`, `of`, `public`

### Tier 4: Domain-Specific (~30 words)

**CRDT keywords**: `shared`, `tally`, `sharedset`, `sharedsequence`, `sharedmap`, `divergent`, `removewins`, `addwins`, `yata`
**Calendar**: `day`/`days`, `week`/`weeks`, `month`/`months`, `year`/`years`, `ago`, `hence`
**Time literals**: `noon`, `midnight`, `am`, `pm`
**Escape blocks**: `Escape to Rust:`

### Tier 5: Morphology (language-specific)

**English morphological rules** (lexer.rs, runtime.rs):
- Verb suffixes: `-ing`, `-ed`, `-s`, `-es`
- Noun plural: `-s`, `-es`, `-ies`
- Irregular forms: explicit lookup tables
- Contractions: `don't`->`do not`, `won't`->`will not`

---

## What Does NOT Change

- `lexer.rs` — zero modifications
- `parser/` — zero modifications
- `analysis/` — zero modifications
- `codegen.rs` — zero modifications
- `interpreter.rs` — zero modifications
- `lexicon` crate — zero modifications
- `build.rs` — zero modifications
- All 179 LSP tests — zero modifications
- All 110 e2e tests — zero modifications

The entire i18n system is additive. It sits upstream of the existing pipeline and can be removed without affecting anything.

---

## Verification Criteria

- **Phase 1**: All existing tests pass. Normalizer with `english()` returns input unchanged.
- **Phase 2**: Spanish overlay loads and compiles. Simple Spanish programs normalize to valid English that compiles.
- **Phase 3**: `compile_program_with_lang(src, "en")` produces identical output to `compile_program_full(src)`.
- **Phase 4**: Unit tests for overlay, source map, normalizer, registry, and validation all pass.
- **Phase 5**: Schema validates the Spanish overlay file. Invalid overlays are rejected.
