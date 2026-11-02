//! Tensix (TT-Metalium) lowering of the spatial IR. Every kernel — reader, compute, writer — is PRINTED
//! from the schedule; no vendor kernel is wired by path, and every construct the artifact carries is the
//! image of an IR object (the choreography census is a bijection, locked).
//!
//! The lowering reads only the verified program and its target:
//!   buffers   → circular buffers (`cb.rs`: one `CircularBufferConfig` per buffer template);
//!   programs  → kernels (`roles.rs`: equal engine signatures share one source — the vendor's four
//!               reader variants are DERIVED, not enumerated);
//!   site data → runtime args (`args.rs`: one layout drives both the kernel text and the host loop);
//!   ops       → C++ (`kernel.rs`: every `ChoreoOp` is one statement kind);
//!   the whole → host + CMake + a C oracle (`host.rs`, `oracle.rs`).

mod args;
mod cb;
mod host;
mod kernel;
mod oracle;
mod roles;

pub use args::{ArgSlot, ArgSource, RuntimeArgLayout};
pub use cb::{CbPlan, CbSlot};
pub use host::mcast_corners;
pub use roles::{KernelClass, Launch};

use super::{
    require_one_plan, require_verified, Census, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use crate::spatial::ir::*;
use crate::spatial::target::{Fidelity, SpatialTarget};

#[derive(Clone, Debug, PartialEq)]
pub enum KernelRole {
    Movement { engine: u8 },
    Compute { fp32_acc: bool, fidelity: Fidelity },
}

/// One kernel SOURCE and its launches: a launch is a `CreateKernel` on one NOC over the maximal rectangles
/// of the sites it covers (the writer of a 2-D GEMM is one source launched on both NOCs).
#[derive(Clone, Debug, PartialEq)]
pub struct Kernel {
    pub name: String,
    pub role: KernelRole,
    pub source: String,
    pub launches: Vec<Launch>,
    pub compile_args: Vec<(String, u32)>,
}

impl Kernel {
    /// Every site the source runs on.
    pub fn sites(&self) -> Vec<Site> {
        self.launches
            .iter()
            .flat_map(|l| l.sites.iter().copied())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TensixArtifact {
    pub name: String,
    pub kernels: Vec<Kernel>,
    pub host_cpp: String,
    pub cmake: String,
    pub runtime_arg_layout: RuntimeArgLayout,
    pub oracle_c: String,
    pub census: Census,
    pub dprint_cores: Option<String>,
    pub pass_marker: &'static str,
    /// The ledger's `LOGOS_MM_CFG` line the host prints (the arrangement, grid and blocking of the plan).
    pub cfg_line: String,
}

/// The files of a bundle as `(relative path, contents)` — the ledger hashes exactly these.
pub fn bundle_files(art: &TensixArtifact) -> Vec<(String, String)> {
    let mut files = vec![("host.cpp".to_string(), art.host_cpp.clone())];
    for k in &art.kernels {
        files.push((format!("kernels/{}", k.name), k.source.clone()));
    }
    files
}

/// Every file a standalone bundle directory needs (the measure harness writes these verbatim).
pub fn bundle_dir_files(art: &TensixArtifact) -> Vec<(String, String)> {
    let mut files = bundle_files(art);
    files.push(("CMakeLists.txt".to_string(), art.cmake.clone()));
    files.push(("logos_oracle.c".to_string(), art.oracle_c.clone()));
    files
}

pub struct TensixBackend {
    pub target: SpatialTarget,
}

impl TensixBackend {
    pub fn new(target: SpatialTarget) -> Self {
        TensixBackend { target }
    }

    /// The lowering proper: a pure function of the verified program and the target.
    /// Just the kernels — everything the compute engines run, and nothing that needs the HOST.
    ///
    /// ★ WHY THIS IS A SEPARATE ENTRY POINT. The host program prints a golden, and a golden needs to
    /// reproduce the input data, so `print_host` refuses a tensor whose fill is `External`. That is the
    /// right refusal for a runnable bundle and the wrong one for a caller that only wants the kernel:
    /// `compile_to_tenstorrent_matmul` returns a kernel string, and a GEMM over data the compiler
    /// cannot reproduce still has a perfectly good kernel. Splitting the two means neither has to
    /// weaken for the other.
    pub fn printed_kernels(&self, p: &SpatialProgram) -> Result<Vec<Kernel>, SpatialLowerError> {
        Ok(self.print_all(p)?.0)
    }

    /// The compute kernel's source alone — what the kernel-returning `compile_to_tenstorrent_*` entry
    /// points hand back.
    pub fn compute_kernel_source(&self, p: &SpatialProgram) -> Result<String, SpatialLowerError> {
        self.printed_kernels(p)?
            .into_iter()
            .find(|k| matches!(k.role, KernelRole::Compute { .. }))
            .map(|k| k.source)
            .ok_or(SpatialLowerError::Unsupported(
                "a program with no compute kernel",
            ))
    }

    /// The shared front half of a Tensix lowering: validate, plan the circular buffers, classify the
    /// engine programs, and PRINT THE KERNELS. Everything after this needs the host, which needs the
    /// input fills; splitting here is what lets `printed_kernels` succeed where a full bundle cannot.
    #[allow(clippy::type_complexity)]
    fn print_all(
        &self,
        p: &SpatialProgram,
    ) -> Result<
        (
            Vec<Kernel>,
            cb::CbPlan,
            Vec<roles::KernelClass>,
            RuntimeArgLayout,
            bool,
            Fidelity,
        ),
        SpatialLowerError,
    > {
        require_verified(p)?;
        require_one_plan(p)?;
        let target = &self.target;
        if target.tile.rows != 32 || target.tile.cols != 32 {
            return Err(SpatialLowerError::Unsupported("Tensix tiles are 32×32"));
        }
        let plan = p
            .work
            .plan
            .ok_or(SpatialLowerError::Unsupported("a tiled plan"))?;
        let exact_f32 = p.numerics == NumericsPolicy::Exact && plan.acc == AccPolicy::F32;
        if matches!(plan.acc, AccPolicy::I32 | AccPolicy::I64) {
            return Err(SpatialLowerError::Unsupported(
                "integer accumulate on a float-primary matrix engine",
            ));
        }
        let fidelity = target
            .matmul_fidelity()
            .ok_or(SpatialLowerError::Unsupported("a matrix engine"))?;
        let cbs = cb::CbPlan::of(p, target)?;
        let classes = roles::classes(p)?;
        let mut layout = RuntimeArgLayout::default();
        let mut kernels = Vec::new();
        for class in &classes {
            let site_slots = args::slots_for(p, class)?;
            let (source, slots) =
                kernel::print_kernel(p, target, &cbs, class, &site_slots, exact_f32)?;
            let role = match class.engine {
                Engine::Movement(e) => KernelRole::Movement { engine: e },
                Engine::Compute => KernelRole::Compute {
                    fp32_acc: exact_f32,
                    fidelity,
                },
            };
            kernels.push(Kernel {
                name: class.file_name.clone(),
                role,
                source,
                launches: class.launches.clone(),
                compile_args: Vec::new(),
            });
            layout.per_kernel.push((class.file_name.clone(), slots));
        }
        Ok((kernels, cbs, classes, layout, exact_f32, fidelity))
    }

    pub fn lower_tensix(&self, p: &SpatialProgram) -> Result<TensixArtifact, SpatialLowerError> {
        let target = &self.target;
        let (kernels, cbs, classes, layout, exact_f32, fidelity) = self.print_all(p)?;
        let cfg_line = host::cfg_line(p);
        let host_cpp = host::print_host(
            p, target, &cbs, &classes, &layout, exact_f32, fidelity, &cfg_line,
        )?;
        let cmake = host::print_cmake();
        let oracle_c = oracle::print_oracle(p)?;
        Ok(TensixArtifact {
            name: p.name.clone(),
            kernels,
            host_cpp,
            cmake,
            runtime_arg_layout: layout,
            oracle_c,
            census: Census::of(p),
            dprint_cores: None,
            pass_marker: "LOGOS_MM_PASS",
            cfg_line,
        })
    }
}

impl SpatialBackend for TensixBackend {
    fn name(&self) -> &str {
        self.target.name
    }
    fn target(&self) -> &SpatialTarget {
        &self.target
    }
    fn lower(&self, p: &SpatialProgram) -> Result<SpatialArtifact, SpatialLowerError> {
        Ok(SpatialArtifact::Tensix(self.lower_tensix(p)?))
    }
}

/// The name of a buffer template: the placed instance's name up to its `@site` suffix.
pub(crate) fn template_name(p: &SpatialProgram, b: &Buffer) -> String {
    let name = &b.name;
    let base = name.split('@').next().unwrap_or(name);
    let _ = p;
    base.to_string()
}

/// The template id of a placed buffer.
pub(crate) fn template_of(b: &Buffer) -> BufferId {
    b.origin.unwrap_or(b.id)
}
