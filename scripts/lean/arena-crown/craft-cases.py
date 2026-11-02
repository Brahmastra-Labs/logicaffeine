#!/usr/bin/env python3
"""Hand-built Arena-format exports that isolate one kernel rule each.

The mutation matrix (mutation-matrix.sh) reports kernel logic the Arena corpus never exercises
alone. Each case here is a `lean4export` 3.1.0 file whose ONLY defect (for a `bad/` case) is the
rule it targets: every other claim — recursor types, rules, flags — is exactly what the kernel
derives if that one rule were skipped. So the case is rejected by that rule or not at all, and a
mutant that disables the rule is caught.

Writes crates/logicaffeine_tests/tests/fixtures/crafted/{good,bad}/<name>.ndjson and manifest.json.
"""
import json, os

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


def main():
    manifest = []
    for outcome, name, targets, description, rejected_by, build in CASES:
        x = Export()
        build(x)
        path = os.path.join(OUT, outcome, name + ".ndjson")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        open(path, "w").write(x.text())
        manifest.append({"name": name, "outcome": outcome, "targets": targets, "description": description,
                         "path": os.path.relpath(path, OUT), "rejected_by": rejected_by})
    json.dump({"schema": "logos-crafted-cases/1", "cases": manifest}, open(os.path.join(OUT, "manifest.json"), "w"),
              indent=1, sort_keys=True)
    print(f"{len(manifest)} crafted cases")


if __name__ == "__main__":
    main()
