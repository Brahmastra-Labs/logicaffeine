//! A model cut between the host's cores and a device (PARITY-HOST-LAYERS-ON-CPU): its decode step's first layers — the
//! token's embedding and every layer before the cut — on the CPU tier, the rest with the head and the arg-max on the
//! CUDA lane, the hidden state at the cut crossing the link twice a token. llama.cpp runs a model past its card the same
//! way round: its CPU the first layers, the card the last and the head (`i_gpu_start = n_layer + 1 − n_gpu_layers`,
//! `src/llama-model.cpp`), so a token reads the layers past the card from the host's memory, not over the link.
//!
//! Both halves are cuts of the one program (`Cut::DecodeSpan`): the front is the CPU tier's link of the span `[0, from)`
//! over the checkpoint's rows read in place, the back the lane's compile of `[from, L)` (`lane::compile_from`), only its
//! matrices on the device. They make one program: the lane's, linked with the front's library, which the lane's driver
//! calls a step at a time (`lk_front`) and whose values follow the lane's in the sidecar.

use std::fmt::Write as _;
use std::path::Path;

use super::kernels::LIBRARY_X86_AVX2_VNNI;
use super::link::{link, BufKind};
use super::model::{buffers_of, declarations, named, placed_graph, placements, CpuForm, RowsConstant};
use super::shim::CpuShim;
use crate::codegen_cuda::lane::weights::Placement;
use crate::codegen_cuda::lane::{compile_from, span_io, Front, Lane, LaneOptions};
use crate::model::gguf::Gguf;
use crate::model::program::Cut;
use crate::model::spec::ModelSpec;

/// A model cut at layer `from`: the device's half (`lane`, its program calling the front) and the host's (`front`, the
/// span's link, the checkpoint rows it reads in place, the values the compiler computed for it and the integer tables
/// the program sets itself).
#[derive(Clone, Debug)]
pub struct Split {
    pub from: usize,
    pub lane: Lane,
    pub front: CpuForm,
    pub rows: Vec<RowsConstant>,
    pub values: Vec<(String, Vec<f32>)>,
    pub ints: Vec<(String, Vec<i64>)>,
}

/// `spec`'s decode step cut at layer `from`, the layers before it linked under `shim` for the host's cores and the rest
/// compiled for the device under `opts`, against the checkpoint at `checkpoint`.
pub fn compile_cut(spec: &ModelSpec, checkpoint: &Path, opts: &LaneOptions, from: usize, shim: &CpuShim) -> Result<Split, String> {
    if spec.ple.as_ref().is_some_and(|p| p.layer >= from) {
        return Err(format!("a per-layer embedding on layer {} of the device's layers from {from}: its hash reads the token, which only the host's layers take", spec.ple.as_ref().map_or(0, |p| p.layer)));
    }
    if from == 0 || from >= spec.layers.len() {
        return Err(format!("the layers from {from} of {}: the host's span is the first layers, the device's the rest, each one or more", spec.layers.len()));
    }
    let io = span_io(spec, 0, from);
    let mut g = Gguf::open(checkpoint).map_err(|e| e.to_string())?;
    let (graph, placed) = placed_graph(spec, &mut g, Cut::DecodeSpan { from: 0, to: from, ctx: opts.ctx }, io.clone(), opts.ctx)?;
    let program = link(&graph, shim).map_err(|e| format!("the host's layers: {e}"))?;
    let identity = graph.identity.ok_or("the host's span names no cut of the program")?;
    let (mut rows, mut values, mut ints) = (Vec::new(), Vec::new(), Vec::new());
    for (name, p) in &placed {
        match p {
            Placement::Rows(r) => {
                let bytes = r.ty.bytes_for(r.cols);
                rows.push(RowsConstant { name: name.clone(), runs: r.runs.iter().map(|run| (run.offset, run.rows * bytes)).collect() });
            }
            Placement::Values(v) => values.push((name.clone(), v.clone())),
            Placement::Ints(v) => ints.push((name.clone(), v.clone())),
        }
    }
    let front_sidecar = 4 * values.iter().map(|(_, v)| v.len()).sum::<usize>();
    let lane = compile_from(spec, checkpoint, opts, from, Front { sidecar: front_sidecar }).map_err(|e| format!("the device's layers: {e}"))?;
    Ok(Split { from, lane, front: CpuForm { rows: 1, identity, program, io }, rows, values, ints })
}

