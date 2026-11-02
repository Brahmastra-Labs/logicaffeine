#!/usr/bin/env python3
"""A minimal valid export for one declaration: its dependency closure and nothing else.

    scripts/lean/arena-crown/closure-slice.py index <export.ndjson> <index.pickle>
    scripts/lean/arena-crown/closure-slice.py cut <export.ndjson> <index.pickle> <out-dir> <name>... [--keep-proofs]

`index` records, in one pass, the byte offset of every name, level, expression and declaration
record. `cut` takes, for each `<name>`, the transitive closure of the records its declaration needs
and writes them in their original order (so every id is still defined before its use), the meta
record first, to `<out-dir>/<name>.ndjson`.

A theorem the target only mentions is written as an axiom of the same type: a proof is never looked
at when checking another declaration, and dropping it keeps the closure to what the target's own
check reaches. Opaque constants likewise become axioms. Definitions keep their values (δ-reduction
needs them), inductive records stay whole, and the target itself is written exactly as exported.
A literal brings the declarations the kernel types it with (`Nat`; `String.ofList` and
`Char.ofNat`), and a quotient declaration brings `Eq` and the other three quotient constants —
quotient reduction is on only once all four are declared — though no expression names them.
The Nat, String and quotient primitives, and everything their definitions reach, keep every proof:
validating the well-founded primitives unfolds through proofs. `--keep-proofs` keeps every proof.
"""
import array
import json
import pickle
import re
import sys

DECL_KINDS = (b'{"thm":', b'{"def":', b'{"axiom":', b'{"opaque":', b'{"quot":', b'{"inductive":')
IE_TAIL = re.compile(rb'"ie":(\d+)\}\s*$')
IE_HEAD = re.compile(rb'^\{"ie":(\d+),')
IN_HEAD = re.compile(rb'^\{"in":(\d+),')
IL_HEAD = re.compile(rb'^\{"il":(\d+),')
NAME_FIELD = re.compile(rb'"name":(\d+)')
PRIMITIVES = {
    "Bool", "Bool.false", "Bool.true", "Nat", "Nat.zero", "Nat.succ", "Nat.add", "Nat.pred", "Nat.sub",
    "Nat.mul", "Nat.pow", "Nat.gcd", "Nat.mod", "Nat.div", "Nat.beq", "Nat.ble", "Nat.bitwise",
    "Nat.land", "Nat.lor", "Nat.xor", "Nat.shiftLeft", "Nat.shiftRight", "String.ofList", "Char.ofNat",
    "Quot", "Quot.mk", "Quot.lift", "Quot.ind",
}


def put(arr, i, v):
    if i >= len(arr):
        arr.extend([-1] * (i + 1 - len(arr) + (len(arr) >> 1)))
    arr[i] = v


def index(src, out):
    names, levels, exprs = array.array("q"), array.array("q"), array.array("q")
    decls = {}
    meta = None
    off = 0
    with open(src, "rb") as f:
        for line in f:
            head = line[:12]
            if head.startswith(b'{"in":'):
                put(names, int(IN_HEAD.match(line).group(1)), off)
            elif head.startswith(b'{"il":'):
                put(levels, int(IL_HEAD.match(line).group(1)), off)
            elif head.startswith(b'{"ie":'):
                put(exprs, int(IE_HEAD.match(line).group(1)), off)
            elif head.startswith((b'{"app":', b'{"forallE":', b'{"bvar":', b'{"const":')):
                put(exprs, int(IE_TAIL.search(line).group(1)), off)
            elif line.startswith(DECL_KINDS):
                if line.startswith(b'{"inductive":'):
                    d = json.loads(line)["inductive"]
                    for part in ("types", "ctors", "recs"):
                        for c in d[part]:
                            decls[c["name"]] = off
                else:
                    decls[int(NAME_FIELD.search(line).group(1))] = off
            elif head.startswith(b'{"meta":'):
                meta = off
            off += len(line)
    with open(out, "wb") as f:
        pickle.dump({"names": names, "levels": levels, "exprs": exprs, "decls": decls, "meta": meta}, f, protocol=5)
    print(f"indexed {src}: {len(decls)} declared names")


class Export:
    def __init__(self, src, ix):
        self.f = open(src, "rb")
        self.ix = ix
        self.dotted = {}

    def line(self, off):
        self.f.seek(off)
        return self.f.readline()

    def rec(self, off):
        return json.loads(self.line(off))

    def name(self, i):
        if i == 0:
            return ""
        if i not in self.dotted:
            r = self.rec(self.ix["names"][i])
            if "str" in r:
                pre, part = r["str"]["pre"], r["str"]["str"]
            else:
                pre, part = r["num"]["pre"], str(r["num"]["i"])
            p = self.name(pre)
            self.dotted[i] = f"{p}.{part}" if p else part
        return self.dotted[i]


def body_of(rec, id_key):
    (kind, body), = [(k, v) for k, v in rec.items() if k != id_key]
    return kind, body


def cut_all(src, ix_path, out_dir, targets, keep_proofs):
    with open(ix_path, "rb") as f:
        ix = pickle.load(f)
    ex = Export(src, ix)
    by_name = {ex.name(nid): nid for nid in ix["decls"]}
    for target in targets:
        if target not in by_name:
            print(f"no declaration named {target}", file=sys.stderr)
            continue
        cut(ex, ix, by_name, target, f"{out_dir}/{target}.ndjson", keep_proofs)


