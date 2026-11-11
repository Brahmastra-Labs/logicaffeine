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
(scripts/accel/fetch-hf-tokenizers.sh), or the directory LOGOS_HF_TOKENIZERS names.
"""
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import torch
import transformers

ROOT = Path(__file__).resolve().parents[2]
LLAMA_CPP = ROOT / "external" / "competitors" / "llama.cpp"
TOKENIZERS = Path(os.environ.get("LOGOS_HF_TOKENIZERS", ROOT / "external" / "hf-tokenizers"))

COMMON = dict(hidden_size=64, num_attention_heads=4, num_key_value_heads=2, head_dim=16, rms_norm_eps=1e-6)

# The converter as its own requirements install it (requirements/requirements-convert_hf_to_gguf.txt), without
# mistral-common: where that library is importable, transformers reads the tokenizer of a directory shipping tekken.json
# through mistral-common's backend, whose vocabulary the converter's byte-pair reader (`get_vocab_base`) cannot read, and
# a Mistral 3 directory, published with tokenizer.json beside tekken.json, does not convert. With
# LOGOS_TWIN_TOKENIZER_FLAGS set, a tokenizer's `add_bos_token` and `add_eos_token` are read from tokenizer_config.json
# as the requirements' transformers 4 reads them (transformers 5 drops them beside a tokenizer.json), for a converter
# class that asserts them (DeepSeek-V3.2's). With LOGOS_TWIN_CONFIG_AS_SAVED set, the converter reads config.json as
# the file says it, as the requirements' transformers 4 reads a configuration whose class it has without a later
# version's defaults (Nemotron-H's: transformers 5 adds the mixture's keys, and with them the converter writes
# Nemotron 3's architecture). With LOGOS_TWIN_TOKENIZER_AS_SAVED set, the converter's tokenizer is tokenizer.json's
# pipeline as it stands, as the requirements' transformers 4 builds a fast tokenizer: transformers 5 builds a class it
# knows by name (MiniMax-Text-01's `GPT2Tokenizer`, under a model type it does not know) from the class's own
# pre-tokenizer, whose probe digest the converter's table does not list. The converter's exponentials (Mamba's `-exp(A_log)`) are taken in f64 and rounded once:
# torch's own f32 exp is, on a build with MKL, MKL's `vsExp`, whose last bit varies with the CPU it dispatches for, and
# the reader presents the correctly rounded value. Run as `python -c CONVERT <convert_hf_to_gguf.py> <args>`.
CONVERT = """
import json, os, runpy, sys
sys.modules["mistral_common"] = None
sys.argv = sys.argv[1:]
sys.path.insert(0, sys.argv[0].rsplit("/", 1)[0])
if os.environ.get("LOGOS_TWIN_TOKENIZER_AS_SAVED"):
    import transformers
    def as_saved(path, *args, **kwargs):
        kwargs.pop("trust_remote_code", None)
        return transformers.PreTrainedTokenizerFast.from_pretrained(path, *args, **kwargs)
    transformers.AutoTokenizer.from_pretrained = as_saved
if os.environ.get("LOGOS_TWIN_TOKENIZER_FLAGS"):
    import transformers
    loaded = transformers.AutoTokenizer.from_pretrained
    def from_pretrained(path, *args, **kwargs):
        config = os.path.join(path, "tokenizer_config.json")
        if os.path.isfile(config):
            stated = json.load(open(config))
            for key in ("add_bos_token", "add_eos_token"):
                if key in stated:
                    kwargs.setdefault(key, stated[key])
        return loaded(path, *args, **kwargs)
    transformers.AutoTokenizer.from_pretrained = from_pretrained
import torch
exp = torch.exp
def rounded_exp(x, *args, **kwargs):
    if not args and not kwargs and getattr(x, "dtype", None) in (torch.float32, torch.float16):
        return exp(x.to(torch.float64)).to(torch.float32).to(x.dtype)
    return exp(x, *args, **kwargs)
torch.exp = rounded_exp
if os.environ.get("LOGOS_TWIN_CONFIG_AS_SAVED"):
    import conversion.base
    class AsSaved:
        @staticmethod
        def from_pretrained(*args, **kwargs):
            raise ValueError("config.json read as saved")
    conversion.base.AutoConfig = AsSaved
