//! Emit the checked-in streamed engine RTL (module `wpe_soc_stream`, WPE_STREAM_CAPS, ConfigBram backend) that the
//! silicon route flow (benchmarks/silicon/nexys_video_wpe_stream.v top) instantiates. Regenerate after any edit to
//! wpe_soc_template.v / the emitter so the checked-in netlist stays in lockstep with the RTL the ≡VM locks prove.
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_verilog_backend, MemBackend, WPE_STREAM_CAPS,
};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/wpe_soc_stream.v".to_string());
    // Optional NREG override (arg 2): the current WPE_STREAM caps carry NREG=512 (the transformer's register count),
    // which the open nextpnr placer cannot legalize its FF clusters for; a smaller NREG yields a placement-tractable
    // engine that still carries the SAME depth-banked heap, so a routed bitstream demonstrates the heap fix on silicon.
    let mut caps = WPE_STREAM_CAPS;
    if let Some(nreg) = std::env::args()
        .nth(2)
        .and_then(|s| s.parse::<usize>().ok())
    {
        caps[0] = nreg;
        caps[9] = caps[9].min(nreg); // MAXV cannot exceed NREG-scale scratch; keep it sane
    }
    let soc = emit_wprog_engine_soc_verilog_backend("stream", caps, MemBackend::ConfigBram);
    std::fs::write(&out, &soc).expect("write wpe_soc_stream.v");
    eprintln!("wrote {out} ({} bytes), NREG={}", soc.len(), caps[0]);
}
