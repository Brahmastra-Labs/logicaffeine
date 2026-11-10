#!/usr/bin/env python3
"""Hand-built Arena-format exports that isolate one kernel rule each.

The mutation matrix (mutation-matrix.sh) reports kernel logic the Arena corpus never exercises
alone. Each case here is a `lean4export` 3.1.0 file whose ONLY defect (for a `bad/` case) is the
rule it targets: every other claim — recursor types, rules, flags — is exactly what the kernel
derives if that one rule were skipped. So the case is rejected by that rule or not at all, and a
mutant that disables the rule is caught.

Writes crates/logicaffeine_tests/tests/fixtures/crafted/{good,bad}/<name>.ndjson and manifest.json, or the same tree
under `--out DIR` (dag_crafted regenerates it there and holds the checked-in fixtures to it).
"""
import json, os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
OUT = os.path.join(ROOT, "crates/logicaffeine_tests/tests/fixtures/crafted")
META = {"meta": {"exporter": {"name": "logos-crafted", "version": "1"}, "format": {"version": "3.1.0"},
                 "lean": {"githash": "", "version": "4.29.1"}}}


class Export:
    """A tiny builder for the lean4export NDJSON format: interned names, levels, expressions."""

    def __init__(self):
        self.lines = [META]
        self.names, self.levels, self.exprs = {(): 0}, {("zero",): 0}, {}

    def _emit(self, rec):
        self.lines.append(rec)

    def name(self, dotted):
        parts = tuple(dotted.split("."))
        for i in range(1, len(parts) + 1):
            key = parts[:i]
            if key not in self.names:
                idx = len(self.names)
                self.names[key] = idx
                self._emit({"in": idx, "str": {"pre": self.names[key[:-1]], "str": key[-1]}})
        return self.names[parts]

    def _level(self, key, body):
        if key not in self.levels:
            idx = len(self.levels)
            self.levels[key] = idx
            self._emit(dict(body, il=idx))
        return self.levels[key]

    def lzero(self):
        return 0

    def lsucc(self, l):
        return self._level(("succ", l), {"succ": l})

    def lparam(self, n):
        return self._level(("param", n), {"param": self.name(n)})

    def _expr(self, key, body):
        if key not in self.exprs:
            idx = len(self.exprs)
            self.exprs[key] = idx
            self._emit(dict(body, ie=idx))
        return self.exprs[key]

    def sort(self, l):
        return self._expr(("sort", l), {"sort": l})

    def prop(self):
        return self.sort(0)

    def typ(self):
        return self.sort(self.lsucc(0))

    def const(self, n, us=()):
        nid = self.name(n)
        return self._expr(("const", nid, tuple(us)), {"const": {"name": nid, "us": list(us)}})

    def bvar(self, i):
        return self._expr(("bvar", i), {"bvar": i})

    def app(self, f, *args):
        for a in args:
            f = self._expr(("app", f, a), {"app": {"arg": a, "fn": f}})
        return f

    def pi(self, ty, body, binder="x"):
        n = self.name(binder)
        return self._expr(("pi", ty, body), {"forallE": {"binderInfo": "default", "body": body, "name": n, "type": ty}})

    def lam(self, ty, body, binder="x"):
        n = self.name(binder)
        return self._expr(("lam", ty, body), {"ie": None, "lam": {"binderInfo": "default", "body": body, "name": n, "type": ty}})

    def axiom(self, n, ty, lparams=()):
        self._emit({"axiom": {"isUnsafe": False, "levelParams": [self.name(p) for p in lparams], "name": self.name(n), "type": ty}})

    def defn(self, n, ty, value, lparams=(), safety="safe", all_=None, hints="abbrev"):
        self._emit({"def": {"all": [self.name(a) for a in (all_ or [n])], "hints": hints,
                            "levelParams": [self.name(p) for p in lparams], "name": self.name(n),
                            "safety": safety, "type": ty, "value": value}})

    def thm(self, n, ty, value, lparams=()):
        self._emit({"thm": {"all": [self.name(n)], "levelParams": [self.name(p) for p in lparams], "name": self.name(n),
                            "type": ty, "value": value}})

    def quot(self, kind, n, ty, lparams):
        self._emit({"quot": {"kind": kind, "levelParams": [self.name(p) for p in lparams], "name": self.name(n),
                             "type": ty}})

    def inductive(self, types, ctors, recs):
        self._emit({"inductive": {"types": types, "ctors": ctors, "recs": recs}})

    def text(self):
        out = []
        for rec in self.lines:
            if "lam" in rec:
                rec = {"ie": rec["ie"], "lam": rec["lam"]}
            out.append(json.dumps(rec, sort_keys=True, separators=(",", ":")))
        return "\n".join(out) + "\n"


