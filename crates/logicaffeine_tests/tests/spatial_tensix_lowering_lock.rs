//! ★ SPATIAL → TENSIX LOWERING LOCK ★ — Tensix is a LOWERING of the spatial IR, not a template emitter.
//!
//! The Tensix backend may read only the verified program and its target. Every construct it emits must be
//! the image of an IR object, and nothing else may exist: the CHOREOGRAPHY CENSUS is a bijection.
//!   A. `CreateSemaphore` calls == IR semaphores; `CreateCircularBuffer` calls == distinct buffer templates;
//!      kernel sources == engine-program classes; `CreateKernel` calls == distinct (source, engine, NOC);
//!      every kernel file the host names exists in the artifact and vice versa; every runtime-arg slot the
//!      layout declares is read by its kernel and every read has a slot; `fp32_dest_acc_en` iff the plan
//!      accumulates fp32 exactly; every reader/writer NOC equals the IR's engine assignment.
//!   B. The emitted program is a FUNCTION of the IR: lowering is deterministic; doubling a FIFO depth changes
//!      exactly that circular buffer's size; erasing the multicast groups erases the semaphores.
//!   C. Structural refusals are typed: an unverified program does not lower.
//!   D. NO VENDOR KERNEL PATHS anywhere in the compiler, the examples or the CLI — a shrink-only ratchet
//!      that is empty by construction.
//!   E. The streamed placement reproduces tt-metal's `split_work_to_cores` exactly (transcribed tables).
//!   F. The host prints the ledger's `LOGOS_MM_CFG` / `LOGOS_MM_PASS` lines and never a knob it does not run.
//!
//! ⚠️ If a lock goes RED, fix the lowering, never the assertion.

