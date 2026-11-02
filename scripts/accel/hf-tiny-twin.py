#!/usr/bin/env python3
"""A tiny random Hugging Face checkpoint of a model family and the GGUF llama.cpp's converter writes from it.

    hf-tiny-twin.py <family> <out-dir>

writes <out-dir>/hf/ (config.json, sharded safetensors with their index, the family's real tokenizer files; bf16, or
f16 for an `-f16` family) and <out-dir>/model-f32.gguf (convert_hf_to_gguf.py at the pinned llama.cpp, --outtype
f32). The Hugging Face intake reads the first and is held to the second: the converter is the oracle of what a
checkpoint means to the GGUF-shaped spec.

A twin keeps its family's real vocabulary and rotation, routing and window settings, and shrinks every width. Every
parameter is drawn from a seeded normal (norm weights around 1), so a transform the converter applies (a q/k
permutation, an expert stack, a norm offset) is visible in the values.

Runs in a Python with torch and transformers (LOGOS_PYTHON); the tokenizers come from external/hf-tokenizers/
(scripts/accel/fetch-hf-tokenizers.sh).
"""
import json
import shutil
import subprocess
import sys
from pathlib import Path

import torch
import transformers

ROOT = Path(__file__).resolve().parents[2]
LLAMA_CPP = ROOT / "external" / "competitors" / "llama.cpp"
TOKENIZERS = ROOT / "external" / "hf-tokenizers"

COMMON = dict(hidden_size=64, num_attention_heads=4, num_key_value_heads=2, head_dim=16, rms_norm_eps=1e-6)

