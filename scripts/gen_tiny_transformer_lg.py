#!/usr/bin/env python3
"""gen_tiny_transformer_lg.py — emit the LOGOS forward pass for the baby transformer.

Reads benchmarks/silicon/tiny_transformer.json and emits a LOGOS program that reproduces the int8
integer forward pass (a faithful translation of the Rust/numpy reference) so it runs byte-identically on
the VM / tree-walk / soft-core. Two variants:
  --step N      : one forward pass over the prompt + the first N golden tokens, Show the argmax token id
  (default)     : autoregressive generation of gen_len tokens, Show each token id (the board program)

Weights are inlined as `Push <lit>` (fine on the VM; the on-chip build streams them via the data loader).
"""
import argparse
import json
import sys

SHIFT = 24


def load(path):
    return json.load(open(path))


def emit_helpers():
    return (
        "## To to8 (x: Int, m: Int) -> Int:\n"
        f"    Let y be (x * m) shifted right by {SHIFT}.\n"
        "    If y is greater than 127:\n"
        "        Return 127.\n"
        "    If y is less than -128:\n"
        "        Return -128.\n"
        "    Return y.\n"
        "## To rqf (x: Int, m: Int) -> Int:\n"
        f"    Return (x * m) shifted right by {SHIFT}.\n"
    )


def emit_seq(name, values):
    out = [f"Let {name} be a new Seq of Int."]
    for v in values:
        out.append(f"Push {int(v)} to {name}.")
    return "\n".join(out)


# The ORDER weight arrays are concatenated into the DATA-segment blob the loader streams into the heap.
# Layout: heap[0]=total_len, heap[1..]=concatenated arrays. `item k of loaded()` = heap[k].
BLOB_ORDER = ["Wq", "Wk", "Wv", "Wo", "W1", "W2", "Eq", "EinA", "PinA", "explut"]


def weight_arrays(j):
    w = j["weights_int8"]; es = j["embed_stream"]
    return {"Wq": w["Wq"], "Wk": w["Wk"], "Wv": w["Wv"], "Wo": w["Wo"], "W1": w["W1"], "W2": w["W2"],
            "Eq": w["Eq"], "EinA": es["EinA"], "PinA": es["PinA"], "explut": j["exp_lut"]}


def weight_blob(j):
    """The int blob the SoC preloads as a `Seq of Int` at heap ptr 0: [total_len, ...all arrays...].
    Returns (blob, offsets) where offsets[name] is the 0-based start of `name` within the concatenation
    (so `name` element t (1-based) = `item (offsets[name] + t) of loaded()` = heap[offsets[name]+t])."""
    arrs = weight_arrays(j)
    blob_vals, offsets, off = [], {}, 0
    for nm in BLOB_ORDER:
        offsets[nm] = off
        blob_vals.extend(int(x) for x in arrs[nm])
        off += len(arrs[nm])
    return [off] + blob_vals, offsets


def emit_copy(name, off, length):
    """Board mode: build weight Seq `name` by COPYING from the preloaded blob via a loop (register-cheap,
    unlike inlining `length` literals which blows the 256-register chip cap). Body then reads `item t of name`
    unchanged — the proven forward-pass body is untouched; only the weight-load preamble differs."""
    return (
        f"Let {name} be a new Seq of Int.\n"
        f"Let _t be 1.\n"
        f"While _t is at most {length}:\n"
        f"    Push item ({off} + _t) of loaded() to {name}.\n"
        f"    Set _t to _t + 1."
    )


