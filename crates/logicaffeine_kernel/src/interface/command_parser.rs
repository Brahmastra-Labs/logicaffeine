//! Parser for Vernacular commands.
//!
//! Commands:
//! - Definition name : type := term.
//! - Definition name := term.  (type inferred)
//! - Definition name.{u, v} : type := term.  (universe-polymorphic)
//! - Theorem name : proposition := proof.  (checked, then opaque)
//! - Opaque name : type := value.  (checked, then opaque — data)
//! - Axiom name : type.  (an explicit, listed assumption)
//! - Check term.
//! - Eval term.
//! - Inductive Name := C1 : T1 | C2 : T2.
//! - Inductive Name.{u} (params) : arity := C1 : T1 | … with Name2 (params) : arity := … .
//!   (a block registered exactly as declared: the written parameters, the declared arity, one
//!   mutual block)
//!
//! # Literate Syntax (dispatched to literate_parser)
//!
//! - A Bool is either Yes or No.
//! - ## To add (n: Nat) and (m: Nat) -> Nat: ...
//!
//! # Surface extensions (dispatched to the REPL's `Surface`)
//!
//! - Notation "a ⊕ b" := ((myOp a) b).
//! - Infixl 65 "⋄" := f.   Infixr 65 "▷" := g.   Prefix "∽" := neg.   Postfix "‼" := bang.
//! - Macro "twice $e" := ...   MacroRules "list![$x, $xs]" := ...
//! - Section Name.   End Name.   (and `Local` before any of the above)
//!
//! # Namespaces (they decide how a name READS, and declare nothing)
//!
//! - Namespace Outer.Inner.   End Outer.Inner.   — declarations inside are keyed `Outer.Inner.x`,
//!   and a short name written inside resolves against the enclosing namespaces, longest first.
//! - Open Alpha Beta.   Open Alpha in `<command>`.   — the scoped form covers exactly one command.
//! - Open Alpha (a b).   Open Alpha hiding (a).   Open Alpha renaming a -> b.   — a NARROWED
//!   opening. Opening a namespace wholesale is what makes short names ambiguous, so these are how
//!   an ambiguity is AVOIDED rather than reported; each names members, so each is checked against
//!   the environment. An explicit form matches a PREFIX (`Open Foo (x).` reaches `Foo.x.y`), a
//!   `hiding` matches a whole name — Lean's asymmetry, and measured against lean 4.14.0.
//! - Protected Definition Alpha.x : …   — an ordinary constant that stops answering to `x`.
//! - Private Definition x : …   — accepted and INERT: Lean's `private` is transparent to the
//!   module that declares it, and a session is one module.
//! - Section.   End.   — Lean's ANONYMOUS section; a bare `End` closes only an anonymous one.
//! - Export Alpha (token other).   — `token` answers to `Alpha.token` in the current namespace.
//! - `_root_.x` is the absolute `x`, whatever is current or open.
//!
//! A DECLARATION NAME MUST BE ONE A TERM CAN SPELL, and it is checked by asking the term door
//! rather than by restating its grammar: see `super::declaration_name`. Two things must hold and
//! implies the other — the term door reads the spelling back as this same name (so no reserved
//! word, and no quoted component after a `.`), and the flat key the context is indexed by reads
//! back as this same name (so «a.b» and `a.b` cannot collide on one key).
//!
//! A DECLARATION NAME IS HIERARCHICAL: `Definition Outer.Inner.value : …` is one name of three
//! components, and Lean's rule that such a declaration elaborates with `Outer.Inner` current is
//! what makes its body able to say `value` and mean its sibling.
//!
//! # Prose
//!
//! - /-- doc -/   /-! module doc -/   /- block -/   -- line

use super::command::Command;
use super::error::ParseError;
use super::literate_parser;
use super::surface::{Assoc, Name, OpenClause, OperatorForm, Surface, SurfaceDecl};
use super::term_parser::TermParser;
use crate::{Term, Universe};

/// Parse ONE command, and require it to be the WHOLE input.
///
/// ★ THE KERNEL MUST NOT ACCEPT A PREFIX. This used to read one command off the front and discard
/// the rest without a word, so `Repl::execute` on a two-block document returned `Ok` having proved
/// only the first — and the discarded remainder could be a theorem the kernel would have REFUSED.
/// `5 < 2` by `lia` is rejected when it stands alone and was accepted by silence when it followed a
/// true theorem, so concatenation walked straight around the conclusion gate. A document is read
/// whole, by [`crate::interface::Repl::execute_document`]; a single command is read whole here.
pub fn parse_command(input: &str) -> Result<Command, ParseError> {
    parse_command_in(&Surface::new(), input)
}

/// [`parse_command`], reading the notation and macros a session has declared.
pub fn parse_command_in(surface: &Surface, input: &str) -> Result<Command, ParseError> {
    let (cmd, consumed) = parse_command_prefix_in(surface, input)?;
    let rest = input[consumed..].trim();
    if !rest.is_empty() {
        return Err(ParseError::TrailingInput {
            at: consumed,
            rest: rest.to_string(),
        });
    }
    Ok(cmd)
}