FAMILIES = {
    # Llama 3.2: tied embeddings, llama3 rotation scaling (the converter writes rope_freqs), interleaved q/k pairs.
    "llama": dict(
        model_type="llama", arch="LlamaForCausalLM", tokenizer="llama",
        config=dict(COMMON, intermediate_size=128, num_hidden_layers=2, vocab_size=128256, rms_norm_eps=1e-5,
                    max_position_embeddings=131072, tie_word_embeddings=True,
                    rope_parameters=dict(rope_type="llama3", rope_theta=500000.0, factor=32.0, low_freq_factor=1.0,
                                         high_freq_factor=4.0, original_max_position_embeddings=8192)),
    ),
    # Llama 4 (Scout's text model): three rotated layers in chunks of 8192 positions with a weightless QK norm, then a
    # rotation-free layer whose queries take the attention temperature; experts on every other layer (one of four a
    # token, its input scaled by its router's sigmoid) beside a shared expert; Scout's llama3 rotation scaling (the
    # converter writes rope_freqs).
    "llama4": dict(
        model_type="llama4_text", arch="Llama4ForCausalLM", tokenizer="llama4",
        config=dict(COMMON, intermediate_size=32, intermediate_size_mlp=128, num_hidden_layers=4, vocab_size=202048,
                    rms_norm_eps=1e-5, num_local_experts=4, num_experts_per_tok=1, interleave_moe_layer_step=2,
                    use_qk_norm=True, attention_chunk_size=8192, attn_temperature_tuning=True, floor_scale=8192,
                    attn_scale=0.1, max_position_embeddings=131072, tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="llama3", rope_theta=500000.0, factor=16.0, low_freq_factor=1.0,
                                         high_freq_factor=1.0, original_max_position_embeddings=8192)),
    ),
    # Qwen 2.5: biased query, key and value projections, NEOX rotation, the head tied (as the small ones are); its
    # window keys present and switched off, as published.
    "qwen2": dict(
        model_type="qwen2", arch="Qwen2ForCausalLM", tokenizer="qwen2",
        config=dict(COMMON, intermediate_size=128, num_hidden_layers=2, vocab_size=151936,
                    max_position_embeddings=32768, use_sliding_window=False, sliding_window=32768,
                    max_window_layers=2, tie_word_embeddings=True,
                    rope_parameters=dict(rope_type="default", rope_theta=1000000.0)),
    ),
    # Mixtral: Mistral's attention (interleaved q/k pairs, no biases) and four experts a layer, two a token, the kept
    # softmax weights renormalized; no window.
    "mixtral": dict(
        model_type="mixtral", arch="MixtralForCausalLM", tokenizer="mixtral",
        config=dict(COMMON, intermediate_size=32, num_local_experts=4, num_experts_per_tok=2, num_hidden_layers=2,
                    vocab_size=32000, rms_norm_eps=1e-5, max_position_embeddings=32768, sliding_window=None,
                    router_jitter_noise=0.0, tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="default", rope_theta=1000000.0)),
    ),
    # Qwen3: untied head, QK-norm, NEOX rotation.
    "qwen3": dict(
        model_type="qwen3", arch="Qwen3ForCausalLM", tokenizer="qwen3",
        config=dict(COMMON, intermediate_size=128, num_hidden_layers=2, vocab_size=151936,
                    max_position_embeddings=40960, tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="default", rope_theta=1000000.0)),
    ),
    # Qwen3-MoE: four experts, two a token, the top-k renormalized.
    "qwen3moe": dict(
        model_type="qwen3_moe", arch="Qwen3MoeForCausalLM", tokenizer="qwen3",
        config=dict(COMMON, intermediate_size=128, moe_intermediate_size=32, num_experts=4, num_experts_per_tok=2,
                    norm_topk_prob=True, decoder_sparse_step=1, mlp_only_layers=[], num_hidden_layers=2,
                    vocab_size=151936, max_position_embeddings=40960, tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="default", rope_theta=1000000.0)),
    ),
    # Qwen1.5/Qwen2-MoE: Qwen2's biased attention and in every layer four experts, two a token, the kept softmax weights
    # not renormalized, beside a shared expert twice a routed one's width scaled by its sigmoid gate; no `head_dim`,
    # as published.
    "qwen2moe": dict(
        model_type="qwen2_moe", arch="Qwen2MoeForCausalLM", tokenizer="qwen2",
        config=dict({k: v for k, v in COMMON.items() if k != "head_dim"}, intermediate_size=128,
                    moe_intermediate_size=32, shared_expert_intermediate_size=64, num_experts=4,
                    num_experts_per_tok=2, norm_topk_prob=False, decoder_sparse_step=1, mlp_only_layers=[],
                    qkv_bias=True, num_hidden_layers=2, vocab_size=151936, max_position_embeddings=8192,
                    use_sliding_window=False, sliding_window=32768, max_window_layers=21,
                    tie_word_embeddings=False, rope_parameters=dict(rope_type="default", rope_theta=1000000.0)),
    ),
    # Qwen3.5: the multimodal checkpoint (its text under `text_config`), three Gated DeltaNet blocks then gated
    # attention with partial interleaved multimodal rotation.
    "qwen35": dict(
        model_type="qwen3_5", arch="Qwen3_5ForConditionalGeneration", tokenizer="qwen35", mtp_like=3,
        config=dict(
            text_config=dict(COMMON, intermediate_size=128, num_hidden_layers=4, vocab_size=248320,
                             max_position_embeddings=262144, tie_word_embeddings=True, attn_output_gate=True,
                             full_attention_interval=4,
                             layer_types=["linear_attention"] * 3 + ["full_attention"],
                             linear_conv_kernel_dim=4, linear_key_head_dim=16, linear_num_key_heads=2,
                             linear_num_value_heads=4, linear_value_head_dim=16, mtp_num_hidden_layers=1,
                             rope_parameters=dict(rope_type="default", rope_theta=10000000.0,
                                                  partial_rotary_factor=0.25, mrope_interleaved=True,
                                                  mrope_section=[1, 1, 0])),
            vision_config=dict(depth=1, hidden_size=32, intermediate_size=64, num_heads=2, out_hidden_size=64,
                               patch_size=16, spatial_merge_size=2, temporal_patch_size=2,
                               num_position_embeddings=64),
            tie_word_embeddings=True,
        ),
    ),
    # Gemma 3: local and global layers (pattern 6), QK-norm, sandwich norms, GELU, the √d embedding scale.
    "gemma3": dict(
        model_type="gemma3_text", arch="Gemma3ForCausalLM", tokenizer="gemma3",
        config=dict(COMMON, num_key_value_heads=1, intermediate_size=128, num_hidden_layers=7, vocab_size=262144,
                    max_position_embeddings=32768, sliding_window=512, query_pre_attn_scalar=16,
                    layer_types=["sliding_attention"] * 5 + ["full_attention"] + ["sliding_attention"],
                    rope_parameters=dict(full_attention=dict(rope_type="default", rope_theta=1000000.0),
                                         sliding_attention=dict(rope_type="default", rope_theta=10000.0)),
                    hidden_activation="gelu_pytorch_tanh", tie_word_embeddings=True),
    ),
    # Gemma: heads wider than the width over the heads (as 7B's 256 over its 192), one rotation, GELU (the legacy `gelu`
    # name, which transformers reads as the tanh approximation), the √d embedding scale.
    "gemma": dict(
        model_type="gemma", arch="GemmaForCausalLM", tokenizer="gemma3",
        config=dict(COMMON, num_key_value_heads=1, head_dim=24, intermediate_size=128, num_hidden_layers=2,
                    vocab_size=262144, max_position_embeddings=8192, rope_theta=10000.0, hidden_act="gelu",
                    tie_word_embeddings=True),
    ),
    # Gemma 2: a 4-token window on alternate layers (pattern 2, the first local), soft caps on the scores and the logits
    # low enough to bind on the twin's values, sandwich norms, GELU, the √d embedding scale, one rotation base.
    "gemma2": dict(
        model_type="gemma2", arch="Gemma2ForCausalLM", tokenizer="gemma3",
        config=dict(COMMON, intermediate_size=128, num_hidden_layers=4, vocab_size=262144,
                    max_position_embeddings=8192, sliding_window=4, query_pre_attn_scalar=16,
                    attn_logit_softcapping=0.1, final_logit_softcapping=0.3, rope_theta=10000.0,
                    hidden_activation="gelu_pytorch_tanh", tie_word_embeddings=True),
    ),
    # gpt-oss: biased attention with sinks, a 128-token window on alternate layers, YaRN, four biased experts.
    "gpt-oss": dict(
        model_type="gpt_oss", arch="GptOssForCausalLM", tokenizer="gpt-oss",
        config=dict(COMMON, intermediate_size=32, num_local_experts=4, num_experts_per_tok=2, num_hidden_layers=2,
                    vocab_size=201088, max_position_embeddings=131072, sliding_window=128,
                    layer_types=["sliding_attention", "full_attention"], tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="yarn", rope_theta=150000.0, factor=32.0, beta_fast=32.0,
                                         beta_slow=1.0, original_max_position_embeddings=4096, truncate=False)),
    ),
    # DeepSeek-V3: latent attention with a query LoRA, a dense first block, then sigmoid group-limited routing with a
    # selection bias, a shared expert and a routed scale; YaRN.
    "deepseek2": dict(
        model_type="deepseek_v3", arch="DeepseekV3ForCausalLM", tokenizer="deepseek2",
        config=dict(hidden_size=64, num_attention_heads=4, num_key_value_heads=4, intermediate_size=128,
                    moe_intermediate_size=32, n_routed_experts=4, n_shared_experts=1, num_experts_per_tok=2,
                    n_group=2, topk_group=1, first_k_dense_replace=1, num_hidden_layers=2, kv_lora_rank=16,
                    q_lora_rank=24, qk_nope_head_dim=12, qk_rope_head_dim=8, v_head_dim=20, vocab_size=129280,
                    routed_scaling_factor=2.5, norm_topk_prob=True, scoring_func="sigmoid", topk_method="noaux_tc",
                    rms_norm_eps=1e-6, max_position_embeddings=163840, tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="yarn", rope_theta=10000.0, factor=40.0, beta_fast=32.0,
                                         beta_slow=1.0, mscale=1.0, mscale_all_dim=1.0,
                                         original_max_position_embeddings=4096)),
    ),
    # Phi-4: Phi-3's one query-key-value projection and one gate-and-up projection, NEOX rotation over the whole head,
    # no window (`sliding_window` None, which the converter writes as 0) and an untied head.
    "phi3": dict(
        model_type="phi3", arch="Phi3ForCausalLM", tokenizer="phi4",
        config=dict({k: v for k, v in COMMON.items() if k != "head_dim"}, intermediate_size=32, num_hidden_layers=2,
                    vocab_size=100352, rms_norm_eps=1e-5, max_position_embeddings=16384,
                    original_max_position_embeddings=16384, sliding_window=None, tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="default", rope_theta=250000.0)),
    ),
    # GLM-4 (0414, the 9B's biased attention): half of each head rotated by interleaved pairs, sandwich norms around the
    # attention and the FFN, one gate-and-up projection, an untied head.
    "glm4": dict(
        model_type="glm4", arch="Glm4ForCausalLM", tokenizer="glm4",
        config=dict(COMMON, intermediate_size=32, num_hidden_layers=2, vocab_size=151552, rms_norm_eps=1e-5,
                    max_position_embeddings=32768, partial_rotary_factor=0.5, attention_bias=True,
                    tie_word_embeddings=False, rope_parameters=dict(rope_type="default", rope_theta=10000.0)),
    ),
    # GLM-4.5: biased projections with per-head QK norms, half of each head rotated (NEOX), heads wider than the model
    # over its width; a dense first block, then DeepSeek's mixture in every block — eight experts scored by sigmoids,
    # two a token chosen by score plus a routing bias (a buffer, drawn here as the parameters are), renormalized and
    # scaled by 2.5, beside one shared expert — and the draft block a published checkpoint carries after the stack.
    "glm4moe": dict(
        model_type="glm4_moe", arch="Glm4MoeForCausalLM", tokenizer="glm45", nextn_like=2,
        random_buffers=["e_score_correction_bias"],
        config=dict(COMMON, head_dim=32, intermediate_size=64, moe_intermediate_size=32, n_routed_experts=8,
                    num_experts_per_tok=2, n_shared_experts=1, first_k_dense_replace=1, num_hidden_layers=3,
                    routed_scaling_factor=2.5, norm_topk_prob=True, n_group=1, topk_group=1, use_qk_norm=True,
                    attention_bias=True, partial_rotary_factor=0.5, vocab_size=151552, rms_norm_eps=1e-5,
                    num_nextn_predict_layers=1, max_position_embeddings=131072, tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="default", rope_theta=1000000.0)),
    ),
    # MiniMax-M2: its query and key each normed across every head of the layer at once (`qk_norm_type` "per_layer"),
    # half of each head rotated, and in every block experts scored by sigmoids, chosen by score plus a routing bias (a
    # buffer, drawn here as the parameters are) and weighed by score, the kept weights renormalized; no shared expert.
    "minimax_m2": dict(
        model_type="minimax_m2", arch="MiniMaxM2ForCausalLM", tokenizer="minimax",
        random_buffers=["e_score_correction_bias"],
        config=dict(COMMON, intermediate_size=32, num_local_experts=4, num_experts_per_tok=2, num_hidden_layers=2,
                    vocab_size=200064, rotary_dim=8, use_qk_norm=True, qk_norm_type="per_layer",
                    scoring_func="sigmoid", use_routing_bias=True, shared_intermediate_size=0,
                    max_position_embeddings=196608, tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="default", rope_theta=5000000.0, partial_rotary_factor=0.5)),
    ),
}

