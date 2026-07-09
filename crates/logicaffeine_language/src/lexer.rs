//! Two-stage lexer for LOGOS natural language input.
//!
//! The lexer transforms natural language text into a token stream suitable
//! for parsing. It operates in two stages:
//!
//! ## Stage 1: Line Lexer
//!
//! The [`LineLexer`] handles structural concerns:
//!
//! - **Indentation**: Tracks indent levels, emits `Indent`/`Dedent` tokens
//! - **Block boundaries**: Identifies significant whitespace
//! - **Content extraction**: Passes line content to Stage 2
//!
//! ## Stage 2: Word Lexer
//!
//! The [`Lexer`] performs word-level tokenization:
//!
//! - **Vocabulary lookup**: Identifies words via the lexicon database
//! - **Morphological analysis**: Handles inflection (verb tenses, plurals)
//! - **Ambiguity resolution**: Uses priority rules for ambiguous words
//!
//! ## Ambiguity Rules
//!
//! When a word matches multiple lexicon entries, priority determines the token:
//!
//! 1. **Quantifiers** over nouns ("some" → Quantifier, not Noun)
//! 2. **Determiners** over adjectives ("the" → Determiner, not Adjective)
//! 3. **Verbs** over nouns for -ing/-ed forms ("running" → Verb)
//!
//! ## Example
//!
//! ```text
//! Input:  "Every cat sleeps."
//! Output: [Quantifier("every"), Noun("cat"), Verb("sleeps"), Period]
//! ```

use logicaffeine_base::Interner;
use crate::lexicon::{self, Aspect, Definiteness, Lexicon, Time};
use crate::token::{BlockType, CalendarUnit, FocusKind, MeasureKind, Span, Token, TokenType};

// ============================================================================
// Stage 1: Line Lexer (Spec §2.5.2)
// ============================================================================

/// Tokens emitted by the LineLexer (Stage 1).
/// Handles structural tokens (Indent, Dedent, Newline) while treating
/// all other content as opaque for Stage 2 word classification.
#[derive(Debug, Clone, PartialEq)]
pub enum LineToken {
    /// Block increased indentation
    Indent,
    /// Block decreased indentation
    Dedent,
    /// Logical newline (statement boundary) - reserved for future use
    Newline,
    /// Content to be further tokenized (line content, trimmed)
    Content { text: String, start: usize, end: usize },
}

/// Stage 1 Lexer: Handles only lines, indentation, and structural tokens.
/// Treats all other text as opaque `Content` for the Stage 2 WordLexer.
pub struct LineLexer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    indent_stack: Vec<usize>,
    pending_dedents: usize,
    position: usize,
    /// True if we need to emit Content for current line
    has_pending_content: bool,
    pending_content_start: usize,
    pending_content_end: usize,
    pending_content_text: String,
    /// True after we've finished processing all lines
    finished_lines: bool,
    /// True if we've emitted at least one Indent (need to emit Dedents at EOF)
    emitted_indent: bool,
    /// Escape block body byte ranges to skip (start_byte, end_byte)
    escape_body_ranges: Vec<(usize, usize)>,
}

impl<'a> LineLexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            indent_stack: vec![0],
            pending_dedents: 0,
            position: 0,
            has_pending_content: false,
            pending_content_start: 0,
            pending_content_end: 0,
            pending_content_text: String::new(),
            finished_lines: false,
            emitted_indent: false,
            escape_body_ranges: Vec::new(),
        }
    }

    pub fn with_escape_ranges(source: &'a str, escape_body_ranges: Vec<(usize, usize)>) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            indent_stack: vec![0],
            pending_dedents: 0,
            position: 0,
            has_pending_content: false,
            pending_content_start: 0,
            pending_content_end: 0,
            pending_content_text: String::new(),
            finished_lines: false,
            emitted_indent: false,
            escape_body_ranges,
        }
    }

    /// Check if a byte position falls within an escape body range.
    fn is_in_escape_body(&self, pos: usize) -> bool {
        self.escape_body_ranges.iter().any(|(start, end)| pos >= *start && pos < *end)
    }

    /// Calculate indentation level at current position (at start of line).
    /// Returns (indent_level, content_start_pos).
    fn measure_indent(&self, line_start: usize) -> (usize, usize) {
        let mut indent = 0;
        let mut pos = line_start;

        while pos < self.bytes.len() {
            match self.bytes[pos] {
                b' ' => {
                    indent += 1;
                    pos += 1;
                }
                b'\t' => {
                    indent += 4; // Tab = 4 spaces
                    pos += 1;
                }
                _ => break,
            }
        }

        (indent, pos)
    }

    /// Read content from current position until end of line or EOF.
    /// Returns (content_text, content_start, content_end, next_line_start).
    fn read_line_content(&self, content_start: usize) -> (String, usize, usize, usize) {
        let mut pos = content_start;

        // Find end of line
        while pos < self.bytes.len() && self.bytes[pos] != b'\n' {
            pos += 1;
        }

        let content_end = pos;
        let text = self.source[content_start..content_end].trim_end().to_string();

        // Move past newline if present
        let next_line_start = if pos < self.bytes.len() && self.bytes[pos] == b'\n' {
            pos + 1
        } else {
            pos
        };

        (text, content_start, content_end, next_line_start)
    }

    /// Check if the line starting at `pos` is blank (only whitespace).
    fn is_blank_line(&self, line_start: usize) -> bool {
        let mut pos = line_start;
        while pos < self.bytes.len() {
            match self.bytes[pos] {
                b' ' | b'\t' => pos += 1,
                b'\n' => return true,
                _ => return false,
            }
        }
        true // EOF counts as blank
    }

    /// Process the next line and update internal state.
    /// Returns true if we have tokens to emit, false if we're done.
    fn process_next_line(&mut self) -> bool {
        // Skip blank lines
        while self.position < self.bytes.len() && self.is_blank_line(self.position) {
            // Skip to next line
            while self.position < self.bytes.len() && self.bytes[self.position] != b'\n' {
                self.position += 1;
            }
            if self.position < self.bytes.len() {
                self.position += 1; // Skip the newline
            }
        }

        // Check if we've reached EOF
        if self.position >= self.bytes.len() {
            self.finished_lines = true;
            // Emit remaining dedents at EOF
            if self.indent_stack.len() > 1 {
                self.pending_dedents = self.indent_stack.len() - 1;
                self.indent_stack.truncate(1);
            }
            return self.pending_dedents > 0;
        }

        // Measure indentation of current line
        let (line_indent, content_start) = self.measure_indent(self.position);

        // Read line content
        let (text, start, end, next_pos) = self.read_line_content(content_start);

        // Skip if content is empty (shouldn't happen after blank line skip, but be safe)
        if text.is_empty() {
            self.position = next_pos;
            return self.process_next_line();
        }

        let current_indent = *self.indent_stack.last().unwrap();

        // Handle indentation changes
        if line_indent > current_indent {
            // Indent: push new level
            self.indent_stack.push(line_indent);
            self.emitted_indent = true;
            // Store content to emit after Indent
            self.has_pending_content = true;
            self.pending_content_text = text;
            self.pending_content_start = start;
            self.pending_content_end = end;
            self.position = next_pos;
            // We'll emit Indent first, then Content
            return true;
        } else if line_indent < current_indent {
            // Dedent: pop until we match
            while self.indent_stack.len() > 1 {
                let top = *self.indent_stack.last().unwrap();
                if line_indent < top {
                    self.indent_stack.pop();
                    self.pending_dedents += 1;
                } else {
                    break;
                }
            }
            // Store content to emit after Dedents
            self.has_pending_content = true;
            self.pending_content_text = text;
            self.pending_content_start = start;
            self.pending_content_end = end;
            self.position = next_pos;
            return true;
        } else {
            // Same indentation level
            self.has_pending_content = true;
            self.pending_content_text = text;
            self.pending_content_start = start;
            self.pending_content_end = end;
            self.position = next_pos;
            return true;
        }
    }
}

impl<'a> Iterator for LineLexer<'a> {
    type Item = LineToken;

    fn next(&mut self) -> Option<LineToken> {
        // 1. Emit pending dedents first
        if self.pending_dedents > 0 {
            self.pending_dedents -= 1;
            return Some(LineToken::Dedent);
        }

        // 2. Emit pending content
        if self.has_pending_content {
            self.has_pending_content = false;
            let text = std::mem::take(&mut self.pending_content_text);
            let start = self.pending_content_start;
            let end = self.pending_content_end;
            return Some(LineToken::Content { text, start, end });
        }

        // 3. Check if we need to emit Indent (after pushing to stack)
        // This happens when we detected an indent but haven't emitted the token yet
        // We need to check if indent_stack was just modified

        // 4. Process next line
        if !self.finished_lines {
            let had_indent = self.indent_stack.len();
            if self.process_next_line() {
                // Check if we added an indent level
                if self.indent_stack.len() > had_indent {
                    return Some(LineToken::Indent);
                }
                // Check if we have pending dedents
                if self.pending_dedents > 0 {
                    self.pending_dedents -= 1;
                    return Some(LineToken::Dedent);
                }
                // Otherwise emit content
                if self.has_pending_content {
                    self.has_pending_content = false;
                    let text = std::mem::take(&mut self.pending_content_text);
                    let start = self.pending_content_start;
                    let end = self.pending_content_end;
                    return Some(LineToken::Content { text, start, end });
                }
            } else if self.pending_dedents > 0 {
                // EOF with pending dedents
                self.pending_dedents -= 1;
                return Some(LineToken::Dedent);
            }
        }

        // 5. Emit any remaining dedents at EOF
        if self.pending_dedents > 0 {
            self.pending_dedents -= 1;
            return Some(LineToken::Dedent);
        }

        None
    }
}

// ============================================================================
// Stage 2: Word Lexer (existing Lexer)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LexerMode {
    #[default]
    Declarative, // Logic, Theorems, Definitions
    Imperative,  // Main, Functions, Code
}

pub struct Lexer<'a> {
    words: Vec<WordItem>,
    pos: usize,
    lexicon: Lexicon,
    interner: &'a mut Interner,
    input_len: usize,
    in_let_context: bool,
    mode: LexerMode,
    source: String,
    /// Escape block body byte ranges: (skip_start, skip_end) for filtering LineLexer events
    escape_body_ranges: Vec<(usize, usize)>,
}

struct WordItem {
    word: String,
    trailing_punct: Option<char>,
    start: usize,
    end: usize,
    punct_pos: Option<usize>,
}

impl<'a> Lexer<'a> {
    /// Creates a new lexer for the given input text.
    ///
    /// The lexer will tokenize natural language text according to the
    /// lexicon database, performing morphological analysis and ambiguity
    /// resolution.
    ///
    /// # Arguments
    ///
    /// * `input` - The natural language text to tokenize
    /// * `interner` - String interner for efficient symbol handling
    ///
    /// # Example
    ///
    /// ```
    /// use logicaffeine_language::lexer::Lexer;
    /// use logicaffeine_base::Interner;
    ///
    /// let mut interner = Interner::new();
    /// let mut lexer = Lexer::new("Every cat sleeps.", &mut interner);
    /// let tokens = lexer.tokenize();
    ///
    /// assert_eq!(tokens.len(), 5); // Quantifier, Noun, Verb, Period, EOI
    /// ```
    pub fn new(input: &str, interner: &'a mut Interner) -> Self {
        let escape_ranges = Self::find_escape_block_ranges(input);
        let escape_body_ranges: Vec<(usize, usize)> = escape_ranges.iter()
            .map(|(_, end, content_start, _)| (*content_start, *end))
            .collect();
        let words = Self::split_into_words(input, &escape_ranges);
        let input_len = input.len();

        Lexer {
            words,
            pos: 0,
            lexicon: Lexicon::new(),
            interner,
            input_len,
            in_let_context: false,
            mode: LexerMode::Declarative,
            source: input.to_string(),
            escape_body_ranges,
        }
    }

    /// Pre-scan source text for escape block bodies.
    /// Returns (skip_start_byte, skip_end_byte, content_start_byte, raw_code) tuples.
    /// `skip_start` is the line start (for byte skipping in split_into_words).
    /// `content_start` is after leading whitespace (for token span alignment with Indent events).
    fn find_escape_block_ranges(source: &str) -> Vec<(usize, usize, usize, String)> {
        let mut ranges = Vec::new();
        let lines: Vec<&str> = source.split('\n').collect();
        let mut line_starts: Vec<usize> = Vec::with_capacity(lines.len());
        let mut pos = 0;
        for line in &lines {
            line_starts.push(pos);
            pos += line.len() + 1; // +1 for the newline
        }

        let mut i = 0;
        while i < lines.len() {
            let trimmed = lines[i].trim();
            // Check if this line contains an escape header: "Escape to Rust:"
            // Matches both statement position (whole line) and expression position
            // (e.g., "Let x: Int be Escape to Rust:")
            let lower = trimmed.to_lowercase();
            if lower == "escape to rust:" ||
               lower.ends_with(" escape to rust:") ||
               (lower.starts_with("escape to ") && lower.ends_with(':'))
            {
                // Find the body: subsequent lines with deeper indentation
                let header_indent = Self::measure_indent_static(lines[i]);
                i += 1;

                // Skip blank lines to find the first body line
                let mut body_start_line = i;
                while body_start_line < lines.len() && lines[body_start_line].trim().is_empty() {
                    body_start_line += 1;
                }

                if body_start_line >= lines.len() {
                    // No body found
                    continue;
                }

                let base_indent = Self::measure_indent_static(lines[body_start_line]);
                if base_indent <= header_indent {
                    // No indented body
                    continue;
                }

                // Capture all lines at base_indent or deeper
                let body_byte_start = line_starts[body_start_line];
                let mut body_end_line = body_start_line;
                let mut code_lines: Vec<String> = Vec::new();

                let mut j = body_start_line;
                while j < lines.len() {
                    let line = lines[j];
                    if line.trim().is_empty() {
                        // Blank lines are preserved
                        code_lines.push(String::new());
                        body_end_line = j;
                        j += 1;
                        continue;
                    }
                    let line_indent = Self::measure_indent_static(line);
                    if line_indent < base_indent {
                        break;
                    }
                    // Strip base indentation
                    let stripped = Self::strip_indent(line, base_indent);
                    code_lines.push(stripped);
                    body_end_line = j;
                    j += 1;
                }

                // Trim trailing empty lines from code
                while code_lines.last().map_or(false, |l| l.is_empty()) {
                    code_lines.pop();
                }

                if !code_lines.is_empty() {
                    let body_byte_end = if body_end_line + 1 < lines.len() {
                        line_starts[body_end_line + 1]
                    } else {
                        source.len()
                    };
                    // Compute content start (after leading whitespace of first body line)
                    let content_start = body_byte_start + Self::leading_whitespace_bytes(lines[body_start_line]);
                    let raw_code = code_lines.join("\n");
                    ranges.push((body_byte_start, body_byte_end, content_start, raw_code));
                }

                i = j;
            } else {
                i += 1;
            }
        }

        ranges
    }

    /// Count leading whitespace bytes in a line.
    fn leading_whitespace_bytes(line: &str) -> usize {
        let mut count = 0;
        for c in line.chars() {
            match c {
                ' ' | '\t' => count += c.len_utf8(),
                _ => break,
            }
        }
        count
    }

    /// Measure indent of a line (static helper for pre-scan).
    fn measure_indent_static(line: &str) -> usize {
        let mut indent = 0;
        for c in line.chars() {
            match c {
                ' ' => indent += 1,
                '\t' => indent += 4,
                _ => break,
            }
        }
        indent
    }

    /// Strip `count` leading spaces/tabs from a line.
    fn strip_indent(line: &str, count: usize) -> String {
        let mut stripped = 0;
        let mut byte_pos = 0;
        for (i, c) in line.char_indices() {
            if stripped >= count {
                byte_pos = i;
                break;
            }
            match c {
                ' ' => { stripped += 1; byte_pos = i + 1; }
                '\t' => { stripped += 4; byte_pos = i + 1; }
                _ => { byte_pos = i; break; }
            }
        }
        if stripped < count {
            byte_pos = line.len();
        }
        line[byte_pos..].to_string()
    }

