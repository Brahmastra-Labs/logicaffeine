//! A checkpoint on the CPU: the forms of its one program the CUDA lane compiles — the decode step and the prefill
//! chunks — each lifted into the one IR with its constants placed from the checkpoint as the program declares them, and
//! linked under a CPU shim into a step function of one program over shared buffers (a tensor of one name is one buffer, its largest form's length: the
//! checkpoint's rows, the caches and every state the forms carry between them), and a driver that maps the checkpoint
//! and reads its rows in place, takes the compiler's values from a sidecar, prefills a prompt in the largest chunks that
//! fit and the rest a token at a time, and decodes greedily, timing each.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use super::kernels::LIBRARY_X86_AVX2_VNNI;
use super::link::{link, BufKind, CpuProgram};
use super::shim::CpuShim;
use crate::codegen_cuda::lane::weights::{self, Placement};
use crate::codegen_cuda::lane::{decode_io, prefill_io};
use crate::model::author;
use crate::model::gguf::Gguf;
use crate::model::program::{written, Cut, Identity};
use crate::model::spec::ModelSpec;
use crate::tensor_graph::graph::{Role, StepIo, Storage, TensorGraph};

/// A constant of checkpoint rows: its tensor's name, and the byte runs of the checkpoint its rows are, in order.
#[derive(Clone, Debug)]
pub struct RowsConstant {
    pub name: String,
    pub runs: Vec<(u64, usize)>,
}

/// A form linked for the CPU: its rows (1 for the decode step), the cut of the model's one program it links, its program
/// and its names across steps.
#[derive(Clone, Debug)]
pub struct CpuForm {
    pub rows: usize,
    pub identity: Identity,
    pub program: CpuProgram,
    pub io: StepIo,
}

/// A model linked for the CPU: its forms (the decode step first, then the prefill chunks, largest first), the checkpoint
/// rows they read in place, the values the compiler computed for them and the integer tables the program sets itself.
#[derive(Clone, Debug)]
pub struct CpuModel {
    pub forms: Vec<CpuForm>,
    pub rows: Vec<RowsConstant>,
    pub values: Vec<(String, Vec<f32>)>,
    pub ints: Vec<(String, Vec<i64>)>,
}

/// A cut of `spec`'s one program as the IR lifts it with `io` its names across steps, each constant placed from the
/// checkpoint `g` (caches of `ctx` positions) as the program declares it: the graph and where each constant lives.
pub fn placed_graph(spec: &ModelSpec, g: &mut Gguf, cut: Cut, io: StepIo, ctx: usize) -> Result<(TensorGraph, BTreeMap<String, Placement>), String> {
    let ((source, _, q8), identity) = written(spec, cut);
    let declared = TensorGraph::lift_written(&source, Some(identity.clone()), Some(io.clone()), &|_| Storage::Values).map_err(|e| e.to_string())?;
    let mut placed: BTreeMap<String, Placement> = BTreeMap::new();
    for t in declared.tensors.values().filter(|t| t.role == Role::Constant) {
        let p = weights::place_declared(spec, g, t, q8.contains_key(&t.name), ctx).map_err(|e| e.to_string())?;
        placed.insert(t.name.clone(), p);
    }
    let storage = |n: &str| match placed.get(n) {
        Some(Placement::Rows(r)) => Storage::Rows { format: r.ty, transposed: r.transposed },
        _ => Storage::Values,
    };
    let graph = TensorGraph::lift_written(&source, Some(identity), Some(io), &storage).map_err(|e| e.to_string())?;
    Ok((graph, placed))
}

