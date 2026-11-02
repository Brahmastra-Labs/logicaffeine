/-!
@case      T10-010
@tier      10 · Literals and primitive types
@title     `BitVec n` — LOGOS's `BVec n` shares the name and nothing else
@lean      BitVec n, BitVec.ofNat, BitVec.toNat, BitVec.add (mod 2^n), BitVec.extractLsb,
           HAnd / HOr / HXor on BitVec
@expect    reject
@target    (none — `BVec` is a different type, not a re-spelling of `BitVec`)
@evidence  kernel/src/prelude.rs:3468-3517 (`register_bvec`: `BVec : Nat → Type 0`, with
           `BVNil : BVec Zero` and `BVCons : Bit → Π(n:Nat). BVec n → BVec (Succ n)` — a
           UNARY CONS-LIST of `Bit`s indexed by its length, i.e. a linked list, not a machine
           word); kernel/src/prelude.rs:3450-3457 (`Bit`/`B0`/`B1` — the element type is a
           two-constructor inductive, not a boolean bit of a word);
           kernel/src/prelude.rs:3826-3849 (the COMPLETE set of `BVec` operations:
           `bv_and`, `bv_or`, `bv_xor`, `bv_not` — bitwise only. There is NO `bv_add`,
           `bv_mul`, `bv_shl`, `bv_ushr`, no `toNat`, no `ofNat`, and no modular arithmetic
           of any kind); kernel/src/prelude.rs:3477 (`ctx.add_inductive("BVec", …)` — NOT
           `add_indexed_inductive`, so per context.rs:249-253 the whole arity is treated as
           uniform PARAMETERS and `BVec` is registered with ZERO indices even though its
           constructors plainly index it by `Zero`/`Succ n`);
           kernel/src/type_checker.rs:612-678 (`infer_indexed_match` has no index unifier and
           no impossible-case elimination)
@diag      E_TYPE_ABSENT_FROM_TARGET(BitVec) — must name `BitVec`, must state that LOGOS's
           `BVec` is a `Bit` cons-list with four bitwise operations and no arithmetic, and must
           NOT propose `BVec` as the translation. Must also refuse any proposal routed through
           `UInt*` (T10-009) or `Fin (2^n)` (T10-013).
@guard     THE NAME IS THE TRAP. `BVec n` and `BitVec n` have the same arity, the same kind
           (`Nat → Type`), and nearly the same name, so a name-directed translator maps one to
           the other and every SIGNATURE typechecks. The semantics are unrelated: Lean's
           `BitVec n` is `structure BitVec (w : Nat) where toFin : Fin (2^w)` — a NUMBER with
           modular arithmetic — while `BVec n` is `BVNil`/`BVCons`, a list. `BitVec.add` is
           addition mod 2^n; there is no `BVec` addition at all (prelude.rs:3826-3849), so
           `x + y` translates to nothing, and if the translator supplies its own it must decide
           bit ordering, carry propagation and endianness with no specification to check
           against.
@guard     Catches the endianness/ordering silent flip. `BVCons : Bit → Π(n:Nat). BVec n →
           BVec (Succ n)` (prelude.rs:3492-3516) puts the NEW bit first and the length index
           SECOND, after the element — an argument order that no Lean constructor has. A
           translator that emits `BVCons n b v` rather than `BVCons b n v` produces an
           ill-typed term (caught), but one that emits the bits in Lean's LSB-first order into
           a structure the LOGOS `bv_*` definitions read MSB-first produces a well-typed term
           that computes the bit-reversal of the intended value (not caught).
@guard     Catches inheriting the prelude's own params/indices misdeclaration. `BVec` is
           registered with `add_inductive` (prelude.rs:3477), which per context.rs:249-253
           treats its entire arity as uniform parameters — so LOGOS believes `BVec` has one
           PARAMETER and zero indices, while its constructors target `BVec Zero` and
           `BVec (Succ n)`. Matching on a `BVec` therefore goes through the parametric path,
           not `infer_indexed_match`, and the length index is not related to the scrutinee at
           all. Any translation that relies on the index to rule out a case is unsound here;
           see T03-005 and T11-010.
@guard     Catches the "it's fine, I only need bitwise ops" rationalisation. Even restricted to
           `&&&`/`|||`/`^^^`/`~~~`, Lean's operations are on a `Fin (2^w)` and LOGOS's are
           `Match`-bodied recursions over a list (prelude.rs:3826-3849), so the two agree on
           values but not on any lemma: nothing in LOGOS states `bv_and x x = x`, and nothing
           can be proved about a `BVec` of symbolic length without a recursor that
           `derive_recursor` pins to a `Type 0` motive (recursor.rs:64-71).
@oracle    Translator exits non-zero with E_TYPE_ABSENT_FROM_TARGET(BitVec) for every
           declaration below and emits no LOGOS declaration. A harness asserts that
           `Global("BVec")` never appears in output translated from a Lean `BitVec`.
-/

namespace T10_010

/-- Literals and the width index. -/
def w8 : BitVec 8 := 0xFF
def zero8 : BitVec 8 := 0

/-- Lean's `BitVec` is a NUMBER: `structure BitVec (w : Nat) where toFin : Fin (2 ^ w)`.
    `BVec` is a cons-list of `Bit`s. -/
example : (7 : BitVec 8).toFin = (7 : Fin (2 ^ 8)) := rfl

/-- MODULAR ARITHMETIC — the entire point of `BitVec`, and entirely absent from `BVec`
    (prelude.rs:3826-3849 has only `bv_and`/`bv_or`/`bv_xor`/`bv_not`). -/
example : (255 : BitVec 8) + 1 = 0 := by decide
example : (200 : BitVec 8) * 2 = 144 := by decide

/-- Shifts and extraction — no LOGOS counterpart. -/
example : (1 : BitVec 8) <<< 3 = 8 := by decide
example : (0xFF : BitVec 8) >>> 4 = 0x0F := by decide

/-- Conversions to and from `Nat`, which `BVec` does not have either. -/
example : (0xAB : BitVec 8).toNat = 171 := by decide
example : BitVec.ofNat 8 171 = 0xAB := by decide

/-- The bitwise fragment — the only place the two types even look alike. Note that Lean's
    result is a number and LOGOS's would be a list, so agreement here is coincidental. -/
example : (0b1100 : BitVec 4) &&& 0b1010 = 0b1000 := by decide

/-- Length-indexed operations, where the index really is an index. LOGOS registers `BVec`'s
    arity as a PARAMETER (prelude.rs:3477 uses `add_inductive`), so this shape has no faithful
    home there. -/
def appendBits {m n : Nat} (x : BitVec m) (y : BitVec n) : BitVec (m + n) := x ++ y

example : appendBits (0b11 : BitVec 2) (0b00 : BitVec 2) = 0b1100 := by decide

end T10_010