def single_ctor_family(x, ind, ctor, field_ty, is_rec, is_reflexive, univ_type):
    """An inductive `ind : Type` with one constructor `ctor : field_ty → ind` whose field is not a
    recursive argument: the recursor the kernel derives, spelled out."""
    u = x.lparam("u")
    indc = x.const(ind)
    mk = x.const(ctor)
    motive_ty = x.pi(indc, x.sort(u), "t")
    # minor : (a : F) → motive (mk a), under the motive binder
    minor_ty = x.pi(field_ty, x.app(x.bvar(1), x.app(mk, x.bvar(0))), "a")
    rec_ty = x.pi(motive_ty, x.pi(minor_ty, x.pi(indc, x.app(x.bvar(2), x.bvar(0)), "t"), "mk"), "motive")
    rhs = x.lam(motive_ty, x.lam(minor_ty, x.lam(field_ty, x.app(x.bvar(1), x.bvar(0)), "a"), "mk"), "motive")
    x.inductive(
        types=[{"all": [x.name(ind)], "ctors": [x.name(ctor)], "isRec": is_rec, "isReflexive": is_reflexive,
                "isUnsafe": False, "levelParams": [], "name": x.name(ind), "numIndices": 0, "numNested": 0,
                "numParams": 0, "type": univ_type}],
        ctors=[{"cidx": 0, "induct": x.name(ind), "isUnsafe": False, "levelParams": [], "name": x.name(ctor),
                "numFields": 1, "numParams": 0, "type": x.pi(field_ty, indc, "a")}],
        recs=[{"all": [x.name(ind)], "isUnsafe": False, "k": False, "levelParams": [x.name("u")],
               "name": x.name(ind + ".rec"), "numIndices": 0, "numMinors": 1, "numMotives": 1, "numParams": 0,
               "rules": [{"ctor": x.name(ctor), "nfields": 1, "rhs": rhs}], "type": rec_ty}])


def unit_proposition(x, ind, ctor):
    """`inductive ind : Prop | ctor : ind`, `True`'s shape, with the K-like recursor the kernel derives."""
    u = x.lparam("u")
    t, c = x.const(ind), x.const(ctor)
    motive_ty = x.pi(t, x.sort(u), "t")
    minor_ty = x.app(x.bvar(0), c)
    rec_ty = x.pi(motive_ty, x.pi(minor_ty, x.pi(t, x.app(x.bvar(2), x.bvar(0)), "t"), "intro"), "motive")
    rhs = x.lam(motive_ty, x.lam(minor_ty, x.bvar(0), "intro"), "motive")
    x.inductive(
        types=[{"all": [x.name(ind)], "ctors": [x.name(ctor)], "isRec": False, "isReflexive": False, "isUnsafe": False,
                "levelParams": [], "name": x.name(ind), "numIndices": 0, "numNested": 0, "numParams": 0,
                "type": x.prop()}],
        ctors=[{"cidx": 0, "induct": x.name(ind), "isUnsafe": False, "levelParams": [], "name": x.name(ctor),
                "numFields": 0, "numParams": 0, "type": t}],
        recs=[{"all": [x.name(ind)], "isUnsafe": False, "k": True, "levelParams": [x.name("u")],
               "name": x.name(ind + ".rec"), "numIndices": 0, "numMinors": 1, "numMotives": 1, "numParams": 0,
               "rules": [{"ctor": x.name(ctor), "nfields": 0, "rhs": rhs}], "type": rec_ty}])