    fn split_into_words(input: &str, escape_ranges: &[(usize, usize, usize, String)]) -> Vec<WordItem> {
        let mut items = Vec::new();
        let mut current_word = String::new();
        let mut word_start = 0;
        let chars: Vec<char> = input.chars().collect();
        let mut char_idx = 0;
        let mut skip_count = 0;
        // Inside `[` … `]` a digit-flanked comma is a list separator, not a
        // thousands separator — `[1,2,3]` is three elements. Money literals
        // (`$125,000`) keep their commas at any depth; line-local so an
        // unbalanced bracket never poisons the rest of the input.
        let mut bracket_depth: usize = 0;
        // Track byte offset for escape range matching
        let mut skip_to_byte: Option<usize> = None;

        for (i, c) in input.char_indices() {
            if skip_count > 0 {
                skip_count -= 1;
                char_idx += 1;
                continue;
            }
            // Skip bytes inside escape block bodies
            if let Some(end) = skip_to_byte {
                if i < end {
                    char_idx += 1;
                    continue;
                }
                skip_to_byte = None;
                word_start = i;
            }
            // Check if this byte position starts an escape block body
            if let Some((_, end, content_start, raw_code)) = escape_ranges.iter().find(|(s, _, _, _)| i == *s) {
                // Flush any pending word
                if !current_word.is_empty() {
                    items.push(WordItem {
                        word: std::mem::take(&mut current_word),
                        trailing_punct: None,
                        start: word_start,
                        end: i,
                        punct_pos: None,
                    });
                }
                // Emit the entire block as a single \x00ESC: marker
                // Use content_start (after whitespace) for span alignment with Indent events
                items.push(WordItem {
                    word: format!("\x00ESC:{}", raw_code),
                    trailing_punct: None,
                    start: *content_start,
                    end: *end,
                    punct_pos: None,
                });
                skip_to_byte = Some(*end);
                word_start = *end;
                char_idx += 1;
                continue;
            }
            let next_pos = i + c.len_utf8();
            match c {
                // "8:15 pm" — a space between a H:MM time and "am"/"pm" is part of
                // the time literal: skip the flush so "pm" joins "8:15" → "8:15pm".
                ' ' if Self::is_time_space_before_ampm(&current_word, &chars, char_idx) => {}
                ' ' | '\t' | '\n' | '\r' => {
                    if c == '\n' {
                        bracket_depth = 0;
                    }
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    word_start = next_pos;
                }
                '.' => {
                    // Check if this is a decimal point (digit before and after)
                    let prev_is_digit = !current_word.is_empty()
                        && current_word.chars().last().map_or(false, |ch| ch.is_ascii_digit());
                    let next_is_digit = char_idx + 1 < chars.len()
                        && chars[char_idx + 1].is_ascii_digit();

                    // Field-access / UFCS DOT: an identifier char (letter/`_`) or a
                    // closing `)`/`]` immediately before, and an identifier-start
                    // immediately after — no whitespace either side. `p.x`, `xs.f(a)`,
                    // `(e).m`. Digit-glue wins (`5.sqrt` stays `5` + period + `sqrt`,
                    // `5.0` stays a decimal), and `Show x.` stays a sentence (the next
                    // char is a newline, not an identifier). Mode is resolved later:
                    // `classify_with_lookahead` maps the `\x00DOT` marker to `Dot`
                    // (imperative) or a sentence `Period` (declarative — prose keeps
                    // `e.g.` exactly as today).
                    let prev_ident = current_word
                        .chars()
                        .last()
                        .map_or(false, |ch| ch.is_alphabetic() || ch == '_');
                    let prev_close = current_word.is_empty()
                        && char_idx > 0
                        && matches!(chars[char_idx - 1], ')' | ']');
                    let next_ident = char_idx + 1 < chars.len()
                        && (chars[char_idx + 1].is_alphabetic() || chars[char_idx + 1] == '_');

                    if prev_is_digit && next_is_digit {
                        // This is a decimal point, include it in the current word
                        current_word.push(c);
                    } else if (prev_ident || prev_close) && next_ident {
                        // Flush the receiver, then the mode-deferred dot marker.
                        if !current_word.is_empty() {
                            items.push(WordItem {
                                word: std::mem::take(&mut current_word),
                                trailing_punct: None,
                                start: word_start,
                                end: i,
                                punct_pos: None,
                            });
                        }
                        items.push(WordItem {
                            word: "\x00DOT".to_string(),
                            trailing_punct: None,
                            start: i,
                            end: next_pos,
                            punct_pos: None,
                        });
                        word_start = next_pos;
                    } else {
                        // This is a sentence period
                        if !current_word.is_empty() {
                            items.push(WordItem {
                                word: std::mem::take(&mut current_word),
                                trailing_punct: Some(c),
                                start: word_start,
                                end: i,
                                punct_pos: Some(i),
                            });
                        } else {
                            items.push(WordItem {
                                word: String::new(),
                                trailing_punct: Some(c),
                                start: i,
                                end: next_pos,
                                punct_pos: Some(i),
                            });
                        }
                        word_start = next_pos;
                    }
                }
                '#' => {
                    // Check for ## block header (markdown-style)
                    if char_idx + 1 < chars.len() && chars[char_idx + 1] == '#' {
                        // This is a ## block header
                        // Skip the second # and capture the next word as a block header
                        if !current_word.is_empty() {
                            items.push(WordItem {
                                word: std::mem::take(&mut current_word),
                                trailing_punct: None,
                                start: word_start,
                                end: i,
                                punct_pos: None,
                            });
                        }
                        // Skip whitespace after ##
                        let header_start = i;
                        let mut j = char_idx + 2;
                        while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
                            j += 1;
                        }
                        // Capture the block type word
                        let mut block_word = String::from("##");
                        while j < chars.len() && chars[j].is_alphabetic() {
                            block_word.push(chars[j]);
                            j += 1;
                        }
                        if block_word.len() > 2 {
                            items.push(WordItem {
                                word: block_word,
                                trailing_punct: None,
                                start: header_start,
                                end: header_start + (j - char_idx),
                                punct_pos: None,
                            });
                        }
                        skip_count = j - char_idx - 1;
                        word_start = header_start + (j - char_idx);
                    } else {
                        // Single # - treat as comment, skip to end of line
                        // Count how many chars to skip (without modifying char_idx here -
                        // the main loop's skip handler will increment it)
                        let mut look_ahead = char_idx + 1;
                        while look_ahead < chars.len() && chars[look_ahead] != '\n' {
                            skip_count += 1;
                            look_ahead += 1;
                        }
                        if !current_word.is_empty() {
                            items.push(WordItem {
                                word: std::mem::take(&mut current_word),
                                trailing_punct: None,
                                start: word_start,
                                end: i,
                                punct_pos: None,
                            });
                        }
                        word_start = look_ahead + 1; // Start after the newline
                    }
                }
                // String literals: "hello world" or """multi-line"""
                '"' => {
                    // `r"…"` RAW string: the adjacent `r` prefix is part of
                    // the literal, not a word — backslashes stay verbatim
                    // (paths, regexes). Only the exact word `r` glued to the
                    // quote triggers it; `Show r.` (a variable) is untouched.
                    let raw_string = current_word == "r";
                    if raw_string {
                        current_word.clear();
                    } else if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }

                    // Check for triple-quote: """
                    if char_idx + 2 < chars.len() && chars[char_idx + 1] == '"' && chars[char_idx + 2] == '"' {
                        let string_start = i;
                        let mut j = char_idx + 3; // skip opening """
                        // Skip optional newline after opening """
                        if j < chars.len() && chars[j] == '\n' {
                            j += 1;
                        }
                        let mut raw_content = String::new();
                        // Scan until closing """
                        while j < chars.len() {
                            if j + 2 < chars.len() && chars[j] == '"' && chars[j + 1] == '"' && chars[j + 2] == '"' {
                                break;
                            }
                            raw_content.push(chars[j]);
                            j += 1;
                        }
                        // Strip trailing newline before closing """
                        if raw_content.ends_with('\n') {
                            raw_content.pop();
                        }
                        // Dedent: find minimum common indentation and strip it
                        let dedented = Self::dedent_triple_quote(&raw_content);
                        let end_pos = if j + 2 < chars.len() { j + 3 } else { chars.len() };
                        items.push(WordItem {
                            word: format!("\x00STR:{}", dedented),
                            trailing_punct: None,
                            start: string_start,
                            end: end_pos,
                            punct_pos: None,
                        });
                        // Skip past the closing """
                        if j + 2 < chars.len() {
                            skip_count = (j + 2) - char_idx;
                        } else {
                            skip_count = chars.len() - 1 - char_idx;
                        }
                        word_start = end_pos;
                    } else {
                        // Single-quoted string: scan until closing quote
                        let string_start = i;
                        let mut j = char_idx + 1;
                        let mut string_content = String::new();
                        while j < chars.len() && chars[j] != '"' {
                            if chars[j] == '\\' && j + 1 < chars.len() && !raw_string {
                                // DECODE the escape (`"a\nb"` is two lines,
                                // not the letters a-n-b). Unknown escapes and
                                // malformed \u{…} stay verbatim rather than
                                // silently dropping the backslash.
                                j += 1;
                                match chars[j] {
                                    'n' => string_content.push('\n'),
                                    't' => string_content.push('\t'),
                                    'r' => string_content.push('\r'),
                                    '0' => string_content.push('\0'),
                                    '\\' => string_content.push('\\'),
                                    '"' => string_content.push('"'),
                                    'u' if j + 1 < chars.len() && chars[j + 1] == '{' => {
                                        let mut k = j + 2;
                                        let mut hex = String::new();
                                        while k < chars.len() && chars[k] != '}' {
                                            hex.push(chars[k]);
                                            k += 1;
                                        }
                                        match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                            Some(c) if k < chars.len() => {
                                                string_content.push(c);
                                                j = k;
                                            }
                                            _ => {
                                                string_content.push('\\');
                                                string_content.push('u');
                                            }
                                        }
                                    }
                                    other => {
                                        string_content.push('\\');
                                        string_content.push(other);
                                    }
                                }
                            } else {
                                string_content.push(chars[j]);
                            }
                            j += 1;
                        }

                        // Create a special marker for string literals
                        // We prefix with a special character to identify in tokenize()
                        //
                        // `end` and the following `word_start` are BYTE offsets
                        // (Span fields are byte positions, like `start`/`i`); `j`
                        // is a CHAR index, so convert through the byte length of
                        // the covered source chars. Emitting `j + 1` (a char
                        // index) produced a byte-start / char-end span that
                        // underflows downstream on multibyte input.
                        let end_char = if j < chars.len() { j + 1 } else { j };
                        let end_byte = string_start
                            + chars[char_idx..end_char].iter().map(|c| c.len_utf8()).sum::<usize>();
                        items.push(WordItem {
                            word: format!("\x00STR:{}", string_content),
                            trailing_punct: None,
                            start: string_start,
                            end: end_byte,
                            punct_pos: None,
                        });

                        // Skip past the closing quote
                        if j < chars.len() {
                            skip_count = j - char_idx;
                        } else {
                            skip_count = j - char_idx - 1;
                        }
                        word_start = end_byte;
                    }
                }
                // Character literals with backticks: `x`
                '`' => {
                    // Push any pending word
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }

                    // Scan for character content and closing backtick
                    let char_start = i;
                    let mut j = char_idx + 1;
                    let mut char_content = String::new();

                    if j < chars.len() {
                        if chars[j] == '\\' && j + 1 < chars.len() {
                            // Escape sequence
                            j += 1;
                            let escaped_char = match chars[j] {
                                'n' => '\n',
                                't' => '\t',
                                'r' => '\r',
                                '\\' => '\\',
                                '`' => '`',
                                '0' => '\0',
                                c => c,
                            };
                            char_content.push(escaped_char);
                            j += 1;
                        } else if chars[j] != '`' {
                            // Regular character
                            char_content.push(chars[j]);
                            j += 1;
                        }
                    }

                    // Expect closing backtick
                    if j < chars.len() && chars[j] == '`' {
                        j += 1; // skip closing backtick
                    }

                    // Create a special marker for char literals
                    items.push(WordItem {
                        word: format!("\x00CHAR:{}", char_content),
                        trailing_punct: None,
                        start: char_start,
                        end: if j <= chars.len() { char_start + (j - char_idx) } else { char_start + 1 },
                        punct_pos: None,
                    });