def cut(ex, ix, by_name, target, out, keep_proofs):
    target_off = ix["decls"][by_name[target]]

    want_names, want_levels = set(), set()
    seen_expr = {}     # expression id -> True once traversed with proofs kept, False once without
    decl_state = {}    # declaration offset -> (keeps its proof, its dependencies keep theirs)
    todo_decls, todo_exprs, todo_levels, todo_names = [], [], [], []

    def need_name(i):
        if i and i not in want_names:
            want_names.add(i)
            todo_names.append(i)

    def need_level(i):
        if i and i not in want_levels:
            want_levels.add(i)
            todo_levels.append(i)

    def need_expr(e, proofs):
        prev = seen_expr.get(e)
        if prev is None or (proofs and not prev):
            seen_expr[e] = proofs or bool(prev)
            todo_exprs.append((e, proofs))

    def need_decl(nid, proofs):
        off = ix["decls"].get(nid)
        if off is None:
            return
        primitive = ex.name(nid) in PRIMITIVES
        deps = keep_proofs or proofs or primitive
        keep = deps or off == target_off
        prev = decl_state.get(off, (False, False))
        new = (prev[0] or keep, prev[1] or deps)
        if off not in decl_state or new != prev:
            decl_state[off] = new
            todo_decls.append(off)

    implied = {
        "strVal": [by_name[n] for n in ("String.ofList", "Char.ofNat") if n in by_name],
        "natVal": [by_name[n] for n in ("Nat",) if n in by_name],
        "quot": [by_name[n] for n in ("Eq", "Quot", "Quot.mk", "Quot.lift", "Quot.ind") if n in by_name],
    }

    need_decl(by_name[target], False)
    while todo_decls or todo_exprs or todo_levels or todo_names:
        while todo_decls:
            off = todo_decls.pop()
            keep, deps = decl_state[off]
            kind, body = body_of(ex.rec(off), None)
            if kind == "inductive":
                for part in ("types", "ctors", "recs"):
                    for c in body[part]:
                        need_name(c["name"])
                        need_expr(c["type"], deps)
                        for n in c.get("levelParams", []) + c.get("all", []) + c.get("ctors", []):
                            need_name(n)
                        if isinstance(c.get("induct"), int):
                            need_name(c["induct"])
                        for r in c.get("rules", []):
                            need_name(r["ctor"])
                            need_expr(r["rhs"], deps)
                continue
            need_name(body["name"])
            for n in body.get("levelParams", []) + body.get("all", []):
                need_name(n)
            for d in implied.get(kind, []):
                need_decl(d, deps)
            need_expr(body["type"], deps)
            if "value" in body and (kind == "def" or keep):
                need_expr(body["value"], deps)
        while todo_exprs:
            e, proofs = todo_exprs.pop()
            kind, v = body_of(ex.rec(ix["exprs"][e]), "ie")
            if kind == "app":
                need_expr(v["fn"], proofs)
                need_expr(v["arg"], proofs)
            elif kind in ("forallE", "lam"):
                need_expr(v["type"], proofs)
                need_expr(v["body"], proofs)
                need_name(v["name"])
            elif kind == "letE":
                need_expr(v["type"], proofs)
                need_expr(v["value"], proofs)
                need_expr(v["body"], proofs)
                need_name(v["name"])
            elif kind == "const":
                need_name(v["name"])
                for u in v["us"]:
                    need_level(u)
                need_decl(v["name"], proofs)
            elif kind == "sort":
                need_level(v)
            elif kind == "proj":
                need_expr(v["struct"], proofs)
                need_name(v["typeName"])
                need_decl(v["typeName"], proofs)
            elif kind == "mdata":
                need_expr(v["expr"], proofs)
            for d in implied.get(kind, []):
                need_decl(d, proofs)
        while todo_levels:
            kind, v = body_of(ex.rec(ix["levels"][todo_levels.pop()]), "il")
            if kind == "succ":
                need_level(v)
            elif kind in ("max", "imax"):
                need_level(v[0])
                need_level(v[1])
            elif kind == "param":
                need_name(v)
        while todo_names:
            r = ex.rec(ix["names"][todo_names.pop()])
            need_name((r.get("str") or r.get("num"))["pre"])

    offsets = [ix["meta"]]
    offsets += [ix["names"][n] for n in want_names]
    offsets += [ix["levels"][l] for l in want_levels]
    offsets += [ix["exprs"][e] for e in seen_expr]
    offsets += list(decl_state)
    offsets.sort()
    axioms = written = 0
    with open(out, "wb") as o:
        for off in offsets:
            line = ex.line(off)
            if off in decl_state and not decl_state[off][0]:
                kind, body = body_of(json.loads(line), None)
                if kind in ("thm", "opaque"):
                    ax = {"isUnsafe": body.get("isUnsafe", False), "levelParams": body["levelParams"],
                          "name": body["name"], "type": body["type"]}
                    line = (json.dumps({"axiom": ax}, separators=(",", ":"), ensure_ascii=False) + "\n").encode()
                    axioms += 1
            o.write(line)
            written += len(line)
    print(f"{out}: {len(offsets)} records, {len(decl_state)} declarations ({axioms} as axioms), {written} bytes")


def main():
    a = sys.argv[1:]
    if len(a) == 3 and a[0] == "index":
        index(a[1], a[2])
    elif len(a) >= 5 and a[0] == "cut":
        keep = "--keep-proofs" in a
        names = [n for n in a[4:] if n != "--keep-proofs"]
        cut_all(a[1], a[2], a[3], names, keep)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
