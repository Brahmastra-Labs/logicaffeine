//! Which buffer the fabric lowering emits a module for but never instantiates.
use logicaffeine_compile::spatial::eval::{Inputs, Value};
use logicaffeine_compile::spatial::ir::BufferKind;
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorRole};
use logicaffeine_compile::spatial::lower::{
    fabric::FabricBackend, Census, SpatialArtifact, SpatialBackend,
};
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_source};
use logicaffeine_compile::spatial::target::SpatialTarget;

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

fn inputs_from_fills(p: &SpatialProgram) -> Inputs {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let n = t.elems.rows * t.elems.cols;
        if t.init.is_float() {
            let vals: Vec<f64> = (0..n)
                .map(|i| t.init.element_f64(i).expect("deterministic float fill"))
                .collect();
            inputs.tensors.insert(t.id, Value::Float(vals));
        } else {
            let vals: Vec<i64> = (0..n)
                .map(|i| t.init.element(i).expect("deterministic fill"))
                .collect();
            inputs.tensors.insert(t.id, Value::Int(vals));
        }
    }
    inputs
}

fn main() {
    let mut t = SpatialTarget::sky_fabric(16, 16);
    t.tile = logicaffeine_compile::spatial::target::TileShape { rows: 4, cols: 4 };
    t.site.acc_tiles = 4;
    let tpl = template_from_source(&gemm_program(16, 8, 8), &t).expect("template");
    let p = run_passes(tpl, &t).expect("passes");
    println!("sites {:?}", p.sites());
    for b in &p.buffers {
        println!(
            "  buf {:>2} {:<10} site {:?}  kind {:?}",
            b.id.idx(),
            b.name,
            b.site,
            b.kind
        );
    }
    println!("IR census: {:?}", Census::of(&p));
    let art = FabricBackend::new(t).lower(&p).expect("lower");
    if let SpatialArtifact::Verilog(v) = art {
        println!("RTL census: {:?}", Census::of_verilog(&v.rtl));
        println!("-- fifo_tpl lines --");
        for l in v.rtl.lines().filter(|l| {
            l.trim_start().starts_with("fifo_tpl_")
                || l.trim_start().starts_with("module fifo_tpl_")
        }) {
            println!("   {}", &l.trim()[..l.trim().len().min(96)]);
        }
        let dir = std::path::Path::new("/tmp/logos_wd_probe");
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("spatial_top.v"), &v.rtl).unwrap();
        std::fs::write(dir.join("tb.v"), &v.testbench).unwrap();
        for (name, body) in &v.hex {
            std::fs::write(dir.join(name), body).unwrap();
        }
        for (name, body) in logicaffeine_compile::spatial::lower::fabric::input_hex_files(
            &p,
            &inputs_from_fills(&p),
        ) {
            std::fs::write(dir.join(&name), &body).unwrap();
        }
        println!("cycle_model {} floor {}", v.cycle_model, v.cycle_floor);
        println!(
            "guard line: {}",
            v.testbench
                .lines()
                .find(|l| l.contains("guard <"))
                .unwrap_or("?")
        );
        println!("wrote {}", dir.display());
    }
    let _ = BufferKind::Fifo {
        depth_pages: 0,
        page_tiles: 0,
    };
}
