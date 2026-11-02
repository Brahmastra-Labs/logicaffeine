//! The Fabric Terminal's wire protocol and screen, in one place: the link between a host and the terminal board
//! (Board B, the Nexys A7-100T) that puts a LOGOS editor on a VGA screen, takes a keyboard on the board's USB port,
//! and runs each program on the fabric engine beside them.
//!
//! The link is one UART, its bytes framed by SLIP (RFC 1055): a frame ends with [`SLIP_END`], and an `END` or
//! [`SLIP_ESC`] inside a frame travels as `ESC` + [`SLIP_ESC_END`] / [`SLIP_ESC_ESC`]. A receiver that starts
//! mid-frame, or loses a byte, is back in step at the next `END`. A frame's first byte is its kind:
//! [`host`] kinds travel host → board, [`board`] kinds board → host.
//!
//! The screen is text: [`COLS`] × [`ROWS`] cells of 8 × 16 pixels on 640 × 480 at 60 Hz. A cell is 16 bits —
//! glyph (7 bits), blink, foreground and background from the 16-colour [`PALETTE`] — addressed `row << 7 | col`,
//! so no multiply sits in the pixel path. The bottom [`PANEL_ROWS`] rows belong to the activity panel the board
//! draws itself from the engine's probes; text written there is covered.
//!
//! Every byte and dimension the RTL uses is filled into its templates from here by [`fill_verilog`], and a host
//! that is not Rust reads [`table_json`] (`fabric_terminal --table`). Nothing restates a value.

/// SLIP frame delimiter.
pub const SLIP_END: u8 = 0xC0;
/// SLIP escape.
pub const SLIP_ESC: u8 = 0xDB;
/// An `END` inside a frame, after `ESC`.
pub const SLIP_ESC_END: u8 = 0xDC;
/// An `ESC` inside a frame, after `ESC`.
pub const SLIP_ESC_ESC: u8 = 0xDD;

/// Frame kinds a host sends.
pub mod host {
    /// `row, col, cell_hi, cell_lo, …` — write consecutive cells from (row, col); each cell is two bytes, the
    /// attribute byte (background << 4 | foreground) then the glyph byte (blink << 7 | glyph).
    pub const PUT: u8 = 0x01;
    /// `row, col, height, width, attribute, glyph` — fill a rectangle with one cell.
    pub const FILL: u8 = 0x02;
    /// `row, col, shown` — place the text cursor; `shown` 0 hides it.
    pub const CURSOR: u8 = 0x03;
    /// `row, col, shown` — place the pointer (a mouse's position, drawn as an inverted cell).
    pub const POINTER: u8 = 0x04;
    /// `row, col, rows, cols` — the small-text window: text rows `row..row + rows` and columns `col..col + cols`
    /// show the small plane ([`super::SMALL_FIRST_ROW`]) instead, two of its 8-pixel rows to each text row, drawn
    /// in the small font ([`super::FONT8_ROWS`]); `rows` 0 closes it.
    pub const WINDOW: u8 = 0x05;
    /// `byte, …` — bytes for the fabric engine's loader, in order; at most [`super::ENGINE_FRAME_MAX`] per frame,
    /// and the next frame only after [`super::board::ENGINE_TAKEN`].
    pub const ENGINE: u8 = 0x10;
    /// No payload — hold the fabric engine in reset for one engine clock and release it: a running program
    /// stops, and the loader waits for a new image.
    pub const ENGINE_RESET: u8 = 0x11;
    /// `b3, b2, b1, b0` — a 32-bit value for the 7-segment display, shown in signed decimal.
    pub const RESULT: u8 = 0x12;
    /// No payload — answered by [`super::board::ACTIVITY`], the fabric engine's last frame of activity.
    pub const ACTIVITY: u8 = 0x13;
    /// No payload — answered by [`super::board::TOTALS`], the engine clocks of the run so far, by block and cell.
    pub const TOTALS: u8 = 0x14;
    /// `byte` — send one byte to the PS/2 device (a mouse needs `0xF4` before it reports).
    pub const PS2_SEND: u8 = 0x20;
    /// `row, col, count` — read `count` cells back from (row, col); answered by [`super::board::CELLS`].
    pub const READ: u8 = 0x30;
    /// No payload — answered by [`super::board::PONG`].
    pub const PING: u8 = 0x3F;
    pub const ALL: [(&str, u8); 13] = [
        ("put", PUT),
        ("fill", FILL),
        ("cursor", CURSOR),
        ("pointer", POINTER),
        ("window", WINDOW),
        ("engine", ENGINE),
        ("engine_reset", ENGINE_RESET),
        ("result", RESULT),
        ("activity", ACTIVITY),
        ("totals", TOTALS),
        ("ps2_send", PS2_SEND),
        ("read", READ),
        ("ping", PING),
    ];
}

