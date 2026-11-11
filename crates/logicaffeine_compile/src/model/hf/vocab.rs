//! The tokenizer keys llama.cpp's converter writes from a checkpoint directory's tokenizer files, computed as the
//! converter computes them: a byte-pair vocabulary from `tokenizer.json` (`get_vocab_base`), a SentencePiece one from
//! `tokenizer.model` (`_create_vocab_sentencepiece`), and the special tokens, added-token flags, merges, normalizer
//! flags and chat template `gguf.SpecialVocab` reads beside them.
//!
//! The converter names a byte-pair vocabulary's pre-tokenizer by the digest of the ids its Hugging Face tokenizer gives
//! a fixed probe text ([`PRE_TOKENIZER_PROBE`]), looked up in a table of the digests of known models
//! ([`PRE_TOKENIZER_DIGESTS`]). This reader encodes the probe with the directory's own `tokenizer.json`, run as the
//! `tokenizers` library runs it ([`HfTokenizer`]), and looks the digest up in the same table.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::Path;

use serde_json::{Map, Value};

use super::encode::HfTokenizer;
use super::tiktoken::{byte_level, Tiktoken};
use super::{invalid, Family, Hparams};
use crate::crypto_accel::sha256;
use crate::model::gguf::MetaValue;

/// llama.cpp's `models/templates/MiniMax-M1.jinja` at the pinned commit, the chat template the converter's MiniMax-01
/// class writes where a directory ships no `chat_template.jinja`.
const MINIMAX_M1_TEMPLATE: &str = include_str!("minimax_m1.jinja");

/// llama.cpp's `models/templates/llama-cpp-rwkv-world.jinja` at the pinned commit, the chat template the converter
/// writes for an RWKV world vocabulary whose tokenizer files name none.
const RWKV_WORLD_TEMPLATE: &str = include_str!("rwkv_world.jinja");

/// The text whose tokenization names a byte-pair pre-tokenizer (the converter's `chktxt`).
pub const PRE_TOKENIZER_PROBE: &str = "\n \n\n \n\n\n \t \t\t \t\n  \n   \n    \n     \n🚀 (normal) 😶\u{200d}🌫️ (multiple emojis concatenated) ✅ 🦙🦙 3 33 333 3333 33333 333333 3333333 33333333 3.3 3..3 3...3 កាន់តែពិសេសអាច😁 ?我想在apple工作1314151天～ ------======= нещо на Български ''''''```````\"\"\"\"......!!!!!!?????? I've been 'told he's there, 'RE you sure? 'M not sure I'll make it, 'D you like some tea? We'Ve a'lL";

/// The converter's `get_vocab_base_pre` table: the SHA-256 of the probe's ids, written as a Python list, and the
/// pre-tokenizer name it writes for them, in the converter's order (a digest listed twice names what its last entry
/// says).
pub const PRE_TOKENIZER_DIGESTS: &[(&str, &str)] = &[
    ("b6e8e1518dc4305be2fe39c313ed643381c4da5db34a98f6a04c093f8afbe99b", "chatglm-bpe"),
    ("81d72c7348a9f0ebe86f23298d37debe0a5e71149e29bd283904c02262b27516", "chatglm-bpe"),
    ("a1336059768a55c99a734006ffb02203cd450fed003e9a71886c88acf24fdbc2", "glm4"),
    ("9ca2dd618e8afaf09731a7cf6e2105b373ba6a1821559f258b272fe83e6eb902", "glm4"),
    ("cdf5f35325780597efd76153d4d1c16778f766173908894c04afc20108536267", "glm4"),
    ("1431a23e583c97432bc230bff598d103ddb5a1f89960c8f1d1051aaa944d0b35", "minerva-7b"),
    ("7e57df22b1fe23a7b1e1c7f3dc4e3f96d43a4eb0836d0c6bdc3436d7b2f1c664", "hunyuan"),
    ("bba3b3366b646dbdded5dbc42d59598b849371afc42f7beafa914afaa5b70aa6", "hunyuan-dense"),
    ("e6ddf9c6686791c12d698d34c31ab9be1fea9af5a3d9a6909783ab382198ae1c", "hy_v4"),
    ("a6b57017d60e6edb4d88ecc2845188e0eb333a70357e45dcc9b53964a73bbae6", "falcon-h1"),
    ("60476e1243776c4fb1b993dbd7a5f15ac22f83c80afdf425fa5ae01c8d44ef86", "falcon-h1"),
    ("3eda48b4c4dc7de733d1a8b3e3b4a85243dbbf704da2ee9d42c6beced8897896", "falcon-h1"),
    ("48f8e02c0359c0bbdd82f26909171fac1c18a457bb47573ed1fe3bbb2c1cfd4b", "falcon-h1"),
    ("81212dc7cdb7e0c1074ca62c5aeab0d43c9f52b8a737be7b12a777c953027890", "kimi-k2"),
    ("d4540891389ea895b53b399da6ac824becc30f2fba0e9ddbb98f92e55ca0e97c", "qwen2"),
    ("1444df51289cfa8063b96f0e62b1125440111bc79a52003ea14b6eac7016fd5f", "qwen35"),
    ("66b8d4e19ab16c3bfd89bce5d785fb7e0155e8648708a1f42077cb9fe002c273", "grok-2"),
    ("b3d1dd861f1d4c5c0d2569ce36baf3f90fe8a102db3de50dd71ff860d91be3df", "jina-v2-de"),
    ("0fe1cf6eda062318a1af7270f3331a85c539a01778ff948e24388e949c5282f4", "gpt-2"),
    ("9e454714343b69b99b71795c1d27a68c2a1d15dab111f4d353109f966af29da7", "lfm2"),
    ("846deafc5b0fa786186fa4ae6c7b49903cf2f1d1895bdb80b9120d60be135252", "gemma4"),
    ("0a766d034107bc736a3f2dc4968fd62e54a3570f1454443e0c5a4cc6bd7941ed", "spark2_5"),
    ("1f9825a388f700a6b591722f17d470cbbcf10973ece35d2fd14239a14110ae1a", "k2-horizon"),
    ("0ef9807a4087ebef797fc749390439009c3b9eda9ad1a097abbe738f486c01e5", "llama-bpe"),
    ("049ecf7629871e3041641907f3de7c733e4dbfdc736f57d882ba0b0845599754", "deepseek-llm"),
    ("347715f544604f9118bb75ed199f68779f423cabb20db6de6f31b908d04d7821", "deepseek-coder"),
    ("8aeee3860c56296a157a1fe2fad249ec40aa59b1bb5709f4ade11c4e6fe652ed", "falcon"),
    ("0876d13b50744004aa9aeae05e7b0647eac9d801b5ba4668afc01e709c15e19f", "bert-bge"),
    ("9d032fcbd5501f4a38150912590928bfb36091efb5df11b8e2124b0390e3fb1e", "falcon3"),
    ("8e62295832751ca1e8f92f2226f403dea30dc5165e448b5bfa05af5340c64ec7", "bert-bge-large"),
    ("b6dc8df998e1cfbdc4eac8243701a65afe638679230920b50d6f17d81c098166", "mpt"),
    ("35d91631860c815f952d711435f48d356ebac988362536bed955d43bfa436e34", "starcoder"),
    ("3ce83efda5659b07b1ad37ca97ca5797ea4285d9b9ab0dc679e4a720c9da7454", "gpt-2"),
    ("32d85c31273f8019248f2559fed492d929ea28b17e51d81d3bb36fff23ca72b3", "stablelm2"),
    ("6221ad2852e85ce96f791f476e0b390cf9b474c9e3d1362f53a24a06dc8220ff", "refact"),
    ("9c2227e4dd922002fb81bde4fc02b0483ca4f12911410dee2255e4987644e3f8", "command-r"),
    ("d772b220ace2baec124bed8cfafce0ead7d6c38a4b65ef11261cf9d5d62246d1", "tiny_aya"),
    ("52df12b4c8d4176e7481aab4b6e8454d1fd0a210a04a574f6d4e067d10e23c3e", "cohere2moe"),
    ("e636dc30a262dcc0d8c323492e32ae2b70728f4df7dfe9737d9f920a282b8aea", "qwen2"),
    ("b6dc8df998e1cfbdc4eac8243701a65afe638679230920b50d6f17d81c098166", "olmo"),
    ("a8594e3edff7c29c003940395316294b2c623e09894deebbc65f33f1515df79e", "dbrx"),
    ("c7699093ba4255a91e702aa38a596aa81669f3525dae06c2953267dde580f448", "jina-v1-en"),
    ("0876d13b50744004aa9aeae05e7b0647eac9d801b5ba4668afc01e709c15e19f", "jina-v2-en"),
    ("171aeeedd6fb548d418a7461d053f11b6f1f1fc9b387bd66640d28a4b9f5c643", "jina-v2-es"),
    ("27949a2493fc4a9f53f5b9b029c82689cfbe5d3a1929bb25e043089e28466de6", "jina-v2-de"),
    ("a023e9fdc5a11f034d3ef515b92350e56fb2af1f66c6b6811a4444ea9bf8763d", "jina-v5-nano"),
    ("c136ed14d01c2745d4f60a9596ae66800e2b61fa45643e72436041855ad4089d", "smaug-bpe"),
    ("c7ea5862a53e4272c035c8238367063e2b270d51faa48c0f09e9d5b54746c360", "poro-chat"),
    ("7967bfa498ade6b757b064f31e964dddbb80f8f9a4d68d4ba7998fcf281c531a", "jina-v2-code"),
    ("7fc505bd3104ca1083b150b17d088b59534ede9bde81f0dd2090967d7fe52cee", "viking"),
    ("b53802fb28e26d645c3a310b34bfe07da813026ec7c7716883404d5e0f8b1901", "jais"),
    ("bc5108ee1eb6a3d600cadd065f63190fbd0554dbc9e4bbd6a0d977970afc8d2a", "jais-2"),
    ("7b3e7548e4308f52a76e8229e4e6cc831195d0d1df43aed21ac6c93da05fec5f", "codeshell"),
    ("63b97e4253352e6f357cc59ea5b583e3a680eaeaf2632188c2b952de2588485e", "tekken"),
    ("855059429035d75a914d1eda9f10a876752e281a054a7a3d421ef0533e5b6249", "smollm"),
    ("3c30d3ad1d6b64202cd222813e7736c2db6e1bd6d67197090fc1211fbc612ae7", "bloom"),
    ("bc01ce58980e1db43859146dc51b1758b3b88729b217a74792e9f8d43e479d21", "gpt3-finnish"),
    ("4e2b24cc4770243d65a2c9ec19770a72f08cffc161adbb73fcbb6b7dd45a0aae", "exaone"),
    ("fcace8b9cac38ce847670c970cd5892031a753a1ef381abd1d9af00f713da085", "phi-2"),
    ("60824e3c0d9401f89943cbb2fff727f0e2d4c545ba4df2d6e4f09a6db0f5b450", "chameleon"),
    ("8b5a93ed704057481f240da0be7e7dca721d7f8f4755263b6807227a2cbeae65", "roberta-bpe"),
    ("ad851be1dba641f2e3711822f816db2c265f788b37c63b4e1aeacb9ee92de8eb", "gigachat"),
    ("d4c8f286ea6b520b3d495c4455483cfa2302c0cfcd4be05d781b6a8a0a7cdaf1", "megrez"),
    ("877081d19cf6996e2c4ff0e1236341e9b7bde288f5311a56a937f0afbbb3aeb5", "deepseek-v3"),
    ("b3f499bb4255f8ca19fccd664443283318f2fd2414d5e0b040fbdd0cc195d6c5", "deepseek-r1-qwen"),
    ("ccc2ef013c104be7bae2965776d611e1d7a8a2a9c547dd93a682c9a9fc80352e", "gpt-4o"),
    ("7dec86086fcc38b66b7bc1575a160ae21cf705be7718b9d5598190d7c12db76f", "superbpe"),
    ("1994ffd01900cfb37395608534236ecd63f2bd5995d6cb1004dda1af50240f15", "trillion"),
    ("96a5f08be6259352137b512d4157e333e21df7edd3fcd152990608735a65b224", "bailingmoe"),
    ("d353350c764d8c3b39c763113960e4fb4919bea5fbf208a0e3b22e8469dc7406", "llama4"),
    ("0e9433cbbb161f89e264eb32e8e64bfe69e834973ffca5d41d3948a604a3e2a3", "pixtral"),
    ("d5f1dd6f980fec569fb218a81a7658ac45fc56b38c5a0adeb1c232fbe04ef5ec", "seed-coder"),
    ("b0a6b1c0bd5998ebd9df08611efde34a4ff03faed45ae09c43e6b31ebd4b94cf", "a.x-4.0"),
    ("f6791d196f87ce6b56a7d234be618e0d58f8cda3549416635b2bebcd22cd95c4", "midm-2.0"),
    ("169bf0296a13c4d9b7672313f749eb36501d931022de052aad6e36f2bf34dd51", "lfm2"),
    ("2085e1638f6c377a0aa4ead21b27bb4cb941bf800df86ed391011769c1758dfb", "exaone4"),
    ("a1e163ecab2e718a4c829d1148b6e86824ec36163bb71941c3dca9cd5ac25756", "mellum"),
    ("a0b64b4385f123663873756336c085744376d015ff328bb1d901598f63c44152", "modern-bert"),
    ("49fc0303c9e0d2c2c565c510f64b2d9b271276acdcdadff733249eda9f7d59df", "afmoe"),
    ("9b1be57e70d20d9501b2b3186e792d81181ae36ada3903c26f9fea418cf87206", "bailingmoe2"),
    ("53e325976a6e142379c19b09afcae354f2f496f147afa8f9e189a33fe4e3024e", "granite-docling"),
    ("f4f37b6c8eb9ea29b3eac6bb8c8487c5ab7885f8d8022e67edc1c68ce8403e95", "minimax-m2"),
    ("4a2e2abae11ca2b86d570fc5b44be4d5eb5e72cc8f22dd136a94b37da83ab665", "kormo"),
    ("9d70134b369a70e5735009b6de918f7581b5211f7c074d1f89f753aea8248af1", "youtu"),
    ("16389f0a1f51ee53e562ffd51c371dc508639ab0e4261502071836e50e223e91", "solar-open"),
    ("6c81ce329e0802883b22eabab0d3fa48357337ef1ecb45443828bf1f6254833f", "exaone-moe"),
    ("d30d75d9059f1aa2c19359de71047b3ae408c70875e8a3ccf8c5fba56c9d8af4", "qwen35"),
    ("b4b8ca1f9769494fbd956ebc4c249de6131fb277a4a3345a7a92c7dd7a55808d", "joyai-llm"),
    ("e4d54df1ebc1f2b91acd986c5b51aa50837d5faf7c7398e73c1f9e9ee5d19869", "kanana2"),
    ("862f827721df956049dff5ca81a57f29e575280bc622e290d3bf4e35eca29015", "f2llmv2"),
    ("62f6fb0a6fd5098caeabb19b07a5c1099cafc8b9c40eab6ea89ece4ec02fbc57", "sarvam-moe"),
    ("f728162c1315c26e40249849799b4ba3fe584c32084b4795b03eb295e63cb5af", "talkie"),
    ("36f3066e97b7f3994b379aaacde306c1444c6ae84e81a5ae3cd2b7ed3b8c42d4", "minicpm5"),
    ("f241072145675bf8322086f115aebad05e9f869557a238bf2150a2a417d1bf60", "granite-embed-multi-97m"),
    ("789696f5946cc0fc59371f39f6097cafed196b3acded6140432f26bbb1ae1669", "granite-embed-multi-311m"),
    ("9dcf830ee9990cdbf78cc523a5f7bd9ad8f3f9890c2d3581d2785ad10f07049d", "mellum2"),
    ("972da7b59cec44d1f0a490a86c96df53859e486e481563e5dddac155013d87ac", "laguna"),
    ("653660222fb704f61cbf2b618a8ae6502b7f8b20c980f9a5de07ed78e13319cd", "ufakzeka"),
    ("4b05e02dad1c5ae07d266fd3342ddb644c6f6be058d728bc0a33af31a1d6ee66", "mmbert"),
    ("a9af07a84191f55098b248ae6f3dfe9e32d3190bebe8eafd91c1ddec9bc3449f", "k2-horizon"),
];

