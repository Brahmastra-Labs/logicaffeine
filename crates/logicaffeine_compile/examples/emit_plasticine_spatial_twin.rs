//! Write a LOGOS dot product, TPC-H Q6, outer product, GDA covariance, Black-Scholes or k-means clustering as Spatial's
//! own regression app at one variant's knobs — the twin of `DotProduct_<k>`, `TPCHQ6_<k>`, `OuterProduct_<k>`,
//! `GDA_<k>`, `BlackScholes_<k>` or `Kmeans_<k>` — into a directory (the public toolchain's `regression/` dir, so `gate.sh <class>` runs it
//! through Spatial → PIR → plastiroute → Tungsten). The LOGOS program is written beside it as `<class>.lg`; the class
//! name is the last line printed. A Black-Scholes variant's PIR arguments (what its `pirArgs` override appends) follow
//! its knobs; a k-means variant's runtime arguments (epochs `I`, points `N`) follow its. Usage:
//!   cargo run -p logicaffeine-compile --example emit_plasticine_spatial_twin -- dot <out_dir> N ts ip op
//!   cargo run -p logicaffeine-compile --example emit_plasticine_spatial_twin -- tpchq6 <out_dir> N ts ip op
//!   cargo run -p logicaffeine-compile --example emit_plasticine_spatial_twin -- outer <out_dir> M N ts1 ts2 ip ip1 ip2 op1 op2
//!   cargo run -p logicaffeine-compile --example emit_plasticine_spatial_twin -- gda <out_dir> C R ts op mp1 mp2 ip
//!   cargo run -p logicaffeine-compile --example emit_plasticine_spatial_twin -- blackscholes <out_dir> N ts op ip [pir-args]
//!   cargo run -p logicaffeine-compile --example emit_plasticine_spatial_twin -- kmeans <out_dir> K D ts ip op mp1 mp2 mp3 I N

use logicaffeine_compile::codegen_sva::plasticine_report::{
    logos_blackscholes_program, logos_dot_program, logos_gda_program, logos_kmeans_program, logos_outer_program,
    logos_tpchq6_program, lower_blackscholes_source, lower_dot_source, lower_gda_source, lower_kmeans_source,
    lower_outer_source, lower_tpchq6_source,
};
use logicaffeine_compile::codegen_sva::spatial_emit::{GdaPar, KmeansPar, MapPar, OuterPar, ZipPar};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let num = |s: &String| s.parse::<usize>().unwrap_or_else(|_| fail(&format!("{s}: not a size")));
    let (out, source, class, spatial) = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [kind @ ("dot" | "tpchq6"), out, ..] if args.len() == 6 => {
            let v: Vec<usize> = args[2..].iter().map(num).collect();
            let par = ZipPar { ts: v[1], ip: v[2], op: v[3] };
            let (source, lowered) = if *kind == "dot" {
                let source = logos_dot_program(v[0]);
                let lowered = lower_dot_source(&source, par);
                (source, lowered)
            } else {
                let source = logos_tpchq6_program(v[0]);
                let lowered = lower_tpchq6_source(&source, par);
                (source, lowered)
            };
            let (_, class, spatial) = lowered.unwrap_or_else(|e| fail(&e));
            (out.to_string(), source, class, spatial)
        }
        ["outer", out, ..] if args.len() == 11 => {
            let v: Vec<usize> = args[2..].iter().map(num).collect();
            let source = logos_outer_program(v[0], v[1]);
            let p = OuterPar { ts1: v[2], ts2: v[3], ip: v[4], ip1: v[5], ip2: v[6], op1: v[7], op2: v[8] };
            let (_, _, class, spatial) = lower_outer_source(&source, p).unwrap_or_else(|e| fail(&e));
            (out.to_string(), source, class, spatial)
        }
        ["gda", out, ..] if args.len() == 9 => {
            let v: Vec<usize> = args[2..].iter().map(num).collect();
            let source = logos_gda_program(v[1], v[0]);
            let p = GdaPar { ts: v[2], op: v[3], mp1: v[4], mp2: v[5], ip: v[6] };
            let (_, _, class, spatial) = lower_gda_source(&source, p).unwrap_or_else(|e| fail(&e));
            (out.to_string(), source, class, spatial)
        }
        ["blackscholes", out, ..] if args.len() == 6 || args.len() == 7 => {
            let v: Vec<usize> = args[2..6].iter().map(num).collect();
            let source = logos_blackscholes_program(v[0]);
            let p = MapPar { ts: v[1], op: v[2], ip: v[3] };
            let (_, class, spatial) =
                lower_blackscholes_source(&source, p, args.get(6).map(String::as_str)).unwrap_or_else(|e| fail(&e));
            (out.to_string(), source, class, spatial)
        }
        ["kmeans", out, ..] if args.len() == 12 => {
            let v: Vec<usize> = args[2..].iter().map(num).collect();
            if v[1] < 2 {
                fail("D counts the app's count column: at least 2");
            }
            let source = logos_kmeans_program(v[9], v[0], v[1] - 1, v[8]);
            let p = KmeansPar { ts: v[2], ip: v[3], op: v[4], mp1: v[5], mp2: v[6], mp3: v[7] };
            let (_, _, _, _, class, spatial) = lower_kmeans_source(&source, p).unwrap_or_else(|e| fail(&e));
            (out.to_string(), source, class, spatial)
        }
        _ => fail("usage: emit_plasticine_spatial_twin dot|tpchq6 <out_dir> N ts ip op | outer <out_dir> M N ts1 ts2 ip ip1 ip2 op1 op2 | gda <out_dir> C R ts op mp1 mp2 ip | blackscholes <out_dir> N ts op ip [pir-args] | kmeans <out_dir> K D ts ip op mp1 mp2 mp3 I N"),
    };
    let dir = std::path::Path::new(&out);
    std::fs::create_dir_all(dir).unwrap_or_else(|e| fail(&format!("{out}: {e}")));
    for (ext, text) in [("scala", &spatial), ("lg", &source)] {
        let path = dir.join(format!("{class}.{ext}"));
        std::fs::write(&path, text).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
    }
    println!("{class}");
}

fn fail(msg: &str) -> ! {
    eprintln!("emit_plasticine_spatial_twin: {msg}");
    std::process::exit(1);
}
