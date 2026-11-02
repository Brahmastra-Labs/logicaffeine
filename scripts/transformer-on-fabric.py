#!/usr/bin/env python3
"""transformer-on-fabric — run the tiny transformer's inference with EVERY weight-matmul executed on the
fabric int8 dot-accelerator on the chip (nexys_matmul_soc over UART), producing the tokens on real silicon.

The whole forward pass (embedding, multi-head attention, integer-LUT softmax, ReLU FFN, tied-output argmax,
autoregressive loop) is the committed int8 reference; the int8×int8 WEIGHT matmuls (QKV / QK-scores / output
proj / FFN / logits) are offloaded dot-by-dot to the fabric — the LLM's actual compute, on the fabric. (The
softmax-weighted context sum uses >int8 weights, so it stays on the host — that's the softmax application,
orchestration, not a weight matmul.)

  python3 scripts/transformer-on-fabric.py                       # local dots — verify logic ≡ golden (fast)
  python3 scripts/transformer-on-fabric.py --port B              # dots on the fabric on Board B's chip
"""
import argparse, json, os, sys, time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
import ftdi_ports  # noqa: E402
import numpy as np

SHIFT = 24
J = json.load(open("benchmarks/silicon/tiny_transformer.json"))
C = J["config"]; D = C["d_model"]; H = C["n_heads"]; HD = D // H; DFF = C["d_ff"]; V = C["vocab"]
RQ = J["requant"]
W = {k: np.array(v, dtype=np.int64) for k, v in J["weights_int8"].items()}
Wq = W["Wq"].reshape(D, D); Wk = W["Wk"].reshape(D, D); Wv = W["Wv"].reshape(D, D)
Wo = W["Wo"].reshape(D, D); W1 = W["W1"].reshape(D, DFF); W2 = W["W2"].reshape(DFF, D); Eq = W["Eq"].reshape(V, D)
EinA = np.array(J["embed_stream"]["EinA"], dtype=np.int64).reshape(-1, D)
PinA = np.array(J["embed_stream"]["PinA"], dtype=np.int64).reshape(-1, D)
EXPL = np.array(J["exp_lut"], dtype=np.int64)
G = J["golden_trace"]

def ms(k): return RQ[k][0], RQ[k][1]
def rq(acc, k):  m, s = ms(k); return (np.asarray(acc, dtype=np.int64) * m) >> s
def to8(x, k):   return np.clip(rq(x, k), -128, 127).astype(np.int64)

_stats = {"dots": 0, "elems": 0}
_ser = None

def fabric_dot(a, b):
    """Σ a[i]·b[i] over int8 vectors — on the fabric accelerator if a serial port is open, else locally."""
    _stats["dots"] += 1; _stats["elems"] += len(a)
    if _ser is None:
        return int(np.dot(np.asarray(a, np.int64), np.asarray(b, np.int64)))
    xs = []
    for x, y in zip(a, b): xs += [int(x) & 0xFF, int(y) & 0xFF]
    _ser.reset_input_buffer()
    _ser.write(bytes([len(xs)]) + bytes(xs))
    rec = _ser.read(11)
    assert len(rec) == 11 and rec[0] == 0xA5, f"bad record {rec!r}"
    return int.from_bytes(rec[1:9], "big", signed=True)

def fmatmul(A, B):
    """A[m,k] @ B[k,n] with every output element computed as a fabric dot of a row of A and a col of B."""
    A = np.asarray(A, np.int64); B = np.asarray(B, np.int64)
    m, k = A.shape; k2, n = B.shape
    out = np.zeros((m, n), dtype=np.int64)
    for i in range(m):
        for jj in range(n):
            out[i, jj] = fabric_dot(A[i], B[:, jj])
    return out

def forward(tokens):
    n = len(tokens)
    stream = np.array([EinA[t] + PinA[i] for i, t in enumerate(tokens)], dtype=np.int64)
    Xi = to8(stream, "r_qkv")
    Q = to8(fmatmul(Xi, Wq), "M_q"); K = to8(fmatmul(Xi, Wk), "M_k"); Vv = to8(fmatmul(Xi, Wv), "M_v")
    ctx = np.zeros((n, D), dtype=np.int64)
    for head in range(H):
        sl = slice(head * HD, (head + 1) * HD)
        Qh, Kh, Vh = Q[:, sl], K[:, sl], Vv[:, sl]
        scores = fmatmul(Qh, Kh.T)                       # int8×int8 → FABRIC
        for i in range(n):
            mx = max(int(scores[i, j]) for j in range(i + 1))
            num = np.zeros(HD, dtype=np.int64); den = 0
            for j in range(i + 1):                        # softmax application (>int8 weights) → host
                idx = int(rq(mx - scores[i, j], "M_score")); idx = 0 if idx < 0 else (255 if idx > 255 else idx)
                w = int(EXPL[idx]); num += w * Vh[j]; den += w
            ctx[i, sl] = num // den if den else 0
    Ci = to8(ctx, "r_ctx")
    stream = stream + rq(fmatmul(Ci, Wo), "M_o")
    Hi = to8(stream, "r_ffn"); hid = np.maximum(rq(fmatmul(Hi, W1), "M_h"), 0)
    Fi = to8(hid, "r_hid"); stream = stream + rq(fmatmul(Fi, W2), "M_f")
    Oi = to8(stream, "r_out")
    logits = fmatmul(Oi, Eq.T)
    return int(np.argmax(logits[-1]))

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", help="a device, or a board letter (A or B)"); ap.add_argument("--baud", type=int, default=115200)
    a = ap.parse_args()
    global _ser
    if a.port:
        import serial
        _ser = serial.Serial(ftdi_ports.port_arg(a.port), a.baud, timeout=5)
    chars = C["chars"] if "chars" in C else None
    toks = list(G["prompt_tokens"]); gen = []
    where = "the FABRIC on the chip" if a.port else "local (logic check)"
    print(f"running the tiny transformer — weight-matmuls on {where}")
    t0 = time.time()
    for step in range(J["gen_len"]):
        nxt = forward(toks); gen.append(nxt); toks.append(nxt)
        ch = chars[nxt] if chars else "?"
        print(f"  token {step+1}: id={nxt} ('{ch}')   [{_stats['dots']} matmul dots on the fabric so far]")
    text = "".join(chars[t] for t in gen) if chars else ""
    dt = time.time() - t0
    ok = gen == G["generated_tokens"]
    print(f"generated tokens {gen} = \"{text}\"  (golden {G['generated_tokens']} = \"{G['generated_text']}\")  {'OK ≡' if ok else 'MISMATCH'}")
    print(f"{_stats['dots']} matmul dot-products ({_stats['elems']} MACs) computed on {where} in {dt:.1f}s")
    sys.exit(0 if ok else 1)

if __name__ == "__main__":
    main()
