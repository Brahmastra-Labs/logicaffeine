//! The chips' byte protocol on the UART, defined once.
//!
//! The whole-program engine talks to its host over one byte channel: it takes the program image, streams a
//! record for every `Show` as the program runs, sends a request and waits for the reply when the program reaches
//! the outside world (a host call), and ends with a result frame. Every byte value in that exchange is defined
//! here and nowhere else. The engine model, the renderer and the host bridge use these constants, and
//! `scripts/logos-chip.py` reads the same table as JSON from the `chip_host` example ([`table_json`]).
//!
//! | record | direction | bytes |
//! |---|---|---|
//! | program image | host → chip | the config memories and the params, then [`IMAGE_GO`] |
//! | show | chip → host | [`REC_SHOW`], a value |
//! | call | chip → host | [`REC_CALL`], the [`HostCall`] code, the argument count, that many values |
//! | reply | host → chip | [`REC_REPLY`], a value |
//! | result frame | chip → host | [`FRAME_SYNC`], result `[63:0]`, cycles `[31:0]`, one end byte |
//!
//! A program that pre-loads its heap sends [`IMAGE_HEAP`] and the pre-load between the params and [`IMAGE_GO`].
//! A value is its tag, then 8 bytes most significant first, except a Text: a 4-byte length, most significant
//! first, and that many UTF-8 bytes. The end byte of the result frame is [`FRAME_END`] for a finished run,
//! [`FRAME_END_CALL_STACK`] when the call stack overflowed and [`FRAME_END_HEAP`] when the heap did. Neither
//! overflow's result bytes are an answer.

/// The program image's heap pre-load segment follows.
pub const IMAGE_HEAP: u8 = 0x48;
/// The program image is complete; the engine starts the run.
pub const IMAGE_GO: u8 = 0x5A;

/// A `Show` record.
pub const REC_SHOW: u8 = 0xA7;
/// A host call's request.
pub const REC_CALL: u8 = 0xA8;
/// The host's reply to a host call.
pub const REC_REPLY: u8 = 0xA9;

/// The first byte of the result frame.
pub const FRAME_SYNC: u8 = 0xA5;
/// The result frame of a run that finished.
pub const FRAME_END: u8 = 0x5A;
/// The result frame of a run whose call stack overflowed the engine's frame stack.
pub const FRAME_END_CALL_STACK: u8 = 0xE1;
/// The result frame of a run whose heap overflowed the engine's heap.
pub const FRAME_END_HEAP: u8 = 0xE2;

/// A signed 64-bit Int.
pub const TAG_INT: u8 = 0;
/// A Real: its IEEE-754 bit pattern, rendered with the language's `fmt_f64`.
pub const TAG_FLOAT: u8 = 1;
/// A Text. On the engine, the shown value is a heap pointer to a char-list `[len, byte0..]`; on the wire, the
/// length and the bytes.
pub const TAG_TEXT: u8 = 2;
/// A `Word32`, rendered unsigned.
pub const TAG_WORD32: u8 = 3;
/// A Bool, carried as 0 or 1.
pub const TAG_BOOL: u8 = 4;

/// Every value tag, in code order.
pub const TAGS: [u8; 5] = [TAG_INT, TAG_FLOAT, TAG_TEXT, TAG_WORD32, TAG_BOOL];

/// The program's operations that reach the outside world through the host. The host services each with the one
/// relay client (`logicaffeine_system::relay`); the chip never frames the relay protocol itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum HostCall {
    /// `Listen on <topic>`.
    Listen = 1,
    /// `Connect to <address>`.
    Connect = 2,
    /// `Send <value> to <peer>`.
    Send = 3,
    /// `Await response from <peer> into <name>`.
    Await = 4,
    /// `Stream <values> to <peer>`.
    Stream = 5,
    /// `Await stream from <peer> into <name>`.
    AwaitStream = 6,
    /// `Sync <value> on <topic>`.
    Sync = 7,
}

impl HostCall {
    /// Every host call, in code order.
    pub const ALL: [HostCall; 7] = [
        HostCall::Listen,
        HostCall::Connect,
        HostCall::Send,
        HostCall::Await,
        HostCall::Stream,
        HostCall::AwaitStream,
        HostCall::Sync,
    ];

    /// The byte a call record carries.
    pub fn code(self) -> u8 {
        self as u8
    }

    /// The call a record's byte names.
    pub fn from_code(code: u8) -> Option<HostCall> {
        HostCall::ALL.into_iter().find(|c| c.code() == code)
    }

    /// The call's name in the table.
    pub fn name(self) -> &'static str {
        match self {
            HostCall::Listen => "listen",
            HostCall::Connect => "connect",
            HostCall::Send => "send",
            HostCall::Await => "await",
            HostCall::Stream => "stream",
            HostCall::AwaitStream => "await_stream",
            HostCall::Sync => "sync",
        }
    }
}