/// Parse ONE command off the FRONT of `input`, returning it with the number of bytes consumed.
///
/// This is the primitive both doors are built from: `parse_command_in` adds "and nothing
/// followed", and the document splitter loops it. Every branch reports its own extent — the
/// literate parsers from their own cursor, the Coq-style ones from the sentence terminator — so no
/// caller has to guess where a command ended.
pub fn parse_command_prefix_in(
    surface: &Surface,
    input: &str,
) -> Result<(Command, usize), ParseError> {
    let lead = input.len() - input.trim_start().len();
    let rest = &input[lead..];

    // ============================================================
    // PROSE — accepted, and never a declaration
    // ============================================================

    if let Some(len) = comment_len(rest)? {
        return Ok((Command::Comment, lead + len));
    }

    // ============================================================
    // LITERATE SYNTAX DISPATCH (check first)
    // ============================================================

    // 1. Literate Data Definition: "A Nat is either..." or "An Option is either..."
    if (rest.starts_with("A ") || rest.starts_with("An ")) && rest.contains(" is either") {
        let (cmd, n) = literate_parser::parse_inductive(rest)?;
        return Ok((cmd, lead + with_terminator(rest, n)));
    }

    // 2. Literate Function Definition: "## To add..."
    if rest.starts_with("## To ") {
        let (cmd, n) = literate_parser::parse_definition(rest)?;
        return Ok((cmd, lead + with_terminator(rest, n)));
    }

    // 3. Literate Constant Definition: "Let X be Y."
    if rest.starts_with("Let ") && rest.contains(" be ") {
        let (cmd, n) = literate_parser::parse_let_definition(rest)?;
        return Ok((cmd, lead + with_terminator(rest, n)));
    }

    // 4. Literate Theorem Declaration: "## Theorem: Name"
    if rest.starts_with("## Theorem:") {
        let (cmd, n) = literate_parser::parse_theorem(rest)?;
        return Ok((cmd, lead + with_terminator(rest, n)));
    }

    // 5. A `## Theory` DEVELOPMENT is not a command of this door. Its `Axiom … : for all …` and
    //    `Theorem … : prove …` sentences are first-order statements proved in citation order by the
    //    theory driver, so the block is refused BY NAME and sent there, rather than reported as an
    //    unknown `##`.
    if let Some(after) = rest.strip_prefix("## Theory") {
        if after.starts_with(char::is_whitespace) {
            let header = rest.lines().next().unwrap_or(rest).trim_end();
            return Err(ParseError::Unsupported {
                what: format!("the development `{header}`"),
                why: "a `## Theory` block is checked by the theory driver \
                      (`logicaffeine_proof::development`, run end to end by \
                      `ui_bridge::compile_theory_for_ui`), which proves each theorem from the \
                      development's axioms; the kernel's document door reads declarations and \
                      commands, not developments"
                    .to_string(),
            });
        }
    }

    // ============================================================
    // NAMESPACE SCOPE — it declares nothing, it decides how names READ
    // ============================================================

    // `Open` is dispatched ahead of the sentence split because `Open Alpha in <command>` brackets a
    // whole command, whose own extent is the only thing that says where this one ends.
    if let Some(after_open) = strip_word(rest, "Open") {
        let (opens, tail) = open_clauses(after_open)?;
        if let Some(after_in) = strip_word(tail, "in") {
            let (inner, n) = parse_command_prefix_in(surface, after_in)?;
            let consumed = (rest.len() - after_in.len()) + n;
            return Ok((
                Command::OpenIn {
                    opens,
                    inner: Box::new(inner),
                },
                lead + consumed,
            ));
        }
        let terminated = tail.strip_prefix('.').ok_or_else(|| {
            ParseError::Missing(
                "`.` after `Open <namespace>` \u{2014} or `in <command>` to scope the opening"
                    .to_string(),
            )
        })?;
        return Ok((Command::Open(opens), lead + (rest.len() - terminated.len())));
    }

    // ============================================================
    // EXISTING COQ-STYLE DISPATCH (fallback)
    // ============================================================

    let end = sentence_end(rest);
    let sentence = rest[..end].trim_end();
    let body = sentence.strip_suffix('.').unwrap_or(sentence).trim();

    // `Local` is the explicit spelling of the scoping a section already gives, and is accepted
    // so a file copied from Coq or Lean reads the same here.
    let body = body.strip_prefix("Local ").unwrap_or(body).trim_start();

    // VISIBILITY IS EXACTLY ONE QUALIFIER, and reading it ONCE is what makes that true by
    // construction rather than by a check somebody has to remember. `private protected def` is a
    // parse error in lean 4.14.0 in BOTH orders ("unexpected token 'protected'; expected … 'def'
    // …"), and here the second word simply stays in `body` and is reported as the unknown command
    // word it then is.
    //
    // `Private` is accepted and INERT, and that is Lean's answer rather than a shrug: Lean mangles
    // a private name with the defining module's hash (`mkPrivateName`) and then makes the mangling
    // transparent to that same module — `resolveQualifiedName` and `resolveExact` both fall back to
    // the private form (`Lean/ResolveName.lean:107-110` and `:127-131`) — so WITHIN one module a
    // private declaration answers to every spelling an ordinary one does. Measured on lean 4.14.0:
    // `private def M.hidden`, then `M.hidden`, `M.user` and (under `open M`) `hidden` all print 3.
    // A session is one module, so refusing it would block a transcription that is CORRECT here.
    //
    // `Protected` is the opposite case, and that is why the two are not one line: it is a
    // resolution fact in EVERY module, including this one. It WRAPS whatever follows, so every
    // declaration door gets it at once and none of them has to carry a flag it might forget to
    // pass on.
    let (body, protected) = match strip_word(body, "Private") {
        Some(rest) => (rest, false),
        None => match strip_word(body, "Protected") {
            Some(rest) => (rest, true),
            None => (body, false),
        },
    };

    let cmd = if body.starts_with("Definition") {
        parse_definition(surface, body[10..].trim_start())
    } else if let Some(rest) = strip_word(body, "Theorem") {
        parse_opaque(surface, rest, true)
    } else if let Some(rest) = strip_word(body, "Opaque") {
        parse_opaque(surface, rest, false)
    } else if let Some(rest) = strip_word(body, "Axiom") {
        parse_axiom(surface, rest)
    } else if let Some(rest) = strip_word(body, "Quotient") {
        parse_quotient(rest)
    } else if let Some(rest) = strip_word(body, "Structure") {
        parse_structure(rest)
    } else if body.starts_with("Check") {
        parse_check(surface, body[5..].trim_start())
    } else if body.starts_with("Eval") {
        parse_eval(surface, body[4..].trim_start())
    } else if body.starts_with("Inductive") {
        parse_inductive(surface, body[9..].trim_start())
    } else if body.starts_with("Notation") {
        parse_notation(body[8..].trim_start())
    } else if body.starts_with("Infixl") {
        parse_fixity(
            body[6..].trim_start(),
            OperatorForm::Infix,
            Assoc::Left,
            true,
        )
    } else if body.starts_with("Infixr") {
        parse_fixity(
            body[6..].trim_start(),
            OperatorForm::Infix,
            Assoc::Right,
            true,
        )
    } else if body.starts_with("Prefix") {
        parse_fixity(
            body[6..].trim_start(),
            OperatorForm::Prefix,
            Assoc::Right,
            false,
        )
    } else if body.starts_with("Postfix") {
        parse_fixity(
            body[7..].trim_start(),
            OperatorForm::Postfix,
            Assoc::Left,
            false,
        )
    } else if body.starts_with("MacroRules") {
        parse_macro(body[10..].trim_start())
    } else if body.starts_with("Macro") {
        parse_macro(body[5..].trim_start())
    } else if let Some(printed) = strip_word(body, "Print") {
        parse_print(printed)
    } else if let Some(exported) = strip_word(body, "Export") {
        parse_export(exported)
    } else if let Some(named) = strip_word(body, "Namespace") {
        Ok(Command::Surface(SurfaceDecl::OpenNamespace(
            super::declaration_name(named)?,
        )))
    } else if let Some(named) = strip_word(body, "Section") {
        Ok(Command::Surface(SurfaceDecl::OpenSection(optional_name(
            named,
        )?)))
    } else if let Some(named) = strip_word(body, "End") {
        Ok(Command::Surface(SurfaceDecl::CloseSection(optional_name(
            named,
        )?)))
    } else {
        Err(ParseError::UnknownCommand(
            body.split_whitespace().next().unwrap_or(body).to_string(),
        ))
    }?;
    let cmd = if protected {
        Command::Protected(Box::new(cmd))
    } else {
        cmd
    };
    Ok((cmd, lead + end))
}

/// A scope name that MAY be absent: `Section.` and `End.` are Lean's anonymous forms
/// (`section` / `end` with no name), and the ordinary way to scope a `local` without inventing a
/// label for it. A name that IS written is held to the whole declaration-name grammar, so
/// `End fun.` is refused for the same reason `Definition fun` is.
fn optional_name(raw: &str) -> Result<Option<String>, ParseError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    declaration_name(raw).map(Some)
}

/// Parse `Print Axioms <name>` — our `#print axioms`.
///
/// `Axioms` is required rather than assumed: `Print` is a whole family in Coq and in Lean
/// (`#print`, `#print axioms`, `#print notation`), and a bare `Print foo` that silently meant one
/// of them would be a command word answering to a question nobody asked. An unknown subject is
/// reported as such, with the one spelling that exists.
fn parse_print(input: &str) -> Result<Command, ParseError> {
    let Some(named) = strip_word(input.trim(), "Axioms") else {
        return Err(ParseError::Unsupported {
            what: format!("`Print {}`", input.trim()),
            why: "the only report is `Print Axioms <name>.` — what a declaration transitively \
                  rests on, which is Lean's `#print axioms`"
                .to_string(),
        });
    };
    Ok(Command::PrintAxioms(super::declaration_name(named)?))
}

