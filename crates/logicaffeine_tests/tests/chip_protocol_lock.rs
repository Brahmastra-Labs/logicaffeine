//! chip_protocol_lock — the chips' UART protocol has one definition, `codegen_sva::chip_protocol`.
//!
//! Every byte the whole-program engine and its hosts exchange (the program image's markers, the show, call and
//! reply records, the result frame, the value tags and the host-call numbers) is a constant of that module. This
//! lock holds the table to one meaning per byte, holds the framing to a lossless round trip that reports a cut-off
//! record as short and never as a value, and holds `scripts/logos-chip.py` to reading the table from `chip_host
//! --table` instead of restating it: a host with its own copy of a wire format drifts from the chip, still runs,
//! and prints a plausible wrong line.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::codegen_sva::chip_protocol::{
    fill_verilog, table_json, ChipValue, FrameError, HostCall, Record, ResultFrame, FRAME_END,
    FRAME_END_CALL_STACK, FRAME_END_HEAP, FRAME_SYNC, IMAGE_GO, IMAGE_HEAP, REC_CALL, REC_REPLY,
    REC_SHOW, TAGS, TAG_BOOL, TAG_FLOAT, TAG_INT, TAG_TEXT, TAG_WORD32,
};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_testbench, emit_wprog_engine_soc_verilog, program_main_fabric,
    wprog_config_image_unchecked,
};

#[path = "common/mod.rs"]
mod common;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// One of each value shape the protocol carries, at the edges of its range.
fn sample_values() -> Vec<ChipValue> {
    vec![
        ChipValue::Int(0),
        ChipValue::Int(-7),
        ChipValue::Int(i64::MIN),
        ChipValue::Int(i64::MAX),
        ChipValue::Float(3.75f64.to_bits()),
        ChipValue::Float((-0.0f64).to_bits()),
        ChipValue::Float(f64::NAN.to_bits() | 0x5),
        ChipValue::Text(String::new()),
        ChipValue::Text("hi fabric".to_string()),
        ChipValue::Text("λ→∞ 日本".to_string()),
        ChipValue::Word32(0),
        ChipValue::Word32(u32::MAX),
        ChipValue::Bool(false),
        ChipValue::Bool(true),
    ]
}

/// A host call is a listed variant, and every variant is listed: naming each one here without a wildcard makes a
/// new variant a compile error until it is added to `HostCall::ALL` and to this count.
fn variant_count() -> usize {
    let listed = |c: HostCall| match c {
        HostCall::Listen
        | HostCall::Connect
        | HostCall::Send
        | HostCall::Await
        | HostCall::Stream
        | HostCall::AwaitStream
        | HostCall::Sync => (),
    };
    HostCall::ALL.iter().for_each(|c| listed(*c));
    7
}

#[test]
fn every_protocol_byte_has_one_meaning() {
    assert_ne!(
        IMAGE_HEAP, IMAGE_GO,
        "the loader tells the heap pre-load from the start of the run by this byte"
    );
    let chip_to_host = [REC_SHOW, REC_CALL, FRAME_SYNC];
    assert_eq!(
        chip_to_host.iter().collect::<HashSet<_>>().len(),
        chip_to_host.len(),
        "a host reads what the chip sent by its first byte, so the show and call records and the frame differ there"
    );
    assert!(
        ![REC_SHOW, REC_CALL, FRAME_SYNC].contains(&REC_REPLY),
        "a reply is never mistaken for the chip's own record"
    );
    let ends = [FRAME_END, FRAME_END_CALL_STACK, FRAME_END_HEAP];
    assert_eq!(
        ends.iter().collect::<HashSet<_>>().len(),
        3,
        "each end of a run has its own end byte"
    );
    assert_eq!(
        TAGS,
        [TAG_INT, TAG_FLOAT, TAG_TEXT, TAG_WORD32, TAG_BOOL],
        "TAGS lists every tag, in code order"
    );
    assert_eq!(
        TAGS.iter().collect::<HashSet<_>>().len(),
        TAGS.len(),
        "each value tag names one type"
    );

    assert_eq!(
        HostCall::ALL.len(),
        variant_count(),
        "HostCall::ALL lists every host call"
    );
    let codes: HashSet<u8> = HostCall::ALL.iter().map(|c| c.code()).collect();
    assert_eq!(
        codes.len(),
        HostCall::ALL.len(),
        "each host call has its own number"
    );
    assert!(
        !codes.contains(&0),
        "no host call is 0, the byte a zeroed record would carry"
    );
    let names: HashSet<&str> = HostCall::ALL.iter().map(|c| c.name()).collect();
    assert_eq!(
        names.len(),
        HostCall::ALL.len(),
        "each host call has its own name"
    );
    for c in HostCall::ALL {
        assert_eq!(
            HostCall::from_code(c.code()),
            Some(c),
            "{c:?}'s number names it"
        );
    }
    assert_eq!(HostCall::from_code(0), None);
}