def emit_forward(j, toks, show_logits=False, board=False):
    """Emit `## Main` computing one forward pass over `toks`. Show the argmax token id, or (show_logits)
    Show every last-position logit in order — the byte-exact target for the golden trace.

    `board=False` inlines the weights as literals (VM/model — no register cap). `board=True` declares
    `loaded()` and builds each weight Seq by copying from the preloaded blob (the on-chip path, register-
    cheap). The forward-pass body below is IDENTICAL in both modes."""
    cfg = j["config"]
    d = cfg["d_model"]; H = cfg["n_heads"]; hd = d // H; dff = cfg["d_ff"]; V = cfg["vocab"]
    rq = j["requant"]
    RQKV = rq["r_qkv"][0]; RFFN = rq["r_ffn"][0]; ROUT = rq["r_out"][0]; RCTX = rq["r_ctx"][0]; RHID = rq["r_hid"][0]
    MQ = rq["M_q"][0]; MK = rq["M_k"][0]; MV = rq["M_v"][0]; MO = rq["M_o"][0]; MH = rq["M_h"][0]; MF = rq["M_f"][0]
    MSCORE = rq["M_score"][0]
    n = len(toks)
    arrs = weight_arrays(j)
    L = []
    L.append(emit_helpers())
    if board:
        L.append("## To native loaded () -> Seq of Int")
    L.append("## Main")
    # weight tables: inline literals (VM/model) OR copy from the preloaded blob via loaded() (board/chip)
    if board:
        _, offsets = weight_blob(j)
        for nm in BLOB_ORDER:
            L.append(emit_copy(nm, offsets[nm], len(arrs[nm])))
    else:
        for nm in BLOB_ORDER:
            L.append(emit_seq(nm, arrs[nm]))
    L.append(emit_seq("toks", toks))
    # dims as constants
    L.append(f"Let n be {n}.")
    L.append(f"Let d be {d}.")
    L.append(f"Let hd be {hd}.")
    L.append(f"Let dff be {dff}.")
    L.append(f"Let V be {V}.")
    # ── stream = EinA[tok] + PinA[pos] ─────────────────────────────────────────
    L.append("Let stream be a new Seq of Int.")
    L.append("Let p be 0.")
    L.append("While p is less than n:")
    L.append("    Let tok be item (p + 1) of toks.")
    L.append("    Let j be 0.")
    L.append("    While j is less than d:")
    L.append("        Push (item (tok * d + j + 1) of EinA) + (item (p * d + j + 1) of PinA) to stream.")
    L.append("        Set j to j + 1.")
    L.append("    Set p to p + 1.")
    # ── Xi = to8(stream, RQKV) ─────────────────────────────────────────────────
    L.append("Let Xi be a new Seq of Int.")
    L.append("Repeat for sv in stream:")
    L.append(f"    Push to8(sv, {RQKV}) to Xi.")
    # ── Q,K,Vv = to8(Xi @ W, M) ───────────────────────────────────────────────
    for out, wn, MM in [("Q", "Wq", MQ), ("K", "Wk", MK), ("Vv", "Wv", MV)]:
        L.append(f"Let {out} be a new Seq of Int.")
        L.append("Set p to 0.")
        L.append("While p is less than n:")
        L.append("    Let o be 0.")
        L.append("    While o is less than d:")
        L.append("        Let acc be 0.")
        L.append("        Let i be 0.")
        L.append("        While i is less than d:")
        L.append(f"            Set acc to acc + (item (p * d + i + 1) of Xi) * (item (i * d + o + 1) of {wn}).")
        L.append("            Set i to i + 1.")
        L.append(f"        Push to8(acc, {MM}) to {out}.")
        L.append("        Set o to o + 1.")
        L.append("    Set p to p + 1.")
    # ── ctx (preallocated zeros) ──────────────────────────────────────────────
    L.append("Let ctx be a new Seq of Int.")
    L.append("Let z be 0.")
    L.append("While z is less than n * d:")
    L.append("    Push 0 to ctx.")
    L.append("    Set z to z + 1.")
    # ── attention per head/query ──────────────────────────────────────────────
    L.append("Let head be 0.")
    L.append("While head is less than " + str(H) + ":")
    L.append("    Let lo be head * hd.")
    L.append("    Let i be 0.")
    L.append("    While i is less than n:")
    L.append("        Let sc be a new Seq of Int.")
    L.append("        Let j be 0.")
    L.append("        While j is at most i:")
    L.append("            Let a be 0.")
    L.append("            Let c be 0.")
    L.append("            While c is less than hd:")
    L.append("                Set a to a + (item (i * d + lo + c + 1) of Q) * (item (j * d + lo + c + 1) of K).")
    L.append("                Set c to c + 1.")
    L.append("            Push a to sc.")
    L.append("            Set j to j + 1.")
    L.append("        Let mx be item 1 of sc.")
    L.append("        Set j to 2.")
    L.append("        While j is at most (i + 1):")
    L.append("            If item j of sc is greater than mx:")
    L.append("                Set mx to item j of sc.")
    L.append("            Set j to j + 1.")
    L.append("        Let num be a new Seq of Int.")
    L.append("        Let c be 0.")
    L.append("        While c is less than hd:")
    L.append("            Push 0 to num.")
    L.append("            Set c to c + 1.")
    L.append("        Let den be 0.")
    L.append("        Set j to 0.")
    L.append("        While j is at most i:")
    L.append("            Let dfc be mx - (item (j + 1) of sc).")
    L.append(f"            Let idx be (dfc * {MSCORE}) shifted right by {SHIFT}.")
    L.append("            If idx is less than 0:")
    L.append("                Set idx to 0.")
    L.append("            If idx is greater than 255:")
    L.append("                Set idx to 255.")
    L.append("            Let wgt be item (idx + 1) of explut.")
    L.append("            Set c to 0.")
    L.append("            While c is less than hd:")
    L.append("                Set item (c + 1) of num to (item (c + 1) of num) + wgt * (item (j * d + lo + c + 1) of Vv).")
    L.append("                Set c to c + 1.")
    L.append("            Set den to den + wgt.")
    L.append("            Set j to j + 1.")
    L.append("        Set c to 0.")
    L.append("        While c is less than hd:")
    L.append("            Set item (i * d + lo + c + 1) of ctx to (item (c + 1) of num) // den.")
    L.append("            Set c to c + 1.")
    L.append("        Set i to i + 1.")
    L.append("    Set head to head + 1.")
    # ── Ci = to8(ctx, RCTX); attn_out = rqf(Ci @ Wo, MO); stream += attn_out ───
    L.append("Let Ci be a new Seq of Int.")
    L.append("Repeat for cv in ctx:")
    L.append(f"    Push to8(cv, {RCTX}) to Ci.")
    L.append("Set p to 0.")
    L.append("While p is less than n:")
    L.append("    Let o be 0.")
    L.append("    While o is less than d:")
    L.append("        Let acc be 0.")
    L.append("        Let i be 0.")
    L.append("        While i is less than d:")
    L.append("            Set acc to acc + (item (p * d + i + 1) of Ci) * (item (i * d + o + 1) of Wo).")
    L.append("            Set i to i + 1.")
    L.append(f"        Set item (p * d + o + 1) of stream to (item (p * d + o + 1) of stream) + rqf(acc, {MO}).")
    L.append("        Set o to o + 1.")
    L.append("    Set p to p + 1.")
    # ── FFN: Hi=to8(stream,RFFN); hid=relu(rqf(Hi@W1,MH)); Fi=to8(hid,RHID); stream += rqf(Fi@W2,MF)
    L.append("Let Hi be a new Seq of Int.")
    L.append("Repeat for hv in stream:")
    L.append(f"    Push to8(hv, {RFFN}) to Hi.")
    L.append("Let hid be a new Seq of Int.")
    L.append("Set p to 0.")
    L.append("While p is less than n:")
    L.append("    Let o be 0.")
    L.append("    While o is less than dff:")
    L.append("        Let acc be 0.")
    L.append("        Let i be 0.")
    L.append("        While i is less than d:")
    L.append("            Set acc to acc + (item (p * d + i + 1) of Hi) * (item (i * dff + o + 1) of W1).")
    L.append("            Set i to i + 1.")
    L.append(f"        Let hv2 be rqf(acc, {MH}).")
    L.append("        If hv2 is less than 0:")
    L.append("            Set hv2 to 0.")
    L.append("        Push hv2 to hid.")
    L.append("        Set o to o + 1.")
    L.append("    Set p to p + 1.")
    L.append("Let Fi be a new Seq of Int.")
    L.append("Repeat for fv in hid:")
    L.append(f"    Push to8(fv, {RHID}) to Fi.")
    L.append("Set p to 0.")
    L.append("While p is less than n:")
    L.append("    Let o be 0.")
    L.append("    While o is less than d:")
    L.append("        Let acc be 0.")
    L.append("        Let i be 0.")
    L.append("        While i is less than dff:")
    L.append("            Set acc to acc + (item (p * dff + i + 1) of Fi) * (item (i * d + o + 1) of W2).")
    L.append("            Set i to i + 1.")
    L.append(f"        Set item (p * d + o + 1) of stream to (item (p * d + o + 1) of stream) + rqf(acc, {MF}).")
    L.append("        Set o to o + 1.")
    L.append("    Set p to p + 1.")
    # ── output projection for the LAST position -> logits -> argmax ────────────
    L.append("Let Oi be a new Seq of Int.")
    L.append("Repeat for ov in stream:")
    L.append(f"    Push to8(ov, {ROUT}) to Oi.")
    L.append("Let last be (n - 1) * d.")
    if show_logits:
        # Show every last-position logit in order (byte-exact target)
        L.append("Let o be 0.")
        L.append("While o is less than V:")
        L.append("    Let acc be 0.")
        L.append("    Let i be 0.")
        L.append("    While i is less than d:")
        L.append("        Set acc to acc + (item (last + i + 1) of Oi) * (item (o * d + i + 1) of Eq).")
        L.append("        Set i to i + 1.")
        L.append("    Show acc.")
        L.append("    Set o to o + 1.")
    else:
        L.append("Let best be -1000000000.")
        L.append("Let bestIdx be 0.")
        L.append("Let o be 0.")
        L.append("While o is less than V:")
        L.append("    Let acc be 0.")
        L.append("    Let i be 0.")
        L.append("    While i is less than d:")
        L.append("        Set acc to acc + (item (last + i + 1) of Oi) * (item (o * d + i + 1) of Eq).")
        L.append("        Set i to i + 1.")
        L.append("    If acc is greater than best:")
        L.append("        Set best to acc.")
        L.append("        Set bestIdx to o.")
        L.append("    Set o to o + 1.")
        L.append("Show bestIdx.")
    return "\n".join(L) + "\n"


