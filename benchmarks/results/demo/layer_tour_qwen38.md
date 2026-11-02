## Qwen3.8-27B, layer 0: a gated-delta layer

26 statements compute; 24 declare its weights, state and inputs.

| LOGOS | CUDA launch | fused by | in the same launch |
|---|---|---|---|
| `Let xn0 be rmsnorm of x with weight an0.` | #0 `gather_norm` | embed_norm, fused_quant | x |
| `Let qr0 be xn0 matmul Wq0.` | #1 `gemv_parts` | gemv_parts | kr0, vr0, zz0 |
| `Let kr0 be xn0 matmul Wk0.` | #1 `gemv_parts` | gemv_parts | qr0, vr0, zz0 |
| `Let vr0 be xn0 matmul Wv0.` | #1 `gemv_parts` | gemv_parts | kr0, qr0, zz0 |
| `Let zz0 be xn0 matmul Wz0.` | #1 `gemv_parts` | gemv_parts | kr0, qr0, vr0 |
| `Let qc0 be causal conv of qr0 with weight Wcq0 state Csq0 over 4 taps.` | #2 `conv` | conv_parts, conv_silu, conv_silu_l2 | kc0, kn0, ks0, qn0, qs0, vc0, vs0 |
| `Let kc0 be causal conv of kr0 with weight Wck0 state Csk0 over 4 taps.` | #2 `conv` | conv_parts, conv_silu, conv_silu_l2 | kn0, ks0, qc0, qn0, qs0, vc0, vs0 |
| `Let vc0 be causal conv of vr0 with weight Wcv0 state Csv0 over 4 taps.` | #2 `conv` | conv_parts, conv_silu | kc0, kn0, ks0, qc0, qn0, qs0, vs0 |
| `Let qs0 be silu of qc0.` | #2 `conv` | conv_parts, conv_silu, conv_silu_l2 | kc0, kn0, ks0, qc0, qn0, vc0, vs0 |
| `Let ks0 be silu of kc0.` | #2 `conv` | conv_parts, conv_silu, conv_silu_l2 | kc0, kn0, qc0, qn0, qs0, vc0, vs0 |
| `Let vs0 be silu of vc0.` | #2 `conv` | conv_parts, conv_silu | kc0, kn0, ks0, qc0, qn0, qs0, vc0 |
| `Let qn0 be l2norm of qs0 across 16 heads.` | #2 `conv` | conv_parts, conv_silu_l2 | kc0, kn0, ks0, qc0, qs0, vc0, vs0 |
| `Let kn0 be l2norm of ks0 across 16 heads.` | #2 `conv` | conv_parts, conv_silu_l2 | kc0, ks0, qc0, qn0, qs0, vc0, vs0 |
| `Let br0 be xn0 matmul Wb0.` | #3 `gates` | gate_projections | ar0, bt0, dy0 |
| `Let bt0 be sigmoid of br0.` | #3 `gates` | gate_projections, gates | ar0, br0, dy0 |
| `Let ar0 be xn0 matmul Wa0.` | #3 `gates` | gate_projections | br0, bt0, dy0 |
| `Let dy0 be decay of ar0 with bias Dt0 rate Al0.` | #3 `gates` | gate_projections, gates | ar0, br0, bt0 |
| `Let od0 be gated delta of qn0 over kn0 and vs0 with gate zz0 beta bt0 decay dy0 state St0 norm Gn0 with 48 heads 16 groups.` | #4 `delta` | fused_quant | — |
| `Let ao0 be od0 matmul Wo0.` | #5 `gemv_norm` | fused_quant, gemv_norm_epilogue | x2n0, xr0 |
| `Let xr0 be x plus ao0.` | #5 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | ao0, x2n0 |
| `Let x2n0 be rmsnorm of xr0 with weight fn0.` | #5 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | ao0, xr0 |
| `Let gt0 be x2n0 matmul Wg0.` | #6 `gemv_glu` | fused_quant, glu_pair | hh0, up0 |
| `Let up0 be x2n0 matmul Wu0.` | #6 `gemv_glu` | fused_quant, glu_pair | gt0, hh0 |
| `Let hh0 be swiglu of gt0 and up0.` | #6 `gemv_glu` | fused_quant, glu_pair | gt0, up0 |
| `Let dn0 be hh0 matmul Wd0.` | #7 `gemv_norm` | fused_quant, gemv_norm_epilogue | xn1, xo0 |
| `Let xo0 be xr0 plus dn0.` | #7 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | dn0, xn1 |