/// A value as the protocol carries it.
#[derive(Clone, Debug, PartialEq)]
pub enum ChipValue {
    Int(i64),
    /// The bit pattern, so every NaN payload and the sign of zero survive the trip.
    Float(u64),
    Text(String),
    Word32(u32),
    Bool(bool),
}

impl ChipValue {
    /// The value's tag.
    pub fn tag(&self) -> u8 {
        match self {
            ChipValue::Int(_) => TAG_INT,
            ChipValue::Float(_) => TAG_FLOAT,
            ChipValue::Text(_) => TAG_TEXT,
            ChipValue::Word32(_) => TAG_WORD32,
            ChipValue::Bool(_) => TAG_BOOL,
        }
    }

    /// Append the value's framing: the tag, then the payload.
    pub fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.tag());
        match self {
            ChipValue::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
            ChipValue::Float(bits) => out.extend_from_slice(&bits.to_be_bytes()),
            ChipValue::Word32(w) => out.extend_from_slice(&u64::from(*w).to_be_bytes()),
            ChipValue::Bool(b) => out.extend_from_slice(&u64::from(*b).to_be_bytes()),
            ChipValue::Text(s) => {
                out.extend_from_slice(&(s.len() as u32).to_be_bytes());
                out.extend_from_slice(s.as_bytes());
            }
        }
    }

    /// Decode one value from the front of `bytes`, returning it and the bytes it took.
    pub fn decode(bytes: &[u8]) -> Result<(ChipValue, usize), FrameError> {
        let tag = *bytes.first().ok_or(FrameError::Short)?;
        let word = |at: usize| -> Result<u64, FrameError> {
            let b: [u8; 8] = bytes
                .get(at..at + 8)
                .ok_or(FrameError::Short)?
                .try_into()
                .expect("8 bytes");
            Ok(u64::from_be_bytes(b))
        };
        match tag {
            TAG_INT => Ok((ChipValue::Int(word(1)? as i64), 9)),
            TAG_FLOAT => Ok((ChipValue::Float(word(1)?), 9)),
            TAG_WORD32 => {
                let w = word(1)?;
                let w = u32::try_from(w)
                    .map_err(|_| FrameError::Malformed(format!("a Word32 of {w:#x}")))?;
                Ok((ChipValue::Word32(w), 9))
            }
            TAG_BOOL => match word(1)? {
                0 => Ok((ChipValue::Bool(false), 9)),
                1 => Ok((ChipValue::Bool(true), 9)),
                w => Err(FrameError::Malformed(format!("a Bool of {w}"))),
            },
            TAG_TEXT => {
                let len: [u8; 4] = bytes
                    .get(1..5)
                    .ok_or(FrameError::Short)?
                    .try_into()
                    .expect("4 bytes");
                let len = u32::from_be_bytes(len) as usize;
                let text = bytes.get(5..5 + len).ok_or(FrameError::Short)?;
                let text = String::from_utf8(text.to_vec())
                    .map_err(|_| FrameError::Malformed("a Text that is not UTF-8".to_string()))?;
                Ok((ChipValue::Text(text), 5 + len))
            }
            t => Err(FrameError::Malformed(format!("value tag {t}"))),
        }
    }
}

/// One record of the exchange.
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    Show(ChipValue),
    Call {
        call: HostCall,
        args: Vec<ChipValue>,
    },
    Reply(ChipValue),
}

/// Why bytes are not a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The bytes end before the record does.
    Short,
    /// The bytes are not a record: an unknown marker, tag or call, or a payload out of its type's range.
    Malformed(String),
}

impl Record {
    /// The record's bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Record::Show(v) => {
                out.push(REC_SHOW);
                v.encode(&mut out);
            }
            Record::Call { call, args } => {
                out.push(REC_CALL);
                out.push(call.code());
                out.push(
                    u8::try_from(args.len()).expect("a host call carries at most 255 arguments"),
                );
                for a in args {
                    a.encode(&mut out);
                }
            }
            Record::Reply(v) => {
                out.push(REC_REPLY);
                v.encode(&mut out);
            }
        }
        out
    }

    /// Decode one record from the front of `bytes`, returning it and the bytes it took.
    pub fn decode(bytes: &[u8]) -> Result<(Record, usize), FrameError> {
        let marker = *bytes.first().ok_or(FrameError::Short)?;
        match marker {
            REC_SHOW => {
                let (v, n) = ChipValue::decode(&bytes[1..])?;
                Ok((Record::Show(v), 1 + n))
            }
            REC_REPLY => {
                let (v, n) = ChipValue::decode(&bytes[1..])?;
                Ok((Record::Reply(v), 1 + n))
            }
            REC_CALL => {
                let code = *bytes.get(1).ok_or(FrameError::Short)?;
                let call = HostCall::from_code(code)
                    .ok_or_else(|| FrameError::Malformed(format!("host call {code}")))?;
                let argc = *bytes.get(2).ok_or(FrameError::Short)? as usize;
                let mut at = 3;
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    let (v, n) = ChipValue::decode(&bytes[at..])?;
                    args.push(v);
                    at += n;
                }
                Ok((Record::Call { call, args }, at))
            }
            m => Err(FrameError::Malformed(format!("record marker {m:#04x}"))),
        }
    }
}