/// Parse `Export Alpha (token other)` — a source namespace and the names to alias into the current
/// one. The list is REQUIRED, exactly as Lean requires it: there is no way to enumerate a namespace's
/// members, so a bare `Export Alpha` could only be a silent no-op.
fn parse_export(input: &str) -> Result<Command, ParseError> {
    let (namespace, len) = Name::scan(input.trim())
        .ok_or_else(|| ParseError::Missing("the namespace to export from".to_string()))?;
    let rest = input.trim()[len..].trim_start();
    let inner = rest
        .strip_prefix('(')
        .and_then(|r| r.strip_suffix(')'))
        .ok_or_else(|| {
            ParseError::Missing(
                "a parenthesised list of names, as in `Export Alpha (token other)` \u{2014} a \
                 namespace's members cannot be enumerated, so the names must be written"
                    .to_string(),
            )
        })?;
    let mut names = Vec::new();
    let mut cursor = inner.trim();
    while !cursor.is_empty() {
        let (name, n) = Name::scan(cursor).ok_or_else(|| {
            ParseError::InvalidIdent(format!("{cursor:?} \u{2014} not a name to export"))
        })?;
        names.push(name);
        cursor = cursor[n..]
            .trim_start()
            .trim_start_matches(',')
            .trim_start();
    }
    if names.is_empty() {
        return Err(ParseError::Missing(
            "at least one name to export".to_string(),
        ));
    }
    Ok(Command::Export { namespace, names })
}

/// Parse `Notation "<pattern>" := <template>` with an optional leading precedence.
fn parse_notation(input: &str) -> Result<Command, ParseError> {
    let (precedence, rest) = leading_precedence(input);
    let (pattern, rest) = quoted(rest)?;
    let template = after_assign(rest)?;
    Ok(Command::Surface(SurfaceDecl::Notation {
        pattern,
        template,
        precedence,
        // With no declared precedence two notations sit at the same level, and an unparenthesised
        // chain of unfamiliar operators reads to the RIGHT.
        assoc: Assoc::Right,
        operator_form: None,
    }))
}

/// Parse `Infixl 65 "<op>" := <fn>` and its siblings, where the command word gives the shape.
fn parse_fixity(
    input: &str,
    form: OperatorForm,
    assoc: Assoc,
    precedence_required: bool,
) -> Result<Command, ParseError> {
    let (precedence, rest) = leading_precedence(input);
    if precedence_required && precedence.is_none() {
        return Err(ParseError::Missing(
            "a precedence — an infix operator must say how tightly it binds".to_string(),
        ));
    }
    let (pattern, rest) = quoted(rest)?;
    let template = after_assign(rest)?;
    Ok(Command::Surface(SurfaceDecl::Notation {
        pattern,
        template,
        precedence,
        assoc,
        operator_form: Some(form),
    }))
}

/// Parse `Macro "<pattern>" := <template>` / `MacroRules "<pattern>" := <template>`.
fn parse_macro(input: &str) -> Result<Command, ParseError> {
    let (pattern, rest) = quoted(input)?;
    let template = after_assign(rest)?;
    Ok(Command::Surface(SurfaceDecl::Macro { pattern, template }))
}

/// A leading decimal precedence, if one is written.
fn leading_precedence(input: &str) -> (Option<u32>, &str) {
    let digits: String = input.chars().take_while(char::is_ascii_digit).collect();
    match digits.parse::<u32>() {
        Ok(n) => (Some(n), input[digits.len()..].trim_start()),
        Err(_) => (None, input),
    }
}

/// The contents of a leading `"…"`, and what follows it.
fn quoted(input: &str) -> Result<(String, &str), ParseError> {
    let input = input.trim_start();
    let rest = input
        .strip_prefix('"')
        .ok_or_else(|| ParseError::Expected {
            expected: "a quoted pattern".to_string(),
            found: input.chars().take(12).collect::<String>(),
        })?;
    let end = rest
        .find('"')
        .ok_or_else(|| ParseError::Missing("the closing quote of the pattern".to_string()))?;
    Ok((rest[..end].to_string(), &rest[end + 1..]))
}

/// The text after `:=`.
fn after_assign(input: &str) -> Result<String, ParseError> {
    let input = input.trim_start();
    let body = input
        .strip_prefix(":=")
        .ok_or(ParseError::Missing(":=".to_string()))?
        .trim();
    if body.is_empty() {
        return Err(ParseError::Missing("the expansion".to_string()));
    }
    Ok(body.to_string())
}

/// A literate block ends where its own parser stopped, PLUS the sentence terminator if one
/// follows.
///
/// `pub(super)` so the CONTEXT-AWARE door in the parent module measures a block with this same
/// function rather than a checked copy of it: two doors that disagreed about where a command ended
/// would split one document two different ways depending on which of them read it. `## To double (n: Int) -> Int: … Yield (add n n).` leaves its final `.` unread — a
/// full stop is punctuation, not input — and without this the block would look like a command
/// with a stray period trailing it.
pub(super) fn with_terminator(rest: &str, n: usize) -> usize {
    let tail = &rest[n..];
    let spaces = tail.len() - tail.trim_start_matches([' ', '\t']).len();
    if tail[spaces..].starts_with('.') {
        n + spaces + 1
    } else {
        n
    }
}

/// The end of a vernacular SENTENCE: just past the first `.` that is followed by whitespace or by
/// end-of-input and is not inside a string literal. Coq's own rule, and the reason `Nat.succ`,
/// `3.14` and `"a. b"` can all live inside a term without ending the command.
pub(super) fn sentence_end(input: &str) -> usize {
    let mut in_string = false;
    let mut in_name = false;
    let mut escaped = false;
    for (i, c) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        // A «quoted name» is verbatim, and a `.` inside one — `«a. b»` — is part of the NAME. The
        // term grammar reads it that way; a sentence splitter that ended the command there would
        // hand the term door half a name.
        if in_name {
            in_name = c != '\u{bb}';
            continue;
        }
        match c {
            '\u{ab}' if !in_string => in_name = true,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '.' if !in_string => {
                let after = i + c.len_utf8();
                if input[after..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
                {
                    return after;
                }
            }
            _ => {}
        }
    }
    input.len()
}

/// How many bytes of documentation or comment start `input`, or `None` if it starts with neither.
///
/// `/-- … -/` (a doc comment), `/-! … -/` (a module doc) and `/- … -/` nest, exactly as they do in
/// Lean; `-- …` runs to the end of the line. An UNTERMINATED block comment is an error rather than
/// a silently-swallowed remainder — otherwise one missing `-/` eats the rest of a document.
fn comment_len(input: &str) -> Result<Option<usize>, ParseError> {
    if input.starts_with("--") {
        return Ok(Some(input.find('\n').map_or(input.len(), |i| i + 1)));
    }
    if !input.starts_with("/-") {
        return Ok(None);
    }
    let bytes = input.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'-' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'-' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Ok(Some(i));
            }
        } else {
            i += 1;
        }
    }
    Err(ParseError::Missing(
        "closing `-/` for a documentation comment".to_string(),
    ))
}

/// A declaration name, as the flat key the context is indexed at.
///
/// ★ THE RULE ITSELF LIVES IN THE PARENT MODULE, and so this is four lines. Both declaration doors
/// — this one's `Definition`/`Inductive` and the literate `## To`/`## Theorem:`/`Let … be`/
/// `A … is either` — must key a name at the same grammar, and a rule that only one door applied
/// would not be a rule but a door's habit. It used to be spelled out here, where the literate door
/// could not reach it; it is now `super::declaration_name` and this is the caller that wants the
/// key rather than the components.
fn declaration_name(raw: &str) -> Result<String, ParseError> {
    super::declaration_name(raw).map(|n| n.key())
}

/// `input` with `word` stripped from its front, if `word` stands there as a whole word.
///
/// The Coq-style dispatch below matches command words by bare prefix, which is fine for words no
/// identifier begins with. `Open` and `Namespace` are matched here instead because both are
/// followed by a NAME, and `Opened` must not read as `Open` + `ed`.
fn strip_word<'a>(input: &'a str, word: &str) -> Option<&'a str> {
    let rest = input.strip_prefix(word)?;
    match rest.chars().next() {
        Some(c) if c.is_alphanumeric() || c == '_' || c == '\'' => None,
        _ => Some(rest.trim_start()),
    }
}