# Qwen3.5-MoE: Qwen3.5's checkpoint with Qwen's mixture in every block — four experts, two a token, renormalized —
# beside a shared expert twice a routed one's width under its sigmoid gate; its draft block stores its experts as one
# gate-and-up tensor.
FAMILIES["qwen35moe"] = dict(
    FAMILIES["qwen35"], model_type="qwen3_5_moe", arch="Qwen3_5MoeForConditionalGeneration",
    config=dict(FAMILIES["qwen35"]["config"], text_config={
        **{k: v for k, v in FAMILIES["qwen35"]["config"]["text_config"].items() if k != "intermediate_size"},
        "moe_intermediate_size": 32, "shared_expert_intermediate_size": 64, "num_experts": 4, "num_experts_per_tok": 2,
    }),
)

# Qwen3.8-2.4T-A95B as it is published: Qwen3.5-MoE's text model alone (`Qwen3_5MoeForCausalLM`, its configuration
# flat, its decoder under `model.`), untied, a quarter of each head rotated with no multimodal sections, Gated
# DeltaNet's gate named swish, each block's experts stored as transformers 5 holds them (one gate-and-up tensor and one
# down tensor), and its draft block beside it.
FAMILIES["qwen35moe-causal"] = dict(
    model_type="qwen3_5_moe_text", arch="Qwen3_5MoeForCausalLM", tokenizer="qwen35", mtp_like=3, fused_experts=True,
    config={
        **FAMILIES["qwen35moe"]["config"]["text_config"],
        "tie_word_embeddings": False,
        "partial_rotary_factor": 0.25,
        "output_gate_type": "swish",
        "rope_parameters": dict(rope_type="default", rope_theta=10000000.0, partial_rotary_factor=0.25),
    },
)

