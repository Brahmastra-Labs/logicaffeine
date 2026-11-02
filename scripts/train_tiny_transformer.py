#!/usr/bin/env python3
"""train_tiny_transformer.py — the baby char-level transformer behind the FPGA inference demo.

Pure numpy (no torch): a tiny transformer is trained (float) to memorize a completion, then quantized
to symmetric int8 and re-run through a PURE-INTEGER reference path. That integer path — matmul in
int8→int32, fixed-point requant `(acc*M)>>S`, an exp LUT softmax, integer argmax — is the GOLDEN
ORACLE the LOGOS/fabric/silicon locks reproduce byte-for-byte. Everything is seeded and deterministic;
`--check` re-derives the exported golden trace from the committed weights.

Stage 1 here: float training + generation (validate the model actually learns the completion).
Later stages add the int8 reference + JSON export.
"""
import argparse
import numpy as np

# ── config ──────────────────────────────────────────────────────────────────────────────────────
SEED = 1234
D_MODEL = 16
N_HEADS = 2
N_LAYERS = 1
D_FF = 32
SEQ_LEN = 24           # context window (>= corpus length so the whole phrase fits)
# The corpus: a handful of short lines. "the " appears twice inside the target line, so completing
# "the cat sat on the " -> "mat" (not "cat") is an ATTENTION/position decision, not a bigram lookup.
CORPUS = [
    "the cat sat on the mat",
    "the dog sat on the log",
    "the cat ran to the mat",
]
PROMPT = "the cat sat on the "
GEN_LEN = 3            # expect "mat"


# ── vocab ───────────────────────────────────────────────────────────────────────────────────────
def build_vocab(corpus):
    chars = sorted(set("".join(corpus)))
    stoi = {c: i for i, c in enumerate(chars)}
    itos = {i: c for c, i in stoi.items()}
    return chars, stoi, itos


# ── a minimal tensor autograd (only the ops this model needs) ─────────────────────────────────────
class T:
    """A numpy array node with reverse-mode grad. Ops: matmul, +, *, relu, softmax(masked), pick-rows,
    cross-entropy. Enough for embed + attention + FFN + tied output."""
    def __init__(self, data, parents=(), backward=None):
        self.data = np.asarray(data, dtype=np.float64)
        self.grad = np.zeros_like(self.data)
        self._parents = parents
        self._backward = backward or (lambda: None)

    def __matmul__(self, other):
        out = T(self.data @ other.data, (self, other))
        def bw():
            self.grad += out.grad @ other.data.T
            other.grad += self.data.T @ out.grad
        out._backward = bw
        return out

    def __add__(self, other):
        out = T(self.data + other.data, (self, other))
        def bw():
            self.grad += _unbroadcast(out.grad, self.data.shape)
            other.grad += _unbroadcast(out.grad, other.data.shape)
        out._backward = bw
        return out

    def __mul__(self, s):  # scalar multiply
        out = T(self.data * s, (self,))
        def bw():
            self.grad += out.grad * s
        out._backward = bw
        return out

    def relu(self):
        out = T(np.maximum(self.data, 0.0), (self,))
        def bw():
            self.grad += out.grad * (self.data > 0.0)
        out._backward = bw
        return out

    def rows(self, idx):  # gather rows self[idx]  (embedding lookup)
        out = T(self.data[idx], (self,))
        def bw():
            np.add.at(self.grad, idx, out.grad)
        out._backward = bw
        return out

    def cols(self, lo, hi):  # column slice self[:, lo:hi]  (head split)
        out = T(self.data[:, lo:hi], (self,))
        def bw():
            self.grad[:, lo:hi] += out.grad
        out._backward = bw
        return out

    def transpose(self):
        out = T(self.data.T, (self,))
        def bw():
            self.grad += out.grad.T
        out._backward = bw
        return out

    def softmax_masked(self, mask):
        """row-wise softmax with an additive mask (0 keep, -inf drop) applied before the exp."""
        z = self.data + mask
        z = z - z.max(axis=-1, keepdims=True)
        e = np.exp(z)
        p = e / e.sum(axis=-1, keepdims=True)
        out = T(p, (self,))
        def bw():
            # dz = p * (g - sum(g*p))
            g = out.grad
            s = (g * p).sum(axis=-1, keepdims=True)
            self.grad += p * (g - s)
        out._backward = bw
        return out

    def backward(self):
        topo, seen = [], set()
        def build(n):
            if id(n) in seen:
                return
            seen.add(id(n))
            for p in n._parents:
                build(p)
            topo.append(n)
        build(self)
        self.grad = np.ones_like(self.data)
        for n in reversed(topo):
            n._backward()