/// `spec`'s decode step and its prefill chunks — of `prefill` rows and each power of two below it down to two, the
/// lane's — over caches of `ctx` positions: each a cut of the one program placed from the checkpoint at `checkpoint`
/// ([`placed_graph`]) and linked under `shim`. A constant two forms read is one, placed alike in both.
pub fn compile_model(spec: &ModelSpec, checkpoint: &Path, ctx: usize, prefill: usize, shim: &CpuShim) -> Result<CpuModel, String> {
    author::check_at(spec, ctx).map_err(|e| format!("the language cannot write this model yet: {e}"))?;
    if !prefill.is_power_of_two() || prefill > ctx {
        return Err(format!("a prefill chunk of {prefill} rows over caches of {ctx} positions: its rows a power of two the caches hold"));
    }
    let mut g = Gguf::open(checkpoint).map_err(|e| format!("{}: {e}", checkpoint.display()))?;
    let chunks = std::iter::successors(Some(prefill), |r| Some(r / 2)).take_while(|&r| r > 1).map(|rows| (rows, Cut::Prefill { rows, ctx }, prefill_io()));
    let (mut forms, mut rows, mut values): (Vec<CpuForm>, Vec<RowsConstant>, Vec<(String, Vec<f32>)>) = (Vec::new(), Vec::new(), Vec::new());
    let mut ints: Vec<(String, Vec<i64>)> = Vec::new();
    for (n, cut, io) in std::iter::once((1, Cut::Decode { ctx }, decode_io())).chain(chunks) {
        let (graph, placed) = placed_graph(spec, &mut g, cut, io, ctx)?;
        let program = link(&graph, shim).map_err(|e| format!("the {cut}: {e}"))?;
        for (name, p) in placed {
            match p {
                Placement::Rows(r) => {
                    let bytes = r.ty.bytes_for(r.cols);
                    let runs: Vec<(u64, usize)> = r.runs.iter().map(|run| (run.offset, run.rows * bytes)).collect();
                    match rows.iter().find(|x| x.name == name) {
                        None => rows.push(RowsConstant { name, runs }),
                        Some(x) if x.runs == runs => {}
                        Some(_) => return Err(format!("`{name}`: the {cut} reads other rows of the checkpoint than a form before it")),
                    }
                }
                Placement::Values(v) => match values.iter().find(|(x, _)| *x == name) {
                    None => values.push((name, v)),
                    Some((_, w)) if *w == v => {}
                    Some(_) => return Err(format!("`{name}`: the {cut} computes other values than a form before it")),
                },
                Placement::Ints(v) => match ints.iter().find(|(x, _)| *x == name) {
                    None => ints.push((name, v)),
                    Some((_, w)) if *w == v => {}
                    Some(_) => return Err(format!("`{name}`: the {cut} sets other integers than a form before it")),
                },
            }
        }
        let identity = graph.identity.ok_or_else(|| format!("the {cut} names no cut of the program"))?;
        let io = graph.io.clone().ok_or_else(|| format!("the {cut} names no names across steps"))?;
        forms.push(CpuForm { rows: n, identity, program, io });
    }
    Ok(CpuModel { forms, rows, values, ints })
}

impl CpuModel {
    /// The sidecar: every value constant's `f32`s, little-endian, in `values`' order.
    pub fn sidecar(&self) -> Vec<u8> {
        self.values.iter().flat_map(|(_, v)| v.iter().flat_map(|x| x.to_le_bytes())).collect()
    }