                    if j > char_idx + 1 {
                        skip_count = j - char_idx - 1;
                    }
                    word_start = char_start + (j - char_idx);
                }
                // Handle -> as a single token for return type syntax
                '-' if char_idx + 1 < chars.len() && chars[char_idx + 1] == '>' => {
                    // Push any pending word first
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    // Push -> as its own word
                    items.push(WordItem {
                        word: "->".to_string(),
                        trailing_punct: None,
                        start: i,
                        end: i + 2,
                        punct_pos: None,
                    });
                    skip_count = 1; // Skip the '>' character
                    word_start = i + 2;
                }
                // Grand Challenge: Handle <= as a single token
                '<' if char_idx + 1 < chars.len() && chars[char_idx + 1] == '=' => {
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    items.push(WordItem {
                        word: "<=".to_string(),
                        trailing_punct: None,
                        start: i,
                        end: i + 2,
                        punct_pos: None,
                    });
                    skip_count = 1;
                    word_start = i + 2;
                }
                // Grand Challenge: Handle >= as a single token
                '>' if char_idx + 1 < chars.len() && chars[char_idx + 1] == '=' => {
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    items.push(WordItem {
                        word: ">=".to_string(),
                        trailing_punct: None,
                        start: i,
                        end: i + 2,
                        punct_pos: None,
                    });
                    skip_count = 1;
                    word_start = i + 2;
                }
                // Handle == as a single token
                '=' if char_idx + 1 < chars.len() && chars[char_idx + 1] == '=' => {
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    items.push(WordItem {
                        word: "==".to_string(),
                        trailing_punct: None,
                        start: i,
                        end: i + 2,
                        punct_pos: None,
                    });
                    skip_count = 1;
                    word_start = i + 2;
                }
                // Handle != as a single token
                '!' if char_idx + 1 < chars.len() && chars[char_idx + 1] == '=' => {
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    items.push(WordItem {
                        word: "!=".to_string(),
                        trailing_punct: None,
                        start: i,
                        end: i + 2,
                        punct_pos: None,
                    });
                    skip_count = 1;
                    word_start = i + 2;
                }
                // Special handling for '-' in ISO-8601 dates (YYYY-MM-DD)
                '-' if Self::is_date_hyphen(&current_word, &chars, char_idx) => {
                    // This hyphen is part of a date, include it in the word
                    current_word.push(c);
                }
                // Special handling for ':' in time literals (9:30am, 11:45pm)
                ':' if Self::is_time_colon(&current_word, &chars, char_idx) => {
                    // This colon is part of a time, include it in the word
                    current_word.push(c);
                }
                // Hyphenated compounds ("weak-until", "highest-priority"):
                // a hyphen BETWEEN letters is part of the word, never the
                // arithmetic minus (which is space- or digit-adjacent).
                '-' if char_idx > 0
                    && chars[char_idx - 1].is_alphabetic()
                    && char_idx + 1 < chars.len()
                    && chars[char_idx + 1].is_alphabetic() => {
                    current_word.push(c);
                }
                // Scientific notation: 4.84e+00, 1.66E-03, 2.5e-2
                '+' | '-' if Self::is_exponent_sign(&current_word, &chars, char_idx) => {
                    current_word.push(c);
                }
                // Alphanumeric codes ("AV-435", "FRZ-192", "I-5"): a hyphen with
                // an UPPERCASE-code word before it and a DIGIT after is part of
                // the identifier, not an arithmetic minus. The all-uppercase gate
                // keeps lowercase variables ("x-1") and digit arithmetic ("5-3")
                // emitting a Minus.
                '-' if char_idx > 0
                    && chars[char_idx - 1].is_alphabetic()
                    && char_idx + 1 < chars.len()
                    && chars[char_idx + 1].is_ascii_digit()
                    && !current_word.is_empty()
                    && current_word.chars().all(|ch| ch.is_ascii_uppercase()) => {
                    current_word.push(c);
                }
                // Letter grades ("B+", "A+"): a '+' directly after an
                // uppercase-letter word, NOT followed by a digit (arithmetic
                // "B+2"), is part of the grade. A spaced "B + C" flushes "B"
                // before the '+', so only the glued grade form is caught.
                '+' if !current_word.is_empty()
                    && current_word.chars().all(|ch| ch.is_ascii_uppercase())
                    && (char_idx + 1 >= chars.len()
                        || !chars[char_idx + 1].is_ascii_digit()) => {
                    current_word.push(c);
                }
                // Thousands separator inside a number: "125,000", "1,234,567".
                // A comma flanked by digits is part of the numeral, not a clause
                // separator — except inside a bracketed list, where `[1,2,3]`
                // means three elements. A money word (`$125,000`) keeps its
                // commas even there.
                ',' if char_idx > 0
                    && chars[char_idx - 1].is_ascii_digit()
                    && char_idx + 1 < chars.len()
                    && chars[char_idx + 1].is_ascii_digit()
                    && (bracket_depth == 0
                        || current_word.chars().next().map_or(false, Self::is_currency_symbol)) => {
                    current_word.push(c);
                }
                // Date separator inside a numeral: "04/2024", "12/25", "04/2024"
                // (MM/YYYY, MM/DD). A slash flanked by digits is part of the date
                // token, not an arithmetic division (rare in NL clues).
                '/' if char_idx > 0
                    && chars[char_idx - 1].is_ascii_digit()
                    && char_idx + 1 < chars.len()
                    && chars[char_idx + 1].is_ascii_digit() => {
                    current_word.push(c);
                }
                // "28-inch", "6-year-old", "12-year-old" — a hyphen with a DIGIT
                // before and a LETTER after is orthographic (= "28 inch"), so
                // split into separate words here (flush the number, skip the
                // hyphen as a word boundary) rather than emitting an arithmetic
                // Minus. A digit-digit hyphen ("5-3") stays Minus (letter-after
                // required); a letter-letter hyphen ("well-known") is handled above.
                '-' if char_idx > 0
                    && chars[char_idx - 1].is_ascii_digit()
                    && char_idx + 1 < chars.len()
                    && chars[char_idx + 1].is_alphabetic() => {
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    word_start = next_pos;
                }
                // `**` exponentiation — a single two-char token (before the
                // generic punct arm and the `*=` arm; `**` is `*` then `*`).
                '*' if char_idx + 1 < chars.len() && chars[char_idx + 1] == '*' => {
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    items.push(WordItem {
                        word: "**".to_string(),
                        trailing_punct: None,
                        start: i,
                        end: i + 2,
                        punct_pos: None,
                    });
                    skip_count = 1;
                    word_start = i + 2;
                }
                // `//` floor division — a single two-char token (before the generic
                // punct arm and the `/=` arm; `//` is `/` then `/`, not `/` then `=`).
                // A digit-flanked `/` is already claimed above as a date separator, so
                // this only fires on a genuine `x // y`.
                '/' if char_idx + 1 < chars.len() && chars[char_idx + 1] == '/' => {
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    items.push(WordItem {
                        word: "//".to_string(),
                        trailing_punct: None,
                        start: i,
                        end: i + 2,
                        punct_pos: None,
                    });
                    skip_count = 1;
                    word_start = i + 2;
                }
                // Compound assignment `+= -= *= /= %=` — a single two-char token
                // (before the generic punct arm so `+` etc. don't split first).
                // `-=` is distinct from `->` (next is `=`, not `>`); `/=` from `//`.
                '+' | '-' | '*' | '/' | '%'
                    if char_idx + 1 < chars.len() && chars[char_idx + 1] == '=' =>
                {
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    items.push(WordItem {
                        word: format!("{c}="),
                        trailing_punct: None,
                        start: i,
                        end: i + 2,
                        punct_pos: None,
                    });
                    skip_count = 1;
                    word_start = i + 2;
                }
                '(' | ')' | '[' | ']' | '{' | '}' | '|' | '~' | '^' | ',' | '?' | '!' | ':' | '+' | '-' | '*' | '/' | '%' | '<' | '>' | '=' => {
                    match c {
                        // `{…}` literals are bracket contexts for the comma
                        // rule too: `{1,2}` is a two-element set, not `{12}`.
                        '[' | '{' => bracket_depth += 1,
                        ']' | '}' => bracket_depth = bracket_depth.saturating_sub(1),
                        _ => {}
                    }
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: Some(c),
                            start: word_start,
                            end: i,
                            punct_pos: Some(i),
                        });
                    } else {
                        items.push(WordItem {
                            word: String::new(),
                            trailing_punct: Some(c),
                            start: i,
                            end: next_pos,
                            punct_pos: Some(i),
                        });
                    }
                    word_start = next_pos;
                }
                '\'' => {
                    // Handle contractions: expand "don't" → "do" + "not", etc.
                    let remaining: String = chars[char_idx + 1..].iter().collect();
                    let remaining_lower = remaining.to_lowercase();

                    if remaining_lower.starts_with("t ") || remaining_lower.starts_with("t.") ||
                       remaining_lower.starts_with("t,") || remaining_lower == "t" ||
                       (char_idx + 1 < chars.len() && chars[char_idx + 1] == 't' &&
                        (char_idx + 2 >= chars.len() || !chars[char_idx + 2].is_alphabetic())) {
                        // The splitter broke the word at the apostrophe, so
                        // `current_word` is the n't-contraction STEM ("isn",
                        // "won"). Which stems contract — and what they expand
                        // to ("won" is suppletive: "will not") — is lexical
                        // knowledge, so the lexicon owns the table; here we
                        // only apply its expansion.
                        let word_lower = current_word.to_lowercase();
                        if let Some(expansion) =
                            crate::lexicon::lookup_negative_contraction(&word_lower)
                        {
                            let words: Vec<&str> = expansion.split_whitespace().collect();
                            let last_idx = words.len().saturating_sub(1);
                            for (idx, part) in words.iter().enumerate() {
                                items.push(WordItem {
                                    word: (*part).to_string(),
                                    trailing_punct: None,
                                    start: if idx == 0 { word_start } else { i },
                                    end: if idx == last_idx { i + 2 } else { i },
                                    punct_pos: None,
                                });
                            }
                            current_word.clear();
                            word_start = next_pos + 1;
                            skip_count = 1;
                        } else {
                            // Unknown contraction, split normally
                            if !current_word.is_empty() {
                                items.push(WordItem {
                                    word: std::mem::take(&mut current_word),
                                    trailing_punct: Some('\''),
                                    start: word_start,
                                    end: i,
                                    punct_pos: Some(i),
                                });
                            }
                            word_start = next_pos;
                        }
                    } else {
                        // Not a 't contraction, handle normally
                        if !current_word.is_empty() {
                            items.push(WordItem {
                                word: std::mem::take(&mut current_word),
                                trailing_punct: Some('\''),
                                start: word_start,
                                end: i,
                                punct_pos: Some(i),
                            });
                        }
                        word_start = next_pos;
                    }
                }
                // Currency-symbol money literal: a `$ € £ ¥` directly before a digit starts a money
                // word (`$19.99`, `€5`, `¥100`), so the symbol survives into the word for
                // `classify_with_lookahead` to read as `MoneyLiteral`. A lone or non-numeric symbol
                // (`$ each`) still falls through to the default arm and is dropped, as before.
                c if Self::is_currency_symbol(c)
                    && char_idx + 1 < chars.len()
                    && chars[char_idx + 1].is_ascii_digit() => {
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    word_start = i;
                    current_word.push(c);
                }
                c if c.is_alphabetic() || c.is_ascii_digit() || (c == '.' && !current_word.is_empty() && current_word.chars().all(|ch| ch.is_ascii_digit())) || c == '_' => {
                    if current_word.is_empty() {
                        word_start = i;
                    }
                    current_word.push(c);
                }
                '&' => {
                    // "&" is COLOR coordination ("black & red" → emit "and" →
                    // Black ∧ Red) when its neighbour is LOWERCASE, but a FIRM-NAME
                    // joiner ("Leach & Mccall", "Ingram & Kemp") when CAPITALIZED —
                    // there it is dropped so the proper-name absorber joins the
                    // names (Leach_Mccall), the prior behaviour. Decide on the next
                    // word's case.
                    if !current_word.is_empty() {
                        items.push(WordItem {
                            word: std::mem::take(&mut current_word),
                            trailing_punct: None,
                            start: word_start,
                            end: i,
                            punct_pos: None,
                        });
                    }
                    let next_cap = chars[char_idx + 1..]
                        .iter()
                        .find(|ch| !ch.is_whitespace())
                        .map_or(false, |ch| ch.is_ascii_uppercase());
                    if !next_cap {
                        // Mode-deferred: `classify_with_lookahead` resolves the
                        // marker to prose "and" (Declarative) or the bitwise
                        // `&` operator (Imperative).
                        items.push(WordItem {
                            word: "\x00AMP".to_string(),
                            trailing_punct: None,
                            start: i,
                            end: next_pos,
                            punct_pos: None,
                        });
                    }
                    word_start = next_pos;
                }
                _ => {
                    word_start = next_pos;
                }
            }
            char_idx += 1;
        }

        if !current_word.is_empty() {
            items.push(WordItem {
                word: current_word,
                trailing_punct: None,
                start: word_start,
                end: input.len(),
                punct_pos: None,
            });
        }

        items
    }

    fn peek_word(&self, offset: usize) -> Option<&str> {
        self.words.get(self.pos + offset).map(|w| w.word.as_str())
    }

    /// Check if the previous word is a determiner (every, each, some, all, any, no, the, a, an).
    fn prev_token_is_determiner(&self) -> bool {
        if self.pos == 0 { return false; }
        if let Some(prev) = self.words.get(self.pos - 1) {
            matches!(prev.word.to_lowercase().as_str(),
                "every" | "each" | "some" | "all" | "any" | "no" | "the" | "a" | "an")
        } else {
            false
        }
    }

    /// Whether the previous word closes a sentence (ends with `.`/`!`/`?`), so the
    /// current word is sentence-initial. Used to tell a capitalized modal that opens a
    /// question ("Will Alice win?") from a capitalized proper name mid-clause ("started
    /// by Will Waters") — a function word never capitalizes mid-sentence.
    fn prev_word_ends_sentence(&self) -> bool {
        if self.pos == 0 {
            return true;
        }
        self.words
            .get(self.pos - 1)
            .and_then(|w| w.word.chars().last())
            .map_or(true, |c| matches!(c, '.' | '!' | '?' | ':' | ';'))
    }

    /// Whether the previous word is a bare number ("30 minutes", "1 second"),
    /// used to disambiguate the singular clock units "second"/"minute" (which are
    /// also an ordinal and an adjective) — they are time units only after a count.
    fn prev_word_is_numeric(&self) -> bool {
        if self.pos == 0 { return false; }
        self.words
            .get(self.pos - 1)
            .map(|p| {
                let w = p.word.trim_start_matches('$').replace(',', "");
                !w.is_empty() && w.chars().all(|c| c.is_ascii_digit() || c == '.')
            })
            .unwrap_or(false)
    }

    fn next_token_is_copula(&self) -> bool {
        if let Some(next) = self.peek_word(1) {
            matches!(next.to_lowercase().as_str(), "is" | "are" | "was" | "were")
        } else {
            false
        }
    }

    fn peek_sequence(&self, expected: &[&str]) -> bool {
        for (i, &exp) in expected.iter().enumerate() {
            match self.peek_word(i + 1) {
                Some(w) if w.to_lowercase() == exp => continue,
                _ => return false,
            }
        }
        true
    }

    fn consume_words(&mut self, count: usize) {
        self.pos += count;
    }

    /// Tokenizes the input text and returns a vector of [`Token`]s.
    ///
    /// Each token includes its type, the interned lexeme, and the source
    /// span for error reporting. Words are classified according to the
    /// lexicon database with priority-based ambiguity resolution.
    ///
    /// # Returns
    ///
    /// A vector of tokens representing the input. The final token is
    /// typically `TokenType::Eof`.
    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();

        while self.pos < self.words.len() {
            let item = &self.words[self.pos];
            let word = item.word.clone();
            let trailing_punct = item.trailing_punct;
            let word_start = item.start;
            let word_end = item.end;
            let punct_pos = item.punct_pos;

            if word.is_empty() {
                if let Some(punct) = trailing_punct {
                    let kind = match punct {
                        '(' => TokenType::LParen,
                        ')' => TokenType::RParen,
                        '[' => TokenType::LBracket,
                        ']' => TokenType::RBracket,
                        '{' => TokenType::LBrace,
                        '}' => TokenType::RBrace,
                        // Bitwise symbols exist only in IMPERATIVE code; in
                        // prose they stay dropped (the old behavior).
                        '|' if matches!(self.mode, LexerMode::Imperative) => TokenType::VBar,
                        '~' if matches!(self.mode, LexerMode::Imperative) => TokenType::Tilde,
                        '^' if matches!(self.mode, LexerMode::Imperative) => TokenType::Caret,
                        ',' => TokenType::Comma,
                        ':' => TokenType::Colon,
                        '.' | '?' => {
                            self.in_let_context = false;
                            TokenType::Period
                        }
                        '!' => TokenType::Exclamation,
                        '+' => TokenType::Plus,
                        '-' => TokenType::Minus,
                        '*' => TokenType::Star,
                        '/' => TokenType::Slash,
                        '%' => TokenType::Percent,
                        '<' => TokenType::Lt,
                        '>' => TokenType::Gt,
                        '=' => TokenType::Assign,
                        _ => {
                            self.pos += 1;
                            continue;
                        }
                    };
                    let lexeme = self.interner.intern(&punct.to_string());
                    let span = Span::new(word_start, word_end);
                    // Collapse consecutive Periods — an abbreviation's own period
                    // followed by the sentence period ("Dorsey Assoc.." → "Assoc"
                    // + one Period). The extra "." would otherwise strand the parse.
                    let dup_period = kind == TokenType::Period
                        && tokens.last().map_or(false, |t: &Token| t.kind == TokenType::Period);
                    if !dup_period {
                        tokens.push(Token::new(kind, lexeme, span));
                    }
                }
                self.pos += 1;
                continue;
            }

            // Check for string literal marker (pre-tokenized in Stage 1)
            if word.starts_with("\x00STR:") {
                let content = &word[5..]; // Skip the marker prefix
                let span = Span::new(word_start, word_end);
                if Self::has_unescaped_brace(content) {
                    let sym = self.interner.intern(content);
                    tokens.push(Token::new(TokenType::InterpolatedString(sym), sym, span));
                } else {
                    // Collapse {{ → { and }} → } for plain strings
                    let normalized = content.replace("{{", "{").replace("}}", "}");
                    let sym = self.interner.intern(&normalized);
                    tokens.push(Token::new(TokenType::StringLiteral(sym), sym, span));
                }
                self.pos += 1;
                continue;
            }

            // Check for character literal marker
            if word.starts_with("\x00CHAR:") {
                let content = &word[6..]; // Skip the marker prefix
                let sym = self.interner.intern(content);
                let span = Span::new(word_start, word_end);
                tokens.push(Token::new(TokenType::CharLiteral(sym), sym, span));
                self.pos += 1;
                continue;
            }

            // Check for escape block marker (pre-captured raw foreign code)
            if word.starts_with("\x00ESC:") {
                let content = &word[5..]; // Skip the "\x00ESC:" prefix
                let sym = self.interner.intern(content);
                let span = Span::new(word_start, word_end);
                tokens.push(Token::new(TokenType::EscapeBlock(sym), sym, span));
                self.pos += 1;
                continue;
            }

            // "exactly N" / "precisely N" — a redundant exactness marker on a following
            // count or measure. The count/measure is already exact in the FOL (a value,
            // not a ≥/≤ bound), so the word adds no constraint; dropping it lets the
            // count object parse ("serves exactly 2 people" ≡ "serves 2 people") with
            // zero meaning loss. (Approximative "about N" is a Preposition and keeps its
            // own About-relation; "at least/most N" are handled separately as bounds.)
            if matches!(word.to_lowercase().as_str(), "exactly" | "precisely")
                && self.peek_word(1).map_or(false, |w| {
                    let w = w.trim_start_matches('$');
                    w.chars().next().map_or(false, |c| c.is_ascii_digit())
                        || crate::lexicon::word_to_number(&w.to_lowercase()).is_some()
                })
            {
                self.pos += 1;
                continue;
            }

            let kind = self.classify_with_lookahead(&word);
            // A subject pronoun or WH-relativizer takes the copula clitic "'s" = "is"
            // ("who's going", "he's …"), never the possessive 's (their genitive is the
            // dedicated form whose / its / his). Captured from the host's lexical
            // CATEGORY before `kind` is moved into the token, so the contraction rule
            // below is data-driven, not a word list.
            let host_takes_copula_clitic = matches!(
                kind,
                TokenType::Pronoun { case: crate::lexicon::Case::Subject, .. }
                    | TokenType::Who
                    | TokenType::That
                    | TokenType::What
                    | TokenType::Where
            );
            let lexeme = self.interner.intern(&word);
            let span = Span::new(word_start, word_end);
            tokens.push(Token::new(kind, lexeme, span));

            if let Some(punct) = trailing_punct {
                if punct == '\'' {
                    if let Some(next_item) = self.words.get(self.pos + 1) {
                        if next_item.word.to_lowercase() == "s" {
                            // The contraction "'s" = "is" iff the host takes the copula
                            // clitic AND the next word is not a past-participle/non-
                            // progressive verb — that case is the perfect "has"
                            // ("he's found/been"), a separate unsupported feature, and
                            // emitting "is" there would misread it as the passive "is
                            // found". Verb aspect comes from the lexicon (data-driven).
                            let next_is_past_verb = self
                                .words
                                .get(self.pos + 2)
                                .map(|a| a.word.to_lowercase())
                                .and_then(|w| self.lexicon.lookup_verb(&w))
                                .map_or(false, |v| v.aspect != crate::lexicon::Aspect::Progressive);
                            let (poss_kind, poss_text) = if host_takes_copula_clitic && !next_is_past_verb {
                                (TokenType::Is, "is")
                            } else {
                                (TokenType::Possessive, "'s")
                            };
                            let poss_lexeme = self.interner.intern(poss_text);
                            let poss_start = punct_pos.unwrap_or(word_end);
                            let poss_end = next_item.end;
                            tokens.push(Token::new(poss_kind, poss_lexeme, Span::new(poss_start, poss_end)));
                            self.pos += 1;
                            if let Some(s_punct) = next_item.trailing_punct {
                                let kind = match s_punct {
                                    '(' => TokenType::LParen,
                                    ')' => TokenType::RParen,
                                    '[' => TokenType::LBracket,
                                    ']' => TokenType::RBracket,
                                    '{' => TokenType::LBrace,
                                    '}' => TokenType::RBrace,
                                    '|' if matches!(self.mode, LexerMode::Imperative) => TokenType::VBar,
                                    '~' if matches!(self.mode, LexerMode::Imperative) => TokenType::Tilde,
                                    '^' if matches!(self.mode, LexerMode::Imperative) => TokenType::Caret,
                                    ',' => TokenType::Comma,
                                    ':' => TokenType::Colon,
                                    '.' | '?' => TokenType::Period,
                                    '!' => TokenType::Exclamation,
                                    '+' => TokenType::Plus,
                                    '-' => TokenType::Minus,
                                    '*' => TokenType::Star,
                                    '/' => TokenType::Slash,
                                    '%' => TokenType::Percent,
                                    '<' => TokenType::Lt,
                                    '>' => TokenType::Gt,
                                    '=' => TokenType::Assign,
                                    _ => {
                                        self.pos += 1;
                                        continue;
                                    }
                                };
                                let s_punct_pos = next_item.punct_pos.unwrap_or(next_item.end);
                                let lexeme = self.interner.intern(&s_punct.to_string());
                                tokens.push(Token::new(kind, lexeme, Span::new(s_punct_pos, s_punct_pos + 1)));
                            }
                            self.pos += 1;
                            continue;
                        }
                    }
                    self.pos += 1;
                    continue;
                }

                // An abbreviation's own dot ("Mr.", "Dr.", "153 ft.", "Mt.") is NOT a
                // sentence terminator when more text follows — emitting a Period there
                // strands the rest of the clause. Which words abbreviate is lexical
                // (lexicon `abbreviations`); suppress the dot only mid-clue, so a
                // genuine clue-final abbreviation ("…on Main St.") keeps its terminator.
                if punct == '.'
                    && lexicon::is_abbreviation(&word.to_lowercase())
                    && self.words.get(self.pos + 1).is_some()
                {
                    self.pos += 1;
                    continue;
                }
                let kind = match punct {
                    '(' => TokenType::LParen,
                    ')' => TokenType::RParen,
                    '[' => TokenType::LBracket,
                    ']' => TokenType::RBracket,
                    '{' => TokenType::LBrace,
                    '}' => TokenType::RBrace,
                    '|' if matches!(self.mode, LexerMode::Imperative) => TokenType::VBar,
                    '~' if matches!(self.mode, LexerMode::Imperative) => TokenType::Tilde,
                    '^' if matches!(self.mode, LexerMode::Imperative) => TokenType::Caret,
                    ',' => TokenType::Comma,
                    ':' => TokenType::Colon,
                    '.' | '?' => {
                        self.in_let_context = false;
                        TokenType::Period
                    }
                    '!' => TokenType::Exclamation,
                    '+' => TokenType::Plus,
                    '-' => TokenType::Minus,
                    '*' => TokenType::Star,
                    '/' => TokenType::Slash,
                    '%' => TokenType::Percent,
                    '<' => TokenType::Lt,
                    '>' => TokenType::Gt,
                    '=' => TokenType::Assign,
                    _ => {
                        self.pos += 1;
                        continue;
                    }
                };
                let p_start = punct_pos.unwrap_or(word_end);
                let lexeme = self.interner.intern(&punct.to_string());
                tokens.push(Token::new(kind, lexeme, Span::new(p_start, p_start + 1)));
            }

            self.pos += 1;
        }

        let eof_lexeme = self.interner.intern("");
        let eof_span = Span::new(self.input_len, self.input_len);
        tokens.push(Token::new(TokenType::EOF, eof_lexeme, eof_span));

        self.insert_indentation_tokens(tokens)
    }

    /// Insert Indent/Dedent tokens using LineLexer's two-pass architecture (Spec §2.5.2).
    ///
    /// Phase 1: LineLexer determines the structural layout (where indents/dedents occur)
    /// Phase 2: We correlate these with word token positions
    fn insert_indentation_tokens(&mut self, tokens: Vec<Token>) -> Vec<Token> {
        let mut result = Vec::new();
        let empty_sym = self.interner.intern("");

        // Phase 1: Run LineLexer to determine structural positions
        let line_lexer = LineLexer::new(&self.source);
        let line_tokens: Vec<LineToken> = line_lexer.collect();

        // Build a list of (byte_position, is_indent) for structural tokens
        // Position is where the NEXT Content starts after the Indent/Dedent
        let mut structural_events: Vec<(usize, bool)> = Vec::new(); // (byte_pos, true=Indent, false=Dedent)
        let mut pending_indents = 0usize;
        let mut pending_dedents = 0usize;

        for line_token in &line_tokens {
            match line_token {
                LineToken::Indent => {
                    pending_indents += 1;
                }
                LineToken::Dedent => {
                    pending_dedents += 1;
                }
                LineToken::Content { start, .. } => {
                    // Emit pending dedents first (they come BEFORE the content)
                    for _ in 0..pending_dedents {
                        structural_events.push((*start, false)); // false = Dedent
                    }
                    pending_dedents = 0;

                    // Emit pending indents (they also come BEFORE the content)
                    for _ in 0..pending_indents {
                        structural_events.push((*start, true)); // true = Indent
                    }
                    pending_indents = 0;
                }
                LineToken::Newline => {}
            }
        }

        // Handle any remaining dedents at EOF
        for _ in 0..pending_dedents {
            structural_events.push((self.input_len, false));
        }

        // Filter out structural events from within escape block bodies.
        // The LineLexer sees raw Rust code lines and generates spurious Indent/Dedent
        // events for their indentation changes. We keep exactly the boundary events
        // (Indent at body start, Dedent at body end) but remove internal ones.
        if !self.escape_body_ranges.is_empty() {
            // For each escape body range, find the first Indent at the body start and
            // track that we're inside the range. Filter out all events strictly inside
            // the range except for the first Indent and events at/after the end.
            let mut filtered = Vec::new();
            for &(pos, is_indent) in &structural_events {
                let is_inside_escape_body = self.escape_body_ranges.iter().any(|(start, end)| {
                    // Strictly inside the body (not at start boundary and not at/after end)
                    pos > *start && pos < *end
                });
                if !is_inside_escape_body {
                    filtered.push((pos, is_indent));
                }
            }
            structural_events = filtered;
        }

        // Filter out structural events from within multi-line string literals.
        // Triple-quote strings span multiple lines; their internal indentation
        // must not generate Indent/Dedent tokens.
        {
            let string_spans: Vec<(usize, usize)> = tokens.iter()
                .filter(|t| matches!(t.kind, TokenType::StringLiteral(_) | TokenType::InterpolatedString(_)))
                .filter(|t| t.span.end.saturating_sub(t.span.start) > 6) // only multi-line strings (""" adds >=6 chars)
                .map(|t| (t.span.start, t.span.end))
                .collect();
            if !string_spans.is_empty() {
                structural_events.retain(|&(pos, _)| {
                    !string_spans.iter().any(|(start, end)| pos > *start && pos < *end)
                });
            }
        }

        // Bracket line-continuation: a collection literal / call / parenthesised expression may span
        // lines with the continuation lines INDENTED (`[\n    1,\n    2,\n]`). The LineLexer sees that
        // indentation and emits Indent/Dedent, which would break element parsing. Drop every
        // structural event that falls strictly inside an unclosed `(`/`[`/`{` … `)`/`]`/`}` span. Both
        // the opening Indent and the matching Dedent lie inside the span, so they are dropped as a
        // balanced pair — the enclosing block level is preserved.
        {
            let mut bracket_ranges: Vec<(usize, usize)> = Vec::new();
            let mut open_stack: Vec<usize> = Vec::new();
            for t in &tokens {
                match t.kind {
                    TokenType::LParen | TokenType::LBracket | TokenType::LBrace => {
                        open_stack.push(t.span.start);
                    }
                    TokenType::RParen | TokenType::RBracket | TokenType::RBrace => {
                        if let Some(open) = open_stack.pop() {
                            bracket_ranges.push((open, t.span.end));
                        }
                    }
                    _ => {}
                }
            }
            if !bracket_ranges.is_empty() {
                structural_events.retain(|&(pos, _)| {
                    !bracket_ranges.iter().any(|(start, end)| pos > *start && pos < *end)
                });
            }
        }

        // Sort events by position, with dedents before indents at same position
        structural_events.sort_by(|a, b| {
            if a.0 != b.0 {
                a.0.cmp(&b.0)
            } else {
                // Dedents (false) before Indents (true) at same position
                a.1.cmp(&b.1)
            }
        });

        // Phase 2: Insert structural tokens at the right positions
        // Strategy: For each word token, check if any structural events should be inserted
        // before it (based on byte position)

        let mut event_idx = 0;
        let mut last_colon_pos: Option<usize> = None;

        for token in tokens.iter() {
            let token_start = token.span.start;

            // Insert any structural tokens that should come BEFORE this token
            while event_idx < structural_events.len() {
                let (event_pos, is_indent) = structural_events[event_idx];

                // Insert structural tokens before this token if the event position <= token start
                if event_pos <= token_start {
                    let span = if is_indent {
                        // Indent is inserted after the preceding Colon
                        Span::new(last_colon_pos.unwrap_or(event_pos), last_colon_pos.unwrap_or(event_pos))
                    } else {
                        Span::new(event_pos, event_pos)
                    };
                    let kind = if is_indent { TokenType::Indent } else { TokenType::Dedent };
                    result.push(Token::new(kind, empty_sym, span));
                    event_idx += 1;
                } else {
                    break;
                }
            }

            result.push(token.clone());

            // Track colon positions for Indent span calculation
            if token.kind == TokenType::Colon && self.is_end_of_line(token.span.end) {
                last_colon_pos = Some(token.span.end);
            }
        }

        // Insert any remaining structural tokens (typically Dedents at EOF)
        while event_idx < structural_events.len() {
            let (event_pos, is_indent) = structural_events[event_idx];
            let span = Span::new(event_pos, event_pos);
            let kind = if is_indent { TokenType::Indent } else { TokenType::Dedent };
            result.push(Token::new(kind, empty_sym, span));
            event_idx += 1;
        }

        // Ensure EOF is at the end
        let eof_pos = result.iter().position(|t| t.kind == TokenType::EOF);
        if let Some(pos) = eof_pos {
            let eof = result.remove(pos);
            result.push(eof);
        }

        result
    }

    /// Check if position is at end of line (only whitespace until newline)
    fn is_end_of_line(&self, from_pos: usize) -> bool {
        let bytes = self.source.as_bytes();
        let mut pos = from_pos;
        while pos < bytes.len() {
            match bytes[pos] {
                b' ' | b'\t' => pos += 1,
                b'\n' => return true,
                _ => return false,
            }
        }
        true // End of input is also end of line
    }

    fn measure_next_line_indent(&self, from_pos: usize) -> Option<usize> {
        let bytes = self.source.as_bytes();
        let mut pos = from_pos;

        while pos < bytes.len() && bytes[pos] != b'\n' {
            pos += 1;
        }

        if pos >= bytes.len() {
            return None;
        }

        pos += 1;

        let mut indent = 0;
        while pos < bytes.len() {
            match bytes[pos] {
                b' ' => indent += 1,
                b'\t' => indent += 4,
                b'\n' => {
                    indent = 0;
                }
                _ => break,
            }
            pos += 1;
        }

        if pos >= bytes.len() {
            return None;
        }

        Some(indent)
    }

    fn word_to_number(word: &str) -> Option<u32> {
        lexicon::word_to_number(&word.to_lowercase())
    }

    /// Check if a hyphen at the current position is part of an ISO-8601 date.
    ///
    /// Detects patterns like:
    /// - "2026-" followed by "05-20" → first hyphen of date
    /// - "2026-05-" followed by "20" → second hyphen of date
    fn is_date_hyphen(current_word: &str, chars: &[char], char_idx: usize) -> bool {
        // Current word must be all digits (year or year-month)
        let word_chars: Vec<char> = current_word.chars().collect();

        // Check for first hyphen pattern: YYYY- followed by MM-DD
        if word_chars.len() == 4 && word_chars.iter().all(|c| c.is_ascii_digit()) {
            // Check if followed by exactly 2 digits, hyphen, 2 digits
            if char_idx + 5 < chars.len()
                && chars[char_idx + 1].is_ascii_digit()
                && chars[char_idx + 2].is_ascii_digit()
                && chars[char_idx + 3] == '-'
                && chars[char_idx + 4].is_ascii_digit()
                && chars[char_idx + 5].is_ascii_digit()
            {
                return true;
            }
        }

        // Check for second hyphen pattern: YYYY-MM- followed by DD
        if word_chars.len() == 7
            && word_chars[0..4].iter().all(|c| c.is_ascii_digit())
            && word_chars[4] == '-'
            && word_chars[5..7].iter().all(|c| c.is_ascii_digit())
        {
            // Check if followed by exactly 2 digits
            if char_idx + 2 < chars.len()
                && chars[char_idx + 1].is_ascii_digit()
                && chars[char_idx + 2].is_ascii_digit()
            {
                // Make sure we're not followed by more digits (would be a longer number)
                let next_not_digit = char_idx + 3 >= chars.len()
                    || !chars[char_idx + 3].is_ascii_digit();
                if next_not_digit {
                    return true;
                }
            }
        }

        false
    }

    /// Check if a colon is part of a time literal (e.g., 9:30am, 11:45pm).
    ///
    /// Detects patterns like:
    /// - "9:" followed by "30am" or "30pm"
    /// - "11:" followed by "45pm"
    fn is_time_colon(current_word: &str, chars: &[char], char_idx: usize) -> bool {
        // Current word must be 1-2 digits (hour)
        let word_chars: Vec<char> = current_word.chars().collect();
        if word_chars.is_empty() || word_chars.len() > 2 {
            return false;
        }
        if !word_chars.iter().all(|c| c.is_ascii_digit()) {
            return false;
        }

        // Followed by exactly 2 digits (the minutes), then "am"/"pm" — which may
        // come immediately ("9:30am") or after a space ("8:15 pm").
        if char_idx + 3 < chars.len()
            && chars[char_idx + 1].is_ascii_digit()
            && chars[char_idx + 2].is_ascii_digit()
        {
            let suffix_start = if chars.get(char_idx + 3) == Some(&' ') {
                char_idx + 4
            } else {
                char_idx + 3
            };
            if suffix_start + 1 < chars.len() {
                let next_two: String = chars[suffix_start..suffix_start + 2].iter().collect();
                let lower = next_two.to_lowercase();
                if (lower == "am" || lower == "pm")
                    && (suffix_start + 2 >= chars.len()
                        || !chars[suffix_start + 2].is_alphabetic())
                {
                    return true;
                }
            }
        }

        false
    }

    /// Whether a space separates a `H:MM` clock time from a trailing "am"/"pm"
    /// ("8:15 pm") — the space is part of the time literal, so the tokenizer keeps
    /// "am"/"pm" attached to the time rather than splitting on it.
    fn is_time_space_before_ampm(current_word: &str, chars: &[char], char_idx: usize) -> bool {
        let valid_time = match current_word.find(':') {
            Some(p) => {
                let hour = &current_word[..p];
                let min = &current_word[p + 1..];
                (1..=2).contains(&hour.len())
                    && hour.chars().all(|c| c.is_ascii_digit())
                    && min.len() == 2
                    && min.chars().all(|c| c.is_ascii_digit())
            }
            None => false,
        };
        if !valid_time {
            return false;
        }
        let next_two: String = chars
            .get(char_idx + 1..char_idx + 3)
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        let lower = next_two.to_lowercase();
        (lower == "am" || lower == "pm")
            && chars.get(char_idx + 3).map_or(true, |c| !c.is_alphabetic())
    }

    /// Check if a string contains an unescaped `{` (i.e., not part of `{{`).
    /// Used to distinguish `InterpolatedString` from `StringLiteral`.
    fn has_unescaped_brace(content: &str) -> bool {
        let bytes = content.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                    i += 2;
                } else {
                    return true;
                }
            } else {
                i += 1;
            }
        }
        false
    }

    /// Check if a `+` or `-` at the current position is the sign of a scientific notation exponent.
    ///
    /// Detects patterns like:
    /// - "4.84e+" followed by "00" → exponent sign in `4.84e+00`
    /// - "2.5e-" followed by "2"  → exponent sign in `2.5e-2`
    fn is_exponent_sign(current_word: &str, chars: &[char], char_idx: usize) -> bool {
        // Word must end with e/E
        if !current_word.ends_with('e') && !current_word.ends_with('E') {
            return false;
        }
        // Before e/E must contain a digit (ensures it's a number, not a bare "e")
        let before_e = &current_word[..current_word.len() - 1];
        if before_e.is_empty() || !before_e.chars().next().unwrap().is_ascii_digit() {
            return false;
        }
        // Next char must be a digit (the exponent value)
        char_idx + 1 < chars.len() && chars[char_idx + 1].is_ascii_digit()
    }

    /// Dedent a triple-quoted string: strip the common leading whitespace from each line.
    /// Joins lines with literal newline characters (not escape sequences).
    fn dedent_triple_quote(raw: &str) -> String {
        let lines: Vec<&str> = raw.lines().collect();
        if lines.is_empty() {
            return String::new();
        }
        // Find minimum indentation of non-empty lines
        let min_indent = lines.iter()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.len() - l.trim_start().len())
            .min()
            .unwrap_or(0);
        // Strip that indentation and join with actual newlines
        lines.iter()
            .map(|l| {
                if l.len() >= min_indent {
                    &l[min_indent..]
                } else {
                    l.trim()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn is_numeric_literal(word: &str) -> bool {
        if word.is_empty() {
            return false;
        }
        let chars: Vec<char> = word.chars().collect();
        let first = chars[0];
        if first.is_ascii_digit() {
            // Numeric literal: starts with digit (may have underscore separators like 1_000)
            return true;
        }
        // Symbolic numbers: only recognize known mathematical symbols
        // (aleph, omega, beth) followed by underscore and digits
        if let Some(underscore_pos) = word.rfind('_') {
            let before_underscore = &word[..underscore_pos];
            let after_underscore = &word[underscore_pos + 1..];
            // Must be a known mathematical symbol prefix AND digits after underscore
            let is_math_symbol = matches!(
                before_underscore.to_lowercase().as_str(),
                "aleph" | "omega" | "beth"
            );
            if is_math_symbol
                && !after_underscore.is_empty()
                && after_underscore.chars().all(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
        false
    }

    /// Parse a duration literal with SI suffix.
    ///
    /// Returns Some((nanoseconds, unit_str)) if the word is a valid duration literal,
    /// None otherwise.
    ///
    /// Supported suffixes:
    /// - ns: nanoseconds
    /// - us, μs: microseconds
    /// - ms: milliseconds
    /// - s, sec: seconds
    /// - min: minutes
    /// - h, hr: hours
    fn parse_duration_literal(word: &str) -> Option<(i64, &str)> {
        if word.is_empty() || !word.chars().next()?.is_ascii_digit() {
            return None;
        }

        // SI suffix table with multipliers to nanoseconds
        const SUFFIXES: &[(&str, i64)] = &[
            ("ns", 1),
            ("μs", 1_000),
            ("us", 1_000),
            ("ms", 1_000_000),
            ("sec", 1_000_000_000),
            ("s", 1_000_000_000),
            ("min", 60_000_000_000),
            ("hr", 3_600_000_000_000),
            ("h", 3_600_000_000_000),
        ];

        // Try each suffix (longer suffixes first to avoid partial matches)
        for (suffix, multiplier) in SUFFIXES {
            if word.ends_with(suffix) {
                let num_part = &word[..word.len() - suffix.len()];
                // Parse the numeric part (may have underscore separators)
                let cleaned: String = num_part.chars().filter(|c| *c != '_').collect();
                if let Ok(n) = cleaned.parse::<i64>() {
                    return Some((n.saturating_mul(*multiplier), *suffix));
                }
            }
        }

        None
    }

    /// Parse an ISO-8601 date literal (YYYY-MM-DD).
    ///
    /// Returns Some(days_since_epoch) if the word is a valid date literal,
    /// None otherwise.
    fn parse_date_literal(word: &str) -> Option<i32> {
        // Must match pattern: YYYY-MM-DD
        if word.len() != 10 {
            return None;
        }

        let bytes = word.as_bytes();

        // Check format: 4 digits, hyphen, 2 digits, hyphen, 2 digits
        if bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }

        // Parse year, month, day
        let year: i32 = word[0..4].parse().ok()?;
        let month: u32 = word[5..7].parse().ok()?;
        let day: u32 = word[8..10].parse().ok()?;

        // Reject calendar-impossible dates (e.g. 2026-02-30, 2026-04-31): the
        // Howard Hinnant day count below would otherwise silently map them onto
        // a real but DIFFERENT day.
        if month < 1 || month > 12 || day < 1 {
            return None;
        }
        let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let max_day = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => if is_leap { 29 } else { 28 },
            _ => return None,
        };
        if day > max_day {
            return None;
        }

        // Convert to days since Unix epoch using Howard Hinnant's algorithm
        // https://howardhinnant.github.io/date_algorithms.html
        let y = if month <= 2 { year - 1 } else { year };
        let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
        let yoe = (y - era * 400) as u32;
        let m = month;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146097 + doe as i32 - 719468;

        Some(days)
    }

    /// Parse a time-of-day literal.
    ///
    /// Supported formats:
    /// - 12-hour with am/pm: "4pm", "9am", "12pm"
    /// - 12-hour with minutes: "9:30am", "11:45pm"
    /// - Special words: "noon" (12:00), "midnight" (00:00)
    ///
    /// Returns Some(nanos_from_midnight) if valid, None otherwise.
    fn parse_time_literal(word: &str) -> Option<i64> {
        let lower = word.to_lowercase();

        // Handle special time words
        if lower == "noon" {
            return Some(12i64 * 3600 * 1_000_000_000);
        }
        if lower == "midnight" {
            return Some(0);
        }

        // Handle 12-hour formats: "4pm", "9am", "9:30am", "11:45pm"
        let is_pm = lower.ends_with("pm");
        let is_am = lower.ends_with("am");

        if !is_pm && !is_am {
            return None;
        }

        // Strip the am/pm suffix
        let time_part = &lower[..lower.len() - 2];

        // Check for hour:minute format
        let (hour, minute): (i64, i64) = if let Some(colon_idx) = time_part.find(':') {
            let hour_str = &time_part[..colon_idx];
            let min_str = &time_part[colon_idx + 1..];
            let h: i64 = hour_str.parse().ok()?;
            let m: i64 = min_str.parse().ok()?;
            (h, m)
        } else {
            // Just hour: "4pm", "9am"
            let h: i64 = time_part.parse().ok()?;
            (h, 0)
        };

        // Validate ranges
        if hour < 1 || hour > 12 || minute < 0 || minute > 59 {
            return None;
        }

        // Convert to 24-hour format
        let hour_24 = if is_am {
            if hour == 12 { 0 } else { hour }  // 12am = midnight = 0
        } else {
            if hour == 12 { 12 } else { hour + 12 }  // 12pm = noon = 12, 4pm = 16
        };

        // Convert to nanoseconds from midnight
        let nanos = (hour_24 * 3600 + minute * 60) * 1_000_000_000;
        Some(nanos)
    }

    /// True for the currency symbols that prefix a money literal (`$ € £ ¥`).
    fn is_currency_symbol(c: char) -> bool {
        matches!(c, '$' | '€' | '£' | '¥')
    }

    /// The ISO-4217 code a leading currency symbol denotes — `$`→USD, `€`→EUR, `£`→GBP, `¥`→JPY (the
    /// dominant reading of each symbol). Unknown symbols yield `None`.
    fn currency_for_symbol(c: char) -> Option<&'static str> {
        Some(match c {
            '$' => "USD",
            '€' => "EUR",
            '£' => "GBP",
            '¥' => "JPY",
            _ => return None,
        })
    }

    fn classify_with_lookahead(&mut self, word: &str) -> TokenType {
        // The `&` character, deferred by the word-splitter: prose keeps the
        // coordination reading (`black & red` → and); imperative code gets
        // the bitwise operator.
        if word == "\x00AMP" {
            return if matches!(self.mode, LexerMode::Imperative) {
                TokenType::Amp
            } else {
                TokenType::And
            };
        }
        if word == "\x00DOT" {
            // Imperative: the field-access / UFCS operator. Declarative/prose:
            // a plain sentence period (so `e.g.` and abbreviations read as today).
            return if matches!(self.mode, LexerMode::Imperative) {
                TokenType::Dot
            } else {
                TokenType::Period
            };
        }
        // Handle block headers (##Theorem, ##Main, etc.)
        if word.starts_with("##") {
            let block_name = &word[2..];
            let block_type = match block_name.to_lowercase().as_str() {
                "theorem" => BlockType::Theorem,
                "main" => BlockType::Main,
                "definition" => BlockType::Definition,
                "define" => BlockType::Define,  // Vernacular-logic predicate definition (Rung 0a)
                "axiom" => BlockType::Axiom,    // Formal first-order axiom (the seam for Tarski)
                "theory" => BlockType::Theory,  // Named development grouping axioms + theorems
                "proof" => BlockType::Proof,
                "example" => BlockType::Example,
                "logic" => BlockType::Logic,
                "note" => BlockType::Note,
                "to" => BlockType::Function,  // Function definition block
                "a" | "an" => BlockType::TypeDef,  // Inline type definitions: ## A Point has:
                "policy" => BlockType::Policy,  // Security policy definitions
                "requires" => BlockType::Requires,  // External crate dependencies
                "hardware" => BlockType::Hardware,  // Signal declarations
                "property" => BlockType::Property,  // Temporal assertions
                "no" => BlockType::No,  // Optimization annotation: ## No Memo, ## No TCO, etc.
                "tier" => BlockType::Tier,  // Tiered-optimizer pin: ## Tier specialize eager, etc.
                other => {
                    // A near-miss of a CONSEQUENTIAL header is a probable
                    // typo — `## Mian` silently becoming prose is the bug
                    // class where a whole program runs to empty output.
                    // Prose-type names (note/example/logic) and the short
                    // forms are excluded, so ordinary literate headings
                    // (`## Notes`, `## Design`) keep working.
                    const CONSEQUENTIAL: &[&str] = &[
                        "main", "theorem", "definition", "define", "axiom",
                        "theory", "proof", "policy", "requires", "hardware",
                        "property", "tier",
                    ];
                    if let Some(similar) =
                        crate::suggest::find_similar(other, CONSEQUENTIAL, 2)
                    {
                        let found = self.interner.intern(block_name);
                        let suggestion = self.interner.intern(similar);
                        BlockType::SuspectedTypo { found, suggestion }
                    } else {
                        BlockType::Note // Unknown block types stay literate prose
                    }
                }
            };

            // Update lexer mode based on block type
            self.mode = match block_type {
                BlockType::Main | BlockType::Function => LexerMode::Imperative,
                _ => LexerMode::Declarative,
            };


            return TokenType::BlockHeader { block_type };
        }

        let lower = word.to_lowercase();

        if lower == "each" && self.peek_sequence(&["other"]) {
            self.consume_words(1);
            return TokenType::Reciprocal;
        }

        if lower == "to" {
            if let Some(next) = self.peek_word(1) {
                if self.is_verb_like(next) {
                    return TokenType::To;
                }
            }
            let sym = self.interner.intern("to");
            return TokenType::Preposition(sym);
        }

        if lower == "at" {
            if let Some(next) = self.peek_word(1) {
                let next_lower = next.to_lowercase();
                if next_lower == "least" {
                    if let Some(num_word) = self.peek_word(2) {
                        if let Some(n) = Self::word_to_number(num_word) {
                            self.consume_words(2);
                            return TokenType::AtLeast(n);
                        }
                    }
                }
                if next_lower == "most" {
                    if let Some(num_word) = self.peek_word(2) {
                        if let Some(n) = Self::word_to_number(num_word) {
                            self.consume_words(2);
                            return TokenType::AtMost(n);
                        }
                    }
                }
            }
        }

        // "Exactly N" → Cardinal(N) — same as bare number but explicit
        if lower == "exactly" {
            if let Some(num_word) = self.peek_word(1) {
                if let Some(n) = Self::word_to_number(num_word) {
                    self.consume_words(1);
                    return TokenType::Cardinal(n);
                }
            }
        }

        if let Some(n) = Self::word_to_number(&lower) {
            return TokenType::Cardinal(n);
        }

        // Check for duration literal first (e.g., "500ms", "2s", "50ns")
        if let Some((nanos, unit)) = Self::parse_duration_literal(word) {
            let unit_sym = self.interner.intern(unit);
            return TokenType::DurationLiteral {
                nanos,
                original_unit: unit_sym,
            };
        }

        // Check for ISO-8601 date literal (e.g., "2026-05-20")
        if let Some(days) = Self::parse_date_literal(word) {
            return TokenType::DateLiteral { days };
        }

        // Check for time-of-day literal (e.g., "4pm", "9:30am", "noon", "midnight")
        if let Some(nanos_from_midnight) = Self::parse_time_literal(word) {
            return TokenType::TimeLiteral { nanos_from_midnight };
        }

        // Currency-symbol money literal: `$19.99`, `€5`, `£10`, `¥100`, `$1,250.50`. The symbol
        // resolves to its ISO-4217 code; the magnitude keeps its digits and decimal point (thousands
        // separators stripped). A `MoneyLiteral` carries both, so the parser builds an exact
        // currency-tagged value rather than dropping the symbol. ONLY in imperative code — in
        // natural-language clauses `$25,000` is a bare magnitude (a logic-grid constraint), handled
        // by the magnitude-stripping block below; the symbol there is just orthography.
        if self.mode == LexerMode::Imperative {
            if let Some(first) = word.chars().next() {
                if let Some(code) = Self::currency_for_symbol(first) {
                    let cleaned: String =
                        word.chars().skip(1).filter(|c| c.is_ascii_digit() || *c == '.').collect();
                    if cleaned.starts_with(|c: char| c.is_ascii_digit()) {
                        let amount = self.interner.intern(&cleaned);
                        let currency = self.interner.intern(code);
                        return TokenType::MoneyLiteral { amount, currency };
                    }
                }
            }
        }

        // Currency / comma-grouped numerals: "$125,000", "€2.60", "1,234". Strip the currency
        // marker and thousands separators; in a natural-language clause the puzzle constraint is the
        // bare magnitude (125000, 2.60). (Imperative code took the `MoneyLiteral` path above.)
        if word.starts_with(|c: char| Self::is_currency_symbol(c))
            || (word.starts_with(|c: char| c.is_ascii_digit()) && word.contains(','))
        {
            let cleaned: String = word
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if cleaned.starts_with(|c: char| c.is_ascii_digit()) {
                let sym = self.interner.intern(&cleaned);
                return TokenType::Number(sym);
            }
        }

        if Self::is_numeric_literal(word) {
            let sym = self.interner.intern(word);
            return TokenType::Number(sym);
        }

        if lower == "if" && self.peek_sequence(&["and", "only", "if"]) {
            self.consume_words(3);
            return TokenType::Iff;
        }

        if lower == "is" {
            if self.peek_sequence(&["equal", "to"]) {
                self.consume_words(2);
                return TokenType::Identity;
            }
            if self.peek_sequence(&["identical", "to"]) {
                self.consume_words(2);
                return TokenType::Identity;
            }
        }

        if (lower == "a" || lower == "an") && word.chars().next().unwrap().is_uppercase() {
            // Capitalized "A" or "An" - disambiguate article vs proper name
            // Heuristic: articles are followed by nouns/adjectives, not verbs or keywords
            if let Some(next) = self.peek_word(1) {
                let next_lower = next.to_lowercase();
                let next_starts_lowercase = next.chars().next().map(|c| c.is_lowercase()).unwrap_or(false);

                // If followed by logical keyword, treat as proper name (propositional variable)
                if matches!(next_lower.as_str(), "if" | "and" | "or" | "implies" | "iff") {
                    let sym = self.interner.intern(word);
                    return TokenType::ProperName(sym);
                }

                // If next word is ONLY a verb (like "has", "is", "ran"), A is likely a name
                // Exception: gerunds (like "running") can follow articles
                // Exception: words in disambiguation_not_verbs (like "red") are not verbs
                // Exception: words that are also nouns/adjectives (like "fire") can follow articles
                let is_verb = self.lexicon.lookup_verb(&next_lower).is_some()
                    && !lexicon::is_disambiguation_not_verb(&next_lower);
                let is_gerund = next_lower.ends_with("ing");
                let is_also_noun_or_adj = self.is_noun_like(&next_lower) || self.is_adjective_like(&next_lower);
                if is_verb && !is_gerund && !is_also_noun_or_adj {
                    let sym = self.interner.intern(word);
                    return TokenType::ProperName(sym);
                }

                // Definition pattern: "A [TypeName] is a..." or "A [TypeName] has:" - treat A as article
                // even when TypeName is capitalized and unknown
                if let Some(third) = self.peek_word(2) {
                    let third_lower = third.to_lowercase();
                    // "has" for struct definitions: "A Point has:"
                    if third_lower == "is" || third_lower == "are" || third_lower == "has" {
                        return TokenType::Article(Definiteness::Indefinite);
                    }
                }

                // It's an article if next word is:
                // - A known noun or adjective, or
                // - Lowercase (likely a common word we don't recognize)
                let is_content_word = self.is_noun_like(&next_lower) || self.is_adjective_like(&next_lower);
                if is_content_word || next_starts_lowercase {
                    return TokenType::Article(Definiteness::Indefinite);
                }
            }
            let sym = self.interner.intern(word);
            return TokenType::ProperName(sym);
        }

        self.classify_word(word)
    }

    fn is_noun_like(&self, word: &str) -> bool {
        if lexicon::is_noun_pattern(word) || lexicon::is_common_noun(word) {
            return true;
        }
        if word.ends_with("er") || word.ends_with("ian") || word.ends_with("ist") {
            return true;
        }
        false
    }

    fn is_adjective_like(&self, word: &str) -> bool {
        lexicon::is_adjective(word) || lexicon::is_non_intersective(word)
    }

    fn classify_word(&mut self, word: &str) -> TokenType {
        let lower = word.to_lowercase();
        let first_char = word.chars().next().unwrap();

        // Disambiguate "that" as determiner vs complementizer/relativizer.
        // "that dog" → Article(Distal); "I know that he ran" → That.
        // After a nominal antecedent ("the vessel that saw …"), "that" heads a
        // relative clause even when the clause verb is also noun-like ("saw"),
        // so it must NOT collapse to a demonstrative there.
        if lower == "that" {
            if let Some(next) = self.peek_word(1) {
                let next_lower = next.to_lowercase();
                // A verb-capable next word ("that saw …", "that played …") makes
                // "that" a relativizer/complementizer even when that word is also
                // noun-like — the demonstrative reading needs a pure-noun head.
                let next_is_verb = self.lexicon.lookup_verb(&next_lower).is_some();
                if !next_is_verb
                    && (self.is_noun_like(&next_lower) || self.is_adjective_like(&next_lower))
                {
                    return TokenType::Article(Definiteness::Distal);
                }
            }
        }

        // Arrow token for return type syntax
        if word == "->" {
            return TokenType::Arrow;
        }

        // Grand Challenge: Comparison operator tokens
        if word == "<=" {
            return TokenType::LtEq;
        }
        if word == ">=" {
            return TokenType::GtEq;
        }
        match word {
            "+=" => return TokenType::PlusEq,
            "-=" => return TokenType::MinusEq,
            "*=" => return TokenType::StarEq,
            "/=" => return TokenType::SlashEq,
            "%=" => return TokenType::PercentEq,
            "**" => return TokenType::StarStar,
            "//" => return TokenType::SlashSlash,
            _ => {}
        }
        if word == "==" {
            return TokenType::EqEq;
        }
        if word == "!=" {
            return TokenType::NotEq;
        }
        if word == "<" {
            return TokenType::Lt;
        }
        if word == ">" {
            return TokenType::Gt;
        }
        // Single = for assignment (must come after == check)
        if word == "=" {
            return TokenType::Assign;
        }

        if let Some(kind) = lexicon::lookup_keyword(&lower) {
            // A capitalized modal/auxiliary MID-sentence is a proper name, not the
            // function word ("the startup started by Will Waters", "the May deadline").
            // Modals/auxiliaries never capitalize mid-clause, so the keyword reading is
            // spurious there; sentence-initial stays the keyword (a question "Will Alice
            // win?"). This is the same name/keyword collision resolved for item/native.
            let is_modal = matches!(
                kind,
                TokenType::Must
                    | TokenType::Shall
                    | TokenType::Should
                    | TokenType::Can
                    | TokenType::May
                    | TokenType::Cannot
                    | TokenType::Would
                    | TokenType::Could
                    | TokenType::Might
            );
            if is_modal && first_char.is_uppercase() && !self.prev_word_ends_sentence() {
                return TokenType::ProperName(self.interner.intern(word));
            }
            // In Declarative (NL) mode, "from" and "for" are prepositions, not Logos keywords.
            // They are listed in the prepositions section of the lexicon and will be
            // correctly re-classified by the is_preposition() check below if we skip here.
            let kind = match (kind, self.mode) {
                (TokenType::From, LexerMode::Declarative) => {
                    // A QUALIFIED IMPORT "Type from Module" — both neighbours are
                    // capitalised identifiers — keeps the `From` keyword even in
                    // declarative text (there are zero "Capitalised from
                    // Capitalised" sequences in NL clues). Otherwise "from" is an
                    // ordinary NL preposition ("the species FROM Australia").
                    let prev_cap = self.pos > 0
                        && self
                            .words
                            .get(self.pos - 1)
                            .and_then(|w| w.word.chars().next())
                            .map_or(false, |c| c.is_uppercase());
                    let next_cap = self
                        .words
                        .get(self.pos + 1)
                        .and_then(|w| w.word.chars().next())
                        .map_or(false, |c| c.is_uppercase());
                    if prev_cap && next_cap {
                        TokenType::From
                    } else {
                        let sym = self.interner.intern("from");
                        TokenType::Preposition(sym)
                    }
                }
                (TokenType::For, LexerMode::Declarative) => {
                    let sym = self.interner.intern("for");
                    TokenType::Preposition(sym)
                }
                (other, _) => other,
            };
            return kind;
        }

        if let Some(kind) = lexicon::lookup_pronoun(&lower) {
            return kind;
        }

        if let Some(def) = lexicon::lookup_article(&lower) {
            return TokenType::Article(def);
        }

        if let Some(time) = lexicon::lookup_auxiliary(&lower) {
            // A capitalized auxiliary MID-sentence is a proper name ("started by Will
            // Waters"), not the function word — auxiliaries never capitalize mid-clause.
            if first_char.is_uppercase() && !self.prev_word_ends_sentence() {
                return TokenType::ProperName(self.interner.intern(word));
            }
            return TokenType::Auxiliary(time);
        }

        // Handle imperative keywords that might conflict with prepositions
        match lower.as_str() {
            "call" => return TokenType::Call,
            "in" if self.mode == LexerMode::Imperative => return TokenType::In,
            // Zone keywords (must come before is_preposition check)
            "inside" if self.mode == LexerMode::Imperative => return TokenType::Inside,
            // "at" for chunk access (must come before is_preposition check)
            "at" if self.mode == LexerMode::Imperative => return TokenType::At,
            // "into" for pipe send (must come before is_preposition check)
            "into" if self.mode == LexerMode::Imperative => return TokenType::Into,
            // Temporal span operator (must come before is_preposition check)
            "before" => return TokenType::Before,
            _ => {}
        }

        // "per" is the rate preposition ("$2.50 per pound", "10 miles per
        // hour"). It is not in the general preposition lexicon, and must not be
        // mistaken for a measure unit or an unknown noun.
        if lower == "per" {
            let sym = self.interner.intern("per");
            return TokenType::Preposition(sym);
        }

        // A word that is BOTH a verb and a preposition ("like") stays
        // ambiguous — the dual-class block below emits the alternatives.
        // Lexicon-disambiguated NON-verbs ("during") are pure prepositions
        // even when morphology over-derives a verb reading.
        if lexicon::is_preposition(&lower)
            && (self.lexicon.lookup_verb(&lower).is_none()
                || lexicon::is_disambiguation_not_verb(&lower))
        {
            let sym = self.interner.intern(&lower);
            return TokenType::Preposition(sym);
        }

        match lower.as_str() {
            "equals" => return TokenType::Equals,
            // "item"/"items" is the indexing keyword in imperative code (where a
            // variable index like "item i of arr" is the whole point and prose nouns
            // never appear), and in declarative text ONLY when an index number follows
            // ("item 1 of list", "items 2 through 5"). Otherwise, in declarative text,
            // it is the ordinary English noun ("the blue item", "the item made of
            // gold"), which leaked as the keyword before and stranded the noun after an
            // adjective. Mode separates code from prose; the following number is the
            // finer signal within prose.
            "item" | "items"
                if self.mode == LexerMode::Imperative
                    || self.peek_word(1).map_or(false, |w| {
                        w.chars().next().map_or(false, |c| c.is_ascii_digit())
                            || crate::lexicon::word_to_number(&w.to_lowercase()).is_some()
                    }) =>
            {
                return if lower == "item" {
                    TokenType::Item
                } else {
                    TokenType::Items
                };
            }
            // Mutability keyword for `mut x = 5` syntax
            "mut" if self.mode == LexerMode::Imperative => return TokenType::Mut,
            "let" => {
                self.in_let_context = true;
                return TokenType::Let;
            }
            "set" => {
                // Check if "set" is used as a type (followed by "of") - "Set of Int"
                // This takes priority over the assignment keyword
                if self.peek_word(1).map_or(false, |w| w.to_lowercase() == "of") {
                    // It's a type like "Set of Int" - don't return keyword, let it be a noun
                } else if self.mode == LexerMode::Imperative {
                    // In Imperative mode, treat "set" as the assignment keyword
                    return TokenType::Set;
                } else {
                    // In Declarative mode, check positions 2-5 for "to"
                    // (handles field access like "set p's x to")
                    for offset in 2..=5 {
                        if self.peek_word(offset).map_or(false, |w| w.to_lowercase() == "to") {
                            return TokenType::Set;
                        }
                    }
                }
            }
            "return" => return TokenType::Return,
            "break" => return TokenType::Break,
            "xor" => return TokenType::Xor,
            "shifted" => return TokenType::Shifted,
            "be" if self.in_let_context => {
                self.in_let_context = false;
                return TokenType::Be;
            }
            "while" => return TokenType::While,
            "assert" => return TokenType::Assert,
            "trust" => return TokenType::Trust,
            // Imperative-only: these are common English words ("the proof requires
            // …", "this ensures …"), so keep them as plain words in declarative mode.
            "require" if self.mode == LexerMode::Imperative => return TokenType::Require,
            "requires" if self.mode == LexerMode::Imperative => return TokenType::Requires,
            "ensures" if self.mode == LexerMode::Imperative => return TokenType::Ensures,
            "check" => return TokenType::Check,
            // Theorem keywords (Declarative mode - for theorem blocks)
            "given" if self.mode == LexerMode::Declarative => return TokenType::Given,
            "prove" if self.mode == LexerMode::Declarative => return TokenType::Prove,
            "auto" if self.mode == LexerMode::Declarative => return TokenType::Auto,
            // P2P Networking keywords (Imperative mode only)
            "listen" if self.mode == LexerMode::Imperative => return TokenType::Listen,
            "connect" if self.mode == LexerMode::Imperative => return TokenType::NetConnect,
            "sleep" if self.mode == LexerMode::Imperative => return TokenType::Sleep,
            // GossipSub keywords (Imperative mode only)
            "sync" if self.mode == LexerMode::Imperative => return TokenType::Sync,
            // Persistence keywords
            "mount" if self.mode == LexerMode::Imperative => return TokenType::Mount,
            "persistent" => return TokenType::Persistent,  // Works in type expressions
            "combined" if self.mode == LexerMode::Imperative => return TokenType::Combined,
            "followed" if self.mode == LexerMode::Imperative => return TokenType::Followed,
            // Go-like Concurrency keywords (Imperative mode only)
            // Note: "first" and "after" are NOT keywords - they're checked via lookahead in parser
            // to avoid conflicting with their use as variable names
            "launch" if self.mode == LexerMode::Imperative => return TokenType::Launch,
            "task" if self.mode == LexerMode::Imperative => return TokenType::Task,
            "pipe" if self.mode == LexerMode::Imperative => return TokenType::Pipe,
            "receive" if self.mode == LexerMode::Imperative => return TokenType::Receive,
            "stop" if self.mode == LexerMode::Imperative => return TokenType::Stop,
            "try" if self.mode == LexerMode::Imperative => return TokenType::Try,
            "into" if self.mode == LexerMode::Imperative => return TokenType::Into,
            "native" if self.mode == LexerMode::Imperative => return TokenType::Native,
            "escape" if self.mode == LexerMode::Imperative => return TokenType::Escape,
            "from" => return TokenType::From,
            "otherwise" => return TokenType::Otherwise,
            // Phase 30c: Else/elif as aliases for Otherwise/Otherwise If
            "else" => return TokenType::Else,
            "elif" => return TokenType::Elif,
            // Sum type definition (Declarative mode only - for enum "either...or...")
            "either" if self.mode == LexerMode::Declarative => return TokenType::Either,
            // Pattern matching statement
            "inspect" if self.mode == LexerMode::Imperative => return TokenType::Inspect,
            // Constructor keyword (Imperative mode only)
            "new" if self.mode == LexerMode::Imperative => return TokenType::New,
            // Only emit Give/Show as keywords in Imperative mode
            // In Declarative mode, they fall through to lexicon lookup as verbs
            "give" if self.mode == LexerMode::Imperative => return TokenType::Give,
            "show" if self.mode == LexerMode::Imperative => return TokenType::Show,
            // Collection operation keywords (Imperative mode only)
            "push" if self.mode == LexerMode::Imperative => return TokenType::Push,
            "pop" if self.mode == LexerMode::Imperative => return TokenType::Pop,
            "copy" if self.mode == LexerMode::Imperative => return TokenType::Copy,
            "through" if self.mode == LexerMode::Imperative => return TokenType::Through,
            "length" if self.mode == LexerMode::Imperative => return TokenType::Length,
            "at" if self.mode == LexerMode::Imperative => return TokenType::At,
            // Set operation keywords (Imperative mode only)
            "add" if self.mode == LexerMode::Imperative => return TokenType::Add,
            "remove" if self.mode == LexerMode::Imperative => return TokenType::Remove,
            "contains" if self.mode == LexerMode::Imperative => return TokenType::Contains,
            "union" if self.mode == LexerMode::Imperative => return TokenType::Union,
            "intersection" if self.mode == LexerMode::Imperative => return TokenType::Intersection,
            // Zone keywords (Imperative mode only)
            "inside" if self.mode == LexerMode::Imperative => return TokenType::Inside,
            "zone" if self.mode == LexerMode::Imperative => return TokenType::Zone,
            "called" if self.mode == LexerMode::Imperative => return TokenType::Called,
            "size" if self.mode == LexerMode::Imperative => return TokenType::Size,
            "mapped" if self.mode == LexerMode::Imperative => return TokenType::Mapped,
            // Structured Concurrency keywords (Imperative mode only)
            "attempt" if self.mode == LexerMode::Imperative => return TokenType::Attempt,
            "following" if self.mode == LexerMode::Imperative => return TokenType::Following,
            "simultaneously" if self.mode == LexerMode::Imperative => return TokenType::Simultaneously,
            // IO keywords (Imperative mode only)
            "read" if self.mode == LexerMode::Imperative => return TokenType::Read,
            "write" if self.mode == LexerMode::Imperative => return TokenType::Write,
            "console" if self.mode == LexerMode::Imperative => return TokenType::Console,
            "file" if self.mode == LexerMode::Imperative => return TokenType::File,
            // Agent System keywords (Imperative mode only)
            "spawn" if self.mode == LexerMode::Imperative => return TokenType::Spawn,
            "send" if self.mode == LexerMode::Imperative => return TokenType::Send,
            "await" if self.mode == LexerMode::Imperative => return TokenType::Await,
            // Serialization keyword (works in Definition blocks too)
            "portable" => return TokenType::Portable,
            // Sipping Protocol keywords (Imperative mode only)
            "manifest" if self.mode == LexerMode::Imperative => return TokenType::Manifest,
            "chunk" if self.mode == LexerMode::Imperative => return TokenType::Chunk,
            // CRDT keywords
            "shared" => return TokenType::Shared,  // Works in Definition blocks like Portable
            "merge" if self.mode == LexerMode::Imperative => return TokenType::Merge,
            "increase" if self.mode == LexerMode::Imperative => return TokenType::Increase,
            // Extended CRDT keywords
            "decrease" if self.mode == LexerMode::Imperative => return TokenType::Decrease,
            "append" if self.mode == LexerMode::Imperative => return TokenType::Append,
            "resolve" if self.mode == LexerMode::Imperative => return TokenType::Resolve,
            "values" if self.mode == LexerMode::Imperative => return TokenType::Values,
            // Type keywords (work in both modes like "Shared"):
            "tally" => return TokenType::Tally,
            "sharedset" => return TokenType::SharedSet,
            "sharedsequence" => return TokenType::SharedSequence,
            "collaborativesequence" => return TokenType::CollaborativeSequence,
            "sharedmap" => return TokenType::SharedMap,
            "divergent" => return TokenType::Divergent,
            "removewins" => return TokenType::RemoveWins,
            "addwins" => return TokenType::AddWins,
            "yata" => return TokenType::YATA,
            // Calendar / clock time unit words (Span expressions). The singular
            // "second"/"minute" are ambiguous (ordinal "second base", adjective
            // "minute detail") so they are time units only after a count; the
            // plurals and "hour" are unambiguous time units.
            "seconds" => return TokenType::CalendarUnit(CalendarUnit::Second),
            "minutes" => return TokenType::CalendarUnit(CalendarUnit::Minute),
            "second" if self.prev_word_is_numeric() => {
                return TokenType::CalendarUnit(CalendarUnit::Second)
            }
            "minute" if self.prev_word_is_numeric() => {
                return TokenType::CalendarUnit(CalendarUnit::Minute)
            }
            "hour" | "hours" => return TokenType::CalendarUnit(CalendarUnit::Hour),
            "day" | "days" => return TokenType::CalendarUnit(CalendarUnit::Day),
            "week" | "weeks" => return TokenType::CalendarUnit(CalendarUnit::Week),
            "month" | "months" => return TokenType::CalendarUnit(CalendarUnit::Month),
            "year" | "years" => return TokenType::CalendarUnit(CalendarUnit::Year),
            // Span-related keywords (note: "before" is handled earlier to avoid preposition conflict)
            "ago" => return TokenType::Ago,
            "hence" => return TokenType::Hence,
            "if" => return TokenType::If,
            "only" => return TokenType::Focus(FocusKind::Only),
            "even" => return TokenType::Focus(FocusKind::Even),
            "just" if self.peek_word(1).map_or(false, |w| {
                !self.is_verb_like(w) || w.to_lowercase() == "john" || w.chars().next().map_or(false, |c| c.is_uppercase())
            }) => return TokenType::Focus(FocusKind::Just),
            "much" => return TokenType::Measure(MeasureKind::Much),
            "little" => return TokenType::Measure(MeasureKind::Little),
            _ => {}
        }

        if lexicon::is_scopal_adverb(&lower) {
            let sym = self.interner.intern(&Self::capitalize(&lower));
            return TokenType::ScopalAdverb(sym);
        }

        if lexicon::is_temporal_adverb(&lower) && !self.prev_token_is_determiner() {
            let sym = self.interner.intern(&Self::capitalize(&lower));
            return TokenType::TemporalAdverb(sym);
        }

        if lexicon::is_non_intersective(&lower) {
            let sym = self.interner.intern(&Self::capitalize(&lower));
            return TokenType::NonIntersectiveAdjective(sym);
        }

        if lexicon::is_adverb(&lower) {
            let sym = self.interner.intern(&Self::capitalize(&lower));
            return TokenType::Adverb(sym);
        }
        // A capitalized "-ly" word is normally a proper name ("Billy", "Holly",
        // "Kelly's stamp"), not an adverb — UNLESS it is a sentence-initial
        // manner adverb modifying a following pronoun ("Quickly he ran").
        let cap_ly_is_name = word.chars().next().map_or(false, |c| c.is_uppercase())
            && !self.peek_word(1).map_or(false, |w| {
                matches!(
                    w.to_lowercase().as_str(),
                    "he" | "she" | "it" | "they" | "we" | "i" | "you" | "who"
                )
            });
        if lower.ends_with("ly")
            && !lexicon::is_not_adverb(&lower)
            && !lexicon::is_common_noun(&lower)
            && lower.len() > 4
            && !cap_ly_is_name
        {
            let sym = self.interner.intern(&Self::capitalize(&lower));
            return TokenType::Adverb(sym);
        }

        if let Some(base) = self.try_parse_superlative(&lower) {
            let sym = self.interner.intern(&base);
            return TokenType::Superlative(sym);
        }

        // Handle irregular comparatives (less, more, better, worse). "fewer" is
        // the count counterpart of "less" — same decreasing direction — and maps
        // to the same Decreasing base so it grades like "less" ("received fewer
        // votes than Y" → Less(Vote(X), Vote(Y))) without turning bare "few" into
        // an adjective (which would break its quantifier reading, "few cookies").
        let irregular_comparative = match lower.as_str() {
            "less" => Some("Little"),
            "fewer" => Some("Little"),
            "more" => Some("Much"),
            "better" => Some("Good"),
            "worse" => Some("Bad"),
            _ => None,
        };
        if let Some(base) = irregular_comparative {
            let sym = self.interner.intern(base);
            return TokenType::Comparative(sym);
        }

        if let Some(base) = self.try_parse_comparative(&lower) {
            // A word that is ALSO a common noun ("stranger" — the noun vs the
            // comparative of "strange") is a comparative only in comparative
            // position, signalled by a following "than" ("is stranger than").
            // Elsewhere ("the stranger", "Let stranger be …") it is the noun, so
            // defer to noun classification below — mirroring the performative /
            // base-verb noun-deference above.
            let next_is_than = self
                .peek_word(1)
                .map_or(false, |w| w.eq_ignore_ascii_case("than"));
            if !lexicon::is_common_noun(&lower) || next_is_than {
                let sym = self.interner.intern(&base);
                return TokenType::Comparative(sym);
            }
            // else fall through to noun classification
        }

        if lexicon::is_performative(&lower) {
            // If the word is also a common noun AND follows a determiner or precedes a copula,
            // don't force performative reading.
            // "every request holds" → request is a noun, not a performative verb.
            // "If request is asserted" → request is a noun (subject before copula).
            // "I promise to come" → promise IS a performative verb.
            let after_determiner = self.prev_token_is_determiner();
            let before_copula = self.next_token_is_copula();
            if !lexicon::is_common_noun(&lower) || (!after_determiner && !before_copula) {
                let sym = self.interner.intern(&Self::capitalize(&lower));
                return TokenType::Performative(sym);
            }
            // Fall through to noun/verb disambiguation below
        }

        if lexicon::is_base_verb_early(&lower) {
            // If the word is also a common noun AND follows a determiner or precedes a copula,
            // don't force verb reading.
            // "every grant holds" → grant is a noun, not a verb.
            // "If grant is low" → grant is a noun (subject before copula).
            let after_determiner = self.prev_token_is_determiner();
            let before_copula = self.next_token_is_copula();
            if !lexicon::is_common_noun(&lower) || (!after_determiner && !before_copula) {
                let sym = self.interner.intern(&Self::capitalize(&lower));
                let class = lexicon::lookup_verb_class(&lower);
                return TokenType::Verb {
                    lemma: sym,
                    time: Time::Present,
                    aspect: Aspect::Simple,
                    class,
                };
            }
            // Fall through to noun/verb disambiguation below
        }

        // Check for gerunds/progressive verbs BEFORE ProperName check
        // "Running" at start of sentence should be Verb, not ProperName
        if lower.ends_with("ing") && lower.len() > 4 {
            // Participial ADJECTIVE in attributive position ("the
            // corresponding output line"): the word has an adjective entry
            // and a content word follows it. -ing PREPOSITIONS ("during")
            // skip the early return too — the dual-class block below emits
            // their Ambiguous alternatives.
            let attributive = self.is_adjective_like(&lower)
                && self
                    .peek_word(1)
                    .map(|next| {
                        let next_lower = next.to_lowercase();
                        self.is_noun_like(&next_lower) || self.is_adjective_like(&next_lower)
                    })
                    .unwrap_or(false);
            if !attributive && !lexicon::is_preposition(&lower) {
                if let Some(entry) = self.lexicon.lookup_verb(&lower) {
                    let sym = self.interner.intern(&entry.lemma);
                    return TokenType::Verb {
                        lemma: sym,
                        time: entry.time,
                        aspect: entry.aspect,
                        class: entry.class,
                    };
                }
            }
        }

        if first_char.is_uppercase() {
            // Smart Lexicon: Check if this capitalized word is actually a common noun
            // Only apply for sentence-initial words (followed by verb) to avoid
            // breaking type definitions like "A Point has:"
            //
            // Pattern: "Farmers walk." → Farmers is plural of Farmer (common noun)
            // Pattern: "A Point has:" → Point is a type name (proper name)
            if let Some(next) = self.peek_word(1) {
                let next_lower = next.to_lowercase();
                // If next word is a verb, this capitalized word is likely a subject noun
                let is_followed_by_verb = self.lexicon.lookup_verb(&next_lower).is_some()
                    || matches!(next_lower.as_str(), "is" | "are" | "was" | "were" | "has" | "have" | "had");

                if is_followed_by_verb {
                    // Check if lowercase version is a derivable common noun
                    if let Some(analysis) = lexicon::analyze_word(&lower) {
                        match analysis {
                            lexicon::WordAnalysis::Noun(meta) if meta.number == lexicon::Number::Plural => {
                                // It's a plural noun - definitely a common noun
                                let sym = self.interner.intern(&lower);
                                return TokenType::Noun(sym);
                            }
                            lexicon::WordAnalysis::DerivedNoun { number: lexicon::Number::Plural, .. } => {
                                // Derived plural agentive noun (e.g., "Bloggers")
                                let sym = self.interner.intern(&lower);
                                return TokenType::Noun(sym);
                            }
                            _ => {
                                // Singular nouns at sentence start could still be proper names
                                // e.g., "John walks." vs "Farmer walks."
                            }
                        }
                    }
                }
            }

            let sym = self.interner.intern(word);
            return TokenType::ProperName(sym);
        }

        let verb_entry = self.lexicon.lookup_verb(&lower);
        // Irregular plurals ("children", "mice") are not in the common-noun
        // table; morphological analysis identifies them. Restricted to
        // non-verb words so "rains" stays a pure verb.
        let is_noun = lexicon::is_common_noun(&lower)
            || (verb_entry.is_none()
                && matches!(
                    lexicon::analyze_word(&lower),
                    Some(lexicon::WordAnalysis::Noun(_))
                ));
        let is_adj = self.is_adjective_like(&lower);
        let is_disambiguated = lexicon::is_disambiguation_not_verb(&lower);

        // Ambiguous: word is Verb AND (Noun OR Adjective OR Preposition),
        // not disambiguated
        let is_prep = lexicon::is_preposition(&lower);
        if verb_entry.is_some() && (is_noun || is_adj || is_prep) && !is_disambiguated {
            let entry = verb_entry.unwrap();
            let verb_token = TokenType::Verb {
                lemma: self.interner.intern(&entry.lemma),
                time: entry.time,
                aspect: entry.aspect,
                class: entry.class,
            };

            let mut alternatives = Vec::new();
            if is_noun {
                alternatives.push(TokenType::Noun(self.interner.intern(word)));
            }
            if is_adj {
                alternatives.push(TokenType::Adjective(self.interner.intern(word)));
            }
            if is_prep {
                alternatives.push(TokenType::Preposition(self.interner.intern(&lower)));
            }

            return TokenType::Ambiguous {
                primary: Box::new(verb_token),
                alternatives,
            };
        }

        // Disambiguated away from verb — only when there is an alternate reading.
        // If the word has no noun or adjective reading, the lexicon verb entry wins.
        if let Some(entry) = &verb_entry {
            if is_disambiguated {
                let sym = self.interner.intern(word);
                if is_noun {
                    return TokenType::Noun(sym);
                }
                if is_adj {
                    return TokenType::Adjective(sym);
                }
                // No alternate reading: honour the lexicon (e.g. "led" = past of "lead").
                return TokenType::Verb {
                    lemma: self.interner.intern(&entry.lemma),
                    time: entry.time,
                    aspect: entry.aspect,
                    class: entry.class,
                };
            }
        }

        // Pure verb
        if let Some(entry) = verb_entry {
            let sym = self.interner.intern(&entry.lemma);
            return TokenType::Verb {
                lemma: sym,
                time: entry.time,
                aspect: entry.aspect,
                class: entry.class,
            };
        }

        // Pure noun
        if is_noun {
            let sym = self.interner.intern(word);
            return TokenType::Noun(sym);
        }

        // Pure adjective — a word the lexicon knows only as an adjective
        // ("happy", "dangerous", "tall"). Without this the word falls through
        // to the Noun fallback and predicative/attributive adjective readings
        // are lost (e.g. "John seems happy." strands "happy").
        if is_adj {
            let sym = self.interner.intern(word);
            return TokenType::Adjective(sym);
        }

        if lexicon::is_base_verb(&lower) {
            let sym = self.interner.intern(&Self::capitalize(&lower));
            let class = lexicon::lookup_verb_class(&lower);
            return TokenType::Verb {
                lemma: sym,
                time: Time::Present,
                aspect: Aspect::Simple,
                class,
            };
        }

        if lower.ends_with("ian")
            || lower.ends_with("er")
            || lower == "logic"
            || lower == "time"
            || lower == "men"
            || lower == "book"
            || lower == "house"
            || lower == "code"
            || lower == "user"
        {
            let sym = self.interner.intern(word);
            return TokenType::Noun(sym);
        }

        if lexicon::is_particle(&lower) {
            let sym = self.interner.intern(&lower);
            return TokenType::Particle(sym);
        }

        // Unknown "-ed" word → regular past-tense verb ("aired", "studied",
        // "debuted"). English marks the past tense morphologically, so an
        // otherwise-unrecognized "-ed" content word is overwhelmingly a verb.
        // Kept Ambiguous with a Noun reading so it can still head an NP where a
        // noun is expected; the parser picks the verb reading in VP position.
        if lower.len() >= 4 && lower.ends_with("ed") {
            let stem = if lower.ends_with("ied") {
                format!("{}y", &lower[..lower.len() - 3])
            } else {
                lower[..lower.len() - 2].to_string()
            };
            let verb_token = TokenType::Verb {
                lemma: self.interner.intern(&Self::capitalize(&stem)),
                time: Time::Past,
                aspect: Aspect::Simple,
                class: lexicon::lookup_verb_class(&stem),
            };
            return TokenType::Ambiguous {
                primary: Box::new(verb_token),
                alternatives: vec![TokenType::Noun(self.interner.intern(word))],
            };
        }

        // Unknown lowercase words default to Noun. In natural language contexts
        // (puzzle clues, prose) unknown content words are overwhelmingly nouns —
        // domain-specific items like dance styles, place names, object names, etc.
        // Genuine adjectives and adverbs are caught by earlier checks.
        let sym = self.interner.intern(word);
        TokenType::Noun(sym)
    }

    fn capitalize(s: &str) -> String {
        let mut chars = s.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        }
    }

    pub fn is_collective_verb(lemma: &str) -> bool {
        lexicon::is_collective_verb(&lemma.to_lowercase())
    }

    pub fn is_mixed_verb(lemma: &str) -> bool {
        lexicon::is_mixed_verb(&lemma.to_lowercase())
    }

    pub fn is_distributive_verb(lemma: &str) -> bool {
        lexicon::is_distributive_verb(&lemma.to_lowercase())
    }

    pub fn is_intensional_predicate(lemma: &str) -> bool {
        lexicon::is_intensional_predicate(&lemma.to_lowercase())
    }

    pub fn is_opaque_verb(lemma: &str) -> bool {
        lexicon::is_opaque_verb(&lemma.to_lowercase())
    }

    pub fn is_ditransitive_verb(lemma: &str) -> bool {
        lexicon::is_ditransitive_verb(&lemma.to_lowercase())
    }

    fn is_verb_like(&self, word: &str) -> bool {
        let lower = word.to_lowercase();
        if lexicon::is_infinitive_verb(&lower) {
            return true;
        }
        if let Some(entry) = self.lexicon.lookup_verb(&lower) {
            return entry.lemma.len() > 0;
        }
        false
    }

    pub fn is_subject_control_verb(lemma: &str) -> bool {
        lexicon::is_subject_control_verb(&lemma.to_lowercase())
    }

    pub fn is_raising_verb(lemma: &str) -> bool {
        lexicon::is_raising_verb(&lemma.to_lowercase())
    }

    pub fn is_object_control_verb(lemma: &str) -> bool {
        lexicon::is_object_control_verb(&lemma.to_lowercase())
    }

    pub fn is_weather_verb(lemma: &str) -> bool {
        matches!(
            lemma.to_lowercase().as_str(),
            "rain" | "snow" | "hail" | "thunder" | "pour"
        )
    }

    fn try_parse_superlative(&self, word: &str) -> Option<String> {
        if !word.ends_with("est") || word.len() < 5 {
            return None;
        }

        let base = &word[..word.len() - 3];

        if base.len() >= 2 {
            let chars: Vec<char> = base.chars().collect();
            let last = chars[chars.len() - 1];
            let second_last = chars[chars.len() - 2];
            if last == second_last && !"aeiou".contains(last) {
                let stem = &base[..base.len() - 1];
                if lexicon::is_gradable_adjective(stem) {
                    return Some(Self::capitalize(stem));
                }
            }
        }

        if base.ends_with("i") {
            let stem = format!("{}y", &base[..base.len() - 1]);
            if lexicon::is_gradable_adjective(&stem) {
                return Some(Self::capitalize(&stem));
            }
        }

        if lexicon::is_gradable_adjective(base) {
            return Some(Self::capitalize(base));
        }

        None
    }

    fn try_parse_comparative(&self, word: &str) -> Option<String> {
        if !word.ends_with("er") || word.len() < 4 {
            return None;
        }

        let base = &word[..word.len() - 2];

        if base.len() >= 2 {
            let chars: Vec<char> = base.chars().collect();
            let last = chars[chars.len() - 1];
            let second_last = chars[chars.len() - 2];
            if last == second_last && !"aeiou".contains(last) {
                let stem = &base[..base.len() - 1];
                if lexicon::is_gradable_adjective(stem) {
                    return Some(Self::capitalize(stem));
                }
            }
        }

        if base.ends_with("i") {
            let stem = format!("{}y", &base[..base.len() - 1]);
            if lexicon::is_gradable_adjective(&stem) {
                return Some(Self::capitalize(&stem));
            }
        }

        if lexicon::is_gradable_adjective(base) {
            return Some(Self::capitalize(base));
        }

        // Silent-e adjectives drop the 'e' before -er: "wide"→"wider",
        // "large"→"larger", "late"→"later". Recover the base by restoring it.
        let with_e = format!("{}e", base);
        if lexicon::is_gradable_adjective(&with_e) {
            return Some(Self::capitalize(&with_e));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexer_handles_apostrophe() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("it's raining", &mut interner);
        let tokens = lexer.tokenize();
        assert!(!tokens.is_empty());
    }

    #[test]
    fn lexer_handles_question_mark() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("Is it raining?", &mut interner);
        let tokens = lexer.tokenize();
        assert!(!tokens.is_empty());
    }

    #[test]
    fn ring_is_not_verb() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("ring", &mut interner);
        let tokens = lexer.tokenize();
        assert!(matches!(tokens[0].kind, TokenType::Noun(_)));
    }

    /// Rung 0a, Stride 0: `## Define` introduces a vernacular-logic predicate
    /// definition. It must lex to its OWN block type — never collapse into the
    /// pre-existing `## Definition` (which `DiscoveryPass` consumes for type
    /// defs). This pins both: `## Define` → `Define`, `## Definition` unchanged.
    #[test]
    fn define_block_header_is_distinct_from_definition() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("## Define\n", &mut interner);
        let tokens = lexer.tokenize();
        let header = tokens
            .iter()
            .find(|t| matches!(t.kind, TokenType::BlockHeader { .. }))
            .expect("## Define should produce a block header token");
        assert!(
            matches!(
                header.kind,
                TokenType::BlockHeader { block_type: BlockType::Define }
            ),
            "## Define must tokenize to BlockType::Define, got {:?}",
            header.kind
        );

        // Regression guard: the long-standing `## Definition` block stays itself.
        let mut interner2 = Interner::new();
        let mut lexer2 = Lexer::new("## Definition\n", &mut interner2);
        let tokens2 = lexer2.tokenize();
        let header2 = tokens2
            .iter()
            .find(|t| matches!(t.kind, TokenType::BlockHeader { .. }))
            .expect("## Definition should produce a block header token");
        assert!(
            matches!(
                header2.kind,
                TokenType::BlockHeader { block_type: BlockType::Definition }
            ),
            "## Definition must still tokenize to BlockType::Definition, got {:?}",
            header2.kind
        );
    }

    #[test]
    fn debug_that_token() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("The cat that runs", &mut interner);
        let tokens = lexer.tokenize();
        for (i, t) in tokens.iter().enumerate() {
            let lex = interner.resolve(t.lexeme);
            eprintln!("Token[{}]: {:?} -> {:?}", i, lex, t.kind);
        }
        let that_token = tokens.iter().find(|t| interner.resolve(t.lexeme) == "that");
        if let Some(t) = that_token {
            // Verify discriminant comparison works
            let check = std::mem::discriminant(&t.kind) == std::mem::discriminant(&TokenType::That);
            eprintln!("Discriminant check for That: {}", check);
            assert!(matches!(t.kind, TokenType::That), "'that' should be TokenType::That, got {:?}", t.kind);
        } else {
            panic!("No 'that' token found");
        }
    }

    #[test]
    fn bus_is_not_verb() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("bus", &mut interner);
        let tokens = lexer.tokenize();
        assert!(matches!(tokens[0].kind, TokenType::Noun(_)));
    }

    #[test]
    fn lowercase_a_is_article() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("a car", &mut interner);
        let tokens = lexer.tokenize();
        for (i, t) in tokens.iter().enumerate() {
            let lex = interner.resolve(t.lexeme);
            eprintln!("Token[{}]: {:?} -> {:?}", i, lex, t.kind);
        }
        assert_eq!(tokens[0].kind, TokenType::Article(Definiteness::Indefinite));
        assert!(matches!(tokens[1].kind, TokenType::Noun(_)), "Expected Noun, got {:?}", tokens[1].kind);
    }

    #[test]
    fn open_is_ambiguous() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("open", &mut interner);
        let tokens = lexer.tokenize();

        if let TokenType::Ambiguous { primary, alternatives } = &tokens[0].kind {
            assert!(matches!(**primary, TokenType::Verb { .. }), "Primary should be Verb");
            assert!(alternatives.iter().any(|t| matches!(t, TokenType::Adjective(_))),
                "Should have Adjective alternative");
        } else {
            panic!("Expected Ambiguous token for 'open', got {:?}", tokens[0].kind);
        }
    }

    #[test]
    fn basic_tokenization() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("All men are mortal.", &mut interner);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0].kind, TokenType::All);
        assert!(matches!(tokens[1].kind, TokenType::Noun(_)));
        assert_eq!(tokens[2].kind, TokenType::Are);
    }

    #[test]
    fn iff_tokenizes_as_single_token() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("A if and only if B", &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            tokens.iter().any(|t| t.kind == TokenType::Iff),
            "should contain Iff token: got {:?}",
            tokens
        );
    }

    #[test]
    fn is_equal_to_tokenizes_as_identity() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("Socrates is equal to Socrates", &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            tokens.iter().any(|t| t.kind == TokenType::Identity),
            "should contain Identity token: got {:?}",
            tokens
        );
    }

    #[test]
    fn is_identical_to_tokenizes_as_identity() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("Clark is identical to Superman", &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            tokens.iter().any(|t| t.kind == TokenType::Identity),
            "should contain Identity token: got {:?}",
            tokens
        );
    }

    #[test]
    fn itself_tokenizes_as_reflexive() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("John loves itself", &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            tokens.iter().any(|t| t.kind == TokenType::Reflexive),
            "should contain Reflexive token: got {:?}",
            tokens
        );
    }

    #[test]
    fn himself_tokenizes_as_reflexive() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("John sees himself", &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            tokens.iter().any(|t| t.kind == TokenType::Reflexive),
            "should contain Reflexive token: got {:?}",
            tokens
        );
    }

    #[test]
    fn to_stay_tokenizes_correctly() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("to stay", &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            tokens.iter().any(|t| t.kind == TokenType::To),
            "should contain To token: got {:?}",
            tokens
        );
        assert!(
            tokens.iter().any(|t| matches!(t.kind, TokenType::Verb { .. })),
            "should contain Verb token for stay: got {:?}",
            tokens
        );
    }

    #[test]
    fn possessive_apostrophe_s() {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("John's dog", &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            tokens.iter().any(|t| t.kind == TokenType::Possessive),
            "should contain Possessive token: got {:?}",
            tokens
        );
        assert!(
            tokens.iter().any(|t| matches!(&t.kind, TokenType::ProperName(_))),
            "should have John as proper name: got {:?}",
            tokens
        );
    }

    #[test]
    fn lexer_produces_valid_spans() {
        let input = "All men are mortal.";
        let mut interner = Interner::new();
        let mut lexer = Lexer::new(input, &mut interner);
        let tokens = lexer.tokenize();

        // "All" at 0..3
        assert_eq!(tokens[0].span.start, 0);
        assert_eq!(tokens[0].span.end, 3);
        assert_eq!(&input[tokens[0].span.start..tokens[0].span.end], "All");

        // "men" at 4..7
        assert_eq!(tokens[1].span.start, 4);
        assert_eq!(tokens[1].span.end, 7);
        assert_eq!(&input[tokens[1].span.start..tokens[1].span.end], "men");

        // "are" at 8..11
        assert_eq!(tokens[2].span.start, 8);
        assert_eq!(tokens[2].span.end, 11);
        assert_eq!(&input[tokens[2].span.start..tokens[2].span.end], "are");

        // "mortal" at 12..18
        assert_eq!(tokens[3].span.start, 12);
        assert_eq!(tokens[3].span.end, 18);
        assert_eq!(&input[tokens[3].span.start..tokens[3].span.end], "mortal");

        // "." at 18..19
        assert_eq!(tokens[4].span.start, 18);
        assert_eq!(tokens[4].span.end, 19);

        // EOF at end
        assert_eq!(tokens[5].span.start, input.len());
        assert_eq!(tokens[5].kind, TokenType::EOF);
    }

    #[test]
    fn triple_quote_produces_string_token() {
        let mut interner = Interner::new();
        let source = "## Main\nLet msg be \"\"\"\n    Hello\n    World\n\"\"\".\nShow msg.";
        let mut lexer = Lexer::new(source, &mut interner);
        let tokens = lexer.tokenize();
        // Dump all tokens for debugging
        for (i, t) in tokens.iter().enumerate() {
            let lex = interner.resolve(t.lexeme);
            eprintln!("Token[{}]: {:?} lex={:?} span={}..{}", i, t.kind, lex, t.span.start, t.span.end);
        }
        // Find the string token
        let str_token = tokens.iter().find(|t| matches!(t.kind, TokenType::StringLiteral(_) | TokenType::InterpolatedString(_)));
        assert!(str_token.is_some(), "Should have a string token. Tokens: {:?}", tokens.iter().map(|t| format!("{:?}", t.kind)).collect::<Vec<_>>());
        if let Some(tok) = str_token {
            let content = interner.resolve(tok.lexeme);
            eprintln!("Triple-quote content: {:?}", content);
            assert!(content.contains("Hello"), "Should contain Hello, got: {:?}", content);
        }
    }

    /// BUG-015: a string-literal span must stay BYTE-indexed even when multibyte
    /// text precedes it; a byte-start / char-end span underflows in
    /// `insert_indentation_tokens` and panics on valid Unicode input.
    #[test]
    fn string_literal_span_stays_byte_indexed_after_leading_multibyte_text() {
        // 7 leading 2-byte Greek letters, a space, then a quoted string.
        let src = "\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1} \"x\"";
        let mut interner = Interner::new();
        let mut lexer = Lexer::new(src, &mut interner);
        let tokens = lexer.tokenize(); // previously panicked: 'attempt to subtract with overflow'

        let s = tokens
            .iter()
            .find(|t| matches!(t.kind, TokenType::StringLiteral(_) | TokenType::InterpolatedString(_)))
            .expect("should produce a string literal token");

        assert!(s.span.end >= s.span.start, "span end {} must be >= start {}", s.span.end, s.span.start);
        assert!(
            src.is_char_boundary(s.span.start) && src.is_char_boundary(s.span.end),
            "span [{}, {}) must lie on char boundaries",
            s.span.start, s.span.end
        );
        assert_eq!(&src[s.span.start..s.span.end], "\"x\"");
    }

    /// BUG-030: calendar-impossible dates (Feb 30, Apr 31, Feb 29 in a non-leap
    /// year) must NOT tokenize to a DateLiteral — they would otherwise silently
    /// map onto a real, different day.
    #[test]
    fn date_literal_rejects_impossible_day_of_month() {
        let impossible = [
            "2026-02-30", "2026-02-31", "2026-02-29", // 2026 is not a leap year
            "2026-04-31", "2026-06-31", "2026-09-31", "2026-11-31",
        ];
        for input in impossible {
            let mut interner = Interner::new();
            let mut lexer = Lexer::new(input, &mut interner);
            let tokens = lexer.tokenize();
            assert!(
                !tokens.iter().any(|t| matches!(t.kind, TokenType::DateLiteral { .. })),
                "{input} is not a real calendar date and must not become a DateLiteral; got {tokens:?}"
            );
        }
        // Control: a real leap-day must still tokenize.
        let mut interner = Interner::new();
        let mut lexer = Lexer::new("2024-02-29", &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            tokens.iter().any(|t| matches!(t.kind, TokenType::DateLiteral { .. })),
            "2024-02-29 is a valid leap-day and must still tokenize; got {tokens:?}"
        );
    }
}