def _unbroadcast(g, shape):
    while g.ndim > len(shape):
        g = g.sum(axis=0)
    for i, s in enumerate(shape):
        if s == 1:
            g = g.sum(axis=i, keepdims=True)
    return g


# ── model ─────────────────────────────────────────────────────────────────────────────────────
class Model:
    def __init__(self, vocab, rng):
        self.V = vocab
        d = D_MODEL
        sd = 0.02
        def p(shape):
            return T(rng.normal(0, sd, shape))
        self.E = p((vocab, d))          # token embedding (tied to output)
        self.P = p((SEQ_LEN, d))        # positional embedding
        self.Wq = p((d, d)); self.Wk = p((d, d)); self.Wv = p((d, d)); self.Wo = p((d, d))
        self.W1 = p((d, D_FF)); self.W2 = p((D_FF, d))

    def params(self):
        return [self.E, self.P, self.Wq, self.Wk, self.Wv, self.Wo, self.W1, self.W2]

    def forward(self, tokens):
        d, H = D_MODEL, N_HEADS
        hd = d // H
        n = len(tokens)
        idx = np.array(tokens)
        h = self.E.rows(idx) + self.P.rows(np.arange(n))    # embed + pos  [n,d]
        # attention (single layer, multi-head, causal)
        Q = h @ self.Wq; K = h @ self.Wk; Vv = h @ self.Wv  # [n,d]
        mask = np.where(np.tril(np.ones((n, n))) == 1, 0.0, -1e9)
        ctx_heads = []
        scale = 1.0 / np.sqrt(hd)
        for head in range(H):
            lo, hi = head * hd, (head + 1) * hd
            Qh = Q.cols(lo, hi); Kh = K.cols(lo, hi); Vh = Vv.cols(lo, hi)
            att = (Qh @ Kh.transpose()) * scale
            p = att.softmax_masked(mask)
            ctx_heads.append(p @ Vh)
        # concat heads
        ctx = _concat_cols(ctx_heads)
        h = h + (ctx @ self.Wo)                             # residual
        f = (h @ self.W1).relu() @ self.W2
        h = h + f                                           # residual
        logits = h @ self.E.transpose()                    # tied output  [n,vocab]
        return logits


def _concat_cols(tensors):
    data = np.concatenate([t.data for t in tensors], axis=1)
    out = T(data, tuple(tensors))
    widths = [t.data.shape[1] for t in tensors]
    def bw():
        off = 0
        for t, w in zip(tensors, widths):
            t.grad += out.grad[:, off:off + w]
            off += w
    out._backward = bw
    return out


def cross_entropy(logits, targets):
    z = logits.data - logits.data.max(axis=-1, keepdims=True)
    e = np.exp(z); p = e / e.sum(axis=-1, keepdims=True)
    n = len(targets)
    loss = -np.log(p[np.arange(n), targets] + 1e-12).mean()
    out = T(loss, (logits,))
    def bw():
        g = p.copy()
        g[np.arange(n), targets] -= 1.0
        g /= n
        logits.grad += g * out.grad
    out._backward = bw
    return out


