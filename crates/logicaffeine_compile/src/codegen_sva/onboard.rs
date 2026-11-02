//! The compiler as the board's own CPU runs it: LOGOS source → register bytecode → the fabric engine's wire
//! image, through exactly the path the laptop's board hosts take ([`super::engine_build`]), with the VM's golden
//! proven against the engine model before a byte of the image exists. The board runs a
//! `riscv64gc-unknown-linux-musl` build of this same code; `onboard_compiler_lock` holds its every answer
//! byte-identical to the host build's.

use super::chip_protocol::{
    ChipValue, FrameError, Record, ResultFrame, FRAME_END, FRAME_END_CALL_STACK, FRAME_SYNC, REC_CALL, REC_SHOW,
    TAG_BOOL, TAG_FLOAT, TAG_INT, TAG_TEXT, TAG_WORD32,
};
use super::engine_build::{fits_caps, lower, lower_main, Lifter, Profile};
use super::wprog_engine_rtl::{
    render_show, wprog_config_image_unchecked, WShow, WPE_HW_DEPTH, WPE_HW_HEAP, WPE_HW_MAXARG, WPE_HW_MAXBLK,
    WPE_HW_MAXCELL, WPE_HW_MAXK, WPE_HW_MAXLIV, WPE_HW_MAXWR, WPE_HW_NREG,
};
use crate::vm::{binisa, Compiler};

/// What the engine is asked to run: the program's `## Main`, or one of its functions on arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    Main,
    Call { func: String, args: Vec<i64> },
}

/// The VM's answer the engine must reproduce: a Main program's output, one line per `Show`, or a function's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Golden {
    Lines(Vec<String>),
    Answer(i64),
}

/// One program compiled for the fabric engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compiled {
    /// The register bytecode the lifter lowered, as the portable binary ISA frame ([`binisa::encode_program`]).
    pub bytecode: Vec<u8>,
    /// The bytes the engine loads: the lowered program's image followed by its arguments.
    pub wire: Vec<u8>,
    /// Which lowering produced the image.
    pub lifter: Lifter,
    /// What the VM answers, which the engine model was proven to answer before the image was emitted.
    pub golden: Golden,
}

/// The register bytecode of `src`, compiled the way the fabric lifters compile it, as a binary ISA frame.
pub fn register_bytecode(src: &str) -> Result<Vec<u8>, String> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed?;
        let program = Compiler::compile(stmts, interner)?;
        Ok(binisa::encode_program(&program, interner))
    })
}

/// Compile `src` for the engine: lower `run` and prove it ≡ VM ([`lower_main`] / [`lower`]), check the image
/// against the engine `profile` and the flashed image's `caps` (`NAME=N,…`, DEPTH and HEAP measured on the exact
/// arguments), then emit the wire image and the bytecode it was lowered from. A refusal is the sentence a person
/// reads on the screen.
pub fn compile(src: &str, run: &Run, profile: Profile, caps: &str) -> Result<Compiled, String> {
    let (cfg, lifter, golden, params) = match run {
        Run::Main => {
            let (cfg, lines) = lower_main(src)?;
            (cfg, Lifter::Main, Golden::Lines(lines), Vec::new())
        }
        Run::Call { func, args } => {
            let (cfg, lifter, answer) = lower(src, func, args)?;
            (cfg, lifter, Golden::Answer(answer), args.clone())
        }
    };
    profile
        .fits(&cfg)
        .map_err(|e| format!("does not fit the {} engine: {e}", profile.name()))?;
    fits_caps(&cfg, &params, caps).map_err(|e| format!("does not fit the flashed image: {e}"))?;
    let wire = wprog_config_image_unchecked(&cfg).wire_bytes(&params);
    let bytecode = register_bytecode(src)?;
    Ok(Compiled {
        bytecode,
        wire,
        lifter,
        golden,
    })
}

/// One program of a library: its title, its source file, and what the engine runs of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    pub program: String,
    pub run: Run,
}

/// The entries of a library file (`benchmarks/silicon/programs/terminal_library.txt`'s format): one per line,
/// `title | program file | function | arguments`, a `## Main` program with neither function nor arguments; `#`
/// starts a comment line.
pub fn library(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('|').map(str::trim).collect();
        let [title, program, func, args] = fields[..] else {
            return Err(format!("library line {}: want `title | program | function | arguments`", n + 1));
        };
        let run = if func.is_empty() {
            if !args.is_empty() {
                return Err(format!("library line {}: a Main program takes no arguments", n + 1));
            }
            Run::Main
        } else {
            let args = args
                .split(',')
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(|a| a.parse::<i64>().map_err(|_| format!("library line {}: bad argument {a:?}", n + 1)))
                .collect::<Result<Vec<_>, _>>()?;
            Run::Call { func: func.to_string(), args }
        };
        entries.push(Entry { title: title.to_string(), program: program.to_string(), run });
    }
    Ok(entries)
}