#[test]
fn the_json_table_is_the_constants() {
    let t: serde_json::Value = serde_json::from_str(&table_json()).expect("table_json is JSON");
    let byte = |path: &[&str]| -> u64 {
        path.iter()
            .fold(&t, |v, k| &v[*k])
            .as_u64()
            .unwrap_or_else(|| panic!("the table has no {path:?}"))
    };
    assert_eq!(byte(&["image", "heap"]), u64::from(IMAGE_HEAP));
    assert_eq!(byte(&["image", "go"]), u64::from(IMAGE_GO));
    assert_eq!(byte(&["records", "show"]), u64::from(REC_SHOW));
    assert_eq!(byte(&["records", "call"]), u64::from(REC_CALL));
    assert_eq!(byte(&["records", "reply"]), u64::from(REC_REPLY));
    assert_eq!(byte(&["frame", "sync"]), u64::from(FRAME_SYNC));
    assert_eq!(byte(&["frame", "end"]), u64::from(FRAME_END));
    assert_eq!(
        byte(&["frame", "end_call_stack"]),
        u64::from(FRAME_END_CALL_STACK)
    );
    assert_eq!(byte(&["frame", "end_heap"]), u64::from(FRAME_END_HEAP));
    for (name, tag) in [
        ("int", TAG_INT),
        ("float", TAG_FLOAT),
        ("text", TAG_TEXT),
        ("word32", TAG_WORD32),
        ("bool", TAG_BOOL),
    ] {
        assert_eq!(byte(&["tags", name]), u64::from(tag), "tag {name}");
    }
    assert_eq!(
        t["tags"].as_object().map(|o| o.len()),
        Some(TAGS.len()),
        "the table lists every tag and no other"
    );
    for c in HostCall::ALL {
        assert_eq!(
            byte(&["calls", c.name()]),
            u64::from(c.code()),
            "host call {}",
            c.name()
        );
    }
    assert_eq!(
        t["calls"].as_object().map(|o| o.len()),
        Some(HostCall::ALL.len()),
        "the table lists every call and no other"
    );
}

#[test]
fn a_value_and_a_record_round_trip_and_a_cut_record_is_short() {
    for v in sample_values() {
        let mut bytes = Vec::new();
        v.encode(&mut bytes);
        assert_eq!(bytes[0], v.tag(), "{v:?} leads with its tag");
        let mut longer = bytes.clone();
        longer.extend_from_slice(&[0xEE, 0xEE]);
        assert_eq!(
            ChipValue::decode(&longer),
            Ok((v.clone(), bytes.len())),
            "{v:?} decodes to itself and takes only its bytes"
        );
        for cut in 0..bytes.len() {
            assert_eq!(
                ChipValue::decode(&bytes[..cut]),
                Err(FrameError::Short),
                "{v:?} cut to {cut} bytes is short"
            );
        }
    }
    let records = vec![
        Record::Show(ChipValue::Int(-42)),
        Record::Show(ChipValue::Text("ok".to_string())),
        Record::Reply(ChipValue::Bool(true)),
        Record::Reply(ChipValue::Text(String::new())),
        Record::Call {
            call: HostCall::Listen,
            args: vec![ChipValue::Text("me".to_string())],
        },
        Record::Call {
            call: HostCall::Send,
            args: vec![ChipValue::Text("me".to_string()), ChipValue::Int(7)],
        },
        Record::Call {
            call: HostCall::Await,
            args: vec![],
        },
        Record::Call {
            call: HostCall::Sync,
            args: sample_values(),
        },
    ];
    for r in records {
        let bytes = r.encode();
        assert_eq!(
            Record::decode(&bytes),
            Ok((r.clone(), bytes.len())),
            "{r:?} round-trips"
        );
        for cut in 0..bytes.len() {
            assert_eq!(
                Record::decode(&bytes[..cut]),
                Err(FrameError::Short),
                "{r:?} cut to {cut} bytes is short"
            );
        }
    }
    for end in [FRAME_END, FRAME_END_CALL_STACK, FRAME_END_HEAP] {
        let f = ResultFrame {
            result: -(1 << 40),
            cycles: 0xFEDC_BA98,
            end,
        };
        let bytes = f.encode();
        assert_eq!(
            ResultFrame::decode(&bytes),
            Ok((f, 14)),
            "the frame ending {end:#04x} round-trips"
        );
        for cut in 0..bytes.len() {
            assert_eq!(
                ResultFrame::decode(&bytes[..cut]),
                Err(FrameError::Short),
                "a frame cut to {cut} bytes is short"
            );
        }
    }
}

