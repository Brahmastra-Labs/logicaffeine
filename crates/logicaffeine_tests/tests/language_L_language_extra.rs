//! Lane L_language — the PARSE-level lock for the math layer (`parser/math.rs`).
//!
//! No prover anywhere in this file: every assertion is about what the English theorem
//! surface can SAY. That is deliberate — the corpus's claim was about expressibility
//! ("`∀ n : Nat, n + 0 = n` is expressible on NO vernacular surface"), and expressibility is
//! a property of the grammar, not of the search. The proving half is locked cross-crate in
//! `logicaffeine_tests/tests/L_language_extra.rs`.

use logicaffeine_language::compile;

/// Every shape the term layer accepts, and exactly what it builds.
#[test]
fn the_math_layer_builds_the_term_and_binder_shapes_it_claims() {
    for (english, expected) in [
        // The headline: a binder over a function term.
        ("For all n, n plus 0 equals n.", "∀n(Add(n, 0) = n)"),
        // Two binders, nested and joint, are the same proposition.
        (
            "For all a b, a plus b equals b plus a.",
            "∀a(∀b(Add(a, b) = Add(b, a)))",
        ),
        (
            "For all a, For all b, a plus b equals b plus a.",
            "∀a(∀b(Add(a, b) = Add(b, a)))",
        ),
        // Precedence: `times` binds tighter than `plus`.
        (
            "For all a b, a plus b times 2 equals a.",
            "∀a(∀b(Add(a, Mul(b, 2)) = a))",
        ),
        // Associativity: `minus` is LEFT-associative.
        (
            "For all a b c, a minus b minus c equals a.",
            "∀a(∀b(∀c(Sub(Sub(a, b), c) = a)))",
        ),
        // Subtraction from a literal zero — the ring's negation, spelled the way the term
        // layer represents a unary minus, so no `neg` symbol ever reaches the oracle as an
        // uninterpreted function.
        (
            "For all n, 0 minus n plus n equals 0.",
            "∀n(Add(Sub(0, n), n) = 0)",
        ),
        // English number words are numerals; `zero` is included because the lexicon's own
        // table starts at `one`.
        ("For all n, n plus zero equals n.", "∀n(Add(n, 0) = n)"),
        ("For all n, n times one equals n.", "∀n(Mul(n, 1) = n)"),
        // `divided by` and the symbolic operators.
        ("For all a, a divided by 2 equals a.", "∀a(Div(a, 2) = a)"),
        (
            "For all a b, a + b equals a - b.",
            "∀a(∀b(Add(a, b) = Sub(a, b)))",
        ),
        ("For all a b, a * b equals a.", "∀a(∀b(Mul(a, b) = a))"),
        // Case is the constant/variable distinction — the FORMAL surface's own rule.
        (
            "For all a b, a plus b equals b plus C.",
            "∀a(∀b(Add(a, b) = Add(b, C)))",
        ),
        // `is equal to` is the same relation as `equals`.
        ("For all n, n plus 0 is equal to n.", "∀n(Add(n, 0) = n)"),
        // Connectives over equations.
        (
            "For all a b, if a equals b then b equals a.",
            "∀a(∀b((a = b → b = a)))",
        ),
        // The EXISTENTIAL half of the binder, alone and mixed with the universal.
        ("For some n, n plus 1 equals 2.", "∃n(Add(n, 1) = 2)"),
        (
            "For all a, For some b, a plus b equals a.",
            "∀a(∃b(Add(a, b) = a))",
        ),
    ] {
        let got = compile(english).unwrap_or_else(|e| panic!("`{english}` must compile: {e:?}"));
        assert_eq!(got, expected, "`{english}` built the wrong proposition");
    }
}

/// A leading `-` is the ring's negation and never an uninterpreted `neg`. Which of the two
/// equivalent spellings the LEXER hands over (`Minus` `1`, or one signed numeral) is its
/// business; what this pins is that the result is arithmetic either way.
#[test]
fn a_leading_minus_is_arithmetic_not_an_uninterpreted_symbol() {
    let negated = compile("For all n, -1 times n equals n.")
        .expect("a leading minus in term position compiles");
    assert!(
        negated.contains("Mul(") && !negated.to_lowercase().contains("neg("),
        "`-1 times n` must be a ring product, never an uninterpreted `neg` the oracle cannot \
         use: {negated}"
    );
}

