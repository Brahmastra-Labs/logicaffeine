//! How much of each mesh a uniform rectangular grid can actually reach.
//!
//! `tile.rs` requires `ny = mt/psm` and `nx = nt/psn` with `psm | mt` and `psn | nt`, and then
//! `fits_mesh = ny <= sites.y && nx <= sites.x`. So the reachable grid dimensions are the DIVISORS of the
//! tile count that are no larger than the mesh dimension. A 2048x2048 output is 64x64 tiles, whose
//! divisors jump 8 -> 16 — straddling both 10 and 13 — so 8x8 = 64 of Blackhole's 130 sites is the most
//! any uniform plan can use, and 66 cores are unreachable for that shape.
//!
//! Before proposing non-uniform decomposition as a feature, this measures what it would be worth: over a
//! sweep of shapes, how many sites the best uniform grid reaches against the mesh it runs on.
//!
//!   cargo run -q --release -p logicaffeine-compile --features codegen --example probe_mesh_utilization

use logicaffeine_compile::spatial::target::SpatialTarget;

fn divisors(n: usize) -> Vec<usize> {
    let mut d = Vec::new();
    let mut i = 1;
    while i * i <= n {
        if n % i == 0 {
            d.push(i);
            if i != n / i {
                d.push(n / i);
            }
        }
        i += 1;
    }
    d.sort_unstable();
    d
}

/// The largest uniform grid that fits the mesh: the biggest divisor of each tile count within its bound.
fn best_uniform_grid(mt: usize, nt: usize, mesh_y: usize, mesh_x: usize) -> (usize, usize) {
    let ny = divisors(mt)
        .into_iter()
        .filter(|d| *d <= mesh_y)
        .max()
        .unwrap_or(1);
    let nx = divisors(nt)
        .into_iter()
        .filter(|d| *d <= mesh_x)
        .max()
        .unwrap_or(1);
    (ny, nx)
}

fn main() {
    let targets = [
        ("wormhole_b0", SpatialTarget::wormhole()),
        ("wormhole_ttsim", SpatialTarget::wormhole_ttsim()),
        ("blackhole", SpatialTarget::blackhole()),
    ];

    // Output extents in ELEMENTS, over sizes a real workload uses. Tiles are 32x32.
    let dims = [
        128usize, 256, 320, 384, 512, 640, 768, 896, 1024, 1280, 1536, 1792, 2048, 2560, 3072, 4096,
    ];

    for (name, t) in &targets {
        let (my, mx) = (t.sites.y, t.sites.x);
        let mesh = my * mx;
        let mut used_total = 0usize;
        let mut n = 0usize;
        let mut full = 0usize;
        let mut worst: Option<(usize, usize, usize)> = None;
        let mut buckets = [0usize; 5]; // <25%, <50%, <75%, <100%, ==100%

        for &m in &dims {
            for &nn in &dims {
                let (mt, nt) = (m / 32, nn / 32);
                if mt == 0 || nt == 0 {
                    continue;
                }
                let (gy, gx) = best_uniform_grid(mt, nt, my, mx);
                let used = gy * gx;
                let pct = used as f64 / mesh as f64;
                n += 1;
                used_total += used;
                if used == mesh {
                    full += 1;
                }
                if worst.map(|(u, _, _)| used < u).unwrap_or(true) {
                    worst = Some((used, m, nn));
                }
                let b = if pct < 0.25 {
                    0
                } else if pct < 0.5 {
                    1
                } else if pct < 0.75 {
                    2
                } else if pct < 1.0 {
                    3
                } else {
                    4
                };
                buckets[b] += 1;
            }
        }
        let (wu, wm, wn) = worst.unwrap();
        println!("{name:16} mesh {my}x{mx} = {mesh:3} sites over {n} shapes");
        println!(
            "{:16}   mean utilization {:.1}%   fully used in {}/{} shapes",
            "",
            100.0 * used_total as f64 / (n * mesh) as f64,
            full,
            n
        );
        println!(
            "{:16}   <25%: {}  <50%: {}  <75%: {}  <100%: {}  100%: {}",
            "", buckets[0], buckets[1], buckets[2], buckets[3], buckets[4]
        );
        println!("{:16}   worst: {wm}x{wn} reaches {wu}/{mesh} sites", "");
        // What a non-uniform grid would reach: any dimension up to the mesh, tiles spread unevenly.
        let mut nonuni = 0usize;
        for &m in &dims {
            for &nn in &dims {
                let (mt, nt) = (m / 32, nn / 32);
                if mt == 0 || nt == 0 {
                    continue;
                }
                nonuni += mt.min(my) * nt.min(mx);
            }
        }
        println!("{:16}   a non-uniform grid would reach {:.1}% (tiles spread unevenly, any extent <= mesh)\n",
                 "", 100.0 * nonuni as f64 / (n * mesh) as f64);
    }
}