/// Frame kinds a board sends.
pub mod board {
    /// `byte, status` — one byte from the PS/2 device; `status` is 0, or [`super::PS2_PARITY_ERROR`] /
    /// [`super::PS2_FRAMING_ERROR`] for a byte received damaged.
    pub const PS2: u8 = 0x81;
    /// `status` — the outcome of a [`super::host::PS2_SEND`]: 0 acknowledged by the device, else
    /// [`super::PS2_NO_ACK`].
    pub const PS2_SENT: u8 = 0x82;
    /// `byte, …` — bytes the fabric engine sent (its Show records and result frame, `chip_protocol`'s format).
    pub const ENGINE: u8 = 0x90;
    /// `count` — the `count` bytes of the last [`super::host::ENGINE`] frame are inside the engine; the host sends
    /// the next. The engine takes bytes slower than the link brings them, so a host waits for this between frames.
    pub const ENGINE_TAKEN: u8 = 0x91;
    /// [`super::ACTIVITY_FIELDS`] — the fabric engine in the last whole video frame, as the activity panel drew it:
    /// the frame's number, whether a program ran, its block, cell, frame depth, heap frontier and run cycles at the
    /// frame's snapshot, and how many engine clocks it spent in each block and in each cell during the frame. The
    /// frame's number comes again last: a reply whose two numbers differ spans a frame's turnover and is not one
    /// frame's counts.
    pub const ACTIVITY: u8 = 0x92;
    /// [`super::TOTALS_FIELDS`] — the run's cycles as the activity panel's last snapshot holds them, then the engine
    /// clocks the run spent in each block and in each cell since the engine was last reset, every one counted, each
    /// total holding at its largest rather than wrapping. Once the run has ended and a frame has passed, they are its
    /// whole profile: its block totals, and its cell totals, each add up to its cycles and one (the clock that sees
    /// it done). Read during a run, each total is as it stood when its bytes were sent.
    pub const TOTALS: u8 = 0x93;
    /// `row, col, (cell_hi, cell_lo) × count` — cells read back for a [`super::host::READ`].
    pub const CELLS: u8 = 0xB0;
    /// [`super::PONG_FIELDS`] — the board is up, the screen it draws, what the image holds (a bit per feature,
    /// [`super::FEATURE_ENGINE`], [`super::FEATURE_SMALL`], [`super::FEATURE_TOTALS`]), and the PS/2 line since the
    /// image started: the clock edges the device drove, the shortest clock-low phase among them in microseconds
    /// (255 until one is shorter), and the bytes read whole and damaged.
    pub const PONG: u8 = 0xBF;
    pub const ALL: [(&str, u8); 8] = [
        ("ps2", PS2),
        ("ps2_sent", PS2_SENT),
        ("engine", ENGINE),
        ("engine_taken", ENGINE_TAKEN),
        ("activity", ACTIVITY),
        ("totals", TOTALS),
        ("cells", CELLS),
        ("pong", PONG),
    ];
}

/// [`board::PONG`]'s payload, a byte each in this order; a two-byte count is high byte first.
pub const PONG_FIELDS: [&str; 11] = [
    "version",
    "cols",
    "rows",
    "panel_rows",
    "features",
    "ps2_edges_hi",
    "ps2_edges_lo",
    "ps2_shortest_low_us",
    "ps2_bytes_hi",
    "ps2_bytes_lo",
    "ps2_damaged",
];

