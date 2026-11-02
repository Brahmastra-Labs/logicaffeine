//! Emit the streamed-transformer engine RTL (ConfigBram backend, WPE_STREAM caps) for a yosys FF/BRAM/LUT budget
//! check. ConfigBram maps the config image (`cell_*`/`blk_*`) to block RAM as well as the state plane, so the
//! whole 158-block streamed transformer fits the XC7A200T's FF budget.
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_verilog_backend, MemBackend, WPE_STREAM_CAPS,
};
fn main() {
    print!(
        "{}",
        emit_wprog_engine_soc_verilog_backend("stream", WPE_STREAM_CAPS, MemBackend::ConfigBram)
    );
}
