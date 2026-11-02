//! The `f64` reference forward (`model::reference`) over any checkpoint the spec reads: the oracle a compiled lane
//! or a competitor is compared with.
//!
//!   model_ref <gguf> <n_gen> <tok,tok,…>        prompt as token ids (comma- or space-separated)
//!   model_ref <gguf> <n_gen> --text "<prompt>"  prompt as text, tokenized by the checkpoint's own vocabulary
//!
//! Prints `REF argmax=<id>(<logit>)` for the prompt's last position, then `REF_TOKENS:<greedy continuation>` and,
//! from a text prompt, `REF_TEXT:<the continuation as text>`. With `LOGOS_REF_LOGITS=<path>` it writes the logits
//! at the prompt's last position — the position `llama_get_logits_ith(n_prompt − 1)` returns — as little-endian
//! `f32`, the precision either GPU lane can be asked for; with `LOGOS_REF_LOGITS_EVERY=<path>` the logits after every
//! prompt token, row after row. For a model with experts it names the prompt's closest routing
//! call ([`Reference::closest_routing`]); `LOGOS_REF_FLIP=<layer>,<position>` decides that call the other way.

use std::io::Write;
use std::path::Path;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::reference::{argmax, Reference, RoutingCall};
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::model::tokenizer::Vocab;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: model_ref <gguf> <n_gen> <tok,tok,…> | --text \"<prompt>\"");
        std::process::exit(2);
    }
    let g = Gguf::open(Path::new(&args[1])).expect("the checkpoint opens");
    let n_gen: usize = args[2].parse().expect("n_gen is a number");
    let vocab = Vocab::from_gguf(&g).expect("the vocabulary reads");
    let from_text = args[3] == "--text";
    let prompt: Vec<u32> = if from_text {
        let text = args.get(4).expect("--text takes the prompt");
        vocab.tokenize(text.as_bytes(), true, false).expect("the prompt tokenizes")
    } else {
        args[3..]
            .iter()
            .flat_map(|s| s.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().unwrap_or_else(|_| panic!("prompt token {s:?} is not a number")))
            .collect()
    };
    let spec = ModelSpec::from_gguf(&g).expect("the spec reads");
    eprintln!(
        "ref: {} d={} layers={} vocab={} prompt={prompt:?}",
        spec.arch,
        spec.d,
        spec.layers.len(),
        spec.vocab
    );
    let mut model = Reference::new(spec, g).expect("the reference runs the model");
    if let Ok(at) = std::env::var("LOGOS_REF_FLIP") {
        let (layer, position) = at.split_once(',').expect("LOGOS_REF_FLIP is <layer>,<position>");
        let call = RoutingCall {
            layer: layer.parse().expect("a layer"),
            position: position.parse().expect("a position"),
            margin: 0.0,
            score: 0.0,
        };
        model.flip_routing(call);
        eprintln!("REF routing at layer {} position {} decided the other way", call.layer, call.position);
    }
    let mut logits = match std::env::var("LOGOS_REF_LOGITS_EVERY") {
        Ok(path) => {
            let every = model.step_every(&prompt).expect("the prompt runs");
            let bytes: Vec<u8> = every.iter().flatten().flat_map(|&v| (v as f32).to_le_bytes()).collect();
            std::fs::write(&path, bytes).expect("write the logit dump");
            eprintln!("REF WROTE_LOGITS_EVERY {path} ({} × {})", every.len(), every[0].len());
            every.into_iter().last().expect("a prompt")
        }
        Err(_) => model.step(&prompt).expect("the prompt runs"),
    };
    let best = argmax(&logits);
    eprintln!("REF argmax={best}({:.4})", logits[best as usize]);
    if let Some(c) = model.closest_routing() {
        eprintln!(
            "REF closest routing: layer {} position {}, the last expert chosen {:e} above the next on a score of {:.6}",
            c.layer, c.position, c.margin, c.score
        );
    }
    if let Ok(path) = std::env::var("LOGOS_REF_LOGITS") {
        let bytes: Vec<u8> = logits.iter().flat_map(|&v| (v as f32).to_le_bytes()).collect();
        let mut f = std::fs::File::create(&path).expect("create the logit dump");
        f.write_all(&bytes).expect("write the logits");
        f.sync_all().expect("flush the logits");
        eprintln!("REF WROTE_LOGITS {path} ({})", logits.len());
    }
    let mut out = Vec::with_capacity(n_gen);
    for i in 0..n_gen {
        let t = argmax(&logits);
        out.push(t);
        eprintln!("pos {} -> argmax {t} (logit {:.4})", prompt.len() + i, logits[t as usize]);
        if vocab.is_eog(t) || i + 1 == n_gen {
            break;
        }
        logits = model.step(&[t]).expect("the step runs");
    }
    println!("REF_TOKENS:{}", out.iter().map(u32::to_string).collect::<Vec<_>>().join(" "));
    if from_text {
        println!("REF_TEXT:{}", String::from_utf8_lossy(&vocab.detokenize(&out, false, false)));
    }
}