def emit_demo(j):
    """Emit a SINGLE self-contained .lg demo: `forwardStep(toks, W)` (the whole forward pass, weights read
    from the inlined blob `W` at offsets) + `## Main` that builds W, sets the prompt, and autoregressively
    generates gen_len tokens, Showing each. Runs standalone on any tier — no external weight blob or loader."""
    cfg = j["config"]
    d = cfg["d_model"]; H = cfg["n_heads"]; hd = d // H; dff = cfg["d_ff"]; V = cfg["vocab"]
    rq = j["requant"]
    RQKV = rq["r_qkv"][0]; RFFN = rq["r_ffn"][0]; ROUT = rq["r_out"][0]; RCTX = rq["r_ctx"][0]; RHID = rq["r_hid"][0]
    MQ = rq["M_q"][0]; MK = rq["M_k"][0]; MV = rq["M_v"][0]; MO = rq["M_o"][0]; MH = rq["M_h"][0]; MF = rq["M_f"][0]
    MSCORE = rq["M_score"][0]
    blob, off = weight_blob(j)
    arrs = weight_arrays(j)
    WQ, WK, WV, WO = off["Wq"], off["Wk"], off["Wv"], off["Wo"]
    W1o, W2o, EQ, EIN, PIN, EL = off["W1"], off["W2"], off["Eq"], off["EinA"], off["PinA"], off["explut"]
    g = j["golden_trace"]
    prompt = g["prompt_tokens"]
    gen_len = j["gen_len"]
    L = []
    L.append(emit_helpers())
    # ── forwardStep: one forward pass over `toks`, weights read from the inlined blob `W` at offsets ──
    L.append("## To forwardStep (toks: Seq of Int, W: Seq of Int) -> Int:")
    B = []  # function body (indented 4 spaces on emit)
    B.append(f"Let n be length of toks.")
    B.append(f"Let d be {d}.")
    B.append(f"Let hd be {hd}.")
    B.append(f"Let dff be {dff}.")
    B.append(f"Let V be {V}.")
    B.append("Let stream be a new Seq of Int.")
    B.append("Let p be 0.")
    B.append("While p is less than n:")
    B.append("    Let tok be item (p + 1) of toks.")
    B.append("    Let j be 0.")
    B.append("    While j is less than d:")
    B.append(f"        Push (item ({EIN} + tok * d + j + 1) of W) + (item ({PIN} + p * d + j + 1) of W) to stream.")
    B.append("        Set j to j + 1.")
    B.append("    Set p to p + 1.")
    B.append("Let Xi be a new Seq of Int.")
    B.append("Repeat for sv in stream:")
    B.append(f"    Push to8(sv, {RQKV}) to Xi.")
    for out, woff, MM in [("Q", WQ, MQ), ("K", WK, MK), ("Vv", WV, MV)]:
        B.append(f"Let {out} be a new Seq of Int.")
        B.append("Set p to 0.")
        B.append("While p is less than n:")
        B.append("    Let o be 0.")
        B.append("    While o is less than d:")
        B.append("        Let acc be 0.")
        B.append("        Let i be 0.")
        B.append("        While i is less than d:")
        B.append(f"            Set acc to acc + (item (p * d + i + 1) of Xi) * (item ({woff} + i * d + o + 1) of W).")
        B.append("            Set i to i + 1.")
        B.append(f"        Push to8(acc, {MM}) to {out}.")
        B.append("        Set o to o + 1.")
        B.append("    Set p to p + 1.")
    B.append("Let ctx be a new Seq of Int.")
    B.append("Let z be 0.")
    B.append("While z is less than n * d:")
    B.append("    Push 0 to ctx.")
    B.append("    Set z to z + 1.")
    B.append(f"Let head be 0.")
    B.append(f"While head is less than {H}:")
    B.append("    Let lo be head * hd.")
    B.append("    Let i be 0.")
    B.append("    While i is less than n:")
    B.append("        Let sc be a new Seq of Int.")
    B.append("        Let j be 0.")
    B.append("        While j is at most i:")
    B.append("            Let a be 0.")
    B.append("            Let c be 0.")
    B.append("            While c is less than hd:")
    B.append("                Set a to a + (item (i * d + lo + c + 1) of Q) * (item (j * d + lo + c + 1) of K).")
    B.append("                Set c to c + 1.")
    B.append("            Push a to sc.")
    B.append("            Set j to j + 1.")
    B.append("        Let mx be item 1 of sc.")
    B.append("        Set j to 2.")
    B.append("        While j is at most (i + 1):")
    B.append("            If item j of sc is greater than mx:")
    B.append("                Set mx to item j of sc.")
    B.append("            Set j to j + 1.")
    B.append("        Let num be a new Seq of Int.")
    B.append("        Let c be 0.")
    B.append("        While c is less than hd:")
    B.append("            Push 0 to num.")
    B.append("            Set c to c + 1.")
    B.append("        Let den be 0.")
    B.append("        Set j to 0.")
    B.append("        While j is at most i:")
    B.append("            Let dfc be mx - (item (j + 1) of sc).")
    B.append(f"            Let idx be (dfc * {MSCORE}) shifted right by 24.")
    B.append("            If idx is less than 0:")
    B.append("                Set idx to 0.")
    B.append("            If idx is greater than 255:")
    B.append("                Set idx to 255.")
    B.append(f"            Let wgt be item ({EL} + idx + 1) of W.")
    B.append("            Set c to 0.")
    B.append("            While c is less than hd:")
    B.append("                Set item (c + 1) of num to (item (c + 1) of num) + wgt * (item (j * d + lo + c + 1) of Vv).")
    B.append("                Set c to c + 1.")
    B.append("            Set den to den + wgt.")
    B.append("            Set j to j + 1.")
    B.append("        Set c to 0.")
    B.append("        While c is less than hd:")
    B.append("            Set item (i * d + lo + c + 1) of ctx to (item (c + 1) of num) // den.")
    B.append("            Set c to c + 1.")
    B.append("        Set i to i + 1.")
    B.append("    Set head to head + 1.")
    B.append("Let Ci be a new Seq of Int.")
    B.append("Repeat for cv in ctx:")
    B.append(f"    Push to8(cv, {RCTX}) to Ci.")
    B.append("Set p to 0.")
    B.append("While p is less than n:")
    B.append("    Let o be 0.")
    B.append("    While o is less than d:")
    B.append("        Let acc be 0.")
    B.append("        Let i be 0.")
    B.append("        While i is less than d:")
    B.append(f"            Set acc to acc + (item (p * d + i + 1) of Ci) * (item ({WO} + i * d + o + 1) of W).")
    B.append("            Set i to i + 1.")
    B.append(f"        Set item (p * d + o + 1) of stream to (item (p * d + o + 1) of stream) + rqf(acc, {MO}).")
    B.append("        Set o to o + 1.")
    B.append("    Set p to p + 1.")
    B.append("Let Hi be a new Seq of Int.")
    B.append("Repeat for hv in stream:")
    B.append(f"    Push to8(hv, {RFFN}) to Hi.")
    B.append("Let hid be a new Seq of Int.")
    B.append("Set p to 0.")
    B.append("While p is less than n:")
    B.append("    Let o be 0.")
    B.append("    While o is less than dff:")
    B.append("        Let acc be 0.")
    B.append("        Let i be 0.")
    B.append("        While i is less than d:")
    B.append(f"            Set acc to acc + (item (p * d + i + 1) of Hi) * (item ({W1o} + i * dff + o + 1) of W).")
    B.append("            Set i to i + 1.")
    B.append(f"        Let hv2 be rqf(acc, {MH}).")
    B.append("        If hv2 is less than 0:")
    B.append("            Set hv2 to 0.")
    B.append("        Push hv2 to hid.")
    B.append("        Set o to o + 1.")
    B.append("    Set p to p + 1.")
    B.append("Let Fi be a new Seq of Int.")
    B.append("Repeat for fv in hid:")
    B.append(f"    Push to8(fv, {RHID}) to Fi.")
    B.append("Set p to 0.")
    B.append("While p is less than n:")
    B.append("    Let o be 0.")
    B.append("    While o is less than d:")
    B.append("        Let acc be 0.")
    B.append("        Let i be 0.")
    B.append("        While i is less than dff:")
    B.append(f"            Set acc to acc + (item (p * dff + i + 1) of Fi) * (item ({W2o} + i * d + o + 1) of W).")
    B.append("            Set i to i + 1.")
    B.append(f"        Set item (p * d + o + 1) of stream to (item (p * d + o + 1) of stream) + rqf(acc, {MF}).")
    B.append("        Set o to o + 1.")
    B.append("    Set p to p + 1.")
    B.append("Let Oi be a new Seq of Int.")
    B.append("Repeat for ov in stream:")
    B.append(f"    Push to8(ov, {ROUT}) to Oi.")
    B.append("Let last be (n - 1) * d.")
    B.append("Let best be -1000000000.")
    B.append("Let bestIdx be 0.")
    B.append("Let o be 0.")
    B.append("While o is less than V:")
    B.append("    Let acc be 0.")
    B.append("    Let i be 0.")
    B.append("    While i is less than d:")
    B.append(f"        Set acc to acc + (item (last + i + 1) of Oi) * (item ({EQ} + o * d + i + 1) of W).")
    B.append("        Set i to i + 1.")
    B.append("    If acc is greater than best:")
    B.append("        Set best to acc.")
    B.append("        Set bestIdx to o.")
    B.append("    Set o to o + 1.")
    B.append("Return bestIdx.")
    for line in B:
        L.append("    " + line)
    # ── Main: build the inlined weight blob W, set the prompt, generate gen_len tokens ──
    L.append("## Main")
    L.append(emit_seq("W", blob[1:]))  # blob[0] is the length header; W is a Seq (length implicit)
    L.append(emit_seq("toks", prompt))
    L.append(f"Let step be 0.")
    L.append(f"While step is less than {gen_len}:")
    L.append("    Let nxt be forwardStep(toks, W).")
    L.append("    Show nxt.")
    L.append("    Push nxt to toks.")
    L.append("    Set step to step + 1.")
    return "\n".join(L) + "\n"