/// The two residues, refused BY NAME. A limit a user can read is a work queue; a limit that
/// reports the wrong construct is a bug report nobody can act on.
#[test]
fn the_math_layer_refuses_what_it_cannot_represent_and_says_which_construct_broke() {
    // A SORT is not representable: neither `LogicExpr::Quantifier` nor `ProofExpr::ForAll`
    // carries one, so `: Nat` is refused rather than silently dropped.
    let sorted = format!(
        "{:?}",
        compile("For all n : Nat, n plus 0 equals n.").unwrap_err()
    );
    assert!(
        sorted.contains("SORT"),
        "`For all n : Nat,` must be refused by naming the SORT, not by reporting a stray \
         colon: {sorted}"
    );

    // An ORDER between arithmetic terms has no production here yet.
    let order = format!(
        "{:?}",
        compile("For all n, n plus 1 is greater than n.").unwrap_err()
    );
    assert!(
        order.contains("equals"),
        "an order between TERMS must be refused by naming the relation this surface DOES \
         have: {order}"
    );
}

/// The production is strictly ADDITIVE. It rewinds unless it consumes the whole sentence, so
/// the English nearest its vocabulary is untouched — including the comparatives the refusal
/// above looks for.
#[test]
fn the_math_layer_never_steals_an_english_sentence() {
    for (english, expected) in [
        ("Every man is mortal.", "∀x((Man(x) → Mortal(x)))"),
        ("All men are mortal.", "∀x((Man(x) → Mortal(x)))"),
        ("Bob is happy or Bob is sad.", "(Happy(Bob) ∨ Sad(Bob))"),
        ("Simon is taller than Mary.", "Taller(Simon, Mary)"),
    ] {
        let got = compile(english).unwrap_or_else(|e| panic!("`{english}` must compile: {e:?}"));
        assert_eq!(
            got, expected,
            "`{english}` must be unaffected by the math layer"
        );
    }

    // A comparative with NO arithmetic never reaches the math layer, so it can never pick up
    // the math layer's refusal.
    for english in [
        "For all cats, the score is greater than five.",
        "The score is greater than five.",
    ] {
        if let Err(e) = compile(english) {
            let rendered = format!("{e:?}");
            assert!(
                !rendered.contains("relates TERMS"),
                "`{english}` carries no arithmetic, so the math layer must never have looked \
                 at it: {rendered}"
            );
        }
    }
}

/// Depth and width past anything a human writes — and a pathological input that must be a
/// PARSE ERROR rather than a stack overflow. The term layer recurses through parentheses,
/// unary minus and application arguments, so it takes the clause grammar's depth bound.
#[test]
fn the_math_grammar_is_bounded_in_depth_and_unbounded_in_width() {
    let eight = compile("For all n, ((((((((n)))))))) plus 0 equals n.")
        .expect("eight levels of parentheses parse");
    assert_eq!(
        eight, "∀n(Add(n, 0) = n)",
        "parentheses are grouping only — they must leave no trace in the term"
    );

    let binders: Vec<String> = ('a'..='t').map(|c| c.to_string()).collect();
    let wide = compile(&format!(
        "For all {}, {} equals a.",
        binders.join(" "),
        binders.join(" plus ")
    ))
    .expect("twenty binders and a twenty-term sum parse");
    assert_eq!(
        wide.matches('∀').count(),
        20,
        "all twenty variables must be BOUND: {wide}"
    );

    // Three hundred opening parens: the depth bound makes this a refusal. The math
    // production rewinds, so the diagnostic comes from the English parser — what matters is
    // that it is an Err and not a crashed process.
    let pathological = format!("For all n, {} n plus 0 equals n.", "(".repeat(300));
    assert!(
        compile(&pathological).is_err(),
        "an unbounded parenthesis nest must be a parse error, never a stack overflow"
    );
}

// ═════════════ WAVE 2 — SCOPE, THEN POSITION, THEN THE WORD TABLE ═════════════

