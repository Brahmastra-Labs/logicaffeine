//! model_host_fill_lock — what a host fills a program's declared tensors with, read from the checkpoint alone.
//!
//! A program the author writes declares its weights in the layout the language reads them, and a bundle's manifest binds
//! each to a checkpoint tensor (`model::bind`). The host fills:
//!   • a weight through its view (`values`), in the layout `View::declared` gives;
//!   • a weight a spatial program reads as it is stored Q8_0 (`spatial::passes::from_q8`) as two planes repacked from
//!     the checkpoint's own blocks (`q8_planes`, a manifest's `q8_0.values` and `q8_0.scales`): the int8 values in the
//!     declared layout, `K × N`, and one fp32 scale per 32 rows of each column, `K/32 × N`, each plane block's scale the
//!     scale of the one checkpoint block its 32 rows come from;
//!   • a rotary table (`rotary_values`) from the spec and, for a rotation that reads them, the checkpoint's frequency
//!     factors.
//!
//! Locked here:
//!   • the layout `View::declared` gives is the one `values` reads and the one the author declares, for every weight of
//!     every tiny model;
//!   • the planes are the checkpoint's bytes, read by hand off a tensor of known blocks;
//!   • `values[k][n] · scales[k/32][n]` is the weight `values` reads, element for element, for every weight of every
//!     tiny model whose blocks run down its declared rows;
//!   • a weight stored in another format, a weight whose blocks run along its declared columns (every tiny model's
//!     weights read as stored), and one whose declared rows are not whole blocks are each refused by name;
//!   • every rotary table a decode step of every tiny model declares is its declared size and turns position `p`'s
//!     frequency `i` by `p·θ^(−2i/dims)` (over the checkpoint's factor when the rotation reads one); a forward's table is
//!     position 0's; a name that is no table is no table, and a rotation the model does not have is refused.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test model_host_fill_lock`

#![cfg(feature = "codegen")]

#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::model::author;
use logicaffeine_compile::model::bind::{bind, q8_planes, rotary_values, rotations, stored_as_q8_planes, values, Binding, RotaryTable, Source, View};
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::program::{written, Cut};
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::reference::WeightSource;
use logicaffeine_compile::model::spec::{ModelSpec, RopeScaling};
use std::collections::BTreeMap;
use tiny_models::{with_rope_factors, write_checkpoint, write_checkpoint_as, Tiny, ALL};

const CTX: usize = 8;

/// Every tiny model at both widths the author writes, with what it is called.
fn models() -> Vec<(String, ModelSpec)> {
    ALL.iter()
        .flat_map(|&t| [(format!("{t:?} at D"), t.spec(CTX)), (format!("{t:?} four times as wide"), t.wide(CTX))])
        .filter(|(_, s)| author::check(s).is_ok())
        .collect()
}

/// The `rows × cols` of every weight the forward program declares: `Let W be a R by C tensor of Real.`
fn declared_shapes(source: &str) -> BTreeMap<String, (usize, usize)> {
    source
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("Let ")?.strip_suffix(" tensor of Real.")?;
            let (var, shape) = rest.split_once(" be a ")?;
            let (r, c) = shape.split_once(" by ")?;
            Some((var.to_string(), (r.parse().ok()?, c.parse().ok()?)))
        })
        .collect()
}

/// The weights `spec`'s forward program declares and reads from its checkpoint (`(var, binding, read Q8_0)`), and the
/// shape it declares each.
fn weights_of(spec: &ModelSpec) -> (Vec<(String, Binding, bool)>, BTreeMap<String, (usize, usize)>) {
    let ((source, f32w, q8w), _) = written(spec, Cut::Forward);
    let shapes = declared_shapes(&source);
    let mut weights = Vec::new();
    for (var, q8) in f32w.keys().map(|v| (v, false)).chain(q8w.keys().map(|v| (v, true))) {
        // The rotary tables are computed from the spec, not read.
        if RotaryTable::named(var).is_some() {
            continue;
        }
        if let Source::Weight(b) = bind(spec, var).unwrap_or_else(|e| panic!("`{var}` is unbound: {e:?}")) {
            weights.push((var.clone(), b, q8));
        }
    }
    weights.sort_by(|a, b| a.0.cmp(&b.0));
    (weights, shapes)
}

