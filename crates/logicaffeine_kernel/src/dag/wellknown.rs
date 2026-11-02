//! Names the kernel gives meaning to, interned once so every test is an id compare.

use super::name::{NameId, Names};

/// One quotient's four constants: its former, constructor, lift and induction principle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuotNames {
    pub ty: NameId,
    pub mk: NameId,
    pub lift: NameId,
    pub ind: NameId,
}

#[derive(Clone)]
pub struct WellKnown {
    pub nat: NameId,
    pub nat_zero: NameId,
    pub nat_succ: NameId,
    pub nat_add: NameId,
    pub nat_sub: NameId,
    pub nat_mul: NameId,
    pub nat_pow: NameId,
    pub nat_gcd: NameId,
    pub nat_mod: NameId,
    pub nat_div: NameId,
    pub nat_beq: NameId,
    pub nat_ble: NameId,
    pub nat_land: NameId,
    pub nat_lor: NameId,
    pub nat_xor: NameId,
    pub nat_shift_left: NameId,
    pub nat_shift_right: NameId,
    pub nat_pred: NameId,
    pub nat_bitwise: NameId,
    pub bool_: NameId,
    pub bool_true: NameId,
    pub bool_false: NameId,
    pub string: NameId,
    pub string_of_list: NameId,
    pub char_: NameId,
    pub char_of_nat: NameId,
    pub list_nil: NameId,
    pub list_cons: NameId,
    pub list: NameId,
    pub eq: NameId,
    pub quot: NameId,
    pub quot_mk: NameId,
    pub quot_lift: NameId,
    pub quot_ind: NameId,
    /// A second quotient the theory admits beside its own: under LOGOS's theory, the one a kernel
    /// installs (`kernel.quot.*`, `quotient::Kind`'s names) for content imported from Lean.
    pub installed_quot: Option<QuotNames>,
    /// The theory's own `true` beside the primitives' `Bool.true`, where they differ: LOGOS's
    /// `true`, read by the `decide` fast path (`Kernel::is_true_constant`).
    pub logos_true: Option<NameId>,
    pub eager_reduce: NameId,
    pub opt_param: NameId,
    pub auto_param: NameId,
    pub out_param: NameId,
    pub semi_out_param: NameId,
    pub nested_prefix: NameId,
    pub u: NameId,
}

impl WellKnown {
    pub fn new(n: &mut Names) -> WellKnown {
        WellKnown {
            nat: n.dotted("Nat"),
            nat_zero: n.dotted("Nat.zero"),
            nat_succ: n.dotted("Nat.succ"),
            nat_add: n.dotted("Nat.add"),
            nat_sub: n.dotted("Nat.sub"),
            nat_mul: n.dotted("Nat.mul"),
            nat_pow: n.dotted("Nat.pow"),
            nat_gcd: n.dotted("Nat.gcd"),
            nat_mod: n.dotted("Nat.mod"),
            nat_div: n.dotted("Nat.div"),
            nat_beq: n.dotted("Nat.beq"),
            nat_ble: n.dotted("Nat.ble"),
            nat_land: n.dotted("Nat.land"),
            nat_lor: n.dotted("Nat.lor"),
            nat_xor: n.dotted("Nat.xor"),
            nat_shift_left: n.dotted("Nat.shiftLeft"),
            nat_shift_right: n.dotted("Nat.shiftRight"),
            nat_pred: n.dotted("Nat.pred"),
            nat_bitwise: n.dotted("Nat.bitwise"),
            bool_: n.dotted("Bool"),
            bool_true: n.dotted("Bool.true"),
            bool_false: n.dotted("Bool.false"),
            string: n.dotted("String"),
            string_of_list: n.dotted("String.ofList"),
            char_: n.dotted("Char"),
            char_of_nat: n.dotted("Char.ofNat"),
            list_nil: n.dotted("List.nil"),
            list_cons: n.dotted("List.cons"),
            list: n.dotted("List"),
            eq: n.dotted("Eq"),
            quot: n.dotted("Quot"),
            quot_mk: n.dotted("Quot.mk"),
            quot_lift: n.dotted("Quot.lift"),
            quot_ind: n.dotted("Quot.ind"),
            installed_quot: None,
            logos_true: None,
            eager_reduce: n.dotted("eagerReduce"),
            opt_param: n.dotted("optParam"),
            auto_param: n.dotted("autoParam"),
            out_param: n.dotted("outParam"),
            semi_out_param: n.dotted("semiOutParam"),
            nested_prefix: n.dotted("_nested"),
            u: n.dotted("u"),
        }
    }