def emit_fabric(j, stream=False):
    """Emit a SINGLE self-contained function `generate () -> Int` that runs the WHOLE autoregressive forward
    pass on the config-driven whole-program FABRIC engine (`program_cfg_fabric`): the LLM directly on the
    reconfigurable fabric, no fetch-decode CPU. Three transforms make the whole net lowerable by that engine:
      • `to8`/`rqf` are INLINED (the fabric lowerer does cross-function CALLS *or* collections, not both — the
        forward pass needs collections, so it must be call-free);
      • every `Repeat for x in xs` is rewritten to an indexed `While` (the CFG engine has no `IterNext`);
      • `n` (the growing token count) is tracked as a counter, not `length of`.
    It generates `gen_len` tokens and returns them packed base-100 as result = ((t0)*100+t1)*100+t2 (so
    "mat" = [8,1,13] → 80113), so a single engine run reproduces the whole generation byte-identically to the
    VM. The forward-pass math is line-for-line the same as `emit_demo`'s proven body."""
    cfg = j["config"]
    d = cfg["d_model"]; H = cfg["n_heads"]; hd = d // H; dff = cfg["d_ff"]; V = cfg["vocab"]
    rq = j["requant"]
    RQKV = rq["r_qkv"][0]; RFFN = rq["r_ffn"][0]; ROUT = rq["r_out"][0]; RCTX = rq["r_ctx"][0]; RHID = rq["r_hid"][0]
    MQ = rq["M_q"][0]; MK = rq["M_k"][0]; MV = rq["M_v"][0]; MO = rq["M_o"][0]; MH = rq["M_h"][0]; MF = rq["M_f"][0]
    MSCORE = rq["M_score"][0]
    blob, off = weight_blob(j)
    WQ, WK, WV, WO = off["Wq"], off["Wk"], off["Wv"], off["Wo"]
    W1o, W2o, EQ, EIN, PIN, EL = off["W1"], off["W2"], off["Eq"], off["EinA"], off["PinA"], off["explut"]
    # STREAMED weights: read the pre-loaded blob directly from `loaded()` (heap ptr 0, filled by the DATA
    # loader) instead of Push-building a `W` Seq. `item k of loaded()` == `item k of W` (both 1-based over the
    # same blob), so only the source name changes — this removes the ~3,136 weight-Push cells (the 18,934-cell
    # load block), collapsing cells / the value array `v` / and the register file `nreg`.
    wsrc = "loaded()" if stream else "W"
    g = j["golden_trace"]
    prompt = g["prompt_tokens"]
    gen_len = j["gen_len"]

    def to8(target, expr, M):
        # inlined `to8(expr, M)` = clamp((expr*M)>>24, -128, 127) then `Push … to target`
        return [
            f"Let _t8 be ({expr} * {M}) shifted right by 24.",
            "If _t8 is greater than 127:",
            "    Set _t8 to 127.",
            "If _t8 is less than -128:",
            "    Set _t8 to -128.",
            f"Push _t8 to {target}.",
        ]

    def flat_to8(dst, src, M, length):
        # `Repeat for v in src: Push to8(v, M) to dst` rewritten as an indexed While over `length` elements.
        out = [f"Let {dst} be a new Seq of Int.", "Let fq be 0.", f"While fq is less than {length}:",
               f"    Let sv be item (fq + 1) of {src}."]
        out += ["    " + ln for ln in to8(dst, "sv", M)]
        out.append("    Set fq to fq + 1.")
        return out

    fwd = []
    # ── stream = EinA[tok] + PinA[pos] ─────────────────────────────────────────
    fwd += ["Let stream be a new Seq of Int.", "Let p be 0.", "While p is less than n:",
            "    Let tok be item (p + 1) of toks.", "    Let j be 0.", "    While j is less than d:",
            f"        Push (item ({EIN} + tok * d + j + 1) of {wsrc}) + (item ({PIN} + p * d + j + 1) of {wsrc}) to stream.",
            "        Set j to j + 1.", "    Set p to p + 1."]
    # ── Xi = to8(stream, RQKV) ─────────────────────────────────────────────────
    fwd += flat_to8("Xi", "stream", RQKV, "n * d")
    # ── Q,K,Vv = to8(Xi @ W, M) ───────────────────────────────────────────────
    for out, woff, MM in [("Q", WQ, MQ), ("K", WK, MK), ("Vv", WV, MV)]:
        fwd += [f"Let {out} be a new Seq of Int.", "Set p to 0.", "While p is less than n:",
                "    Let o be 0.", "    While o is less than d:", "        Let acc be 0.", "        Let i be 0.",
                "        While i is less than d:",
                f"            Set acc to acc + (item (p * d + i + 1) of Xi) * (item ({woff} + i * d + o + 1) of {wsrc}).",
                "            Set i to i + 1."]
        fwd += ["        " + ln for ln in to8(out, "acc", MM)]
        fwd += ["        Set o to o + 1.", "    Set p to p + 1."]
    # ── ctx (preallocated zeros) ──────────────────────────────────────────────
    fwd += ["Let ctx be a new Seq of Int.", "Let z be 0.", "While z is less than n * d:",
            "    Push 0 to ctx.", "    Set z to z + 1."]
    # ── attention per head/query ──────────────────────────────────────────────
    fwd += ["Let head be 0.", f"While head is less than {H}:", "    Let lo be head * hd.", "    Let i be 0.",
            "    While i is less than n:", "        Let sc be a new Seq of Int.", "        Let j be 0.",
            "        While j is at most i:", "            Let a be 0.", "            Let c be 0.",
            "            While c is less than hd:",
            "                Set a to a + (item (i * d + lo + c + 1) of Q) * (item (j * d + lo + c + 1) of K).",
            "                Set c to c + 1.", "            Push a to sc.", "            Set j to j + 1.",
            "        Let mx be item 1 of sc.", "        Set j to 2.", "        While j is at most (i + 1):",
            "            If item j of sc is greater than mx:", "                Set mx to item j of sc.",
            "            Set j to j + 1.", "        Let num be a new Seq of Int.", "        Let c be 0.",
            "        While c is less than hd:", "            Push 0 to num.", "            Set c to c + 1.",
            "        Let den be 0.", "        Set j to 0.", "        While j is at most i:",
            "            Let dfc be mx - (item (j + 1) of sc).",
            f"            Let idx be (dfc * {MSCORE}) shifted right by 24.",
            "            If idx is less than 0:", "                Set idx to 0.",
            "            If idx is greater than 255:", "                Set idx to 255.",
            f"            Let wgt be item ({EL} + idx + 1) of {wsrc}.", "            Set c to 0.",
            "            While c is less than hd:",
            "                Set item (c + 1) of num to (item (c + 1) of num) + wgt * (item (j * d + lo + c + 1) of Vv).",
            "                Set c to c + 1.", "            Set den to den + wgt.", "            Set j to j + 1.",
            "        Set c to 0.", "        While c is less than hd:",
            "            Set item (i * d + lo + c + 1) of ctx to (item (c + 1) of num) // den.",
            "            Set c to c + 1.", "        Set i to i + 1.", "    Set head to head + 1."]
    # ── Ci = to8(ctx, RCTX); attn_out = rqf(Ci @ Wo, MO); stream += attn_out ───
    fwd += flat_to8("Ci", "ctx", RCTX, "n * d")
    fwd += ["Set p to 0.", "While p is less than n:", "    Let o be 0.", "    While o is less than d:",
            "        Let acc be 0.", "        Let i be 0.", "        While i is less than d:",
            f"            Set acc to acc + (item (p * d + i + 1) of Ci) * (item ({WO} + i * d + o + 1) of {wsrc}).",
            "            Set i to i + 1.", f"        Let rq1 be (acc * {MO}) shifted right by 24.",
            "        Set item (p * d + o + 1) of stream to (item (p * d + o + 1) of stream) + rq1.",
            "        Set o to o + 1.", "    Set p to p + 1."]
    # ── FFN: Hi=to8(stream,RFFN); hid=relu(rqf(Hi@W1,MH)); Fi=to8(hid,RHID); stream += rqf(Fi@W2,MF) ──
    fwd += flat_to8("Hi", "stream", RFFN, "n * d")
    fwd += ["Let hid be a new Seq of Int.", "Set p to 0.", "While p is less than n:", "    Let o be 0.",
            "    While o is less than dff:", "        Let acc be 0.", "        Let i be 0.",
            "        While i is less than d:",
            f"            Set acc to acc + (item (p * d + i + 1) of Hi) * (item ({W1o} + i * dff + o + 1) of {wsrc}).",
            "            Set i to i + 1.", f"        Let hv2 be (acc * {MH}) shifted right by 24.",
            "        If hv2 is less than 0:", "            Set hv2 to 0.", "        Push hv2 to hid.",
            "        Set o to o + 1.", "    Set p to p + 1."]
    fwd += flat_to8("Fi", "hid", RHID, "n * dff")
    fwd += ["Set p to 0.", "While p is less than n:", "    Let o be 0.", "    While o is less than d:",
            "        Let acc be 0.", "        Let i be 0.", "        While i is less than dff:",
            f"            Set acc to acc + (item (p * dff + i + 1) of Fi) * (item ({W2o} + i * d + o + 1) of {wsrc}).",
            "            Set i to i + 1.", f"        Let rq2 be (acc * {MF}) shifted right by 24.",
            "        Set item (p * d + o + 1) of stream to (item (p * d + o + 1) of stream) + rq2.",
            "        Set o to o + 1.", "    Set p to p + 1."]
    # ── output projection for the LAST position -> logits -> argmax ────────────
    fwd += flat_to8("Oi", "stream", ROUT, "n * d")
    fwd += ["Let last be (n - 1) * d.", "Let best be -1000000000.", "Let bestIdx be 0.", "Let o be 0.",
            "While o is less than V:", "    Let acc be 0.", "    Let i be 0.", "    While i is less than d:",
            f"        Set acc to acc + (item (last + i + 1) of Oi) * (item ({EQ} + o * d + i + 1) of {wsrc}).",
            "        Set i to i + 1.", "    If acc is greater than best:", "        Set best to acc.",
            "        Set bestIdx to o.", "    Set o to o + 1."]

    # ── assemble generate(): weights + prompt + the autoregressive loop over the forward pass ──
    #  streamed: weights are NOT Push-built — they arrive pre-loaded in the heap (`loaded()` = heap ptr 0),
    #  streamed to the chip separately from the kernel (the DATA-segment model). The blob is written to a
    #  sidecar so the engine model / host loader can pre-load it. This removes the 18,934-cell weight-load
    #  block, so the whole config drops into flip-flops and only the heap needs block RAM.
    B = [] if stream else [emit_seq("W", blob[1:])]
    B += [emit_seq("toks", prompt),
          f"Let d be {d}.", f"Let hd be {hd}.", f"Let dff be {dff}.", f"Let V be {V}.",
          f"Let n be {len(prompt)}.", "Let result be 0.", "Let step be 0.",
          f"While step is less than {gen_len}:"]
    B += ["    " + ln for ln in fwd]
    B += ["    Set result to result * 100 + bestIdx.", "    Push bestIdx to toks.",
          "    Set n to n + 1.", "    Set step to step + 1.", "Return result."]
    if stream:
        import json as _json
        with open("/tmp/tf_blob.json", "w") as f:
            _json.dump(blob, f)   # [total_len, w0, w1, …] — heap pre-load image (heap[0]=len, heap[1..]=weights)
    L = ["## To native loaded () -> Seq of Int"] if stream else []
    L += [emit_helpers(), "## To generate () -> Int:"]
    for chunk in B:
        for line in chunk.split("\n"):
            L.append("    " + line)
    L += ["## Main", "    Show generate()."]
    return "\n".join(L) + "\n"