/// The result frame that closes a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultFrame {
    pub result: i64,
    pub cycles: u32,
    /// [`FRAME_END`], [`FRAME_END_CALL_STACK`] or [`FRAME_END_HEAP`].
    pub end: u8,
}

impl ResultFrame {
    /// The frame's bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![FRAME_SYNC];
        out.extend_from_slice(&self.result.to_be_bytes());
        out.extend_from_slice(&self.cycles.to_be_bytes());
        out.push(self.end);
        out
    }

    /// Decode a frame from the front of `bytes`, returning it and the bytes it took.
    pub fn decode(bytes: &[u8]) -> Result<(ResultFrame, usize), FrameError> {
        let sync = *bytes.first().ok_or(FrameError::Short)?;
        if sync != FRAME_SYNC {
            return Err(FrameError::Malformed(format!("frame sync {sync:#04x}")));
        }
        let body = bytes.get(1..14).ok_or(FrameError::Short)?;
        let result = i64::from_be_bytes(body[0..8].try_into().expect("8 bytes"));
        let cycles = u32::from_be_bytes(body[8..12].try_into().expect("4 bytes"));
        let end = body[12];
        if ![FRAME_END, FRAME_END_CALL_STACK, FRAME_END_HEAP].contains(&end) {
            return Err(FrameError::Malformed(format!("frame end {end:#04x}")));
        }
        Ok((
            ResultFrame {
                result,
                cycles,
                end,
            },
            14,
        ))
    }
}

/// The protocol bytes the engine's Verilog templates name, by the placeholder name they use.
const VERILOG_BYTES: [(&str, u8); 8] = [
    ("IMAGE_HEAP", IMAGE_HEAP),
    ("IMAGE_GO", IMAGE_GO),
    ("REC_SHOW", REC_SHOW),
    ("REC_CALL", REC_CALL),
    ("FRAME_SYNC", FRAME_SYNC),
    ("FRAME_END", FRAME_END),
    ("FRAME_END_CALL_STACK", FRAME_END_CALL_STACK),
    ("FRAME_END_HEAP", FRAME_END_HEAP),
];

/// Fill a Verilog template's protocol bytes from this table: `__V_<NAME>__` becomes the byte as a Verilog literal
/// (`8'hA7`), `__X_<NAME>__` the hex a comment quotes (`0xA7`). Panics if a placeholder names no protocol byte.
pub fn fill_verilog(template: &str) -> String {
    let mut out = template.to_string();
    for (name, byte) in VERILOG_BYTES {
        out = out.replace(&format!("__V_{name}__"), &format!("8'h{byte:02X}"));
        out = out.replace(&format!("__X_{name}__"), &format!("0x{byte:02X}"));
    }
    if let Some(at) = out.find("__V_").or_else(|| out.find("__X_")) {
        let name: String = out[at..]
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();
        panic!("the Verilog template names a protocol byte chip_protocol does not define: {name}");
    }
    out
}

/// The whole table as JSON, for a host that is not Rust: `scripts/logos-chip.py` reads it from
/// `chip_host --table` rather than restating a byte of it.
pub fn table_json() -> String {
    let tags = [
        ("int", TAG_INT),
        ("float", TAG_FLOAT),
        ("text", TAG_TEXT),
        ("word32", TAG_WORD32),
        ("bool", TAG_BOOL),
    ];
    let tags: Vec<String> = tags.iter().map(|(n, v)| format!("\"{n}\": {v}")).collect();
    let calls: Vec<String> = HostCall::ALL
        .iter()
        .map(|c| format!("\"{}\": {}", c.name(), c.code()))
        .collect();
    format!(
        "{{\"image\": {{\"heap\": {IMAGE_HEAP}, \"go\": {IMAGE_GO}}}, \
         \"records\": {{\"show\": {REC_SHOW}, \"call\": {REC_CALL}, \"reply\": {REC_REPLY}}}, \
         \"frame\": {{\"sync\": {FRAME_SYNC}, \"end\": {FRAME_END}, \"end_call_stack\": {FRAME_END_CALL_STACK}, \
         \"end_heap\": {FRAME_END_HEAP}}}, \"tags\": {{{}}}, \"calls\": {{{}}}}}",
        tags.join(", "),
        calls.join(", ")
    )
}
