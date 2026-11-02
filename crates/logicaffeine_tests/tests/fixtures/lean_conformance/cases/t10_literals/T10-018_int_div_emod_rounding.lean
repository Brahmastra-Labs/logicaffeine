/-!
@case      T10-018
@tier      10 · Literals and primitive types
@title     `Int` division and remainder — LOGOS's `div`/`mod` TRUNCATE toward zero; Lean 4's
           `/` and `%` on `Int` are EUCLIDEAN
@lean      HDiv.hDiv / HMod.hMod on Int, Int.ediv, Int.emod, Int.tdiv, Int.tmod, Int.fdiv,
           Int.fmod, Int.emod_nonneg
@expect    degrade
@target    `Global("div")` / `Global("mod")` — the prelude's opaque `Int → Int → Int`
           declarations — ONLY for Lean's `Int.tdiv` / `Int.tmod`. For Lean's `/` and `%`
           (i.e. `Int.ediv` / `Int.emod`) the translator must emit a CORRECTION WRAPPER around
           them, or reject.
@evidence  **Read out of the repo for this case.** kernel/src/prelude.rs:380-381
           (`add_declaration("div", Int → Int → Int)` and `("mod", …)` — opaque axioms, no
           body, no laws: the 16 axioms at prelude.rs:92-340 cover
           `add`/`mul`/`sub`/order only, and say NOTHING about `div` or `mod`);
           kernel/src/reduction.rs:455-466 (the `i64` fast path: `"div" => a.checked_div(*b)`,
           `"mod" => a.checked_rem(*b)` — Rust's `/` and `%` on `i64`, which TRUNCATE toward
           zero and give the remainder the DIVIDEND's sign);
           kernel/src/reduction.rs:486-500 (the exact `BigInt` path:
           `"div" => xb.div_rem(&yb).map(|(q, _)| q)`,
           `"mod" => xb.div_rem(&yb).map(|(_, r)| r)`);
           base/src/numeric.rs:350-375 (`BigInt::div_rem`'s own doc comment: "Truncated
           division toward zero: returns `(quotient, remainder)` with `self = q*other + r` and
           the remainder carrying the dividend's sign — exactly matching Rust/`i64` `/` and
           `%`"; the implementation takes `q_neg` as the XOR of the signs and gives `r` the
           DIVIDEND's sign at :371-372); base/src/numeric.rs:1662-1677 (the test
           `div_rem_matches_i64_truncation_including_signs` pins this against `i128` `/` and
           `%` across negative operands); kernel/src/reduction.rs:490 and
           base/src/numeric.rs:355-357 (division by zero yields `None`, so the term stays STUCK
           — it does not error and does not produce a value)
@loss      ROUNDING_CONVENTION_MISMATCH. Lean 4's `Div Int` / `Mod Int` instances are
           `Int.ediv` / `Int.emod` — EUCLIDEAN division, whose defining property is that the
           remainder is always NON-NEGATIVE (`Int.emod_nonneg`). LOGOS's `div`/`mod` are
           TRUNCATED (T-division), whose remainder carries the dividend's sign. The two agree
           whenever the dividend is non-negative and disagree on every negative dividend not
           exactly divisible:
             Lean  `(-7) / 2 = -4`   LOGOS `div (-7) 2 ⤳ -3`
             Lean  `(-7) % 2 =  1`   LOGOS `mod (-7) 2 ⤳ -1`
             Lean  `7 / (-2) = -3`   LOGOS `div 7 (-2) ⤳ -3`   (agree)
             Lean  `7 % (-2) =  1`   LOGOS `mod 7 (-2) ⤳  1`   (agree)
           So the quotient differs only when the DIVIDEND is negative, and the remainder's
           SIGN differs in exactly the same cases. A faithful translation of Lean's `/` and `%`
           must therefore emit a wrapper, not the bare primitive.
@loss      NO_DIVISION_LAWS. `div` and `mod` are `add_declaration`s (prelude.rs:380-381) with
           no axioms at all, so nothing about them is PROVABLE on symbolic arguments — not
           `a = (a / b) * b + a % b`, not `0 ≤ a % b`, not `a % b < b`. They compute on
           literals via the reduction hook and are inert otherwise. Every Lean lemma about
           `Int.ediv`/`Int.emod` must be re-proved from literal computation or rejected.
@guard     THE CENTRAL BUG: mapping Lean's `/` and `%` on `Int` to `div` and `mod` by name.
           The names match, the types match, the arities match, and the results agree on every
           non-negative example a translator author is likely to test. `infer_type` accepts the
           term, `normalize` reduces it, and the answer is wrong by one on the quotient and
           wrong in SIGN on the remainder for negative dividends. Nothing in LOGOS can detect
           this, because `div` and `mod` have no axioms to contradict.
@guard     Catches mapping Lean's DEPRECATED `Int.div`/`Int.mod` by the same name-directed
           reasoning. In Lean 4.14 `Int.div`/`Int.mod` are the TRUNCATED operations (renamed to
           `Int.tdiv`/`Int.tmod`), so `Int.div ↦ div` happens to be correct while `HDiv.hDiv
           ↦ div` is not — even though `HDiv.hDiv` on `Int` is what the surface `/` elaborates
           to. A translator must dispatch on the ELABORATED constant, and must know that Lean
           has FOUR division conventions (`ediv`, `fdiv`, `tdiv`, and the deprecated `div`
           alias) of which LOGOS implements exactly one.
@guard     Catches `Int.fdiv`/`Int.fmod` (FLOOR division, remainder taking the DIVISOR's sign).
           Floor and Euclidean agree when the divisor is positive and differ when it is
           negative, so a translator that "fixes" the mismatch by assuming floor semantics gets
           `7 / (-2)`: Lean's `ediv` gives `-3`, `fdiv` gives `-4`. Three conventions, three
           answers, one LOGOS primitive.
@guard     Catches division by zero. Lean TOTALISES: `n / 0 = 0` and `n % 0 = n`. LOGOS's
           reduction returns `None` for a zero divisor (reduction.rs:490,
           base/src/numeric.rs:355-357), so the application stays STUCK — and `normalize`
           returns a stuck term without any error (reduction.rs:52-69), so `def_eq(n / 0, 0)`
           is simply FALSE rather than diagnosed. A translation of any Lean proof that uses
           `Int.ediv_zero` silently fails to go through.
@guard     Catches reusing `div`/`mod` for `Nat`. There is no `Nat` division in LOGOS at all
           (prelude.rs:529-552 registers `Nat`, `Zero`, `Succ` and nothing else), and the
           primitive hook matches `Literal::Int`/`BigInt` operands only via `lit_bigint`
           (term.rs:59-63), which returns `None` for `Literal::Nat`. See T10-004.
@oracle    For every Lean `/` or `%` on `Int` in the file, the emitted term either (a) is a
           translator-emitted wrapper whose `normalize` on literal arguments agrees with Lean's
           value on ALL SIXTEEN sign/divisibility combinations below, or (b) is absent and a
           rejection was reported. A bare `App(App(Global("div"), a), b)` translating a Lean `/`
           is a FAILURE. The loss record must contain ROUNDING_CONVENTION_MISMATCH and
           NO_DIVISION_LAWS.
@note      INDEX lists this as `degrade`, which is right for `Int.tdiv`/`Int.tmod` (a faithful
           map onto `div`/`mod`, losing only the laws). For Lean's `/` and `%` — the operations
           that actually appear in source — the honest verdict is closer to `reject` unless the
           translator emits the correction wrapper, because the bare mapping is not lossy, it
           is WRONG. This file is kept at `degrade` on the reading that emitting the wrapper is
           a real translation and the loss is NO_DIVISION_LAWS; a translator that emits the
           bare primitive must be scored as failing the oracle, not as degrading.
-/

namespace T10_018

/-! ### What Lean 4.14 actually does: `/` and `%` on `Int` are EUCLIDEAN -/

example : (-7 : Int) / 2 = -4 := by decide
example : (-7 : Int) % 2 = 1 := by decide

example : (7 : Int) / 2 = 3 := by decide
example : (7 : Int) % 2 = 1 := by decide

example : (7 : Int) / (-2) = -3 := by decide
example : (7 : Int) % (-2) = 1 := by decide

example : (-7 : Int) / (-2) = 4 := by decide
example : (-7 : Int) % (-2) = 1 := by decide

/-- The defining property of Euclidean remainder, and the one LOGOS's `mod` violates. -/
example (a b : Int) (h : b ≠ 0) : 0 ≤ a % b := Int.emod_nonneg a h

/-! ### What LOGOS's `div`/`mod` compute — Lean's `Int.tdiv` / `Int.tmod`
     (reduction.rs:461-462 via `i64::checked_div`/`checked_rem`;
      reduction.rs:493-494 via `BigInt::div_rem`, base/src/numeric.rs:350-375) -/

example : Int.tdiv (-7) 2 = -3 := by decide
example : Int.tmod (-7) 2 = -1 := by decide

example : Int.tdiv 7 (-2) = -3 := by decide
example : Int.tmod 7 (-2) = 1 := by decide

/-- The quotients differ exactly when the dividend is negative and the division is inexact. -/
example : (-7 : Int) / 2 ≠ Int.tdiv (-7) 2 := by decide

/-- The remainders differ in SIGN in exactly those cases. -/
example : (-7 : Int) % 2 ≠ Int.tmod (-7) 2 := by decide

/-- And they agree on every non-negative dividend — which is why testing does not catch it. -/
example : (7 : Int) / 2 = Int.tdiv 7 2 := by decide
example : (7 : Int) % 2 = Int.tmod 7 2 := by decide

/-! ### The third convention, to rule out the "assume floor" repair -/

example : Int.fdiv 7 (-2) = -4 := by decide
example : Int.fmod 7 (-2) = -1 := by decide

/-- Floor and Euclidean agree for a positive divisor and differ for a negative one. -/
example : Int.fdiv (-7) 2 = (-7 : Int) / 2 := by decide
example : Int.fdiv 7 (-2) ≠ (7 : Int) / (-2) := by decide

/-! ### Division by zero: Lean totalises, LOGOS gets stuck -/

example : (5 : Int) / 0 = 0 := by decide
example : (5 : Int) % 0 = 5 := by decide

/-! ### The wrapper the translator must emit, written in Lean so the target is unambiguous.
     Euclidean from truncated: adjust when the truncated remainder is negative. -/

def ediv' (a b : Int) : Int :=
  if Int.tmod a b < 0 then (if 0 < b then Int.tdiv a b - 1 else Int.tdiv a b + 1)
  else Int.tdiv a b

def emod' (a b : Int) : Int :=
  if Int.tmod a b < 0 then (if 0 < b then Int.tmod a b + b else Int.tmod a b - b)
  else Int.tmod a b

example : ediv' (-7) 2 = -4 := by decide
example : emod' (-7) 2 = 1 := by decide
example : ediv' 7 (-2) = -3 := by decide
example : emod' 7 (-2) = 1 := by decide
example : ediv' (-7) (-2) = 4 := by decide
example : emod' (-7) (-2) = 1 := by decide
example : ediv' 7 2 = 3 := by decide
example : emod' 7 2 = 1 := by decide

end T10_018