runpy.run_path(sys.argv[0], run_name="__main__")
"""

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
    # Mistral 7B v0.3 (`MistralForCausalLM`, a `llama` GGUF): Llama's interleaved q/k pairs, no biases, no window
    # (`sliding_window` null, as the release states it), Mistral's SentencePiece vocabulary (Mixtral's 32000 tokens).
    "mistral": dict(
        model_type="mistral", arch="MistralForCausalLM", tokenizer="mixtral",
        config=dict(COMMON, intermediate_size=128, num_hidden_layers=2, vocab_size=32000, rms_norm_eps=1e-5,
                    max_position_embeddings=32768, sliding_window=None, tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="default", rope_theta=1000000.0)),
    ),
    # Ministral 3's text model: Mistral's attention (interleaved q/k pairs, no biases) under YaRN over an original
    # context of 4 positions, and Llama 4's attention temperature on every layer (`llama_4_scaling_beta`, at 0.5 where
    # the releases' 0.1, so it binds), its floor the original context: a prompt of 9 positions crosses two floors.
    "ministral3": dict(
        model_type="ministral3", arch="Ministral3ForCausalLM", tokenizer="ministral3",
        config=dict(COMMON, intermediate_size=128, num_hidden_layers=2, vocab_size=131072, rms_norm_eps=1e-5,
                    max_position_embeddings=64, sliding_window=None, tie_word_embeddings=False,
                    rope_parameters=dict(rope_type="yarn", type="yarn", rope_theta=1000000.0, factor=16.0,
                                         original_max_position_embeddings=4, max_position_embeddings=64,
                                         beta_fast=32.0, beta_slow=1.0, mscale=1.0, mscale_all_dim=1.0,
                                         llama_4_scaling_beta=0.5)),
    ),
    # PhiMoE (`PhiMoEForCausalLM`, the class name its releases publish, built as transformers' `PhimoeForCausalLM`):
    # LayerNorms with their biases, biased attention under a NEOX rotation over the whole head, a 4-token window that a
    # 9-token prompt crosses, four experts routed by sparsemixer at its 0.01 jitter, a biased head; its original context
    # written, as the converter's Phi-3 class requires one.
    "phimoe": dict(
        model_type="phimoe", arch="PhiMoEForCausalLM", model_class="PhimoeForCausalLM", tokenizer="phimoe",
        config=dict({k: v for k, v in COMMON.items() if k != "head_dim"}, intermediate_size=32, num_local_experts=4,
                    num_experts_per_tok=2, num_hidden_layers=2, vocab_size=32064, rms_norm_eps=1e-5,
                    max_position_embeddings=4096, original_max_position_embeddings=4096, sliding_window=4,
                    attention_bias=True, lm_head_bias=True, router_jitter_noise=0.01, input_jitter_noise=0.01,
                    tie_word_embeddings=False, rope_parameters=dict(rope_type="default", rope_theta=10000.0)),
    ),
    # LFM2 (`Lfm2ForCausalLM`): LFM2's gated short convolution of 3 taps on all but the third and fifth of five blocks,
    # which attend with per-head QK norms under a NEOX rotation; the FFN width adjusted as LFM2 adjusts it (two thirds,
    # then up to a multiple of 32); the head tied.
    "lfm2": dict(
        model_type="lfm2", arch="Lfm2ForCausalLM", tokenizer="lfm2",
        config=dict(hidden_size=64, num_attention_heads=4, num_key_value_heads=2, intermediate_size=192,
                    block_multiple_of=32, block_ffn_dim_multiplier=1.0, block_auto_adjust_ff_dim=True,
                    num_hidden_layers=5, conv_L_cache=3, conv_bias=False,
                    layer_types=["conv", "conv", "full_attention", "conv", "full_attention"], vocab_size=65536,
                    norm_eps=1e-5, max_position_embeddings=128000, tie_word_embeddings=True,
                    rope_parameters=dict(rope_type="default", rope_theta=1000000.0)),
    ),
    # Mamba-2 (`Mamba2ForCausalLM`): two blocks of the state-space mixer alone, eight heads of 16 over an inner width of
    # twice the model's, one group of B and C of 16 states, a 4-tap convolution with its bias, the gated norm; GPT-NeoX's
    # vocabulary, padded to a multiple of 16 as the converter pads it; an untied head. One group, since transformers'
    # torch path RMS-normalizes the gated output across the whole inner width where llama.cpp and transformers' CUDA
    # kernel normalize each group's (Mamba-Codestral's eight).
    "mamba2": dict(
        model_type="mamba2", arch="Mamba2ForCausalLM", tokenizer="mamba2",
        config=dict(hidden_size=64, num_heads=8, head_dim=16, expand=2, state_size=16, n_groups=1, conv_kernel=4,
                    num_hidden_layers=2, vocab_size=50288, layer_norm_epsilon=1e-5, use_bias=False,
                    use_conv_bias=True, tie_word_embeddings=False),
    ),
    # Falcon-H1 (`FalconH1ForCausalLM`): in each of three blocks attention under a NEOX rotation beside Mamba-2's scan on
    # one normed input, their outputs summed, then a dense SwiGLU; the scan eight heads of 16 in two groups of B and C of
    # 16 states, with the grouped gated norm (7B's and 34B's `mamba_rms_norm`); every MuP multiplier the converter folds
    # into the weights at Falcon-H1-0.5B's values, but the attention's input multiplier, which is 1 there; an untied head.
    "falcon-h1": dict(
        model_type="falcon_h1", arch="FalconH1ForCausalLM", tokenizer="falcon-h1",
        config=dict(hidden_size=64, num_attention_heads=4, num_key_value_heads=2, head_dim=16, intermediate_size=128,
                    num_hidden_layers=3, vocab_size=32784, rms_norm_eps=1e-5, max_position_embeddings=16384,
                    mamba_d_ssm=128, mamba_n_heads=8, mamba_d_head=16, mamba_n_groups=2, mamba_d_state=16,
                    mamba_d_conv=4, mamba_expand=2, mamba_chunk_size=128, mamba_conv_bias=True, mamba_proj_bias=False,
                    mamba_rms_norm=True, mamba_norm_before_gate=False, embedding_multiplier=5.656854249492381,
                    lm_head_multiplier=0.0390625, mlp_multipliers=[0.8838834764831844, 0.5859375],
                    key_multiplier=0.39062499999999994, attention_in_multiplier=0.75, attention_out_multiplier=0.9375,
                    ssm_in_multiplier=1.25, ssm_out_multiplier=0.23570226039551587,
                    ssm_multipliers=[0.3535533905932738, 0.25, 0.3535533905932738, 0.5, 0.3535533905932738],
                    tie_word_embeddings=False, rope_parameters=dict(rope_type="default", rope_theta=100000000000.0)),
    ),
    # Nemotron-H (`NemotronHForCausalLM`): blocks of one sublayer each under its one norm, Mamba-2's scan (`M`),
    # attention with no rotation (`*`) or an FFN of relu² with no gate (`-`); the scan eight heads of 16 in two groups
    # of B and C of 16 states with the grouped gated norm; Nemotron-Nano-9B-v2's vocabulary. The configuration as the
    # dense checkpoints publish it: the block pattern the string transformers 4 saves, and none of the mixture's keys
    # transformers 5 saves with their defaults (with them, the converter writes Nemotron 3's architecture).
    "nemotron_h": dict(
        model_type="nemotron_h", arch="NemotronHForCausalLM", tokenizer="nemotron_h",
        config=dict(hidden_size=64, num_attention_heads=4, num_key_value_heads=2, head_dim=16, intermediate_size=128,
                    hybrid_override_pattern="M-M*-", num_hidden_layers=5, mamba_num_heads=8, mamba_head_dim=16,
                    ssm_state_size=16, n_groups=2, conv_kernel=4, expand=2, use_conv_bias=True, mamba_proj_bias=False,
                    mlp_hidden_act="relu2", mlp_bias=False, attention_bias=False, layer_norm_epsilon=1e-5,
                    vocab_size=131072, max_position_embeddings=8192, tie_word_embeddings=False),
        published_pattern="M-M*-",
        published_drop=["moe_intermediate_size", "moe_latent_size", "moe_shared_expert_intermediate_size",
                        "moe_shared_expert_overlap", "mtp_layers_block_type", "n_group", "n_routed_experts",
                        "n_shared_experts", "norm_topk_prob", "num_experts_per_tok", "num_nextn_predict_layers",
                        "output_router_logits", "routed_scaling_factor", "topk_group", "mamba_ssm_cache_dtype"],
        config_as_saved=True,
    ),
    # Jamba (`JambaForCausalLM`): Mamba-1's selective scan on blocks 0 and 2 (an inner width of twice the model's, a state
    # of 16, a time step of rank 8, its dt, B and C RMS-normed), attention with no rotation on blocks 1 and 3; four
    # experts, two a token, after each scan block and a dense SwiGLU after each attention block; the head tied, as
    # Jamba-Reasoning-3B's is, and its vocabulary.
    "jamba": dict(
        model_type="jamba", arch="JambaForCausalLM", tokenizer="jamba",
        config=dict(hidden_size=64, num_attention_heads=4, num_key_value_heads=2, intermediate_size=128,
                    num_hidden_layers=4, attn_layer_offset=1, attn_layer_period=2, expert_layer_offset=0,
                    expert_layer_period=2, num_experts=4, num_experts_per_tok=2, mamba_d_state=16, mamba_d_conv=4,
                    mamba_expand=2, mamba_dt_rank=8, mamba_conv_bias=True, mamba_proj_bias=False, rms_norm_eps=1e-6,
                    vocab_size=65536, max_position_embeddings=262144, tie_word_embeddings=True, use_mamba_kernels=False),
    ),
    # Gemma 3n (`Gemma3nForCausalLM`): three blocks, two windowed to 4 under their own base and a global one, their gates
    # sparsified at the 95th percentile (llama.cpp sparsifies the first ten blocks'); per-head QK norms, the values normed
    # with no weight, the scores unscaled; four AltUp streams, LAuReL of 64 and per-layer inputs of 256, the numbers
    # llama.cpp fixes; a softcap of 30 on the logits; the head tied; Gemma 3n's vocabulary.
    "gemma3n": dict(
        model_type="gemma3n_text", arch="Gemma3nForCausalLM", tokenizer="gemma3n",
        config=dict(hidden_size=64, num_attention_heads=4, num_key_value_heads=2, head_dim=16, intermediate_size=128,
                    num_hidden_layers=3, layer_types=["sliding_attention", "sliding_attention", "full_attention"],
                    sliding_window=4, activation_sparsity_pattern=[0.95, 0.95, 0.95], num_kv_shared_layers=0,
                    altup_num_inputs=4, altup_active_idx=0, laurel_rank=64, hidden_size_per_layer_input=256,
                    vocab_size=262400, vocab_size_per_layer_input=262144, final_logit_softcapping=30.0,
                    rms_norm_eps=1e-6, max_position_embeddings=32768, hidden_activation="gelu_pytorch_tanh",
                    rope_parameters=dict(full_attention=dict(rope_type="default", rope_theta=1000000.0),
                                         sliding_attention=dict(rope_type="default", rope_theta=10000.0)),
                    tie_word_embeddings=True),
    ),
    # Gemma 4 as its text model (`Gemma4ForCausalLM`), shaped as Gemma-4-26B-A4B: six blocks, a window of 4 on all but
    # blocks 2 and 5; four heads of 16 over two KV heads in the windowed blocks, of 32 over one in the others, whose
    # values are their keys (`attention_k_eq_v`) and whose rotation is proportional (a quarter of each head turning);
    # every block's dense GELU FFN of 96 beside four experts of 32, two a token, routed by Gemma 4's scaled router with
    # its per-expert scales; each block's output scale drawn about 1, as a parameter is; the scores unscaled, the logits
    # capped at 30, the head tied; Gemma 4's vocabulary.
    "gemma4-text": dict(
        model_type="gemma4_text", arch="Gemma4ForCausalLM", tokenizer="gemma4",
        config=dict(hidden_size=64, intermediate_size=96, num_hidden_layers=6, num_attention_heads=4, num_key_value_heads=2,
                    head_dim=16, global_head_dim=32, num_global_key_value_heads=1, attention_k_eq_v=True,
                    enable_moe_block=True, num_experts=4, top_k_experts=2, moe_intermediate_size=32,
                    hidden_size_per_layer_input=0, num_kv_shared_layers=0, use_double_wide_mlp=False, sliding_window=4,
                    final_logit_softcapping=30.0, vocab_size=262144, vocab_size_per_layer_input=262144, rms_norm_eps=1e-6,
                    max_position_embeddings=262144, hidden_activation="gelu_pytorch_tanh",
                    layer_types=["sliding_attention"] * 2 + ["full_attention"] + ["sliding_attention"] * 2 + ["full_attention"],
                    rope_parameters=dict(full_attention=dict(rope_type="proportional", partial_rotary_factor=0.25, rope_theta=1000000.0),
                                         sliding_attention=dict(rope_type="default", rope_theta=10000.0)),
                    tie_word_embeddings=True),
        drawn_buffers=["layer_scalar"],
    ),
    # Gemma 4 shaped as its E2B and E4B: six blocks, the last two attending the caches of blocks 2 (windowed) and 3
    # (global) and their FFNs twice as wide (`use_double_wide_mlp`); per-layer inputs of 16; one KV head everywhere, a
    # value projection of its own; dense FFNs.
    "gemma4-e-text": dict(
        model_type="gemma4_text", arch="Gemma4ForCausalLM", tokenizer="gemma4",
        config=dict(hidden_size=64, intermediate_size=96, num_hidden_layers=6, num_attention_heads=4, num_key_value_heads=1,
                    head_dim=16, global_head_dim=32, attention_k_eq_v=False, enable_moe_block=False,
                    hidden_size_per_layer_input=16, num_kv_shared_layers=2, use_double_wide_mlp=True, sliding_window=4,
                    final_logit_softcapping=30.0, vocab_size=262144, vocab_size_per_layer_input=262144, rms_norm_eps=1e-6,
                    max_position_embeddings=131072, hidden_activation="gelu_pytorch_tanh",
                    layer_types=["sliding_attention"] * 3 + ["full_attention", "sliding_attention", "full_attention"],
                    rope_parameters=dict(full_attention=dict(rope_type="proportional", partial_rotary_factor=0.25, rope_theta=1000000.0),
                                         sliding_attention=dict(rope_type="default", rope_theta=10000.0)),
                    tie_word_embeddings=True),
        drawn_buffers=["layer_scalar"],
    ),
    # Kimi-Linear (`KimiLinearForCausalLM`): Kimi delta attention on blocks 0 to 2 (four heads of 16, a 4-tap convolution
    # of the queries, keys and values each), latent attention with no rotation on block 3 (a full-rank query, a latent
    # of 32, keys of 16 + 8, values of 16); a dense SwiGLU on block 0, then four experts, two a token, under sigmoid
    # scores with a selection bias, renormalized and scaled by 2.446, beside a shared expert; the configuration as
    # Moonshot publishes it (its `linear_attn_config`, which the converter reads), Kimi's tiktoken vocabulary. Its
    # epsilon is the releases' 10⁻⁵, which every norm takes but the latent's, normed at 10⁻⁶ whatever `rms_norm_eps`
    # says (llama.cpp norms it at the model's).
    "kimi-linear": dict(
        model_type="kimi_linear", arch="KimiLinearForCausalLM", tokenizer="kimi-linear",
        config=dict(hidden_size=64, intermediate_size=128, moe_intermediate_size=32, num_hidden_layers=4,
                    num_attention_heads=4, num_key_value_heads=4, n_shared_experts=1, kv_lora_rank=32, q_lora_rank=None,
                    qk_rope_head_dim=8, qk_nope_head_dim=16, v_head_dim=16, num_local_experts=4,
                    num_experts_per_tok=2, linear_head_dim=16, linear_num_heads=4, linear_conv_kernel_dim=4,
                    layer_types=["linear_attention"] * 3 + ["full_attention"], mlp_layer_types=["dense"] + ["sparse"] * 3,
                    routed_scaling_factor=2.446, rms_norm_eps=1e-5, vocab_size=163840, tie_word_embeddings=False),
        published_config="kimi_linear",
        config_as_saved=True,
        random_buffers=["e_score_correction_bias"],
    ),
    # MiniMax-01 as transformers publishes it (`MiniMaxForCausalLM`, MiniMax-Text-01-hf's class): lightning attention on
    # blocks 0 to 6 (four heads of 16, the queries, keys and values through SiLU, each head's memory decaying at its ALiBi
    # slope scaled by the block's depth, the read normed and gated), attention under the whole head's NEOX rotation on
    # block 7 (every eighth, as llama.cpp fixes it where the GGUF names no recurrent blocks); four experts, two a token,
    # renormalized; each sublayer's residual its normed input times MiniMax-Text-01's 3.5566. The converter registers
    # only MiniMax's own classes, and reads this checkpoint under the configuration MiniMax's code reads
    # (`minimax_text01_config`); MiniMax-Text-01's vocabulary, its tokenizer read as the converter's transformers 4
    # reads it (MiniMax-M2's pre-tokenizer), the embedding's rows past its 200,026 tokens zero as a padded embedding
    # holds them, and one row among the added tokens and one below them zero too: the converter suppresses the zero
    # rows past the tokenizer's own 200,000.
    "minimax-01": dict(
        model_type="minimax", arch="MiniMaxForCausalLM", tokenizer="minimax-01", tokenizer_as_saved=True,
        config=dict(hidden_size=64, intermediate_size=32, num_hidden_layers=8, num_attention_heads=4,
                    num_key_value_heads=2, head_dim=16, rotary_dim=16, num_local_experts=4, num_experts_per_tok=2,
                    layer_types=["linear_attention"] * 7 + ["full_attention"], vocab_size=200064,
                    full_attn_alpha_factor=3.5565588200778455, linear_attn_alpha_factor=3.5565588200778455,
                    mlp_alpha_factor=3.5565588200778455, full_attn_beta_factor=1.0, linear_attn_beta_factor=1.0,
                    mlp_beta_factor=1.0, postnorm=True, rms_norm_eps=1e-5, max_position_embeddings=10240000,
                    rope_parameters=dict(rope_type="default", rope_theta=10000000.0), tie_word_embeddings=False),
        converted_as="minimax_text01",
        config_as_saved=True,
        dropped_buffers=["slope_rate", "query_decay", "key_decay", "diagonal_decay"],
        zero_embedding_rows=[199999, 200005, *range(200026, 200064)],
    ),
    # RWKV-7 as fla-hub publishes it (`RWKV7ForCausalLM`; no transformers class computes it, so the twin is written
    # tensor by tensor): three blocks of two heads of 64 (every release's head width, and the narrowest llama.cpp's
    # vectorized WKV7 kernel steps through whole on a CPU with AVX-512), the decay's, the learning rate's, the value
    # mix's and the gate's low-rank pairs of 16, 16, 8 and 32, a channel mix of 256, LayerNorms with biases; the RWKV
    # world vocabulary.
    "rwkv7": dict(
        written="rwkv7", tokenizer="rwkv7", config_as_saved=True,
        config=dict(architectures=["RWKV7ForCausalLM"], model_type="rwkv7", a_low_rank_dim=16, attn=None, attn_mode="chunk",
                    auto_map={"AutoConfig": "modeling_rwkv7.RWKV7Config", "AutoModelForCausalLM": "modeling_rwkv7.RWKV7ForCausalLM"},
                    bos_token_id=1, decay_low_rank_dim=16, eos_token_id=2, fuse_cross_entropy=True, fuse_norm=False,
                    gate_low_rank_dim=32, head_dim=64, hidden_act="sqrelu", hidden_ratio=4.0, hidden_size=128,
                    initializer_range=0.02, intermediate_size=256, max_position_embeddings=2048, norm_bias=True,
                    norm_eps=1e-5, norm_first=True, num_heads=None, num_hidden_layers=3, tie_word_embeddings=False,
                    torch_dtype="bfloat16", transformers_version="4.48.1", use_cache=True, v_low_rank_dim=8,
                    vocab_size=65536),
    ),
    # RWKV-7 in the Hugging Face port of BlinkDL's checkpoints (`Rwkv7ForCausalLM`): three blocks of two heads of 64,
    # the low-rank pairs' ranks unset (the converter's defaults, 32 each at a width of 128) but the gate's, 16, the
    # channel mix four times the width.
    "rwkv7-hf": dict(
        written="rwkv7-hf", tokenizer="rwkv7", config_as_saved=True,
        config=dict(architectures=["Rwkv7ForCausalLM"], model_type="rwkv7", attention_hidden_size=128,
                    auto_map={"AutoConfig": "configuration_rwkv7.Rwkv7Config", "AutoModelForCausalLM": "modeling_rwkv7.Rwkv7ForCausalLM"},
                    bos_token_id=0, eos_token_id=0, head_size=64, hidden_size=128, intermediate_size=None,
                    layer_norm_epsilon=1e-5, lora_rank_decay=None, lora_rank_gate=16, lora_rank_iclr=None,
                    lora_rank_value_residual_mix=None, num_hidden_layers=3, tie_word_embeddings=False,
                    transformers_version="4.46.2", use_cache=True, vocab_size=65536),
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

# Mistral 3's multimodal checkpoints, the form Ministral 3 and Mistral Small 3.1 and 3.2 are published in: the text
# model under the wrapper's prefix beside a tiny Pixtral tower and its projector, which the converter skips. Ministral
# 3's text model is the twin above; Mistral Small's is Mistral's own, with neither YaRN nor a temperature.
_PIXTRAL = dict(model_type="pixtral", hidden_size=32, intermediate_size=64, num_hidden_layers=1, num_attention_heads=2,
                head_dim=16, image_size=56, patch_size=14)
FAMILIES["ministral3-mm"] = dict(
    model_type="mistral3", arch="Mistral3ForConditionalGeneration", tokenizer="ministral3",
    config=dict(text_config=dict(FAMILIES["ministral3"]["config"], model_type="ministral3"), vision_config=_PIXTRAL,
                spatial_merge_size=2, multimodal_projector_bias=False, projector_hidden_act="gelu",
                vision_feature_layer=-1, tie_word_embeddings=False),
)
FAMILIES["mistral3-small"] = dict(
    model_type="mistral3", arch="Mistral3ForConditionalGeneration", tokenizer="ministral3",
    config=dict(text_config=dict(COMMON, model_type="mistral", intermediate_size=128, num_hidden_layers=2,
                                 vocab_size=131072, rms_norm_eps=1e-5, max_position_embeddings=131072,
                                 sliding_window=None, rope_theta=1000000000.0),
                vision_config=_PIXTRAL, spatial_merge_size=2, multimodal_projector_bias=False,
                projector_hidden_act="gelu", vision_feature_layer=-1, tie_word_embeddings=False),
)

# Gemma 3n as its checkpoints are published (`Gemma3nForConditionalGeneration`): the text model above under the wrapper's
# prefix and configuration, without the towers (a vision tower needs timm), which the converter skips.
FAMILIES["gemma3n-mm"] = dict(FAMILIES["gemma3n"], wrapped=dict(arch="Gemma3nForConditionalGeneration", model_type="gemma3n"))

# Gemma 4 as its checkpoints are published (`Gemma4ForConditionalGeneration`): each text model above under the
# wrapper's prefix and configuration, without the towers, which the converter skips.
FAMILIES["gemma4"] = dict(FAMILIES["gemma4-text"], wrapped=dict(arch="Gemma4ForConditionalGeneration", model_type="gemma4"))
FAMILIES["gemma4-e"] = dict(FAMILIES["gemma4-e-text"], wrapped=dict(arch="Gemma4ForConditionalGeneration", model_type="gemma4"))

# MiniMax-01 as MiniMax-Text-01 and MiniMax-M1 publish it (`MiniMaxText01ForCausalLM`, the class the converter
# registers): the checkpoint above under MiniMax's own configuration, converted as it stands.
FAMILIES["minimax-text-01"] = dict(FAMILIES["minimax-01"], converted_as=None, published_config="minimax_text01")

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

# DeepSeek-V3.2: DeepSeek-V3's twin with DeepSeek's sparse attention on every layer — eight index heads of 16 over the
# query's latent, a key through a LayerNorm, NEOX rotation over the first 8 dimensions — choosing 4 positions, so a 9-token
# prompt attends a choice. Eight heads, so a position every head's ReLU zeroes, whose score ties others' at 0 and whose
# choice no implementation orders alike, is rare. Its released tokenizer_config.json does not add the beginning token, which the converter
# asserts; the twin's says it does, as the converter's message asks, and is read as transformers 4 reads it.
FAMILIES["deepseek32"] = dict(
    model_type="deepseek_v32", arch="DeepseekV32ForCausalLM", tokenizer="deepseek32",
    tokenizer_config=dict(add_bos_token=True), tokenizer_flags=True,
    config=dict({k: v for k, v in FAMILIES["deepseek2"]["config"].items()}, index_n_heads=8, index_head_dim=16,
                index_topk=4, num_nextn_predict_layers=0),
)

# GLM-5.2: GLM-5's latent attention (the query and the latent low-rank, values wider than the keys) with sparse attention
# whose second layer shares the first's choice (`indexer_types` full, shared, full), interleaved rotation in the indexer
# as in the attention; a dense first block, then sigmoid-scored experts chosen with a routing bias, renormalized and
# scaled by 2.5, beside a shared expert. Its epsilon is the releases' 1e-5, which every norm takes but the latents',
# normed at 1e-6 whatever `rms_norm_eps` says (llama.cpp norms them at the model's).
FAMILIES["glm-dsa"] = dict(
    model_type="glm_moe_dsa", arch="GlmMoeDsaForCausalLM", tokenizer="glm5", random_buffers=["e_score_correction_bias"],
    config=dict(hidden_size=64, num_attention_heads=4, num_key_value_heads=4, intermediate_size=128,
                moe_intermediate_size=32, n_routed_experts=4, n_shared_experts=1, num_experts_per_tok=2, n_group=1,
                topk_group=1, first_k_dense_replace=1, num_hidden_layers=3, kv_lora_rank=16, q_lora_rank=24,
                qk_nope_head_dim=12, qk_rope_head_dim=8, v_head_dim=20, vocab_size=154880, routed_scaling_factor=2.5,
                norm_topk_prob=True, scoring_func="sigmoid", topk_method="noaux_tc", index_n_heads=8,
                index_head_dim=16, index_topk=4, indexer_types=["full", "shared", "full"], num_nextn_predict_layers=0,
                rms_norm_eps=1e-5, max_position_embeddings=202752, tie_word_embeddings=False,
                rope_parameters=dict(rope_type="default", rope_theta=1000000.0)),
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


def published_architecture(hf: Path, arch: str) -> None:
    """The configuration's class as the releases publish it, where transformers saves its own class's name."""
    path = hf / "config.json"
    config = json.loads(path.read_text())
    config["architectures"] = [arch]
    path.write_text(json.dumps(config, indent=2))


def publish_layer_types(hf: Path) -> None:
    """The configuration's layer types as Qwen publishes them: transformers saves a QSA layer as `indexed_attention`,
    its own name, where the published configuration and llama.cpp's converter say `full_attention`."""
    path = hf / "config.json"
    config = json.loads(path.read_text())
    text = config.get("text_config", config)
    text["layer_types"] = ["full_attention" if t == "indexed_attention" else t for t in text["layer_types"]]
    path.write_text(json.dumps(config, indent=2))