/// One or more `open` clauses, and what follows them.
///
/// `Open Alpha Beta.`, `Open Alpha (a b).`, `Open Alpha hiding (a).` and `Open Alpha in …` all
/// land here. A bare namespace is a WHOLESALE opening; a following `(…)` or `hiding` narrows the
/// one just read, and then the list continues, so `Open Alpha (x) Beta.` is two clauses. The list
/// is greedy and stops at the first token that is not a name — which `in` and `hiding` are not
/// allowed to be, since one scopes the opening and the other narrows it.
fn open_clauses(input: &str) -> Result<(Vec<OpenClause>, &str), ParseError> {
    let mut clauses: Vec<OpenClause> = Vec::new();
    let mut rest = input.trim_start();
    loop {
        if strip_word(rest, "in").is_some() || strip_word(rest, "hiding").is_some() {
            break;
        }
        let Some((namespace, len)) = Name::scan(rest) else {
            break;
        };
        rest = rest[len..].trim_start();
        let mut clause = OpenClause::all(namespace);
        if rest.starts_with('(') {
            let (only, after) = parenthesised_names(rest, "open")?;
            clause.only = Some(only);
            rest = after;
        } else if let Some(after_hiding) = strip_word(rest, "hiding") {
            let (hidden, after) = if after_hiding.starts_with('(') {
                parenthesised_names(after_hiding, "hide")?
            } else {
                bare_names(after_hiding)?
            };
            clause.hiding = hidden;
            rest = after;
        } else if let Some(after_renaming) = strip_word(rest, "renaming") {
            let (pairs, after) = renaming_pairs(after_renaming)?;
            clause.renaming = pairs;
            rest = after;
        }
        clauses.push(clause);
    }
    if clauses.is_empty() {
        return Err(ParseError::Missing("a namespace to open".to_string()));
    }
    Ok((clauses, rest))
}

/// `(a b c)` — a parenthesised name list, and what follows it. Commas are optional separators,
/// as they are in `Export`'s list.
fn parenthesised_names<'a>(input: &'a str, what: &str) -> Result<(Vec<Name>, &'a str), ParseError> {
    let after_open = input
        .strip_prefix('(')
        .expect("the caller checked for the opening parenthesis");
    let close = after_open
        .find(')')
        .ok_or_else(|| ParseError::Missing(format!("`)` closing the list of names to {what}")))?;
    let (names, _) = bare_names(&after_open[..close])?;
    Ok((names, after_open[close + 1..].trim_start()))
}

/// `a -> b, c -> d` — the pairs of a `renaming` clause, and what follows them.
///
/// The arrow is written `->` or `→`, as Lean writes it either way: `Lean/Elab/Open.lean:89`
/// matches `$ns renaming $[$froms -> $tos],*`, and every library writes the glyph. The MEMBER is
/// on the left and the local spelling on the right — the direction that reads as "this becomes
/// that", and the direction Lean records it in (`OpenDecl.explicit to.getId declName`, line 96).
fn renaming_pairs(input: &str) -> Result<(Vec<(Name, Name)>, &str), ParseError> {
    let mut pairs = Vec::new();
    let mut rest = input.trim_start();
    loop {
        let Some((member, len)) = Name::scan(rest) else {
            break;
        };
        let after_member = rest[len..].trim_start();
        let Some(after_arrow) = after_member
            .strip_prefix("->")
            .or_else(|| after_member.strip_prefix('\u{2192}'))
        else {
            // No arrow: this name is not part of the renaming list, so the list ends HERE rather
            // than half-consuming a name the caller still has to read.
            break;
        };
        let after_arrow = after_arrow.trim_start();
        let (local, n) = Name::scan(after_arrow).ok_or_else(|| {
            ParseError::Missing(format!(
                "the local spelling after `{member} ->` — a renaming is `<member> -> <name>`"
            ))
        })?;
        pairs.push((member, local));
        rest = after_arrow[n..]
            .trim_start()
            .trim_start_matches(',')
            .trim_start();
    }
    if pairs.is_empty() {
        return Err(ParseError::Missing(
            "at least one `<member> -> <name>` renaming".to_string(),
        ));
    }
    Ok((pairs, rest))
}

/// A run of space-separated names, and what follows them. Commas are skipped so a list may be
/// written either way.
fn bare_names(input: &str) -> Result<(Vec<Name>, &str), ParseError> {
    let mut names = Vec::new();
    let mut rest = input.trim_start();
    while let Some((name, len)) = Name::scan(rest) {
        names.push(name);
        rest = rest[len..]
            .trim_start()
            .trim_start_matches(',')
            .trim_start();
    }
    if names.is_empty() {
        return Err(ParseError::Missing("at least one name".to_string()));
    }
    Ok((names, rest))
}

/// The byte offset of `keyword` in `haystack` as a standalone word, if it occurs.
fn find_keyword(haystack: &str, keyword: &str) -> Option<usize> {
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(keyword) {
        let at = from + rel;
        let before = haystack[..at].chars().next_back();
        let after = haystack[at + keyword.len()..].chars().next();
        if boundary(before) && boundary(after) {
            return Some(at);
        }
        from = at + keyword.len();
    }
    None
}

/// `input` cut at every TOP-LEVEL occurrence of `sep` — outside every bracket, string and
/// «quoted name». [`TermParser::find_top_level`] applied until the input is exhausted, which is
/// the splitting counterpart of the same one scan.
fn split_top_level<'a>(input: &'a str, sep: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut rest = input;
    while let Some(at) = TermParser::find_top_level(rest, sep) {
        parts.push(&rest[..at]);
        rest = &rest[at + sep.len()..];
    }
    parts.push(rest);
    parts
}

/// Parse: name : type := term  OR  name := term
///
/// ★ THE SENTENCE IS SPLIT AT A TOP-LEVEL DELIMITER, NEVER BY A RAW SEARCH. The FIRST `:=` in
/// the text is not always the separator, and neither is the first `:`. A defaulted binder puts a
/// `:=` inside the TYPE — `Definition f : Int -> (m : Int := 3) -> Int := …` split at the DEFAULT
/// and handed the term parser the truncated `Int -> (m : Int`, refused for `Expected ')' found
/// EOF`: a message naming neither the default nor the body. A «guillemet name» puts one inside
/// the NAME (`Definition «a := b» : Nat := Zero.`), and every `(x : T)` binder in a type puts a
/// `:` after the ascription's own. Where a bracket, a string and a «quoted name» begin and end is
/// the TERM grammar's knowledge, so the scan is asked of it rather than re-derived — once per
/// bug — by each door that has to split a sentence.
fn parse_definition(surface: &Surface, input: &str) -> Result<Command, ParseError> {
    // Find the := delimiter
    let assign_pos =
        TermParser::find_top_level(input, ":=").ok_or(ParseError::Missing(":=".to_string()))?;

    let before_assign = input[..assign_pos].trim();
    let body_str = input[assign_pos + 2..].trim();

    // Check if there's a type annotation (: before :=)
    if let Some(colon_pos) = TermParser::find_top_level(before_assign, ":") {
        // Has type annotation: name : type
        let (name, universes) = declaration_head(&before_assign[..colon_pos])?;
        let type_str = before_assign[colon_pos + 1..].trim();

        let (ty, implicit_count) = TermParser::read_declared_type_in(surface, type_str)?;
        let body = TermParser::parse_in(surface, body_str)?;

        Ok(with_universes(
            universes,
            Command::Definition {
                name,
                ty: Some(ty),
                body,
                is_hint: false,
                implicit_count,
                proof_goal: None,
            },
        ))
    } else {
        // No type annotation: name := term
        let (name, universes) = declaration_head(before_assign)?;

        let (body, implicit_count) = TermParser::read_declared_type_in(surface, body_str)?;

        Ok(with_universes(
            universes,
            Command::Definition {
                name,
                ty: None,
                body,
                is_hint: false,
                implicit_count,
                proof_goal: None,
            },
        ))
    }
}