#[test]
fn bytes_that_are_not_a_record_are_malformed_never_a_value() {
    let malformed =
        |r: Result<(Record, usize), FrameError>| matches!(r, Err(FrameError::Malformed(_)));
    assert!(malformed(Record::decode(&[0x00])), "an unknown marker");
    assert!(
        malformed(Record::decode(&[REC_SHOW, 0xEE, 0, 0, 0, 0, 0, 0, 0, 0])),
        "an unknown tag"
    );
    assert!(
        malformed(Record::decode(&[REC_CALL, 0xEE, 0])),
        "an unknown host call"
    );
    let mut bool2 = vec![REC_SHOW, TAG_BOOL];
    bool2.extend_from_slice(&2u64.to_be_bytes());
    assert!(
        malformed(Record::decode(&bool2)),
        "a Bool that is neither 0 nor 1"
    );
    let mut wide = vec![REC_SHOW, TAG_WORD32];
    wide.extend_from_slice(&(1u64 << 32).to_be_bytes());
    assert!(malformed(Record::decode(&wide)), "a Word32 past 2^32 - 1");
    let mut bad_utf8 = vec![REC_SHOW, TAG_TEXT];
    bad_utf8.extend_from_slice(&2u32.to_be_bytes());
    bad_utf8.extend_from_slice(&[0xC3, 0x28]);
    assert!(
        malformed(Record::decode(&bad_utf8)),
        "a Text that is not UTF-8"
    );
    let mut frame = ResultFrame {
        result: 1,
        cycles: 2,
        end: FRAME_END,
    }
    .encode();
    frame[13] = 0x00;
    assert!(
        matches!(ResultFrame::decode(&frame), Err(FrameError::Malformed(_))),
        "a frame with an unknown end"
    );
    assert!(
        matches!(
            ResultFrame::decode(&[REC_SHOW]),
            Err(FrameError::Malformed(_))
        ),
        "a frame without its sync"
    );
}

/// The protocol bytes a Python host would restate, and the Python spellings that restate them.
const RESTATED: &[&str] = &[
    "0xA5", "0xA7", "0xA8", "0xA9", "0x5A", "0xE1", "0xE2", "0x48", "\\xA5", "\\xA7", "\\xA8",
    "\\xA9", "\\x5A", "\\xE1", "\\xE2",
];

#[test]
fn the_python_host_restates_no_protocol_byte() {
    let path = root().join("scripts/logos-chip.py");
    let src = std::fs::read_to_string(&path).expect("read scripts/logos-chip.py");
    let restated: Vec<String> = src
        .lines()
        .enumerate()
        .filter(|(_, l)| {
            let upper = l.to_uppercase();
            RESTATED.iter().any(|r| upper.contains(&r.to_uppercase()))
                || l.trim_start().starts_with("SHOW_")
        })
        .map(|(i, l)| format!("  {}: {}", i + 1, l.trim()))
        .collect();
    assert!(
        restated.is_empty(),
        "scripts/logos-chip.py restates the chip protocol instead of reading it from `chip_host --table`:\n{}",
        restated.join("\n")
    );
    assert!(
        src.contains("\"chip_host\"") && src.contains("\"--table\""),
        "scripts/logos-chip.py reads the protocol table from the chip_host example"
    );
}

