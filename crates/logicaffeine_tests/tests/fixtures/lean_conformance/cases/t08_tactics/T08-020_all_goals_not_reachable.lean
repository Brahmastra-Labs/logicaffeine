/-!
@case      T08-020
@tier      8 · Tactics
@title     `all_goals` / `any_goals` — `all_goals` is Rust-only and `any_goals` does not exist
           at all
@lean      all_goals, any_goals, focus, iterate
@expect    reject
@target    (none — no sound translation)
@evidence  proof/src/tactic_script.rs:308-327 (`canonical` — no arm maps any word to
           "all_goals"; the lexer WILL produce a single `Tok::Word("all_goals")`, since `_` is an
           identifier character at :89-93, so the failure is the generic
           `ScriptError("unknown tactic \`all_goals\`")` at :262, not a lexing error);
           proof/src/tactic_script.rs:275-292 (`parse_primitive`'s dispatch — fifteen arms, none
           of them `all_goals`);
           proof/src/tactic.rs:1124-1136 (`combinators::all_goals` DOES exist as a Rust value:
           it snapshots `st.open`, focuses each goal in turn, runs `t`, and collects the
           resulting goals in original order — but nothing in the script grammar constructs it
           directly);
           proof/src/tactic.rs:1138-1146 (`then_all`, the `<;>` combinator, is the ONLY
           script-reachable consumer of `all_goals` — and it always runs `t1` first, so
           `all_goals t` standing alone has no spelling);
           a grep of `crates/logicaffeine_proof/src` for `any_goals` returns NOTHING — the
           combinator does not exist in Rust either;
           proof/src/tactic.rs:1130 (`all_goals` propagates failure with `t(st)?` — it is
           strictly `all_goals`, never `any_goals`, and there is no `try`-wrapped variant)
@diag      E_TACTIC_NOT_IN_SCRIPT_GRAMMAR — must name the tactic (`all_goals` or `any_goals`),
           state that the script primitive set is closed (tactic_script.rs:275-292), and for
           `any_goals` additionally state that no Rust combinator exists either, so it is not
           reachable at any level. Must NOT emit a script.
@guard     THE `<;>` SUBSTITUTION TRAP. `all_goals t` is *almost* `<;> t`, and a translator will
           be tempted to synthesise `something <;> t`. It cannot, because `<;>` needs a
           left-hand tactic and `all_goals` has none — the goals it acts on were produced by an
           EARLIER, already-emitted step. Splicing `t` onto that earlier step changes the
           script's structure: the earlier step's own effects are now inside a chain, and — per
           T08-018 — LOGOS's `then_all` spreads `t` over the WHOLE open set, which may include
           goals from steps before that. The rewrite is not local and is not sound.
@guard     Catches substituting `repeat t`. `repeat` (tactic.rs:1104-1119) applies `t` to the
           FOCUSED goal over and over; `all_goals` applies it ONCE to each of many goals. On a
           state with three open goals and a `t` that closes the focused one, `repeat t` closes
           all three — by accident, because closing one refocuses the next — and on a `t` that
           does not close its goal, `repeat t` applies `t` to the same goal 100_000 times. The
           two combinators coincide only in the special case where `t` closes every goal it
           touches, which is exactly the case a translator's tests will cover.
@guard     Catches emitting `t` alone. Dropping `all_goals` leaves `t` acting on the focused
           goal only. The remaining goals stay open, `qed()` fails with
           `TacticError::GoalsRemain` (tactic.rs:967-968), and the reported count points at the
           end of the proof rather than at the dropped combinator.
@guard     Catches conflating `any_goals` with `all_goals`. Lean's `any_goals t` succeeds if `t`
           succeeds on at least one goal; `all_goals t` requires all. LOGOS's combinator is
           `all_goals` (failure propagates at tactic.rs:1130) and there is no `try`-wrapped
           variant, so even a Rust-level translator cannot express `any_goals` without writing
           new Rust. A translator that emits `all_goals` for Lean's `any_goals` turns a
           permissive step into a strict one and fails proofs Lean accepted.
@guard     Catches `focus` and `iterate`. Neither has a script spelling either; `focus` is
           additionally meaningless in LOGOS because there is no focusing bracket at all
           (see T08-025), and `iterate n t` has no bounded-repetition form — `repeat` is
           unbounded-until-failure.
@oracle    Translator exits non-zero with E_TACTIC_NOT_IN_SCRIPT_GRAMMAR for every theorem below
           and emits no script. A harness asserts that no emitted script contains the substrings
           `all_goals`, `any_goals`, `focus` or `iterate`, and that every emitted `<;>` in the
           corpus corresponds to a `<;>` in the Lean source rather than to a synthesised one.
-/

namespace T08_020

/-- `all_goals` after a branching step. -/
theorem allGoalsAfterConstructor (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  constructor
  all_goals assumption

/-- `all_goals` where the goals came from TWO steps back — the case that makes a `<;>` rewrite
    non-local. -/
theorem allGoalsTwoStepsBack (p q r : Prop) (hp : p) (hq : q) (hr : r) : (p ∧ q) ∧ r := by
  constructor
  constructor
  all_goals assumption

/-- `any_goals`: succeeds because SOME goal yields. `all_goals` would fail here. -/
theorem anyGoals (p : Prop) (hp : p) : p ∧ (0 = 0) := by
  constructor
  any_goals assumption
  rfl

/-- `all_goals` with a tactic that does NOT close its goals — so `repeat` is not equivalent. -/
theorem allGoalsNonClosing (p q : Prop) (hp : p) (hq : q) : (p ∧ p) ∧ (q ∧ q) := by
  constructor
  all_goals constructor
  · exact hp
  · exact hp
  · exact hq
  · exact hq

/-- `focus`: restrict attention to the first goal. No LOGOS analogue. -/
theorem focused (p q : Prop) (hp : p) (hq : q) : p ∧ q := by
  constructor
  focus
    exact hp
  exact hq

/-- `iterate`: bounded repetition. LOGOS's `repeat` is unbounded-until-failure. -/
theorem iterated (p q r s : Prop) (hs : s) : p → q → r → s := by
  iterate 3 intro _h
  exact hs

end T08_020
