//! The pre-tokenizers of byte-pair vocabularies: which `tokenizer.ggml.pre` names select which splitter, the regular
//! expressions each one splits a fragment by (in order, each refining the spans of the last), and the vocabulary
//! settings a name implies. The table is llama.cpp's (`llama-vocab.cpp`, `llm_tokenizer_bpe` and
//! `llama_vocab::impl::load`); `model_tokenizer_lock` reads both tables out of the pinned source and holds this one
//! equal to them.

/// A byte-pair pre-tokenizer, named as llama.cpp's `LLAMA_VOCAB_PRE_TYPE_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreType {
    Default,
    Llama3,
    DeepseekLlm,
    DeepseekCoder,
    Falcon,
    Mpt,
    Starcoder,
    Gpt2,
    Refact,
    CommandR,
    Stablelm2,
    Qwen2,
    Qwen35,
    Olmo,
    Dbrx,
    Smaug,
    Poro,
    Chatglm4,
    Viking,
    Jais,
    Jais2,
    Tekken,
    Smollm,
    Codeshell,
    Bloom,
    Gpt3Finnish,
    Exaone,
    ExaoneMoe,
    Chameleon,
    Minerva,
    Deepseek3Llm,
    Gpt4o,
    GraniteEmbMulti,
    TinyAya,
    Superbpe,
    Trillion,
    GraniteDocling,
    Bailingmoe,
    SeedCoder,
    Hunyuan,
    HunyuanDense,
    HyV4,
    JoyaiLlm,
    KimiK2,
    Ufakzeka,
    Grok2,
    Afmoe,
    Laguna,
    MinimaxM2,
    SolarOpen,
    Mellum2,
    K2Horizon,
    Spark2_5,
    Youtu,
    Gemma4,
    SarvamMoe,
    Mmbert,
    Minicpm5,
    Whitespace,
}

/// The vocabulary settings a pre-tokenizer name implies, over the byte-pair defaults (`clean_spaces` on, the rest
/// off). `None` leaves the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreSettings {
    pub clean_spaces: Option<bool>,
    pub ignore_merges: Option<bool>,
    pub add_bos: Option<bool>,
    pub escape_whitespaces: Option<bool>,
    /// The name clears the default beginning-of-sequence token (`chatglm-bpe`, `glm4`, `glm5`).
    pub no_bos_token: bool,
    /// The name clears the lowercase normalizer (`whitespace`).
    pub no_lowercase: bool,
}

const LLAMA3: &str = "(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+";
const QWEN2: &str = "(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+";
const GPT2: &str = "'s|'t|'re|'ve|'m|'ll|'d| ?\\p{L}+| ?\\p{N}+| ?[^\\s\\p{L}\\p{N}]+|\\s+(?!\\S)";
const PORO: &str = " ?[^(\\s|.,!?…。，、।۔،)]+";
const NEWLINES: &str = "[^\\n]+|[\\n]+";

