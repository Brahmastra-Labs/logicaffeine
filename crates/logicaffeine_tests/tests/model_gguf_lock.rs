//! The GGUF writer (`model::gguf::write`) writes what the reader reads back: every metadata type (an empty array
//! keeps its element type), the keys in their order, the tensor directory, and each tensor's bytes at its aligned
//! offset — and a real checkpoint's whole metadata table round-trips unchanged.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/scratch.rs"]
mod scratch;

use logicaffeine_compile::model::gguf::{self, Gguf, MetaValue, TensorOut};
use logicaffeine_compile::model::quant::GgmlType;

fn scratch(name: &str) -> std::path::PathBuf {
    scratch::dir("logos_gguf_lock").join(name)
}

#[test]
fn every_metadata_type_and_tensor_round_trips() {
    let meta: Vec<(String, MetaValue)> = vec![
        ("general.architecture".into(), MetaValue::Str(b"test".to_vec())),
        ("general.alignment".into(), MetaValue::U32(64)),
        ("k.u8".into(), MetaValue::U8(250)),
        ("k.i8".into(), MetaValue::I8(-7)),
        ("k.u16".into(), MetaValue::U16(65000)),
        ("k.i16".into(), MetaValue::I16(-30000)),
        ("k.u32".into(), MetaValue::U32(4_000_000_000)),
        ("k.i32".into(), MetaValue::I32(-2_000_000_000)),
        ("k.f32".into(), MetaValue::F32(1e-6)),
        ("k.bool".into(), MetaValue::Bool(true)),
        ("k.bytes".into(), MetaValue::Str(vec![0xff, 0x00, b'a'])),
        ("k.u64".into(), MetaValue::U64(u64::MAX - 3)),
        ("k.i64".into(), MetaValue::I64(i64::MIN + 5)),
        ("k.f64".into(), MetaValue::F64(-0.1)),
        ("k.empty".into(), MetaValue::Array { ty: 6, items: vec![] }),
        (
            "k.strings".into(),
            MetaValue::Array { ty: 8, items: vec![MetaValue::Str(b"a".to_vec()), MetaValue::Str(vec![])] },
        ),
        (
            "k.nested".into(),
            MetaValue::Array {
                ty: 9,
                items: vec![MetaValue::Array { ty: 5, items: vec![MetaValue::I32(1), MetaValue::I32(-1)] }],
            },
        ),
    ];
    let tensors = vec![
        TensorOut { name: "a".into(), dims: vec![3], ty: GgmlType::F32 },
        TensorOut { name: "b".into(), dims: vec![32, 2], ty: GgmlType::Q8_0 },
        TensorOut { name: "c".into(), dims: vec![5, 1, 1], ty: GgmlType::F16 },
    ];
    let data: Vec<Vec<u8>> = tensors
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let n = t.ty.bytes_for(t.dims.iter().product::<u64>() as usize);
            (0..n).map(|k| (k * 7 + i * 13) as u8).collect()
        })
        .collect();
    let path = scratch("round.gguf");
    gguf::write(&path, &meta, &tensors, |i| Ok(data[i].clone())).unwrap();
    let mut g = Gguf::open(&path).unwrap();
    assert_eq!(g.version, 3);
    assert_eq!(g.order, meta.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(), "the keys keep their order");
    for (k, v) in &meta {
        assert_eq!(&g.meta[k], v, "{k}");
    }
    assert_eq!(g.tensors().len(), 3);
    for (i, t) in tensors.iter().enumerate() {
        let info = g.info(&t.name).unwrap().clone();
        assert_eq!(info.dims, t.dims, "{}", t.name);
        assert_eq!(g.tensor_type(&t.name).unwrap(), t.ty);
        assert_eq!(info.offset % 64, 0, "{} is aligned", t.name);
        let rows = t.dims.iter().product::<u64>() as usize / t.dims[0] as usize;
        assert_eq!(g.row_bytes(&t.name, 0, rows).unwrap(), data[i], "{}", t.name);
    }
}

#[test]
fn a_tensor_whose_bytes_do_not_fit_its_type_is_refused() {
    let path = scratch("short.gguf");
    let err = gguf::write(
        &path,
        &[],
        &[TensorOut { name: "t".into(), dims: vec![32], ty: GgmlType::Q8_0 }],
        |_| Ok(vec![0u8; 3]),
    )
    .unwrap_err();
    assert!(err.to_string().contains("3 bytes for 32 q8_0 weights"), "{err}");
}

#[test]
fn a_real_checkpoints_metadata_round_trips() {
    if !common::require_tool("checkpoint:qwen3.5-0.8b") {
        return;
    }
    let mut g = Gguf::open(&common::checkpoint("qwen3.5-0.8b").unwrap()).unwrap();
    let meta: Vec<(String, MetaValue)> = g.order.iter().map(|k| (k.clone(), g.meta[k].clone())).collect();
    let keep: Vec<String> = g.tensors().iter().take(4).map(|t| t.name.clone()).collect();
    let tensors: Vec<TensorOut> = keep
        .iter()
        .map(|n| {
            let info = g.info(n).unwrap();
            TensorOut { name: n.clone(), dims: info.dims.clone(), ty: g.tensor_type(n).unwrap() }
        })
        .collect();
    let mut bytes = Vec::new();
    for t in &tensors {
        let rows = t.dims.iter().product::<u64>() as usize / t.dims[0] as usize;
        bytes.push(g.row_bytes(&t.name, 0, rows).unwrap());
    }
    let path = scratch("real.gguf");
    gguf::write(&path, &meta, &tensors, |i| Ok(bytes[i].clone())).unwrap();
    let mut back = Gguf::open(&path).unwrap();
    assert_eq!(back.order, g.order);
    assert_eq!(back.meta, g.meta, "every key, every value, every token string");
    for (t, b) in tensors.iter().zip(&bytes) {
        let rows = t.dims.iter().product::<u64>() as usize / t.dims[0] as usize;
        assert_eq!(&back.row_bytes(&t.name, 0, rows).unwrap(), b, "{}", t.name);
    }
}