## Qwen3.8-27B, layer 3: a gated-attention layer

22 statements compute; 18 declare its weights, state and inputs.

| LOGOS | CUDA launch | fused by | in the same launch |
|---|---|---|---|
| `Let xn3 be rmsnorm of xo2 with weight an3.` | #21 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | dn2, xo2 |
| `Let qr3 be xn3 matmul Wq3.` | #22 `gemv_rope` | gemv_parts, qkv_head_epilogue | Kc3, Vc3, gr3, km3, kp3, kr3, qm3, qp3, vr3 |
| `Let gr3 be xn3 matmul Wqg3.` | #22 `gemv_rope` | gemv_parts | Kc3, Vc3, km3, kp3, kr3, qm3, qp3, qr3, vr3 |
| `Let kr3 be xn3 matmul Wk3.` | #22 `gemv_rope` | gemv_parts, qkv_head_epilogue | Kc3, Vc3, gr3, km3, kp3, qm3, qp3, qr3, vr3 |
| `Let vr3 be xn3 matmul Wv3.` | #22 `gemv_rope` | gemv_parts, row_write | Kc3, Vc3, gr3, km3, kp3, kr3, qm3, qp3, qr3 |
| `Let qm3 be rmsnorm of qr3 with weight Qn3 across 24 heads.` | #22 `gemv_rope` | gemv_parts, head_norm_rope, qkv_head_epilogue | Kc3, Vc3, gr3, km3, kp3, kr3, qp3, qr3, vr3 |
| `Let km3 be rmsnorm of kr3 with weight Kn3 across 4 heads.` | #22 `gemv_rope` | gemv_parts, head_norm_rope, qkv_head_epilogue, row_write | Kc3, Vc3, gr3, kp3, kr3, qm3, qp3, qr3, vr3 |
| `Let qp3 be rope of qm3 with cos Rcos and sin Rsin from position pos across 24 heads.` | #22 `gemv_rope` | gemv_parts, head_norm_rope, qkv_head_epilogue | Kc3, Vc3, gr3, km3, kp3, kr3, qm3, qr3, vr3 |
| `Let kp3 be rope of km3 with cos Rcos and sin Rsin from position pos across 4 heads.` | #22 `gemv_rope` | gemv_parts, head_norm_rope, qkv_head_epilogue, row_write | Kc3, Vc3, gr3, km3, kr3, qm3, qp3, qr3, vr3 |
| `Set row pos of Kc3 to kp3.` | #22 `gemv_rope` | gemv_parts, qkv_head_epilogue, row_write | Vc3, gr3, km3, kp3, kr3, qm3, qp3, qr3, vr3 |
| `Set row pos of Vc3 to vr3.` | #22 `gemv_rope` | gemv_parts, row_write | Kc3, gr3, km3, kp3, kr3, qm3, qp3, qr3, vr3 |
| `Let ct3 be attention of qp3 over Kc3 and Vc3 with 24 heads 4 groups up to pos.` | #23 `attention` | attn_output_gate, fused_quant | cg3, gs3 |
| `Let gs3 be sigmoid of gr3.` | #23 `attention` | act_gate, attn_output_gate, fused_quant | cg3, ct3 |
| `Let cg3 be ct3 hadamard gs3.` | #23 `attention` | act_gate, attn_output_gate, fused_quant | ct3, gs3 |
| `Let ao3 be cg3 matmul Wo3.` | #24 `gemv_norm` | fused_quant, gemv_norm_epilogue | x2n3, xr3 |
| `Let xr3 be xo2 plus ao3.` | #24 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | ao3, x2n3 |
| `Let x2n3 be rmsnorm of xr3 with weight fn3.` | #24 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | ao3, xr3 |
| `Let gt3 be x2n3 matmul Wg3.` | #25 `gemv_glu` | fused_quant, glu_pair | hh3, up3 |
| `Let up3 be x2n3 matmul Wu3.` | #25 `gemv_glu` | fused_quant, glu_pair | gt3, hh3 |
| `Let hh3 be swiglu of gt3 and up3.` | #25 `gemv_glu` | fused_quant, glu_pair | gt3, up3 |
| `Let dn3 be hh3 matmul Wd3.` | #26 `gemv_norm` | fused_quant, gemv_norm_epilogue | xn4, xo3 |
| `Let xo3 be xr3 plus dn3.` | #26 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | dn3, xn4 |