def drop_buffers(hf: Path, suffixes) -> None:
    """The checkpoint without the buffers transformers saves that a published one leaves for the model to compute (MiniMax's
    lightning decays)."""
    rename_tensors(hf, lambda k: None if any(k.endswith(s) for s in suffixes) else k)


def minimax_text01_config(c: dict) -> dict:
    """A MiniMax configuration transformers saves (`MiniMaxForCausalLM`) as MiniMax's own code reads it and MiniMax-Text-01
    publishes it (`MiniMaxText01ForCausalLM`, the class the converter registers): the attention kinds as `attn_type_list`
    (1 a full-attention block), the residual multipliers by their `layernorm_` names, the rotation's base at the top, the
    normed-input residual (`postnorm`) and no shared expert stated."""
    kinds = {"linear_attention": 0, "full_attention": 1}
    t = {k: v for k, v in c.items() if k not in ("layer_types", "rope_parameters", "block_size") and not k.endswith("_factor")}
    t["architectures"] = ["MiniMaxText01ForCausalLM"]
    t["model_type"] = "minimax_text_01"
    t["attn_type_list"] = [kinds[k] for k in c["layer_types"]]
    for ours, theirs in [("full_attn", "full_attention"), ("linear_attn", "linear_attention"), ("mlp", "mlp")]:
        t[f"layernorm_{theirs}_alpha"] = c[f"{ours}_alpha_factor"]
        t[f"layernorm_{theirs}_beta"] = c[f"{ours}_beta_factor"]
    t["rope_theta"] = c["rope_parameters"]["rope_theta"]
    t["shared_intermediate_size"] = 0
    t["shared_moe_mode"] = "sigmoid"
    return t


