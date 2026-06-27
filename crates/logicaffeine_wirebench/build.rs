//! Codegen for the toolchain-backed competitors. Each block is gated on its own
//! feature, and the codegen crates (`prost-build` / `capnpc`) are optional
//! build-dependencies pulled in only by those same features — so a default or
//! `--features arrow-bench` build runs this script as a no-op and never needs
//! `protoc` or the `capnp` compiler on PATH.

fn main() {
    #[cfg(feature = "protobuf")]
    {
        println!("cargo:rerun-if-changed=schemas/bench.proto");
        prost_build::compile_protos(&["schemas/bench.proto"], &["schemas"])
            .expect("protoc must be installed for --features protobuf (see bench-wire-vs-protocols.sh --heavy)");
    }
    #[cfg(feature = "capnproto")]
    {
        println!("cargo:rerun-if-changed=schemas/bench.capnp");
        capnpc::CompilerCommand::new()
            .file("schemas/bench.capnp")
            .run()
            .expect("the capnp compiler must be installed for --features capnproto");
    }
}