/// Parse `Theorem name : P := proof` (`theorem` true) or `Opaque name : T := value` — both
/// require their type, because both are checked against it and then kept only as their header.
fn parse_opaque(surface: &Surface, input: &str, theorem: bool) -> Result<Command, ParseError> {
    let word = if theorem { "Theorem" } else { "Opaque" };
    let assign_pos = TermParser::find_top_level(input, ":=").ok_or_else(|| {
        ParseError::Missing(format!(
            "`:=` and the {} of the `{word}` \u{2014} it is checked, so it must be written",
            if theorem { "proof" } else { "value" }
        ))
    })?;
    let before_assign = input[..assign_pos].trim();
    let colon_pos = TermParser::find_top_level(before_assign, ":").ok_or_else(|| {
        ParseError::Missing(format!(
            "the `: <type>` of the `{word}` \u{2014} an opaque declaration is known only by its type"
        ))
    })?;
    let (name, universes) = declaration_head(&before_assign[..colon_pos])?;
    // The statement keeps its binder-kind markers, exactly as a `Definition`'s declared type does:
    // `Theorem t {α : Type} (a : α) : …` records how each argument is supplied at a use site.
    let (ty, _) =
        TermParser::read_declared_type_in(surface, before_assign[colon_pos + 1..].trim())?;
    let proof_text = input[assign_pos + 2..].trim();
    if theorem {
        if let Some(script) = proof_text
            .strip_prefix("by")
            .filter(|rest| rest.starts_with(char::is_whitespace))
        {
            let script = parse_tactic_script(surface, script.trim())?;
            return Ok(with_universes(
                universes,
                Command::TheoremBy { name, ty, script },
            ));
        }
    }
    let value = TermParser::parse_in(surface, proof_text)?;
    let cmd = if theorem {
        Command::Theorem {
            name,
            ty,
            proof: value,
        }
    } else {
        Command::Opaque { name, ty, value }
    };
    Ok(with_universes(universes, cmd))
}

/// A tactic script: steps separated by top-level `;` (`intro A x; exact (refl A x)`). The terms a
/// step carries are read in the surface's notation; a name a goal's hypothesis binds is resolved
/// against that goal when the step runs.
fn parse_tactic_script(
    surface: &Surface,
    text: &str,
) -> Result<Vec<crate::tactics::TacticStep>, ParseError> {
    let mut steps = Vec::new();
    let mut rest = text;
    loop {
        let end = TermParser::find_top_level(rest, ";").unwrap_or(rest.len());
        let step = rest[..end].trim();
        if step.is_empty() {
            return Err(ParseError::Missing(
                "a tactic: a script step is empty".to_string(),
            ));
        }
        steps.extend(parse_tactic(surface, step)?);
        if end == rest.len() {
            return Ok(steps);
        }
        rest = &rest[end + 1..];
    }
}

/// One tactic of a script. `rw [h₁, ← h₂]` is one step per rule.
fn parse_tactic(
    surface: &Surface,
    step: &str,
) -> Result<Vec<crate::tactics::TacticStep>, ParseError> {
    use crate::tactics::TacticStep;
    let (word, arg) = match step.find(char::is_whitespace) {
        Some(i) => (&step[..i], step[i..].trim()),
        None => (step, ""),
    };
    let term = |text: &str| {
        if text.is_empty() {
            Err(ParseError::Missing(format!("the term `{word}` takes")))
        } else {
            TermParser::parse_in(surface, text)
        }
    };
    let bare = |s: TacticStep| {
        if arg.is_empty() {
            Ok(vec![s])
        } else {
            Err(ParseError::TrailingInput {
                at: word.len(),
                rest: arg.to_string(),
            })
        }
    };
    let names = |text: &str| -> Result<Vec<String>, ParseError> {
        text.split_whitespace()
            .map(|n| {
                if n.chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
                    && n.chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
                {
                    Ok(n.to_string())
                } else {
                    Err(ParseError::InvalidIdent(n.to_string()))
                }
            })
            .collect()
    };
    // `x` or `x as [a b | c d]`: the hypothesis eliminated and each case's names.
    let eliminated = |text: &str| -> Result<(String, Vec<Vec<String>>), ParseError> {
        let (x, pattern) = match text.find(" as ") {
            Some(i) => (text[..i].trim(), Some(text[i + 4..].trim())),
            None => (text.trim(), None),
        };
        let x = names(x)?;
        let [x] = x.as_slice() else {
            return Err(ParseError::Missing(format!(
                "the one hypothesis `{word}` eliminates"
            )));
        };
        let cases = match pattern {
            None => Vec::new(),
            Some(p) => {
                let inner = p
                    .strip_prefix('[')
                    .and_then(|p| p.strip_suffix(']'))
                    .ok_or_else(|| ParseError::Expected {
                        expected: "`[names | names …]`".to_string(),
                        found: p.to_string(),
                    })?;
                inner.split('|').map(names).collect::<Result<_, _>>()?
            }
        };
        Ok((x.clone(), cases))
    };
    Ok(match word {
        "intro" | "intros" => {
            let xs = names(arg)?;
            if xs.is_empty() {
                return Err(ParseError::Missing("the names `intro` introduces".to_string()));
            }
            vec![TacticStep::Intro(xs)]
        }
        "exact" => vec![TacticStep::Exact(term(arg)?)],
        "apply" => vec![TacticStep::Apply(term(arg)?)],
        "exists" => vec![TacticStep::Exists(term(arg)?)],
        "change" | "show" => vec![TacticStep::Change(term(arg)?)],
        "assumption" => bare(TacticStep::Assumption)?,
        "exfalso" => bare(TacticStep::Exfalso)?,
        "constructor" => bare(TacticStep::Constructor)?,
        "left" => bare(TacticStep::Left)?,
        "right" => bare(TacticStep::Right)?,
        "rfl" | "reflexivity" => bare(TacticStep::Rfl)?,
        "cc" => bare(TacticStep::Cc)?,
        "decide" => bare(TacticStep::Decide)?,
        "simp" if arg.is_empty() => vec![TacticStep::Simp(Vec::new())],
        "simp" => {
            let lemmas = arg.strip_prefix('[').and_then(|a| a.strip_suffix(']')).ok_or_else(|| {
                ParseError::Expected { expected: "`simp` or `simp [l₁, l₂, …]`".to_string(), found: arg.to_string() }
            })?;
            let mut out = Vec::new();
            let mut rest = lemmas;
            loop {
                let end = TermParser::find_top_level(rest, ",").unwrap_or(rest.len());
                out.push(term(rest[..end].trim())?);
                if end == rest.len() {
                    break;
                }
                rest = &rest[end + 1..];
            }
            vec![TacticStep::Simp(out)]
        }
        "have" => {
            let colon = TermParser::find_top_level(arg, ":")
                .ok_or_else(|| ParseError::Missing("`have h : <statement>`'s `:`".to_string()))?;
            let name = names(arg[..colon].trim())?;
            let [name] = name.as_slice() else {
                return Err(ParseError::Missing("the one name `have` gives its statement".to_string()));
            };
            vec![TacticStep::Have(name.clone(), term(arg[colon + 1..].trim())?)]
        }
        "rw" | "rewrite" => {
            let rules = arg.strip_prefix('[').and_then(|a| a.strip_suffix(']')).ok_or_else(|| {
                ParseError::Expected { expected: "`[h]`, `[← h]` or `[h₁, h₂]`".to_string(), found: arg.to_string() }
            })?;
            let mut out = Vec::new();
            let mut rest = rules;
            loop {
                let end = TermParser::find_top_level(rest, ",").unwrap_or(rest.len());
                let rule = rest[..end].trim();
                let (reverse, rule) = match rule.strip_prefix("<-").or_else(|| rule.strip_prefix('←')) {
                    Some(r) => (true, r.trim()),
                    None => (false, rule),
                };
                out.push(TacticStep::Rewrite(term(rule)?, reverse));
                if end == rest.len() {
                    break;
                }
                rest = &rest[end + 1..];
            }
            out
        }
        "induction" => {
            let (x, cases) = eliminated(arg)?;
            vec![TacticStep::Induction(x, cases)]
        }
        "cases" => {
            let (x, cases) = eliminated(arg)?;
            vec![TacticStep::Cases(x, cases)]
        }
        "refine" => vec![TacticStep::Refine(term(arg)?)],
        "subst" => {
            let h = names(arg)?;
            let [h] = h.as_slice() else {
                return Err(ParseError::Missing("the one equation `subst` substitutes along".to_string()));
            };
            vec![TacticStep::Subst(h.clone())]
        }
        "injection" => {
            let (h, groups) = eliminated(arg)?;
            let names = match groups.as_slice() {
                [] => Vec::new(),
                [names] => names.clone(),
                _ => return Err(ParseError::Missing("one list of names for `injection`'s equations".to_string())),
            };
            vec![TacticStep::Injection(h, names)]
        }
        "calc" => {
            let body = arg.strip_prefix('[').and_then(|a| a.strip_suffix(']')).ok_or_else(|| ParseError::Expected {
                expected: "`[R₁ := p₁, R₂ := p₂, …]`".to_string(),
                found: arg.to_string(),
            })?;
            let mut steps = Vec::new();
            let mut rest = body;
            loop {
                let end = TermParser::find_top_level(rest, ",").unwrap_or(rest.len());
                let step = rest[..end].trim();
                let at = TermParser::find_top_level(step, ":=")
                    .ok_or_else(|| ParseError::Missing(format!("the `:=` and proof of the step `{step}`")))?;
                steps.push((term(step[..at].trim())?, term(step[at + 2..].trim())?));
                if end == rest.len() {
                    break;
                }
                rest = &rest[end + 1..];
            }
            vec![TacticStep::Calc(steps)]
        }
        other => {
            return Err(ParseError::Unsupported {
                what: format!("the tactic `{other}`"),
                why: "the tactics are intro, exact, apply, refine, assumption, exfalso, constructor, left, right, \
                      exists, have, rfl, change, rw, subst, injection, calc, cc, decide, simp, induction and cases"
                    .to_string(),
            })
        }
    })
}