def as_minimax_text01(hf: Path, into: Path) -> None:
    """The directory `hf` under MiniMax's own configuration (`minimax_text01_config`) at `into`, its other files linked:
    the same tensors, as the converter reads a MiniMax-01 checkpoint."""
    into.mkdir()
    for f in hf.iterdir():
        if f.name != "config.json":
            os.symlink(f.resolve(), into / f.name)
    c = json.loads((hf / "config.json").read_text())
    (into / "config.json").write_text(json.dumps(minimax_text01_config(c), indent=2))


def publish_minimax_text01(hf: Path) -> None:
    """The checkpoint's configuration as MiniMax-Text-01 publishes it (`minimax_text01_config`)."""
    path = hf / "config.json"
    path.write_text(json.dumps(minimax_text01_config(json.loads(path.read_text())), indent=2))


def rename_tensors(hf: Path, rename) -> None:
    """Every tensor of the directory under the name `rename` gives it (`None` dropping it), its shards and index
    rewritten."""
    from safetensors.torch import load_file, save_file
    index_path = hf / "model.safetensors.index.json"
    index = json.loads(index_path.read_text())
    renamed = {}
    for shard in sorted(set(index["weight_map"].values())):
        moved = {rename(k): v for k, v in load_file(hf / shard).items() if rename(k) is not None}
        save_file(moved, hf / shard, metadata={"format": "pt"})
        renamed.update({k: shard for k in moved})
    index["weight_map"] = renamed
    index_path.write_text(json.dumps(index, indent=2))