/// The caps of Board B's hardware engine, written the way a build recipe writes them.
pub fn hw_caps() -> String {
    format!(
        "NREG={WPE_HW_NREG},MAXBLK={WPE_HW_MAXBLK},MAXCELL={WPE_HW_MAXCELL},MAXWR={WPE_HW_MAXWR},\
         MAXARG={WPE_HW_MAXARG},MAXLIV={WPE_HW_MAXLIV},MAXK={WPE_HW_MAXK},HEAP={WPE_HW_HEAP},DEPTH={WPE_HW_DEPTH}"
    )
}

/// A compile's report, one `key=value` line per fact: `lifter=`, then `golden=` (a function's value) or
/// `golden_json=` (a Main program's lines), then `bytecode=` and `wire=` in lowercase hex. A refusal is the single
/// line `error=` followed by the sentence.
pub fn report(result: &Result<Compiled, String>) -> String {
    match result {
        Ok(c) => {
            let golden = match &c.golden {
                Golden::Answer(v) => format!("golden={v}"),
                Golden::Lines(lines) => format!(
                    "golden_json={}",
                    serde_json::to_string(lines).expect("lines serialize")
                ),
            };
            format!(
                "lifter={}\n{golden}\nbytecode={}\nwire={}\n",
                c.lifter.name(),
                hex(&c.bytecode),
                hex(&c.wire)
            )
        }
        Err(e) => format!("error={}\n", e.replace('\n', " ")),
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// How an engine run ended, read from its result frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The program finished: its result and the engine cycles it took.
    Finished { result: i64, cycles: u32 },
    /// The call stack overflowed the engine's DEPTH: the result bytes are no answer.
    CallStack { cycles: u32 },
    /// The heap outgrew the engine's HEAP: no answer either.
    Heap { cycles: u32 },
}

/// The engine's reply so far: every `Show` it sent, each rendered as the VM prints it, and how the run ended once
/// the result frame is whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub shows: Vec<String>,
    pub ended: Option<Ended>,
}

/// One shown value as the VM prints it — through the engine model's own rendering, so the board and the laptop can
/// never print a value two ways.
pub fn shown(v: &ChipValue) -> String {
    let (tag, value, heap) = match v {
        ChipValue::Int(i) => (TAG_INT, *i, Vec::new()),
        ChipValue::Float(bits) => (TAG_FLOAT, *bits as i64, Vec::new()),
        ChipValue::Word32(w) => (TAG_WORD32, i64::from(*w), Vec::new()),
        ChipValue::Bool(b) => (TAG_BOOL, i64::from(*b), Vec::new()),
        ChipValue::Text(t) => {
            let mut heap = vec![t.len() as i64];
            heap.extend(t.bytes().map(i64::from));
            (TAG_TEXT, 0, heap)
        }
    };
    render_show(WShow { tag, value }, &heap)
}

/// Decode what the engine has sent of one run — its bytes arrive in pieces, so a reply cut short is not an error,
/// only not over yet. Bytes before the first record or frame (the engine's power-on byte) are skipped. A program
/// that calls the host is refused by name: the board answers no host calls.
pub fn engine_reply(bytes: &[u8]) -> Result<Reply, String> {
    let mut reply = Reply { shows: Vec::new(), ended: None };
    let mut at = bytes.iter().position(|&b| b == REC_SHOW || b == REC_CALL || b == FRAME_SYNC).unwrap_or(bytes.len());
    while at < bytes.len() && bytes[at] != FRAME_SYNC {
        match Record::decode(&bytes[at..]) {
            Ok((Record::Show(v), n)) => {
                reply.shows.push(shown(&v));
                at += n;
            }
            Ok((Record::Call { call, .. }, _)) => {
                return Err(format!("the program calls the host ({}), which the board does not answer", call.name()))
            }
            Ok((Record::Reply(_), _)) => return Err("the engine sent a reply record".to_string()),
            Err(FrameError::Short) => return Ok(reply),
            Err(FrameError::Malformed(why)) => return Err(format!("the engine sent no record: {why}")),
        }
    }
    if at < bytes.len() {
        match ResultFrame::decode(&bytes[at..]) {
            Ok((f, _)) => {
                reply.ended = Some(match f.end {
                    FRAME_END => Ended::Finished { result: f.result, cycles: f.cycles },
                    FRAME_END_CALL_STACK => Ended::CallStack { cycles: f.cycles },
                    _ => Ended::Heap { cycles: f.cycles },
                })
            }
            Err(FrameError::Short) => {}
            Err(FrameError::Malformed(why)) => return Err(format!("the engine's result frame is damaged: {why}")),
        }
    }
    Ok(reply)
}

/// FABRIC == VM: a finished run whose shows are the VM's lines (a Main program) or whose result is the VM's answer
/// (a function). An overflowed run never agrees.
pub fn agrees(golden: &Golden, reply: &Reply) -> bool {
    match (golden, reply.ended) {
        (Golden::Lines(lines), Some(Ended::Finished { .. })) => &reply.shows == lines,
        (Golden::Answer(v), Some(Ended::Finished { result, .. })) => result == *v,
        _ => false,
    }
}
