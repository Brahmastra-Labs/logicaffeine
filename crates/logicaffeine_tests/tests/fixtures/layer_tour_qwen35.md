## Qwen3.5 (tiny), layer 0: a gated-delta layer

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
| `Let qn0 be l2norm of qs0 across 2 heads.` | #2 `conv` | conv_parts, conv_silu_l2 | kc0, kn0, ks0, qc0, qs0, vc0, vs0 |
| `Let kn0 be l2norm of ks0 across 2 heads.` | #2 `conv` | conv_parts, conv_silu_l2 | kc0, ks0, qc0, qn0, qs0, vc0, vs0 |
| `Let br0 be xn0 matmul Wb0.` | #3 `gates` | gate_projections | ar0, bt0, dy0 |
| `Let bt0 be sigmoid of br0.` | #3 `gates` | gate_projections, gates | ar0, br0, dy0 |
| `Let ar0 be xn0 matmul Wa0.` | #3 `gates` | gate_projections | br0, bt0, dy0 |
| `Let dy0 be decay of ar0 with bias Dt0 rate Al0.` | #3 `gates` | gate_projections, gates | ar0, br0, bt0 |
| `Let od0 be gated delta of qn0 over kn0 and vs0 with gate zz0 beta bt0 decay dy0 state St0 norm Gn0 with 4 heads 2 groups.` | #4 `delta` | fused_quant | — |
| `Let ao0 be od0 matmul Wo0.` | #5 `gemv_norm` | fused_quant, gemv_norm_epilogue | x2n0, xr0 |
| `Let xr0 be x plus ao0.` | #5 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | ao0, x2n0 |
| `Let x2n0 be rmsnorm of xr0 with weight fn0.` | #5 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | ao0, xr0 |
| `Let gt0 be x2n0 matmul Wg0.` | #6 `gemv_glu` | fused_quant, glu_pair | hh0, up0 |
| `Let up0 be x2n0 matmul Wu0.` | #6 `gemv_glu` | fused_quant, glu_pair | gt0, hh0 |
| `Let hh0 be swiglu of gt0 and up0.` | #6 `gemv_glu` | fused_quant, glu_pair | gt0, up0 |
| `Let dn0 be hh0 matmul Wd0.` | #7 `gemv_norm` | fused_quant, gemv_norm_epilogue | xn1, xo0 |
| `Let xo0 be xr0 plus dn0.` | #7 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | dn0, xn1 |

## Qwen3.5 (tiny), layer 2: a gated-attention layer

22 statements compute; 18 declare its weights, state and inputs.

| LOGOS | CUDA launch | fused by | in the same launch |
|---|---|---|---|
| `Let xn2 be rmsnorm of xo1 with weight an2.` | #14 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | dn1, xo1 |
| `Let qr2 be xn2 matmul Wq2.` | #15 `gemv_rope` | gemv_parts, qkv_head_epilogue | Kc2, Vc2, gr2, km2, kp2, kr2, qm2, qp2, vr2 |
| `Let gr2 be xn2 matmul Wqg2.` | #15 `gemv_rope` | gemv_parts | Kc2, Vc2, km2, kp2, kr2, qm2, qp2, qr2, vr2 |
| `Let kr2 be xn2 matmul Wk2.` | #15 `gemv_rope` | gemv_parts, qkv_head_epilogue | Kc2, Vc2, gr2, km2, kp2, qm2, qp2, qr2, vr2 |
| `Let vr2 be xn2 matmul Wv2.` | #15 `gemv_rope` | gemv_parts, row_write | Kc2, Vc2, gr2, km2, kp2, kr2, qm2, qp2, qr2 |
| `Let qm2 be rmsnorm of qr2 with weight Qn2 across 2 heads.` | #15 `gemv_rope` | gemv_parts, head_norm_rope, qkv_head_epilogue | Kc2, Vc2, gr2, km2, kp2, kr2, qp2, qr2, vr2 |
| `Let km2 be rmsnorm of kr2 with weight Kn2 across 1 heads.` | #15 `gemv_rope` | gemv_parts, head_norm_rope, qkv_head_epilogue, row_write | Kc2, Vc2, gr2, kp2, kr2, qm2, qp2, qr2, vr2 |
| `Let qp2 be rope of qm2 with cos Rcos and sin Rsin from position pos across 2 heads.` | #15 `gemv_rope` | gemv_parts, head_norm_rope, qkv_head_epilogue | Kc2, Vc2, gr2, km2, kp2, kr2, qm2, qr2, vr2 |
| `Let kp2 be rope of km2 with cos Rcos and sin Rsin from position pos across 1 heads.` | #15 `gemv_rope` | gemv_parts, head_norm_rope, qkv_head_epilogue, row_write | Kc2, Vc2, gr2, km2, kr2, qm2, qp2, qr2, vr2 |
| `Set row pos of Kc2 to kp2.` | #15 `gemv_rope` | gemv_parts, qkv_head_epilogue, row_write | Vc2, gr2, km2, kp2, kr2, qm2, qp2, qr2, vr2 |
| `Set row pos of Vc2 to vr2.` | #15 `gemv_rope` | gemv_parts, row_write | Kc2, gr2, km2, kp2, kr2, qm2, qp2, qr2, vr2 |
| `Let ct2 be attention of qp2 over Kc2 and Vc2 with 2 heads 1 groups up to pos.` | #16 `attention` | attn_output_gate, fused_quant | cg2, gs2 |
| `Let gs2 be sigmoid of gr2.` | #16 `attention` | act_gate, attn_output_gate, fused_quant | cg2, ct2 |
| `Let cg2 be ct2 hadamard gs2.` | #16 `attention` | act_gate, attn_output_gate, fused_quant | ct2, gs2 |
| `Let ao2 be cg2 matmul Wo2.` | #17 `gemv_norm` | fused_quant, gemv_norm_epilogue | x2n2, xr2 |
| `Let xr2 be xo1 plus ao2.` | #17 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | ao2, x2n2 |
| `Let x2n2 be rmsnorm of xr2 with weight fn2.` | #17 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | ao2, xr2 |
| `Let gt2 be x2n2 matmul Wg2.` | #18 `gemv_glu` | fused_quant, glu_pair | hh2, up2 |
| `Let up2 be x2n2 matmul Wu2.` | #18 `gemv_glu` | fused_quant, glu_pair | gt2, hh2 |
| `Let hh2 be swiglu of gt2 and up2.` | #18 `gemv_glu` | fused_quant, glu_pair | gt2, up2 |
| `Let dn2 be hh2 matmul Wd2.` | #19 `gemv_norm` | fused_quant, gemv_norm_epilogue | xf, xo2 |
| `Let xo2 be xr2 plus dn2.` | #19 `gemv_norm` | add_norm, fused_quant, gemv_norm_epilogue | dn2, xf |