    /// The same meanings in a LOGOS context. LOGOS's own constants keep its spelling: `Nat`,
    /// `Zero`/`Succ`, `Eq` and the quotient's `Quot_mk`/`Quot_lift`/`Quot_ind`, its `true` read by
    /// the `decide` fast path. A meaning only Lean content has (the `Nat` operations, `Bool`,
    /// `Char`, `String`, `List`, the elaborator's markers) is spelled as the Lean importer names it
    /// in a LOGOS context (`dag::name::kernel_name_of_components`), so an imported
    /// `Nat.div` is the primitive it is, checked and computed as in Lean's theory.
    pub fn logos(n: &mut Names) -> WellKnown {
        let one = |n: &mut Names, s: &str| n.str(NameId::ANON, s);
        let lean = |n: &mut Names, s: &str| {
            let components: Vec<&str> = s.split('.').collect();
            let spelled = super::name::kernel_name_of_components(&components);
            n.str(NameId::ANON, &spelled)
        };
        WellKnown {
            nat: one(n, "Nat"),
            nat_zero: one(n, "Zero"),
            nat_succ: one(n, "Succ"),
            nat_add: lean(n, "Nat.add"),
            nat_sub: lean(n, "Nat.sub"),
            nat_mul: lean(n, "Nat.mul"),
            nat_pow: lean(n, "Nat.pow"),
            nat_gcd: lean(n, "Nat.gcd"),
            nat_mod: lean(n, "Nat.mod"),
            nat_div: lean(n, "Nat.div"),
            nat_beq: lean(n, "Nat.beq"),
            nat_ble: lean(n, "Nat.ble"),
            nat_land: lean(n, "Nat.land"),
            nat_lor: lean(n, "Nat.lor"),
            nat_xor: lean(n, "Nat.xor"),
            nat_shift_left: lean(n, "Nat.shiftLeft"),
            nat_shift_right: lean(n, "Nat.shiftRight"),
            nat_pred: lean(n, "Nat.pred"),
            nat_bitwise: lean(n, "Nat.bitwise"),
            bool_: lean(n, "Bool"),
            bool_true: lean(n, "Bool.true"),
            bool_false: lean(n, "Bool.false"),
            string: lean(n, "String"),
            string_of_list: lean(n, "String.ofList"),
            char_: lean(n, "Char"),
            char_of_nat: lean(n, "Char.ofNat"),
            list_nil: lean(n, "List.nil"),
            list_cons: lean(n, "List.cons"),
            list: lean(n, "List"),
            quot_mk: one(n, "Quot_mk"),
            quot_lift: one(n, "Quot_lift"),
            quot_ind: one(n, "Quot_ind"),
            installed_quot: Some(QuotNames {
                ty: one(n, crate::quotient::Kind::Type.name()),
                mk: one(n, crate::quotient::Kind::Constructor.name()),
                lift: one(n, crate::quotient::Kind::Lift.name()),
                ind: one(n, crate::quotient::Kind::Induction.name()),
            }),
            logos_true: Some(one(n, "true")),
            eager_reduce: lean(n, "eagerReduce"),
            opt_param: lean(n, "optParam"),
            auto_param: lean(n, "autoParam"),
            out_param: lean(n, "outParam"),
            semi_out_param: lean(n, "semiOutParam"),
            ..WellKnown::new(n)
        }
    }

    /// The theory's own quotient.
    pub fn quot_names(&self) -> QuotNames {
        QuotNames {
            ty: self.quot,
            mk: self.quot_mk,
            lift: self.quot_lift,
            ind: self.quot_ind,
        }
    }

    /// The names Lean's kernel treats as primitive: declaring one is only allowed in its
    /// canonical form.
    pub fn primitives(&self) -> [NameId; 24] {
        [
            self.bool_,
            self.bool_false,
            self.bool_true,
            self.nat,
            self.nat_zero,
            self.nat_succ,
            self.nat_add,
            self.nat_pred,
            self.nat_sub,
            self.nat_mul,
            self.nat_pow,
            self.nat_gcd,
            self.nat_mod,
            self.nat_div,
            self.nat_beq,
            self.nat_ble,
            self.nat_bitwise,
            self.nat_land,
            self.nat_lor,
            self.nat_xor,
            self.nat_shift_left,
            self.nat_shift_right,
            self.string_of_list,
            self.char_of_nat,
        ]
    }
}