/// GGUF value types of the arrays the converter writes.
const STRING: u32 = 8;
const INT32: u32 = 5;
const FLOAT32: u32 = 6;

/// `gguf.TokenType`.
const NORMAL: i32 = 1;
const UNKNOWN: i32 = 2;
const CONTROL: i32 = 3;
const USER_DEFINED: i32 = 4;
const UNUSED: i32 = 5;
const BYTE: i32 = 6;

/// The special-token kinds `SpecialVocab` reads by default.
const SPECIAL_TYPES: [&str; 7] = ["bos", "eos", "unk", "sep", "pad", "cls", "mask"];

/// The keys as the converter's writer leaves them: a later write of a key replaces the earlier.
#[derive(Default)]
struct Keys(BTreeMap<String, MetaValue>);

impl Keys {
    fn put(&mut self, key: &str, v: MetaValue) {
        self.0.insert(key.to_string(), v);
    }
    fn str(&mut self, key: &str, v: &str) {
        self.put(key, MetaValue::Str(v.as_bytes().to_vec()));
    }
    fn u32(&mut self, key: &str, v: u32) {
        self.put(key, MetaValue::U32(v));
    }
    fn bool(&mut self, key: &str, v: bool) {
        self.put(key, MetaValue::Bool(v));
    }
    fn strings(&mut self, key: &str, items: &[Vec<u8>]) {
        self.put(key, MetaValue::Array { ty: STRING, items: items.iter().map(|s| MetaValue::Str(s.clone())).collect() });
    }
}

fn read_json(path: &Path) -> io::Result<Option<Value>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text).map(Some).map_err(|e| invalid(format!("{}: {e}", path.display())))
}

/// The size of `dir`'s tokenizer's own vocabulary, its added tokens aside: the `tokenizers` library's
/// `get_vocab_size(with_added_tokens=False)`, which transformers 4 reports as a fast tokenizer's `vocab_size`.
pub(super) fn base_vocab_size(dir: &Path) -> io::Result<usize> {
    let path = dir.join("tokenizer.json");
    let tj = read_json(&path)?.ok_or_else(|| invalid(format!("{}: no tokenizer.json", dir.display())))?;
    let vocab = &tj["model"]["vocab"];
    vocab
        .as_object()
        .map(Map::len)
        .or_else(|| vocab.as_array().map(Vec::len))
        .ok_or_else(|| invalid(format!("{}: a model with no vocabulary", path.display())))
}

/// Python's truth value of a JSON value: absent, `null`, `false`, zero and empty strings, arrays and objects are false.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Whether a JSON value is the string `s` (Python's `==` between a string and whatever the file holds).
fn is(v: &Option<Value>, s: &str) -> bool {
    matches!(v, Some(Value::String(x)) if x == s)
}

/// `ModelBase.does_token_look_special`: added tokens some checkpoints leave unmarked that are control tokens.
fn looks_special(t: &str) -> bool {
    matches!(t, "<pad>" | "<mask>" | "<2mass>" | "[@BOS@]")
        || (t.starts_with("<|") && t.ends_with("|>"))
        || (t.starts_with("<｜") && t.ends_with("｜>"))
        || (t.starts_with("<unused") && t.ends_with('>'))
}