/// A NEW defect, not in the corpus, and a CAPTURE bug of exactly the family this campaign
/// exists to hunt: the term layer consulted the NUMERAL TABLE — the lexer's `Cardinal` token
/// for `one`…`ten`, and this module's own `zero` row — before it consulted the enclosing
/// binder.
///
/// So `For all one, one times one equals one.` bound `one` and then read every occurrence of
/// it in the body as the literal `1`. The binder bound nothing. What reached the kernel was
/// `∀one(Mul(1, 1) = 1)` — true, KERNEL-CERTIFIED, and not the sentence, because the sentence
/// says `∀x(x × x = x)` and that is FALSE. The certificate was honest about the term it was
/// handed; the term was not honest about the sentence, and nothing downstream could tell,
/// because everything downstream reasons about the term.
///
/// The rule is Lean's, and it holds for every name and not only a number word: a binder
/// SHADOWS whatever its name meant outside it. `∀ one : Nat, one * one = one` binds in Lean
/// too.
#[test]
fn a_math_binder_shadows_the_numeral_table_inside_its_own_body() {
    for (english, expected) in [
        (
            "For all one, one times one equals one.",
            "∀one(Mul(one, one) = one)",
        ),
        (
            "For all zero, zero plus zero equals zero.",
            "∀zero(Add(zero, zero) = zero)",
        ),
        (
            "For all two, two plus two equals two.",
            "∀two(Add(two, two) = two)",
        ),
        (
            "For some ten, ten plus 1 equals ten.",
            "∃ten(Add(ten, 1) = ten)",
        ),
    ] {
        let got = compile(english).unwrap_or_else(|e| panic!("`{english}` must compile: {e:?}"));
        assert_eq!(
            got, expected,
            "`{english}` built the wrong proposition — a binder must OWN its name inside its \
             own body"
        );
    }
}

/// The shadow reaches exactly as far as the binder that cast it, and no further. One rung
/// either side: a sibling number word the prefix did NOT bind is still the numeral, and the
/// same word outside every binder is still the numeral — so the sentence this whole module
/// exists for, `n plus zero equals n`, is untouched.
#[test]
fn the_shadow_stops_at_the_binder_that_cast_it() {
    for (english, expected) in [
        (
            "For all one, one plus two equals one.",
            "∀one(Add(one, 2) = one)",
        ),
        ("For all n, n plus zero equals n.", "∀n(Add(n, 0) = n)"),
        ("For all n, n times one equals n.", "∀n(Mul(n, 1) = n)"),
        (
            "For all n, n plus two minus two equals n.",
            "∀n(Sub(Add(n, 2), 2) = n)",
        ),
    ] {
        let got = compile(english).unwrap_or_else(|e| panic!("`{english}` must compile: {e:?}"));
        assert_eq!(got, expected, "`{english}` built the wrong proposition");
    }
}

