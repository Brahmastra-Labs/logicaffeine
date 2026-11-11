//! A checkpoint on the CPU: every form of its one program the CUDA lane compiles (`lane::compile`) — the decode step and
//! the prefill chunks — lifted into the one IR with its constants' storage, each linked under a CPU shim into a step
//! function of one program over shared buffers (a tensor of one name is one buffer, its largest form's length: the
//! checkpoint's rows, the caches and every state the forms carry between them), and a driver that maps the checkpoint
//! and reads its rows in place, takes the compiler's values from a sidecar, prefills a prompt in the largest chunks that
//! fit and the rest a token at a time, and decodes greedily, timing each.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use super::kernels::LIBRARY_X86_AVX2_VNNI;
use super::link::{link, BufKind, CpuProgram};
use super::shim::CpuShim;
use crate::codegen_cuda::knobs::CudaKnobs;
use crate::codegen_cuda::lane::plan::Fill;
use crate::codegen_cuda::lane::weights::Placement;
use crate::codegen_cuda::lane::{compile, decode_io, prefill_io, Form, FormKind, LaneOptions};
use crate::model::spec::ModelSpec;
use crate::tensor_graph::graph::{SeqIo, StepIo, Storage, TensorGraph};

/// A constant of checkpoint rows: its tensor's name, and the byte runs of the checkpoint its rows are, in order.
#[derive(Clone, Debug)]
pub struct RowsConstant {
    pub name: String,
    pub runs: Vec<(u64, usize)>,
}

/// A form linked for the CPU: its rows (1 for the decode step), its program and its names across steps.
#[derive(Clone, Debug)]
pub struct CpuForm {
    pub rows: usize,
    pub program: CpuProgram,
    pub io: StepIo,
}

/// A model linked for the CPU: its forms (the decode step first, then the prefill chunks, largest first), the checkpoint
/// rows they read in place and the values the compiler computed for them.
#[derive(Clone, Debug)]
pub struct CpuModel {
    pub forms: Vec<CpuForm>,
    pub rows: Vec<RowsConstant>,
    pub values: Vec<(String, Vec<f32>)>,
}

/// A lane form's cut as the one IR lifts it: each constant's storage from the form's buffers.
pub fn lifted(form: &Form) -> Result<TensorGraph, String> {
    let placed: BTreeMap<String, Storage> = form
        .schedule
        .buffers
        .iter()
        .filter_map(|(n, b)| match &b.fill {
            Fill::Constant(Placement::Rows(r)) => Some((n.clone(), Storage::Rows { format: r.ty, transposed: r.transposed })),
            Fill::Constant(Placement::Values(_)) => Some((n.clone(), Storage::Values)),
            _ => None,
        })
        .collect();
    let io = match form.kind {
        FormKind::Decode => decode_io(),
        FormKind::Prefill => prefill_io(),
        FormKind::Batch => return Err("a batch form runs no prompt of its own".into()),
    };
    let io = StepIo {
        token: io.token,
        position: io.position,
        logits: io.logits,
        next: io.next,
        more: io.more.into_iter().map(|q| SeqIo { position: q.position, logits: q.logits, next: q.next }).collect(),
        logits_read: io.logits_read,
    };
    TensorGraph::lift_written(&form.source, Some(form.identity.clone()), Some(io), &|n| placed.get(n).cloned().unwrap_or(Storage::Values))
        .map_err(|e| e.to_string())
}

/// `spec`'s decode step and prefill chunks of up to `prefill` rows over caches of `ctx` positions, linked under `shim`.
pub fn compile_model(spec: &ModelSpec, checkpoint: &Path, ctx: usize, prefill: usize, shim: &CpuShim) -> Result<CpuModel, String> {
    let lane = compile(spec, checkpoint, &LaneOptions { ctx, knobs: CudaKnobs::default(), prefill }).map_err(|e| e.to_string())?;
    let (mut forms, mut rows, mut values): (Vec<CpuForm>, Vec<RowsConstant>, Vec<(String, Vec<f32>)>) = (Vec::new(), Vec::new(), Vec::new());
    for form in lane.forms.iter().filter(|f| f.kind != FormKind::Batch) {
        let g = lifted(form)?;
        let program = link(&g, shim).map_err(|e| format!("the {}-row form: {e}", form.rows))?;
        for (name, b) in &form.schedule.buffers {
            match &b.fill {
                Fill::Constant(Placement::Rows(r)) if !rows.iter().any(|x| &x.name == name) => {
                    let bytes = r.ty.bytes_for(r.cols);
                    rows.push(RowsConstant { name: name.clone(), runs: r.runs.iter().map(|run| (run.offset, run.rows * bytes)).collect() });
                }
                Fill::Constant(Placement::Values(v)) if !values.iter().any(|(n, _)| n == name) => values.push((name.clone(), v.clone())),
                _ => {}
            }
        }
        let io = g.io.clone().ok_or("a form names no io")?;
        forms.push(CpuForm { rows: form.rows, program, io });
    }
    forms.sort_by_key(|f| (f.rows != 1, std::cmp::Reverse(f.rows)));
    if forms.first().map(|f| f.rows) != Some(1) {
        return Err("the lane compiles no decode step".into());
    }
    Ok(CpuModel { forms, rows, values })
}