def eq_family(x):
    """Lean's `Eq.{u_1} : {α : Sort u_1} → α → α → Prop` with `Eq.refl` and the K-like recursor the kernel derives:
    the equality a quotient's lift states its respect hypothesis with."""
    u1, u = x.lparam("u_1"), x.lparam("u")
    alpha = x.sort(u1)

    def eq(*args):
        return x.app(x.const("Eq", [u1]), *args)

    refl_ty = x.pi(alpha, x.pi(x.bvar(0), eq(x.bvar(1), x.bvar(0), x.bvar(0)), "a"), "α")
    motive_ty = x.pi(x.bvar(1), x.pi(eq(x.bvar(2), x.bvar(1), x.bvar(0)), x.sort(u), "t"), "b")
    minor_ty = x.app(x.bvar(0), x.bvar(1), x.app(x.const("Eq.refl", [u1]), x.bvar(2), x.bvar(1)))
    major = x.pi(eq(x.bvar(4), x.bvar(3), x.bvar(0)), x.app(x.bvar(3), x.bvar(1), x.bvar(0)), "t")
    rec_ty = x.pi(alpha, x.pi(x.bvar(0), x.pi(motive_ty, x.pi(minor_ty, x.pi(x.bvar(3), major, "b"), "refl"),
                                              "motive"), "a"), "α")
    rhs = x.lam(alpha, x.lam(x.bvar(0), x.lam(motive_ty, x.lam(minor_ty, x.bvar(0), "refl"), "motive"), "a"), "α")
    x.inductive(
        types=[{"all": [x.name("Eq")], "ctors": [x.name("Eq.refl")], "isRec": False, "isReflexive": False,
                "isUnsafe": False, "levelParams": [x.name("u_1")], "name": x.name("Eq"), "numIndices": 1,
                "numNested": 0, "numParams": 2,
                "type": x.pi(alpha, x.pi(x.bvar(0), x.pi(x.bvar(1), x.prop(), "b"), "a"), "α")}],
        ctors=[{"cidx": 0, "induct": x.name("Eq"), "isUnsafe": False, "levelParams": [x.name("u_1")],
                "name": x.name("Eq.refl"), "numFields": 0, "numParams": 2, "type": refl_ty}],
        recs=[{"all": [x.name("Eq")], "isUnsafe": False, "k": True, "levelParams": [x.name("u"), x.name("u_1")],
               "name": x.name("Eq.rec"), "numIndices": 1, "numMinors": 1, "numMotives": 1, "numParams": 2,
               "rules": [{"ctor": x.name("Eq.refl"), "nfields": 0, "rhs": rhs}], "type": rec_ty}])


# Each quotient constant: its name and universe parameters, as Lean's `init_quot` declares them.
QUOT_CONSTANTS = {"type": ("Quot", ["u"]), "ctor": ("Quot.mk", ["u"]), "lift": ("Quot.lift", ["u", "v"]),
                  "ind": ("Quot.ind", ["u"])}


def relation(x, alpha):
    """`α → α → Prop`, for `α` the de Bruijn index `alpha` where the relation is bound."""
    return x.pi(x.bvar(alpha), x.pi(x.bvar(alpha + 1), x.prop(), "b"), "a")


def respect(x):
    """`(a b : α) → r a b → Eq.{v} β (f a) (f b)` under the binders `α r β f`: the lift's respect hypothesis."""
    v = x.lparam("v")
    eq = x.app(x.const("Eq", [v]), x.bvar(4), x.app(x.bvar(3), x.bvar(2)), x.app(x.bvar(3), x.bvar(1)))
    return x.pi(x.bvar(3), x.pi(x.bvar(4), x.pi(x.app(x.bvar(4), x.bvar(1), x.bvar(0)), eq, "h"), "b"), "a")


def quot_type(x, kind):
    """The canonical type of the quotient constant `kind`, exactly as the kernel builds it."""
    u = x.lparam("u")
    sort_u = x.sort(u)

    def quot(a, r):
        return x.app(x.const("Quot", [u]), a, r)

    if kind == "type":
        return x.pi(sort_u, x.pi(relation(x, 0), sort_u, "r"), "α")
    if kind == "ctor":
        return x.pi(sort_u, x.pi(relation(x, 0), x.pi(x.bvar(1), quot(x.bvar(2), x.bvar(1)), "a"), "r"), "α")
    if kind == "lift":
        v = x.lparam("v")
        body = x.pi(respect(x), x.pi(quot(x.bvar(4), x.bvar(3)), x.bvar(3), "q"), "h")
        return x.pi(sort_u, x.pi(relation(x, 0), x.pi(x.sort(v), x.pi(x.pi(x.bvar(2), x.bvar(1), "a"), body, "f"),
                                                      "β"), "r"), "α")
    motive = x.pi(quot(x.bvar(1), x.bvar(0)), x.prop(), "q")
    minor = x.pi(x.bvar(2), x.app(x.bvar(1), x.app(x.const("Quot.mk", [u]), x.bvar(3), x.bvar(2), x.bvar(0))), "a")
    concl = x.pi(quot(x.bvar(3), x.bvar(2)), x.app(x.bvar(2), x.bvar(0)), "q")
    return x.pi(sort_u, x.pi(relation(x, 0), x.pi(motive, x.pi(minor, concl, "mk"), "β"), "r"), "α")