# Qwen3.8-Flash-Next (`qwen4_exp`): Qwen3.5's hybrid with hyper-connections in place of every norm (four lanes over a
# rank of 16), QSA on its full-attention layer (two index heads of 16 over one key head, a budget of 8 tokens in blocks
# of 4), the per-layer n-gram embedding on layer 2 (trigrams, two heads an n-gram over the primes past 1,000, its table
# stored in four shards), Gated DeltaNet's sigmoid gate, Qwen's mixture in every block, untied, and its draft block.
FAMILIES["qwen4exp"] = dict(
    model_type="qwen4_exp", arch="Qwen4ExpForConditionalGeneration", tokenizer="qwen35", qwen4_mtp_like=3,
    published_layer_types=True,
    config=dict(
        text_config=dict(
            COMMON, num_hidden_layers=4, vocab_size=248320, max_position_embeddings=262144, tie_word_embeddings=False,
            bos_token_id=248044, eos_token_id=248044, full_attention_interval=4,
            layer_types=["linear_attention"] * 3 + ["full_attention"],
            linear_conv_kernel_dim=4, linear_key_head_dim=16, linear_num_key_heads=2, linear_num_value_heads=4,
            linear_value_head_dim=16, output_gate_type="sigmoid",
            moe_intermediate_size=32, shared_expert_intermediate_size=64, num_experts=4, num_experts_per_tok=2,
            hc_count=4, hc_lowrank=16,
            indexer_n_heads=2, indexer_kv_heads=1, indexer_head_dim=16, indexer_budget=8, indexer_compress_ratio=4,
            ple_layer_ids=[2], ple_embed_dim=32, ple_conv_kernel_size=4, ngram_size=3, heads_per_ngram=2,
            ngram_vocab_size_base=1000, make_ngram_vocab_size_divisible_by=128, split_ngram_parts=4,
            mtp_num_hidden_layers=1, partial_rotary_factor=0.25,
            rope_parameters=dict(rope_type="default", rope_theta=10000000.0, partial_rotary_factor=0.25,
                                 mrope_interleaved=True, mrope_section=[1, 1, 0])),
        vision_config=dict(depth=1, hidden_size=32, intermediate_size=64, num_heads=2, out_hidden_size=64,
                           patch_size=16, spatial_merge_size=2, temporal_patch_size=2, num_position_embeddings=64),
        tie_word_embeddings=False,
    ),
)