/// Parse `Axiom name : T` — a type and nothing else. A `:=` at top level would be a body, and an
/// axiom has none, so it is refused rather than dropped.
fn parse_axiom(surface: &Surface, input: &str) -> Result<Command, ParseError> {
    if TermParser::find_top_level(input, ":=").is_some() {
        return Err(ParseError::Unsupported {
            what: "a body on an `Axiom`".to_string(),
            why:
                "an axiom is an ASSUMPTION \u{2014} it has a type and no proof. A declaration with \
                  a body is a `Theorem`, an `Opaque` or a `Definition`"
                    .to_string(),
        });
    }
    let colon_pos = TermParser::find_top_level(input, ":")
        .ok_or_else(|| ParseError::Missing("the `: <type>` an axiom assumes".to_string()))?;
    let (name, universes) = declaration_head(&input[..colon_pos])?;
    let (ty, _) = TermParser::read_declared_type_in(surface, input[colon_pos + 1..].trim())?;
    Ok(with_universes(universes, Command::Axiom { name, ty }))
}

/// Parse `Quotient <Primitive>.{u, …} (using <equality>)?` — see [`Command::Quotient`].
fn parse_quotient(input: &str) -> Result<Command, ParseError> {
    use crate::quotient::Kind;
    let input = input.trim();
    let word_len = input.find(".{").ok_or_else(|| {
        ParseError::Missing(
            "the universe parameters of a quotient primitive, written `.{u}` right after its name"
                .to_string(),
        )
    })?;
    let kind = match &input[..word_len] {
        "Type" => Kind::Type,
        "Constructor" => Kind::Constructor,
        "Lift" => Kind::Lift,
        "Induction" => Kind::Induction,
        other => {
            return Err(ParseError::Expected {
                expected: "a quotient primitive: `Type`, `Constructor`, `Lift` or `Induction`"
                    .to_string(),
                found: other.to_string(),
            })
        }
    };
    let after = &input[word_len + 2..];
    let close = after.find('}').ok_or_else(|| {
        ParseError::Missing("`}` closing the quotient's universe parameters".to_string())
    })?;
    let params = universe_names(&after[..close])?;
    let rest = after[close + 1..].trim();
    let equality = match strip_word(rest, "using") {
        Some(named) => {
            let (name, len) = Name::scan(named.trim()).ok_or_else(|| {
                ParseError::Missing("the equality a quotient lift is stated with".to_string())
            })?;
            if len != named.trim().len() {
                return Err(ParseError::TrailingInput {
                    at: 0,
                    rest: named.trim()[len..].to_string(),
                });
            }
            Some(name.key())
        }
        None if rest.is_empty() => None,
        None => {
            return Err(ParseError::TrailingInput {
                at: 0,
                rest: rest.to_string(),
            })
        }
    };
    if (kind == Kind::Lift) != equality.is_some() {
        return Err(ParseError::Unsupported {
            what: format!(
                "`Quotient {}` {} an equality",
                &input[..word_len],
                if equality.is_some() {
                    "naming"
                } else {
                    "without"
                }
            ),
            why: "only `Lift` states its respect hypothesis with an equality, and it must name it"
                .to_string(),
        });
    }
    Ok(Command::Universes {
        params,
        inner: Box::new(Command::Quotient { kind, equality }),
    })
}

/// Parse `Structure <inductive> := <projection>*` — see [`Command::Structure`]. Both are
/// REFERENCES to declarations that already exist, read as a term reads a name.
fn parse_structure(input: &str) -> Result<Command, ParseError> {
    let assign = TermParser::find_top_level(input, ":=").ok_or_else(|| {
        ParseError::Missing(
            "`:=` and the projections, one per field, in field order — `Structure Prod := fst snd.`"
                .to_string(),
        )
    })?;
    let written = input[..assign].trim();
    let inductive = match Name::scan(written) {
        Some((name, len)) if len == written.len() => name.key(),
        _ => {
            return Err(ParseError::InvalidIdent(format!(
                "{written:?} \u{2014} `Structure` names the inductive it makes a structure"
            )))
        }
    };
    let mut projections = Vec::new();
    let mut rest = input[assign + 2..].trim();
    while !rest.is_empty() {
        let (name, len) = Name::scan(rest).ok_or_else(|| {
            ParseError::InvalidIdent(format!("{rest:?} \u{2014} a projection is a name"))
        })?;
        projections.push(name.key());
        rest = rest[len..].trim_start();
    }
    Ok(Command::Structure {
        inductive,
        projections,
    })
}

/// A declaration's HEAD: its name, and the universe telescope written after it as `.{u, v}`.
///
/// The name is read by the one name grammar (`super::declaration_name`), and whatever the name
/// grammar does not consume must be exactly one `.{…}` list of distinct-or-not universe names —
/// repetition is the kernel's to refuse, so that the vernacular refuses exactly what the kernel
/// refuses and says it the kernel's way.
fn declaration_head(raw: &str) -> Result<(String, Option<Vec<String>>), ParseError> {
    let raw = raw.trim();
    let name_len = Name::scan(raw).map_or(raw.len(), |(_, len)| len);
    let (name, rest) = raw.split_at(name_len);
    let name = declaration_name(name)?;
    if rest.is_empty() {
        return Ok((name, None));
    }
    let list = rest
        .strip_prefix(".{")
        .and_then(|r| r.strip_suffix('}'))
        .ok_or_else(|| {
            ParseError::InvalidIdent(format!(
                "{raw:?} \u{2014} after a declaration's name may come only its universe \
                 parameters, written `.{{u, v}}`"
            ))
        })?;
    Ok((name, Some(universe_names(list)?)))
}