def quotient_constant(x, kind, as_axiom=False):
    """The quotient constant `kind` by its own record, or as an ordinary axiom of the same name and type."""
    name, lparams = QUOT_CONSTANTS[kind]
    ty = quot_type(x, kind)
    if as_axiom:
        x.axiom(name, ty, lparams)
    else:
        x.quot(kind, name, ty, lparams)


def lift_computes(x):
    """`quotLiftReduction : Quot.lift f h (Quot.mk r a) = f a` by `Eq.refl`: it checks only where the lift computes."""
    u, v = x.lparam("u"), x.lparam("v")
    sort_u = x.sort(u)
    lifted = x.app(x.const("Quot.lift", [u, v]), x.bvar(5), x.bvar(4), x.bvar(3), x.bvar(2), x.bvar(1),
                   x.app(x.const("Quot.mk", [u]), x.bvar(5), x.bvar(4), x.bvar(0)))
    goal = x.app(x.const("Eq", [v]), x.bvar(3), lifted, x.app(x.bvar(2), x.bvar(0)))
    proof = x.app(x.const("Eq.refl", [v]), x.bvar(3), lifted)

    def binders(close, body):
        return close(sort_u, close(relation(x, 0), close(x.sort(v), close(x.pi(x.bvar(2), x.bvar(1), "a"), close(
            respect(x), close(x.bvar(4), body, "a"), "h"), "f"), "β"), "r"), "α")

    x.thm("quotLiftReduction", binders(x.pi, goal), binders(x.lam, proof), ["u", "v"])


def quotient_package(x, order=("type", "ctor", "lift", "ind"), as_axiom=()):
    """`Eq`, the four quotient constants in `order` (those in `as_axiom` as ordinary axioms), the lift's
    computation rule at work, and a proposition declared after them."""
    eq_family(x)
    for kind in order:
        quotient_constant(x, kind, kind in as_axiom)
    lift_computes(x)
    unit_proposition(x, "T", "T.intro")


CASES = []


def case(outcome, name, targets, description, rejected_by=None):
    def wrap(f):
        CASES.append((outcome, name, targets, description, rejected_by, f))
        return f
    return wrap


@case("good", "positive-control", "the recursor builder itself",
      "A non-recursive inductive whose single field mentions no inductive: accepted, so the recursor "
      "the builder spells out is exactly the one the kernel derives.")
def positive_control(x):
    single_ctor_family(x, "Good", "Good.mk", x.pi(x.prop(), x.prop(), "p"), False, False, x.typ())


@case("bad", "non-strictly-positive", "inductive.rs: check_positivity for safe families",
      "`Bad | mk : ((Bad → Prop) → Prop) → Bad`: `Bad` occurs to the left of an arrow inside a field. "
      "The field is not a recursive argument, so recursor construction goes through; only strict "
      "positivity rejects it.", rejected_by="non positive occurrence")
def non_strictly_positive(x):
    bad = x.const("Bad")
    field = x.pi(x.pi(bad, x.prop(), "b"), x.prop(), "k")
    single_ctor_family(x, "Bad", "Bad.mk", field, True, True, x.typ())


@case("bad", "undefined-universe-parameter", "level.rs: undef_param / tc.rs: check_level",
      "`axiom foo : Sort u` with no universe parameters declared: `u` is unbound.",
      rejected_by="undefined universe level parameter")
def undefined_universe(x):
    x.axiom("foo", x.sort(x.lparam("u")))


@case("bad", "reserved-nested-prefix", "inductive.rs: check_no_nested_aux",
      "A constructor whose field mentions a constant named `_nested.X`: the prefix is reserved for the "
      "kernel's own nested-inductive auxiliaries.", rejected_by="reserved prefix")
def reserved_nested_prefix(x):
    x.axiom("_nested.X", x.typ())
    single_ctor_family(x, "T", "T.mk", x.const("_nested.X"), False, False, x.typ())


@case("good", "mutual-unsafe-pair", "check.rs: add_mutual (unsafe blocks)",
      "`unsafe def f : Prop := g` and `unsafe def g : Prop := f` as one mutual block.")