/// [`board::ACTIVITY`]'s payload, field by field: (name, bytes each, how many), every count big-endian. The per-block
/// and per-cell clocks are one count per block slot and per cell slot of Board B's engine.
pub const ACTIVITY_FIELDS: [(&str, usize, usize); 10] = [
    ("frame", 2, 1),
    ("running", 1, 1),
    ("block", 2, 1),
    ("cell", 2, 1),
    ("depth", 2, 1),
    ("heap", 2, 1),
    ("cycles", 4, 1),
    ("block_clocks", 2, super::wprog_engine_rtl::WPE_HW_MAXBLK),
    ("cell_clocks", 2, super::wprog_engine_rtl::WPE_HW_MAXCELL),
    ("frame_again", 2, 1),
];

/// [`board::TOTALS`]'s payload, field by field, as [`ACTIVITY_FIELDS`] is laid out.
pub const TOTALS_FIELDS: [(&str, usize, usize); 3] = [
    ("cycles", 4, 1),
    ("block_totals", 4, super::wprog_engine_rtl::WPE_HW_MAXBLK),
    ("cell_totals", 4, super::wprog_engine_rtl::WPE_HW_MAXCELL),
];

const fn payload_bytes(fields: &[(&str, usize, usize)]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < fields.len() {
        n += fields[i].1 * fields[i].2;
        i += 1;
    }
    n
}

/// The length of [`board::ACTIVITY`]'s payload.
pub const fn activity_bytes() -> usize {
    payload_bytes(&ACTIVITY_FIELDS)
}

/// The length of [`board::TOTALS`]'s payload.
pub const fn totals_bytes() -> usize {
    payload_bytes(&TOTALS_FIELDS)
}

/// The protocol's version, as [`board::PONG`] reports it.
pub const VERSION: u8 = 1;

/// [`board::PONG`]'s feature bit for an image with the fabric engine: ENGINE frames reach an engine that answers.
/// Without it (the screen-and-keyboard shell) ENGINE bytes are taken and dropped.
pub const FEATURE_ENGINE: u8 = 0x01;

/// [`board::PONG`]'s feature bit for an image with the small-text plane and [`host::WINDOW`].
pub const FEATURE_SMALL: u8 = 0x02;

/// [`board::PONG`]'s feature bit for an image that keeps the run's totals: [`host::TOTALS`] is answered.
pub const FEATURE_TOTALS: u8 = 0x04;

/// The small-text plane: text-buffer rows `SMALL_FIRST_ROW..SMALL_FIRST_ROW + SMALL_ROWS`, of which columns
/// `0..SMALL_COLS` hold cells. PUT, FILL and READ address them like any row; [`host::WINDOW`] puts them on screen.
pub const SMALL_FIRST_ROW: u8 = 32;
pub const SMALL_ROWS: u8 = 32;
pub const SMALL_COLS: u8 = 32;

/// The small font: glyph `g`'s row `r` is row `FONT8_ROWS[r]` of `g` in the 8×16 font — the rows that keep its
/// round letters round, its digits whole and its box-drawing lines joined from one small row to the next.
pub const FONT8_ROWS: [usize; 8] = [2, 4, 5, 7, 8, 10, 11, 13];

/// The most engine bytes one [`host::ENGINE`] frame carries: the board's engine FIFO holds one frame.
pub const ENGINE_FRAME_MAX: usize = 255;

/// The fabric engine's clock divider from the board clock, and its UART's clocks per bit: the board feeds the
/// engine's loader through an internal UART at `BOARD_HZ / ENGINE_CLOCK_DIVIDER / ENGINE_CLKS_PER_BIT` baud.
pub const ENGINE_CLOCK_DIVIDER: u32 = 128;
pub const ENGINE_CLKS_PER_BIT: u32 = 8;

/// A PS/2 byte whose odd parity did not hold.
pub const PS2_PARITY_ERROR: u8 = 0x01;
/// A PS/2 frame whose start bit was not 0 or stop bit not 1.
pub const PS2_FRAMING_ERROR: u8 = 0x02;
/// A host-to-device byte the device never acknowledged.
pub const PS2_NO_ACK: u8 = 0x01;

/// Text columns.
pub const COLS: u8 = 80;
/// Text rows.
pub const ROWS: u8 = 30;
/// Rows at the bottom that the activity panel draws over.
pub const PANEL_ROWS: u8 = 4;
/// Glyphs in the font ROM: a cell's glyph field is 7 bits.
pub const GLYPHS: usize = 128;
/// Glyph height in pixels (width is 8).
pub const GLYPH_ROWS: usize = 16;

