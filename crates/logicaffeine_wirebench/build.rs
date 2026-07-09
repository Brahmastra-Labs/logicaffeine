//! Codegen for the toolchain-backed competitors. Each block is gated on its own
//! feature, and the codegen crates (`prost-build` / `capnpc`) are optional
//! build-dependencies pulled in only by those same features — so a default or
//! `--features arrow-bench` build runs this script as a no-op and never needs
//! `protoc` or the `capnp` compiler on PATH.

/// The `version` of the first `[[package]]` block in a `Cargo.lock` whose `name` matches.
fn lockfile_version(lock: &str, name: &str) -> Option<String> {
    for block in lock.split("[[package]]") {
        let mut is_match = false;
        let mut ver = None;
        for line in block.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("name = ") {
                is_match = rest.trim().trim_matches('"') == name;
            } else if let Some(rest) = l.strip_prefix("version = ") {
                ver = Some(rest.trim().trim_matches('"').to_string());
            }
        }
        if is_match {
            return ver;
        }
    }
    None
}

fn main() {
    // Capture resolved competitor versions from the workspace lockfile so the codec report's
    // metadata is honest about exactly what it benched (keys sanitized for the env-var name).
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    if let Ok(lock) = std::fs::read_to_string("../../Cargo.lock") {
        for want in ["bincode", "postcard", "rmp-serde", "ciborium", "serde_json", "arrow", "prost", "capnp"] {
            if let Some(ver) = lockfile_version(&lock, want) {
                let key = want.replace('-', "_");
                println!("cargo:rustc-env=WIREBENCH_VER_{key}={ver}");
            }
        }
    }

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