# Qwen3-Next: Qwen3.5's hybrid of Gated DeltaNet and gated attention (a partial NEOX rotation, no multimodal sections)
# with Qwen's mixture in every block — four experts, two a token, renormalized, beside a gated shared expert; its value
# heads grouped by key head and its beta and alpha one projection, and the draft block its converter requires. Six
# value heads over two key heads: at as many value heads a key head as key heads, the reorder between grouped and tiled
# value heads is its own inverse, and a presentation that reorders the wrong way reads the right weights.
FAMILIES["qwen3next"] = dict(
    model_type="qwen3_next", arch="Qwen3NextForCausalLM", tokenizer="qwen3", mtp_like=3,
    config=dict(COMMON, intermediate_size=128, num_hidden_layers=4, vocab_size=151936, max_position_embeddings=262144,
                tie_word_embeddings=False, layer_types=["linear_attention"] * 3 + ["full_attention"],
                full_attention_interval=4, linear_conv_kernel_dim=4, linear_key_head_dim=16, linear_num_key_heads=2,
                linear_num_value_heads=6, linear_value_head_dim=16, moe_intermediate_size=32,
                shared_expert_intermediate_size=64, num_experts=4, num_experts_per_tok=2, norm_topk_prob=True,
                decoder_sparse_step=1, mlp_only_layers=[],
                rope_parameters=dict(rope_type="default", rope_theta=10000000.0, partial_rotary_factor=0.25)),
)

# The same twins stored in f16. The converter widens every dtype but f16 and f32 to f32 before it computes a norm's
# `1 + w` or a decay's `-exp(A_log)`, so only an f16 checkpoint has those values rounded by the converter: the one
# place the converted GGUF and the checkpoint's own f32 forward part.
for _base in ("gemma3", "qwen35"):
    FAMILIES[f"{_base}-f16"] = dict(FAMILIES[_base], dtype="float16")

# Gemma 3's and Llama 4's multimodal checkpoints, the only form their larger releases are published in: the text model
# under the wrapper's prefix beside a tiny vision tower and its projector, which the converter skips.
FAMILIES["gemma3-mm"] = dict(
    model_type="gemma3", arch="Gemma3ForConditionalGeneration", tokenizer="gemma3",
    config=dict(text_config=dict(FAMILIES["gemma3"]["config"]),
                vision_config=dict(hidden_size=32, intermediate_size=64, num_hidden_layers=1, num_attention_heads=2,
                                   image_size=56, patch_size=14),
                mm_tokens_per_image=4, tie_word_embeddings=True),
)
FAMILIES["llama4-mm"] = dict(
    model_type="llama4", arch="Llama4ForConditionalGeneration", tokenizer="llama4",
    config=dict(text_config=dict(FAMILIES["llama4"]["config"]),
                vision_config=dict(hidden_size=32, intermediate_size=64, num_hidden_layers=1, num_attention_heads=2,
                                   image_size=56, patch_size=14, vision_output_dim=64, projector_input_dim=64,
                                   projector_output_dim=64, pixel_shuffle_ratio=0.5),
                tie_word_embeddings=False),
)

# Gemma 3 with only its SentencePiece model, as an older checkpoint ships it: no tokenizer_config.json lists the special
# tokens, so every piece's type is the model's own (its control pieces stay control).
FAMILIES["gemma3-spm"] = dict(FAMILIES["gemma3"], tokenizer_files=["tokenizer.model"])

# The converter reads Qwen2's and Qwen3's SentencePiece model when a directory ships one, and Qwen3-MoE's
# tokenizer.json always (its class has no SentencePiece reader): Qwen2 with Mistral's SentencePiece files and vocabulary,
# and Qwen3-MoE with a stray SentencePiece model beside its own files.
FAMILIES["qwen2-spm"] = dict(FAMILIES["qwen2"], tokenizer="mixtral",
                             config=dict(FAMILIES["qwen2"]["config"], vocab_size=32000))
FAMILIES["qwen3moe-spm"] = dict(FAMILIES["qwen3moe"], stray=("mixtral", "tokenizer.model"))

# Kimi K2: DeepSeek-V3's architecture with one expert group (no group limit), a dense first block and a 2.827 routed
# scale, YaRN with Kimi's narrow ramp over a 4,096-position original context, and the training settings its
# configuration carries. Its vocabulary is DeepSeek-V3's: the converter reads Kimi's own by running Kimi's tokenizer
# code, which a twin build does not run; `model_hf_intake_lock` holds Kimi's vocabulary to its published GGUF instead,
# and the deepseek2-fp8 twin holds its FP8 release's storage.
FAMILIES["kimi_k2"] = dict(
    model_type="deepseek_v3", arch="DeepseekV3ForCausalLM", tokenizer="deepseek2",
    config=dict(hidden_size=64, num_attention_heads=4, num_key_value_heads=4, intermediate_size=128,
                moe_intermediate_size=32, n_routed_experts=8, n_shared_experts=1, num_experts_per_tok=2, n_group=1,
                topk_group=1, first_k_dense_replace=1, num_hidden_layers=2, kv_lora_rank=16, q_lora_rank=24,
                qk_nope_head_dim=12, qk_rope_head_dim=8, v_head_dim=12, vocab_size=163840, routed_scaling_factor=2.827,
                norm_topk_prob=True, scoring_func="sigmoid", topk_method="noaux_tc", aux_loss_alpha=0.001, seq_aux=True,
                moe_layer_freq=1, ep_size=1, rms_norm_eps=1e-6, max_position_embeddings=131072,
                tie_word_embeddings=False,
                rope_parameters=dict(rope_type="yarn", rope_theta=50000.0, factor=32.0, beta_fast=1.0, beta_slow=1.0,
                                     mscale=1.0, mscale_all_dim=1.0, original_max_position_embeddings=4096)),
)