/// Whether a view lays a checkpoint row's elements down a declared column: the checkpoint's blocks then run down the
/// declared rows.
fn transposes(view: &View) -> bool {
    match view {
        View::Same => false,
        View::Transposed | View::HeadBlocks { .. } => true,
        View::NeoxPairs { transpose, .. } | View::RowRange { transpose, .. } | View::HeadPart { transpose, .. } => *transpose,
        View::FusedPart { bias, .. } => !bias,
    }
}

#[test]
fn every_view_reads_the_shape_the_author_declares() {
    let mut checked = 0;
    for (what, spec) in models() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.gguf");
        write_checkpoint(&spec, &path);
        let mut gguf = Gguf::open(&path).unwrap();
        let (weights, shapes) = weights_of(&spec);
        for (var, b, _) in &weights {
            let (_, dims) = gguf.describe(&b.tensor).unwrap_or_else(|e| panic!("{what}: `{}`: {e}", b.tensor));
            let inner = dims[0] as usize;
            let rows = dims.iter().product::<u64>() as usize / inner;
            let (r, c) = b.view.declared(inner, rows);
            assert_eq!(Some(&(r, c)), shapes.get(var), "{what}: `{var}` ({:?} of `{}`) reads {r} × {c}", b.view, b.tensor);
            assert_eq!(values(b, &mut gguf).unwrap().len(), r * c, "{what}: `{var}`: `values` reads {r} × {c}");
            checked += 1;
        }
    }
    assert!(checked > 100, "{checked} weights checked");
}

/// A source of one tensor of `rows` rows of `inner` elements, stored as given.
struct OneTensor {
    ty: GgmlType,
    inner: usize,
    rows: usize,
    bytes: Vec<u8>,
}

impl WeightSource for OneTensor {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        assert_eq!(name, "t");
        Ok((self.ty, vec![self.inner as u64, self.rows as u64]))
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        assert_eq!(name, "t");
        let row = self.bytes.len() / self.rows;
        Ok(self.bytes[first * row..(first + count) * row].to_vec())
    }
    fn has(&self, name: &str) -> bool {
        name == "t"
    }
}

/// The IEEE half with the given bits' value.
fn half(bits: u16) -> f64 {
    f64::from(logicaffeine_compile::model::quant::f16_to_f32(bits))
}

#[test]
fn the_planes_are_the_checkpoints_bytes() {
    // Three checkpoint rows of 64 (two Q8_0 blocks each), block `(r, j)` scaled by its own half and holding
    // `q = (7r + 3j + i) mod 255 − 127`, read transposed: a 64 × 3 weight.
    let (inner, rows) = (64, 3);
    let scale_bits = |r: usize, j: usize| 0x3000u16 + (r * 2 + j) as u16 * 0x0111;
    let q = |r: usize, j: usize, i: usize| ((7 * r + 3 * j + i) % 255) as i64 - 127;
    let mut bytes = Vec::new();
    for r in 0..rows {
        for j in 0..inner / 32 {
            bytes.extend_from_slice(&scale_bits(r, j).to_le_bytes());
            bytes.extend((0..32).map(|i| q(r, j, i) as i8 as u8));
        }
    }
    let mut src = OneTensor { ty: GgmlType::Q8_0, inner, rows, bytes };
    let planes = q8_planes(&Binding { tensor: "t".into(), view: View::Transposed }, &mut src).expect("the planes");
    assert_eq!((planes.rows, planes.cols), (inner, rows), "the declared layout is K × N");
    for k in 0..inner {
        for n in 0..rows {
            assert_eq!(planes.values[k * rows + n], q(n, k / 32, k % 32) as f64, "values[{k}][{n}]");
        }
    }
    for b in 0..inner / 32 {
        for n in 0..rows {
            assert_eq!(planes.scales[b * rows + n], half(scale_bits(n, b)), "scales[{b}][{n}]");
        }
    }
    assert_eq!(planes.values.len(), inner * rows);
    assert_eq!(planes.scales.len(), inner / 32 * rows);

    // Rows 1..3 of the same tensor, transposed: column 0 is checkpoint row 1.
    let planes = q8_planes(&Binding { tensor: "t".into(), view: View::RowRange { first: 1, count: 2, transpose: true } }, &mut src)
        .expect("a row range's planes");
    assert_eq!((planes.rows, planes.cols), (inner, 2));
    for b in 0..inner / 32 {
        for n in 0..2 {
            assert_eq!(planes.scales[b * 2 + n], half(scale_bits(n + 1, b)), "a row range's scales[{b}][{n}]");
        }
    }
    assert_eq!(planes.values[2 * 33], q(1, 1, 1) as f64, "a row range's values[33][0]");
}