/// A NEW defect, not in the corpus, and the one this lane found that is a SOUNDNESS hole
/// rather than a capability gap.
///
/// A `Number` token that did not fit an `i64` was widened to an `f64` and travelled on as
/// `NumberKind::Real`, which `proof_convert::term_to_proof_term` renders back through
/// `f64::to_string`. That RENDERING is the term the kernel sees, and it is not the number
/// that was written: `99999999999999999999` and `100000000000000000000` are the same `f64`,
/// and both print `100000000000000000000`. So
///
/// ```text
/// For all n, n plus 99999999999999999999 equals n plus 100000000000000000000.
/// ```
///
/// reached the kernel as `t = t` and came back KERNEL-CERTIFIED — a closed proof of a false
/// arithmetic statement on the flagship surface. Two distinct numbers had become one term,
/// and every check downstream agreed, because by the time anything checked there was only one
/// term left to check. The same collision swallows `0.1` and `0.10000000000000000001`.
///
/// The fix is not a wider float. A numeral the ring can VALUE is an `i64`; anything else keeps
/// its SOURCE SPELLING and travels as an opaque constant, because the lexeme is the identity.
/// Distinct spellings stay distinct terms. It costs completeness — `2.50` and `2.5` are two
/// constants now — and buys back soundness, which is the right side of that trade.
#[test]
fn a_numeral_the_ring_cannot_value_keeps_its_own_spelling() {
    let collide =
        compile("For all n, n plus 99999999999999999999 equals n plus 100000000000000000000.")
            .expect("an oversized numeral still parses — it is a TERM, it is just not a value");
    assert_eq!(
        collide, "∀n(Add(n, 99999999999999999999) = Add(n, 100000000000000000000))",
        "two numerals written differently must reach the proposition as two DIFFERENT terms — \
         the `f64` round trip printed both of them as `100000000000000000000`"
    );

    let decimals = compile("For all n, n plus 0.1 equals n plus 0.10000000000000000001.")
        .expect("a decimal numeral still parses");
    assert_eq!(
        decimals, "∀n(Add(n, 0.1) = Add(n, 0.10000000000000000001))",
        "a decimal too fine for an `f64` must keep its own spelling rather than collapse onto \
         its neighbour"
    );

    // …and the numerals the ring CAN value are still values, so the arithmetic is untouched.
    assert_eq!(
        compile("For all n, n plus 0 equals n.").expect("compiles"),
        "∀n(Add(n, 0) = n)",
        "an `i64` numeral is still the integer it denotes"
    );
    assert_eq!(
        compile("For all n, n plus 2.5 equals n.").expect("compiles"),
        "∀n(Add(n, 2.5) = n)",
        "a decimal is still a TERM — it is refused a VALUE, not a place in the grammar"
    );
}

/// The case convention and its ONE exception, pinned so a divergence between two vernacular
/// surfaces is a recorded fact rather than a surprise. `formula::ident_to_term` reads an
/// uppercase-leading name as an opaque CONSTANT and this surface agrees — except for an
/// English NUMBER WORD, which is a numeral whatever its case, because English capitalises
/// sentence-initially and `Zero plus n equals n.` is an ordinary sentence. The English
/// surface is deliberately the more generous of the two here.
#[test]
fn an_english_number_word_is_a_numeral_whatever_its_case() {
    assert_eq!(
        compile("For all n, n plus Zero equals n.").expect("compiles"),
        "∀n(Add(n, 0) = n)",
        "`Zero` is the numeral on the ENGLISH surface — `formula::parse_term` reads it as an \
         opaque constant, and that difference is deliberate"
    );
    assert_eq!(
        compile("For all n, n times One equals n.").expect("compiles"),
        "∀n(Mul(n, 1) = n)",
        "…and so is `One`, which the LEXER already hands over as a `Cardinal`"
    );
    assert_eq!(
        compile("For all a b, a plus b equals b plus C.").expect("compiles"),
        "∀a(∀b(Add(a, b) = Add(b, C)))",
        "an uppercase name that is NOT a numeral is still an opaque constant"
    );
}

/// A sentence that is unambiguously arithmetic does not also have to CONTAIN arithmetic.
///
/// The pre-scan asked every binder-less sentence for an infix operator before it would read
/// it as a term equation, which protects `a equals b` — as much English as algebra — but
/// also refused `2 equals 2.` and `f(3) equals 3.`, two sentences with no English reading at
/// all. A surface whose headline is `∀ n, n + 0 = n` could not state `2 = 2`, and the
/// diagnostic named a stray `equals` rather than the construct.
///
/// The gate is now three-valued: a numeral, a parenthesis, a leading `-` or a function
/// APPLICATION opens a sentence English does not write, so the relation alone is enough; a
/// bare NAME still has to carry arithmetic.
#[test]
fn a_sentence_english_does_not_write_needs_no_operator_to_be_arithmetic() {
    for (english, expected) in [
        ("2 equals 2.", "2 = 2"),
        ("f(3) equals 3.", "F(3) = 3"),
        ("2 plus 2 equals 4.", "Add(2, 2) = 4"),
    ] {
        let got = compile(english).unwrap_or_else(|e| panic!("`{english}` must compile: {e:?}"));
        assert_eq!(got, expected, "`{english}` built the wrong proposition");
    }

    // THE NEGATIVE, and it is the whole reason the gate is three-valued rather than off: a
    // bare NAME still needs arithmetic, because `a equals b` is an English sentence and the
    // math layer must not claim it.
    assert!(
        compile("a equals a.").is_err(),
        "a bare name followed by `equals` is English until it carries arithmetic: {:?}",
        compile("a equals a.")
    );
    // …and the English nearest the relaxed opening is still English.
    for english in [
        "Every man is mortal.",
        "All men are mortal.",
        "Simon is taller than Mary.",
    ] {
        assert!(
            compile(english).is_ok(),
            "`{english}` must be unaffected by the relaxed opening: {:?}",
            compile(english)
        );
    }
}