/// The tokenizer keys of the checkpoint in `dir`, with `general.architecture`, as the converter writes them for
/// family `f`.
pub(super) fn tokenizer_meta(dir: &Path, h: &Hparams, f: Family) -> io::Result<BTreeMap<String, MetaValue>> {
    let mut w = Keys::default();
    w.str("general.architecture", f.arch());
    let sentencepiece_file = dir.join("tokenizer.model").is_file();
    match f {
        // `LlamaModel.set_vocab`, which Mistral 3's text class inherits.
        Family::Llama | Family::Mistral3 => llama(dir, h, &mut w)?,
        // `Qwen2Model.set_vocab`, which Qwen3's class inherits; the mixtures' and Qwen3.5's classes read tokenizer.json.
        Family::Qwen2 | Family::Qwen3 if sentencepiece_file => sentencepiece(dir, h, &mut w)?,
        // `Gemma3NModel.set_vocab` hides `vocab_size_per_layer_input` from the lookup, so its vocabulary runs to the whole
        // `vocab_size`, the multimodal tokens' places padded.
        Family::Gemma3 | Family::Gemma3n if sentencepiece_file => {
            let mut map = h.map.clone();
            if f == Family::Gemma3n {
                map.remove("vocab_size_per_layer_input");
            }
            sentencepiece(dir, &Hparams { map, architecture: h.architecture.clone() }, &mut w)?;
            w.bool("tokenizer.ggml.add_space_prefix", false);
        }
        // `Glm4Model.set_vocab` and `_set_vocab_glm` (GLM-4.5's class): the byte-pair vocabulary with special tokens
        // named by their text, each kept only where the files name none of its kind.
        Family::Phi3 | Family::PhiMoe => phi3(dir, h, &mut w)?,
        // `Mamba2Model.set_vocab`: the vocabulary padded to a multiple of `pad_vocab_size_multiple` (16), read from a
        // SentencePiece model where the directory ships one, else from tokenizer.json; the converter's built-in
        // GPT-NeoX vocabulary, for a directory with neither, is not read.
        Family::Mamba2 => {
            let pad = h.u64(&["pad_vocab_size_multiple"]).filter(|&p| p > 0).unwrap_or(16);
            let vocab = h.u64(&["vocab_size"]).ok_or_else(|| invalid("config.json: no vocab_size"))?;
            let mut map = h.map.clone();
            map.insert("vocab_size".into(), Value::from(vocab.div_ceil(pad) * pad));
            let padded = Hparams { map, architecture: h.architecture.clone() };
            if dir.join("tokenizer.model").is_file() {
                sentencepiece(dir, &padded, &mut w)?
            } else if dir.join("tokenizer.model.v3").is_file() {
                return Err(invalid(format!("{}: tokenizer.model.v3, which the converter asks be renamed tokenizer.model", dir.display())));
            } else if dir.join("tokenizer.json").is_file() {
                gpt2(dir, &padded, &mut w, &[])?
            } else {
                return Err(invalid(format!("{}: no tokenizer files; the converter's built-in GPT-NeoX vocabulary is not read", dir.display())));
            }
        }
        // `JambaModel.set_vocab`: a SentencePiece model where the directory ships one, else the fast tokenizer's
        // vocabulary as `LlamaHfVocab` reads it; no space prefix either way.
        Family::Jamba => {
            if sentencepiece_file {
                sentencepiece(dir, h, &mut w)?;
            } else {
                llama_hf(dir, &mut w)?;
            }
            w.bool("tokenizer.ggml.add_space_prefix", false);
        }
        // `NemotronHModel.set_vocab`: the byte-pair vocabulary through the tokenizer, padded to a multiple of 8 with
        // `[PAD<n>]` entries.
        Family::NemotronH => {
            let vocab = h.u64(&["vocab_size"]).ok_or_else(|| invalid("config.json: no vocab_size"))?;
            let mut map = h.map.clone();
            map.insert("vocab_size".into(), Value::from(vocab.div_ceil(8) * 8));
            gpt2(dir, &Hparams { map, architecture: h.architecture.clone() }, &mut w, &[])?
        }
        Family::Glm4 => gpt2(dir, h, &mut w, &[("eos", "<|endoftext|>"), ("eot", "<|user|>"), ("unk", "<|endoftext|>"), ("bos", "<|endoftext|>")])?,
        Family::Glm4Moe | Family::GlmDsa => {
            gpt2(dir, h, &mut w, &[("bos", "[gMASK]"), ("eot", "<|user|>"), ("unk", "<|endoftext|>"), ("eom", "<|observation|>")])?
        }
        // `DeepseekV32Model.set_vocab`: the byte-pair vocabulary of a tokenizer that adds its beginning token, as the
        // converter asserts. transformers 4, the converter's own requirement, reads the flag from tokenizer_config.json
        // and rebuilds the post-processor around it (`bos $A`), so the probe the pre-tokenizer is named by begins with
        // that token; transformers 5 drops the flag beside a tokenizer.json, and the class converts nothing.
        Family::Deepseek32 => {
            let tc = read_json(&dir.join("tokenizer_config.json"))?;
            if tc.as_ref().and_then(|c| c.get("add_bos_token")).and_then(Value::as_bool) != Some(true) {
                return Err(invalid("tokenizer_config.json: add_bos_token is not true, which the converter's DeepSeek-V3.2 class asserts"));
            }
            if tc.as_ref().and_then(|c| c.get("add_eos_token")).and_then(Value::as_bool) == Some(true) {
                return Err(invalid("tokenizer_config.json: add_eos_token is true, a probe the converter's table names no pre-tokenizer for"));
            }
            gpt2_probed(dir, h, &mut w, &[], true)?
        }
        // `DeepseekV2Model.set_vocab`: a directory with no `tokenizer.json` is read through its tokenizer code, and the
        // converter writes only Kimi K2's tiktoken vocabulary.
        Family::Deepseek2 if !dir.join("tokenizer.json").is_file() && dir.join("tiktoken.model").is_file() => kimi(dir, h, &mut w)?,
        // `KimiLinearModel.set_vocab`: a byte-pair tokenizer.json where the directory ships one, else Kimi's tiktoken
        // vocabulary as DeepSeek's class reads Kimi K2's.
        Family::KimiLinear if !dir.join("tokenizer.json").is_file() => {
            kimi(dir, h, &mut w)?;
            kimi_linear_eos(dir, &mut w)?;
        }
        // `MiniMaxText01Model.set_vocab`: the byte-pair vocabulary, its chat template replaced by the directory's
        // `chat_template.jinja`, else by the pinned llama.cpp's `models/templates/MiniMax-M1.jinja`.
        Family::Rwkv7 => rwkv_world(dir, h, &mut w)?,
        Family::Gemma4 => gemma4(dir, &mut w)?,
        Family::Minimax01 => {
            gpt2(dir, h, &mut w, &[])?;
            let jinja = dir.join("chat_template.jinja");
            let template = if jinja.is_file() { std::fs::read_to_string(&jinja)? } else { MINIMAX_M1_TEMPLATE.to_string() };
            w.str("tokenizer.chat_template", &template);
        }
        _ => gpt2(dir, h, &mut w, &[])?,
    }
    Ok(w.0)
}

/// `LlamaModel.set_vocab`: a SentencePiece model when the checkpoint ships one, else its byte-pair `tokenizer.json`;
/// Code Llama's infill tokens and the small Granite models' unadded beginning token on top.
fn llama(dir: &Path, h: &Hparams, w: &mut Keys) -> io::Result<()> {
    if let Some(tc) = read_json(&dir.join("tokenizer_config.json"))? {
        match tc.get("add_prefix_space") {
            None | Some(Value::Null) => {}
            Some(Value::Bool(b)) => w.bool("tokenizer.ggml.add_space_prefix", *b),
            Some(other) => return Err(invalid(format!("tokenizer_config.json: add_prefix_space is {other}"))),
        }
        if tc.get("tokenizer_class").and_then(Value::as_str) == Some("HybridDNATokenizer") {
            return Err(invalid("tokenizer_config.json: a HybridDNA tokenizer, which this reader does not read"));
        }
    }
    if dir.join("tekken.json").is_file() && !dir.join("tokenizer.json").is_file() {
        return Err(invalid(format!("{}: a Mistral tekken tokenizer, which this reader does not read", dir.display())));
    }
    if dir.join("tokenizer.model").is_file() {
        sentencepiece(dir, h, w)?;
    } else {
        // `LlamaHfVocab` takes a byte-fallback `tokenizer.json` with a sequence decoder; Llama 3's and every other
        // byte-pair vocabulary go to `_set_vocab_gpt2`.
        if let Some(tj) = read_json(&dir.join("tokenizer.json"))? {
            let m = &tj["model"];
            let bpe = m["type"].as_str() == Some("BPE");
            let llama3 = bpe && truthy(m.get("ignore_merges")) && !truthy(Some(m.get("byte_fallback").unwrap_or(&Value::Bool(true))));
            let hf_spm = !llama3 && bpe && truthy(m.get("byte_fallback")) && tj["decoder"]["type"].as_str() == Some("Sequence");
            if hf_spm {
                return Err(invalid(format!(
                    "{}/tokenizer.json: a byte-fallback vocabulary the converter reads through LlamaHfVocab, which this reader does not read",
                    dir.display()
                )));
            }
        }
        gpt2(dir, h, w, &[])?;
    }
    let vocab_size = h.u64(&["vocab_size"]).unwrap_or(32000);
    if vocab_size == 32016 {
        let mut sv = SpecialVocab::load(dir, false, None, &["prefix", "suffix", "middle", "eot"])?;
        for (typ, id) in [("prefix", 32007), ("suffix", 32008), ("middle", 32009), ("eot", 32010)] {
            sv.set_id(typ, &Value::from(id))?;
        }
        sv.write(w);
    }
    if vocab_size == 49152 {
        w.bool("tokenizer.ggml.add_bos_token", false);
    }
    Ok(())
}

