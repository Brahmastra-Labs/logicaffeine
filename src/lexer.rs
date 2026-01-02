use crate::intern::Interner;
use crate::lexicon::{self, Aspect, Definiteness, Lexicon, Time};
use crate::token::{BlockType, FocusKind, MeasureKind, Span, Token, TokenType};

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
        }
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
}

struct WordItem {
    word: String,
    trailing_punct: Option<char>,
    start: usize,
    end: usize,
    punct_pos: Option<usize>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &str, interner: &'a mut Interner) -> Self {
        let words = Self::split_into_words(input);
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
        }
    }

    fn split_into_words(input: &str) -> Vec<WordItem> {
        let mut items = Vec::new();
        let mut current_word = String::new();
        let mut word_start = 0;
        let chars: Vec<char> = input.chars().collect();
        let mut char_idx = 0;
        let mut skip_count = 0;

        for (i, c) in input.char_indices() {
            if skip_count > 0 {
                skip_count -= 1;
                char_idx += 1;
                continue;
            }
            let next_pos = i + c.len_utf8();
            match c {
                ' ' | '\t' | '\n' | '\r' => {
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

                    if prev_is_digit && next_is_digit {
                        // This is a decimal point, include it in the current word
                        current_word.push(c);
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
                // Phase 33: String literals "hello world"
                '"' => {
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

                    // Scan until closing quote
                    let string_start = i;
                    let mut j = char_idx + 1;
                    let mut string_content = String::new();
                    while j < chars.len() && chars[j] != '"' {
                        if chars[j] == '\\' && j + 1 < chars.len() {
                            // Escape sequence - skip backslash, include next char
                            j += 1;
                            if j < chars.len() {
                                string_content.push(chars[j]);
                            }
                        } else {
                            string_content.push(chars[j]);
                        }
                        j += 1;
                    }

                    // Create a special marker for string literals
                    // We prefix with a special character to identify in tokenize()
                    items.push(WordItem {
                        word: format!("\x00STR:{}", string_content),
                        trailing_punct: None,
                        start: string_start,
                        end: if j < chars.len() { j + 1 } else { j },
                        punct_pos: None,
                    });

                    // Skip past the closing quote
                    if j < chars.len() {
                        skip_count = j - char_idx;
                    } else {
                        skip_count = j - char_idx - 1;
                    }
                    word_start = if j < chars.len() { j + 1 } else { j };
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
                // Phase 38: Handle -> as a single token for return type syntax
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
                '(' | ')' | '[' | ']' | ',' | '?' | '!' | ':' | '+' | '-' | '*' | '/' | '%' | '<' | '>' | '=' => {
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
                        // This is a contraction ending in 't (don't, doesn't, won't, can't, etc.)
                        let word_lower = current_word.to_lowercase();
                        if word_lower == "don" || word_lower == "doesn" || word_lower == "didn" {
                            // do/does/did + not
                            let base = if word_lower == "don" { "do" }
                                      else if word_lower == "doesn" { "does" }
                                      else { "did" };
                            items.push(WordItem {
                                word: base.to_string(),
                                trailing_punct: None,
                                start: word_start,
                                end: i,
                                punct_pos: None,
                            });
                            items.push(WordItem {
                                word: "not".to_string(),
                                trailing_punct: None,
                                start: i,
                                end: i + 2,
                                punct_pos: None,
                            });
                            current_word.clear();
                            word_start = next_pos + 1;
                            skip_count = 1;
                        } else if word_lower == "won" {
                            // will + not
                            items.push(WordItem {
                                word: "will".to_string(),
                                trailing_punct: None,
                                start: word_start,
                                end: i,
                                punct_pos: None,
                            });
                            items.push(WordItem {
                                word: "not".to_string(),
                                trailing_punct: None,
                                start: i,
                                end: i + 2,
                                punct_pos: None,
                            });
                            current_word.clear();
                            word_start = next_pos + 1;
                            skip_count = 1;
                        } else if word_lower == "can" {
                            // cannot
                            items.push(WordItem {
                                word: "cannot".to_string(),
                                trailing_punct: None,
                                start: word_start,
                                end: i + 2,
                                punct_pos: None,
                            });
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
                c if c.is_alphabetic() || c.is_ascii_digit() || (c == '.' && !current_word.is_empty() && current_word.chars().all(|ch| ch.is_ascii_digit())) || c == '_' => {
                    if current_word.is_empty() {
                        word_start = i;
                    }
                    current_word.push(c);
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
                        _ => {
                            self.pos += 1;
                            continue;
                        }
                    };
                    let lexeme = self.interner.intern(&punct.to_string());
                    let span = Span::new(word_start, word_end);
                    tokens.push(Token::new(kind, lexeme, span));
                }
                self.pos += 1;
                continue;
            }

            // Phase 33: Check for string literal marker
            if word.starts_with("\x00STR:") {
                let content = &word[5..]; // Skip the marker prefix
                let sym = self.interner.intern(content);
                let span = Span::new(word_start, word_end);
                tokens.push(Token::new(TokenType::StringLiteral(sym), sym, span));
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

            let kind = self.classify_with_lookahead(&word);
            let lexeme = self.interner.intern(&word);
            let span = Span::new(word_start, word_end);
            tokens.push(Token::new(kind, lexeme, span));

            if let Some(punct) = trailing_punct {
                if punct == '\'' {
                    if let Some(next_item) = self.words.get(self.pos + 1) {
                        if next_item.word.to_lowercase() == "s" {
                            let poss_lexeme = self.interner.intern("'s");
                            let poss_start = punct_pos.unwrap_or(word_end);
                            let poss_end = next_item.end;
                            tokens.push(Token::new(TokenType::Possessive, poss_lexeme, Span::new(poss_start, poss_end)));
                            self.pos += 1;
                            if let Some(s_punct) = next_item.trailing_punct {
                                let kind = match s_punct {
                                    '(' => TokenType::LParen,
                                    ')' => TokenType::RParen,
                                    '[' => TokenType::LBracket,
                                    ']' => TokenType::RBracket,
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

                let kind = match punct {
                    '(' => TokenType::LParen,
                    ')' => TokenType::RParen,
                    '[' => TokenType::LBracket,
                    ']' => TokenType::RBracket,
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

    fn classify_with_lookahead(&mut self, word: &str) -> TokenType {
        // Handle block headers (##Theorem, ##Main, etc.)
        if word.starts_with("##") {
            let block_name = &word[2..];
            let block_type = match block_name.to_lowercase().as_str() {
                "theorem" => BlockType::Theorem,
                "main" => BlockType::Main,
                "definition" => BlockType::Definition,
                "proof" => BlockType::Proof,
                "example" => BlockType::Example,
                "logic" => BlockType::Logic,
                "note" => BlockType::Note,
                "to" => BlockType::Function,  // Phase 32: ## To blocks
                "a" | "an" => BlockType::TypeDef,  // Inline type definitions: ## A Point has:
                "policy" => BlockType::Policy,  // Phase 50: Security policy definitions
                _ => BlockType::Note, // Default unknown block types to Note
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

        if let Some(n) = Self::word_to_number(&lower) {
            return TokenType::Cardinal(n);
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
                    // Phase 31: Added "has" for struct definitions
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

        // Disambiguate "that" as determiner vs complementizer
        // "that dog" → Article(Distal), "I know that he ran" → That (complementizer)
        if lower == "that" {
            if let Some(next) = self.peek_word(1) {
                let next_lower = next.to_lowercase();
                if self.is_noun_like(&next_lower) || self.is_adjective_like(&next_lower) {
                    return TokenType::Article(Definiteness::Distal);
                }
            }
        }

        // Phase 38: Arrow token for return type syntax
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

        if let Some(kind) = lexicon::lookup_keyword(&lower) {
            return kind;
        }

        if let Some(kind) = lexicon::lookup_pronoun(&lower) {
            return kind;
        }

        if let Some(def) = lexicon::lookup_article(&lower) {
            return TokenType::Article(def);
        }

        if let Some(time) = lexicon::lookup_auxiliary(&lower) {
            return TokenType::Auxiliary(time);
        }

        // Handle imperative keywords that might conflict with prepositions
        match lower.as_str() {
            "call" => return TokenType::Call,
            "in" if self.mode == LexerMode::Imperative => return TokenType::In,
            // Phase 8.5: Zone keywords (must come before is_preposition check)
            "inside" if self.mode == LexerMode::Imperative => return TokenType::Inside,
            // Phase 48: "at" for chunk access (must come before is_preposition check)
            "at" if self.mode == LexerMode::Imperative => return TokenType::At,
            // Phase 54: "into" for pipe send (must come before is_preposition check)
            "into" if self.mode == LexerMode::Imperative => return TokenType::Into,
            _ => {}
        }

        if lexicon::is_preposition(&lower) {
            let sym = self.interner.intern(&lower);
            return TokenType::Preposition(sym);
        }

        match lower.as_str() {
            "equals" => return TokenType::Equals,
            "item" => return TokenType::Item,
            "items" => return TokenType::Items,
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
                    // Phase 31: In Declarative mode, check positions 2-5 for "to"
                    // (handles field access like "set p's x to")
                    for offset in 2..=5 {
                        if self.peek_word(offset).map_or(false, |w| w.to_lowercase() == "to") {
                            return TokenType::Set;
                        }
                    }
                }
            }
            "return" => return TokenType::Return,
            "be" if self.in_let_context => {
                self.in_let_context = false;
                return TokenType::Be;
            }
            "while" => return TokenType::While,
            "assert" => return TokenType::Assert,
            "trust" => return TokenType::Trust,  // Phase 35: Trust statement
            "check" => return TokenType::Check,  // Phase 50: Security check
            // Phase 51: P2P Networking keywords (Imperative mode only)
            "listen" if self.mode == LexerMode::Imperative => return TokenType::Listen,
            "connect" if self.mode == LexerMode::Imperative => return TokenType::NetConnect,
            "sleep" if self.mode == LexerMode::Imperative => return TokenType::Sleep,
            // Phase 52: GossipSub keywords (Imperative mode only)
            "sync" if self.mode == LexerMode::Imperative => return TokenType::Sync,
            // Phase 53: Persistence keywords
            "mount" if self.mode == LexerMode::Imperative => return TokenType::Mount,
            "persistent" => return TokenType::Persistent,  // Works in type expressions
            "combined" if self.mode == LexerMode::Imperative => return TokenType::Combined,
            // Phase 54: Go-like Concurrency keywords (Imperative mode only)
            // Note: "first" and "after" are NOT keywords - they're checked via lookahead in parser
            // to avoid conflicting with their use as variable names
            "launch" if self.mode == LexerMode::Imperative => return TokenType::Launch,
            "task" if self.mode == LexerMode::Imperative => return TokenType::Task,
            "pipe" if self.mode == LexerMode::Imperative => return TokenType::Pipe,
            "receive" if self.mode == LexerMode::Imperative => return TokenType::Receive,
            "stop" if self.mode == LexerMode::Imperative => return TokenType::Stop,
            "try" if self.mode == LexerMode::Imperative => return TokenType::Try,
            "into" if self.mode == LexerMode::Imperative => return TokenType::Into,
            "native" => return TokenType::Native,  // Phase 38: Native function modifier
            "from" => return TokenType::From,  // Phase 36: Module qualification
            "otherwise" => return TokenType::Otherwise,
            // Phase 33: Sum type definition (Declarative mode only - for enum "either...or...")
            "either" if self.mode == LexerMode::Declarative => return TokenType::Either,
            // Phase 33: Pattern matching statement
            "inspect" if self.mode == LexerMode::Imperative => return TokenType::Inspect,
            // Phase 31: Constructor keyword (Imperative mode only)
            "new" if self.mode == LexerMode::Imperative => return TokenType::New,
            // Only emit Give/Show as keywords in Imperative mode
            // In Declarative mode, they fall through to lexicon lookup as verbs
            "give" if self.mode == LexerMode::Imperative => return TokenType::Give,
            "show" if self.mode == LexerMode::Imperative => return TokenType::Show,
            // Phase 43D: Collection operation keywords (Imperative mode only)
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
            // Phase 8.5: Zone keywords (Imperative mode only)
            "inside" if self.mode == LexerMode::Imperative => return TokenType::Inside,
            "zone" if self.mode == LexerMode::Imperative => return TokenType::Zone,
            "called" if self.mode == LexerMode::Imperative => return TokenType::Called,
            "size" if self.mode == LexerMode::Imperative => return TokenType::Size,
            "mapped" if self.mode == LexerMode::Imperative => return TokenType::Mapped,
            // Phase 9: Structured Concurrency keywords (Imperative mode only)
            "attempt" if self.mode == LexerMode::Imperative => return TokenType::Attempt,
            "following" if self.mode == LexerMode::Imperative => return TokenType::Following,
            "simultaneously" if self.mode == LexerMode::Imperative => return TokenType::Simultaneously,
            // Phase 10: IO keywords (Imperative mode only)
            "read" if self.mode == LexerMode::Imperative => return TokenType::Read,
            "write" if self.mode == LexerMode::Imperative => return TokenType::Write,
            "console" if self.mode == LexerMode::Imperative => return TokenType::Console,
            "file" if self.mode == LexerMode::Imperative => return TokenType::File,
            // Phase 46: Agent System keywords (Imperative mode only)
            "spawn" if self.mode == LexerMode::Imperative => return TokenType::Spawn,
            "send" if self.mode == LexerMode::Imperative => return TokenType::Send,
            "await" if self.mode == LexerMode::Imperative => return TokenType::Await,
            // Phase 47: Serialization keyword (works in Definition blocks too)
            "portable" => return TokenType::Portable,
            // Phase 48: Sipping Protocol keywords (Imperative mode only)
            "manifest" if self.mode == LexerMode::Imperative => return TokenType::Manifest,
            "chunk" if self.mode == LexerMode::Imperative => return TokenType::Chunk,
            // Phase 49: CRDT keywords
            "shared" => return TokenType::Shared,  // Works in Definition blocks like Portable
            "merge" if self.mode == LexerMode::Imperative => return TokenType::Merge,
            "increase" if self.mode == LexerMode::Imperative => return TokenType::Increase,
            // Phase 49b: Extended CRDT keywords (Wave 5)
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

        if lexicon::is_temporal_adverb(&lower) {
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
        if lower.ends_with("ly") && !lexicon::is_not_adverb(&lower) && lower.len() > 4 {
            let sym = self.interner.intern(&Self::capitalize(&lower));
            return TokenType::Adverb(sym);
        }

        if let Some(base) = self.try_parse_superlative(&lower) {
            let sym = self.interner.intern(&base);
            return TokenType::Superlative(sym);
        }

        // Phase 35: Handle irregular comparatives (less, more, better, worse)
        let irregular_comparative = match lower.as_str() {
            "less" => Some("Little"),
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
            let sym = self.interner.intern(&base);
            return TokenType::Comparative(sym);
        }

        if lexicon::is_performative(&lower) {
            let sym = self.interner.intern(&Self::capitalize(&lower));
            return TokenType::Performative(sym);
        }

        if lexicon::is_base_verb_early(&lower) {
            let sym = self.interner.intern(&Self::capitalize(&lower));
            let class = lexicon::lookup_verb_class(&lower);
            return TokenType::Verb {
                lemma: sym,
                time: Time::Present,
                aspect: Aspect::Simple,
                class,
            };
        }

        // Check for gerunds/progressive verbs BEFORE ProperName check
        // "Running" at start of sentence should be Verb, not ProperName
        if lower.ends_with("ing") && lower.len() > 4 {
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

        if first_char.is_uppercase() {
            let sym = self.interner.intern(word);
            return TokenType::ProperName(sym);
        }

        let verb_entry = self.lexicon.lookup_verb(&lower);
        let is_noun = lexicon::is_common_noun(&lower);
        let is_adj = self.is_adjective_like(&lower);
        let is_disambiguated = lexicon::is_disambiguation_not_verb(&lower);

        // Ambiguous: word is Verb AND (Noun OR Adjective), not disambiguated
        if verb_entry.is_some() && (is_noun || is_adj) && !is_disambiguated {
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

            return TokenType::Ambiguous {
                primary: Box::new(verb_token),
                alternatives,
            };
        }

        // Disambiguated to noun/adjective (not verb)
        if let Some(_) = &verb_entry {
            if is_disambiguated {
                let sym = self.interner.intern(word);
                if is_noun {
                    return TokenType::Noun(sym);
                }
                return TokenType::Adjective(sym);
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

        let sym = self.interner.intern(word);
        TokenType::Adjective(sym)
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
}
