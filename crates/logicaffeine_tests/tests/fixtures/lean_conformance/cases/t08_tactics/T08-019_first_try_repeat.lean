/-!
@case      T08-019
@tier      8 · Tactics
@title     `first` / `try` / `repeat` — the three combinators are LOWERCASE-ONLY keywords and
           `first` uses brackets, not Lean's leading bar
@lean      first | t1 | t2, try t, repeat t, repeat' , nested combinators
@expect    translate
@target    LOGOS script atoms `first [ s1 | s2 | … ]`, `try <atom>`, `repeat <atom>`
           (proof/src/tactic_script.rs:221-239) → `combinators::first`
           (proof/src/tactic.rs:1076-1090), `combinators::try_` (:1091-1099),
           `combinators::repeat` (:1104-1119)
@evidence  proof/src/tactic_script.rs:221 (`Some(Tok::Word(w)) if w == "first"` — an EXACT,
           case-SENSITIVE byte comparison, unlike `canonical` which lowercases at :309);
           proof/src/tactic_script.rs:232, :236 (same for `try` and `repeat`);
           proof/src/tactic_script.rs:222-230 (`first` requires `[`, then `script ('|' script)*`,
           then `]` — brackets and bars, where Lean writes `first | t1 | t2` with no brackets);
           proof/src/tactic_script.rs:233, :237 (`try` and `repeat` each take a single ATOM, not
           a sequence — `try a ; b` is `(try a) ; b`, and a multi-step body needs parentheses,
           which `parse_atom` accepts at :215-220);
           proof/src/tactic.rs:1076-1090 (`first` speculates on `st.clone()` per alternative and
           commits the first that succeeds — genuine backtracking);
           proof/src/tactic.rs:1091-1099 (`try_` — clone, attempt, commit on success, ALWAYS
           `Ok`); proof/src/tactic.rs:1104-1119 (`repeat` — bounded at 100_000 iterations, stops
           early when `st.open.is_empty()`, ALWAYS `Ok`);
           proof/src/tactic_script.rs:169-173, :298-303 (`skip_filler` runs before every atom
           and discards `by`, `the`, `on`, `that`, `now`, `with`, `we`, `it`, `a`, `an`, `of`,
           `to`, `and`)
@guard     THE CASE-SENSITIVITY TRAP. Every PRIMITIVE tactic name is matched case-insensitively
           (`canonical` lowercases at tactic_script.rs:309), so `Assume h` and `assume h` are the
           same atom. The three COMBINATORS are not: `parse_atom` compares the raw word
           (tactic_script.rs:221, :232, :236). A translator that capitalises the first word of a
           sentence — the obvious thing to do when emitting an English-esque `Proof:` line —
           turns `try simp` into `Try simp`, which falls through to `parse_primitive`,
           `canonical("Try")` returns `None`, and the whole script dies with "unknown tactic
           `Try`". The failure is at parse time, so nothing runs.
@guard     Catches emitting Lean's `first` syntax. Lean writes `first | t1 | t2`; LOGOS requires
           `first [ t1 | t2 ]` (tactic_script.rs:222-229). Emitting the Lean form gives
           `expected LBracket, found Bar` — or worse, if the leading bar is dropped, `first`
           consumes `t1` as the bracket it wanted and reports a mismatch about a token the
           user never wrote.
@guard     Catches the ATOM/SEQUENCE distinction. `try` and `repeat` each take one atom
           (tactic_script.rs:233, :237). Lean's `try (t1; t2)` and LOGOS's `try (t1 ; t2)` agree
           only because both need the parentheses; but a translator that emits `try t1 ; t2`
           from Lean's `try t1 <;> t2` has silently moved `t2` outside the `try`, so a failure
           in `t2` now fails the whole script instead of being swallowed.
@guard     Catches `repeat`'s bound and its early exit. It iterates at most 100_000 times and
           breaks as soon as `st.open.is_empty()` (tactic.rs:1106-1116). It ALWAYS returns `Ok`,
           like Lean's `repeat`. But a non-progressing tactic that nevertheless SUCCEEDS —
           `try assumption`, say — will run the full 100_000 iterations, cloning the entire
           `ProofState` each time (tactic.rs:1107). `repeat (try t)` is therefore a performance
           trap a translator can easily emit while "being safe".
@guard     Catches the FILLER words. `skip_filler` discards thirteen words before every atom
           (tactic_script.rs:298-303). A translator emitting readable prose — "first try the
           simp" — has `the` silently removed, which happens to work, but `first try a simp`
           loses `a` too and `try on h` loses `on` and then reads `h` as an atom. The filler
           list makes malformed scripts parse into DIFFERENT well-formed scripts rather than
           failing, so the translator must emit the canonical spelling with no connective words.
@guard     Catches assuming `first` is left-biased in the same way. LOGOS's `first` clones the
           state per alternative (tactic.rs:1078-1087), so an alternative that partially applies
           and then fails leaves no trace — same as Lean. But the alternatives are full
           `parse_seq`s (tactic_script.rs:224-228), so `first [ intro h ; exact h | assumption ]`
           commits the whole first branch or none of it, whereas Lean's
           `first | (intro h; exact h) | assumption` needs the parentheses to mean that.
@oracle    Emitted combinator keywords are byte-for-byte lowercase `first` / `try` / `repeat`;
           every `first` is bracketed with `[` … `]` and bar-separated; every multi-step `try` or
           `repeat` body is parenthesised; and no emitted script contains a filler word from
           `is_filler` (tactic_script.rs:298-303) in a position where it would be consumed.
-/

namespace T08_019

/-- `first`: the first alternative that applies. LOGOS spelling: `first [ assumption | simp ]`. -/
theorem firstAlternative (p : Prop) (hp : p) : p := by
  first
  | assumption
  | simp

/-- `first` where the EARLIER alternative fails and the later one succeeds — the case that
    exercises the backtracking clone. -/
theorem firstBacktracks (n : Nat) : n + 0 = n := by
  first
  | assumption
  | simp

/-- `try` on a tactic that does NOT apply: always succeeds, changes nothing. -/
theorem tryNoOp (p : Prop) (hp : p) : p := by
  try omega
  exact hp

/-- `try` on a tactic that DOES apply. -/
theorem tryApplies (n : Nat) : n + 0 = n := by
  try simp

/-- `try` with a MULTI-STEP body — the parentheses are load-bearing in both languages. -/
theorem tryMultiStep (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  try (constructor; exact hp; exact hq)

/-- `repeat`: apply until it stops applying. Always succeeds. -/
theorem repeatIntros (p q r : Prop) (hr : r) : p → q → r := by
  repeat intro _h
  exact hr

/-- `repeat` on a tactic that never applies: a no-op, not a failure. -/
theorem repeatNoOp (p : Prop) (hp : p) : p := by
  repeat omega
  exact hp

/-- NESTED combinators — each keyword must be lowercase and each multi-step body bracketed. -/
theorem nested (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  constructor <;> first
    | assumption
    | (try simp)

end T08_019