/// The THIRD instance of the same class, and the worst-behaved of the three: the
/// MEASURE-PHRASE reader defaulted a numeral it could not parse to `0`.
///
/// `parse_number_kind` (parser/pragmatics.rs) was `s.parse::<i64>().unwrap_or(0)` for an
/// all-digit numeral and `s.parse::<f64>().unwrap_or(0.0)` for a decimal, so
/// `The price is 99999999999999999999 dollars.` reached the proposition as
/// `Price = 0 dollars`. Not an approximation — **zero**. Zero is the most dangerous default a
/// measure can take: it is a legal value, it compares, it is the additive identity, and it
/// makes a premise about an enormous price agree with a goal about no price at all.
///
/// A number this surface cannot VALUE stays SYMBOLIC and keeps its lexeme. That is what the
/// comparative-measure reader in `parser/verb.rs` already did for the same numeral, so the
/// two readers of the same token disagreed — and the one that reached a `Given:` line was the
/// one that renumbered it.
#[test]
fn a_measure_phrase_never_renumbers_a_numeral_it_cannot_value() {
    let huge = compile("The price is 99999999999999999999 dollars.").expect("compiles");
    assert!(
        huge.contains("99999999999999999999 dollars"),
        "a price too large for an `i64` must keep its own spelling: {huge}"
    );
    assert!(
        !huge.contains("0 dollars"),
        "…and it must never be readable as no price at all: {huge}"
    );

    // The same numeral through the OTHER measure reader (`verb.rs`'s comparative offset),
    // which is the one that was already right. The two must now agree.
    let wingspan = compile("Bob has a wingspan of 99999999999999999999 points.").expect("compiles");
    assert!(
        wingspan.contains("99999999999999999999"),
        "both measure readers must hold the same numeral the same way: {wingspan}"
    );

    // THE BOUNDARY, recorded rather than assumed: a NEGATIVE measure has no spelling on this
    // surface at all. `-5` reaches the measure reader as `Minus` then `Number`, and the
    // reader requires a single `Number` token, so the refusal names a missing content word.
    // That is a residue of the grammar, not of the numeral rule, and it is pinned here so
    // the numeral rule is never blamed for it.
    assert!(
        compile("The price is -5 dollars.").is_err(),
        "a negative measure is not writable on this surface: {:?}",
        compile("The price is -5 dollars.")
    );

    // The values it CAN hold are untouched, on both arms.
    for (english, expected) in [
        ("The price is 5 dollars.", "5 dollars"),
        ("The weight is 2.5 kilograms.", "2.5 kilograms"),
    ] {
        let got = compile(english).unwrap_or_else(|e| panic!("`{english}` must compile: {e:?}"));
        assert!(
            got.contains(expected),
            "`{english}` must still hold `{expected}`: {got}"
        );
    }
}