def wrap_text_model(hf: Path, arch: str, model_type: str) -> None:
    """A text model's directory as its multimodal wrapper publishes it: the configuration under `text_config` beside empty
    tower configurations, every tensor under `model.language_model.` (the head where the wrapper keeps it)."""
    path = hf / "config.json"
    text = json.loads(path.read_text())
    wrapper = {"architectures": [arch], "model_type": model_type, "text_config": text, "vision_config": {}, "audio_config": {},
               "tie_word_embeddings": text.get("tie_word_embeddings", True), "transformers_version": text.get("transformers_version")}
    path.write_text(json.dumps(wrapper, indent=2))
    rename_tensors(hf, lambda k: "model.language_model." + k[len("model."):] if k.startswith("model.") else k)


def publish_kimi_linear(hf: Path) -> None:
    """Kimi-Linear's checkpoint as Moonshot publishes it, the keys its remote code and the converter read: the attention
    kinds in `linear_attn_config` (1-indexed), the leading dense blocks as `first_k_dense_replace`, the router's
    activation, its group limit and its renormalization by their own names; the leading dense blocks' FFNs under `mlp.`,
    where transformers 5 saves every block's under the mixture's `block_sparse_moe.`."""
    path = hf / "config.json"
    c = json.loads(path.read_text())
    types, mlp = c.pop("layer_types"), c.pop("mlp_layer_types")
    c["linear_attn_config"] = {
        "full_attn_layers": [i + 1 for i, t in enumerate(types) if t == "full_attention"],
        "kda_layers": [i + 1 for i, t in enumerate(types) if t == "linear_attention"],
        "head_dim": c.pop("linear_head_dim"),
        "num_heads": c.pop("linear_num_heads"),
        "short_conv_kernel_size": c.pop("linear_conv_kernel_dim"),
    }
    c["first_k_dense_replace"] = mlp.index("sparse") if "sparse" in mlp else len(mlp)
    c["moe_layer_freq"] = 1
    c["moe_router_activation_func"] = "sigmoid"
    c["use_grouped_topk"] = True
    c["mla_use_nope"] = True
    for derived in ("qk_head_dim", "head_dim"):
        c.pop(derived, None)
    path.write_text(json.dumps(c, indent=2))
    dense = c["first_k_dense_replace"]
    def rename(k):
        parts = k.split(".")
        if len(parts) > 3 and parts[1] == "layers" and int(parts[2]) < dense and parts[3] == "block_sparse_moe":
            return ".".join(parts[:3] + ["mlp"] + parts[4:])
        return k
    rename_tensors(hf, rename)