def train(model, stoi, steps=3000, lr=0.01):
    # Adam
    ps = model.params()
    m = [np.zeros_like(p.data) for p in ps]
    v = [np.zeros_like(p.data) for p in ps]
    b1, b2, eps = 0.9, 0.999, 1e-8
    data = [[stoi[c] for c in line] for line in CORPUS]
    t = 0
    for step in range(1, steps + 1):
        for seq in data:
            if len(seq) < 2:
                continue
            for p in ps:
                p.grad = np.zeros_like(p.data)
            logits = model.forward(seq[:-1])
            loss = cross_entropy(logits, np.array(seq[1:]))
            loss.backward()
            t += 1
            for i, p in enumerate(ps):
                g = p.grad
                m[i] = b1 * m[i] + (1 - b1) * g
                v[i] = b2 * v[i] + (1 - b2) * (g * g)
                mh = m[i] / (1 - b1 ** t)
                vh = v[i] / (1 - b2 ** t)
                p.data -= lr * mh / (np.sqrt(vh) + eps)
        if step % 500 == 0 or step == 1:
            print(f"  step {step:5d}  loss {loss.data:.5f}")
    return model


def generate_float(model, stoi, itos, prompt, n):
    toks = [stoi[c] for c in prompt]
    out = []
    for _ in range(n):
        logits = model.forward(toks)
        nxt = int(np.argmax(logits.data[-1]))
        out.append(nxt)
        toks.append(nxt)
    return "".join(itos[t] for t in out)


# ── INT8 QUANTIZATION + PURE-INTEGER REFERENCE (the golden oracle) ────────────────────────────────
# Every matmul has the identical 3-step shape, all reproducible in LOGOS integer arithmetic:
#   (1) input-requant a wide int activation -> int8:  xq = clip((x * M) >> S, -128, 127)
#   (2) int8 MAC -> wide int:                          acc = Σ xq[i] * Wq[o,i]
#   (3) output-requant wide int -> target scale:       y  = (acc * M) >> S
# `>>` is an ARITHMETIC shift (floor toward -inf), matching LOGOS `shifted right by` and numpy int `>>`.
# Requant is TRUNCATING (no rounding bias) so numpy and LOGOS agree bit-for-bit. Softmax uses a shared
# integer exp LUT. The residual stream is a wide int at a single scale sA; embedding+positional tables
# are pre-quantized directly to sA so the initial stream is a clean integer ADD. Weights are int8.
QMAX = 127
LUT_BITS = 8                      # exp LUT indexed by an 8-bit score deficit (max - score), 0..255
LUT_ONE = 1 << 15                 # fixed-point 1.0 for the exp weights
SHIFT = 24                        # fixed-point shift for all (M,S) requant multipliers


def ms(r):
    """Fixed-point (M, S) with S=SHIFT such that M>>S ≈ r (r>0)."""
    return int(round(r * (1 << SHIFT))), SHIFT


def q_weight(W):
    s = float(np.max(np.abs(W))) / QMAX
    if s == 0.0:
        s = 1.0
    return np.clip(np.round(W / s), -QMAX, QMAX).astype(np.int64), s


def rq(acc, MS):
    return (np.asarray(acc, dtype=np.int64) * MS[0]) >> MS[1]     # arithmetic shift (floor)


def to8(x32, MS):
    return np.clip(rq(x32, MS), -128, QMAX).astype(np.int64)      # input-requant -> int8


def calibrate(model, stoi):
    """Run the FULL float forward over the corpus; record max-abs at EACH int8 matmul-input site
    separately (per-site scales — a single global scale starves the small early activations)."""
    d, H = D_MODEL, N_HEADS
    hd = d // H
    sites = ("s_qkv", "s_ffn", "s_out", "Q", "K", "V", "ctx", "hidden", "deficit")
    a = {k: 1e-9 for k in sites}
    for line in CORPUS:
        toks = [stoi[c] for c in line[:-1]]
        n = len(toks)
        h = model.E.data[np.array(toks)] + model.P.data[:n]
        a["s_qkv"] = max(a["s_qkv"], np.max(np.abs(h)))              # QKV matmul input (init stream)
        Q = h @ model.Wq.data; K = h @ model.Wk.data; Vv = h @ model.Wv.data
        a["Q"] = max(a["Q"], np.max(np.abs(Q))); a["K"] = max(a["K"], np.max(np.abs(K)))
        a["V"] = max(a["V"], np.max(np.abs(Vv)))
        ctx = np.zeros((n, d))
        for head in range(H):
            sl = slice(head * hd, (head + 1) * hd)
            raw = (Q[:, sl] @ K[:, sl].T) * (1.0 / np.sqrt(hd))
            for i in range(n):                                       # causal, valid j<=i only
                row = raw[i, : i + 1]
                a["deficit"] = max(a["deficit"], float(row.max() - row.min()))
                e = np.exp(row - row.max()); p = e / e.sum()
                ctx[i, sl] = p @ Vv[: i + 1, sl]
        a["ctx"] = max(a["ctx"], np.max(np.abs(ctx)))
        h = h + ctx @ model.Wo.data                                  # residual (post-attention)
        a["s_ffn"] = max(a["s_ffn"], np.max(np.abs(h)))              # W1 matmul input
        hid = np.maximum(h @ model.W1.data, 0.0)
        a["hidden"] = max(a["hidden"], np.max(hid))
        h = h + hid @ model.W2.data                                  # residual (post-FFN)
        a["s_out"] = max(a["s_out"], np.max(np.abs(h)))              # output-projection input
    return a