/// The characters the font holds below the printable range, slot `i` holding character `i`: light and double box
/// lines, blocks and shades, arrows, markers and a tick — what a terminal UI draws with. Slots 0x20–0x7E are ASCII
/// and slot 0x7F holds [`DEL_GLYPH`]. A host maps a Unicode character to its slot by this table, and
/// `scripts/dev/gen-terminal-font.py` builds the font ROM by it.
pub const UI_GLYPHS: &str = "─│┌┐└┘├┤┬┴┼═║╔╗╚╝█▓▒░▀▄▌▐▶◀▲▼•·√";
/// The character slot 0x7F holds.
pub const DEL_GLYPH: char = '■';

/// The font slot that draws `ch`: ASCII as itself, [`UI_GLYPHS`] and [`DEL_GLYPH`] by their slots, anything else
/// as `?`.
pub fn glyph_slot(ch: char) -> u8 {
    match ch {
        ' '..='~' => ch as u8,
        _ if ch == DEL_GLYPH => 0x7F,
        _ => UI_GLYPHS.chars().position(|c| c == ch).map_or(b'?', |i| i as u8),
    }
}

/// The link's baud rate: 100 clocks per bit at the board's 100 MHz, and FTDI's 3 MHz base divided by 3, so
/// both ends run it exactly.
pub const BAUD: u32 = 1_000_000;
/// The board clock in Hz.
pub const BOARD_HZ: u32 = 100_000_000;

/// 640 × 480 at 60 Hz: (visible, front porch, sync, back porch) per axis in pixels / lines, sync active low,
/// one pixel per 4 board clocks (25 MHz; monitors lock to it within the standard's 0.7% of 25.175 MHz).
pub const H_TIMING: (u16, u16, u16, u16) = (640, 16, 96, 48);
pub const V_TIMING: (u16, u16, u16, u16) = (480, 10, 2, 33);
pub const CLOCKS_PER_PIXEL: u32 = 4;

/// The 16 colours a cell can name, as the board's 4-bit-per-channel DAC drives them (0xRGB), in CGA order.
pub const PALETTE: [u16; 16] = [
    0x000, 0x00A, 0x0A0, 0x0AA, 0xA00, 0xA0A, 0xA50, 0xAAA, 0x555, 0x55F, 0x5F5, 0x5FF, 0xF55, 0xF5F, 0xFF5, 0xFFF,
];

/// The activity panel, drawn by the board from the fabric engine's probes over the bottom [`PANEL_ROWS`] rows. The
/// host writes the panel's text (title, labels) as ordinary cells; the board paints its graphics over the cells
/// named here, every frame, with nothing from the host:
/// - [`PANEL_TITLE_ROW`]: the live run-cycle count, 10 decimal digits from column [`PANEL_CYCLES_COL`];
/// - [`PANEL_BLOCK_ROW`]: one tile per engine block from column [`PANEL_BLOCK_COL`], [`PANEL_BLOCK_TILE`] columns
///   wide, coloured by how often the block ran in the last frame (the [`HEAT`] ramp, log scale), the block running
///   now outlined; the frame-stack gauge from [`PANEL_STACK_COL`] (a segment per frame) and the heap gauge from
///   [`PANEL_HEAP_COL`] (a pixel per heap word);
/// - [`PANEL_CELL_ROW`]: one square per program cell from column [`PANEL_CELL_COL`], two columns apart, coloured
///   the same way, the cell being walked now outlined.
/// The labels go on [`PANEL_LABEL_ROW`]. Tile and square counts are the engine's caps (`WPE_HW_MAXBLK`,
/// `WPE_HW_MAXCELL`, `WPE_HW_DEPTH`, `WPE_HW_HEAP`).
pub const PANEL_TITLE_ROW: u8 = ROWS - PANEL_ROWS;
pub const PANEL_LABEL_ROW: u8 = PANEL_TITLE_ROW + 1;
pub const PANEL_BLOCK_ROW: u8 = PANEL_TITLE_ROW + 2;
pub const PANEL_CELL_ROW: u8 = PANEL_TITLE_ROW + 3;
pub const PANEL_CYCLES_COL: u8 = 62;
pub const PANEL_BLOCK_COL: u8 = 8;
pub const PANEL_BLOCK_TILE: u8 = 6;
pub const PANEL_CELL_COL: u8 = 8;
pub const PANEL_STACK_COL: u8 = 58;
pub const PANEL_HEAP_COL: u8 = 69;