impl Split {
    /// The program's sidecar: the lane's values, then the front's, each `f32` little-endian.
    pub fn sidecar(&self) -> Vec<u8> {
        let mut s = self.lane.program.sidecar.clone();
        s.extend(self.values.iter().flat_map(|(_, v)| v.iter().flat_map(|x| x.to_le_bytes())));
        s
    }

    /// The front's C++ source: the cut of the one program it links, the library, its buffers and its step, and the three
    /// functions the lane's program calls — `lk_front_init(file, side, threads)` pointing every buffer at its storage
    /// (its rows in the checkpoint the program maps, its values from `side`, every other zero) and starting its threads;
    /// `lk_front_reset()` zeroing every buffer it computes or carries; `lk_front(tok, pos, hidden)` its step at token
    /// `tok` and position `pos` (both 1-based, as the step reads them) and its hidden state into `hidden`.
    pub fn front_source(&self) -> String {
        let forms = std::slice::from_ref(&self.front);
        let buffers = buffers_of(forms);
        let id = |n: &str| buffers.get(n).map_or_else(|| "nullptr".to_string(), |b| b.2.clone());
        let io = &self.front.io;
        let hidden = io.hidden_out.as_deref().expect("the front ends at the cut");
        let width = buffers.get(hidden).map_or(0, |b| b.1);
        let mut s = named(forms);
        s.push_str(LIBRARY_X86_AVX2_VNNI);
        s.push('\n');
        s.push_str(&declarations(&buffers));
        s.push_str("#define LK_AFTER(at)\nstatic void lk_step_front(lk_pool& pool){\n");
        s.push_str(&self.front.program.step);
        s.push_str("}\nstatic lk_pool* lk_front_pool = nullptr;\n");
        s.push_str("extern \"C\" void lk_front_init(const unsigned char* file, const unsigned char* side, int threads){\n  const uint8_t* ck = file;\n");
        s.push_str(&placements(
            &buffers,
            &self.rows,
            &self.values,
            &self.ints,
            &|ident, len, offset| format!("  {ident} = (float*)std::malloc(sizeof(float) * {len}); std::memcpy({ident}, side + {offset}L, sizeof(float) * {len});\n"),
            "std::exit(4);",
        ));
        s.push_str("  lk_front_pool = new lk_pool(threads);\n}\nextern \"C\" void lk_front_reset(){\n");
        let valued: std::collections::BTreeSet<&str> = self.values.iter().map(|(n, _)| n.as_str()).chain(self.ints.iter().map(|(n, _)| n.as_str())).collect();
        for (name, (kind, len, ident)) in &buffers {
            let len = (*len).max(1);
            match kind {
                BufKind::F32 if !valued.contains(name.as_str()) => {
                    let _ = writeln!(s, "  std::memset({ident}, 0, sizeof(float) * {len});");
                }
                BufKind::I64 if !valued.contains(name.as_str()) => {
                    let _ = writeln!(s, "  std::memset({ident}, 0, sizeof(int64_t) * {len});");
                }
                BufKind::F16 => {
                    let _ = writeln!(s, "  std::memset({ident}.p, 0, sizeof(uint16_t) * {len});");
                }
                BufKind::Int8Rows { unit, .. } => {
                    let _ = writeln!(s, "  std::memset({ident}.q, 0, {len}); std::memset({ident}.s, 0, sizeof(uint16_t) * {});", (len / unit).max(1));
                }
                _ => {}
            }
        }
        let _ = write!(
            s,
            "}}\nextern \"C\" void lk_front(int tok, int pos, float* hidden){{\n  {}[0] = tok; {}[0] = pos;\n  lk_step_front(*lk_front_pool);\n  std::memcpy(hidden, {}, sizeof(float) * {width});\n}}\n",
            id(&io.token),
            id(&io.position),
            id(hidden)
        );
        s
    }
}