/// ONE numeral rule for the whole English surface, not three.
///
/// Three readers turned a `Number` token into a `NumberKind` and they disagreed:
/// `pragmatics::parse_measure_phrase` and the COMPARISON TARGET both fell back to `0`, while
/// `verb.rs`'s comparative-measure offset fell back to the lexeme. The comparison target is
/// the worst place for it, because the renumbered value feeds ORDER reasoning directly —
/// `taller than 99999999999999999999` became `taller than 0`, which is true of almost
/// everything. All three now share `parse_number_kind`, so the surface cannot hold two
/// opinions about one token again.
#[test]
fn every_numeral_reader_on_this_surface_holds_a_numeral_the_same_way() {
    let compared = compile("Bob is taller than 99999999999999999999.").expect("compiles");
    assert!(
        compared.contains("99999999999999999999"),
        "a comparison target too large for an `i64` must keep its own spelling, not become \
         zero — `taller than 0` is true of almost everything: {compared}"
    );

    let measured = compile("The price is 99999999999999999999 dollars.").expect("compiles");
    let offset = compile("Bob has a wingspan of 99999999999999999999 points.").expect("compiles");
    for (what, rendered) in [
        ("comparison target", &compared),
        ("measure phrase", &measured),
        ("measure offset", &offset),
    ] {
        assert!(
            rendered.contains("99999999999999999999"),
            "the {what} reader must hold the numeral the same way as the other two: {rendered}"
        );
        assert!(
            !rendered.contains(" 0 ") && !rendered.ends_with(" 0"),
            "…and none of them may renumber it to zero: {rendered}"
        );
    }

    // The values every reader CAN hold are unchanged.
    assert!(compile("Bob is taller than 5.")
        .expect("compiles")
        .contains('5'));
    assert!(compile("The price is 5 dollars.")
        .expect("compiles")
        .contains("5 dollars"));
}

// ============================================================================
// THE PREMISE NAMESPACE — `Given (h):` and the positional `hp<n>`
// ============================================================================
//
// `ProofState::start_with_names` names premise `i` by its `Given (label):` when it has one and
// by the positional `hp{i}` otherwise, and a script's citation is resolved with
// `hyps.iter().find(|h| h.name == name)` — the FIRST hypothesis carrying that name. So two
// premises under one name is not a shadowing rule, it is a silent choice, and the block gives
// the reader no way to see which one was taken.
//
// Lean cannot reach that state, twice over: `LocalContext.findFromUserName?` resolves with
// `findSomeRev?`, so the LAST binding of a user name wins
// (`Lean/LocalContext.lean:273-277`), and `getUnusedNameAux` guarantees a GENERATED name is
// never one already in use (`Lean/LocalContext.lean:282-286`). Our positional names are
// citable rather than inaccessible, so that second guarantee has to be stated on this surface
// instead of being a property of the elaborator.

/// The theorem door. `compile` is the SENTENCE door and does not parse a `## Theorem` block
/// at all, so a premise-namespace assertion written against it would pass for the wrong
/// reason.
fn theorem(src: &str) -> Result<String, String> {
    logicaffeine_language::compile_theorem(src).map_err(|e| format!("{e:?}"))
}

/// **`hp<n>` is the system's namespace, and a label may not enter it at another index.**
///
/// `Given (hp1):` on the FIRST premise used to be accepted, and then `hp1` named premise 0 to
/// every script while the positional convention this surface documents (and T12-004 pins with
/// a differential) says `hp1` is premise 1. Nothing anywhere reported the capture.
#[test]
fn a_premise_label_never_captures_another_premises_positional_name() {
    let captured = "## Theorem: Captured\n\
                    Given (hp1): Bob is happy.\n\
                    Given: Bob is sad.\n\
                    Prove: Bob is sad.\n\
                    Proof: exact hp1.\n";
    let err = format!("{:?}", theorem(captured).unwrap_err());
    assert!(
        err.contains("hp1") && err.contains("hp0"),
        "a label that captures another premise's positional name must be refused by naming \
         BOTH the captured name and the label this premise may use: {err}"
    );

    // THE BOUNDARY, one rung either side. A label at its OWN index is a no-op relabel and
    // stays legal; the same label one index over is the capture.
    let own_index = "## Theorem: OwnIndex\n\
                     Given (hp0): Bob is happy.\n\
                     Given: Bob is sad.\n\
                     Prove: Bob is happy.\n\
                     Proof: exact hp0.\n";
    assert!(
        theorem(own_index).is_ok(),
        "`Given (hp0):` on premise 0 names what it already named: {:?}",
        theorem(own_index)
    );
    let later_index = "## Theorem: LaterIndex\n\
                       Given: Bob is happy.\n\
                       Given (hp1): Bob is sad.\n\
                       Prove: Bob is sad.\n\
                       Proof: exact hp1.\n";
    assert!(
        theorem(later_index).is_ok(),
        "`Given (hp1):` on premise 1 is its own positional name: {:?}",
        theorem(later_index)
    );

    // A padded spelling is NOT the positional name — `hp01` reads as premise 1 and is not it.
    let padded = "## Theorem: Padded\n\
                  Given: Bob is happy.\n\
                  Given (hp01): Bob is sad.\n\
                  Prove: Bob is sad.\n\
                  Proof: exact hp01.\n";
    assert!(
        theorem(padded).is_err(),
        "`hp01` is not the positional name of any premise and must not pass for one: {:?}",
        theorem(padded)
    );

    // …and a name that merely BEGINS with `hp` is ordinary English and untouched.
    for ordinary in ["hp", "hpx", "hypothesis", "h2"] {
        let src = format!(
            "## Theorem: Ordinary\n\
             Given ({ordinary}): Bob is happy.\n\
             Given: Bob is sad.\n\
             Prove: Bob is happy.\n\
             Proof: exact {ordinary}.\n"
        );
        assert!(
            theorem(&src).is_ok(),
            "`{ordinary}` is outside the positional namespace and must stay a legal label: {:?}",
            theorem(&src)
        );
    }
}