impl CpuModel {
    /// The sidecar: every value constant's `f32`s, little-endian, in `values`' order.
    pub fn sidecar(&self) -> Vec<u8> {
        self.values.iter().flat_map(|(_, v)| v.iter().flat_map(|x| x.to_le_bytes())).collect()
    }

    /// The program's C++ source: the library, a step function a form over the forms' buffers (one buffer a name, at
    /// the largest of its forms' lengths), and a driver run as `program <checkpoint> <sidecar> <threads> <n_gen>
    /// <token>…` — every checkpoint row read in place from the mapped file (a constant of several runs copied together
    /// once, its runs a table), every value constant from the sidecar, every other buffer zero; the prompt prefilled in
    /// the largest chunks that fit and the rest a token at a time (tokens and positions 1-based, as the steps read them),
    /// then `n_gen` tokens decoded greedily, printing `TOKENS`, `PREFILL_MS`, `DECODE_MS_PER_TOKEN` and
    /// `DECODE_TOK_PER_S`.
    pub fn source(&self) -> String {
        let mut buffers: BTreeMap<String, (BufKind, usize, String)> = BTreeMap::new();
        for f in &self.forms {
            for b in &f.program.buffers {
                let e = buffers.entry(b.name.clone()).or_insert((b.kind.clone(), 0, b.ident.clone()));
                e.1 = e.1.max(b.len);
            }
        }
        let mut s = String::from(LIBRARY_X86_AVX2_VNNI);
        s.push_str("\n#include <chrono>\n#include <cstdio>\n#include <cstdlib>\n#include <fcntl.h>\n#include <sys/mman.h>\n#include <sys/stat.h>\n#include <unistd.h>\n");
        for (kind, _, ident) in buffers.values() {
            let ty = match kind {
                BufKind::F32 => "float*",
                BufKind::F16 => "lk_h16buf",
                BufKind::Int8Rows { .. } => "lk_i8rows",
                BufKind::I64 => "int64_t*",
                BufKind::Rows { .. } => "const uint8_t*",
            };
            let _ = writeln!(s, "static {ty} {ident};");
        }
        s.push_str("#define LK_AFTER(at)\n");
        for f in &self.forms {
            let _ = writeln!(s, "static void lk_step_{}(lk_pool& pool){{", f.rows);
            s.push_str(&f.program.step);
            s.push_str("}\n");
        }
        s.push_str("int main(int argc, char** argv){\n  if(argc < 5){ std::fprintf(stderr, \"usage: program <checkpoint> <sidecar> <threads> <n_gen> <token>...\\n\"); return 2; }\n");
        s.push_str("  int fd = open(argv[1], O_RDONLY); if(fd < 0){ std::perror(argv[1]); return 1; }\n  struct stat st; fstat(fd, &st);\n");
        s.push_str("  const uint8_t* ck = (const uint8_t*)mmap(nullptr, st.st_size, PROT_READ, MAP_PRIVATE, fd, 0); if(ck == MAP_FAILED){ std::perror(\"mmap\"); return 1; }\n");
        s.push_str("  madvise((void*)ck, st.st_size, MADV_WILLNEED);\n");
        s.push_str("  FILE* sf = std::fopen(argv[2], \"rb\"); if(!sf){ std::perror(argv[2]); return 1; }\n");
        let rows: BTreeMap<&str, &RowsConstant> = self.rows.iter().map(|r| (r.name.as_str(), r)).collect();
        let values: BTreeMap<&str, usize> = self.values.iter().map(|(n, v)| (n.as_str(), v.len())).collect();
        for (name, len) in self.values.iter().map(|(n, v)| (n, v.len())) {
            let Some((_, _, id)) = buffers.get(name) else { continue };
            let _ = writeln!(s, "  {id} = (float*)std::malloc(sizeof(float) * {len}); if(std::fread({id}, sizeof(float), {len}, sf) != {len}) return 3;");
        }
        for (name, (kind, len, id)) in &buffers {
            match kind {
                BufKind::Rows { .. } => {
                    let Some(r) = rows.get(name.as_str()) else {
                        let _ = writeln!(s, "  std::fprintf(stderr, \"no checkpoint rows for {name}\\n\"); return 4;");
                        continue;
                    };
                    if let [(off, _)] = r.runs.as_slice() {
                        let _ = writeln!(s, "  {id} = ck + {off}L;");
                    } else {
                        // The runs as a table the copy walks: a constant of rows picked one by one is thousands of runs.
                        let total: usize = r.runs.iter().map(|(_, n)| n).sum();
                        let table: Vec<String> = r.runs.iter().map(|(off, n)| format!("{{{off}L,{n}L}}")).collect();
                        let _ = writeln!(
                            s,
                            "  {{ static const long runs[][2] = {{{}}}; uint8_t* c = (uint8_t*)std::malloc({total}); long at = 0; for(const auto& r : runs){{ std::memcpy(c + at, ck + r[0], r[1]); at += r[1]; }} {id} = c; }}",
                            table.join(",")
                        );
                    }
                }
                BufKind::F32 if values.contains_key(name.as_str()) => {}
                BufKind::F32 => {
                    let _ = writeln!(s, "  {id} = (float*)std::calloc({}, sizeof(float));", len.max(&1));
                }
                BufKind::I64 => {
                    let _ = writeln!(s, "  {id} = (int64_t*)std::calloc({}, sizeof(int64_t));", len.max(&1));
                }
                BufKind::F16 => {
                    let _ = writeln!(s, "  {id}.p = (uint16_t*)std::calloc({}, sizeof(uint16_t));", len.max(&1));
                }
                BufKind::Int8Rows { unit, .. } => {
                    let _ = writeln!(s, "  {id}.q = (int8_t*)std::calloc({}, 1); {id}.s = (uint16_t*)std::calloc({}, sizeof(uint16_t)); {id}.unit = {unit};", len.max(&1), (len / unit).max(1));
                }
            }
        }
        let id = |n: &str| buffers.get(n).map_or_else(|| "nullptr".to_string(), |b| b.2.clone());
        let decode = &self.forms[0];
        let (tok, pos, next) = (id(&decode.io.token), id(&decode.io.position), id(&decode.io.next));
        let _ = write!(
            s,
            "  lk_pool pool(std::atoi(argv[3]));\n  const int n_gen = std::atoi(argv[4]);\n  std::vector<int64_t> prompt; for(int i = 5; i < argc; ++i) prompt.push_back(std::atoll(argv[i]));\n\
             \x20 using clock = std::chrono::steady_clock;\n  long at = 0; size_t p = 0; int64_t t = 0;\n  auto t0 = clock::now();\n"
        );
        for f in self.forms.iter().skip(1) {
            let (toks, cpos, cnext) = (id(&f.io.token), id(&f.io.position), id(&f.io.next));
            let _ = writeln!(
                s,
                "  while(prompt.size() - p >= {r}){{ for(int i = 0; i < {r}; ++i) {toks}[i] = prompt[p + i] + 1; {cpos}[0] = at + 1; lk_step_{r}(pool); at += {r}; p += {r}; t = {cnext}[0] - 1; }}",
                r = f.rows
            );
        }
        let _ = write!(
            s,
            "  for(; p < prompt.size(); ++p){{ {tok}[0] = prompt[p] + 1; {pos}[0] = ++at; lk_step_1(pool); t = {next}[0] - 1; }}\n\
             \x20 auto t1 = clock::now();\n  std::vector<int64_t> out;\n\
             \x20 for(int i = 0; i < n_gen; ++i){{ out.push_back(t); {tok}[0] = t + 1; {pos}[0] = ++at; lk_step_1(pool); t = {next}[0] - 1; }}\n\
             \x20 auto t2 = clock::now();\n\
             \x20 std::printf(\"TOKENS:\"); for(auto x : out) std::printf(\" %lld\", (long long)x); std::printf(\"\\n\");\n\
             \x20 const double pm = std::chrono::duration<double, std::milli>(t1 - t0).count(), dm = std::chrono::duration<double, std::milli>(t2 - t1).count();\n\
             \x20 std::printf(\"PREFILL_MS: %.3f\\nDECODE_MS_PER_TOKEN: %.4f\\nDECODE_TOK_PER_S: %.2f\\n\", pm, n_gen ? dm / n_gen : 0.0, n_gen ? n_gen * 1000.0 / dm : 0.0);\n\
             \x20 return 0;\n}}\n"
        );
        s
    }
}