impl PreType {
    /// llama.cpp's name for the type, without the `LLAMA_VOCAB_PRE_TYPE_` prefix.
    pub fn llama_name(self) -> &'static str {
        match self {
            PreType::Default => "DEFAULT",
            PreType::Llama3 => "LLAMA3",
            PreType::DeepseekLlm => "DEEPSEEK_LLM",
            PreType::DeepseekCoder => "DEEPSEEK_CODER",
            PreType::Falcon => "FALCON",
            PreType::Mpt => "MPT",
            PreType::Starcoder => "STARCODER",
            PreType::Gpt2 => "GPT2",
            PreType::Refact => "REFACT",
            PreType::CommandR => "COMMAND_R",
            PreType::Stablelm2 => "STABLELM2",
            PreType::Qwen2 => "QWEN2",
            PreType::Qwen35 => "QWEN35",
            PreType::Olmo => "OLMO",
            PreType::Dbrx => "DBRX",
            PreType::Smaug => "SMAUG",
            PreType::Poro => "PORO",
            PreType::Chatglm4 => "CHATGLM4",
            PreType::Viking => "VIKING",
            PreType::Jais => "JAIS",
            PreType::Jais2 => "JAIS2",
            PreType::Tekken => "TEKKEN",
            PreType::Smollm => "SMOLLM",
            PreType::Codeshell => "CODESHELL",
            PreType::Bloom => "BLOOM",
            PreType::Gpt3Finnish => "GPT3_FINNISH",
            PreType::Exaone => "EXAONE",
            PreType::ExaoneMoe => "EXAONE_MOE",
            PreType::Chameleon => "CHAMELEON",
            PreType::Minerva => "MINERVA",
            PreType::Deepseek3Llm => "DEEPSEEK3_LLM",
            PreType::Gpt4o => "GPT4O",
            PreType::GraniteEmbMulti => "GRANITE_EMB_MULTI",
            PreType::TinyAya => "TINY_AYA",
            PreType::Superbpe => "SUPERBPE",
            PreType::Trillion => "TRILLION",
            PreType::GraniteDocling => "GRANITE_DOCLING",
            PreType::Bailingmoe => "BAILINGMOE",
            PreType::SeedCoder => "SEED_CODER",
            PreType::Hunyuan => "HUNYUAN",
            PreType::HunyuanDense => "HUNYUAN_DENSE",
            PreType::HyV4 => "HY_V4",
            PreType::JoyaiLlm => "JOYAI_LLM",
            PreType::KimiK2 => "KIMI_K2",
            PreType::Ufakzeka => "UFAKZEKA",
            PreType::Grok2 => "GROK_2",
            PreType::Afmoe => "AFMOE",
            PreType::Laguna => "LAGUNA",
            PreType::MinimaxM2 => "MINIMAX_M2",
            PreType::SolarOpen => "SOLAR_OPEN",
            PreType::Mellum2 => "MELLUM2",
            PreType::K2Horizon => "K2_HORIZON",
            PreType::Spark2_5 => "SPARK2_5",
            PreType::Youtu => "YOUTU",
            PreType::Gemma4 => "GEMMA4",
            PreType::SarvamMoe => "SARVAM_MOE",
            PreType::Mmbert => "MMBERT",
            PreType::Minicpm5 => "MINICPM5",
            PreType::Whitespace => "WHITESPACE",
        }
    }

    /// Every type, in declaration order.
    pub const ALL: [PreType; 59] = [
        PreType::Default,
        PreType::Llama3,
        PreType::DeepseekLlm,
        PreType::DeepseekCoder,
        PreType::Falcon,
        PreType::Mpt,
        PreType::Starcoder,
        PreType::Gpt2,
        PreType::Refact,
        PreType::CommandR,
        PreType::Stablelm2,
        PreType::Qwen2,
        PreType::Qwen35,
        PreType::Olmo,
        PreType::Dbrx,
        PreType::Smaug,
        PreType::Poro,
        PreType::Chatglm4,
        PreType::Viking,
        PreType::Jais,
        PreType::Jais2,
        PreType::Tekken,
        PreType::Smollm,
        PreType::Codeshell,
        PreType::Bloom,
        PreType::Gpt3Finnish,
        PreType::Exaone,
        PreType::ExaoneMoe,
        PreType::Chameleon,
        PreType::Minerva,
        PreType::Deepseek3Llm,
        PreType::Gpt4o,
        PreType::GraniteEmbMulti,
        PreType::TinyAya,
        PreType::Superbpe,
        PreType::Trillion,
        PreType::GraniteDocling,
        PreType::Bailingmoe,
        PreType::SeedCoder,
        PreType::Hunyuan,
        PreType::HunyuanDense,
        PreType::HyV4,
        PreType::JoyaiLlm,
        PreType::KimiK2,
        PreType::Ufakzeka,
        PreType::Grok2,
        PreType::Afmoe,
        PreType::Laguna,
        PreType::MinimaxM2,
        PreType::SolarOpen,
        PreType::Mellum2,
        PreType::K2Horizon,
        PreType::Spark2_5,
        PreType::Youtu,
        PreType::Gemma4,
        PreType::SarvamMoe,
        PreType::Mmbert,
        PreType::Minicpm5,
        PreType::Whitespace,
    ];

    /// The patterns that split a fragment, applied in order.
    pub fn regexes(self) -> &'static [&'static str] {
        match self {
            PreType::Llama3 | PreType::Dbrx | PreType::Smaug | PreType::Chatglm4 => &[LLAMA3],
            PreType::Jais2 => &["(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s{512}(?!\\S)|\\s{256}(?!\\S)|\\s{128}(?!\\S)|\\s{64}(?!\\S)|\\s{32}(?!\\S)|\\s{16}(?!\\S)|\\s{8}(?!\\S)|\\s{4}(?!\\S)|\\s{1,2}(?!\\S)|\\s{1}"],
            PreType::DeepseekLlm => &[
                "[\r\n]",
                "\\s?[A-Za-zµÀ-ÖØ-öø-ƺƼ-ƿǄ-ʓʕ-ʯͰ-ͳͶͷͻ-ͽͿΆΈ-ΊΌΎ-ΡΣ-ϵϷ-ҁҊ-ԯԱ-ՖႠ-ჅᎠ-Ᏽᏸ-ᏽᲐ-ᲺᲽ-Ჿᴀ-ᴫᵫ-ᵷᵹ-ᶚḀ-ἕἘ-Ἕἠ-ὅὈ-Ὅὐ-ὗὙὛὝὟ-ώᾀ-ᾴᾶ-ᾼιῂ-ῄῆ-ῌῐ-ΐῖ-Ίῠ-Ῥῲ-ῴῶ-ῼℂℇℊ-ℓℕℙ-ℝℤΩℨK-ℭℯ-ℴℹℼ-ℿⅅ-ⅉⅎↃↄⰀ-ⱻⱾ-ⳤⳫ-ⳮⳲⳳꙀ-ꙭꚀ-ꚛꜢ-ꝯꝱ-ꞇꞋ-ꞎꭰ-ꮿﬀ-ﬆﬓ-ﬗＡ-Ｚａ-ｚ𐐀-𐑏𐒰-𐓓𐓘-𐓻𐲀-𐲲𐳀-𐳲𑢠-𑣟𞤀-𞥃]+",
                "\\s?[!-/:-~！-／：-～‘-‟　-。]+",
                "\\s+$",
                "[一-龥ࠀ-一가-퟿]+",
                "\\p{N}+",
            ],
            PreType::Deepseek3Llm | PreType::HunyuanDense | PreType::JoyaiLlm | PreType::HyV4 => &[
                "\\p{N}{1,3}",
                "[一-龥぀-ゟ゠-ヿ]+",
                "[!\"#$%&'()*+,\\-./:;<=>?@\\[\\\\\\]^_`{|}~][A-Za-z]+|[^\r\n\\p{L}\\p{P}\\p{S}]?[\\p{L}\\p{M}]+| ?[\\p{P}\\p{S}]+[\r\n]*|\\s*[\r\n]+|\\s+(?!\\S)|\\s+",
            ],
            PreType::Spark2_5 => &[
                "\\p{N}{1,3}",
                "[一-龥぀-ゟ゠-ヿ]+",
                "[!\"#$%&'()*+,\\-./:;<=>?@\\[\\\\\\]^_`{|}~][A-Za-z]+|[^\r\n\\p{L}\\p{P}\\p{S}]?[\\p{L}\\p{M}]+| ?[\\p{P}\\p{S}]+|[\r\n]|\\s+(?!\\S)|\\s+",
                "\\p{N}",
            ],
            PreType::Youtu => &[
                "[가-힣ㄱ-ㆎ]+|[！…“”‘’—：；，、-〿︰-﹏]+|[ㄅ-ㄯ]+|[一-龥぀-ゟ゠-ヿ]+",
                "[^\\r\\n\\p{L}\\p{N}]?[\\p{Lu}\\p{Lt}\\p{Lm}\\p{Lo}\\p{M}]*[\\p{Ll}\\p{Lm}\\p{Lo}\\p{M}]+(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])?|[^\\r\\n\\p{L}\\p{N}]?[\\p{Lu}\\p{Lt}\\p{Lm}\\p{Lo}\\p{M}]+[\\p{Ll}\\p{Lm}\\p{Lo}\\p{M}]*(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])?|\\p{N}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n/]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+",
            ],
            PreType::DeepseekCoder => &["[\r\n]", "\\s?\\p{L}+", "\\s?\\p{P}+", "[一-龥ࠀ-一가-퟿]+", "\\p{N}"],
            PreType::Falcon => &[
                "[\\p{P}\\$\\+<=>\\^~\\|`]+",
                GPT2,
                "[0-9][0-9][0-9]",
            ],
            PreType::Starcoder
            | PreType::Refact
            | PreType::CommandR
            | PreType::Smollm
            | PreType::Codeshell
            | PreType::Exaone
            | PreType::Minerva
            | PreType::Mellum2 => &["\\p{N}", GPT2],
            PreType::Gpt2 | PreType::Mpt | PreType::Olmo | PreType::Jais | PreType::Trillion | PreType::GraniteDocling => {
                &[GPT2]
            }
            PreType::Stablelm2 | PreType::Qwen2 | PreType::Hunyuan | PreType::SolarOpen => &[QWEN2],
            PreType::Qwen35 => &["(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?[\\p{L}\\p{M}]+|\\p{N}| ?[^\\s\\p{L}\\p{M}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+"],
            PreType::Poro | PreType::Bloom | PreType::Gpt3Finnish => &[PORO],
            PreType::Viking => &[PORO, "\\p{N}"],
            PreType::Tekken => &["[^\\r\\n\\p{L}\\p{N}]?((?=[\\p{L}])([^a-z]))*((?=[\\p{L}])([^A-Z]))+|[^\\r\\n\\p{L}\\p{N}]?((?=[\\p{L}])([^a-z]))+((?=[\\p{L}])([^A-Z]))*|\\p{N}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n/]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+"],
            PreType::Chameleon => &[
                "<sentinel:[0-9]+>",
                "(IMGIMG)((A|B|C|D|E|F|G|H|I){1,4})Z",
                "([\\t\\n]|    |  )",
                "\\p{N}",
                "[\\p{P}!-/:-@\\[-`{-~]",
                GPT2,
            ],
            PreType::Gpt4o | PreType::MinimaxM2 => &["[^\\r\\n\\p{L}\\p{N}]?((?=[\\p{L}])([^a-z]))*((?=[\\p{L}])([^A-Z]))+(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])?|[^\\r\\n\\p{L}\\p{N}]?((?=[\\p{L}])([^a-z]))+((?=[\\p{L}])([^A-Z]))*(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])?|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n/]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+"],
            PreType::GraniteEmbMulti => &["[^\\r\\n\\p{L}\\p{N}]?((?=[\\p{L}\\p{M}])([^a-z]))*((?=[\\p{L}\\p{M}])([^A-Z]))+(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])?|[^\\r\\n\\p{L}\\p{N}]?((?=[\\p{L}\\p{M}])([^a-z]))+((?=[\\p{L}\\p{M}])([^A-Z]))*(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])?|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n/]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+"],
            PreType::TinyAya => &[
                "\\d{1,3}(?=(?:\\d{3})*\\b)",
                "[^\\r\\n\\p{L}\\p{N}]?[\\p{Lu}\\p{Lt}\\p{Lm}\\p{Lo}\\p{M}]*[\\p{Ll}\\p{Lm}\\p{Lo}\\p{M}]+(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])?|[^\\r\\n\\p{L}\\p{N}]?[\\p{Lu}\\p{Lt}\\p{Lm}\\p{Lo}\\p{M}]+[\\p{Ll}\\p{Lm}\\p{Lo}\\p{M}]*(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])?|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n/]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+",
            ],
            PreType::KimiK2 => &["\\p{Han}+"],
            PreType::Superbpe => &["\\p{N}+", "(?=(\\d{3})+(?!\\d))"],
            PreType::Bailingmoe => &["'(?:[sSdDmMtT]|[lL][lL]|[vV][eE]|[rR][eE])|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]|\\s+(?!\\S)|\\s+"],
            PreType::SeedCoder => &["(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}{1}| ?[^\\s\\p{L}\\p{N}\\r\\n]+|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+"],
            PreType::Ufakzeka => &["[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+"],
            PreType::Grok2 => &[QWEN2],
            PreType::Afmoe => &[
                "\\p{AFMoE_digits}",
                "[一-鿿㐀-䶿豈-﫿぀-ゟ゠-ヿ･-ﾟ⼀-⿟เ-๿຀-໿ក-៿က-႟ꩠ-ꩿꧠ-꧿가-힯ᄀ-ᇿ]+",
                "[!\"#$%&'()*+,\\-./:;<=>?@\\[\\\\\\]^_`{|}~][A-Za-z]+|[^\\r\\n\\p{L}\\p{P}\\p{S}]?[\\p{L}\\p{M}]+| ?[\\p{P}\\p{S}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+",
            ],
            PreType::Laguna => &[NEWLINES, QWEN2],
            PreType::ExaoneMoe => &["(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?(?:\\p{L}\\p{M}*(?: \\p{L}\\p{M}*)*)+|\\p{N}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n/]?|\\s*[\\r\\n]|\\s+(?!\\S)|\\s+"],
            PreType::Gemma4 | PreType::SarvamMoe | PreType::Mmbert => &[NEWLINES],
            PreType::Minicpm5 => &[
                "\\p{N}{1,3}",
                "(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}+| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+",
            ],
            PreType::K2Horizon => &["(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?(?:\\p{L}|\\p{M}|\\u200C|\\u200D)+|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+"],
            PreType::Whitespace => &["\\S+"],
            PreType::Default => &[
                "[\\p{P}\\$\\+<=>\\^~\\|]+",
                GPT2,
                "\\p{N}+",
                "[0-9][0-9][0-9]",
            ],
        }
    }

    /// Whether a split word is written in GPT-2's byte alphabet before merging (raw UTF-8 otherwise).
    pub fn byte_encode(self) -> bool {
        !matches!(self, PreType::Gemma4 | PreType::SarvamMoe | PreType::Mmbert | PreType::Whitespace)
    }
}