#[test]
fn the_planes_are_the_weight_for_every_tiny_model() {
    for (what, spec) in models() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.gguf");
        write_checkpoint(&spec, &path);
        let mut gguf = Gguf::open(&path).unwrap();
        let (weights, _) = weights_of(&spec);
        let mut folded = 0;
        for (var, b, q8) in &weights {
            let stored = gguf.describe(&b.tensor).unwrap().0;
            if !*q8 || stored != GgmlType::Q8_0 {
                continue;
            }
            if !transposes(&b.view) {
                // Read along its stored rows, a weight's blocks run along its declared columns.
                assert!(q8_planes(b, &mut gguf).is_err(), "{what}: `{var}` ({:?} of `{}`) has no planes", b.view, b.tensor);
                continue;
            }
            let p = q8_planes(b, &mut gguf).unwrap_or_else(|e| panic!("{what}: `{var}` ({:?} of `{}`): {e}", b.view, b.tensor));
            let w = values(b, &mut gguf).unwrap();
            assert_eq!((p.values.len(), p.scales.len()), (p.rows * p.cols, p.rows / 32 * p.cols), "{what}: `{var}`'s planes");
            assert_eq!(p.rows * p.cols, w.len(), "{what}: `{var}`'s planes cover the weight");
            for k in 0..p.rows {
                for n in 0..p.cols {
                    let (q, s) = (p.values[k * p.cols + n], p.scales[(k / 32) * p.cols + n]);
                    assert!(q.fract() == 0.0 && (-128.0..=127.0).contains(&q), "{what}: `{var}`[{k}][{n}] = {q} is not an int8");
                    assert_eq!(q * s, w[k * p.cols + n], "{what}: `{var}`[{k}][{n}]: {q} · {s}");
                }
            }
            folded += 1;
        }
        assert!(folded > 0, "{what}: some weight is read Q8_0 down its rows");
    }
}

#[test]
fn a_weight_stored_in_another_format_has_no_planes() {
    let spec = Tiny::Llama.spec(CTX);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.gguf");
    write_checkpoint_as(&spec, &path, &|_| GgmlType::F32);
    let mut gguf = Gguf::open(&path).unwrap();
    let Source::Weight(b) = bind(&spec, "Wq0").unwrap() else { panic!("`Wq0` is a weight") };
    let why = q8_planes(&b, &mut gguf).expect_err("an f32 tensor has no Q8_0 planes").to_string();
    for piece in [b.tensor.as_str(), "f32", "Q8_0"] {
        assert!(why.contains(piece), "the refusal names `{piece}`: {why}");
    }
}

#[test]
fn a_weight_whose_blocks_run_along_its_columns_has_no_planes() {
    // 3 rows of 64 read as stored: the declared 3 × 64 has no whole block of 32 rows.
    let mut src = OneTensor { ty: GgmlType::Q8_0, inner: 64, rows: 3, bytes: vec![0; 3 * 2 * 34] };
    let why = q8_planes(&Binding { tensor: "t".into(), view: View::Same }, &mut src).expect_err("3 rows").to_string();
    assert!(why.contains("`t`") && why.contains("3 rows") && why.contains("32"), "the refusal names the rows: {why}");
    // 64 rows of 64 read as stored: whole blocks of rows, but each block of 32 declared rows comes from 32 checkpoint
    // blocks.
    let mut src = OneTensor { ty: GgmlType::Q8_0, inner: 64, rows: 64, bytes: vec![0; 64 * 2 * 34] };
    let why = q8_planes(&Binding { tensor: "t".into(), view: View::Same }, &mut src).expect_err("blocks along columns").to_string();
    assert!(why.contains("`t`") && why.contains("one Q8_0 block"), "the refusal names the split block: {why}");
}