/// `Phi3MiniModel.set_vocab`, which PhiMoE's class inherits: the byte-pair vocabulary where `tokenizer_config.json` names
/// the `GPT2Tokenizer` (Phi-4), else the pieces of `tokenizer.model` each typed by its own kind (a user-defined piece a
/// normal one), the added tokens of `added_tokens.json` within the vocabulary, and then every added token
/// `tokenizer_config.json` and `tokenizer.json` list, each user-defined, or control where it is special, with its text as
/// written; the special tokens read without merges.
fn phi3(dir: &Path, h: &Hparams, w: &mut Keys) -> io::Result<()> {
    let tc = read_json(&dir.join("tokenizer_config.json"))?;
    if let Some(tc) = &tc {
        let class = tc.get("tokenizer_class").ok_or_else(|| invalid("tokenizer_config.json: no tokenizer_class, which the converter's Phi-3 class reads"))?;
        if class.as_str() == Some("GPT2Tokenizer") {
            return gpt2(dir, h, w, &[]);
        }
    }
    let path = dir.join("tokenizer.model");
    if !path.is_file() {
        return Err(invalid(format!("{}: no tokenizer.model, which the converter's Phi-3 class reads", dir.display())));
    }
    let pieces = sentencepiece_pieces(&std::fs::read(&path)?)?;
    let vocab_size = h.u64(&["vocab_size"]).map_or(pieces.len(), |n| n as usize);
    if pieces.len() > vocab_size {
        return Err(invalid(format!("{}: {} pieces past a vocabulary of {vocab_size}", path.display(), pieces.len())));
    }
    let mut tokens: Vec<Vec<u8>> = (0..vocab_size).map(|i| format!("[PAD{i}]").into_bytes()).collect();
    let mut scores = vec![-10000.0f32; vocab_size];
    let mut types = vec![UNUSED; vocab_size];
    for (id, p) in pieces.iter().enumerate() {
        tokens[id] = p.text.clone();
        scores[id] = p.score;
        types[id] = match p.kind {
            2 => UNKNOWN,
            3 => CONTROL,
            5 => UNUSED,
            6 => BYTE,
            _ => NORMAL,
        };
    }
    if let Some(Value::Object(added)) = read_json(&dir.join("added_tokens.json"))? {
        for (text, id) in added {
            let id = id.as_u64().ok_or_else(|| invalid(format!("added_tokens.json: {text} has no id")))? as usize;
            if id < vocab_size {
                tokens[id] = text.into_bytes();
                scores[id] = -1000.0;
                types[id] = USER_DEFINED;
            }
        }
    }
    let mut listed: Vec<(usize, String, bool)> = Vec::new();
    if let Some(Value::Object(decoder)) = tc.as_ref().and_then(|c| c.get("added_tokens_decoder")) {
        for (id, data) in decoder {
            let id: usize = id.parse().map_err(|_| invalid(format!("tokenizer_config.json: added token id `{id}`")))?;
            let content = data["content"].as_str().ok_or_else(|| invalid(format!("tokenizer_config.json: added token {id} has no content")))?;
            listed.push((id, content.to_string(), truthy(data.get("special"))));
        }
    }
    if let Some(tj) = read_json(&dir.join("tokenizer.json"))? {
        for a in tj["added_tokens"].as_array().into_iter().flatten() {
            let id = a["id"].as_u64().ok_or_else(|| invalid("tokenizer.json: an added token without an id"))? as usize;
            listed.push((id, a["content"].as_str().unwrap_or_default().to_string(), truthy(a.get("special"))));
        }
    }
    for (id, content, special) in listed {
        if id >= vocab_size {
            return Err(invalid(format!("added token {id} `{content}` past a vocabulary of {vocab_size}")));
        }
        tokens[id] = content.into_bytes();
        scores[id] = -1000.0;
        types[id] = if special { CONTROL } else { USER_DEFINED };
    }
    w.str("tokenizer.ggml.model", "llama");
    w.str("tokenizer.ggml.pre", "default");
    w.strings("tokenizer.ggml.tokens", &tokens);
    w.put("tokenizer.ggml.scores", MetaValue::Array { ty: FLOAT32, items: scores.into_iter().map(MetaValue::F32).collect() });
    w.put("tokenizer.ggml.token_type", MetaValue::Array { ty: INT32, items: types.into_iter().map(MetaValue::I32).collect() });
    SpecialVocab::load(dir, false, Some(tokens.len()), &SPECIAL_TYPES)?.write(w);
    Ok(())
}

/// One `SentencePiece` of a `ModelProto`: its piece, score and type.
struct Piece {
    text: Vec<u8>,
    score: f32,
    kind: u64,
}

/// The pieces of a serialized SentencePiece `ModelProto` (field 1; each a piece, a score and a type, `NORMAL` when
/// unstated).
fn sentencepiece_pieces(bytes: &[u8]) -> io::Result<Vec<Piece>> {
    fn varint(b: &[u8], at: &mut usize) -> io::Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *b.get(*at).ok_or_else(|| invalid("tokenizer.model ends inside a varint"))?;
            *at += 1;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(invalid("tokenizer.model holds an overlong varint"))
    }
    /// The next field of a message: its number, wire type and, for a length-delimited field, its bytes.
    fn field<'a>(b: &'a [u8], at: &mut usize) -> io::Result<(u64, u64, &'a [u8], u64)> {
        let key = varint(b, at)?;
        let (num, wire) = (key >> 3, key & 7);
        let (bytes, value) = match wire {
            0 => (&b[0..0], varint(b, at)?),
            1 | 5 => {
                let n = if wire == 1 { 8 } else { 4 };
                let s = b.get(*at..*at + n).ok_or_else(|| invalid("tokenizer.model ends inside a fixed field"))?;
                *at += n;
                (s, 0)
            }
            2 => {
                let n = varint(b, at)? as usize;
                let s = b.get(*at..*at + n).ok_or_else(|| invalid("tokenizer.model ends inside a field"))?;
                *at += n;
                (s, 0)
            }
            w => return Err(invalid(format!("tokenizer.model: wire type {w}"))),
        };
        Ok((num, wire, bytes, value))
    }
    let mut pieces = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let (num, wire, body, _) = field(bytes, &mut at)?;
        if num != 1 || wire != 2 {
            continue;
        }
        let mut p = Piece { text: Vec::new(), score: 0.0, kind: 1 };
        let mut i = 0;
        while i < body.len() {
            let (n, w, b, v) = field(body, &mut i)?;
            match (n, w) {
                (1, 2) => p.text = b.to_vec(),
                (2, 5) => p.score = f32::from_le_bytes(b.try_into().unwrap()),
                (3, 0) => p.kind = v,
                _ => {}
            }
        }
        pieces.push(p);
    }
    Ok(pieces)
}

/// `_set_vocab_sentencepiece`: the pieces of `tokenizer.model` (an id past the model's own vocabulary a `[PAD<id>]`),
/// the added tokens of `added_tokens.json` and `tokenizer_config.json` over them, and the special tokens.
fn sentencepiece(dir: &Path, h: &Hparams, w: &mut Keys) -> io::Result<()> {
    let path = dir.join("tokenizer.model");
    let pieces = sentencepiece_pieces(&std::fs::read(&path)?)?;
    let vocab_size = match h.u64(&["vocab_size_per_layer_input", "vocab_size"]) {
        Some(n) if n > 0 => n as usize,
        _ => pieces.len(),
    };
    let mut tokens: Vec<Vec<u8>> = (0..vocab_size).map(|i| format!("[PAD{i}]").into_bytes()).collect();
    let mut scores = vec![-10000.0f32; vocab_size];
    let mut types = vec![UNUSED; vocab_size];
    for (id, p) in pieces.iter().enumerate().take(vocab_size) {
        tokens[id] = p.text.clone();
        scores[id] = p.score;
        types[id] = match p.kind {
            2 => UNKNOWN,
            3 => CONTROL,
            5 => UNUSED,
            6 => BYTE,
            _ => NORMAL,
        };
    }
    if let Some(Value::Object(added)) = read_json(&dir.join("added_tokens.json"))? {
        for (text, id) in added {
            let id = id.as_u64().ok_or_else(|| invalid(format!("added_tokens.json: {text} has no id")))? as usize;
            if id < vocab_size {
                tokens[id] = text.into_bytes();
                scores[id] = -1000.0;
                types[id] = USER_DEFINED;
            }
        }
    }
    if let Some(tc) = read_json(&dir.join("tokenizer_config.json"))? {
        if let Some(Value::Object(decoder)) = tc.get("added_tokens_decoder") {
            for (id, data) in decoder {
                let id: usize = id.parse().map_err(|_| invalid(format!("tokenizer_config.json: added token id `{id}`")))?;
                if id >= vocab_size {
                    continue;
                }
                let content = data["content"].as_str().ok_or_else(|| invalid(format!("tokenizer_config.json: added token {id} has no content")))?;
                let text = if truthy(data.get("special")) || looks_special(content) {
                    types[id] = CONTROL;
                    content.to_string()
                } else {
                    types[id] = USER_DEFINED;
                    content.replace('\u{2581}', " ")
                };
                scores[id] = -1000.0;
                tokens[id] = text.into_bytes();
            }
        }
    }
    w.str("tokenizer.ggml.model", "llama");
    w.str("tokenizer.ggml.pre", "default");
    w.strings("tokenizer.ggml.tokens", &tokens);
    w.put("tokenizer.ggml.scores", MetaValue::Array { ty: FLOAT32, items: scores.into_iter().map(MetaValue::F32).collect() });
    w.put("tokenizer.ggml.token_type", MetaValue::Array { ty: INT32, items: types.into_iter().map(MetaValue::I32).collect() });
    SpecialVocab::load(dir, false, Some(tokens.len()), &SPECIAL_TYPES)?.write(w);
    Ok(())
}

/// An added token as the fast tokenizer holds it.
struct Added {
    content: String,
    special: bool,
    normalized: bool,
}

/// `_set_vocab_gpt2`: the byte-pair vocabulary of `tokenizer.json` (`get_vocab_base`), its pre-tokenizer named by
/// the probe digest, and the special tokens and merges. `named` are the converter's `_set_special_token` calls after the
/// files are read: each kind's id the added token of that text, where the files set no id of the kind.
fn gpt2(dir: &Path, h: &Hparams, w: &mut Keys, named: &[(&str, &str)]) -> io::Result<()> {
    gpt2_probed(dir, h, w, named, false)
}