def mutual_unsafe_pair(x):
    f, g = x.const("f"), x.const("g")
    x.defn("f", x.prop(), g, safety="unsafe", all_=["f", "g"])
    x.defn("g", x.prop(), f, safety="unsafe", all_=["f", "g"])


@case("bad", "mutual-mixed-safety", "check.rs: add_mutual (one safety per block)",
      "A mutual block mixing an `unsafe` and a `partial` definition.", rejected_by="same safety annotation")
def mutual_mixed_safety(x):
    f, g = x.const("f"), x.const("g")
    x.defn("f", x.prop(), g, safety="unsafe", all_=["f", "g"])
    x.defn("g", x.prop(), f, safety="partial", all_=["f", "g"])


@case("bad", "mutual-mismatched-universes", "check.rs: add_mutual (one universe list per block)",
      "A mutual unsafe block whose two definitions declare different universe parameters.",
      rejected_by="same universe parameters")
def mutual_mismatched_universes(x):
    u = x.lparam("u")
    f, g = x.const("f", [u]), x.const("g")
    x.defn("f", x.prop(), g, lparams=["u"], safety="unsafe", all_=["f", "g"])
    x.defn("g", x.prop(), x.const("f", [0]), safety="unsafe", all_=["f", "g"])


@case("good", "quotient-package", "check.rs: add_quot; the quotient's computation rule in every engine",
      "`Eq`, then `Quot`, `Quot.mk`, `Quot.lift` and `Quot.ind` by their own records with their canonical types, "
      "`quotLiftReduction : Quot.lift f h (Quot.mk r a) = f a` by `Eq.refl` (it checks only where the lift "
      "computes) and `inductive T : Prop | T.intro`. The control every quotient case below differs from.")
def quotient_package_control(x):
    quotient_package(x)


@case("bad", "quotient-ctor-as-axiom", "check.rs: check_name (a begun quotient owns its names)",
      "`Quot.mk` an ordinary axiom with the canonical type, in place of its record: once `Quot` is declared the "
      "quotient's four names are its own, as Lean's kernel declares all four at once. Admitted, it would let "
      "the lift compute on whatever the axiom builds (Arena `bugs/axiom-quot-mk`).",
      rejected_by="'Quot.mk' is the quotient's constructor")
def quotient_ctor_as_axiom(x):
    quotient_package(x, as_axiom=("ctor",))


@case("bad", "quotient-lift-as-axiom", "check.rs: check_name (a begun quotient owns its names)",
      "`Quot.lift` an ordinary axiom with the canonical type, in place of its record.",
      rejected_by="'Quot.lift' is the quotient's lift")
def quotient_lift_as_axiom(x):
    quotient_package(x, as_axiom=("lift",))


@case("bad", "quotient-ind-as-axiom", "check.rs: check_name (a begun quotient owns its names)",
      "`Quot.ind` an ordinary axiom with the canonical type, in place of its record.",
      rejected_by="'Quot.ind' is the quotient's induction principle")
def quotient_ind_as_axiom(x):
    quotient_package(x, as_axiom=("ind",))


@case("bad", "quotient-type-as-axiom", "check.rs: add_quot (each member declared is the quotient's own)",
      "`Quot` an ordinary axiom with the canonical type, in place of its record: no quotient has begun, so the "
      "axiom is admitted, and `Quot.mk`'s record then finds the name taken by a constant that is not the "
      "quotient's type.",
      rejected_by="'Quot' is declared, but not as the quotient's type")
def quotient_type_as_axiom(x):
    quotient_package(x, as_axiom=("type",))


@case("bad", "quotient-type-as-definition", "check.rs: add_quot (each member declared is the quotient's own)",
      "`def Quot {α : Sort u} (r : α → α → Prop) : Sort u := α`, then the other three records: read over this "
      "definition `Quot.ind` would cover every inhabitant of `α` (Arena `bugs/forged-quot-type`).",
      rejected_by="'Quot' is declared, but not as the quotient's type")
def quotient_type_as_definition(x):
    eq_family(x)
    u = x.lparam("u")
    x.defn("Quot", quot_type(x, "type"), x.lam(x.sort(u), x.lam(relation(x, 0), x.bvar(1), "r"), "α"),
           lparams=["u"])
    for kind in ("ctor", "lift", "ind"):
        quotient_constant(x, kind)
    lift_computes(x)
    unit_proposition(x, "T", "T.intro")