#[test]
fn every_rotary_table_turns_each_position_by_its_frequency() {
    let mut specs = models();
    specs.push(("Llama with its checkpoint's frequency factors".to_string(), with_rope_factors(Tiny::Llama.spec(CTX))));
    let (mut tables, mut factored) = (0, 0);
    for (what, spec) in specs {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.gguf");
        write_checkpoint(&spec, &path);
        let mut gguf = Gguf::open(&path).unwrap();
        let factors: Option<Vec<f64>> = gguf
            .has("rope_freqs.weight")
            .then(|| gguf.float_tensor("rope_freqs.weight").unwrap().into_iter().map(f64::from).collect());
        let ((source, f32w, _), _) = written(&spec, Cut::Decode { ctx: CTX });
        let shapes = declared_shapes(&source);
        for var in f32w.keys() {
            let Some(table) = RotaryTable::named(var) else {
                assert!(rotary_values(&spec, var, CTX, &mut gguf).unwrap().is_none(), "{what}: `{var}` is no rotary table");
                continue;
            };
            let got = rotary_values(&spec, var, CTX, &mut gguf).unwrap().unwrap_or_else(|| panic!("{what}: `{var}` is a table"));
            let (rows, cols) = shapes[var.as_str()];
            assert_eq!(got.len(), rows * cols, "{what}: `{var}` fills its declared {rows} × {cols}");
            let rope = rotations(&spec)[table.rotation];
            assert_eq!((rows, cols), (if table.cached { CTX } else { 1 }, rope.dims / 2), "{what}: `{var}`'s shape");
            for p in 0..rows {
                for i in 0..cols {
                    let mut freq = rope.theta.powf(-2.0 * i as f64 / rope.dims as f64);
                    if rope.scaling == RopeScaling::Factors {
                        freq /= factors.as_ref().expect("the checkpoint's factors")[i];
                        factored += 1;
                    }
                    // Plain rotations of text positions: the angle is the position times the frequency.
                    if !matches!(rope.scaling, RopeScaling::None | RopeScaling::Factors) || rope.sections.is_some() {
                        let (c, s) = rope.rotation(i, p, factors.as_ref().map(|f| f[i]));
                        assert_eq!(got[p * cols + i], if table.cos { c } else { s }, "{what}: `{var}`[{p}][{i}]");
                        continue;
                    }
                    let angle = p as f64 * freq;
                    let want = if table.cos { angle.cos() } else { angle.sin() };
                    assert!((got[p * cols + i] - want).abs() < 1e-12, "{what}: `{var}`[{p}][{i}] = {} for {want}", got[p * cols + i]);
                }
            }
            tables += 1;
        }
    }
    assert!(tables > 10, "{tables} rotary tables checked");
    assert!(factored > 0, "a rotation that reads the checkpoint's factors is among them");
}

#[test]
fn a_forwards_rotary_table_is_position_zeros() {
    let spec = Tiny::Qwen3.spec(CTX);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.gguf");
    write_checkpoint(&spec, &path);
    let mut gguf = Gguf::open(&path).unwrap();
    let half = rotations(&spec)[0].dims / 2;
    assert_eq!(rotary_values(&spec, "RC", CTX, &mut gguf).unwrap(), Some(vec![1.0; half]), "cos 0 = 1 for every frequency");
    assert_eq!(rotary_values(&spec, "RS", CTX, &mut gguf).unwrap(), Some(vec![0.0; half]), "sin 0 = 0 for every frequency");
}

#[test]
fn a_rotation_the_model_does_not_have_is_refused() {
    let spec = Tiny::Qwen3.spec(CTX);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.gguf");
    write_checkpoint(&spec, &path);
    let mut gguf = Gguf::open(&path).unwrap();
    assert_eq!(rotations(&spec).len(), 1, "Qwen3 turns by one rotation");
    let why = rotary_values(&spec, "Rcos1", CTX, &mut gguf).expect_err("no second rotation").to_string();
    assert!(why.contains("Rcos1") && why.contains("rotation 1"), "the refusal names the table and the rotation: {why}");
}

#[test]
fn a_weight_is_stored_as_planes_exactly_when_its_planes_repack() {
    // What the emitter reads from a checkpoint's description alone is what the host can repack from its bytes.
    let mut checked = 0;
    for (what, spec) in models() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.gguf");
        write_checkpoint(&spec, &path);
        let mut gguf = Gguf::open(&path).unwrap();
        for (var, b, _) in weights_of(&spec).0 {
            let described = stored_as_q8_planes(&b, &gguf).unwrap();
            assert_eq!(described, q8_planes(&b, &mut gguf).is_ok(), "{what}: `{var}` ({:?} of `{}`)", b.view, b.tensor);
            checked += usize::from(described);
        }
    }
    assert!(checked > 50, "{checked} weights stored as planes");
}