/// [`gpt2`], the probe that names the pre-tokenizer begun with the beginning token where `bos_first` (a tokenizer whose
/// post-processor transformers rebuilt around `bos $A`).
fn gpt2_probed(dir: &Path, h: &Hparams, w: &mut Keys, named: &[(&str, &str)], bos_first: bool) -> io::Result<()> {
    let tj_path = dir.join("tokenizer.json");
    let tj = read_json(&tj_path)?
        .ok_or_else(|| invalid(format!("{}: no tokenizer.json (a vocabulary only a slow tokenizer reads)", dir.display())))?;
    let tc = read_json(&dir.join("tokenizer_config.json"))?;
    let model = &tj["model"];
    if model["type"].as_str() != Some("BPE") {
        return Err(invalid(format!("{}: a `{}` model where a byte-pair vocabulary is read", tj_path.display(), model["type"])));
    }
    let mut added: BTreeMap<u64, Added> = BTreeMap::new();
    for a in tj["added_tokens"].as_array().into_iter().flatten() {
        let id = a["id"].as_u64().ok_or_else(|| invalid(format!("{}: an added token without an id", tj_path.display())))?;
        let content = a["content"].as_str().unwrap_or_default().to_string();
        added.insert(id, Added { content, special: truthy(a.get("special")), normalized: truthy(a.get("normalized")) });
    }
    if let Some(Value::Object(decoder)) = tc.as_ref().and_then(|c| c.get("added_tokens_decoder")) {
        for (id, d) in decoder {
            let Ok(id) = id.parse::<u64>() else { continue };
            added.entry(id).or_insert_with(|| Added {
                content: d["content"].as_str().unwrap_or_default().to_string(),
                special: truthy(d.get("special")),
                normalized: truthy(d.get("normalized")),
            });
        }
    }
    // `tokenizer.vocab`: the model's vocabulary with the added tokens over it, keyed by text.
    let mut vocab: HashMap<String, u64> = HashMap::new();
    for (text, id) in model["vocab"].as_object().into_iter().flatten() {
        vocab.insert(text.clone(), id.as_u64().ok_or_else(|| invalid(format!("{}: `{text}` has no id", tj_path.display())))?);
    }
    let added_vocab: HashMap<&str, u64> = added.iter().map(|(&id, a)| (a.content.as_str(), id)).collect();
    for (&text, &id) in &added_vocab {
        vocab.insert(text.to_string(), id);
    }
    let vocab_size = h.u64(&["vocab_size"]).unwrap_or(vocab.len() as u64);
    if let Some(top) = vocab.values().max().filter(|&&m| m >= vocab_size) {
        return Err(invalid(format!("{}: token id {top} past the vocabulary of {vocab_size}", tj_path.display())));
    }
    let reverse: HashMap<u64, &str> = vocab.iter().map(|(t, &id)| (id, t.as_str())).collect();
    let clean_up = tc.as_ref().and_then(|c| c.get("clean_up_tokenization_spaces")).and_then(Value::as_bool).unwrap_or(false);
    let mut tokens = Vec::with_capacity(vocab_size as usize);
    let mut types = Vec::with_capacity(vocab_size as usize);
    for i in 0..vocab_size {
        let Some(&text) = reverse.get(&i) else {
            tokens.push(format!("[PAD{i}]").into_bytes());
            types.push(UNUSED);
            continue;
        };
        let mut token = text.to_string();
        if added_vocab.contains_key(text) {
            let a = added.get(&i).ok_or_else(|| invalid(format!("{}: `{text}` is an added token under another id", tj_path.display())))?;
            if !a.normalized && clean_up {
                // The converter writes `decode(encode(token))`: the added token itself, with the decoder's clean-up.
                token = clean_up_tokenization(&token);
            }
            if a.special || looks_special(&token) {
                types.push(CONTROL);
            } else {
                token = token.replace('\u{2581}', " ");
                types.push(USER_DEFINED);
            }
        } else {
            types.push(NORMAL);
        }
        tokens.push(token.into_bytes());
    }
    let mut specials = SpecialVocab::load(dir, true, None, &SPECIAL_TYPES)?;
    for &(typ, text) in named {
        let id = added_vocab
            .get(text)
            .ok_or_else(|| invalid(format!("{}: no added token `{text}`, the converter's {typ} token", tj_path.display())))?;
        specials.set_id(typ, &Value::from(*id))?;
    }
    let first = if bos_first {
        let bos = tc.as_ref().and_then(|c| c.get("bos_token")).and_then(|b| b.as_str().or_else(|| b.get("content").and_then(Value::as_str)));
        let id = bos.and_then(|b| vocab.get(b)).ok_or_else(|| invalid(format!("{}: no beginning token in the vocabulary", tj_path.display())))?;
        Some(*id as u32)
    } else {
        None
    };
    let pre = pre_tokenizer_name(&tj, &tj_path, first)?;

    w.str("tokenizer.ggml.model", "gpt2");
    w.str("tokenizer.ggml.pre", &pre);
    w.strings("tokenizer.ggml.tokens", &tokens);
    w.put("tokenizer.ggml.token_type", MetaValue::Array { ty: INT32, items: types.into_iter().map(MetaValue::I32).collect() });
    specials.write(w);
    Ok(())
}

/// `TextModel._set_vocab_rwkv_world`: `<s>` (a control token), then each line of `rwkv_vocab_v20230424.txt` — an id, a
/// Python string or bytes literal, its length in bytes — written as the token's bytes escaped as Python's `repr` of
/// bytes escapes them (the quotes dropped), then `[PAD<i>]` entries to `vocab_size` (65,536 where unset); the
/// tokenizer files' special tokens and chat template (llama.cpp's RWKV world template where they name none), `\n\n`
/// (261) as the end of a turn, and 0 as the beginning and end token whatever the files say.
fn rwkv_world(dir: &Path, h: &Hparams, w: &mut Keys) -> io::Result<()> {
    let path = dir.join("rwkv_vocab_v20230424.txt");
    let text = std::fs::read_to_string(&path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
    let vocab_size = h.u64(&["vocab_size"]).unwrap_or(65536) as usize;
    let mut tokens: Vec<Vec<u8>> = vec![b"<s>".to_vec()];
    let mut types = vec![CONTROL];
    for line in text.split_inclusive('\n') {
        let parts: Vec<&str> = line.split(' ').collect();
        if parts.len() < 3 {
            return Err(invalid(format!("{}: `{}` is not an id, a literal and a length", path.display(), line.trim_end())));
        }
        let literal = parts[1..parts.len() - 1].join(" ");
        let bytes = python_literal(&literal).ok_or_else(|| invalid(format!("{}: `{literal}` is not a Python string or bytes literal", path.display())))?;
        let length: usize = parts[parts.len() - 1].trim().parse().map_err(|_| invalid(format!("{}: `{}` is not a length", path.display(), line.trim_end())))?;
        if bytes.len() != length {
            return Err(invalid(format!("{}: `{literal}` is {} bytes, not the {length} its line says", path.display(), bytes.len())));
        }
        tokens.push(python_bytes_repr(&bytes));
        types.push(NORMAL);
    }
    if tokens.len() > vocab_size {
        return Err(invalid(format!("{}: {} tokens past a vocabulary of {vocab_size}", path.display(), tokens.len())));
    }
    for i in tokens.len()..vocab_size {
        tokens.push(format!("[PAD{i}]").into_bytes());
        types.push(UNUSED);
    }
    let mut specials = SpecialVocab::load(dir, false, None, &SPECIAL_TYPES)?;
    if specials.chat_template.is_none() {
        specials.chat_template = Some(Template::One(RWKV_WORLD_TEMPLATE.to_string()));
    }
    specials.set_id("eot", &Value::from(261))?;
    specials.force_id("bos", 0);
    specials.force_id("eos", 0);
    w.str("tokenizer.ggml.model", "rwkv");
    w.strings("tokenizer.ggml.tokens", &tokens);
    w.put("tokenizer.ggml.token_type", MetaValue::Array { ty: INT32, items: types.into_iter().map(MetaValue::I32).collect() });
    specials.write(w);
    Ok(())
}

/// The value of a Python string or bytes literal (`ast.literal_eval` of one): a string's as UTF-8, a bytes literal's
/// bytes; `None` for any other text.
fn python_literal(s: &str) -> Option<Vec<u8>> {
    let (bytes, rest) = match s.strip_prefix('b').or_else(|| s.strip_prefix('B')) {
        Some(r) => (true, r),
        None => (false, s),
    };
    let quote = rest.chars().next().filter(|&q| q == '\'' || q == '"')?;
    let body = rest.strip_prefix(quote)?.strip_suffix(quote)?;
    let mut out = Vec::new();
    let mut chars = body.chars().peekable();
    let push = |out: &mut Vec<u8>, c: char| {
        let mut buf = [0u8; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    };
    while let Some(c) = chars.next() {
        if c == quote {
            return None;
        }
        if c != '\\' {
            if bytes && !c.is_ascii() {
                return None;
            }
            push(&mut out, c);
            continue;
        }
        let e = chars.next()?;
        let hex = |chars: &mut std::iter::Peekable<std::str::Chars>, n: usize| -> Option<u32> {
            let digits: String = (0..n).map(|_| chars.next()).collect::<Option<_>>()?;
            u32::from_str_radix(&digits, 16).ok()
        };
        match e {
            '\n' => {}
            '\\' | '\'' | '"' => push(&mut out, e),
            'a' => out.push(0x07),
            'b' => out.push(0x08),
            'f' => out.push(0x0c),
            'n' => out.push(b'\n'),
            'r' => out.push(b'\r'),
            't' => out.push(b'\t'),
            'v' => out.push(0x0b),
            'x' => {
                let v = hex(&mut chars, 2)?;
                if bytes {
                    out.push(v as u8);
                } else {
                    push(&mut out, char::from_u32(v)?);
                }
            }
            '0'..='7' => {
                let mut v = e.to_digit(8)?;
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            v = v * 8 + d;
                            chars.next();
                        }
                        None => break,
                    }
                }
                if bytes {
                    out.push((v & 0xff) as u8);
                } else {
                    push(&mut out, char::from_u32(v)?);
                }
            }
            'u' if !bytes => push(&mut out, char::from_u32(hex(&mut chars, 4)?)?),
            'U' if !bytes => push(&mut out, char::from_u32(hex(&mut chars, 8)?)?),
            other => {
                out.push(b'\\');
                push(&mut out, other);
            }
        }
    }
    Some(out)
}

/// Python's `repr` of `bytes` with its `b` and quotes dropped (`repr(token)[2:-1]`): the quote it chooses (a double one
/// where the bytes hold a single quote and no double) and the backslash escaped, tab, newline and return by name, every
/// other byte outside printable ASCII as `\xhh`.
fn python_bytes_repr(b: &[u8]) -> Vec<u8> {
    let quote = if b.contains(&b'\'') && !b.contains(&b'"') { b'"' } else { b'\'' };
    let mut out = Vec::with_capacity(b.len());
    for &c in b {
        match c {
            _ if c == quote || c == b'\\' => out.extend_from_slice(&[b'\\', c]),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            0x20..=0x7e => out.push(c),
            _ => out.extend_from_slice(format!("\\x{c:02x}").as_bytes()),
        }
    }
    out
}

