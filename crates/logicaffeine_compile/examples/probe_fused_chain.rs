//! Probe: the fused-block lane's choreography on the sky fabric, printed stage by stage.
//!
//! `from_fused_block` builds matmul → requant → bias → relu on one site. This prints the program the passes
//! produce, or the fault they refuse with, so a deadlock in that lane can be read rather than guessed at.

use logicaffeine_compile::codegen_sva::systolic::{FusedBlock, Systolic};
use logicaffeine_compile::spatial::passes::from_tensor_op::from_fused_block;
use logicaffeine_compile::spatial::target::SpatialTarget;

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    lo + (x % (hi - lo + 1) as u64) as i64
}

fn main() {
    let sky = SpatialTarget::sky_fabric(64, 64);
    let (k, n, m) = (6usize, 4usize, 5usize);
    let weights: Vec<i64> = (0..k * n).map(|i| prng(3, i, -110, 110)).collect();
    let bias: Vec<i64> = (0..n).map(|i| prng(5, i, -400, 400)).collect();
    let fb = FusedBlock {
        sys: Systolic::new(k, n, weights),
        requant_m: 181,
        requant_s: 8,
        bias,
    };
    let p = from_fused_block(&fb, m, &sky).expect("template");
    println!("── template ──");
    for b in &p.buffers {
        println!("  buffer {:>2} {:<10} {:?}", b.id.idx(), b.name, b.kind);
    }
    for s in &p.stages {
        println!(
            "  stage {:>2} {:<10} role={:?} ins={:?} outs={:?}",
            s.id.idx(),
            s.name,
            s.role,
            s.ins.iter().map(|b| b.idx()).collect::<Vec<_>>(),
            s.outs.iter().map(|b| b.idx()).collect::<Vec<_>>()
        );
    }
    // Step the passes by hand so the schedule can be read even when verification refuses it.
    let mut cur = p;
    for (name, pass) in logicaffeine_compile::spatial::pipeline::PASS_ORDER {
        match pass(&cur, &sky) {
            Ok(next) => cur = next,
            Err(e) => {
                println!("── {name} refused ──\n  {e:?}");
                break;
            }
        }
        if *name == "persist" {
            println!("── scheduled ──");
            for prog in &cur.schedule.as_ref().expect("scheduled").programs {
                println!(
                    "  {:?} {:?}: {} ops",
                    prog.site,
                    prog.engine,
                    prog.ops.len()
                );
                for (i, op) in prog.ops.iter().enumerate() {
                    println!("    {i:>3} {:?}", op.op);
                }
            }
        }
    }
}