# DeepSeek-V3 as its FP8 release stores it: every projection in e4m3 beside a float32 scale per block (blocks of 16 × 24
# here, which divide few of the twin's axes, where the release's are 128 × 128).
FAMILIES["deepseek2-fp8"] = dict(FAMILIES["deepseek2"], fp8_blocks=[16, 24])

# Qwen3 with its tensors stored in five dtypes at once, as a checkpoint may mix them: each name's dtype by the first
# rule it meets.
FAMILIES["qwen3-mixed"] = dict(FAMILIES["qwen3"], mixed=[
    ("embed_tokens", "float32"),
    ("lm_head", "float8_e5m2"),
    ("norm", "float32"),
    ("q_proj", "float16"),
    ("k_proj", "float16"),
    ("v_proj", "float8_e4m3fn"),
    ("o_proj", "float8_e4m3fn"),
    ("gate_proj", "float16"),
    ("up_proj", "float8_e4m3fn"),
])


def mix_dtypes(hf: Path, rules) -> None:
    """Rewrite every shard with each tensor in the dtype its name's first rule gives (bf16 when none does). An 8-bit
    float tensor's first 256 elements are then every code of its format, NaNs, infinities and subnormals among them."""
    from safetensors.torch import load_file, save_file
    for shard in sorted(hf.glob("*.safetensors")):
        tensors = load_file(shard)
        for name, t in tensors.items():
            dtype = next((d for key, d in rules if key in name), None)
            if dtype:
                t = t.to(getattr(torch, dtype))
                if t.element_size() == 1 and t.numel() >= 256:
                    t.view(torch.uint8).view(-1)[:256] = torch.arange(256, dtype=torch.uint8)
                tensors[name] = t
        save_file(tensors, shard, metadata={"format": "pt"})


def fuse_experts(hf: Path) -> None:
    """Every block's experts stored as transformers 5 holds them and Qwen3.8-2.4T publishes them, in place of a tensor
    per expert and projection: one gate-and-up tensor `[experts, 2·ff, d]`, the gate's rows first, and one down tensor
    `[experts, d, ff]`, written as one more shard and named in the index."""
    import re
    from safetensors.torch import load_file, save_file
    index_path = hf / "model.safetensors.index.json"
    index = json.loads(index_path.read_text())
    experts = {}
    for shard in sorted({hf / f for f in index["weight_map"].values()}):
        tensors, kept = load_file(shard), {}
        for name, t in tensors.items():
            m = re.fullmatch(r"(.+\.mlp\.experts)\.(\d+)\.(gate|up|down)_proj\.weight", name)
            if m:
                experts.setdefault(m[1], {})[(int(m[2]), m[3])] = t
                del index["weight_map"][name]
            else:
                kept[name] = t
        save_file(kept, shard, metadata={"format": "pt"})
    fused = {}
    for base, parts in sorted(experts.items()):
        n = 1 + max(e for e, _ in parts)
        fused[f"{base}.gate_up_proj"] = torch.stack([torch.cat([parts[(e, "gate")], parts[(e, "up")]]) for e in range(n)])
        fused[f"{base}.down_proj"] = torch.stack([parts[(e, "down")] for e in range(n)])
    save_file(fused, hf / "model-experts.safetensors", metadata={"format": "pt"})
    for k in fused:
        index["weight_map"][k] = "model-experts.safetensors"
    index_path.write_text(json.dumps(index, indent=2))


def publish_layer_types(hf: Path) -> None:
    """The configuration's layer types as Qwen publishes them: transformers saves a QSA layer as `indexed_attention`,
    its own name, where the published configuration and llama.cpp's converter say `full_attention`."""
    path = hf / "config.json"
    config = json.loads(path.read_text())
    text = config.get("text_config", config)
    text["layer_types"] = ["full_attention" if t == "indexed_attention" else t for t in text["layer_types"]]
    path.write_text(json.dumps(config, indent=2))