/// `TextModel._set_vocab_llama_hf` (`gguf.LlamaHfVocab`): a fast tokenizer's byte-fallback byte-pair vocabulary in id
/// order, every entry scored −1000 and typed a byte (`<0xXX>`), a control token where it is one of the tokenizer's special
/// tokens (its named ones and its additional ones, as transformers lists them), else normal; then the added tokens past
/// the model's vocabulary, control where special, else user-defined; `SpecialVocab`'s ids and flags over as many entries.
fn llama_hf(dir: &Path, w: &mut Keys) -> io::Result<()> {
    let (tokens, types) = llama_hf_entries(dir)?;
    let n = tokens.len();
    w.str("tokenizer.ggml.model", "llama");
    w.str("tokenizer.ggml.pre", "default");
    w.strings("tokenizer.ggml.tokens", &tokens);
    w.put("tokenizer.ggml.scores", MetaValue::Array { ty: FLOAT32, items: vec![MetaValue::F32(-1000.0); n] });
    w.put("tokenizer.ggml.token_type", MetaValue::Array { ty: INT32, items: types.into_iter().map(MetaValue::I32).collect() });
    SpecialVocab::load(dir, false, Some(n), &SPECIAL_TYPES)?.write(w);
    Ok(())
}

/// `Gemma4Model.set_vocab`: `LlamaHfVocab`'s entries under the `gemma4` model, the channel, tool-call, tool-response and
/// quote markers a chat parser reads kept visible (user-defined), every merge, no space prefix, the beginning token
/// added.
fn gemma4(dir: &Path, w: &mut Keys) -> io::Result<()> {
    const VISIBLE: [&str; 7] = ["<|channel>", "<channel|>", "<|tool_call>", "<tool_call|>", "<|tool_response>", "<tool_response|>", "<|\"|>"];
    let (tokens, mut types) = llama_hf_entries(dir)?;
    for (t, ty) in tokens.iter().zip(&mut types) {
        if VISIBLE.iter().any(|v| v.as_bytes() == t.as_slice()) {
            *ty = USER_DEFINED;
        }
    }
    let n = tokens.len();
    w.str("tokenizer.ggml.model", "gemma4");
    w.strings("tokenizer.ggml.tokens", &tokens);
    w.put("tokenizer.ggml.scores", MetaValue::Array { ty: FLOAT32, items: vec![MetaValue::F32(-1000.0); n] });
    w.put("tokenizer.ggml.token_type", MetaValue::Array { ty: INT32, items: types.into_iter().map(MetaValue::I32).collect() });
    SpecialVocab::load(dir, true, None, &SPECIAL_TYPES)?.write(w);
    w.bool("tokenizer.ggml.add_space_prefix", false);
    w.bool("tokenizer.ggml.add_bos_token", true);
    Ok(())
}

/// `LlamaHfVocab.all_tokens`: a fast tokenizer's byte-fallback byte-pair vocabulary in id order — a byte (`<0xXX>`), a
/// control token where it is one of the tokenizer's special tokens (its named ones and its additional ones, as
/// transformers lists them), else normal — then the added tokens past the model's vocabulary, control where special,
/// else user-defined.
fn llama_hf_entries(dir: &Path) -> io::Result<(Vec<Vec<u8>>, Vec<i32>)> {
    let tj_path = dir.join("tokenizer.json");
    let tj = read_json(&tj_path)?.ok_or_else(|| invalid(format!("{}: no tokenizer.json, which LlamaHfVocab reads", dir.display())))?;
    let m = &tj["model"];
    let llama3 = m["type"].as_str() == Some("BPE") && truthy(m.get("ignore_merges")) && !truthy(Some(m.get("byte_fallback").unwrap_or(&Value::Bool(true))));
    if llama3 || m["type"].as_str() != Some("BPE") || !truthy(m.get("byte_fallback")) || tj["decoder"]["type"].as_str() != Some("Sequence") {
        return Err(invalid(format!("{}: not a byte-fallback byte-pair vocabulary with a sequence decoder, which LlamaHfVocab reads", tj_path.display())));
    }
    let mut vocab: HashMap<String, u64> = HashMap::new();
    for (text, id) in m["vocab"].as_object().into_iter().flatten() {
        vocab.insert(text.clone(), id.as_u64().ok_or_else(|| invalid(format!("{}: `{text}` has no id", tj_path.display())))?);
    }
    let base = vocab.len() as u64;
    let mut added: BTreeMap<u64, (String, bool)> = BTreeMap::new();
    for a in tj["added_tokens"].as_array().into_iter().flatten() {
        let id = a["id"].as_u64().ok_or_else(|| invalid(format!("{}: an added token without an id", tj_path.display())))?;
        let content = a["content"].as_str().unwrap_or_default().to_string();
        vocab.insert(content.clone(), id);
        added.insert(id, (content, truthy(a.get("special"))));
    }
    // `all_special_tokens`: the named special tokens and the additional ones, from tokenizer_config.json over
    // special_tokens_map.json, and transformers 5's model-specific ones — every other key of tokenizer_config.json
    // ending in `_token` that names a token (Gemma 4's `boi_token`, `think_token` and the rest).
    const NAMED: [&str; 7] = ["bos_token", "eos_token", "unk_token", "sep_token", "pad_token", "cls_token", "mask_token"];
    let text_of = |v: &Value| v.as_str().map(str::to_string).or_else(|| v.get("content").and_then(Value::as_str).map(str::to_string));
    let mut specials: Vec<String> = Vec::new();
    for file in ["special_tokens_map.json", "tokenizer_config.json"] {
        let Some(c) = read_json(&dir.join(file))? else { continue };
        for key in NAMED {
            if let Some(t) = c.get(key).and_then(text_of) {
                specials.push(t);
            }
        }
        for key in ["additional_special_tokens", "extra_special_tokens"] {
            match c.get(key) {
                Some(Value::Array(list)) => specials.extend(list.iter().filter_map(text_of)),
                Some(Value::Object(named)) => specials.extend(named.values().filter_map(text_of)),
                _ => {}
            }
        }
        if file == "tokenizer_config.json" {
            if let Some(o) = c.as_object() {
                specials.extend(o.iter().filter(|(k, _)| k.ends_with("_token") && !NAMED.contains(&k.as_str())).filter_map(|(_, v)| text_of(v)));
            }
        }
    }
    let special_ids: BTreeSet<u64> = specials.iter().filter_map(|t| vocab.get(t).copied()).collect();
    let reverse: HashMap<u64, &str> = vocab.iter().map(|(t, &id)| (id, t.as_str())).collect();
    let mut tokens = Vec::new();
    let mut types = Vec::new();
    for id in 0..base {
        let text = reverse.get(&id).ok_or_else(|| invalid(format!("{}: no token of id {id}", tj_path.display())))?;
        let byte = text.len() == 6 && text.starts_with("<0x") && text.ends_with('>') && text[3..5].bytes().all(|b| b.is_ascii_hexdigit());
        types.push(if byte {
            BYTE
        } else if special_ids.contains(&id) {
            CONTROL
        } else {
            NORMAL
        });
        tokens.push(text.as_bytes().to_vec());
    }
    for (id, (text, _)) in added.range(base..) {
        types.push(if special_ids.contains(id) { CONTROL } else { USER_DEFINED });
        tokens.push(text.as_bytes().to_vec());
    }
    Ok((tokens, types))
}

