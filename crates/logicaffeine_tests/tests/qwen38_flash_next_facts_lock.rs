//! qwen38_flash_next_facts_lock — section 7 of `benchmarks/PREPARE_FOR_SAMBA.md` is a reading of Qwen3.8-Flash-Next itself.
//!
//! The checkpoint's tensor table is read from its safetensors headers (fetched by range request into
//! `external/papers/qwen38-flash-next-headers/`, revision de4b8e4d), and every parameter budget section 7.3 states is
//! recomputed from it and held to the precision the book prints. The hashed n-gram embedding's row ids
//! (`model::ngram_hash`) are held to transformers' own module (`scripts/accel/qwen4exp-oracle.py`) on sampled token
//! histories: end-of-sequence resets, the largest token, a cached context.
//!
//! The checkpoint's three integer tensors (the hash's multipliers, the heads' primes and offsets, read from their byte
//! ranges) hold the same rows to the published model itself. Every tensor of the checkpoint belongs to a component
//! section 7.2 names.
//!
//! The headers, the constants and the configuration live under the gitignored `external/papers/`
//! (`scripts/accel/qwen4exp-headers.py`); the oracle needs a Python with transformers (`LOGOS_PYTHON`). Without them
//! each test records an honest skip (`qwen4exp-headers`, `hf-twins`).

#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::model::ngram_hash::NgramHash;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn headers_dir() -> PathBuf {
    repo().join("external/papers/qwen38-flash-next-headers")
}

fn config() -> serde_json::Value {
    let path = repo().join("external/papers/hf-qwen3.8-flash-next-config.json");
    let c: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    c.get("text_config").cloned().unwrap_or(c)
}

/// Every tensor of the checkpoint: name to shape.
fn tensor_table() -> BTreeMap<String, Vec<u64>> {
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(headers_dir()).unwrap() {
        let path = e.unwrap().path();
        if !path.to_string_lossy().ends_with(".header.json") {
            continue;
        }
        let h: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for (name, v) in h {
            if name == "__metadata__" {
                continue;
            }
            let shape = v["shape"].as_array().unwrap().iter().map(|d| d.as_u64().unwrap()).collect();
            out.insert(name, shape);
        }
    }
    out
}

fn size(shape: &[u64]) -> u64 {
    shape.iter().product()
}

/// The parameters of every tensor whose name starts with `prefix` and passes `keep`.
fn sum(table: &BTreeMap<String, Vec<u64>>, prefix: &str, keep: impl Fn(&str) -> bool) -> u64 {
    table.iter().filter(|(n, _)| n.starts_with(prefix) && keep(&n[prefix.len()..])).map(|(_, s)| size(s)).sum()
}

/// Section 7.3's rows: item to the figure the book prints.
fn budget_rows() -> BTreeMap<String, String> {
    let book = std::fs::read_to_string(repo().join("benchmarks/PREPARE_FOR_SAMBA.md")).unwrap();
    let at = book.find("### 7.3 Budgets").expect("section 7.3");
    let rest = &book[at..];
    let end = rest[1..].find("\n### ").map(|i| i + 1).unwrap_or(rest.len());
    rest[..end]
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("| Item") && !l.starts_with("|---"))
        .map(|l| {
            let cells: Vec<&str> = l.split('|').map(str::trim).collect();
            (cells[1].to_string(), cells[2].to_string())
        })
        .collect()
}

/// The printed figure's value in parameters (`57.96M`, `2,521.5M`, `about 125.7B`) and its decimal places.
fn printed(figure: &str) -> (f64, u32, f64) {
    let f = figure.trim_start_matches("about ").replace(',', "");
    let token = f.split_whitespace().next().unwrap();
    let (num, scale) = match token.chars().last().unwrap() {
        'M' => (&token[..token.len() - 1], 1e6),
        'B' => (&token[..token.len() - 1], 1e9),
        _ => panic!("a figure in parameters: {figure}"),
    };
    let decimals = num.split('.').nth(1).map_or(0, str::len) as u32;
    (num.parse().unwrap(), decimals, scale)
}

/// `value` parameters, rounded as the book prints `figure`, equal the figure.
fn agrees(figure: &str, value: u64) -> bool {
    let (shown, decimals, scale) = printed(figure);
    let p = 10f64.powi(decimals as i32);
    ((value as f64 / scale) * p).round() == (shown * p).round()
}