/// `u, v, «w»` — the names of a universe telescope. Empty is allowed and means a telescope of no
/// parameters, which registers through the polymorphic door all the same.
fn universe_names(list: &str) -> Result<Vec<String>, ParseError> {
    let mut names = Vec::new();
    let mut rest = list.trim();
    if rest.is_empty() {
        return Ok(names);
    }
    loop {
        let (name, len) = super::term_parser::scan_name(rest).ok_or_else(|| {
            ParseError::InvalidIdent(format!("{rest:?} \u{2014} a universe parameter is a name"))
        })?;
        if !rest.starts_with('\u{ab}')
            && (name == "_"
                || name == "max"
                || name == "imax"
                || super::term_parser::is_sort_word(&name)
                || super::term_parser::is_reserved_word(&name))
        {
            return Err(ParseError::InvalidIdent(format!(
                "`{name}` is a word of the level grammar; a universe parameter spelled that way is \
                 written \u{ab}{name}\u{bb}"
            )));
        }
        names.push(name);
        rest = rest[len..].trim_start();
        if rest.is_empty() {
            return Ok(names);
        }
        rest = rest
            .strip_prefix(',')
            .ok_or_else(|| {
                ParseError::Missing(format!("`,` between universe parameters, before {rest:?}"))
            })?
            .trim_start();
    }
}

/// `cmd` bound by the universe telescope, if one was written.
fn with_universes(universes: Option<Vec<String>>, cmd: Command) -> Command {
    match universes {
        Some(params) => Command::Universes {
            params,
            inner: Box::new(cmd),
        },
        None => cmd,
    }
}

/// Parse: term
fn parse_check(surface: &Surface, input: &str) -> Result<Command, ParseError> {
    let term = TermParser::parse_in(surface, input)?;
    Ok(Command::Check(term))
}

/// Parse: term
fn parse_eval(surface: &Surface, input: &str) -> Result<Command, ParseError> {
    let term = TermParser::parse_in(surface, input)?;
    Ok(Command::Eval(term))
}