/// The heat ramp a tile or square is painted with, by level: level `k` means the block or cell ran between 2^(k-1)
/// and 2^k - 1 engine cycles in the last frame (level 0: not at all) — a frame holds at most
/// [`panel_frame_samples`] engine cycles, so 14 is the hottest a frame reaches. Dark blue through cyan, green,
/// yellow and orange to red.
pub const HEAT: [u16; 16] = [
    0x112, 0x124, 0x136, 0x148, 0x16A, 0x18B, 0x1AB, 0x2C9, 0x4D6, 0x7D4, 0xAD3, 0xCC2, 0xEA1, 0xF81, 0xF51, 0xF22,
];
/// The panel's other colours (0xRGB): the outline of the tile or square the engine is in, the cycle digits, and the
/// gauges' lit and unlit segments. Everything else in a panel region is black.
pub const PANEL_OUTLINE: u16 = 0xFFF;
pub const PANEL_DIGIT: u16 = 0xFFF;
pub const PANEL_STACK_LIT: u16 = 0x4D6;
pub const PANEL_HEAP_LIT: u16 = 0x1AB;
pub const PANEL_GAUGE_UNLIT: u16 = 0x112;

/// The most engine clocks one frame of the activity panel can count: snapshots are a frame apart on the engine's
/// clock, so a frame's counts span a frame's board clocks over [`ENGINE_CLOCK_DIVIDER`], plus the one sample a
/// snapshot can slip by.
pub const fn panel_frame_samples() -> u32 {
    let ht = (H_TIMING.0 + H_TIMING.1 + H_TIMING.2 + H_TIMING.3) as u32;
    let vt = (V_TIMING.0 + V_TIMING.1 + V_TIMING.2 + V_TIMING.3) as u32;
    ht * vt * CLOCKS_PER_PIXEL / ENGINE_CLOCK_DIVIDER + 1
}

/// The heat level a count of engine clocks paints: its bit length.
pub const fn heat_level(hits: u32) -> usize {
    (32 - hits.leading_zeros()) as usize
}

/// One cell: its attribute and glyph bytes as they travel and as the text buffer holds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub glyph: u8,
    pub blink: bool,
    pub fg: u8,
    pub bg: u8,
}

impl Cell {
    pub fn new(glyph: u8, fg: u8, bg: u8) -> Self {
        Cell { glyph: glyph & 0x7F, blink: false, fg: fg & 0x0F, bg: bg & 0x0F }
    }

    /// The two bytes on the wire and in the buffer: attribute (bg << 4 | fg), then glyph (blink << 7 | glyph).
    pub fn bytes(self) -> [u8; 2] {
        [(self.bg << 4) | self.fg, ((self.blink as u8) << 7) | (self.glyph & 0x7F)]
    }

    pub fn from_bytes(attr: u8, glyph: u8) -> Self {
        Cell { glyph: glyph & 0x7F, blink: glyph & 0x80 != 0, fg: attr & 0x0F, bg: attr >> 4 }
    }
}

/// A text-buffer address: `row << 7 | col`.
pub fn cell_address(row: u8, col: u8) -> u16 {
    ((row as u16) << 7) | col as u16
}

/// One frame as SLIP puts it on the wire: the escaped bytes, then `END`.
pub fn slip_encode(frame: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.len() + 2);
    for &b in frame {
        match b {
            SLIP_END => out.extend_from_slice(&[SLIP_ESC, SLIP_ESC_END]),
            SLIP_ESC => out.extend_from_slice(&[SLIP_ESC, SLIP_ESC_ESC]),
            _ => out.push(b),
        }
    }
    out.push(SLIP_END);
    out
}

/// A SLIP receiver: bytes in, whole frames out. An empty frame (two `END`s in a row) is no frame; an `ESC`
/// followed by anything but `ESC_END` / `ESC_ESC` drops the frame it is in.
#[derive(Default, Debug)]
pub struct SlipDecoder {
    frame: Vec<u8>,
    escaped: bool,
    damaged: bool,
}

