#!/usr/bin/env python3
"""gen_qwen_tiny_lg.py — emit a FLOAT, Qwen3-shaped tiny transformer forward pass in LOGOS.

This is the Phase-0 beachhead of the "Qwen authored in LOGOS" campaign: the FIRST *float* transformer
written in LOGOS `.lg`. It is a faithful structural shrink of the real Qwen3 forward
(`crates/logicaffeine_compile/src/codegen_cuda/qwen.rs`): RMSNorm, per-head QK-RMSNorm, RoPE-NEOX,
grouped-query causal attention with a real (exp) softmax, SwiGLU FFN, residuals, final norm + LM head,
argmax next token — every stage in `Real` (f64) arithmetic, across 2 layers.

Unlike the int8 `gen_tiny_transformer_lg.py`, everything is floating point. Two invariants make the
`.lg`-on-VM run BYTE-EXACT against the Rust oracle in `tests/qwen_tiny_lg_vm_lock.rs`:
  • the only transcendental is `expT` (a 20-term Taylor series identical to `assets/std/math.lg`'s `exp`)
    and the builtin `sqrt` (IEEE `f64::sqrt`) — the oracle mirrors both exactly;
  • RoPE cos/sin are precomputed HOST-SIDE and inlined as data, so both sides read identical rotations
    (dodging the range-reduction gap in Taylor sin/cos for large angles).

The deterministic weights + config live in `benchmarks/silicon/qwen_tiny.json` (`--emit-json`). Float
literals are emitted WITHOUT scientific notation (the LOGOS lexer takes `0.000001`, not `1e-06`) at full
round-trip precision, so a literal and its JSON counterpart denote the identical f64.

Modes:
  --emit-json          write the model JSON to stdout (deterministic; seeded)
  --toks a,b,c         emit `## Main` running one forward over those token ids, Show the argmax token id
  --toks a,b,c --logits    ... Show every last-position logit instead (the byte-exact target)
"""
import argparse
import json
import math
import random
import sys

JSON_PATH = "benchmarks/silicon/qwen_tiny.json"


# ── float literal: full round-trip precision, never scientific notation ────────────────────────────
def flit(x):
    x = float(x)
    s = repr(x)
    if "e" in s or "E" in s:
        s = format(x, ".30f")
        if "." in s:
            s = s.rstrip("0")
            if s.endswith("."):
                s += "0"
    if s in ("-0.0",):
        s = "0.0"
    if "." not in s and "e" not in s:
        s += ".0"
    return s


# ── deterministic tiny Qwen3-shaped model ──────────────────────────────────────────────────────────
def build_model():
    cfg = dict(d_model=16, n_layers=2, n_heads=2, n_kv_heads=1, head_dim=8, d_ff=32,
               vocab=16, eps=1e-6, rope_base=10000.0, max_seq=8)
    D, H, KV, hd = cfg["d_model"], cfg["n_heads"], cfg["n_kv_heads"], cfg["head_dim"]
    dff, V, L = cfg["d_ff"], cfg["vocab"], cfg["n_layers"]
    kvd = KV * hd
    rng = random.Random(0xC0FFEE)

    def mat(rows, cols, sc):
        # row-major [rows, cols], small uniform init so exp/silu arguments stay Taylor-friendly
        return [round(rng.uniform(-sc, sc), 6) for _ in range(rows * cols)]

    def norm_w(n):
        # RMSNorm gains near 1.0
        return [round(1.0 + rng.uniform(-0.1, 0.1), 6) for _ in range(n)]

    layers = []
    for _ in range(L):
        layers.append(dict(
            attn_norm=norm_w(D), q_norm=norm_w(hd), k_norm=norm_w(hd), ffn_norm=norm_w(D),
            wq=mat(D, D, 0.25), wk=mat(kvd, D, 0.25), wv=mat(kvd, D, 0.25), wo=mat(D, D, 0.25),
            gate=mat(dff, D, 0.25), up=mat(dff, D, 0.25), down=mat(D, dff, 0.25),
        ))
    weights = dict(
        token_embd=mat(V, D, 0.5), output_norm=norm_w(D), output=mat(V, D, 0.25), layers=layers,
    )
    half = hd // 2
    cos_t, sin_t = [], []
    for pos in range(cfg["max_seq"]):
        for i in range(half):
            freq = cfg["rope_base"] ** (-2.0 * i / hd)
            ang = pos * freq
            cos_t.append(round(math.cos(ang), 9))
            sin_t.append(round(math.sin(ang), 9))
    rope = dict(cos=cos_t, sin=sin_t, half=half)
    return dict(config=cfg, weights=weights, rope=rope, prompt=[1, 2, 3], gen_len=3)


