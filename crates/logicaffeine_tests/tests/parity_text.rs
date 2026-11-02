//! Stdlib-parity north-star corpus — domain: text
//!
//! Auto-scaffolded from benchmarks/parity/catalog/superset.json by
//! scripts/bench/gen-parity-corpus.py. Each test mirrors one deduplicated capability of the
//! C/Python/Rust/Go (+Java) superset. `#[ignore]` = parity-pending (not yet implemented);
//! the tree stays green. To LAND a capability: implement it, replace the body with a real
//! `assert_all_tiers` behavioral spec, DELETE the `#[ignore]` line, flip the catalog entry
//! to `status: green`, and bump GREEN_FLOOR in parity_catalog_lock.rs.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Whether a substring occurs in a string.
/// provenance — c: string.h.strstr · python: operator.contains · rust: str.contains · go: strings.Contains · java: java.lang.String.contains
/// LOGOS surface: `s contains sub`
#[test]
fn parity_text_contains() {
    assert_all_tiers(
        "## Main\nLet s be \"hello world\".\nShow s contains \"world\".\nShow s contains \"xyz\".\n",
        "true\nfalse",
    );
}

/// Count non-overlapping occurrences of a substring.
/// provenance — python: builtins.str.count · go: strings.Count
/// LOGOS surface: `the count of sub in s`
#[test]
fn parity_text_count() {
    assert_all_tiers(
        r#"## Main
Show countSub("banana", "a").
Show countSub("aaa", "aa").
"#,
        "3
1",
    );
}

/// Whether a string ends with a suffix.
/// provenance — python: builtins.str.endswith · rust: str.ends_with · go: strings.HasSuffix · java: java.lang.String.endsWith
/// LOGOS surface: `s ends with p`
#[test]
fn parity_text_ends_with() {
    assert_all_tiers(
        "## Main\nShow endsWith(\"hello world\", \"world\").\nShow endsWith(\"hello\", \"xyz\").\n",
        "true\nfalse",
    );
}