impl SlipDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one byte; returns a frame when this byte ended one.
    pub fn push(&mut self, b: u8) -> Option<Vec<u8>> {
        if b == SLIP_END {
            let frame = std::mem::take(&mut self.frame);
            let damaged = std::mem::replace(&mut self.damaged, false);
            self.escaped = false;
            return (!frame.is_empty() && !damaged).then_some(frame);
        }
        if self.escaped {
            self.escaped = false;
            match b {
                SLIP_ESC_END => self.frame.push(SLIP_END),
                SLIP_ESC_ESC => self.frame.push(SLIP_ESC),
                _ => self.damaged = true,
            }
        } else if b == SLIP_ESC {
            self.escaped = true;
        } else {
            self.frame.push(b);
        }
        None
    }

    /// Every whole frame in `bytes`, in order.
    pub fn decode_all(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        bytes.iter().filter_map(|&b| self.push(b)).collect()
    }
}

/// Clocks per UART bit on the board for [`BAUD`].
pub fn clocks_per_bit() -> u32 {
    BOARD_HZ / BAUD
}

const VERILOG_BYTES: [(&str, u8); 33] = [
    ("SLIP_END", SLIP_END),
    ("SLIP_ESC", SLIP_ESC),
    ("SLIP_ESC_END", SLIP_ESC_END),
    ("SLIP_ESC_ESC", SLIP_ESC_ESC),
    ("H_PUT", host::PUT),
    ("H_FILL", host::FILL),
    ("H_CURSOR", host::CURSOR),
    ("H_POINTER", host::POINTER),
    ("H_WINDOW", host::WINDOW),
    ("H_ENGINE", host::ENGINE),
    ("H_ENGINE_RESET", host::ENGINE_RESET),
    ("H_RESULT", host::RESULT),
    ("H_ACTIVITY", host::ACTIVITY),
    ("H_TOTALS", host::TOTALS),
    ("H_PS2_SEND", host::PS2_SEND),
    ("H_READ", host::READ),
    ("H_PING", host::PING),
    ("B_PS2", board::PS2),
    ("B_PS2_SENT", board::PS2_SENT),
    ("B_ENGINE", board::ENGINE),
    ("B_ENGINE_TAKEN", board::ENGINE_TAKEN),
    ("B_ACTIVITY", board::ACTIVITY),
    ("B_TOTALS", board::TOTALS),
    ("B_CELLS", board::CELLS),
    ("B_PONG", board::PONG),
    ("VERSION", VERSION),
    ("PS2_PARITY_ERROR", PS2_PARITY_ERROR),
    ("PS2_FRAMING_ERROR", PS2_FRAMING_ERROR),
    ("PS2_NO_ACK", PS2_NO_ACK),
    ("PANEL_ROWS", PANEL_ROWS),
    ("FEATURE_ENGINE", FEATURE_ENGINE),
    ("FEATURE_SMALL", FEATURE_SMALL),
    ("FEATURE_TOTALS", FEATURE_TOTALS),
];

const VERILOG_COLOURS: [(&str, u16); 5] = [
    ("PANEL_OUTLINE", PANEL_OUTLINE),
    ("PANEL_DIGIT", PANEL_DIGIT),
    ("PANEL_STACK_LIT", PANEL_STACK_LIT),
    ("PANEL_HEAP_LIT", PANEL_HEAP_LIT),
    ("PANEL_GAUGE_UNLIT", PANEL_GAUGE_UNLIT),
];