/// **Two premises can never carry one name.**
///
/// The old behaviour is the reason this is a defect rather than a style rule: with two
/// premises labelled `h`, `exact h` PROVED the first one's proposition and reported nothing
/// at all about the second. A reader of the block cannot tell which `h` a script meant, and
/// this surface — unlike Lean, which renders a shadowed hypothesis as `h✝` — has no way to
/// show them.
#[test]
fn two_premises_can_never_carry_the_same_label() {
    let duplicated = "## Theorem: Duplicated\n\
                      Given (h): Bob is happy.\n\
                      Given (h): Bob is sad.\n\
                      Prove: Bob is happy.\n\
                      Proof: exact h.\n";
    let err = format!("{:?}", theorem(duplicated).unwrap_err());
    assert!(
        err.contains('h') && err.contains('0') && err.contains('1'),
        "a duplicated premise label must be refused by naming the label and BOTH positions \
         that claim it: {err}"
    );

    // THE NEGATIVE. The refusal is about the NAME, not about the premises: the same two
    // premises under two labels prove, and each label cites its own.
    let distinct = "## Theorem: Distinct\n\
                    Given (h): Bob is happy.\n\
                    Given (k): Bob is sad.\n\
                    Prove: Bob is sad.\n\
                    Proof: exact k.\n";
    assert!(
        theorem(distinct).is_ok(),
        "two distinct labels over the same two premises must still prove: {:?}",
        theorem(distinct)
    );
    let first = distinct
        .replace("Prove: Bob is sad.", "Prove: Bob is happy.")
        .replace("exact k.", "exact h.");
    assert!(
        theorem(&first).is_ok(),
        "…and the other label must cite the other premise: {:?}",
        theorem(&first)
    );
}