/// Section 7.2's components, each with the tensor families it owns (layer and shard numbers read as `N`).
const COMPONENTS: &[(&str, &[&str])] = &[
    (
        "Hyper-connections",
        &[
            "model.language_model.hyper_connection_mixer.hc_norm.weight",
            "model.language_model.hyper_connection_mixer.input_mix_weight_down.weight",
            "model.language_model.hyper_connection_mixer.input_mix_weight_up.weight",
            "model.language_model.layers.N.attn_hyper_connection.block_inject_weight.weight",
            "model.language_model.layers.N.attn_hyper_connection.hc_norm.weight",
            "model.language_model.layers.N.attn_hyper_connection.input_mix_weight_down.weight",
            "model.language_model.layers.N.attn_hyper_connection.input_mix_weight_up.weight",
            "model.language_model.layers.N.mlp_hyper_connection.block_inject_weight.weight",
            "model.language_model.layers.N.mlp_hyper_connection.hc_norm.weight",
            "model.language_model.layers.N.mlp_hyper_connection.input_mix_weight_down.weight",
            "model.language_model.layers.N.mlp_hyper_connection.input_mix_weight_up.weight",
        ],
    ),
    (
        "The hashed n-gram embedding",
        &[
            "model.language_model.layers.N.ple.conv1d.weight",
            "model.language_model.layers.N.ple.key_proj.weight",
            "model.language_model.layers.N.ple.value_proj.weight",
            "model.language_model.layers.N.ple.norm_conv.weight",
            "model.language_model.layers.N.ple.norm_key.weight",
            "model.language_model.layers.N.ple.norm_query.weight",
            "model.language_model.layers.N.ple.ple_embedding.layer_multipliers",
            "model.language_model.layers.N.ple.ple_embedding.ngram_heads_offsets",
            "model.language_model.layers.N.ple.ple_embedding.ngram_heads_vocab_sizes",
            "model.language_model.layers.N.ple.ple_embedding.ngram_embedding.shard_N.weight",
        ],
    ),
    (
        "Qwen sparse attention",
        &[
            "model.language_model.layers.N.self_attn.indexer.index_qk_proj.weight",
            "model.language_model.layers.N.self_attn.indexer.k_layernorm.weight",
            "model.language_model.layers.N.self_attn.indexer.q_layernorm.weight",
            "model.language_model.layers.N.self_attn.q_proj.weight",
            "model.language_model.layers.N.self_attn.k_proj.weight",
            "model.language_model.layers.N.self_attn.v_proj.weight",
            "model.language_model.layers.N.self_attn.o_proj.weight",
            "model.language_model.layers.N.self_attn.q_norm.weight",
            "model.language_model.layers.N.self_attn.k_norm.weight",
        ],
    ),
    (
        "Gated delta",
        &[
            "model.language_model.layers.N.linear_attn.A_log",
            "model.language_model.layers.N.linear_attn.dt_bias",
            "model.language_model.layers.N.linear_attn.conv1d.weight",
            "model.language_model.layers.N.linear_attn.in_proj_a.weight",
            "model.language_model.layers.N.linear_attn.in_proj_b.weight",
            "model.language_model.layers.N.linear_attn.in_proj_qkv.weight",
            "model.language_model.layers.N.linear_attn.in_proj_z.weight",
            "model.language_model.layers.N.linear_attn.norm.weight",
            "model.language_model.layers.N.linear_attn.out_proj.weight",
        ],
    ),
    (
        "Experts",
        &[
            "model.language_model.layers.N.mlp.gate.weight",
            "model.language_model.layers.N.mlp.experts.gate_up_proj",
            "model.language_model.layers.N.mlp.experts.down_proj",
            "model.language_model.layers.N.mlp.shared_expert.gate_proj.weight",
            "model.language_model.layers.N.mlp.shared_expert.up_proj.weight",
            "model.language_model.layers.N.mlp.shared_expert.down_proj.weight",
            "model.language_model.layers.N.mlp.shared_expert_gate.weight",
        ],
    ),
    ("The embedding and the head", &["model.language_model.embed_tokens.weight", "lm_head.weight"]),
    (
        "The draft head",
        &[
            "mtp.fc_embedding.weight",
            "mtp.fc_hidden.weight",
            "mtp.pre_fc_norm_embedding.weight",
            "mtp.pre_fc_norm_hidden.weight",
            "mtp.hyper_connection_mixer.hc_norm.weight",
            "mtp.hyper_connection_mixer.input_mix_weight_down.weight",
            "mtp.hyper_connection_mixer.input_mix_weight_up.weight",
            "mtp.layers.N.attn_hyper_connection.block_inject_weight.weight",
            "mtp.layers.N.attn_hyper_connection.hc_norm.weight",
            "mtp.layers.N.attn_hyper_connection.input_mix_weight_down.weight",
            "mtp.layers.N.attn_hyper_connection.input_mix_weight_up.weight",
            "mtp.layers.N.mlp_hyper_connection.block_inject_weight.weight",
            "mtp.layers.N.mlp_hyper_connection.hc_norm.weight",
            "mtp.layers.N.mlp_hyper_connection.input_mix_weight_down.weight",
            "mtp.layers.N.mlp_hyper_connection.input_mix_weight_up.weight",
            "mtp.layers.N.self_attn.indexer.index_qk_proj.weight",
            "mtp.layers.N.self_attn.indexer.k_layernorm.weight",
            "mtp.layers.N.self_attn.indexer.q_layernorm.weight",
            "mtp.layers.N.self_attn.q_proj.weight",
            "mtp.layers.N.self_attn.k_proj.weight",
            "mtp.layers.N.self_attn.v_proj.weight",
            "mtp.layers.N.self_attn.o_proj.weight",
            "mtp.layers.N.self_attn.q_norm.weight",
            "mtp.layers.N.self_attn.k_norm.weight",
            "mtp.layers.N.mlp.gate.weight",
            "mtp.layers.N.mlp.experts.gate_up_proj",
            "mtp.layers.N.mlp.experts.down_proj",
            "mtp.layers.N.mlp.shared_expert.gate_proj.weight",
            "mtp.layers.N.mlp.shared_expert.up_proj.weight",
            "mtp.layers.N.mlp.shared_expert.down_proj.weight",
            "mtp.layers.N.mlp.shared_expert_gate.weight",
        ],
    ),
    (
        "The vision tower",
        &[
            "model.visual.patch_embed.proj.weight",
            "model.visual.patch_embed.proj.bias",
            "model.visual.pos_embed.weight",
            "model.visual.blocks.N.norm1.weight",
            "model.visual.blocks.N.norm1.bias",
            "model.visual.blocks.N.norm2.weight",
            "model.visual.blocks.N.norm2.bias",
            "model.visual.blocks.N.attn.qkv.weight",
            "model.visual.blocks.N.attn.qkv.bias",
            "model.visual.blocks.N.attn.proj.weight",
            "model.visual.blocks.N.attn.proj.bias",
            "model.visual.blocks.N.mlp.linear_fc1.weight",
            "model.visual.blocks.N.mlp.linear_fc1.bias",
            "model.visual.blocks.N.mlp.linear_fc2.weight",
            "model.visual.blocks.N.mlp.linear_fc2.bias",
            "model.visual.merger.norm.weight",
            "model.visual.merger.norm.bias",
            "model.visual.merger.linear_fc1.weight",
            "model.visual.merger.linear_fc1.bias",
            "model.visual.merger.linear_fc2.weight",
            "model.visual.merger.linear_fc2.bias",
        ],
    ),
];