    /// The program's C++ source: the cut of the one program each form links, a line each; the library; a step function
    /// a form over the forms' buffers (one buffer a name, at the largest of its forms' lengths); and a driver run as
    /// `program <checkpoint> <sidecar> <threads> <n_gen> <token>…` — every checkpoint row read in place from the mapped
    /// file (a constant of several runs copied together once, its runs a table), every value constant from the sidecar,
    /// every other buffer zero; the prompt prefilled in the largest chunks that fit and the rest a token at a time
    /// (tokens and positions 1-based, as the steps read them), then `n_gen` tokens decoded greedily, printing `TOKENS`,
    /// `PREFILL_MS`, `DECODE_MS_PER_TOKEN` and `DECODE_TOK_PER_S`.
    pub fn source(&self) -> String {
        let buffers = buffers_of(&self.forms);
        let mut s = named(&self.forms);
        s.push_str(LIBRARY_X86_AVX2_VNNI);
        s.push_str("\n#include <chrono>\n#include <cstdio>\n#include <cstdlib>\n#include <fcntl.h>\n#include <sys/mman.h>\n#include <sys/stat.h>\n#include <unistd.h>\n");
        s.push_str(&declarations(&buffers));
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
        s.push_str(&placements(&buffers, &self.rows, &self.values, &self.ints, &|id, len, _| {
            format!("  {id} = (float*)std::malloc(sizeof(float) * {len}); if(std::fread({id}, sizeof(float), {len}, sf) != {len}) return 3;\n")
        }, "return 4;"));
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

/// The threads a CPU program runs on this host, and whether they are a hybrid CPU's performance cores': on a CPU of
/// performance and efficiency cores (Linux lists the performance cores' CPUs in `/sys/devices/cpu_core/cpus`) the
/// performance cores' — a phase waits for its slowest chunk, so a thread on an efficiency core holds back every other —
/// else every thread the host runs at once.
pub fn host_threads() -> (usize, bool) {
    match std::fs::read_to_string("/sys/devices/cpu_core/cpus").ok().and_then(|l| cpus_in(l.trim())) {
        Some(n) if n > 0 => (n, true),
        _ => (std::thread::available_parallelism().map_or(1, |n| n.get()), false),
    }
}

/// How many CPUs a Linux CPU list names (`0-15,32,34-35` names 19), or `None` for text that is not one.
fn cpus_in(list: &str) -> Option<usize> {
    list.split(',')
        .map(|r| match r.split_once('-') {
            Some((a, b)) => b.parse::<usize>().ok()?.checked_sub(a.parse::<usize>().ok()?).map(|n| n + 1),
            None => r.parse::<usize>().ok().map(|_| 1),
        })
        .sum()
}

/// The cut of the one program each of `forms` links, a comment line each (`// LOGOS program: <cut>, sha256 <hex>`).
pub(crate) fn named(forms: &[CpuForm]) -> String {
    forms.iter().map(|f| format!("// {}\n", f.identity.line())).collect()
}

/// Every buffer of `forms`: one a name, at the largest of its forms' lengths — its kind, its length and its C name.
pub(crate) fn buffers_of(forms: &[CpuForm]) -> BTreeMap<String, (BufKind, usize, String)> {
    let mut buffers: BTreeMap<String, (BufKind, usize, String)> = BTreeMap::new();
    for f in forms {
        for b in &f.program.buffers {
            let e = buffers.entry(b.name.clone()).or_insert((b.kind.clone(), 0, b.ident.clone()));
            e.1 = e.1.max(b.len);
        }
    }
    buffers
}

/// Each buffer's declaration, a global of its kind's type.
pub(crate) fn declarations(buffers: &BTreeMap<String, (BufKind, usize, String)>) -> String {
    let mut s = String::new();
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
    s
}

/// The statements that point each buffer at its storage, a checkpoint mapped at `ck`: a constant's rows read in place (a
/// constant of several runs copied together once, its runs a table; `missing` where the checkpoint holds none), each
/// value constant's `f32`s as `value(id, len, offset)` reads them — `offset` its first byte in `values`' order — each
/// integer table the program sets itself from its values, written into the program, and every other buffer zero.
pub(crate) fn placements(
    buffers: &BTreeMap<String, (BufKind, usize, String)>,
    rows: &[RowsConstant],
    values: &[(String, Vec<f32>)],
    ints: &[(String, Vec<i64>)],
    value: &dyn Fn(&str, usize, usize) -> String,
    missing: &str,
) -> String {
    let mut s = String::new();
    let by_name: BTreeMap<&str, &RowsConstant> = rows.iter().map(|r| (r.name.as_str(), r)).collect();
    let valued: BTreeMap<&str, usize> = values.iter().map(|(n, v)| (n.as_str(), v.len())).collect();
    let mut offset = 0;
    for (name, len) in values.iter().map(|(n, v)| (n, v.len())) {
        if let Some((_, _, id)) = buffers.get(name) {
            s.push_str(&value(id, len, offset));
        }
        offset += 4 * len;
    }
    for (name, (kind, len, id)) in buffers {
        match kind {
            BufKind::Rows { .. } => {
                let Some(r) = by_name.get(name.as_str()) else {
                    let _ = writeln!(s, "  std::fprintf(stderr, \"no checkpoint rows for {name}\\n\"); {missing}");
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
            BufKind::F32 if valued.contains_key(name.as_str()) => {}
            BufKind::F32 => {
                let _ = writeln!(s, "  {id} = (float*)std::calloc({}, sizeof(float));", len.max(&1));
            }
            BufKind::I64 => match ints.iter().find(|(n, _)| n == name) {
                Some((_, v)) => {
                    let list: Vec<String> = v.iter().map(|x| format!("{x}LL")).collect();
                    let _ = writeln!(s, "  {{ static int64_t set[] = {{{}}}; {id} = set; }}", list.join(", "));
                }
                None => {
                    let _ = writeln!(s, "  {id} = (int64_t*)std::calloc({}, sizeof(int64_t));", len.max(&1));
                }
            },
            BufKind::F16 => {
                let _ = writeln!(s, "  {id}.p = (uint16_t*)std::calloc({}, sizeof(uint16_t));", len.max(&1));
            }
            BufKind::Int8Rows { unit, .. } => {
                let _ = writeln!(s, "  {id}.q = (int8_t*)std::calloc({}, 1); {id}.s = (uint16_t*)std::calloc({}, sizeof(uint16_t)); {id}.unit = {unit};", len.max(&1), (len / unit).max(1));
            }
        }
    }
    s
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_cpu/model/tests.rs"]
mod tests;
