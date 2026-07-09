//! v4/v7 generation (the version+variant STAMP) and canonical PARSE written IN LOGOS (uuid.lg) — pure
//! byte arithmetic over `Seq of Int`, no native kernel. `uuidV4`/`uuidV7` stamp the RFC 9562 bits in
//! Logos; `uuidParse` hex-decodes the canonical form in Logos. Byte-exact vs the reference on all tiers.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
const SRC: &str = "## Main\n\
    Let mutable r16 be a new Seq of Int.\n\
    Push 1 to r16.\n\
    Push 35 to r16.\n\
    Push 69 to r16.\n\
    Push 103 to r16.\n\
    Push 137 to r16.\n\
    Push 171 to r16.\n\
    Push 205 to r16.\n\
    Push 239 to r16.\n\
    Push 254 to r16.\n\
    Push 220 to r16.\n\
    Push 186 to r16.\n\
    Push 152 to r16.\n\
    Push 118 to r16.\n\
    Push 84 to r16.\n\
    Push 50 to r16.\n\
    Push 16 to r16.\n\
    Show uuidV4(r16).\n\
    Let mutable r10 be a new Seq of Int.\n\
    Push 1 to r10.\n\
    Push 35 to r10.\n\
    Push 69 to r10.\n\
    Push 103 to r10.\n\
    Push 137 to r10.\n\
    Push 171 to r10.\n\
    Push 205 to r10.\n\
    Push 239 to r10.\n\
    Push 254 to r10.\n\
    Push 220 to r10.\n\
    Show uuidV7(1718560703573, r10).\n\
    Show uuidParse(\"550e8400-e29b-41d4-a716-446655440000\").\n\
    Show uuidFormat(uuidParse(\"550e8400-e29b-41d4-a716-446655440000\")).";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_v4_v7_parse_written_in_logos() {
    let expected = "01234567-89ab-4def-bedc-ba9876543210\n\
                    01902233-4455-7123-8567-89abcdeffedc\n\
                    550e8400-e29b-41d4-a716-446655440000\n\
                    550e8400-e29b-41d4-a716-446655440000";
    // tree-walker == bytecode VM.
    common::assert_interpreter_output(SRC, expected);
    // AOT: the Logos stamp + hex-decode + hex-encode compile to native Rust and produce the identical ids.
    common::assert_output_lines(
        SRC,
        &[
            "01234567-89ab-4def-bedc-ba9876543210",
            "01902233-4455-7123-8567-89abcdeffedc",
            "550e8400-e29b-41d4-a716-446655440000",
            "550e8400-e29b-41d4-a716-446655440000",
        ],
    );
}