/// `DeepseekV2Model.set_vocab`'s Kimi K2 path: the tiktoken vocabulary written as a byte-pair one — each mergeable token
/// in GPT-2's byte alphabet with its merge, the reserved special tokens after them, `[PAD<id>]` for an id neither names
/// — named by the probe digest of the tokenizer's own encoding, which must be Kimi K2's; the special-token ids and
/// template as `SpecialVocab` reads them without merges.
fn kimi(dir: &Path, h: &Hparams, w: &mut Keys) -> io::Result<()> {
    let t = Tiktoken::open(dir)?;
    let ids = t.encode(PRE_TOKENIZER_PROBE);
    let listed = format!("[{}]", ids.iter().map(u32::to_string).collect::<Vec<_>>().join(", "));
    let digest: String = sha256(listed.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let pre = pre_tokenizer_named(&digest)
        .ok_or_else(|| invalid(format!("{}: a tiktoken pre-tokenizer the converter does not recognize (probe digest {digest})", dir.display())))?;
    if pre != "kimi-k2" {
        return Err(invalid(format!("{}: a tiktoken vocabulary whose pre-tokenizer is `{pre}`; the converter writes only kimi-k2's", dir.display())));
    }
    // `{**vocab, **special_tokens}`: each text keeps its first place and takes its last id; the reverse map, in that
    // order, keeps the last text of an id.
    let mut merges: Vec<Vec<u8>> = Vec::new();
    let mut combined: Vec<(String, u32)> = Vec::new();
    let mut place: HashMap<String, usize> = HashMap::new();
    let mut put = |text: String, id: u32, combined: &mut Vec<(String, u32)>| match place.get(&text) {
        Some(&i) => combined[i].1 = id,
        None => {
            place.insert(text.clone(), combined.len());
            combined.push((text, id));
        }
    };
    for (bytes, rank) in t.tokens() {
        put(byte_level(bytes), *rank, &mut combined);
        if bytes.len() > 1 {
            if let Some((a, b)) = t.merge_of(bytes, *rank) {
                merges.push(format!("{} {}", byte_level(&a), byte_level(&b)).into_bytes());
            }
        }
    }
    for (text, id) in t.specials() {
        put(text.clone(), *id, &mut combined);
    }
    let special: std::collections::HashSet<u32> = t.specials().iter().map(|&(_, id)| id).collect();
    let mut reverse: HashMap<u32, &str> = HashMap::new();
    for (text, id) in &combined {
        reverse.insert(*id, text.as_str());
    }
    let vocab_size = h.u64(&["vocab_size"]).ok_or_else(|| invalid("config.json: no vocab_size, which the Kimi vocabulary reads"))?;
    let (mut tokens, mut types) = (Vec::with_capacity(vocab_size as usize), Vec::with_capacity(vocab_size as usize));
    for i in 0..vocab_size as u32 {
        match reverse.get(&i) {
            None => {
                tokens.push(format!("[PAD{i}]").into_bytes());
                types.push(UNUSED);
            }
            Some(text) => {
                tokens.push(text.as_bytes().to_vec());
                types.push(if special.contains(&i) { CONTROL } else { NORMAL });
            }
        }
    }
    w.str("tokenizer.ggml.model", "gpt2");
    w.str("tokenizer.ggml.pre", pre);
    w.strings("tokenizer.ggml.tokens", &tokens);
    w.put("tokenizer.ggml.token_type", MetaValue::Array { ty: INT32, items: types.into_iter().map(MetaValue::I32).collect() });
    w.strings("tokenizer.ggml.merges", &merges);
    SpecialVocab::load(dir, false, None, &SPECIAL_TYPES)?.write(w);
    Ok(())
}

/// `KimiLinearModel.set_vocab`'s end token, written over `SpecialVocab`'s: the tiktoken tokenizer's own (`eos_id`), its
/// `eos_token` (`[EOS]` where tokenizer_config.json names none) among its special tokens.
fn kimi_linear_eos(dir: &Path, w: &mut Keys) -> io::Result<()> {
    let tc = read_json(&dir.join("tokenizer_config.json"))?;
    let named = tc.as_ref().and_then(|c| c.get("eos_token")).and_then(|e| e.as_str().or_else(|| e.get("content").and_then(Value::as_str)));
    let eos = named.unwrap_or("[EOS]");
    let t = Tiktoken::open(dir)?;
    let id = t
        .specials()
        .iter()
        .find(|(text, _)| text == eos)
        .map(|&(_, id)| id)
        .ok_or_else(|| invalid(format!("{}: the end token `{eos}` is none of the tiktoken vocabulary's special tokens", dir.display())))?;
    w.u32("tokenizer.ggml.eos_token_id", id);
    Ok(())
}

/// `PreTrainedTokenizerBase.clean_up_tokenization`.
fn clean_up_tokenization(s: &str) -> String {
    s.replace(" .", ".")
        .replace(" ?", "?")
        .replace(" !", "!")
        .replace(" ,", ",")
        .replace(" ' ", "'")
        .replace(" n't", "n't")
        .replace(" 'm", "'m")
        .replace(" 's", "'s")
        .replace(" 've", "'ve")
        .replace(" 're", "'re")
}

/// `get_vocab_base_pre`: the name the converter's table gives the digest of the probe's ids, the probe encoded as the
/// Hugging Face tokenizer encodes it (its special tokens added) and the ids written as a Python list.
fn pre_tokenizer_name(tj: &Value, path: &Path, first: Option<u32>) -> io::Result<String> {
    let tokenizer = HfTokenizer::from_json(tj).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
    let ids: Vec<u32> = match first {
        None => tokenizer.encode(PRE_TOKENIZER_PROBE, true),
        Some(bos) => std::iter::once(bos).chain(tokenizer.encode(PRE_TOKENIZER_PROBE, false)).collect(),
    };
    let listed = format!("[{}]", ids.iter().map(u32::to_string).collect::<Vec<_>>().join(", "));
    let digest: String = sha256(listed.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    pre_tokenizer_named(&digest)
        .map(str::to_string)
        .ok_or_else(|| invalid(format!("{}: the byte-pair pre-tokenizer is not one the converter recognizes (probe digest {digest})", path.display())))
}

/// The pre-tokenizer the converter's table names for a probe digest: its last entry for the digest, as the converter's
/// chain of `if`s leaves it when a digest is listed twice.
pub fn pre_tokenizer_named(digest: &str) -> Option<&'static str> {
    PRE_TOKENIZER_DIGESTS.iter().rev().find(|(d, _)| *d == digest).map(|(_, name)| *name)
}

/// A chat template as `tokenizer_config.json` or the template files state it: one, or several by name.
enum Template {
    One(String),
    Named(Vec<(String, Option<String>)>),
}

/// `gguf.SpecialVocab`: the merges, special-token ids, added-token flags, chat template and normalizer flags read
/// from `tokenizer.json`, `tokenizer_config.json`, `config.json` and `merges.txt`.
struct SpecialVocab {
    n_vocab: Option<usize>,
    types: Vec<String>,
    merges: Vec<Vec<u8>>,
    ids: Vec<(String, u32)>,
    add: Vec<(String, bool)>,
    chat_template: Option<Template>,
    lowercase: Option<bool>,
    strip_accents: Option<bool>,
}

impl SpecialVocab {
    fn load(dir: &Path, load_merges: bool, n_vocab: Option<usize>, types: &[&str]) -> io::Result<SpecialVocab> {
        let mut sv = SpecialVocab {
            n_vocab,
            types: types.iter().map(|t| t.to_string()).collect(),
            merges: Vec::new(),
            ids: Vec::new(),
            add: Vec::new(),
            chat_template: None,
            lowercase: None,
            strip_accents: None,
        };
        sv.from_tokenizer_json(dir, load_merges)?;
        sv.from_config_json(dir)?;
        if load_merges && sv.merges.is_empty() {
            sv.from_merges_txt(dir)?;
        }
        Ok(sv)
    }

    /// `_set_special_token`: the first id a kind is given stands; an id past the vocabulary is skipped.
    fn set_id(&mut self, typ: &str, tid: &Value) -> io::Result<()> {
        let tid = match tid {
            Value::Bool(b) => i64::from(*b),
            Value::Number(n) if n.is_i64() || n.is_u64() => n.as_i64().unwrap_or(i64::MAX),
            _ => return Ok(()),
        };
        if tid < 0 {
            return Err(invalid(format!("invalid value for special token type {typ}: {tid}")));
        }
        if self.n_vocab.is_none_or(|n| (tid as u64) < n as u64) && !self.ids.iter().any(|(t, _)| t == typ) {
            self.ids.push((typ.to_string(), tid as u32));
        }
        Ok(())
    }

    /// An id set over whatever the files gave the kind (the converter's `special_token_ids[typ] = id`).
    fn force_id(&mut self, typ: &str, id: u32) {
        match self.ids.iter_mut().find(|(t, _)| t == typ) {
            Some(e) => e.1 = id,
            None => self.ids.push((typ.to_string(), id)),
        }
    }

    fn set_add(&mut self, typ: &str, v: bool) {
        match self.add.iter_mut().find(|(t, _)| t == typ) {
            Some(e) => e.1 = v,
            None => self.add.push((typ.to_string(), v)),
        }
    }

    fn add_default(&mut self, typ: &str, v: bool) {
        if !self.add.iter().any(|(t, _)| t == typ) {
            self.add.push((typ.to_string(), v));
        }
    }

    fn has_add(&self, typ: &str) -> bool {
        self.add.iter().any(|(t, _)| t == typ)
    }

    fn parse_normalizer(&mut self, n: &Value) {
        match n["type"].as_str() {
            Some("Lowercase") => self.lowercase = Some(true),
            Some("StripAccents") => self.strip_accents = Some(true),
            Some("BertNormalizer") => {
                if let Some(v) = n.get("lowercase") {
                    self.lowercase = v.as_bool();
                }
                if let Some(v) = n.get("strip_accents") {
                    self.strip_accents = v.as_bool();
                }
            }
            Some("Sequence") => {
                for m in n["normalizers"].as_array().into_iter().flatten() {
                    self.parse_normalizer(m);
                }
            }
            _ => {}
        }
    }

    /// `_try_load_from_tokenizer_json`.
    fn from_tokenizer_json(&mut self, dir: &Path, load_merges: bool) -> io::Result<()> {
        let tj_path = dir.join("tokenizer.json");
        let tokenizer = read_json(&tj_path)?;
        let empty = Vec::new();
        let mut added_tokens: &Vec<Value> = &empty;
        if let Some(t) = &tokenizer {
            if load_merges {
                if let Some(merges) = t["model"]["merges"].as_array().filter(|m| !m.is_empty()) {
                    if merges[0].is_string() {
                        self.merges = merges.iter().map(|m| m.as_str().unwrap_or_default().as_bytes().to_vec()).collect();
                    } else if merges[0].as_array().is_some_and(|p| p.len() == 2 && p[0].is_string()) {
                        // A pair's own spaces are written as U+0120, as the converter writes them.
                        self.merges = merges
                            .iter()
                            .map(|pair| {
                                pair.as_array()
                                    .into_iter()
                                    .flatten()
                                    .map(|part| part.as_str().unwrap_or_default().replace(' ', "\u{120}"))
                                    .collect::<Vec<_>>()
                                    .join(" ")
                                    .into_bytes()
                            })
                            .collect();
                    } else {
                        return Err(invalid(format!("{}: an unknown merges format", tj_path.display())));
                    }
                }
            }
            if truthy(t.get("normalizer")) {
                self.parse_normalizer(&t["normalizer"]);
            }
            if let Some(a) = t["added_tokens"].as_array() {
                added_tokens = a;
            }
        }
        let tc_path = dir.join("tokenizer_config.json");
        let mut tc: Option<Map<String, Value>> = match read_json(&tc_path)? {
            Some(Value::Object(o)) => Some(o),
            Some(_) => return Err(invalid(format!("{}: not an object", tc_path.display()))),
            None => None,
        };
        let tc_truthy = |tc: &Option<Map<String, Value>>| tc.as_ref().is_some_and(|c| !c.is_empty());
        if truthy(tokenizer.as_ref()) {
            let t = tokenizer.as_ref().unwrap();
            let get = |tc: &Option<Map<String, Value>>, k: &str| tc.as_ref().and_then(|c| c.get(k)).cloned();
            let mut special_bos = get(&tc, "bos_token");
            let mut special_cls = get(&tc, "cls_token");
            let mut special_eos = get(&tc, "eos_token");
            let mut special_sep = get(&tc, "sep_token");
            if !truthy(special_bos.as_ref()) && truthy(special_cls.as_ref()) && tc_truthy(&tc) {
                special_bos = special_cls.clone();
                tc.as_mut().unwrap().insert("bos_token".into(), special_bos.clone().unwrap());
            }
            if !truthy(special_eos.as_ref()) && truthy(special_sep.as_ref()) && tc_truthy(&tc) {
                special_eos = special_sep.clone();
                tc.as_mut().unwrap().insert("eos_token".into(), special_eos.clone().unwrap());
            }
            if truthy(t.get("post_processor")) {
                let pp = &t["post_processor"];
                let processors: Vec<Value> = match pp.get("processors") {
                    Some(Value::Array(a)) => a.clone(),
                    _ => vec![pp.clone()],
                };
                for processor in &processors {
                    match processor["type"].as_str() {
                        Some("ByteLevel") => {
                            self.add_default("bos", false);
                            self.add_default("eos", false);
                        }
                        Some("RobertaProcessing") => {
                            self.set_add("bos", true);
                            self.set_add("eos", true);
                            self.set_add("sep", true);
                            if !truthy(special_cls.as_ref()) && tc_truthy(&tc) {
                                special_cls = Some(processor.get("cls").and_then(|c| c.get(0)).cloned().unwrap_or(special_bos.clone().unwrap_or(Value::Null)));
                                tc.as_mut().unwrap().insert("cls_token".into(), special_cls.clone().unwrap());
                            }
                            if !truthy(special_sep.as_ref()) && tc_truthy(&tc) {
                                special_sep = Some(processor.get("sep").and_then(|c| c.get(0)).cloned().unwrap_or(special_eos.clone().unwrap_or(Value::Null)));
                                tc.as_mut().unwrap().insert("sep_token".into(), special_sep.clone().unwrap());
                            }
                        }
                        Some("TemplateProcessing") => {
                            let single = processor["single"].as_array().cloned().unwrap_or_default();
                            let mut pair = processor["pair"].as_array().cloned().unwrap_or_default();
                            let special_id = |v: &Value| v.get("SpecialToken").and_then(|s| s.get("id")).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
                            let mut special_first: Option<String> = None;
                            let mut special_last: Option<String> = None;
                            if single.len() > 1 {
                                special_first = special_id(&single[0]);
                                if let Some(first) = &special_first {
                                    if !tc_truthy(&tc) {
                                        special_bos = Some(Value::String(first.clone()));
                                    } else if !is(&special_bos, first) && !is(&special_cls, first) {
                                        if !truthy(special_bos.as_ref()) {
                                            special_bos = Some(Value::String(first.clone()));
                                            tc.as_mut().unwrap().insert("bos_token".into(), Value::String(first.clone()));
                                        }
                                        if !truthy(special_cls.as_ref()) {
                                            special_cls = Some(Value::String(first.clone()));
                                            tc.as_mut().unwrap().insert("cls_token".into(), Value::String(first.clone()));
                                        }
                                    }
                                    let v = is(&special_bos, first) || is(&special_cls, first);
                                    self.set_add("bos", v);
                                }
                                special_last = special_id(&single[single.len() - 1]);
                                if let Some(last) = &special_last {
                                    if !tc_truthy(&tc) {
                                        special_eos = Some(Value::String(last.clone()));
                                    } else if !is(&special_eos, last) {
                                        if !self.types.iter().any(|t| t == "eot") {
                                            self.types.push("eot".into());
                                            tc.as_mut().unwrap().insert("eot_token".into(), special_eos.clone().unwrap_or(Value::Null));
                                        } else if !self.types.iter().any(|t| t == "eom") {
                                            self.types.push("eom".into());
                                            tc.as_mut().unwrap().insert("eom_token".into(), special_eos.clone().unwrap_or(Value::Null));
                                        }
                                        special_eos = Some(Value::String(last.clone()));
                                        tc.as_mut().unwrap().insert("eos_token".into(), Value::String(last.clone()));
                                    }
                                    let v = is(&special_eos, last);
                                    self.set_add("eos", v);
                                }
                            }
                            if !pair.is_empty() {
                                let start = usize::from(special_first.is_some() && special_id(&pair[0]) == special_first);
                                let stop = special_last.is_some() && special_id(&pair[pair.len() - 1]) == special_last;
                                let end = if stop { pair.len().saturating_sub(1) } else { pair.len() };
                                pair = if start <= end { pair[start..end].to_vec() } else { Vec::new() };
                                if !pair.is_empty() {
                                    let seq = |v: &Value| v.get("Sequence").and_then(|s| s.get("id")).and_then(Value::as_str).map(str::to_string);
                                    let (a, b) = (seq(&pair[0]), seq(&pair[pair.len() - 1]));
                                    if a.as_deref() == Some("A") && b.as_deref() == Some("B") && pair.len() > 2 {
                                        let inner = &pair[1..pair.len() - 1];
                                        let mut add_sep = false;
                                        if let Some(entry) = special_id(&inner[0]) {
                                            if (is(&special_sep, &entry) || is(&special_eos, &entry)) && special_last.is_none() {
                                                add_sep = true;
                                            }
                                        }
                                        if inner.len() == 2 {
                                            if let Some(entry) = special_id(&inner[1]) {
                                                if is(&special_sep, &entry) || is(&special_eos, &entry) {
                                                    add_sep = true;
                                                }
                                            }
                                        }
                                        self.set_add("sep", add_sep);
                                        if add_sep && !truthy(special_sep.as_ref()) && tc_truthy(&tc) {
                                            tc.as_mut().unwrap().insert("sep_token".into(), special_eos.clone().unwrap_or(Value::Null));
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        let Some(tc) = tc.filter(|c| !c.is_empty()) else { return Ok(()) };
        let jinja = dir.join("chat_template.jinja");
        let alt: Option<Template> = if jinja.is_file() {
            let default = std::fs::read_to_string(&jinja)?;
            let mut more: Vec<(String, Option<String>)> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(dir.join("additional_chat_templates")) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "jinja") {
                        let name = p.file_stem().unwrap().to_string_lossy().to_string();
                        more.push((name, Some(std::fs::read_to_string(&p)?)));
                    }
                }
            }
            if more.is_empty() {
                Some(Template::One(default))
            } else {
                more.insert(0, ("default".into(), Some(default)));
                Some(Template::Named(more))
            }
        } else {
            match read_json(&dir.join("chat_template.json"))? {
                Some(j) => template_of(j.get("chat_template")),
                None => None,
            }
        };
        self.chat_template = match tc.get("chat_template") {
            Some(v) => template_of(Some(v)),
            None => alt,
        };
        let types = self.types.clone();
        for typ in &types {
            if let Some(Value::Bool(b)) = tc.get(&format!("add_{typ}_token")) {
                if !self.has_add(typ) {
                    self.add.push((typ.clone(), *b));
                }
            }
            let content = match tc.get(&format!("{typ}_token")) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Object(o)) => match o.get("content") {
                    Some(Value::String(s)) => s.clone(),
                    _ => continue,
                },
                _ => continue,
            };
            let id = added_tokens.iter().find(|a| a["content"].as_str() == Some(content.as_str())).and_then(|a| a.get("id")).cloned();
            if let Some(id) = id {
                self.set_id(typ, &id)?;
            }
        }
        Ok(())
    }

    /// `_try_load_from_config_json`: `<kind>_token_id` of `config.json`, or of its `text_config`.
    fn from_config_json(&mut self, dir: &Path) -> io::Result<()> {
        let Some(config) = read_json(&dir.join("config.json"))? else { return Ok(()) };
        for typ in self.types.clone() {
            let key = format!("{typ}_token_id");
            let mut id = config.get(&key).filter(|v| !v.is_null());
            if id.is_none() {
                id = config.get("text_config").and_then(|t| t.get(&key));
            }
            if let Some(id) = id {
                self.set_id(&typ, &id.clone())?;
            }
        }
        Ok(())
    }

    /// `_try_load_merges_txt`.
    fn from_merges_txt(&mut self, dir: &Path) -> io::Result<()> {
        let path = dir.join("merges.txt");
        if !path.is_file() {
            return Ok(());
        }
        let text = std::fs::read_to_string(&path)?;
        let mut lines = text.lines().peekable();
        if lines.peek().is_some_and(|l| l.trim().starts_with('#')) {
            lines.next();
        }
        let mut merges = Vec::new();
        for line in lines {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() == 2 {
                merges.push(format!("{} {}", parts[0], parts[1]).into_bytes());
            }
        }
        self.merges = merges;
        Ok(())
    }

    /// `add_to_gguf`: every kind with a key of its own; a kind the writer has no key for is left out.
    fn write(&self, w: &mut Keys) {
        if !self.merges.is_empty() {
            w.strings("tokenizer.ggml.merges", &self.merges);
        }
        for (typ, id) in &self.ids {
            let key = match typ.as_str() {
                "bos" => "tokenizer.ggml.bos_token_id",
                "eos" => "tokenizer.ggml.eos_token_id",
                "unk" => "tokenizer.ggml.unknown_token_id",
                "sep" => "tokenizer.ggml.seperator_token_id",
                "pad" => "tokenizer.ggml.padding_token_id",
                "mask" => "tokenizer.ggml.mask_token_id",
                "eot" => "tokenizer.ggml.eot_token_id",
                "eom" => "tokenizer.ggml.eom_token_id",
                _ => continue,
            };
            w.u32(key, *id);
        }
        for (typ, v) in &self.add {
            let key = match typ.as_str() {
                "bos" => "tokenizer.ggml.add_bos_token",
                "eos" => "tokenizer.ggml.add_eos_token",
                "sep" => "tokenizer.ggml.add_sep_token",
                _ => continue,
            };
            w.bool(key, *v);
        }
        match &self.chat_template {
            None => {}
            Some(Template::One(t)) => w.str("tokenizer.chat_template", t),
            Some(Template::Named(choices)) => {
                let mut default = None;
                let mut names: Vec<Vec<u8>> = Vec::new();
                for (name, template) in choices {
                    let name: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
                    let Some(template) = template else { continue };
                    if name.is_empty() {
                        continue;
                    }
                    if name == "default" {
                        default = Some(template.clone());
                    } else {
                        if !names.contains(&name.as_bytes().to_vec()) {
                            names.push(name.as_bytes().to_vec());
                        }
                        w.str(&format!("tokenizer.chat_template.{name}"), template);
                    }
                }
                if !names.is_empty() {
                    w.strings("tokenizer.chat_templates", &names);
                }
                if let Some(t) = default {
                    w.str("tokenizer.chat_template", &t);
                }
            }
        }
        if let Some(b) = self.lowercase {
            w.bool("tokenizer.ggml.normalizer.lowercase", b);
        }
        if let Some(b) = self.strip_accents {
            w.bool("tokenizer.ggml.normalizer.strip_accents", b);
        }
    }
}

/// A `chat_template` value: a string, or a list of `{name, template}` choices; anything else is ignored.
fn template_of(v: Option<&Value>) -> Option<Template> {
    match v {
        Some(Value::String(s)) => Some(Template::One(s.clone())),
        Some(Value::Array(a)) => Some(Template::Named(
            a.iter()
                .map(|c| (c["name"].as_str().unwrap_or_default().to_string(), c["template"].as_str().map(str::to_string)))
                .collect(),
        )),
        _ => None,
    }
}