@case("bad", "quotient-ctor-declared-first", "check.rs: add_quot (each member declared is the quotient's own)",
      "`axiom Quot.mk : Prop` before any quotient record, then the package: an ordinary constant may take a "
      "quotient's name while no quotient is declared (see `quotient-names-without-a-quotient`), but the quotient "
      "cannot then be declared over it.",
      rejected_by="'Quot.mk' is declared, but not as the quotient's constructor")
def quotient_ctor_declared_first(x):
    eq_family(x)
    x.axiom("Quot.mk", x.prop())
    for kind in ("type", "lift", "ind"):
        quotient_constant(x, kind)


@case("good", "quotient-names-without-a-quotient", "check.rs: check_name (only a begun quotient owns its names)",
      "`Eq` and `axiom Quot.mk : Prop`, with no quotient record anywhere: Lean's kernel admits an ordinary "
      "constant of that name while the quotient is not declared, and so does this one.")
def quotient_names_without_a_quotient(x):
    eq_family(x)
    x.axiom("Quot.mk", x.prop())


@case("bad", "quotient-ind-before-ctor", "check.rs: add_quot (the constants a canonical type names come first)",
      "The package with `Quot.ind`'s record before `Quot.mk`'s: the induction principle's type names the "
      "constructor, which is not declared yet.",
      rejected_by="quotient constant declared before 'Quot.mk'")
def quotient_ind_before_ctor(x):
    quotient_package(x, order=("type", "ind", "ctor", "lift"))


@case("bad", "quotient-ctor-named-by-a-constructor", "inductive.rs: check_name on constructors",
      "After `Quot`'s record, `inductive T : Prop | Quot.mk : T`: a constructor of another inductive may not take "
      "the quotient constructor's name.",
      rejected_by="'Quot.mk' is the quotient's constructor")
def quotient_ctor_named_by_a_constructor(x):
    eq_family(x)
    quotient_constant(x, "type")
    unit_proposition(x, "T", "Quot.mk")


@case("bad", "quotient-ind-named-by-an-inductive", "inductive.rs: check_name on inductive types",
      "After the records of `Quot`, `Quot.mk` and `Quot.lift`, `inductive Quot.ind : Prop | Quot.ind.intro`: an "
      "inductive type may not take the induction principle's name.",
      rejected_by="'Quot.ind' is the quotient's induction principle")
def quotient_ind_named_by_an_inductive(x):
    eq_family(x)
    for kind in ("type", "ctor", "lift"):
        quotient_constant(x, kind)
    unit_proposition(x, "Quot.ind", "Quot.ind.intro")


@case("bad", "quotient-lift-named-in-a-mutual-block", "check.rs: check_name on every member of a mutual block",
      "After `Quot`'s and `Quot.mk`'s records, the mutual unsafe pair `f : Prop := Quot.lift` and "
      "`Quot.lift : Prop := f` (`mutual-unsafe-pair`'s shape): no member of a block, the first or a later one, "
      "may take the lift's name.",
      rejected_by="'Quot.lift' is the quotient's lift")
def quotient_lift_named_in_a_mutual_block(x):
    eq_family(x)
    for kind in ("type", "ctor"):
        quotient_constant(x, kind)
    f, lift = x.const("f"), x.const("Quot.lift")
    x.defn("f", x.prop(), lift, safety="unsafe", all_=["f", "Quot.lift"])
    x.defn("Quot.lift", x.prop(), f, safety="unsafe", all_=["f", "Quot.lift"])


def main():
    out = OUT
    if sys.argv[1:2] == ["--out"] and len(sys.argv) == 3:
        out = sys.argv[2]
    elif len(sys.argv) > 1:
        sys.exit("usage: craft-cases.py [--out DIR]")
    manifest = []
    for outcome, name, targets, description, rejected_by, build in CASES:
        x = Export()
        build(x)
        path = os.path.join(out, outcome, name + ".ndjson")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        open(path, "w").write(x.text())
        manifest.append({"name": name, "outcome": outcome, "targets": targets, "description": description,
                         "path": os.path.relpath(path, out), "rejected_by": rejected_by})
    json.dump({"schema": "logos-crafted-cases/1", "cases": manifest}, open(os.path.join(out, "manifest.json"), "w"),
              indent=1, sort_keys=True)
    print(f"{len(manifest)} crafted cases")


if __name__ == "__main__":
    main()