def publish_pattern(hf: Path, pattern: str, drop) -> None:
    """The configuration as Nemotron-H's dense checkpoints publish it: the block pattern the string
    `hybrid_override_pattern` transformers 4 saves, where transformers 5 saves the list `layers_block_type`, and without
    the keys `drop` names, which transformers 5 saves at their defaults."""
    path = hf / "config.json"
    config = json.loads(path.read_text())
    config.pop("layers_block_type", None)
    for key in drop:
        config.pop(key, None)
    config["hybrid_override_pattern"] = pattern
    config["num_hidden_layers"] = len(pattern)
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


def draw(shape, generator: torch.Generator) -> torch.Tensor:
    """A parameter of `shape`: a norm's weight or another vector around 1, a matrix's entries small."""
    if len(shape) == 1:
        return 1.0 + 0.1 * torch.randn(shape, generator=generator)
    return 0.05 * torch.randn(shape, generator=generator)


def write_rwkv7(config: dict, hf: Path, generator: torch.Generator, dtype: torch.dtype) -> None:
    """RWKV-7 as fla-hub publishes it (`RWKV7ForCausalLM`, flash-linear-attention's remote code, which transformers does
    not carry): its configuration, and every tensor of the model by fla's names and shapes in one `model.safetensors`
    — block 0 holding the stack's first norm (`pre_norm`) and no value-residual pair, every other block its pair —
    each drawn as every twin's parameters are."""
    from safetensors.torch import save_file
    d, ff, vocab, hd = config["hidden_size"], config["intermediate_size"], config["vocab_size"], config["head_dim"]
    ranks = {"w": config["decay_low_rank_dim"], "a": config["a_low_rank_dim"], "v": config["v_low_rank_dim"], "g": config["gate_low_rank_dim"]}
    shapes = {"model.embeddings.weight": (vocab, d), "lm_head.weight": (vocab, d), "model.norm.weight": (d,), "model.norm.bias": (d,)}
    for l in range(config["num_hidden_layers"]):
        p = f"model.layers.{l}."
        for norm in ["attn_norm", "ffn_norm", "attn.g_norm"] + (["pre_norm"] if l == 0 else []):
            shapes[f"{p}{norm}.weight"] = (d,)
            shapes[f"{p}{norm}.bias"] = (d,)
        for x in "rwkvag":
            shapes[f"{p}attn.x_{x}"] = (1, 1, d)
        shapes[f"{p}attn.k_k"] = (d,)
        shapes[f"{p}attn.k_a"] = (d,)
        shapes[f"{p}attn.r_k"] = (d // hd, hd)
        for proj in "rkvo":
            shapes[f"{p}attn.{proj}_proj.weight"] = (d, d)
        for lora, rank in ranks.items():
            if lora == "v" and l == 0:
                continue
            shapes[f"{p}attn.{lora}_lora.lora.0.weight"] = (rank, d)
            shapes[f"{p}attn.{lora}_lora.lora.2.weight"] = (d, rank)
            if lora != "g":
                shapes[f"{p}attn.{lora}_lora.lora.2.bias"] = (d,)
        shapes[f"{p}ffn.x_k"] = (d,)
        shapes[f"{p}ffn.key.weight"] = (ff, d)
        shapes[f"{p}ffn.value.weight"] = (d, ff)
    hf.mkdir(parents=True)
    save_file({name: draw(shape, generator).to(dtype) for name, shape in sorted(shapes.items())}, hf / "model.safetensors", metadata={"format": "pt"})
    (hf / "config.json").write_text(json.dumps(config, indent=2))


def write_rwkv7_hf(config: dict, hf: Path, generator: torch.Generator, dtype: torch.dtype) -> None:
    """RWKV-7 in the Hugging Face port BlinkDL's checkpoints are published in (`Rwkv7ForCausalLM`, SmerkyG's and
    mollysama's): its configuration, its low-rank pairs stored input-major and unnamed (`w1` `[d, rank]`, `w0` a
    `[1, 1, d]` row), its stack under `model.blocks.`, the first norm `model.pre_ln`, the head `head` — every tensor
    drawn as every twin's parameters are, in one `model.safetensors`."""
    from safetensors.torch import save_file
    d, layers, vocab, hs = config["hidden_size"], config["num_hidden_layers"], config["vocab_size"], config["head_size"]
    ff = config["intermediate_size"] or 4 * d
    rank = lambda key, exponent, multiplier: config[key] or max(1, round(d ** exponent * multiplier / 32)) * 32
    ranks = {"w": rank("lora_rank_decay", 0.5, 1.8), "a": rank("lora_rank_iclr", 0.5, 1.8),
             "v": rank("lora_rank_value_residual_mix", 0.5, 1.3), "g": rank("lora_rank_gate", 0.8, 0.6)}
    shapes = {"model.embeddings.weight": (vocab, d), "head.weight": (vocab, d), "model.ln_out.weight": (d,), "model.ln_out.bias": (d,),
              "model.pre_ln.weight": (d,), "model.pre_ln.bias": (d,)}
    for l in range(layers):
        p = f"model.blocks.{l}."
        for norm in ("ln1", "ln2", "attention.ln_x"):
            shapes[f"{p}{norm}.weight"] = (d,)
            shapes[f"{p}{norm}.bias"] = (d,)
        for x in ("x_r", "x_w", "x_k", "x_v", "x_a", "x_g", "k_k", "k_a"):
            shapes[f"{p}attention.{x}"] = (1, 1, d)
        shapes[f"{p}attention.r_k"] = (d // hs, hs)
        for proj in ("receptance", "key", "value", "output"):
            shapes[f"{p}attention.{proj}.weight"] = (d, d)
        for lora, r in ranks.items():
            if lora == "v" and l == 0:
                continue
            shapes[f"{p}attention.{lora}1"] = (d, r)
            shapes[f"{p}attention.{lora}2"] = (r, d)
            if lora != "g":
                shapes[f"{p}attention.{lora}0"] = (1, 1, d)
        shapes[f"{p}feed_forward.x_k"] = (1, 1, d)
        shapes[f"{p}feed_forward.key.weight"] = (ff, d)
        shapes[f"{p}feed_forward.value.weight"] = (d, ff)
    hf.mkdir(parents=True)
    save_file({name: draw(shape, generator).to(dtype) for name, shape in sorted(shapes.items())}, hf / "model.safetensors", metadata={"format": "pt"})
    (hf / "config.json").write_text(json.dumps(config, indent=2))


# The families no transformers class builds, each written by its own function from its configuration.
WRITTEN = {"rwkv7": write_rwkv7, "rwkv7-hf": write_rwkv7_hf}


def build(family: str, out: Path) -> None:
    spec = FAMILIES[family]
    torch.manual_seed(0)
    generator = torch.Generator().manual_seed(1)
    dtype = getattr(torch, spec.get("dtype", "bfloat16"))
    hf = out / "hf"
    if hf.exists():
        shutil.rmtree(hf)
    model = None
    if "written" in spec:
        WRITTEN[spec["written"]](spec["config"], hf, generator, dtype)
    else:
        config = transformers.AutoConfig.for_model(spec["model_type"], architectures=[spec["arch"]], **spec["config"])
        model_class = getattr(transformers, spec.get("model_class", spec["arch"]))
        model = model_class(config)
        with torch.no_grad():
            for _, p in sorted(model.named_parameters()):
                p.copy_(draw(p.shape, generator))
            for name, b in sorted(model.named_buffers()):
                if any(name.endswith(r) for r in spec.get("random_buffers", [])):
                    b.copy_(0.1 * torch.randn(b.shape, generator=generator))
                if any(name.endswith(r) for r in spec.get("drawn_buffers", [])):
                    b.copy_(draw(b.shape, generator))
            if "zero_embedding_rows" in spec:
                dict(model.named_parameters())["model.embed_tokens.weight"][spec["zero_embedding_rows"]] = 0.0
        model = model.to(dtype)
        model.save_pretrained(hf, max_shard_size="2MB", safe_serialization=True)
    if "model_class" in spec:
        published_architecture(hf, spec["arch"])
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
    if "dropped_buffers" in spec:
        drop_buffers(hf, spec["dropped_buffers"])
    if spec.get("published_config") == "kimi_linear":
        publish_kimi_linear(hf)
    if spec.get("published_config") == "minimax_text01":
        publish_minimax_text01(hf)
    if "wrapped" in spec:
        wrap_text_model(hf, spec["wrapped"]["arch"], spec["wrapped"]["model_type"])
    if "published_pattern" in spec:
        publish_pattern(hf, spec["published_pattern"], spec.get("published_drop", []))
    if "mixed" in spec:
        mix_dtypes(hf, spec["mixed"])
    if "fp8_blocks" in spec:
        fp8_blocks(hf, spec["fp8_blocks"])
    for f in (TOKENIZERS / spec["tokenizer"]).iterdir():
        if f.name != "SOURCE" and f.name in spec.get("tokenizer_files", [f.name]):
            shutil.copy(f, hf / f.name)
    if "tokenizer_config" in spec:
        path = hf / "tokenizer_config.json"
        path.write_text(json.dumps({**json.loads(path.read_text()), **spec["tokenizer_config"]}, indent=2))
    if "stray" in spec:
        shutil.copy(TOKENIZERS / spec["stray"][0] / spec["stray"][1], hf / spec["stray"][1])
    gguf = out / "model-f32.gguf"
    env = dict(os.environ, **({"LOGOS_TWIN_TOKENIZER_FLAGS": "1"} if spec.get("tokenizer_flags") else {}))
    if spec.get("config_as_saved"):
        env["LOGOS_TWIN_CONFIG_AS_SAVED"] = "1"
    if spec.get("tokenizer_as_saved"):
        env["LOGOS_TWIN_TOKENIZER_AS_SAVED"] = "1"
    converted = hf
    if spec.get("converted_as") == "minimax_text01":
        (out / "converted").mkdir()
        converted = out / "converted" / "hf"
        as_minimax_text01(hf, converted)
    subprocess.run(
        [sys.executable, "-c", CONVERT, str(LLAMA_CPP / "convert_hf_to_gguf.py"), str(converted), "--outtype", "f32",
         "--outfile", str(gguf)],
        check=True,
        stdout=subprocess.DEVNULL,
        env=env,
    )
    if converted != hf:
        shutil.rmtree(converted.parent)
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