use logicaffeine_compile::spatial::ir::{
    AccPolicy, BufferKind, ChoreoOp, Engine, EnginePacking, Granularity, NumericsPolicy, Operand,
    Site, StageKind,
};
use logicaffeine_compile::spatial::lower::tensix::{
    bundle_files, mcast_corners, ArgSource, KernelRole, TensixArtifact, TensixBackend,
};
use logicaffeine_compile::spatial::lower::{
    Census, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::passes::place::split_work_to_sites;
use logicaffeine_compile::spatial::pipeline::{
    calibration_candidates, calibration_candidates_matching, compile_to_spatial,
    compile_to_spatial_with, template_from_source, CandidateKind, PlanFilter,
};
use logicaffeine_compile::spatial::target::{ComputeCap, MeshShape, SpatialTarget};

fn gemm_program(m: usize, n: usize, k: usize) -> String {
    format!(
        r#"## Main
Let rows be {m}.
Let cols be {n}.
Let mid be {k}.
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable p be 0.
While p is less than rows * mid:
    Push (p % 7) - 3 to a.
    Set p to p + 1.
Set p to 0.
While p is less than mid * cols:
    Push (p % 11) - 5 to b.
    Set p to p + 1.
Set p to 0.
While p is less than rows * cols:
    Push 0 to c.
    Set p to p + 1.
Let mutable i be 0.
While i is less than rows:
    Let mutable k be 0.
    While k is less than mid:
        Let mutable j be 0.
        While j is less than cols:
            Let idx be i * cols + j + 1.
            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Let mutable q be 1.
While q is at most rows * cols:
    Set checksum to checksum + item q of c.
    Set q to q + 1.
Show checksum.
"#
    )
}

/// The streamed (per-tile FIFO, no multicast) candidate the pipeline can emit for a shape on a target.
fn streamed_candidate(
    m: usize,
    n: usize,
    k: usize,
    target: &SpatialTarget,
    numerics: NumericsPolicy,
) -> logicaffeine_compile::spatial::ir::SpatialProgram {
    calibration_candidates(&gemm_program(m, n, k), target, numerics)
        .unwrap()
        .into_iter()
        .find(|(kind, _)| *kind == CandidateKind::Streamed)
        .map(|(_, p)| p)
        .expect("a streamed candidate")
}

fn lower(p: &logicaffeine_compile::spatial::ir::SpatialProgram) -> TensixArtifact {
    match TensixBackend::new(SpatialTarget::wormhole())
        .lower(p)
        .unwrap()
    {
        SpatialArtifact::Tensix(a) => a,
        other => panic!("{other:?}"),
    }
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

fn census_bijection(m: usize, n: usize, k: usize) {
    let wh = SpatialTarget::wormhole();
    let p = compile_to_spatial(&gemm_program(m, n, k), &wh).unwrap();
    let art = lower(&p);
    let host = &art.host_cpp;
    let census = Census::of(&p);
    assert_eq!(
        art.census, census,
        "{m}x{n}x{k}: the artifact reports the IR's census"
    );

    assert_eq!(
        count(host, "CreateSemaphore("),
        p.semaphores.len(),
        "{m}x{n}x{k}: one semaphore per IR semaphore"
    );
    let templates: std::collections::BTreeSet<_> =
        p.buffers.iter().map(|b| b.origin.unwrap_or(b.id)).collect();
    // A template that shares another's memory (`Buffer::alias_of`) is an index of that template's circular buffer:
    // one buffer per region of memory, one index per template.
    let regions: std::collections::BTreeSet<_> = p
        .buffers
        .iter()
        .filter(|b| b.alias_of.is_none())
        .map(|b| b.origin.unwrap_or(b.id))
        .collect();
    assert_eq!(
        count(host, "CreateCircularBuffer("),
        regions.len(),
        "{m}x{n}x{k}: one circular buffer per region of memory"
    );
    assert_eq!(
        count(host, ".set_page_size("),
        templates.len(),
        "{m}x{n}x{k}: one circular-buffer index per buffer template"
    );
    assert_eq!(
        art.kernels.len(),
        p.summary().engine_program_classes,
        "{m}x{n}x{k}: one kernel source per engine-program class"
    );
    let pl = p.placement.as_ref().unwrap();
    let sched = p.schedule.as_ref().unwrap();
    let mut launches = std::collections::BTreeSet::new();
    for prog in &sched.programs {
        let noc = pl
            .engine_of
            .iter()
            .find(|e| e.site == prog.site && e.engine == prog.engine)
            .and_then(|e| e.noc);
        launches.insert((p.engine_signature(prog), prog.engine, noc));
    }
    assert_eq!(
        count(host, "CreateKernel("),
        launches.len(),
        "{m}x{n}x{k}: one launch per distinct (source, engine, NOC)"
    );

    for kern in &art.kernels {
        assert!(
            host.contains(&format!("LOGOS_KERNEL_DIR \"{}\"", kern.name)),
            "{m}x{n}x{k}: host does not launch {}",
            kern.name
        );
        let layout = art
            .runtime_arg_layout
            .per_kernel
            .iter()
            .find(|(name, _)| *name == kern.name)
            .map(|(_, slots)| slots.len())
            .unwrap_or(0);
        let mut reads: Vec<usize> = Vec::new();
        for (i, _) in kern.source.match_indices("get_arg_val<uint32_t>(") {
            let rest = &kern.source[i + "get_arg_val<uint32_t>(".len()..];
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = num.parse::<usize>() {
                reads.push(n);
            }
        }
        reads.sort();
        reads.dedup();
        assert_eq!(
            reads,
            (0..layout).collect::<Vec<_>>(),
            "{m}x{n}x{k}: kernel {} reads slots {reads:?} but the layout declares {layout}",
            kern.name
        );
        match kern.role {
            KernelRole::Movement { engine } => {
                for launch in &kern.launches {
                    let noc = launch.noc.expect("a movement launch names its NOC");
                    for rect in &launch.rects {
                        for site in rect.sites() {
                            let slot = pl
                                .engine_of
                                .iter()
                                .find(|e| e.site == site && e.engine == Engine::Movement(engine))
                                .unwrap();
                            assert_eq!(slot.noc, Some(noc), "{m}x{n}x{k}: kernel {} at {site:?} runs on NOC {noc:?} but the IR assigned {:?}", kern.name, slot.noc);
                        }
                    }
                }
            }
            KernelRole::Compute { fp32_acc, .. } => {
                let exact_f32 = p.numerics == NumericsPolicy::Exact
                    && p.work.plan.unwrap().acc == AccPolicy::F32;
                assert_eq!(fp32_acc, exact_f32);
                assert_eq!(host.contains("fp32_dest_acc_en = true"), exact_f32);
            }
        }
    }
    for launched in host.match_indices("LOGOS_KERNEL_DIR \"") {
        let rest = &host[launched.0 + "LOGOS_KERNEL_DIR \"".len()..];
        let name: String = rest.chars().take_while(|c| *c != '"').collect();
        assert!(
            art.kernels.iter().any(|k| k.name == name),
            "{m}x{n}x{k}: host launches {name} which the artifact does not carry"
        );
    }
    assert!(
        !host.contains("TT_METAL_HOME") || host.contains("getenv"),
        "no kernel path is built from the SDK tree"
    );
    assert!(
        host.contains("LOGOS_MM_CFG strategy=") && host.contains("LOGOS_MM_PASS fidelity="),
        "the host speaks the ledger's vocabulary"
    );
    assert!(
        art.oracle_c.contains("int main("),
        "the ≡VM C oracle travels with the bundle"
    );
}

#[test]
fn the_choreography_census_is_a_bijection_for_the_streamed_plan() {
    census_bijection(640, 640, 640);
}

#[test]
fn the_choreography_census_is_a_bijection_for_the_multicast_plan() {
    census_bijection(2048, 1024, 512);
    census_bijection(2048, 2048, 1024);
}

/// The measured 2048×1024×512 multicast program built with its plan stated (the structure of the fused program
/// is what this lock is about; the decision is locked against the ledger elsewhere).
fn fused_2048() -> logicaffeine_compile::spatial::ir::SpatialProgram {
    use logicaffeine_compile::spatial::ir::{BlockPlan, BlockShape, InnerLoop};
    use logicaffeine_compile::spatial::passes::{
        fuse_readers::fuse_readers_forced, persist::persist, place::place,
        size_buffers::size_buffers, synthesize_handshakes::synthesize_handshakes, tile::apply_plan,
        verify::verify,
    };
    let wh = SpatialTarget::wormhole();
    let t = template_from_source(&gemm_program(2048, 1024, 512), &wh).unwrap();
    let plan = BlockPlan {
        per_site_m: 8,
        per_site_n: 4,
        grid: (8, 8),
        inner: InnerLoop {
            num_blocks: 1,
            block_k_tiles: 16,
        },
        block: BlockShape {
            osh: 1,
            osw: 4,
            ibw: 16,
        },
        acc: AccPolicy::F32,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    };
    let placed = place(&apply_plan(&t, plan, &wh), &wh).unwrap();
    let fused = fuse_readers_forced(&placed, &wh, true).unwrap();
    verify(
        &persist(
            &synthesize_handshakes(&size_buffers(&fused, &wh).unwrap(), &wh).unwrap(),
            &wh,
        )
        .unwrap(),
        &wh,
    )
    .unwrap()
}

#[test]
fn the_multicast_plan_yields_exactly_the_vendor_role_structure_derived() {
    let p = fused_2048();
    let art = lower(&p);
    let movement: Vec<_> = art
        .kernels
        .iter()
        .filter(|k| matches!(k.role, KernelRole::Movement { .. }))
        .collect();
    let compute: Vec<_> = art
        .kernels
        .iter()
        .filter(|k| matches!(k.role, KernelRole::Compute { .. }))
        .collect();
    assert_eq!(compute.len(), 1);
    assert_eq!(movement.len(), 5, "4 reader roles + 1 writer source");
    assert_eq!(count(&art.host_cpp, "CreateSemaphore("), 4);
    let sender_sender = movement
        .iter()
        .find(|k| k.source.matches("noc_async_write_multicast(").count() == 2)
        .expect("a kernel that multicasts both operands");
    assert_eq!(
        sender_sender.sites(),
        vec![Site {
            x: 0,
            y: 0,
            chip: 0
        }],
        "the corner site sends both operands"
    );
    let receiver_receiver = movement
        .iter()
        .find(|k| {
            k.source.matches("noc_semaphore_inc(").count() == 2
                && !k.source.contains("noc_async_write_multicast(")
        })
        .expect("a kernel that receives both operands");
    assert_eq!(
        receiver_receiver.sites().len(),
        49,
        "7×7 interior sites receive both"
    );
    let writer = movement
        .iter()
        .find(|k| {
            k.source.contains("noc_async_write_page(") && !k.source.contains("noc_async_read_page(")
        })
        .expect("the writer source");
    assert_eq!(
        writer.launches.len(),
        2,
        "one writer source, launched on both NOCs (left column on NOC 1)"
    );
    for k in &movement {
        assert!(
            !k.source.contains("get_semaphore(0)") || k.source.contains("get_arg_val"),
            "semaphore ids come through runtime args, never literals"
        );
    }
}

#[test]
fn the_emitted_program_is_a_function_of_the_ir() {
    let wh = SpatialTarget::wormhole();
    let p = compile_to_spatial(&gemm_program(2048, 1024, 512), &wh).unwrap();
    let a = lower(&p);
    let b = lower(&p);
    assert_eq!(bundle_files(&a), bundle_files(&b), "deterministic");
    // Double one FIFO template's depth: exactly that circular buffer's byte size doubles.
    let mut deeper = p.clone();
    deeper.certificate = None;
    let in0s: Vec<usize> = deeper
        .buffers
        .iter()
        .enumerate()
        .filter(|(_, b)| b.name.starts_with("in0"))
        .map(|(i, _)| i)
        .collect();
    for i in &in0s {
        if let BufferKind::Fifo {
            depth_pages,
            page_tiles,
        } = deeper.buffers[*i].kind
        {
            deeper.buffers[*i].kind = BufferKind::Fifo {
                depth_pages: depth_pages * 2,
                page_tiles,
            };
        }
    }
    let deeper = logicaffeine_compile::spatial::passes::verify::verify(&deeper, &wh)
        .expect("a deeper FIFO still verifies");
    let d = lower(&deeper);
    let size_of = |host: &str, cb: &str| -> usize {
        let i = host.find(cb).unwrap_or_else(|| panic!("{cb} not in host"));
        let rest = &host[i..];
        let n: String = rest
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        n.parse().unwrap()
    };
    assert_eq!(
        size_of(&d.host_cpp, "CircularBufferConfig cb_in0"),
        2 * size_of(&a.host_cpp, "CircularBufferConfig cb_in0"),
        "the in0 circular buffer doubled"
    );
    assert_eq!(
        size_of(&d.host_cpp, "CircularBufferConfig cb_in1"),
        size_of(&a.host_cpp, "CircularBufferConfig cb_in1"),
        "in1 unchanged"
    );
    // Erase the groups: the semaphores go with them.
    //
    // ★ BY REMOVING THE CAPABILITY, not by pricing it out of reach. This used to set `c_mcast_block` to
    // 2e7 in the fitted cycle model — and that model no longer decides anything, because
    // `ttsim_instrument_audit.json` shows the counter it was fitted to is not a function of the program
    // (8x the reduction depth moves it 0.08%). Pricing multicast at twenty million therefore changed
    // nothing at all and the planner kept choosing 16 groups, so the three assertions below were about to
    // start testing a FUSED program while still named for an unfused one. A target without a multicast
    // interconnect cannot fuse whatever anything costs, which is what this wanted to say.
    let unfused = compile_to_spatial(&gemm_program(2048, 1024, 512), &{
        let mut t = wh.clone();
        t.interconnect
            .remove(&logicaffeine_compile::spatial::target::Interconnect::MulticastRect);
        t
    })
    .unwrap();
    assert_eq!(unfused.summary().multicast_groups, 0);
    let u = lower(&unfused);
    assert_eq!(count(&u.host_cpp, "CreateSemaphore("), 0);
    assert!(!u.kernels.iter().any(|k| k.source.contains("noc_semaphore")));
}

#[test]
fn an_unverified_program_does_not_lower() {
    let wh = SpatialTarget::wormhole();
    let t = template_from_source(&gemm_program(64, 64, 64), &wh).unwrap();
    assert!(matches!(
        TensixBackend::new(wh.clone()).lower(&t),
        Err(SpatialLowerError::NotVerified { .. })
    ));
    let mut p = compile_to_spatial(&gemm_program(64, 64, 64), &wh).unwrap();
    p.certificate = None;
    assert!(matches!(
        TensixBackend::new(wh).lower(&p),
        Err(SpatialLowerError::NotVerified { .. })
    ));
}

#[test]
fn no_vendor_kernel_paths_anywhere() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap();
    let mut hits = Vec::new();
    fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if p.file_name().map(|n| n == "target").unwrap_or(false) {
                    continue;
                }
                walk(&p, out);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    for dir in [
        "crates/logicaffeine_compile/src",
        "crates/logicaffeine_compile/examples",
        "apps/logicaffeine_cli/src",
    ] {
        walk(&root.join(dir), &mut files);
    }
    let me = std::path::Path::new(file!())
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    for f in files {
        if f.file_name()
            .map(|n| n.to_string_lossy() == me)
            .unwrap_or(false)
        {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        for needle in [
            "reader_bmm",
            "writer_bmm",
            "matmul_common",
            "reader_mm_output",
            "writer_unary",
            "programming_examples/matmul/",
            "logos_mm.cpp",
            "OVERRIDE_KERNEL_PREFIX",
        ] {
            if text.contains(needle) {
                hits.push(format!("{}: {needle}", f.display()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "vendor kernels must never be wired by path; found:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_streamed_placement_reproduces_tt_metal_split_work_to_cores() {
    let mesh = MeshShape { x: 8, y: 8 };
    // 400 output tiles: 16 sites × 7 (column-major rect (0,0)-(1,7), x-fastest inside) then 48 × 6 from (2,0).
    let s = split_work_to_sites(mesh, 400);
    assert_eq!(s.len(), 64);
    assert_eq!(
        &s[..4],
        &[
            (
                Site {
                    x: 0,
                    y: 0,
                    chip: 0
                },
                0,
                7
            ),
            (
                Site {
                    x: 1,
                    y: 0,
                    chip: 0
                },
                7,
                7
            ),
            (
                Site {
                    x: 0,
                    y: 1,
                    chip: 0
                },
                14,
                7
            ),
            (
                Site {
                    x: 1,
                    y: 1,
                    chip: 0
                },
                21,
                7
            )
        ]
    );
    assert_eq!(
        s[15],
        (
            Site {
                x: 1,
                y: 7,
                chip: 0
            },
            105,
            7
        )
    );
    assert_eq!(
        s[16],
        (
            Site {
                x: 2,
                y: 0,
                chip: 0
            },
            112,
            6
        )
    );
    assert_eq!(
        s[17],
        (
            Site {
                x: 3,
                y: 0,
                chip: 0
            },
            118,
            6
        )
    );
    assert_eq!(
        s[22],
        (
            Site {
                x: 2,
                y: 1,
                chip: 0
            },
            148,
            6
        )
    );
    assert_eq!(
        s[63],
        (
            Site {
                x: 7,
                y: 7,
                chip: 0
            },
            394,
            6
        )
    );
    assert_eq!(s.iter().map(|(_, _, c)| c).sum::<usize>(), 400);
    // 64 tiles: every site one tile, x-fastest over the whole grid.
    let s = split_work_to_sites(mesh, 64);
    assert_eq!(
        s[1],
        (
            Site {
                x: 1,
                y: 0,
                chip: 0
            },
            1,
            1
        )
    );
    assert_eq!(
        s[8],
        (
            Site {
                x: 0,
                y: 1,
                chip: 0
            },
            8,
            1
        )
    );
    // 2048 tiles: 32 each.
    let s = split_work_to_sites(mesh, 2048);
    assert!(s.iter().all(|(_, _, c)| *c == 32));
    // 100 tiles: 36 sites × 2 (rect (0,0)-(3,7) then (4,0)-(4,3)), then 28 × 1 from (4,4): (4,4)-(4,7) then (5,0)-(7,7).
    let s = split_work_to_sites(mesh, 100);
    assert_eq!(s.len(), 64);
    assert_eq!(
        s[0],
        (
            Site {
                x: 0,
                y: 0,
                chip: 0
            },
            0,
            2
        )
    );
    assert_eq!(
        s[31],
        (
            Site {
                x: 3,
                y: 7,
                chip: 0
            },
            62,
            2
        )
    );
    assert_eq!(
        s[32],
        (
            Site {
                x: 4,
                y: 0,
                chip: 0
            },
            64,
            2
        )
    );
    assert_eq!(
        s[35],
        (
            Site {
                x: 4,
                y: 3,
                chip: 0
            },
            70,
            2
        )
    );
    assert_eq!(
        s[36],
        (
            Site {
                x: 4,
                y: 4,
                chip: 0
            },
            72,
            1
        )
    );
    assert_eq!(
        s[39],
        (
            Site {
                x: 4,
                y: 7,
                chip: 0
            },
            75,
            1
        )
    );
    assert_eq!(
        s[40],
        (
            Site {
                x: 5,
                y: 0,
                chip: 0
            },
            76,
            1
        )
    );
    assert_eq!(
        s[63],
        (
            Site {
                x: 7,
                y: 7,
                chip: 0
            },
            99,
            1
        )
    );
    // Fewer tiles than sites: only the first `n` sites, column-major.
    let s = split_work_to_sites(mesh, 10);
    assert_eq!(s.len(), 10);
    assert_eq!(
        s[8],
        (
            Site {
                x: 1,
                y: 0,
                chip: 0
            },
            8,
            1
        )
    );
}

#[test]
fn the_streamed_placement_reproduces_the_simulated_wormhole_grid_table() {
    // The vendor `matmul_multi_core` host, instrumented to print its `split_work_to_cores` mapping on the
    // simulated Wormhole (8×9 compute-with-storage cores under slow dispatch), printed exactly this table for
    // 400 output tiles: 40 cores × 6 (rect (0,0)-(3,8) x-fastest, then the partial column (4,0)-(4,3)), then
    // 32 cores × 5 ((4,4)-(4,8), then (5,0)-(7,8)). A program placed onto 8×8 (16×7 + 48×6) runs 800 cycles
    // slower than this: the ninth row is the whole host-side gap to the vendor's 9,735.
    let mesh = SpatialTarget::wormhole_ttsim().sites;
    assert_eq!((mesh.x, mesh.y), (8, 9));
    let s = split_work_to_sites(mesh, 400);
    assert_eq!(s.len(), 72);
    assert_eq!(
        &s[..4],
        &[
            (
                Site {
                    x: 0,
                    y: 0,
                    chip: 0
                },
                0,
                6
            ),
            (
                Site {
                    x: 1,
                    y: 0,
                    chip: 0
                },
                6,
                6
            ),
            (
                Site {
                    x: 2,
                    y: 0,
                    chip: 0
                },
                12,
                6
            ),
            (
                Site {
                    x: 3,
                    y: 0,
                    chip: 0
                },
                18,
                6
            )
        ]
    );
    assert_eq!(
        s[4],
        (
            Site {
                x: 0,
                y: 1,
                chip: 0
            },
            24,
            6
        )
    );
    assert_eq!(
        s[35],
        (
            Site {
                x: 3,
                y: 8,
                chip: 0
            },
            210,
            6
        )
    );
    assert_eq!(
        s[36],
        (
            Site {
                x: 4,
                y: 0,
                chip: 0
            },
            216,
            6
        )
    );
    assert_eq!(
        s[39],
        (
            Site {
                x: 4,
                y: 3,
                chip: 0
            },
            234,
            6
        )
    );
    assert_eq!(
        s[40],
        (
            Site {
                x: 4,
                y: 4,
                chip: 0
            },
            240,
            5
        )
    );
    assert_eq!(
        s[44],
        (
            Site {
                x: 4,
                y: 8,
                chip: 0
            },
            260,
            5
        )
    );
    assert_eq!(
        s[45],
        (
            Site {
                x: 5,
                y: 0,
                chip: 0
            },
            265,
            5
        )
    );
    assert_eq!(
        s[47],
        (
            Site {
                x: 7,
                y: 0,
                chip: 0
            },
            275,
            5
        )
    );
    assert_eq!(
        s[48],
        (
            Site {
                x: 5,
                y: 1,
                chip: 0
            },
            280,
            5
        )
    );
    assert_eq!(
        s[71],
        (
            Site {
                x: 7,
                y: 8,
                chip: 0
            },
            395,
            5
        )
    );
    assert_eq!(s.iter().map(|(_, _, c)| c).sum::<usize>(), 400);
    // The streamed candidate on the simulated grid places the 640³ program onto all 72 sites with that split.
    let p = streamed_candidate(
        640,
        640,
        640,
        &SpatialTarget::wormhole_ttsim(),
        NumericsPolicy::AllowSpillRounding,
    );
    let pl = p.placement.as_ref().unwrap();
    assert_eq!(pl.sites().len(), 72);
    assert_eq!((pl.mesh.x, pl.mesh.y), (8, 9));
    let counts: Vec<usize> = s
        .iter()
        .map(|(site, _, _)| pl.items_at(*site).len())
        .collect();
    assert_eq!(
        counts,
        s.iter().map(|(_, _, c)| *c).collect::<Vec<_>>(),
        "items per site follow the vendor split"
    );
}

#[test]
fn the_host_refuses_a_device_smaller_than_the_placement_and_records_the_mesh() {
    // The deployment grid is a target fact the host checks at run time: a program placed onto the simulator's
    // 8×9 grid must fail loudly on an 8×8 n150 instead of silently launching kernels on cores that do not exist.
    let art = lower(&streamed_candidate(
        640,
        640,
        640,
        &SpatialTarget::wormhole_ttsim(),
        NumericsPolicy::AllowSpillRounding,
    ));
    assert!(art.host_cpp.contains("compute_with_storage_grid_size()"));
    assert!(art
        .host_cpp
        .contains("constexpr uint32_t PLACED_X = 8, PLACED_Y = 9;"));
    assert!(art
        .host_cpp
        .contains("LOGOS_GRID_MISMATCH device={}x{} placed={}x{}"));
    assert!(art
        .host_cpp
        .contains("if (device_grid.x < PLACED_X || device_grid.y < PLACED_Y)"));
    assert!(
        art.cfg_line.ends_with("mesh=8x9 sites=72"),
        "{}",
        art.cfg_line
    );
    let art8 = lower(&streamed_candidate(
        640,
        640,
        640,
        &SpatialTarget::wormhole(),
        NumericsPolicy::AllowSpillRounding,
    ));
    assert!(art8
        .host_cpp
        .contains("constexpr uint32_t PLACED_X = 8, PLACED_Y = 8;"));
    assert!(
        art8.cfg_line.ends_with("mesh=8x8 sites=64"),
        "{}",
        art8.cfg_line
    );
}

#[test]
fn shape_enters_a_movement_kernel_only_as_tensor_tile_dimensions() {
    // Bisected on ttsim at 640³: shape constants folded into a movement kernel cost +844 cycles (the vendor
    // reader with Mt/Kt/Nt as constants), and runtime-valued LAYOUT strides the compiler cannot see are 1
    // cost +300 (10,056 vs 9,764). The one form that ties the vendor (9,735) is theirs: block extents literal,
    // every stride and count a literal times or over a tensor's runtime tile width, a one-page block read as
    // one page, and a per-tile output written by its item id.
    let art = lower(&streamed_candidate(
        640,
        640,
        640,
        &SpatialTarget::wormhole_ttsim(),
        NumericsPolicy::AllowSpillRounding,
    ));
    let reader = &art
        .kernels
        .iter()
        .find(|k| k.name.starts_with("dm1_"))
        .unwrap()
        .source;
    assert!(
        reader.contains("for (uint32_t kb = 0; kb < p_a_cols; ++kb)"),
        "{reader}"
    );
    assert!(
        reader.contains("noc_async_read_page(oy * p_a_cols + kb, acc_t0, wp_CB_IN0);"),
        "{reader}"
    );
    assert!(
        reader.contains("noc_async_read_page(ox + kb * p_c_cols, acc_t1, wp_CB_IN1);"),
        "N is ONE symbol (B's width ≡ C's width): {reader}"
    );
    assert!(
        reader.contains("const uint32_t oy = item_id / p_c_cols;"),
        "{reader}"
    );
    assert!(
        !reader.contains("tile_bytes_") && !reader.contains("for (uint32_t h = 0"),
        "a one-page block is one read: {reader}"
    );
    let writer = &art
        .kernels
        .iter()
        .find(|k| k.name.starts_with("dm0_"))
        .unwrap()
        .source;
    assert!(
        writer.contains("noc_async_write_page(item_id, acc_t2, get_read_ptr(CB_OUT));"),
        "{writer}"
    );
    assert!(
        !writer.contains("const uint32_t oy"),
        "the writer never derives coordinates it does not use: {writer}"
    );
    // The runtime-arg layout carries shape only as tensor dimensions: no free-standing numbers.
    for (name, slots) in &art.runtime_arg_layout.per_kernel {
        for slot in slots {
            assert!(
                !matches!(
                    slot.source,
                    ArgSource::ItemY0 | ArgSource::ItemX0 | ArgSource::ItemCountX
                ) || name.starts_with("dm"),
                "{name}: {slot:?}"
            );
            if let ArgSource::TensorDim { .. } = slot.source {
                assert!(
                    slot.name.starts_with("p_")
                        && (slot.name.ends_with("_cols") || slot.name.ends_with("_rows")),
                    "{name}: {slot:?}"
                );
            }
        }
    }
    let dims: Vec<&str> = art
        .runtime_arg_layout
        .per_kernel
        .iter()
        .find(|(n, _)| n.starts_with("dm1_"))
        .unwrap()
        .1
        .iter()
        .filter(|s| matches!(s.source, ArgSource::TensorDim { .. }))
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(dims, ["p_c_cols", "p_a_cols"], "the reader's shape parameters: N (the item divisor and B's stride) and K, in first-use order");
    // The compute kernel folds everything: the block loops vanish and the K loop is a literal.
    let compute = &art
        .kernels
        .iter()
        .find(|k| k.name.starts_with("compute_"))
        .unwrap()
        .source;
    assert!(
        compute.contains("for (uint32_t kb = 0; kb < 20u; ++kb)"),
        "{compute}"
    );
    assert!(!compute.contains("p_a_cols"), "{compute}");
    // A blocked plan keeps its block extents literal and its row strides on the tensor width.
    let mc = lower(&fused_2048());
    let reader = &mc
        .kernels
        .iter()
        .find(|k| k.name.starts_with("dm1_"))
        .unwrap()
        .source;
    assert!(
        reader.contains("p_a_cols") && reader.contains("p_c_cols") && !reader.contains("p_b_cols"),
        "{reader}"
    );
    assert!(
        !reader.contains("p_rd_") && !reader.contains("p_n_kb") && !reader.contains("p_grid_nx"),
        "no ad-hoc shape parameters: {reader}"
    );
}

#[test]
fn a_spilled_partial_in_another_format_reconfigures_the_unpacker_and_multicast_corners_follow_the_noc(
) {
    // ttsim, 640³ blocked with exact fp32 partials: PCC 0.165 until the reload reconfigured srcA — tt-metal's
    // plain `copy_tile_to_dst_init_short` does not touch data formats, so an fp32 interm read under the bf16
    // operand configuration is garbage. The bf16-interm program (PCC 1.0 at 7,370 cycles) keeps the plain form.
    //
    // ★ THE WIDE-SPILL HALF RUNS ON A PART THAT DECLARES IT CAN RELOAD ONE, and real Wormhole is not that
    //   part. Reconfiguring srcA is necessary but NOT sufficient: re-measured 2026-09-10, the emitted
    //   exact-numerics 640³ program still aborts on the real part —
    //
    //       [4907] ERROR: UndefinedBehavior:
    //              tensix_unpacr: unpack_to_dst=0 in_data_format=0 out_data_format=0
    //
    //   — the same refusal `spatial_choreography_semantics_lock` records, so `wormhole()` declares
    //   `wide_spill_reload: false` and an exact plan there holds the reduction in one block instead
    //   (PCC 1.0). Asserting the reconfiguring reload on the real part asserted that the compiler EMITS a
    //   program the device rejects. Isolating the one capability under test is what keeps this a lowering
    //   lock, and it is the same contract-A/contract-B split `spatial_choreography_semantics_lock` uses.
    let wh = SpatialTarget::wormhole_ttsim();
    let mut reloads_wide = wh.clone();
    for c in reloads_wide.compute.iter_mut() {
        if let ComputeCap::MatmulTile {
            wide_spill_reload, ..
        } = c
        {
            *wide_spill_reload = true;
        }
    }
    assert!(
        reloads_wide.reloads_wide_spills(),
        "contract A needs a part that can reload a wide spill"
    );
    // ★ AND THE PLAN MUST ACTUALLY SPILL, which is now ASSERTED rather than assumed. There is no reload to
    // reconfigure unless the reduction is split across K blocks, and at 640³ on a full-sized L1 the planner
    // holds the whole depth in ONE block (ibw=20 over kt=20) — so every assertion below was about to start
    // passing over a kernel with no `CB_INTERM` in it at all. Shrinking L1 is what forces the split; the
    // `num_blocks` check is what stops this lapsing silently a second time.
    reloads_wide.site.local_mem_bytes = 96 * 1024;
    // ★ AND THE PLAN MUST RELOAD. An integer GEMM's partial sums are exact in any order, so the planner may finish
    // its reduction in the output's own page and reload nothing (`tile::finishes_in_place`); the packer's
    // accumulate is pinned off here so the lane under test is still the reload's.
    let reload_only = PlanFilter {
        pack_accumulate: Some(false),
        ..PlanFilter::default()
    };
    let exact_p = calibration_candidates_matching(
        &gemm_program(640, 640, 640),
        &reloads_wide,
        NumericsPolicy::Exact,
        reload_only,
    )
    .unwrap()
    .into_iter()
    .find(|(kind, _)| *kind == CandidateKind::Best)
    .map(|(_, p)| p)
    .expect("the best plan that reloads its partial");
    assert!(
        exact_p.summary().num_blocks > 1,
        "the spill lane needs a multi-block plan to reload anything: {:?}",
        exact_p.summary()
    );
    let exact = lower(&exact_p);
    let compute = &exact
        .kernels
        .iter()
        .find(|k| k.name.starts_with("compute_"))
        .unwrap()
        .source;
    assert!(
        compute.contains("copy_tile_to_dst_init_short_with_dt(CB_IN1, CB_INTERM);"),
        "{compute}"
    );
    // …and source A is put BACK before the matmul: the matmul init does not touch data formats, so leaving
    // source A on the spill's Float32 makes the next operand unpack undefined (the simulator refuses it).
    assert!(
        compute.contains("reconfig_data_format_srca(CB_INTERM, CB_IN1);"),
        "{compute}"
    );
    assert!(
        !compute.contains("copy_tile_to_dst_init_short(CB_INTERM);"),
        "{compute}"
    );
    // …and the reload's Float32 buffer unpacks straight to DST (SrcA is 19 bits wide: through it every block
    // boundary rounds the partial to TF32 — PCC 0.14 on ttsim until the buffer carried this mode).
    assert!(
        exact
            .host_cpp
            .contains("utd[24] = UnpackToDestMode::UnpackToDestFp32;"),
        "{}",
        exact.host_cpp
    );
    assert!(exact.host_cpp.contains(".unpack_to_dest_mode = []{ std::vector<UnpackToDestMode> utd(NUM_CIRCULAR_BUFFERS, UnpackToDestMode::Default);"), "{}", exact.host_cpp);
    let relaxed = lower(
        &compile_to_spatial_with(
            &gemm_program(640, 640, 640),
            &wh,
            NumericsPolicy::AllowSpillRounding,
        )
        .unwrap(),
    );
    let compute = &relaxed
        .kernels
        .iter()
        .find(|k| k.name.starts_with("compute_"))
        .unwrap()
        .source;
    if compute.contains("CB_INTERM") {
        assert!(
            compute.contains("copy_tile_to_dst_init_short(CB_INTERM);")
                && !compute.contains("_with_dt"),
            "{compute}"
        );
    }
    assert!(
        relaxed.host_cpp.contains(".unpack_to_dest_mode = {}"),
        "a bf16 program marks nothing: {}",
        relaxed.host_cpp
    );
    // A program that packs into one format never reconfigures the packer; one that packs into two does before
    // every pack (the LLK compares against the live configuration, so the call is idempotent).
    for art in [&exact, &relaxed] {
        let compute = &art
            .kernels
            .iter()
            .find(|k| k.name.starts_with("compute_"))
            .unwrap()
            .source;
        let formats: std::collections::BTreeSet<&str> = ["CB_OUT", "CB_INTERM"]
            .into_iter()
            .filter(|cb| compute.contains(&format!("pack_tile(i, {cb});")))
            .collect();
        let cb_dtypes: std::collections::BTreeSet<_> = art
            .host_cpp
            .lines()
            .filter(|l| {
                l.contains("CircularBufferConfig cb_out")
                    || l.contains("CircularBufferConfig cb_interm")
            })
            .map(|l| {
                l.split("tt::DataFormat::")
                    .nth(1)
                    .unwrap()
                    .split('}')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            compute.contains("pack_reconfig_data_format("),
            formats.len() > 1 && cb_dtypes.len() > 1,
            "{compute}"
        );
    }
    // ttsim, 640³ multicast: `UnimplementedFunctionality: noc_cmd_ctrl: dst_start_y=2 dst_end_y=5` — a NOC 1
    // multicast rectangle must be given far corner first.
    let p = fused_2048();
    let pl = p.placement.as_ref().unwrap();
    let g = &pl.groups[0];
    let (n0, n1) = (
        mcast_corners(&p, g.sender, g.operand, 0).unwrap(),
        mcast_corners(&p, g.sender, g.operand, 1).unwrap(),
    );
    assert_eq!(n0.1, n1.0, "NOC 1 starts where NOC 0 ends");
    assert_eq!(n0.0, n1.1);
    assert!(n0.0 <= n0.1 && n1.0 >= n1.1);
    let art = lower(&p);
    // Every launch on NOC 1 prints its start corner as the far corner in the host.
    for (i, c) in art.kernels.iter().enumerate() {
        for (j, l) in c.launches.iter().enumerate() {
            if l.noc.map(|n| n.0) != Some(1) {
                continue;
            }
            for site in &l.sites {
                let Some(line) = art.host_cpp.lines().find(|ln| {
                    ln.contains(&format!("k_{i}_{j}, CoreCoord{{{}, {}}}", site.x, site.y))
                }) else {
                    continue;
                };
                for operand in [Operand::A, Operand::B] {
                    if let Ok(((sx, sy), (ex, ey))) = mcast_corners(&p, *site, operand, 1) {
                        let start = format!("P({sx}, {sy}).x");
                        let end = format!("P({ex}, {ey}).x");
                        if let (Some(a), Some(b)) = (line.find(&start), line.find(&end)) {
                            assert!(a < b, "{line}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn the_host_never_prints_a_knob_it_does_not_run() {
    let wh = SpatialTarget::wormhole();
    let exact = lower(&compile_to_spatial(&gemm_program(640, 640, 640), &wh).unwrap());
    assert!(
        exact.host_cpp.contains("fp32=1") && exact.host_cpp.contains("fp32_dest_acc_en = true")
    );
    let relaxed = lower(
        &compile_to_spatial_with(
            &gemm_program(640, 640, 640),
            &wh,
            NumericsPolicy::AllowSpillRounding,
        )
        .unwrap(),
    );
    assert!(
        relaxed.host_cpp.contains("fp32=0")
            && !relaxed.host_cpp.contains("fp32_dest_acc_en = true")
    );
    for art in [&exact, &relaxed] {
        assert!(
            art.host_cpp.contains("l1acc=0"),
            "packer L1-accumulate is not emitted yet and must not be claimed"
        );
        assert!(
            art.host_cpp.contains("fidelity=HiFi4") && art.host_cpp.contains("MathFidelity::HiFi4")
        );
        assert!(art.host_cpp.contains("LOGOS_MM_CFG strategy="));
    }
    let _ = (
        ChoreoOp::ReadBarrier,
        StageKind::Recv {
            group: logicaffeine_compile::spatial::ir::GroupId(0),
        },
    );
}