/// Parse: Name (params) := C1 : T1 | C2 : T2
///
/// Supports polymorphic inductives like:
/// `Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.`
///
/// ★ AND A BLOCK, WHEN THE HEADER SAYS MORE. A header that DECLARES its arity
/// (`Inductive Vec (A : Type) : Nat -> Type := …`), names universe parameters
/// (`Inductive PBox.{u} …`) or joins members with `with` is an [`Command::InductiveBlock`]: its
/// written parameters ARE its parameters, the rest of the arity is indices, and its members are
/// one mutual block — registered through the kernel's checked block door, the one the Lean
/// importer uses. Without any of those, the header is the legacy form, which lets the kernel
/// infer the parameter split, and reads exactly as it always did.
fn parse_inductive(surface: &Surface, input: &str) -> Result<Command, ParseError> {
    // ★ A `deriving` CLAUSE IS REFUSED, NOT ABSORBED. Lean's `deriving` runs a handler — an
    // `Array Name -> CommandElabM Bool` that reads the environment, emits declarations and may
    // decline — and the kernel is pure CoC by construction, so there is nowhere for one to live.
    // Refusing it is the honest answer. MIS-PARSING it was not: `Inductive Colour := red : Colour
    // deriving BEq.` used to be accepted and registered `red : ((Colour deriving) BEq)`, applying
    // the inductive to two globals that are not in the context at all.
    if find_keyword(input, "deriving").is_some() {
        return Err(ParseError::Unsupported {
            what: "a `deriving` clause".to_string(),
            why: "a deriving handler is a CommandElabM PROGRAM, and the kernel is pure CoC — \
                  there is no handler registry for one to live in. Write the declarations it \
                  would have generated; they are ordinary and they check"
                .to_string(),
        });
    }

    let pieces = split_top_level_word(input, "with");
    let mut members = Vec::with_capacity(pieces.len());
    let mut telescope: Option<Vec<String>> = None;
    let mut declared = pieces.len() > 1;
    for piece in &pieces {
        let (header, ctors_str) = match TermParser::find_top_level(piece, ":=") {
            Some(at) => (piece[..at].trim(), Some(piece[at + 2..].trim())),
            None => (piece.trim(), None),
        };
        let head = parse_inductive_header(surface, header)?;
        declared |= head.universes.is_some() || head.arity.is_some();
        if let Some(own) = head.universes {
            match &telescope {
                None if members.is_empty() => telescope = Some(own),
                Some(first) if *first == own => {}
                _ => {
                    return Err(ParseError::Unsupported {
                        what: format!("a second universe telescope on `{}`", head.name),
                        why: "the members of one inductive block share ONE universe telescope, \
                              written on the first member"
                            .to_string(),
                    })
                }
            }
        }
        let constructors = match ctors_str {
            Some(text) => parse_constructors(surface, text)?,
            None => Vec::new(),
        };
        members.push((
            head.name,
            head.params,
            head.arity,
            ctors_str.is_some(),
            constructors,
        ));
    }

    // A restated constructor telescope is block syntax: the legacy form has no field for it.
    declared |= members.iter().any(|m| m.4.iter().any(|(_, _, full)| *full));
    if !declared {
        let (name, params, _, _, constructors) = members.pop().expect("one member");
        if constructors.is_empty() {
            return Err(ParseError::Missing("constructors".to_string()));
        }
        return Ok(Command::Inductive {
            name,
            params,
            sort: Term::Sort(Universe::Type(0)),
            constructors: constructors.into_iter().map(|(n, t, _)| (n, t)).collect(),
        });
    }
    let members = members
        .into_iter()
        .map(|(name, params, arity, has_assign, constructors)| {
            if !has_assign && arity.is_none() {
                return Err(ParseError::Missing(format!(
                    "`:=` and the constructors of `{name}` \u{2014} a member with no constructors \
                     declares its arity (`{name} : Type`)"
                )));
            }
            let full = constructors.iter().any(|(_, _, f)| *f);
            if full && constructors.iter().any(|(_, _, f)| !*f) {
                return Err(ParseError::Unsupported {
                    what: format!(
                        "`{name}` restating the parameters of some constructors and not others"
                    ),
                    why: "a member's constructors are written one way: every one restates the \
                          parameter telescope, or none does"
                        .to_string(),
                });
            }
            if full && params.is_empty() {
                return Err(ParseError::Unsupported {
                    what: format!("a restated telescope on a constructor of `{name}`"),
                    why: format!("`{name}` has no parameters to restate"),
                });
            }
            Ok(super::command::InductiveMember {
                name,
                params,
                sort: arity.unwrap_or(Term::Sort(Universe::Type(0))),
                constructors: constructors.into_iter().map(|(n, t, _)| (n, t)).collect(),
                full_constructors: full,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(with_universes(
        telescope,
        Command::InductiveBlock { members },
    ))
}

/// `C1 : T1 | C2 : T2` — split at a TOP-LEVEL `|`. A `|` inside a bracket, a string or a
/// «quoted name» belongs to the constructor's type, not to the alternation.
///
/// A constructor may RESTATE its parameter telescope before the colon — `C (p : T) … : rest` —
/// and then its type is the full `(p : T) -> … -> rest`, the binders in scope in `rest`. The third
/// component says which form was written.
fn parse_constructors(
    surface: &Surface,
    input: &str,
) -> Result<Vec<(String, Term, bool)>, ParseError> {
    let mut constructors = Vec::new();
    for ctor_part in split_top_level(input, "|") {
        let ctor_part = ctor_part.trim();
        if ctor_part.is_empty() {
            continue;
        }

        // Each constructor is: Name : Type, or Name (p : T) … : Type
        let colon_pos = TermParser::find_top_level(ctor_part, ":").ok_or(ParseError::Missing(
            "constructor type annotation".to_string(),
        ))?;
        let head = ctor_part[..colon_pos].trim();
        let name_len = Name::scan(head).map_or(head.len(), |(_, len)| len);
        let ctor_name = declaration_name(&head[..name_len])?;
        let restated = head[name_len..].trim();
        let ctor_type_str = ctor_part[colon_pos + 1..].trim();

        if restated.is_empty() {
            constructors.push((
                ctor_name,
                TermParser::parse_in(surface, ctor_type_str)?,
                false,
            ));
        } else {
            if !restated.starts_with('(') {
                return Err(ParseError::Expected {
                    expected: format!(
                        "`:` or the parameter binders `(p : T)` after the constructor `{ctor_name}`"
                    ),
                    found: restated.chars().take(12).collect(),
                });
            }
            let full = format!("{restated} -> {ctor_type_str}");
            constructors.push((ctor_name, TermParser::parse_in(surface, &full)?, true));
        }
    }
    Ok(constructors)
}

/// `input` cut at every TOP-LEVEL occurrence of the WORD `word` — outside every bracket, string
/// and «quoted name», and never inside a longer identifier (`without` holds no `with`).
fn split_top_level_word<'a>(input: &'a str, word: &str) -> Vec<&'a str> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '\'';
    let mut parts = Vec::new();
    let mut start = 0;
    let mut from = 0;
    while let Some(rel) = TermParser::find_top_level(&input[from..], word) {
        let at = from + rel;
        let before = input[..at].chars().next_back();
        let after = input[at + word.len()..].chars().next();
        if before.is_none_or(|c| !is_ident(c)) && after.is_none_or(|c| !is_ident(c)) {
            parts.push(&input[start..at]);
            start = at + word.len();
        }
        from = at + word.len();
    }
    parts.push(&input[start..]);
    parts
}

/// An inductive member's header, read: its name, its universe telescope if it wrote one, its
/// parameters, and its declared arity if it wrote one.
struct InductiveHeader {
    name: String,
    universes: Option<Vec<String>>,
    params: Vec<(String, Term)>,
    arity: Option<Term>,
}

/// Parse the inductive header: name, optional `.{u, …}`, parameters, optional `: arity`.
///
/// Examples:
/// - `List` -> ("List", [])
/// - `List (A : Type)` -> ("List", [("A", Type)])
/// - `Either (A : Type) (B : Type)` -> ("Either", [("A", Type), ("B", Type)])
/// - `Vec.{u} (A : Sort u) : Nat -> Sort (max 1 u)` -> ("Vec", [u], [("A", Sort u)], arity)
fn parse_inductive_header(surface: &Surface, header: &str) -> Result<InductiveHeader, ParseError> {
    let header = header.trim();

    // The NAME is however much of the header the name grammar reads, and the rest is the parameter
    // list. Scanning for the first `(` instead put the split INSIDE a «quoted name» that contained
    // one, so `«odd(name»` was refused for a reason that named neither the name nor the parameters
    // — the same raw-search mistake the sentence split was making, one door along.
    let name_len = Name::scan(header).map_or(header.len(), |(_, len)| len);
    let name = declaration_name(&header[..name_len])?;
    let mut rest = header[name_len..].trim_start();
    let mut universes = None;
    if let Some(after) = rest.strip_prefix(".{") {
        let close = after.find('}').ok_or_else(|| {
            ParseError::Missing(format!("`}}` closing the universe parameters of `{name}`"))
        })?;
        universes = Some(universe_names(&after[..close])?);
        rest = after[close + 1..].trim_start();
    }
    let (params_str, arity) = match TermParser::find_top_level(rest, ":") {
        Some(at) => (
            rest[..at].trim(),
            Some(TermParser::parse_in(surface, rest[at + 1..].trim())?),
        ),
        None => (rest.trim(), None),
    };
    let params = if params_str.is_empty() {
        Vec::new()
    } else {
        // Parse all parameter bindings: (A : Type) (B : Type) ...
        parse_param_bindings(surface, params_str)?
    };
    Ok(InductiveHeader {
        name,
        universes,
        params,
        arity,
    })
}

/// Parse a sequence of parameter bindings: (A : Type) (B : Type)
///
/// Each binding is of the form (name : type).
fn parse_param_bindings(surface: &Surface, input: &str) -> Result<Vec<(String, Term)>, ParseError> {
    let mut params = Vec::new();
    let mut remaining = input.trim();

    while !remaining.is_empty() {
        // Skip whitespace
        remaining = remaining.trim();
        if remaining.is_empty() {
            break;
        }

        // Expect '('
        if !remaining.starts_with('(') {
            return Err(ParseError::Missing("opening '(' for parameter".to_string()));
        }

        // Find matching ')'
        let close_pos = find_matching_paren(remaining)?;
        let binding = &remaining[1..close_pos]; // Contents inside parens

        // Parse name : type — at TOP LEVEL, so `(f : (x : Nat) -> Nat)` splits at its OWN colon.
        let colon_pos = TermParser::find_top_level(binding, ":")
            .ok_or(ParseError::Missing("':' in parameter binding".to_string()))?;

        // The parameter's name is read by the NAME GRAMMAR, not taken verbatim. A binder name is
        // what the constructor types refer to, and the term door hands those references back
        // UNQUOTED — so a verbatim `«a:b»` bound a parameter under a spelling no term could
        // produce and the registration failed with `UnboundVariable("a:b")`, one layer away from
        // the binding that caused it.
        let written = binding[..colon_pos].trim();
        let param_name = match Name::scan(written) {
            Some((name, len)) if len == written.len() => name.key(),
            _ => {
                return Err(ParseError::Missing(format!(
                    "a parameter name in {written:?}"
                )))
            }
        };
        let param_type_str = binding[colon_pos + 1..].trim();

        let param_type = TermParser::parse_in(surface, param_type_str)?;
        params.push((param_name, param_type));

        // Move past this binding
        remaining = remaining[close_pos + 1..].trim();
    }

    Ok(params)
}

/// The BYTE offset of the `)` that matches the opening `(` at position 0.
///
/// ★ A CHAR INDEX IS NOT A BYTE INDEX. This counted characters and the caller sliced with the
/// result, so one multi-byte character anywhere inside a parameter list moved every later
/// boundary: `Inductive Box («a:b» : Type)` cut the binding short of its own `)` and the next
/// iteration reported a missing `(` for a parameter nobody wrote. Every non-ASCII spelling this
/// vernacular deliberately supports — a «quoted name», a declared notation, a Unicode identifier —
/// is in that class, and a slice that lands mid-character panics outright rather than misreads.
fn find_matching_paren(input: &str) -> Result<usize, ParseError> {
    let mut depth = 0;
    let mut in_string = false;
    let mut in_name = false;
    let mut escaped = false;
    for (i, c) in input.char_indices() {
        // A parenthesis inside a string literal or a «quoted name» is TEXT, not structure — the
        // same rule `TermParser::find_top_level` applies, so a parameter's type may hold either.
        if escaped {
            escaped = false;
            continue;
        }
        if in_string {
            match c {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        if in_name {
            in_name = c != '\u{bb}';
            continue;
        }
        match c {
            '"' => in_string = true,
            '\u{ab}' => in_name = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => {}
        }
    }
    Err(ParseError::Missing("closing ')' for parameter".to_string()))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/command_parser/tests.rs"]
mod tests;