/// The integer parameters the RTL takes from here: `__N_<NAME>__` becomes the decimal number.
pub fn verilog_numbers() -> Vec<(&'static str, u32)> {
    vec![
        ("COLS", COLS as u32),
        ("ROWS", ROWS as u32),
        ("SMALL_FIRST_ROW", SMALL_FIRST_ROW as u32),
        ("SMALL_ROWS", SMALL_ROWS as u32),
        ("SMALL_COLS", SMALL_COLS as u32),
        ("PANEL_FIRST_ROW", (ROWS - PANEL_ROWS) as u32),
        ("CLKS_PER_BIT", clocks_per_bit()),
        ("ENGINE_CLKS_PER_BIT", ENGINE_CLKS_PER_BIT),
        ("ENGINE_CLOCK_DIVIDER", ENGINE_CLOCK_DIVIDER),
        ("ENGINE_LINK_CLKS_PER_BIT", ENGINE_CLKS_PER_BIT * ENGINE_CLOCK_DIVIDER),
        ("ENGINE_FRAME_MAX", ENGINE_FRAME_MAX as u32),
        ("CLOCKS_PER_PIXEL", CLOCKS_PER_PIXEL),
        ("H_VISIBLE", H_TIMING.0 as u32),
        ("H_FRONT", H_TIMING.1 as u32),
        ("H_SYNC", H_TIMING.2 as u32),
        ("H_BACK", H_TIMING.3 as u32),
        ("V_VISIBLE", V_TIMING.0 as u32),
        ("V_FRONT", V_TIMING.1 as u32),
        ("V_SYNC", V_TIMING.2 as u32),
        ("V_BACK", V_TIMING.3 as u32),
        ("PANEL_TITLE_ROW", PANEL_TITLE_ROW as u32),
        ("PANEL_LABEL_ROW", PANEL_LABEL_ROW as u32),
        ("PANEL_BLOCK_ROW", PANEL_BLOCK_ROW as u32),
        ("PANEL_CELL_ROW", PANEL_CELL_ROW as u32),
        ("PANEL_CYCLES_COL", PANEL_CYCLES_COL as u32),
        ("PANEL_BLOCK_COL", PANEL_BLOCK_COL as u32),
        ("PANEL_BLOCK_TILE", PANEL_BLOCK_TILE as u32),
        ("PANEL_CELL_COL", PANEL_CELL_COL as u32),
        ("PANEL_STACK_COL", PANEL_STACK_COL as u32),
        ("PANEL_HEAP_COL", PANEL_HEAP_COL as u32),
        ("MAXBLK", super::wprog_engine_rtl::WPE_HW_MAXBLK as u32),
        ("MAXCELL", super::wprog_engine_rtl::WPE_HW_MAXCELL as u32),
        ("DEPTH", super::wprog_engine_rtl::WPE_HW_DEPTH as u32),
        ("HEAP", super::wprog_engine_rtl::WPE_HW_HEAP as u32),
    ]
}

