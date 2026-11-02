//! A checkpoint with every tensor dequantized to `f32` — the weights the `f64` reference reads, at the precision
//! llama.cpp computes `f32` weights in, so its run of the copy (with an `f32` KV cache) is the oracle for what the
//! reference computes, apart from quantization.
//!
//!   gguf_f32 <in.gguf> <out.gguf>

use std::path::Path;

use logicaffeine_compile::model::gguf::{self, Gguf, MetaValue, TensorOut};
use logicaffeine_compile::model::quant::GgmlType;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: gguf_f32 <in.gguf> <out.gguf>");
    let mut g = Gguf::open(Path::new(&args[1])).expect("the checkpoint opens");
    let mut meta: Vec<(String, MetaValue)> = g.order.iter().map(|k| (k.clone(), g.meta[k].clone())).collect();
    for (k, v) in &mut meta {
        if k == "general.file_type" {
            *v = MetaValue::U32(0);
        }
    }
    let tensors: Vec<TensorOut> = g
        .tensors()
        .iter()
        .map(|t| TensorOut { name: t.name.clone(), dims: t.dims.clone(), ty: GgmlType::F32 })
        .collect();
    let names: Vec<String> = tensors.iter().map(|t| t.name.clone()).collect();
    gguf::write(Path::new(&args[2]), &meta, &tensors, |i| {
        let values = g.dequant_tensor(&names[i])?;
        eprintln!("{} ({} values)", names[i], values.len());
        Ok(values.iter().flat_map(|v| v.to_le_bytes()).collect())
    })
    .expect("the copy writes");
}