# ── LOGOS emission ─────────────────────────────────────────────────────────────────────────────────
def emit_fseq(name, values):
    out = [f"Let {name} be a new Seq of Float."]
    for v in values:
        out.append(f"Push {flit(v)} to {name}.")
    return "\n".join(out)


HELPERS = (
    "## To expT (x: Real) -> Real:\n"
    "    Let mutable term be 1.0.\n"
    "    Let mutable result be 1.0.\n"
    "    Let mutable k be 1.0.\n"
    "    While k is at most 20.0:\n"
    "        Set term to (term * x) / k.\n"
    "        Set result to result + term.\n"
    "        Set k to k + 1.0.\n"
    "    Return result.\n"
    "## To siluT (x: Real) -> Real:\n"
    "    Return x / (1.0 + expT(0.0 - x)).\n"
)


def emit_forward(j, toks, show_logits):
    cfg = j["config"]
    D, H, KV, hd = cfg["d_model"], cfg["n_heads"], cfg["n_kv_heads"], cfg["head_dim"]
    dff, V, L, eps = cfg["d_ff"], cfg["vocab"], cfg["n_layers"], cfg["eps"]
    half = j["rope"]["half"]
    kvd = KV * hd
    group = H // KV
    n = len(toks)
    w = j["weights"]
    S = []
    S.append(HELPERS)
    S.append("## Main")
    # ── weight tables (inlined float literals) ──────────────────────────────────
    S.append(emit_fseq("EMB", w["token_embd"]))
    S.append(emit_fseq("ONORM", w["output_norm"]))
    S.append(emit_fseq("OUTP", w["output"]))
    S.append(emit_fseq("RC", j["rope"]["cos"]))
    S.append(emit_fseq("RS", j["rope"]["sin"]))
    for l, ly in enumerate(w["layers"]):
        for key in ("attn_norm", "q_norm", "k_norm", "ffn_norm", "wq", "wk", "wv", "wo", "gate", "up", "down"):
            S.append(emit_fseq(f"{key}_{l}", ly[key]))
    S.append(emit_fseq("toks", [float(t) for t in toks]))
    # dims / constants
    S.append(f"Let n be {n}.")
    S.append(f"Let ascale be 1.0 / sqrt({flit(hd)}).")
    S.append(f"Let eps be {flit(eps)}.")
    # ── embedding: x[pos][j] = EMB[tok*D + j] ───────────────────────────────────
    S.append("Let x be a new Seq of Float.")
    S.append("Let p be 0.")
    S.append("While p is less than n:")
    S.append("    Let tok be item (p + 1) of toks.")
    S.append("    Let tokd be floor(tok) * " + str(D) + ".")
    S.append("    Let jj be 0.")
    S.append(f"    While jj is less than {D}:")
    S.append("        Push item (tokd + jj + 1) of EMB to x.")
    S.append("        Set jj to jj + 1.")
    S.append("    Set p to p + 1.")

    def rmsnorm(dst, src, base_expr, wname, dim):
        # dst (Seq) gets the RMSNorm of the `dim`-vector at src[base_expr .. base_expr+dim] with gain `wname`
        return [
            f"Let ss be 0.0.",
            f"Let ri be 0.",
            f"While ri is less than {dim}:",
            f"    Let rv be item ({base_expr} + ri + 1) of {src}.",
            f"    Set ss to ss + rv * rv.",
            f"    Set ri to ri + 1.",
            f"Let rscale be 1.0 / sqrt((ss / {flit(dim)}) + eps).",
            f"Set ri to 0.",
            f"While ri is less than {dim}:",
            f"    Let rv be item ({base_expr} + ri + 1) of {src}.",
            f"    Push (rv * rscale) * (item (ri + 1) of {wname}) to {dst}.",
            f"    Set ri to ri + 1.",
        ]

    def matvec(dst, wname, out_dim, in_dim, xsrc, xbase_expr, clamp_relu=False):
        # dst gets y[o] = Σ_i W[o*in_dim + i] * xsrc[xbase_expr + i], o in 0..out_dim (row-major [out,in])
        body = [
            f"Let mo be 0.",
            f"While mo is less than {out_dim}:",
            f"    Let acc be 0.0.",
            f"    Let mi be 0.",
            f"    While mi is less than {in_dim}:",
            f"        Set acc to acc + (item (mo * {in_dim} + mi + 1) of {wname}) * (item ({xbase_expr} + mi + 1) of {xsrc}).",
            f"        Set mi to mi + 1.",
            f"    Push acc to {dst}.",
            f"    Set mo to mo + 1.",
        ]
        return body

    for l in range(L):
        an, qn, kn, fn = f"attn_norm_{l}", f"q_norm_{l}", f"k_norm_{l}", f"ffn_norm_{l}"
        wq, wk, wv, wo = f"wq_{l}", f"wk_{l}", f"wv_{l}", f"wo_{l}"
        gate, up, down = f"gate_{l}", f"up_{l}", f"down_{l}"
        # xn[pos] = rmsnorm(x[pos], attn_norm)  → flat Seq of n*D
        S.append("Let xn be a new Seq of Float.")
        S.append("Set p to 0.")
        S.append("While p is less than n:")
        S += ["    " + ln for ln in rmsnorm("xn", "x", f"p * {D}", an, D)]
        S.append("    Set p to p + 1.")
        # Q/K/V projections (flat: Q is n*D, K/V are n*kvd)
        S.append("Let Q be a new Seq of Float.")
        S.append("Set p to 0.")
        S.append("While p is less than n:")
        S += ["    " + ln for ln in matvec("Q", wq, D, D, "xn", f"p * {D}")]
        S.append("    Set p to p + 1.")
        for dst, wn in (("K", wk), ("V", wv)):
            S.append(f"Let {dst} be a new Seq of Float.")
            S.append("Set p to 0.")
            S.append("While p is less than n:")
            S += ["    " + ln for ln in matvec(dst, wn, kvd, D, "xn", f"p * {D}")]
            S.append("    Set p to p + 1.")
        # per-head QK-RMSNorm + RoPE (in place on Q and K)
        #   Q head h at position p occupies Q[p*D + h*hd .. +hd]; K head hk at Kp*kvd + hk*hd
        for (seq, nh, rowstride, wnorm) in (("Q", H, D, qn), ("K", KV, kvd, kn)):
            S.append("Set p to 0.")
            S.append("While p is less than n:")
            S.append(f"    Let hh be 0.")
            S.append(f"    While hh is less than {nh}:")
            S.append(f"        Let hb be p * {rowstride} + hh * {hd}.")
            # qk-rmsnorm the head slice in place
            S.append("        Let ss be 0.0.")
            S.append("        Let ri be 0.")
            S.append(f"        While ri is less than {hd}:")
            S.append(f"            Let rv be item (hb + ri + 1) of {seq}.")
            S.append("            Set ss to ss + rv * rv.")
            S.append("            Set ri to ri + 1.")
            S.append(f"        Let rscale be 1.0 / sqrt((ss / {flit(hd)}) + eps).")
            S.append("        Set ri to 0.")
            S.append(f"        While ri is less than {hd}:")
            S.append(f"            Let rv be item (hb + ri + 1) of {seq}.")
            S.append(f"            Set item (hb + ri + 1) of {seq} to (rv * rscale) * (item (ri + 1) of {wnorm}).")
            S.append("            Set ri to ri + 1.")
            # RoPE-NEOX on the head slice using precomputed cos/sin at (pos=p, i)
            S.append(f"        Let rb be p * {half}.")
            S.append("        Let ci be 0.")
            S.append(f"        While ci is less than {half}:")
            S.append("            Let cc be item (rb + ci + 1) of RC.")
            S.append("            Let sinv be item (rb + ci + 1) of RS.")
            S.append(f"            Let x0 be item (hb + ci + 1) of {seq}.")
            S.append(f"            Let x1 be item (hb + ci + {half} + 1) of {seq}.")
            S.append(f"            Set item (hb + ci + 1) of {seq} to x0 * cc - x1 * sinv.")
            S.append(f"            Set item (hb + ci + {half} + 1) of {seq} to x0 * sinv + x1 * cc.")
            S.append("            Set ci to ci + 1.")
            S.append("        Set hh to hh + 1.")
            S.append("    Set p to p + 1.")
        # GQA causal attention → attn (flat n*D), then wo projection + residual
        S.append("Let attn be a new Seq of Float.")
        S.append("Let z be 0.")
        S.append("While z is less than n * " + str(D) + ":")
        S.append("    Push 0.0 to attn.")
        S.append("    Set z to z + 1.")
        S.append("Let head be 0.")
        S.append(f"While head is less than {H}:")
        S.append(f"    Let kvh be head / {group}.")
        S.append("    Let qi be 0.")
        S.append("    While qi is less than n:")
        S.append("        Let scores be a new Seq of Float.")
        S.append("        Let kj be 0.")
        S.append("        While kj is at most qi:")
        S.append("            Let d be 0.0.")
        S.append("            Let e be 0.")
        S.append(f"            While e is less than {hd}:")
        S.append(f"                Set d to d + (item (qi * {D} + head * {hd} + e + 1) of Q) * (item (kj * {kvd} + kvh * {hd} + e + 1) of K).")
        S.append("                Set e to e + 1.")
        S.append("            Push d * ascale to scores.")
        S.append("            Set kj to kj + 1.")
        # softmax over scores[0..qi]
        S.append("        Let mx be item 1 of scores.")
        S.append("        Let si be 2.")
        S.append("        While si is at most (qi + 1):")
        S.append("            If item si of scores is greater than mx:")
        S.append("                Set mx to item si of scores.")
        S.append("            Set si to si + 1.")
        S.append("        Let ex be a new Seq of Float.")
        S.append("        Let den be 0.0.")
        S.append("        Set si to 0.")
        S.append("        While si is at most qi:")
        S.append("            Let ev be expT((item (si + 1) of scores) - mx).")
        S.append("            Push ev to ex.")
        S.append("            Set den to den + ev.")
        S.append("            Set si to si + 1.")
        # weighted sum of V into attn[qi head slice]
        S.append("        Let e be 0.")
        S.append(f"        While e is less than {hd}:")
        S.append("            Let acc be 0.0.")
        S.append("            Set si to 0.")
        S.append("            While si is at most qi:")
        S.append(f"                Set acc to acc + (item (si + 1) of ex) * (item (si * {kvd} + kvh * {hd} + e + 1) of V).")
        S.append("                Set si to si + 1.")
        S.append(f"            Set item (qi * {D} + head * {hd} + e + 1) of attn to acc / den.")
        S.append("            Set e to e + 1.")
        S.append("        Set qi to qi + 1.")
        S.append("    Set head to head + 1.")
        # ao = wo @ attn ; x += ao
        S.append("Let ao be a new Seq of Float.")
        S.append("Set p to 0.")
        S.append("While p is less than n:")
        S += ["    " + ln for ln in matvec("ao", wo, D, D, "attn", f"p * {D}")]
        S.append("    Set p to p + 1.")
        S.append("Set p to 0.")
        S.append(f"While p is less than n * {D}:")
        S.append("    Set item (p + 1) of x to (item (p + 1) of x) + (item (p + 1) of ao).")
        S.append("    Set p to p + 1.")
        # SwiGLU FFN + residual
        S.append("Let xn2 be a new Seq of Float.")
        S.append("Set p to 0.")
        S.append("While p is less than n:")
        S += ["    " + ln for ln in rmsnorm("xn2", "x", f"p * {D}", fn, D)]
        S.append("    Set p to p + 1.")
        S.append("Let hid be a new Seq of Float.")
        S.append("Set p to 0.")
        S.append("While p is less than n:")
        S.append("    Let gg be a new Seq of Float.")
        S += ["    " + ln for ln in matvec("gg", gate, dff, D, "xn2", f"p * {D}")]
        S.append("    Let uu be a new Seq of Float.")
        S += ["    " + ln for ln in matvec("uu", up, dff, D, "xn2", f"p * {D}")]
        S.append("    Let fi be 0.")
        S.append(f"    While fi is less than {dff}:")
        S.append("        Push siluT(item (fi + 1) of gg) * (item (fi + 1) of uu) to hid.")
        S.append("        Set fi to fi + 1.")
        S.append("    Set p to p + 1.")
        S.append("Let dn be a new Seq of Float.")
        S.append("Set p to 0.")
        S.append("While p is less than n:")
        S += ["    " + ln for ln in matvec("dn", down, D, dff, "hid", f"p * {dff}")]
        S.append("    Set p to p + 1.")
        S.append("Set p to 0.")
        S.append(f"While p is less than n * {D}:")
        S.append("    Set item (p + 1) of x to (item (p + 1) of x) + (item (p + 1) of dn).")
        S.append("    Set p to p + 1.")
    # final norm on last position + LM head
    S.append("Let xf be a new Seq of Float.")
    S += rmsnorm("xf", "x", f"(n - 1) * {D}", "ONORM", D)
    if show_logits:
        S.append("Let o be 0.")
        S.append(f"While o is less than {V}:")
        S.append("    Let acc be 0.0.")
        S.append("    Let i be 0.")
        S.append(f"    While i is less than {D}:")
        S.append(f"        Set acc to acc + (item (o * {D} + i + 1) of OUTP) * (item (i + 1) of xf).")
        S.append("        Set i to i + 1.")
        S.append("    Show acc.")
        S.append("    Set o to o + 1.")
    else:
        S.append("Let best be 0.0 - 1000000000.0.")
        S.append("Let bestIdx be 0.")
        S.append("Let o be 0.")
        S.append(f"While o is less than {V}:")
        S.append("    Let acc be 0.0.")
        S.append("    Let i be 0.")
        S.append(f"    While i is less than {D}:")
        S.append(f"        Set acc to acc + (item (o * {D} + i + 1) of OUTP) * (item (i + 1) of xf).")
        S.append("        Set i to i + 1.")
        S.append("    If acc is greater than best:")
        S.append("        Set best to acc.")
        S.append("        Set bestIdx to o.")
        S.append("    Set o to o + 1.")
        S.append("Show bestIdx.")
    return "\n".join(S) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--emit-json", action="store_true", help="write the deterministic model JSON to stdout")
    ap.add_argument("--json", default=JSON_PATH, help="model JSON to read for .lg emission")
    ap.add_argument("--toks", default="1,2,3", help="comma-separated token ids for the forward pass")
    ap.add_argument("--logits", action="store_true", help="Show every last-position logit (byte-exact target)")
    args = ap.parse_args()
    if args.emit_json:
        json.dump(build_model(), sys.stdout)
        return
    j = json.load(open(args.json))
    toks = [int(t) for t in args.toks.split(",") if t.strip() != ""]
    sys.stdout.write(emit_forward(j, toks, show_logits=args.logits))


if __name__ == "__main__":
    main()