/// The pre-tokenizer a `tokenizer.ggml.pre` name selects, with the settings it implies; `None` for a name llama.cpp
/// does not know (it refuses the model). An absent name reads as `"default"`.
pub fn pre_type(name: &str) -> Option<(PreType, PreSettings)> {
    let s = PreSettings::default;
    let no_clean = || PreSettings { clean_spaces: Some(false), ..s() };
    Some(match name {
        "default" => (PreType::Default, s()),
        "minicpm5" => (PreType::Minicpm5, PreSettings { ignore_merges: Some(true), ..s() }),
        "llama3" | "llama-v3" | "llama-bpe" | "falcon3" | "falcon-h1" | "pixtral" | "midm-2.0" | "lfm2"
        | "jina-v5-nano" => (PreType::Llama3, PreSettings { ignore_merges: Some(true), add_bos: Some(true), ..s() }),
        "deepseek-llm" => (PreType::DeepseekLlm, no_clean()),
        "deepseek-coder" => (PreType::DeepseekCoder, no_clean()),
        "deepseek-v3" => (PreType::Deepseek3Llm, no_clean()),
        "spark2_5" => (PreType::Spark2_5, no_clean()),
        "youtu" => (PreType::Youtu, PreSettings { ignore_merges: Some(true), ..no_clean() }),
        "falcon" => (PreType::Falcon, s()),
        "mpt" => (PreType::Mpt, s()),
        "starcoder" => (PreType::Starcoder, s()),
        "gpt-2" | "phi-2" | "jina-es" | "jina-de" | "gigachat" | "jina-v2-es" | "jina-v2-de" | "a.x-4.0" | "mellum"
        | "modern-bert" => (PreType::Gpt2, s()),
        "jais-2" => (PreType::Jais2, s()),
        "gemma4" | "granite-embed-multi-311m" => {
            (PreType::Gemma4, PreSettings { escape_whitespaces: Some(true), ..s() })
        }
        "mmbert" => (PreType::Mmbert, PreSettings { escape_whitespaces: Some(true), ..s() }),
        "sarvam-moe" => (PreType::SarvamMoe, PreSettings { escape_whitespaces: Some(true), ..no_clean() }),
        "jina-v1-en" | "jina-v2-code" | "roberta-bpe" => (PreType::Gpt2, s()),
        "whitespace" => (PreType::Whitespace, PreSettings { no_lowercase: true, ..s() }),
        "refact" => (PreType::Refact, s()),
        "command-r" => (PreType::CommandR, no_clean()),
        "qwen2" | "deepseek-r1-qwen" | "kormo" | "f2llmv2" => (PreType::Qwen2, no_clean()),
        "qwen35" => (PreType::Qwen35, no_clean()),
        "stablelm2" => (PreType::Stablelm2, s()),
        "olmo" => (PreType::Olmo, s()),
        "dbrx" => (PreType::Dbrx, s()),
        "smaug-bpe" => (PreType::Smaug, s()),
        "poro-chat" => (PreType::Poro, no_clean()),
        "chatglm-bpe" => (PreType::Chatglm4, PreSettings { no_bos_token: true, ..s() }),
        "glm4" | "glm5" => {
            (PreType::Chatglm4, PreSettings { no_bos_token: true, ignore_merges: Some(true), ..s() })
        }
        "viking" => (PreType::Viking, no_clean()),
        "jais" => (PreType::Jais, s()),
        "tekken" => (
            PreType::Tekken,
            PreSettings { ignore_merges: Some(true), add_bos: Some(true), ..no_clean() },
        ),
        "smollm" => (PreType::Smollm, no_clean()),
        "codeshell" => (PreType::Codeshell, s()),
        "bloom" => (PreType::Bloom, s()),
        "gpt3-finnish" => (PreType::Gpt3Finnish, s()),
        "exaone" => (PreType::Exaone, s()),
        "exaone4" => (PreType::Gpt2, s()),
        "exaone-moe" => (PreType::ExaoneMoe, s()),
        "chameleon" => (PreType::Chameleon, PreSettings { add_bos: Some(true), ..no_clean() }),
        "minerva-7b" => (PreType::Minerva, s()),
        "megrez" => (PreType::Qwen2, s()),
        "gpt-4o" | "llama4" | "kanana2" | "talkie" => (PreType::Gpt4o, no_clean()),
        "granite-embed-multi-97m" => {
            (PreType::GraniteEmbMulti, PreSettings { ignore_merges: Some(true), ..no_clean() })
        }
        "tiny_aya" | "cohere2moe" => (PreType::TinyAya, no_clean()),
        "superbpe" => (PreType::Superbpe, no_clean()),
        "trillion" => (PreType::Trillion, no_clean()),
        "granite-docling" => (PreType::GraniteDocling, no_clean()),
        "bailingmoe" | "bailingmoe2" | "llada-moe" => (PreType::Bailingmoe, no_clean()),
        "seed-coder" => (PreType::SeedCoder, no_clean()),
        "hunyuan" => (PreType::Hunyuan, no_clean()),
        "hunyuan-dense" => (PreType::HunyuanDense, no_clean()),
        "hy_v4" => (PreType::HyV4, no_clean()),
        "joyai-llm" => (PreType::JoyaiLlm, no_clean()),
        "kimi-k2" => (PreType::KimiK2, no_clean()),
        "ufakzeka" => (PreType::Ufakzeka, no_clean()),
        "grok-2" => (PreType::Grok2, no_clean()),
        "afmoe" => (PreType::Afmoe, no_clean()),
        "laguna" => (PreType::Laguna, no_clean()),
        "minimax-m2" => (PreType::MinimaxM2, no_clean()),
        "solar-open" => (PreType::SolarOpen, no_clean()),
        "mellum2" => (PreType::Mellum2, s()),
        "k2-horizon" => (PreType::K2Horizon, no_clean()),
        _ => return None,
    })
}