/// Fill a terminal Verilog template from this module: `__V_<NAME>__` becomes a byte as a Verilog literal
/// (`8'hC0`), `__X_<NAME>__` the hex a comment quotes (`0xC0`), `__N_<NAME>__` a decimal number, `__C_<NAME>__` a
/// colour (`12'hFFF`), `__PALETTE_CASE__` the palette as `case` arms (`4'd1: palette = 12'h00A;`), `__HEAT_CASE__` the
/// heat ramp the same way, and `__DIGIT_ROM_CASE__` the font's digit glyphs keyed `{digit, row}`. Panics on a
/// placeholder this module does not define, so a template cannot name a value nobody owns.
pub fn fill_verilog(template: &str) -> String {
    let mut out = template.to_string();
    for (name, byte) in VERILOG_BYTES {
        out = out.replace(&format!("__V_{name}__"), &format!("8'h{byte:02X}"));
        out = out.replace(&format!("__X_{name}__"), &format!("0x{byte:02X}"));
    }
    for (name, n) in verilog_numbers() {
        out = out.replace(&format!("__N_{name}__"), &n.to_string());
    }
    for (name, rgb) in VERILOG_COLOURS {
        out = out.replace(&format!("__C_{name}__"), &format!("12'h{rgb:03X}"));
    }
    let palette: Vec<String> =
        PALETTE.iter().enumerate().map(|(i, rgb)| format!("      4'd{i}: palette = 12'h{rgb:03X};")).collect();
    out = out.replace("__PALETTE_CASE__", &palette.join("\n"));
    let heat: Vec<String> =
        HEAT.iter().enumerate().map(|(i, rgb)| format!("      4'd{i}: heat_rgb = 12'h{rgb:03X};")).collect();
    out = out.replace("__HEAT_CASE__", &heat.join("\n"));
    if out.contains("__DIGIT_ROM_CASE__") {
        let font = super::terminal_rtl::font();
        let mut arms = Vec::new();
        for d in 0..10u8 {
            for row in 0..GLYPH_ROWS {
                let bits = font[(b'0' + d) as usize * GLYPH_ROWS + row];
                arms.push(format!("      8'h{d:X}{row:X}: digit_bits = 8'h{bits:02X};"));
            }
        }
        out = out.replace("__DIGIT_ROM_CASE__", &arms.join("\n"));
    }
    for marker in ["__V_", "__X_", "__N_", "__C_", "__PALETTE", "__HEAT", "__DIGIT"] {
        if let Some(at) = out.find(marker) {
            let name: String = out[at..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            panic!("a terminal template names a value terminal_protocol does not define: {name}");
        }
    }
    out
}

/// The whole protocol as JSON, for a host that is not Rust.
pub fn table_json() -> String {
    let kinds = |all: &[(&str, u8)]| -> String {
        all.iter().map(|(n, v)| format!("\"{n}\": {v}")).collect::<Vec<_>>().join(", ")
    };
    let palette: Vec<String> = PALETTE.iter().map(|c| c.to_string()).collect();
    format!(
        "{{\"version\": {VERSION}, \
         \"slip\": {{\"end\": {SLIP_END}, \"esc\": {SLIP_ESC}, \"esc_end\": {SLIP_ESC_END}, \"esc_esc\": {SLIP_ESC_ESC}}}, \
         \"host\": {{{}}}, \"board\": {{{}}}, \
         \"ps2\": {{\"parity_error\": {PS2_PARITY_ERROR}, \"framing_error\": {PS2_FRAMING_ERROR}, \"no_ack\": {PS2_NO_ACK}}}, \
         \"screen\": {{\"cols\": {COLS}, \"rows\": {ROWS}, \"panel_rows\": {PANEL_ROWS}, \"glyphs\": {GLYPHS}, \
         \"small_first_row\": {SMALL_FIRST_ROW}, \"small_rows\": {SMALL_ROWS}, \"small_cols\": {SMALL_COLS}, \
         \"glyph_rows\": {GLYPH_ROWS}, \"ui_glyphs\": \"{UI_GLYPHS}\", \"del_glyph\": \"{DEL_GLYPH}\", \
         \"palette\": [{}]}}, \
         \"panel\": {{\"title_row\": {PANEL_TITLE_ROW}, \"label_row\": {PANEL_LABEL_ROW}, \"block_row\": {PANEL_BLOCK_ROW}, \
         \"cell_row\": {PANEL_CELL_ROW}, \"cycles_col\": {PANEL_CYCLES_COL}, \"block_col\": {PANEL_BLOCK_COL}, \
         \"block_tile\": {PANEL_BLOCK_TILE}, \"cell_col\": {PANEL_CELL_COL}, \"stack_col\": {PANEL_STACK_COL}, \
         \"heap_col\": {PANEL_HEAP_COL}, \"blocks\": {}, \"cells\": {}, \"depth\": {}, \"heap\": {}}}, \
         \"features\": {{\"engine\": {FEATURE_ENGINE}, \"small\": {FEATURE_SMALL}, \"totals\": {FEATURE_TOTALS}}}, \"pong\": [{}], \"activity\": [{}], \"totals\": [{}], \"engine_frame_max\": {ENGINE_FRAME_MAX}, \"baud\": {BAUD}}}",
        kinds(&host::ALL),
        kinds(&board::ALL),
        palette.join(", "),
        super::wprog_engine_rtl::WPE_HW_MAXBLK,
        super::wprog_engine_rtl::WPE_HW_MAXCELL,
        super::wprog_engine_rtl::WPE_HW_DEPTH,
        super::wprog_engine_rtl::WPE_HW_HEAP,
        PONG_FIELDS.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", "),
        layout(&ACTIVITY_FIELDS),
        layout(&TOTALS_FIELDS),
    )
}

fn layout(fields: &[(&str, usize, usize)]) -> String {
    fields
        .iter()
        .map(|(name, bytes, count)| format!("{{\"name\": \"{name}\", \"bytes\": {bytes}, \"count\": {count}}}"))
        .collect::<Vec<_>>()
        .join(", ")
}
