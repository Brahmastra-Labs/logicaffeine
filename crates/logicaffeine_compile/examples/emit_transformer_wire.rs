//! Emit the whole STREAMED transformer's over-the-wire byte stream for the on-chip ConfigBram engine:
//! the flat config image + the pre-loaded weight blob (0x48 heap-preload segment) + no params + the 0x5A GO.
//! The on-chip loader (wpe_soc_stream) fills its config/heap memories and runs the whole autoregressive
//! generation, returning the packed result 80113 = "mat". Fed to scripts/nexys-wpe-host.py over the DPTI FIFO.
use logicaffeine_compile::codegen_sva::fabric_rtl::{program_cfg_fabric, with_seq_reserve};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config_counted_heap_preloaded, serialize_program, wprog_config_fits_stream_engine,
    wprog_config_image_unchecked,
};
fn main() {
    let src = std::fs::read_to_string("/tmp/tffab_stream.lg")
        .expect("gen --fabric --stream > /tmp/tffab_stream.lg first");
    let blob: Vec<i64> =
        serde_json::from_str(&std::fs::read_to_string("/tmp/tf_blob.json").unwrap()).unwrap();
    let (wire, model) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(&src, "generate"));
        wprog_config_fits_stream_engine(&cfg)
            .expect("streamed transformer must fit the WPE_STREAM engine caps");
        let im = wprog_config_image_unchecked(&cfg);
        let wire = im.wire_bytes_preloaded(&[], &blob);
        let model = eval_wprog_config_counted_heap_preloaded(&cfg, &[], &blob).0;
        (wire, model)
    });
    std::fs::write("/tmp/transformer.wire", &wire).expect("write wire");
    eprintln!(
        "wrote /tmp/transformer.wire: {} bytes (config image + {} pre-loaded weight words + GO)",
        wire.len(),
        blob.len()
    );
    eprintln!("engine model result = {model} (want 80113 = \"mat\"); flash then: python3 scripts/nexys-wpe-host.py /tmp/transformer.wire --port /dev/ttyUSB1 --expect 80113");
    assert_eq!(model, 80113, "model must generate mat before we flash");
}