class QModel:
    """int8 weights + sA-quantized embed/pos tables + PER-SITE (M,S) requant params + the exp LUT."""
    def __init__(self, model, calib, chars):
        self.chars = chars
        self.V = model.V
        self.d = D_MODEL
        self.Wq, sWq = q_weight(model.Wq.data)
        self.Wk, sWk = q_weight(model.Wk.data)
        self.Wv, sWv = q_weight(model.Wv.data)
        self.Wo, sWo = q_weight(model.Wo.data)
        self.W1, sW1 = q_weight(model.W1.data)
        self.W2, sW2 = q_weight(model.W2.data)
        self.Eq, sE = q_weight(model.E.data)               # int8 output-projection weights (scale sE)
        # per-site int8 LSBs (real magnitude / 127)
        s_qkv = calib["s_qkv"] / QMAX; s_ffn = calib["s_ffn"] / QMAX; s_out = calib["s_out"] / QMAX
        s_ctx = calib["ctx"] / QMAX
        sQ = calib["Q"] / QMAX; sK = calib["K"] / QMAX; sV = calib["V"] / QMAX; sH = calib["hidden"] / QMAX
        sA = s_qkv                                          # stream scale = QKV-input LSB (ratio 1 there)
        self.sA = sA
        # embed + pos quantized DIRECTLY to the stream scale sA -> integer add builds the initial stream
        self.EinA = np.round(model.E.data / sA).astype(np.int64)   # [V,d]
        self.PinA = np.round(model.P.data / sA).astype(np.int64)   # [seq,d]
        # input-requant ratios (source_scale / target_int8_LSB) — downscale the wide stream per site
        self.r_qkv = ms(sA / s_qkv)                        # = 1.0
        self.r_ffn = ms(sA / s_ffn)
        self.r_out = ms(sA / s_out)
        self.r_ctx = ms(sV / s_ctx)                        # ctx (scale sV) -> int8
        self.r_hid = ms(1.0)                               # hidden already int8 (sH) after W1 requant
        # output-requant multipliers (fold input*weight scales into the target scale)
        self.M_q = ms((s_qkv * sWq) / sQ); self.M_k = ms((s_qkv * sWk) / sK); self.M_v = ms((s_qkv * sWv) / sV)
        self.M_o = ms((s_ctx * sWo) / sA)                  # Wo acc -> stream (residual add)
        self.M_h = ms((s_ffn * sW1) / sH)                  # W1 acc -> hidden int8 (pre-ReLU)
        self.M_f = ms((sH * sW2) / sA)                     # W2 acc -> stream (residual add)
        # exp LUT: index = score deficit (max-score) mapped to 0..255
        self.deficit_step = calib["deficit"] / ((1 << LUT_BITS) - 1)
        self.attn_scale = 1.0 / np.sqrt(self.d // N_HEADS)
        self.M_score = ms((sQ * sK * self.attn_scale) / self.deficit_step)  # score-units -> LUT index
        self.exp_lut = np.array(
            [int(round(np.exp(-i * self.deficit_step) * LUT_ONE)) for i in range(1 << LUT_BITS)],
            dtype=np.int64,
        )


def int8_forward(q, tokens, trace=None):
    """The pure-integer forward pass. Returns int logits for the last position (argmax = next token)."""
    d, H = q.d, N_HEADS
    hd = d // H
    n = len(tokens)
    # embed + pos -> residual stream (wide int, scale sA): a clean integer add of two sA-scaled tables
    stream = np.array([q.EinA[t] + q.PinA[i] for i, t in enumerate(tokens)], dtype=np.int64)  # [n,d]
    # attention (single layer, multi-head, causal)
    Xi = to8(stream, q.r_qkv)                                # int8 matmul input
    Q = to8(Xi @ q.Wq, q.M_q); K = to8(Xi @ q.Wk, q.M_k); Vv = to8(Xi @ q.Wv, q.M_v)  # [n,d] int8
    ctx = np.zeros((n, d), dtype=np.int64)
    for head in range(H):
        sl = slice(head * hd, (head + 1) * hd)
        Qh, Kh, Vh = Q[:, sl], K[:, sl], Vv[:, sl]
        scores = Qh @ Kh.T                                  # [n,n] wide int (scale sQ*sK)
        for i in range(n):
            mx = max(int(scores[i, j]) for j in range(i + 1))          # causal max over j<=i
            num = np.zeros(hd, dtype=np.int64); den = 0
            for j in range(i + 1):
                idx = int(rq(mx - scores[i, j], q.M_score))            # deficit -> LUT index
                idx = 0 if idx < 0 else (255 if idx > 255 else idx)
                w = int(q.exp_lut[idx])
                num += w * Vh[j]; den += w
            ctx[i, sl] = num // den if den != 0 else 0                 # weighted average (int divide)
    Ci = to8(ctx, q.r_ctx)
    attn_out = rq(Ci @ q.Wo, q.M_o)                         # -> stream scale
    stream = stream + attn_out                              # residual
    # FFN + ReLU
    Hi = to8(stream, q.r_ffn)
    hid = rq(Hi @ q.W1, q.M_h)                              # pre-activation (scale sH)
    hid = np.maximum(hid, 0)                                # ReLU
    Fi = to8(hid, q.r_hid)
    ffn = rq(Fi @ q.W2, q.M_f)
    stream = stream + ffn                                   # residual
    # tied output projection -> logits (argmax is scale-invariant, no requant)
    Oi = to8(stream, q.r_out)
    logits = Oi @ q.Eq.T                                    # [n, V] wide int
    if trace is not None:
        trace["logits_last"] = logits[-1].tolist()
    return logits[-1]


def generate_int8(q, stoi, itos, prompt, n):
    toks = [stoi[c] for c in prompt]
    out = []
    for _ in range(n):
        nxt = int(np.argmax(int8_forward(q, toks)))
        out.append(nxt); toks.append(nxt)
    return "".join(itos[t] for t in out)


def golden_trace(q, stoi, itos, prompt, n):
    """Autoregressive generation capturing per-step last-position int logits — the byte-exact target
    the LOGOS/fabric/silicon locks reproduce. argmax over logits = next token."""
    toks = [stoi[c] for c in prompt]
    steps = []
    for _ in range(n):
        logits = int8_forward(q, toks)
        nxt = int(np.argmax(logits))
        steps.append({"context_len": len(toks), "logits": [int(x) for x in logits], "argmax": nxt})
        toks.append(nxt)
    text = "".join(itos[s["argmax"]] for s in steps)
    return {"prompt": prompt, "prompt_tokens": [stoi[c] for c in prompt],
            "generated_tokens": [s["argmax"] for s in steps], "generated_text": text, "steps": steps}


def export(model, q, calib, chars, stoi, itos, path):
    import json
    d = D_MODEL
    def flat(a):
        return [int(x) for x in np.asarray(a).ravel()]
    trace = golden_trace(q, stoi, itos, PROMPT, GEN_LEN)
    obj = {
        "meta": {
            "artifact": "tiny char-level transformer — int8 quantized inference oracle",
            "note": "The int8 integer path (matmul int8->int, requant (acc*M)>>S arithmetic-shift, exp-LUT "
                    "softmax, integer argmax) is the byte-exact reference the LOGOS/fabric/silicon locks "
                    "reproduce. 'byte-identical to the reference' means THIS integer path, not float PyTorch.",
            "deterministic": f"numpy seed {SEED}; --check re-derives this file from the committed corpus",
        },
        "config": {"d_model": d, "n_heads": N_HEADS, "n_layers": N_LAYERS, "d_ff": D_FF,
                   "seq_len": SEQ_LEN, "vocab": len(chars), "chars": chars,
                   "SHIFT": SHIFT, "LUT_BITS": LUT_BITS, "LUT_ONE": LUT_ONE, "QMAX": QMAX},
        "prompt": PROMPT, "gen_len": GEN_LEN,
        "scales_debug": {k: float(v) for k, v in calib.items()},
        "requant": {  # (M, S) fixed-point requant params — LOGOS uses these integers directly
            "r_qkv": q.r_qkv, "r_ffn": q.r_ffn, "r_out": q.r_out, "r_ctx": q.r_ctx, "r_hid": q.r_hid,
            "M_q": q.M_q, "M_k": q.M_k, "M_v": q.M_v, "M_o": q.M_o, "M_h": q.M_h, "M_f": q.M_f,
            "M_score": q.M_score,
        },
        "weights_int8": {  # row-major flattened; shapes in "config"
            "Wq": flat(q.Wq), "Wk": flat(q.Wk), "Wv": flat(q.Wv), "Wo": flat(q.Wo),
            "W1": flat(q.W1), "W2": flat(q.W2), "Eq": flat(q.Eq),
        },
        "embed_stream": {  # embedding+positional pre-quantized to the stream scale sA (integer add)
            "EinA": flat(q.EinA), "PinA": flat(q.PinA),
        },
        "exp_lut": flat(q.exp_lut),
        "golden_trace": trace,
    }
    with open(path, "w") as f:
        json.dump(obj, f, separators=(",", ":"))
    return obj, trace


DEFAULT_OUT = "benchmarks/silicon/tiny_transformer.json"


def build_all(steps, lr):
    rng = np.random.default_rng(SEED)
    chars, stoi, itos = build_vocab(CORPUS)
    model = Model(len(chars), rng)
    train(model, stoi, steps=steps, lr=lr)
    calib = calibrate(model, stoi)
    q = QModel(model, calib, chars)
    return model, q, calib, chars, stoi, itos


def main():
    global D_MODEL, N_HEADS, D_FF
    ap = argparse.ArgumentParser()
    ap.add_argument("--steps", type=int, default=3000)
    ap.add_argument("--d-model", type=int, default=D_MODEL)
    ap.add_argument("--n-heads", type=int, default=N_HEADS)
    ap.add_argument("--d-ff", type=int, default=D_FF)
    ap.add_argument("--lr", type=float, default=0.01)
    ap.add_argument("--out", default=DEFAULT_OUT)
    ap.add_argument("--check", action="store_true", help="re-derive and compare to the committed JSON")
    args = ap.parse_args()
    D_MODEL, N_HEADS, D_FF = args.d_model, args.n_heads, args.d_ff

    model, q, calib, chars, stoi, itos = build_all(args.steps, args.lr)
    print(f"vocab({len(chars)}): {chars!r}")
    fgen = generate_float(model, stoi, itos, PROMPT, GEN_LEN)
    igen = generate_int8(q, stoi, itos, PROMPT, GEN_LEN)
    print(f"prompt: {PROMPT!r}")
    print(f"float generation: {fgen!r}   (expect 'mat')")
    print(f"int8  generation: {igen!r}   (expect 'mat')")
    assert igen == "mat", f"int8 model must generate 'mat', got {igen!r}"

    if args.check:
        import json
        with open(args.out) as f:
            committed = json.load(f)
        _, trace = export(model, q, calib, chars, stoi, itos, "/tmp/_tt_check.json")
        assert trace == committed["golden_trace"], "golden trace drifted from the committed JSON!"
        print(f"--check OK: re-derived golden trace matches {args.out}")
    else:
        _, trace = export(model, q, calib, chars, stoi, itos, args.out)
        print(f"exported {args.out}  (generated_text={trace['generated_text']!r}, {len(trace['steps'])} steps)")


if __name__ == "__main__":
    main()