def fabric_packed_result(j):
    """The base-100 packed integer `generate()` returns for the golden generation (the fabric-engine oracle)."""
    r = 0
    for t in j["golden_trace"]["generated_tokens"]:
        r = r * 100 + int(t)
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--json", default="benchmarks/silicon/tiny_transformer.json")
    ap.add_argument("--step", type=int, default=0, help="forward over prompt + first STEP golden tokens")
    ap.add_argument("--logits", action="store_true", help="Show every last-position logit (byte-exact)")
    ap.add_argument("--board", action="store_true", help="on-chip variant: weights via loaded() from the blob")
    ap.add_argument("--blob", action="store_true", help="emit the preload blob (JSON int list) instead of .lg")
    ap.add_argument("--demo", action="store_true", help="SINGLE self-contained file: inline weights + autoregressive gen")
    ap.add_argument("--fabric", action="store_true", help="single call-free `generate()` for the config fabric engine")
    ap.add_argument("--stream", action="store_true", help="with --fabric: read weights from pre-loaded `loaded()` (heap) instead of Push-building them (writes the blob to /tmp/tf_blob.json)")
    ap.add_argument("--packed", action="store_true", help="emit the base-100 packed generation integer (fabric oracle)")
    args = ap.parse_args()
    j = load(args.json)
    g = j["golden_trace"]
    if args.packed:
        sys.stdout.write(str(fabric_packed_result(j)))
        return
    if args.fabric:
        sys.stdout.write(emit_fabric(j, stream=args.stream))
        return
    if args.demo:
        sys.stdout.write(emit_demo(j))
        return
    if args.blob:
        blob, _ = weight_blob(j)
        json.dump(blob, sys.stdout)
        return
    toks = list(g["prompt_tokens"]) + list(g["generated_tokens"])[:args.step]
    sys.stdout.write(emit_forward(j, toks, show_logits=args.logits, board=args.board))


if __name__ == "__main__":
    main()