/// Position of the first occurrence of a substring, or absent.
/// provenance — c: string.h.strstr · python: builtins.str.find · rust: str.find · go: strings.Index · java: java.lang.String.indexOf
/// LOGOS surface: `the index of sub in s`
#[test]
fn parity_text_index_of() {
    assert_all_tiers(
        "## Main
Show indexOf(\"hello\", \"ll\").
Show indexOf(\"hello\", \"xyz\").
",
        "3
0",
    );
}

/// Whether every character is a decimal digit.
/// provenance — c: ctype.h.isdigit · python: builtins.str.isdigit
/// LOGOS surface: `s is all digits`
#[test]
fn parity_text_is_digit() {
    assert_all_tiers(
        r#"## Main
Show allDigits("12345").
Show allDigits("12a45").
"#,
        "true
false",
    );
}

/// Join a sequence of strings with a separator.
/// provenance — python: builtins.str.join · go: strings.Join · java: java.lang.String.join
/// LOGOS surface: `parts joined with sep`
#[test]
fn parity_text_join() {
    assert_all_tiers(
        r#"## Main
Let p be split("a,b,c", ",").
Show join(p, "-").
"#,
        "a-b-c",
    );
}

/// Number of characters in a string.
/// provenance — c: string.h.strlen · python: builtins.len · rust: str.len · java: java.lang.String.length
/// LOGOS surface: `length of s`
#[test]
fn parity_text_length() {
    assert_all_tiers("## Main\nLet s be \"hello\".\nShow length of s.\n", "5");
}

/// Split a string into its lines.
/// provenance — python: builtins.str.splitlines · rust: str.lines
/// LOGOS surface: `the lines of s`
#[test]
fn parity_text_lines() {
    assert_all_tiers(
        "## Main
Let l be lines(\"a
b
c\").
Show length of l.
Show item 2 of l.
",
        "3
b",
    );
}

/// Lowercase a string (locale-independent).
/// provenance — python: builtins.str.lower · rust: str.to_lowercase · go: strings.ToLower · java: java.lang.String.toLowerCase
/// LOGOS surface: `the lowercase form of s`
#[test]
fn parity_text_lower() {
    assert_all_tiers(
        r#"## Main
Show lower("Hello World").
"#,
        "hello world",
    );
}

/// Concatenate a string with itself n times.
/// provenance — python: operator.mul · rust: str.repeat · go: strings.Repeat · java: java.lang.String.repeat
/// LOGOS surface: `n copies of s`
#[test]
fn parity_text_repeat() {
    assert_all_tiers("## Main\nShow repeatText(\"ab\", 3).\n", "ababab");
}

/// Replace all occurrences of a substring.
/// provenance — python: builtins.str.replace · rust: str.replace · go: strings.ReplaceAll · java: java.lang.String.replace
/// LOGOS surface: `s with from replaced by to`
#[test]
fn parity_text_replace() {
    assert_all_tiers(
        r#"## Main
Show replace("a-b-c", "-", "+").
"#,
        "a+b+c",
    );
}

/// Reverse the characters of a string.
/// provenance — python: builtins.reversed · java: java.lang.String.chars
/// LOGOS surface: `the reversed form of s`
#[test]
fn parity_text_reverse() {
    assert_all_tiers(
        r#"## Main
Show reverseText("hello").
"#,
        "olleh",
    );
}

/// Split a string on a separator into a sequence.
/// provenance — python: builtins.str.split · rust: str.split · go: strings.Split · java: java.lang.String.split
/// LOGOS surface: `s split on sep`
#[test]
fn parity_text_split() {
    assert_all_tiers(
        r#"## Main
Let p be split("a,b,c", ",").
Show length of p.
Show item 1 of p.
Show item 3 of p.
"#,
        "3
a
c",
    );
}

/// Split a string on runs of whitespace.
/// provenance — python: builtins.str.split · rust: str.split_whitespace · go: strings.Fields
/// LOGOS surface: `the words of s`
#[test]
fn parity_text_words() {
    assert_all_tiers(
        r#"## Main
Let w be words("  hello   world  ").
Show length of w.
Show item 1 of w.
Show item 2 of w.
"#,
        "2
hello
world",
    );
}

/// Whether a string begins with a prefix.
/// provenance — python: builtins.str.startswith · rust: str.starts_with · go: strings.HasPrefix · java: java.lang.String.startsWith
/// LOGOS surface: `s starts with p`
#[test]
fn parity_text_starts_with() {
    assert_all_tiers(
        "## Main\nShow startsWith(\"hello world\", \"hello\").\nShow startsWith(\"hello\", \"world\").\n",
        "true\nfalse",
    );
}

/// Remove leading and trailing whitespace.
/// provenance — python: builtins.str.strip · rust: str.trim · go: strings.TrimSpace · java: java.lang.String.strip
/// LOGOS surface: `the trimmed form of s`
#[test]
fn parity_text_trim() {
    assert_all_tiers(
        r#"## Main
Show trim("  hi there  ").
"#,
        "hi there",
    );
}

/// Remove trailing whitespace.
/// provenance — python: builtins.str.rstrip · rust: str.trim_end · go: strings.TrimRight · java: java.lang.String.stripTrailing
/// LOGOS surface: `s with trailing spaces removed`
#[test]
fn parity_text_trim_end() {
    assert_all_tiers(
        r#"## Main
Show trimEnd("hi   ").
"#,
        "hi",
    );
}

/// Remove leading whitespace.
/// provenance — python: builtins.str.lstrip · rust: str.trim_start · go: strings.TrimLeft · java: java.lang.String.stripLeading
/// LOGOS surface: `s with leading spaces removed`
#[test]
fn parity_text_trim_start() {
    assert_all_tiers(
        r#"## Main
Show trimStart("   hi").
"#,
        "hi",
    );
}

/// Uppercase a string (locale-independent).
/// provenance — python: builtins.str.upper · rust: str.to_uppercase · go: strings.ToUpper · java: java.lang.String.toUpperCase
/// LOGOS surface: `the uppercase form of s`
#[test]
fn parity_text_upper() {
    assert_all_tiers(
        r#"## Main
Show upper("Hello World").
"#,
        "HELLO WORLD",
    );
}

/// Whether every character is a letter. c: ctype isalpha · py: str.isalpha · java: Character.isLetter
#[test]
fn parity_text_is_alpha() {
    assert_all_tiers(
        "## Main\nShow isAlpha(\"hello\").\nShow isAlpha(\"hel1o\").\nShow isAlpha(\"\").\n",
        "true\nfalse\nfalse",
    );
}
/// Whether every character is a letter or digit. c: ctype isalnum · py: str.isalnum
#[test]
fn parity_text_is_alnum() {
    assert_all_tiers(
        "## Main\nShow isAlnum(\"abc123\").\nShow isAlnum(\"abc!\").\n",
        "true\nfalse",
    );
}
/// Whether every character is whitespace. c: ctype isspace · py: str.isspace
#[test]
fn parity_text_is_space() {
    assert_all_tiers(
        "## Main\nShow isSpaceText(\"   \").\nShow isSpaceText(\" a \").\n",
        "true\nfalse",
    );
}
/// Whether the string is uppercased (has letters, none lower). c: ctype isupper · py: str.isupper
#[test]
fn parity_text_is_upper() {
    assert_all_tiers(
        "## Main\nShow isUpper(\"HELLO\").\nShow isUpper(\"Hello\").\n",
        "true\nfalse",
    );
}
/// Whether the string is lowercased. c: ctype islower · py: str.islower
#[test]
fn parity_text_is_lower() {
    assert_all_tiers(
        "## Main\nShow isLower(\"hello\").\nShow isLower(\"Hello\").\n",
        "true\nfalse",
    );
}
/// First character uppercased, rest lowercased. py: str.capitalize
#[test]
fn parity_text_capitalize() {
    assert_all_tiers(
        "## Main\nShow capitalize(\"hELLO\").\nShow capitalize(\"world\").\n",
        "Hello\nWorld",
    );
}

/// Left-pad to a width with a fill char. py: str.rjust · java: (String.format)
#[test]
fn parity_text_pad_start() {
    assert_all_tiers(
        "## Main\nShow padStart(\"42\", 5, \"0\").\nShow padStart(\"hello\", 3, \"0\").\n",
        "00042\nhello",
    );
}
/// Right-pad to a width. py: str.ljust
#[test]
fn parity_text_pad_end() {
    assert_all_tiers("## Main\nShow padEnd(\"42\", 5, \"-\").\n", "42---");
}
/// Swap the case of every letter. py: str.swapcase
#[test]
fn parity_text_swapcase() {
    assert_all_tiers("## Main\nShow swapCase(\"Hello World\").\n", "hELLO wORLD");
}

/// Title-case each word. py: str.title
#[test]
fn parity_text_title() {
    assert_all_tiers("## Main\nShow title(\"hello WORLD\").\n", "Hello World");
}
/// Remove a prefix if present. py: str.removeprefix · rust: str.strip_prefix
#[test]
fn parity_text_remove_prefix() {
    assert_all_tiers(
        "## Main\nShow removePrefix(\"hello\", \"he\").\nShow removePrefix(\"hello\", \"xy\").\n",
        "llo\nhello",
    );
}
/// Remove a suffix if present. py: str.removesuffix · rust: str.strip_suffix
#[test]
fn parity_text_remove_suffix() {
    assert_all_tiers(
        "## Main\nShow removeSuffix(\"hello\", \"lo\").\nShow removeSuffix(\"hello\", \"xy\").\n",
        "hel\nhello",
    );
}
/// Last occurrence position (0 if none). py: str.rfind · java: String.lastIndexOf
#[test]
fn parity_text_last_index_of() {
    assert_all_tiers(
        "## Main\nShow lastIndexOf(\"abcabc\", \"bc\").\nShow lastIndexOf(\"abc\", \"xy\").\n",
        "5\n0",
    );
}
/// Character at a 1-based position. py: str.__getitem__ · java: String.charAt
#[test]
fn parity_text_char_at() {
    assert_all_tiers("## Main\nShow charAt(\"hello\", 2).\n", "e");
}

/// Center within a width using spaces. py: str.center
#[test]
fn parity_text_center() {
    assert_all_tiers(
        "## Main\nShow \"[\" combined with center(\"hi\", 6) combined with \"]\".\n",
        "[  hi  ]",
    );
}
/// Left-pad with zeros to a width. py: str.zfill
#[test]
fn parity_text_zfill() {
    assert_all_tiers("## Main\nShow zfill(\"42\", 5).\n", "00042");
}

/// Case-insensitive equality. go: strings.EqualFold · java: String.equalsIgnoreCase
#[test]
fn parity_text_equal_fold() {
    assert_all_tiers(
        "## Main\nShow equalFold(\"Hello\", \"hello\").\nShow equalFold(\"Hello\", \"world\").\n",
        "true\nfalse",
    );
}

/// Three-way lexicographic comparison (-1/0/1). go: strings.Compare · rust: str::cmp
#[test]
fn parity_text_compare() {
    assert_all_tiers(
        "## Main\nShow compareText(\"abc\", \"abd\").\nShow compareText(\"abc\", \"abc\").\nShow compareText(\"abcd\", \"abc\").\n",
        "-1\n0\n1",
    );
}

/// Does the string contain any of the given characters? go: strings.ContainsAny
#[test]
fn parity_text_contains_any() {
    assert_all_tiers(
        "## Main\nShow containsAny(\"hello\", \"xyz\").\nShow containsAny(\"hello\", \"xyl\").\n",
        "false\ntrue",
    );
}

/// Index (1-based) of the first character that is any of the given set, else 0. go: strings.IndexAny
#[test]
fn parity_text_index_any() {
    assert_all_tiers(
        "## Main\nShow indexAny(\"hello\", \"lo\").\nShow indexAny(\"hello\", \"z\").\n",
        "3\n0",
    );
}

/// Substring before the first separator (whole string if absent). go: strings.Cut · py: str.partition
#[test]
fn parity_text_cut_before() {
    assert_all_tiers(
        "## Main\nShow cutBefore(\"key=value\", \"=\").\nShow cutBefore(\"nosep\", \"=\").\n",
        "key\nnosep",
    );
}

/// Substring after the first separator (empty if absent). go: strings.Cut · py: str.partition
#[test]
fn parity_text_cut_after() {
    assert_all_tiers("## Main\nShow cutAfter(\"a->b->c\", \"->\").\nShow \"[\" combined with cutAfter(\"nosep\", \"=\") combined with \"]\".\n", "b->c\n[]");
}

/// Index (1-based) of the last character that is any of the given set, else 0. go: strings.LastIndexAny
#[test]
fn parity_text_last_index_any() {
    assert_all_tiers(
        "## Main\nShow lastIndexAny(\"hello\", \"lo\").\nShow lastIndexAny(\"hello\", \"z\").\n",
        "5\n0",
    );
}

/// Split into at most n parts; the last part keeps the remainder. go: strings.SplitN
#[test]
fn parity_text_split_n() {
    assert_all_tiers(
        "## Main\nLet p be splitN(\"a,b,c,d\", \",\", 2).\nShow length of p.\nShow item 1 of p.\nShow item 2 of p.\n",
        "2\na\nb,c,d",
    );
}

/// Trim any leading/trailing character in the cutset. go: strings.Trim
#[test]
fn parity_text_trim_chars() {
    assert_all_tiers(
        "## Main\nShow trimChars(\"xxyhixyx\", \"xy\").\nShow trimChars(\"hi\", \"z\").\n",
        "hi\nhi",
    );
}

/// Apply a function to every character code, building a new string. go: strings.Map
#[test]
fn parity_text_map() {
    assert_all_tiers(
        "## Main\nShow mapText((c: Int) -> c + 1, \"abc\").\n",
        "bcd",
    );
}

/// Greedy word-wrap into a single string. py: textwrap.fill
#[test]
fn parity_text_fill() {
    assert_all_tiers(
        "## Main\nShow fillText(\"The quick brown fox jumps\", 10).\nShow fillText(\"one two three four\", 8).\n",
        "The quick\nbrown fox\njumps\none two\nthree\nfour",
    );
}
/// Greedy word-wrap into a list of lines. py: textwrap.wrap
#[test]
fn parity_text_wrap() {
    assert_all_tiers(
        "## Main\nShow length of wrapText(\"aa bb cc dd ee\", 5).\nShow item 2 of wrapText(\"aa bb cc dd ee\", 5).\nShow item 1 of wrapText(\"The quick brown fox jumps\", 10).\n",
        "3\ncc dd\nThe quick",
    );
}