/// Build `chip_host` once, as logos-chip.py builds its compiler examples.
fn chip_host_built() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let r = Command::new("cargo")
            .args([
                "build",
                "-q",
                "-p",
                "logicaffeine-compile",
                "--example",
                "chip_host",
            ])
            .current_dir(root())
            .output()
            .expect("launch cargo build");
        assert!(
            r.status.success(),
            "building chip_host failed:\n{}",
            String::from_utf8_lossy(&r.stderr)
        );
    });
}

/// How the VM prints a shown value.
fn rendered(v: &ChipValue) -> String {
    match v {
        ChipValue::Int(i) => i.to_string(),
        ChipValue::Float(bits) => logicaffeine_data::fmt::fmt_f64(f64::from_bits(*bits)),
        ChipValue::Text(s) => s.clone(),
        ChipValue::Word32(w) => w.to_string(),
        ChipValue::Bool(b) => b.to_string(),
    }
}

#[test]
fn the_python_host_decodes_what_the_protocol_encodes() {
    if !common::require_tool("python3") {
        return;
    }
    chip_host_built();
    let dir = std::env::temp_dir().join(format!("chip_protocol_lock_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let decode = |bytes: &[u8], name: &str| -> serde_json::Value {
        let file = dir.join(name);
        std::fs::write(&file, bytes).expect("write the reply");
        let o = Command::new("python3")
            .arg("scripts/logos-chip.py")
            .args(["decode", file.to_str().expect("utf-8 path"), "--no-build"])
            .current_dir(root())
            .output()
            .expect("launch scripts/logos-chip.py");
        assert!(
            o.status.success(),
            "`logos-chip.py decode` failed on {name}:\n{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        serde_json::from_slice(&o.stdout)
            .unwrap_or_else(|e| panic!("`logos-chip.py decode` printed no JSON ({e})"))
    };

    let shows: Vec<ChipValue> = sample_values();
    let mut reply = vec![0x00];
    for v in &shows {
        reply.extend(Record::Show(v.clone()).encode());
    }
    reply.extend(
        ResultFrame {
            result: -42,
            cycles: 12345,
            end: FRAME_END,
        }
        .encode(),
    );
    let got = decode(&reply, "finished.bin");
    let want: Vec<String> = shows.iter().map(rendered).collect();
    assert_eq!(
        got["shows"],
        serde_json::json!(want),
        "the host renders every shown value as the VM prints it"
    );
    assert_eq!(
        (
            got["result"].clone(),
            got["cycles"].clone(),
            got["status"].clone()
        ),
        (
            serde_json::json!(-42),
            serde_json::json!(12345),
            serde_json::json!("ok")
        )
    );

    for (end, status) in [(FRAME_END_CALL_STACK, "overflow"), (FRAME_END_HEAP, "heap")] {
        let got = decode(
            &ResultFrame {
                result: 99,
                cycles: 500,
                end,
            }
            .encode(),
            &format!("end_{end:02x}.bin"),
        );
        assert_eq!(
            got["status"],
            serde_json::json!(status),
            "the end byte {end:#04x} is `{status}`, not an answer"
        );
        assert_eq!(
            got["result"],
            serde_json::Value::Null,
            "an overflowed run's result bytes are not an answer"
        );
    }
    let cut = Record::Show(ChipValue::Text("hi fabric".to_string())).encode();
    assert_eq!(
        decode(&cut[..cut.len() - 3], "cut.bin")["status"],
        serde_json::json!("short"),
        "a cut record is short"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Every byte of the protocol, as the engine's Verilog could spell it.
const PROTOCOL_BYTES: [u8; 9] = [
    IMAGE_HEAP,
    IMAGE_GO,
    REC_SHOW,
    REC_CALL,
    REC_REPLY,
    FRAME_SYNC,
    FRAME_END,
    FRAME_END_CALL_STACK,
    FRAME_END_HEAP,
];

/// Whether `line` spells `byte` as a Verilog literal (`8'hA7`) or a quoted hex byte (`0xA7`), the two hex digits
/// standing alone.
fn spells_byte(line: &str, byte: u8) -> bool {
    let upper = line.to_uppercase();
    [format!("8'H{byte:02X}"), format!("0X{byte:02X}")]
        .iter()
        .any(|lit| {
            upper.match_indices(lit.as_str()).any(|(at, m)| {
                let next = upper[at + m.len()..].chars().next();
                !next.is_some_and(|c| c.is_ascii_hexdigit() || c == '_')
            })
        })
}

#[test]
fn the_engine_templates_take_every_protocol_byte_from_the_table() {
    for (name, src) in [
        (
            "wpe_soc_template.v",
            include_str!("../../logicaffeine_compile/src/codegen_sva/wpe_soc_template.v"),
        ),
        (
            "tb_wpe_soc_template.v",
            include_str!("../../logicaffeine_compile/src/codegen_sva/tb_wpe_soc_template.v"),
        ),
    ] {
        let raw: Vec<String> = src
            .lines()
            .enumerate()
            .filter(|(_, l)| PROTOCOL_BYTES.iter().any(|b| spells_byte(l, *b)))
            .map(|(i, l)| format!("  {}: {}", i + 1, l.trim()))
            .collect();
        assert!(
            raw.is_empty(),
            "{name} spells a protocol byte itself instead of naming it (`__V_<NAME>__` in code, `__X_<NAME>__` in a \
             comment), so a change to chip_protocol would leave it behind:\n{}",
            raw.join("\n")
        );
        let filled = fill_verilog(src);
        assert!(
            !filled.contains("__V_") && !filled.contains("__X_"),
            "{name} fills to no leftover placeholder"
        );
    }
    let unknown = std::panic::catch_unwind(|| fill_verilog("tx_data <= __V_NO_SUCH_BYTE__;"))
        .expect_err("must refuse");
    let unknown = unknown
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(
        unknown.contains("__V_NO_SUCH_BYTE__"),
        "a placeholder that names no protocol byte stops the emitter, naming it: {unknown}"
    );

    let engine = emit_wprog_engine_soc_verilog("proto");
    for (site, what) in [
        (
            format!("rx_data=={}", literal(IMAGE_GO)),
            "the loader starts the run on the image's GO byte",
        ),
        (
            format!("rx_data=={}", literal(IMAGE_HEAP)),
            "the loader takes the heap pre-load on its marker",
        ),
        (
            format!("tx_data <= {}", literal(REC_SHOW)),
            "a Show streams the show record's marker",
        ),
        (
            format!("tx_data<={}", literal(FRAME_SYNC)),
            "the result frame starts with its sync byte",
        ),
        (
            format!(
                "sp_ovf ? {} : heap_ovf ? {} : {}",
                literal(FRAME_END_CALL_STACK),
                literal(FRAME_END_HEAP),
                literal(FRAME_END)
            ),
            "the result frame ends with the end byte of the way the run ended",
        ),
    ] {
        assert!(
            engine.contains(&site),
            "{what}: the emitted engine has no `{site}`"
        );
    }
    let tb = emit_wprog_engine_soc_testbench("proto", 1, 2, 16);
    for site in [
        format!("sync == {}", literal(REC_SHOW)),
        format!("sync!=={}", literal(FRAME_SYNC)),
        format!("endb!=={}", literal(FRAME_END)),
    ] {
        assert!(
            tb.contains(&site),
            "the host testbench reads the frame by the table: no `{site}`"
        );
    }
}

/// A byte as the engine's Verilog spells it.
fn literal(byte: u8) -> String {
    format!("8'h{byte:02X}")
}

#[test]
fn the_program_image_is_framed_by_the_table() {
    let image = wprog_config_image_unchecked(&program_main_fabric("## Main\nShow 1.\n"));
    let plain = image.wire_bytes(&[]);
    assert_eq!(
        plain.last(),
        Some(&IMAGE_GO),
        "the program image ends with the go byte, which starts the run"
    );
    let preloaded = image.wire_bytes_preloaded(&[], &[7, -8]);
    let body = &plain[..plain.len() - 1];
    assert_eq!(
        &preloaded[..body.len()],
        body,
        "a heap pre-load leaves the image before it as it was"
    );
    assert_eq!(
        preloaded[body.len()],
        IMAGE_HEAP,
        "the heap pre-load follows its marker"
    );
    assert_eq!(
        &preloaded[body.len() + 1..body.len() + 3],
        &2u16.to_be_bytes(),
        "then its word count"
    );
    assert_eq!(
        preloaded.len(),
        body.len() + 3 + 2 * 8 + 1,
        "then its words, 8 bytes each"
    );
    assert_eq!(
        preloaded.last(),
        Some(&IMAGE_GO),
        "and the image still ends with the go byte"
    );
}