/// **The class, not the instance: a DIFFERENTIAL against the namespace the tactic layer
/// actually builds.**
///
/// The independent model is six lines long and is not the parser's code. Premise `i` is named
/// by its label when it has one and by `hp{i}` otherwise (`ProofState::start_with_names`), and
/// a block is legal exactly when (a) that list has no repeat — no citation is ambiguous — and
/// (b) no label entered the `hp<n>` namespace at an index that is not its own. The grid
/// deliberately contains the adversarial names, the positional spellings themselves, because
/// those are the ones a hand-written exception list forgets.
#[test]
fn the_premise_namespace_is_refused_exactly_when_a_citation_would_be_ambiguous() {
    let labels = [
        None,
        Some("h"),
        Some("k"),
        Some("hp0"),
        Some("hp1"),
        Some("hp2"),
    ];
    let mut disagreements: Vec<String> = Vec::new();

    for a in labels {
        for b in labels {
            for c in labels {
                let chosen = [a, b, c];
                // THE MODEL: what `start_with_names` will call each premise.
                let effective: Vec<String> = chosen
                    .iter()
                    .enumerate()
                    .map(|(i, l)| l.map(str::to_string).unwrap_or_else(|| format!("hp{i}")))
                    .collect();
                let ambiguous =
                    (0..effective.len()).any(|i| effective[..i].contains(&effective[i]));
                let positional = |l: &str| {
                    l.len() > 2 && l.starts_with("hp") && l[2..].bytes().all(|b| b.is_ascii_digit())
                };
                let captured = chosen
                    .iter()
                    .enumerate()
                    .any(|(i, l)| l.is_some_and(|l| positional(l) && l != format!("hp{i}")));
                let must_refuse = ambiguous || captured;

                let given = |i: usize, text: &str| match chosen[i] {
                    Some(l) => format!("Given ({l}): {text}\n"),
                    None => format!("Given: {text}\n"),
                };
                let src = format!(
                    "## Theorem: Grid\n{}{}{}Prove: Bob is happy.\nProof: exact {}.\n",
                    given(0, "Bob is happy."),
                    given(1, "Bob is sad."),
                    given(2, "Bob is tall."),
                    effective[0],
                );
                let accepted = theorem(&src).is_ok();
                if accepted == must_refuse {
                    disagreements.push(format!(
                        "{chosen:?} → effective {effective:?}, ambiguous={ambiguous}, \
                         captured={captured}, accepted={accepted}"
                    ));
                }
            }
        }
    }

    assert!(
        disagreements.is_empty(),
        "the parser's premise-namespace guard must agree with the namespace \
         `start_with_names` builds, on every shape — accept exactly the unambiguous ones: \
         {disagreements:#?}"
    );
}

/// **A premise written after the goal is not a premise, and the block used to say nothing.**
///
/// `parse_theorem_block` collects `Given:` lines up to the `Prove:` line and stops. A `Given:`
/// below the goal is therefore left for the PROGRAM parser as an ordinary sentence: the
/// theorem is built with one fewer premise, fails to prove, and reports `Premises: 0` — which
/// sends the author to the prover for a defect that is in their layout. The premise did not
/// fail to help; it was never there.
///
/// Refusing is the loud answer, and it names the fix.
#[test]
fn a_premise_below_the_goal_is_refused_rather_than_dropped() {
    let below = "## Theorem: Below\n\
                 Prove: Bob is happy.\n\
                 Given: Bob is happy.\n\
                 Proof: Auto.\n";
    let err = theorem(below).expect_err("a premise below the goal must not be silently dropped");
    assert!(
        err.contains("Given") && err.contains("Prove"),
        "the refusal must name both lines, so the author knows it is the ORDER that is \
         wrong: {err}"
    );

    // THE POSITIVE. The same three lines in the order the surface defines prove.
    let above = "## Theorem: Above\n\
                 Given: Bob is happy.\n\
                 Prove: Bob is happy.\n\
                 Proof: Auto.\n";
    assert!(theorem(above).is_ok(), "{:?}", theorem(above));

    // …and a NAMED premise below the goal is the same mistake and gets the same answer.
    let named_below = "## Theorem: NamedBelow\n\
                       Prove: Bob is happy.\n\
                       Given (h): Bob is happy.\n\
                       Proof: exact h.\n";
    assert!(
        theorem(named_below).is_err(),
        "a `Given (h):` below the goal is dropped just as silently: {:?}",
        theorem(named_below)
    );

    // THE NEGATIVE, and the reason the guard tests for a COLON rather than for the word.
    // `Given` is ordinary English, and a sentence that merely opens with it is not a premise
    // line: the guard must not reach into the prose after a theorem block.
    let prose_after = "## Theorem: ProseAfter\n\
                       Given: Bob is happy.\n\
                       Prove: Bob is happy.\n\
                       Proof: Auto.\n";
    assert!(theorem(prose_after).is_ok(), "{:?}", theorem(prose_after));
    let no_proof_line = "## Theorem: NoProof\n\
                         Given: Bob is happy.\n\
                         Prove: Bob is happy.\n";
    assert!(
        theorem(no_proof_line).is_ok(),
        "a block with no `Proof:` line still defaults to Auto: {:?}",
        theorem(no_proof_line)
    );
}