def fp8_blocks(hf: Path, block) -> None:
    """Every projection weight (`*_proj.weight`, a matrix) stored as an FP8 release stores it: e4m3 codes beside a
    float32 scale per block, `<name>_scale_inv` (the factor the codes are multiplied back by), each block's scale its
    largest magnitude over e4m3's 448; and `quantization_config` naming the method and block in the configuration."""
    from safetensors.torch import load_file, save_file
    index_path = hf / "model.safetensors.index.json"
    index = json.loads(index_path.read_text())
    b0, b1 = block
    for shard in sorted(hf.glob("*.safetensors")):
        tensors = load_file(shard)
        for name in [n for n in tensors if n.endswith("_proj.weight") and tensors[n].dim() == 2]:
            w = tensors[name].float()
            rows, cols = w.shape
            scale = torch.ones(-(-rows // b0), -(-cols // b1))
            codes = torch.empty(rows, cols, dtype=torch.float8_e4m3fn)
            for i in range(scale.shape[0]):
                for j in range(scale.shape[1]):
                    part = w[i * b0:(i + 1) * b0, j * b1:(j + 1) * b1]
                    largest = part.abs().max()
                    if largest > 0:
                        scale[i, j] = largest / 448.0
                    codes[i * b0:(i + 1) * b0, j * b1:(j + 1) * b1] = (part / scale[i, j]).to(torch.float8_e4m3fn)
            tensors[name] = codes
            tensors[f"{name}_scale_inv"] = scale
            index["weight_map"][f"{name}_scale_inv"] = shard.name
        save_file(tensors, shard, metadata={"format": "pt"})
    index_path.write_text(json.dumps(index, indent=2))
    config = json.loads((hf / "config.json").read_text())
    config["quantization_config"] = dict(activation_scheme="dynamic", fmt="e4m3", quant_method="fp8",
                                         weight_block_size=[b0, b1])
    (hf / "config.json").write_text(json.dumps(config, indent=2))


def add_mtp(model, hf: Path, like: int, generator: torch.Generator, dtype: torch.dtype) -> None:
    """The draft block a published Qwen3.5 checkpoint carries beside its model (`mtp.*`), which transformers does not
    build: one decoder layer shaped as full-attention layer `like`, the projection of [embedding | hidden] (`fc`, d × 2d)
    and its three norms, written as one more shard and named in the index."""
    from safetensors.torch import save_file
    state = model.state_dict()
    prefix = next(k for k in state if k.endswith(f"layers.{like}.input_layernorm.weight")).rsplit(f"layers.{like}.", 1)[0]
    d = state[f"{prefix}layers.{like}.input_layernorm.weight"].shape[0]
    draw = lambda shape: (1.0 + 0.1 * torch.randn(shape, generator=generator)) if len(shape) == 1 else 0.05 * torch.randn(shape, generator=generator)
    mtp = {f"mtp.layers.0.{k[len(prefix) + len(f'layers.{like}.'):]}": draw(v.shape).to(dtype)
           for k, v in state.items() if k.startswith(f"{prefix}layers.{like}.")}
    mtp["mtp.fc.weight"] = draw((d, 2 * d)).to(dtype)
    for norm in ("norm", "pre_fc_norm_embedding", "pre_fc_norm_hidden"):
        mtp[f"mtp.{norm}.weight"] = draw((d,)).to(dtype)
    save_file(mtp, hf / "model-mtp.safetensors", metadata={"format": "pt"})
    index_path = hf / "model.safetensors.index.json"
    index = json.loads(index_path.read_text())
    for k in mtp:
        index["weight_map"][k] = "model-mtp.safetensors"
    index_path.write_text(json.dumps(index, indent=2))


def add_qwen4_mtp(model, hf: Path, like: int, generator: torch.Generator, dtype: torch.dtype) -> None:
    """The draft block a published Qwen3.8-Flash-Next checkpoint carries beside its model (`mtp.*`), which transformers
    does not build: one decoder layer shaped as full-attention layer `like` (its hyper-connections, its QSA indexer, its
    experts as one gate-and-up and one down tensor), the projections of the embedding and of the hidden lanes
    (`fc_embedding`, `fc_hidden`, d × d each), their norms (the hidden's over every lane) and its own hyper-connection
    mixer, written as one more shard and named in the index."""
    from safetensors.torch import save_file
    state = model.state_dict()
    prefix = next(k for k in state if k.endswith(f"layers.{like}.attn_hyper_connection.hc_norm.weight")).rsplit(f"layers.{like}.", 1)[0]
    text = model.config.get_text_config()
    d, lanes, rank = text.hidden_size, text.hc_count, text.hc_lowrank
    draw = lambda shape: (1.0 + 0.1 * torch.randn(shape, generator=generator)) if len(shape) == 1 else 0.05 * torch.randn(shape, generator=generator)
    mtp = {f"mtp.layers.0.{k[len(prefix) + len(f'layers.{like}.'):]}": draw(v.shape).to(dtype)
           for k, v in state.items() if k.startswith(f"{prefix}layers.{like}.")}
    mtp["mtp.fc_embedding.weight"] = draw((d, d)).to(dtype)
    mtp["mtp.fc_hidden.weight"] = draw((d, d)).to(dtype)
    mtp["mtp.pre_fc_norm_embedding.weight"] = draw((d,)).to(dtype)
    mtp["mtp.pre_fc_norm_hidden.weight"] = draw((lanes * d,)).to(dtype)
    mtp["mtp.hyper_connection_mixer.hc_norm.weight"] = draw((lanes * d,)).to(dtype)
    mtp["mtp.hyper_connection_mixer.input_mix_weight_down.weight"] = draw((rank, lanes * d)).to(dtype)
    mtp["mtp.hyper_connection_mixer.input_mix_weight_up.weight"] = draw((lanes * d, rank)).to(dtype)
    save_file(mtp, hf / "model-mtp.safetensors", metadata={"format": "pt"})
    index_path = hf / "model.safetensors.index.json"
    index = json.loads(index_path.read_text())
    for k in mtp:
        index["weight_map"][k] = "model-mtp.safetensors"
    index_path.write_text(json.dumps(index, indent=2))


def add_nextn(model, hf: Path, like: int, generator: torch.Generator, dtype: torch.dtype) -> None:
    """The draft block a published GLM-4.5 checkpoint carries after its stack (`model.layers.<n>.*`, `n` the layer
    count), which transformers does not build: one decoder layer with the tensors layer `like` is saved with (each
    expert its own, as published), the projection of [embedding | hidden] (`eh_proj`, d × 2d), its three norms, and its
    own embedding and head, written as one more shard and named in the index."""
    from safetensors import safe_open
    from safetensors.torch import save_file
    index_path = hf / "model.safetensors.index.json"
    index = json.loads(index_path.read_text())
    n, d, vocab = model.config.num_hidden_layers, model.config.hidden_size, model.config.vocab_size
    draw = lambda shape: (1.0 + 0.1 * torch.randn(shape, generator=generator)) if len(shape) == 1 else 0.05 * torch.randn(shape, generator=generator)
    src, dst = f"model.layers.{like}.", f"model.layers.{n}."
    block = {}
    for k in sorted(k for k in index["weight_map"] if k.startswith(src)):
        with safe_open(hf / index["weight_map"][k], framework="pt") as f:
            shape = tuple(f.get_slice(k).get_shape())
        block[dst + k[len(src):]] = draw(shape).to(dtype)
    block[dst + "eh_proj.weight"] = draw((d, 2 * d)).to(dtype)
    for norm in ("enorm", "hnorm", "shared_head.norm"):
        block[f"{dst}{norm}.weight"] = draw((d,)).to(dtype)
    block[dst + "embed_tokens.weight"] = draw((vocab, d)).to(dtype)
    block[dst + "shared_head.head.weight"] = draw((vocab, d)).to(dtype)
    save_file(block, hf / "model-nextn.safetensors", metadata={"format": "pt"})
    for k in block:
        index["weight_map"][k] = "model-nextn.safetensors"
    index_path.write_text(json.dumps(index, indent=2))


def build(family: str, out: Path) -> None:
    spec = FAMILIES[family]
    torch.manual_seed(0)
    config = transformers.AutoConfig.for_model(spec["model_type"], architectures=[spec["arch"]], **spec["config"])
    model_class = getattr(transformers, spec["arch"])
    model = model_class(config)
    generator = torch.Generator().manual_seed(1)
    with torch.no_grad():
        for _, p in sorted(model.named_parameters()):
            if p.dim() == 1:
                p.copy_(1.0 + 0.1 * torch.randn(p.shape, generator=generator))
            else:
                p.copy_(0.05 * torch.randn(p.shape, generator=generator))
        for name, b in sorted(model.named_buffers()):
            if any(name.endswith(r) for r in spec.get("random_buffers", [])):
                b.copy_(0.1 * torch.randn(b.shape, generator=generator))
    dtype = getattr(torch, spec.get("dtype", "bfloat16"))
    model = model.to(dtype)
    hf = out / "hf"
    if hf.exists():
        shutil.rmtree(hf)
    model.save_pretrained(hf, max_shard_size="2MB", safe_serialization=True)
    if spec.get("fused_experts"):
        fuse_experts(hf)
    if "mtp_like" in spec:
        add_mtp(model, hf, spec["mtp_like"], generator, dtype)
    if "nextn_like" in spec:
        add_nextn(model, hf, spec["nextn_like"], generator, dtype)
    if "qwen4_mtp_like" in spec:
        add_qwen4_mtp(model, hf, spec["qwen4_mtp_like"], generator, dtype)
    if spec.get("published_layer_types"):
        publish_layer_types(hf)
    if "mixed" in spec:
        mix_dtypes(hf, spec["mixed"])
    if "fp8_blocks" in spec:
        fp8_blocks(hf, spec["fp8_blocks"])
    for f in (TOKENIZERS / spec["tokenizer"]).iterdir():
        if f.name != "SOURCE" and f.name in spec.get("tokenizer_files", [f.name]):
            shutil.copy(f, hf / f.name)
    if "stray" in spec:
        shutil.copy(TOKENIZERS / spec["stray"][0] / spec["stray"][1], hf / spec["stray"][1])
    gguf = out / "model-f32.gguf"
    subprocess.run(
        [sys.executable, str(LLAMA_CPP / "convert_hf_to_gguf.py"), str(hf), "--outtype", "f32", "--outfile", str(gguf)],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    (out / "STAMP").write_text(json.dumps({
        "family": family,
        "torch": torch.__version__,
        "transformers": transformers.__version__,
        "script_sha256": __import__("hashlib").sha256(Path(__file__).read_bytes()).hexdigest(),
    }, indent=1))


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in FAMILIES:
        sys.exit(f"usage: hf-tiny-twin.py <{'|'.join(FAMILIES)}> <out-dir>")
    build(sys.argv[1], Path(sys.argv[2]))