/// A tensor's family: every dotted part that is a number, and every `shard_<number>`, read as `N`.
fn family(name: &str) -> String {
    name.split('.')
        .map(|part| {
            if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) {
                "N"
            } else if part.strip_prefix("shard_").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
                "shard_N"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Section 7.2's text.
fn section_7_2() -> String {
    let book = std::fs::read_to_string(repo().join("benchmarks/PREPARE_FOR_SAMBA.md")).unwrap();
    let at = book.find("### 7.2 ").expect("section 7.2");
    let rest = &book[at..];
    let end = rest[1..].find("\n### ").map(|i| i + 1).unwrap_or(rest.len());
    rest[..end].to_string()
}

#[test]
fn every_tensor_belongs_to_a_component_section_7_names() {
    if !common::require_tool("qwen4exp-headers") {
        return;
    }
    let text = section_7_2();
    for (component, _) in COMPONENTS {
        assert!(text.contains(&format!("**{component}**")), "section 7.2 names no component `{component}`");
    }
    let mut seen = std::collections::BTreeSet::new();
    for name in tensor_table().keys() {
        let f = family(name);
        assert!(
            COMPONENTS.iter().any(|(_, families)| families.contains(&f.as_str())),
            "`{name}` (family `{f}`) belongs to no component section 7.2 names"
        );
        seen.insert(f);
    }
    for (component, families) in COMPONENTS {
        for f in *families {
            assert!(seen.contains(*f), "`{component}` claims `{f}`, which the checkpoint does not have");
        }
    }
}

#[test]
fn the_tensor_family_reads_layer_and_shard_numbers_as_n() {
    assert_eq!(family("model.language_model.layers.17.mlp.gate.weight"), "model.language_model.layers.N.mlp.gate.weight");
    assert_eq!(
        family("model.language_model.layers.1.ple.ple_embedding.ngram_embedding.shard_127.weight"),
        "model.language_model.layers.N.ple.ple_embedding.ngram_embedding.shard_N.weight"
    );
    assert_eq!(family("lm_head.weight"), "lm_head.weight");
    assert_eq!(family("model.visual.blocks.0.norm1.bias"), "model.visual.blocks.N.norm1.bias");
    assert_eq!(family("a.shard_.b"), "a.shard_.b");
    assert_eq!(family("a.shard_x1.b"), "a.shard_x1.b");
}

/// The checkpoint's own integer tensors (`constants.json`, read from their byte ranges by
/// `scripts/accel/qwen4exp-headers.py`).
fn checkpoint_constants() -> BTreeMap<String, Vec<u64>> {
    let path = headers_dir().join("constants.json");
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

#[test]
fn the_hash_constants_equal_the_checkpoints_own() {
    if !common::require_tool("qwen4exp-headers") {
        return;
    }
    let c = config();
    let ple_layers: Vec<u64> = c["ple_layer_ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    let stored = checkpoint_constants();
    assert_eq!(stored.len(), 3 * ple_layers.len(), "three integer tensors for each layer that carries the embedding");
    for (place, id) in ple_layers.iter().enumerate() {
        // `ple_layer_ids` counts decoder layers from one; the checkpoint's layer index counts from zero.
        let prefix = format!("model.language_model.layers.{}.ple.ple_embedding.", id - 1);
        let hash = NgramHash::from_config(&c, place as u64).unwrap();
        let get = |k: &str| stored.get(&format!("{prefix}{k}")).unwrap_or_else(|| panic!("the checkpoint has no `{prefix}{k}`"));
        assert_eq!(hash.multipliers(), get("layer_multipliers").as_slice(), "the multipliers, at the default seed");
        assert_eq!(hash.head_primes(), get("ngram_heads_vocab_sizes").as_slice(), "each head's prime");
        assert_eq!(hash.head_offsets(), get("ngram_heads_offsets").as_slice(), "each head's offset into the table");
    }
    let shards = tensor_table()
        .iter()
        .filter(|(n, _)| n.contains(".ngram_embedding.shard_"))
        .map(|(_, s)| s[0])
        .sum::<u64>();
    let hash = NgramHash::from_config(&c, 0).unwrap();
    assert_eq!(shards, hash.rows(), "the table's shards hold exactly the padded rows");
}

#[test]
fn every_budget_in_section_7_equals_the_tensor_table() {
    if !common::require_tool("qwen4exp-headers") {
        return;
    }
    let t = tensor_table();
    assert_eq!(t.len(), 1658, "the checkpoint's tensor count");
    let c = config();
    let layer = |l: usize| format!("model.language_model.layers.{l}.");
    let types: Vec<String> = c["layer_types"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let gdn = types.iter().position(|s| s == "linear_attention").unwrap();
    let attn = types.iter().position(|s| s == "full_attention").unwrap();
    let (n_gdn, n_attn) = (types.iter().filter(|s| *s == "linear_attention").count() as u64, types.iter().filter(|s| *s == "full_attention").count() as u64);
    let gated_delta = sum(&t, &layer(gdn), |n| n.starts_with("linear_attn."));
    let attention = sum(&t, &layer(attn), |n| n.starts_with("self_attn."));
    let experts_stored = sum(&t, &layer(gdn), |n| n.starts_with("mlp.experts.") || n.starts_with("mlp.shared_expert."));
    let one_expert = size(&t[&format!("{}mlp.experts.gate_up_proj", layer(gdn))]) / 512 + size(&t[&format!("{}mlp.experts.down_proj", layer(gdn))]) / 512;
    let top_k = c["num_experts_per_tok"].as_u64().unwrap();
    let shared = sum(&t, &layer(gdn), |n| n.starts_with("mlp.shared_expert."));
    let experts_active = top_k * one_expert + shared;
    let hyper = sum(&t, &layer(gdn), |n| n.contains("hyper_connection."));
    let head = size(&t["lm_head.weight"]);
    let embed = size(&t["model.language_model.embed_tokens.weight"]);
    let main_stack = sum(&t, "model.language_model.layers.", |n| !n.contains(".ple.")) + head + embed;
    let routers = sum(&t, &layer(gdn), |n| n == "mlp.gate.weight" || n == "mlp.shared_expert_gate.weight");
    let ple_projections = sum(&t, "model.language_model.layers.", |n| n.contains(".ple.") && !n.contains("ngram_embedding"));
    // The model-level mixer that folds the residual lanes before the head runs for every token too.
    let final_mixer = sum(&t, "model.language_model.hyper_connection_mixer.", |_| true);
    let layers = n_gdn + n_attn;
    let active = n_gdn * gated_delta
        + n_attn * attention
        + layers * (experts_active + routers + hyper)
        + head
        + ple_projections
        + final_mixer;

    let rows = budget_rows();
    let want = [
        ("A gated-delta layer", gated_delta),
        ("An attention layer", attention),
        ("Experts stored a layer", experts_stored),
        ("Experts active a token a layer", experts_active),
        ("Hyper-connections a layer", hyper),
        ("Head; embedding", head),
        ("Stored, main stack", main_stack),
        ("Active a token", active),
    ];
    for (item, value) in want {
        let figure = rows.get(item).unwrap_or_else(|| panic!("section 7.3 has no row `{item}`"));
        assert!(agrees(figure, value), "`{item}`: the book prints {figure}; the tensor table gives {value}");
    }
    assert_eq!(head, embed, "the head and the embedding are the same size, untied");
}

/// Sampled histories: tokens across the vocabulary, the end-of-sequence token where it resets the n-grams, the
/// largest token, and contexts carried from a previous call.
fn cases(eos: u32, vocab: u32) -> Vec<(Vec<u32>, Vec<u32>)> {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut out = vec![
        (vec![], vec![5, eos, 17, 9, 9]),
        (vec![100, eos], vec![3, 4]),
        (vec![vocab - 1, 0], vec![vocab - 1]),
        (vec![], vec![eos]),
        (vec![], vec![eos, eos, eos]),
        (vec![], vec![0, 0, 0, 0]),
        (vec![], vec![vocab - 1; 6]),
    ];
    for _ in 0..200 {
        let len = 1 + (next() % 12) as usize;
        let tokens: Vec<u32> = (0..len).map(|_| if next() % 7 == 0 { eos } else { (next() % vocab as u64) as u32 }).collect();
        let context = if next() % 2 == 0 { vec![] } else { vec![(next() % vocab as u64) as u32, (next() % vocab as u64) as u32] };
        out.push((context, tokens));
    }
    out
}

#[test]
fn the_hash_matches_the_reference_on_sampled_token_triples() {
    if !common::require_tool("hf-twins") || !common::require_tool("qwen4exp-headers") {
        return;
    }
    let c = config();
    let eos = c["eos_token_id"].as_u64().unwrap() as u32;
    let vocab = c["vocab_size"].as_u64().unwrap() as u32;
    let hash = NgramHash::from_config(&c, 0).unwrap();
    let cases = cases(eos, vocab);
    let input: Vec<serde_json::Value> = cases.iter().map(|(ctx, tok)| serde_json::json!({"context": ctx, "tokens": tok})).collect();
    let scratch = repo().join("target/hf-twins/scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join(format!("ngram-cases-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::Value::Array(input).to_string()).unwrap();
    let out = std::process::Command::new(common::python_for_tools())
        .arg(repo().join("scripts/accel/qwen4exp-oracle.py"))
        .arg("ngram")
        .arg(repo().join("external/papers/hf-qwen3.8-flash-next-config.json"))
        .arg(&path)
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(out.status.success(), "the oracle: {}", String::from_utf8_lossy(&out.stderr));
    let theirs: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let as_u64 = |v: &serde_json::Value| -> Vec<u64> { v.as_array().unwrap().iter().map(|x| x.as_u64().unwrap()).collect() };
    assert_eq!(hash.multipliers(), as_u64(&theirs["multipliers"]).as_slice(), "the layer's multipliers");
    assert_eq!(hash.head_primes(), as_u64(&theirs["primes"]).as_slice(), "each head's prime");
    assert_eq!(hash.rows_before_padding(), theirs["rows_before_padding"].as_u64().unwrap(), "the table's rows");
    assert_eq!(hash.rows() % 128, 0, "the table is padded to a multiple of 128 rows");
    for (k, (context, tokens)) in cases.iter().enumerate() {
        let ours = hash.ids(context, tokens);
        let want: Vec<Vec<u64>> = theirs["rows"][k].as_array().unwrap().iter().map(as_u64).collect();
        assert_eq!(ours, want, "case {k}: context {context:?}, tokens {tokens:?}");
    }
}
