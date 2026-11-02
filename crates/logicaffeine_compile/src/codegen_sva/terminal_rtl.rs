//! The Fabric Terminal's RTL, emitted from its templates (`codegen_sva/terminal/`) with every protocol byte and
//! dimension filled from [`super::terminal_protocol`]: the UART, SLIP framing, the PS/2 port, the VGA text plane,
//! the link controller, the activity panel and the Nexys A7-100T top with its pins; and [`render`], the picture the
//! screen's frames are held to. The board build reads the emitted files ([`board_files`]) from `benchmarks/silicon/`,
//! where `fabric_terminal --emit` writes them.

use super::terminal_protocol::fill_verilog;

const UART: &str = include_str!("terminal/term_uart.v");
const SLIP: &str = include_str!("terminal/term_slip.v");
const PS2: &str = include_str!("terminal/term_ps2.v");
const VGA: &str = include_str!("terminal/term_vga.v");
const LINK: &str = include_str!("terminal/term_link.v");
const ACTIVITY: &str = include_str!("terminal/term_activity.v");
const TOP: &str = include_str!("terminal/term_top.v");
const XDC: &str = include_str!("terminal/term_top.xdc");

/// The font ROM the text plane reads with `$readmemh`, one glyph row per line (see `scripts/dev/gen-terminal-font.py`).
pub const FONT_HEX: &str = include_str!("terminal/term_font.hex");
/// The font's license, which travels with the ROM.
pub const FONT_LICENSE: &str = include_str!("terminal/term_font.LICENSE");

/// The module files a terminal top instantiates, in dependency order: (file name, filled Verilog). The activity
/// panel belongs to a top with an engine.
pub fn modules(engine: bool) -> Vec<(&'static str, String)> {
    let mut files = vec![
        ("term_uart.v", fill_verilog(UART)),
        ("term_slip.v", fill_verilog(SLIP)),
        ("term_ps2.v", fill_verilog(PS2)),
        ("term_vga.v", fill_verilog(VGA)),
        ("term_link.v", fill_verilog(LINK)),
    ];
    if engine {
        files.push(("term_activity.v", fill_verilog(ACTIVITY)));
    }
    files
}

/// Keep the template text between `//__<keep>_BEGIN__` and `//__<keep>_END__`, and drop the `drop` section with
/// its markers.
fn select(text: &str, keep: &str, drop: &str) -> String {
    let (kb, ke) = (format!("//__{keep}_BEGIN__\n"), format!("//__{keep}_END__\n"));
    let (db, de) = (format!("//__{drop}_BEGIN__\n"), format!("//__{drop}_END__\n"));
    let start = text.find(&db).unwrap_or_else(|| panic!("the terminal top has no {drop} section"));
    let end = text[start..].find(&de).map(|e| start + e + de.len()).expect("an unterminated section");
    let without = format!("{}{}", &text[..start], &text[end..]);
    without.replace(&kb, "").replace(&ke, "")
}

/// The board top `top`: with `engine` naming the fabric engine module it instantiates — an engine emitted with its
/// probes (`emit_wprog_engine_soc_verilog_hw_probed`), built beside it — or without one: the screen, keyboard and
/// link alone, where ENGINE bytes are taken and dropped and the panel is not drawn.
pub fn top(top: &str, engine: Option<&str>) -> String {
    let text = match engine {
        Some(module) => select(TOP, "ENGINE", "NO_ENGINE").replace("__ENGINE_MODULE__", module),
        None => select(TOP, "NO_ENGINE", "ENGINE"),
    };
    fill_verilog(&text.replace("__TOP__", top))
}

/// The board's pin constraints for any terminal top.
pub fn xdc() -> String {
    XDC.to_string()
}

/// The fabric engine the Fabric Terminal instantiates: Board B's deployed engine, with its probes.
pub const ENGINE_MODULE: &str = "wpe_soc_fpga_probed";
/// The file [`board_files`] emits the engine into, beside the terminal's other files.
pub const ENGINE_FILE: &str = "nexys_fabric_terminal_engine.v";

/// Every file the board build reads from `benchmarks/silicon/`, as (file name, contents): the modules, the font
/// ROM and its license, the pin constraints, the probed engine, and two tops — `nexys_fabric_terminal` with the
/// engine and `nexys_terminal_shell`, the screen, keyboard and link alone.
pub fn board_files() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = modules(true).into_iter().map(|(name, text)| (name.to_string(), text)).collect();
    out.push(("term_font.hex".into(), FONT_HEX.into()));
    out.push(("term_font8.hex".into(), font8().iter().map(|b| format!("{b:02x}\n")).collect()));
    out.push(("term_font.LICENSE".into(), FONT_LICENSE.into()));
    out.push(("nexys_fabric_terminal.xdc".into(), xdc()));
    out.push((
        ENGINE_FILE.to_string(),
        super::wprog_engine_rtl::emit_wprog_engine_soc_verilog_hw_probed(
            ENGINE_MODULE.strip_prefix("wpe_soc_").expect("an engine module is wpe_soc_<name>"),
        ),
    ));
    out.push(("nexys_fabric_terminal.v".into(), top("nexys_fabric_terminal", Some(ENGINE_MODULE))));
    out.push(("nexys_terminal_shell.v".into(), top("nexys_terminal_shell", None)));
    out
}

/// The font ROM as bytes: glyph `g`, row `r` is `font()[g * 16 + r]`, the MSB its leftmost pixel.
pub fn font() -> Vec<u8> {
    FONT_HEX.lines().map(|l| u8::from_str_radix(l.trim(), 16).expect("the font ROM is hex bytes")).collect()
}

/// The small font ROM, made from [`font`]: glyph `g`, row `r` is `font8()[g * 8 + r]`, row
/// `terminal_protocol::FONT8_ROWS[r]` of the 8×16 glyph.
pub fn font8() -> Vec<u8> {
    let font = font();
    (0..font.len() / 16)
        .flat_map(|g| super::terminal_protocol::FONT8_ROWS.iter().map(move |&r| (g, r)))
        .map(|(g, r)| font[g * 16 + r])
        .collect()
}

/// What the text plane is told to draw: every cell, the cursor and the pointer, the flash phase that blinking
/// cells and the cursor follow, and the activity panel when the top has an engine.
#[derive(Clone, Debug)]
pub struct Screen {
    /// `ROWS × COLS` cells, row-major.
    pub cells: Vec<super::terminal_protocol::Cell>,
    /// The small plane: `SMALL_ROWS × SMALL_COLS` cells, row-major.
    pub small: Vec<super::terminal_protocol::Cell>,
    /// The small-text window as `(row, col, rows, cols)` in text cells, when open.
    pub window: Option<(u8, u8, u8, u8)>,
    pub cursor: Option<(u8, u8)>,
    pub pointer: Option<(u8, u8)>,
    pub flash: bool,
    pub panel: Option<Panel>,
}

impl Screen {
    /// The screen at power-on: every cell a light-grey space on black.
    pub fn blank() -> Self {
        use super::terminal_protocol::{Cell, COLS, ROWS, SMALL_COLS, SMALL_ROWS};
        Screen {
            cells: vec![Cell::new(b' ', 7, 0); ROWS as usize * COLS as usize],
            small: vec![Cell::new(b' ', 7, 0); SMALL_ROWS as usize * SMALL_COLS as usize],
            window: None,
            cursor: None,
            pointer: None,
            flash: false,
            panel: None,
        }
    }
}

/// What one frame of the activity panel shows: the engine's state at the frame's snapshot, and the engine clocks
/// each block and each cell ran in the frame before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    /// One count per engine block (`WPE_HW_MAXBLK`).
    pub block_hits: Vec<u32>,
    /// One count per program cell (`WPE_HW_MAXCELL`).
    pub cell_hits: Vec<u32>,
    pub running: bool,
    pub pc: u16,
    pub cell: u16,
    pub sp: u16,
    pub hp: u16,
    pub cycles: u32,
}

impl Panel {
    /// The panel before any program: nothing counted, the engine idle.
    pub fn idle() -> Self {
        use super::wprog_engine_rtl::{WPE_HW_MAXBLK, WPE_HW_MAXCELL};
        Panel {
            block_hits: vec![0; WPE_HW_MAXBLK],
            cell_hits: vec![0; WPE_HW_MAXCELL],
            running: false,
            pc: 0,
            cell: 0,
            sp: 0,
            hp: 0,
            cycles: 0,
        }
    }

    /// The panel's colour at screen pixel (`x`, `y`), or `None` where the text shows through. Inside its regions:
    /// - the cycle count: ten digit glyphs from the font, [`PANEL_DIGIT`] on black, leading zeros black, the units
    ///   digit always drawn;
    /// - a block tile or cell square: black but for an inner box two pixels in, painted with the [`HEAT`] colour of
    ///   its count, and the box's one-pixel border [`PANEL_OUTLINE`] when the engine is running in it;
    /// - a stack segment: eight pixels wide, a 6 × 10 box one pixel in from the left and three down, lit
    ///   ([`PANEL_STACK_LIT`]) for each frame below `sp`, else [`PANEL_GAUGE_UNLIT`];
    /// - a heap column: one pixel wide, eight tall from four down, lit ([`PANEL_HEAP_LIT`]) below `hp`.
    ///
    /// [`PANEL_DIGIT`]: super::terminal_protocol::PANEL_DIGIT
    /// [`HEAT`]: super::terminal_protocol::HEAT
    /// [`PANEL_OUTLINE`]: super::terminal_protocol::PANEL_OUTLINE
    /// [`PANEL_STACK_LIT`]: super::terminal_protocol::PANEL_STACK_LIT
    /// [`PANEL_GAUGE_UNLIT`]: super::terminal_protocol::PANEL_GAUGE_UNLIT
    /// [`PANEL_HEAP_LIT`]: super::terminal_protocol::PANEL_HEAP_LIT
    pub fn pixel(&self, font: &[u8], x: usize, y: usize) -> Option<u16> {
        use super::terminal_protocol::{
            heat_level, GLYPH_ROWS, HEAT, PANEL_BLOCK_COL, PANEL_BLOCK_ROW, PANEL_BLOCK_TILE, PANEL_CELL_COL,
            PANEL_CELL_ROW, PANEL_CYCLES_COL, PANEL_DIGIT, PANEL_GAUGE_UNLIT, PANEL_HEAP_COL, PANEL_HEAP_LIT,
            PANEL_OUTLINE, PANEL_STACK_COL, PANEL_STACK_LIT, PANEL_TITLE_ROW,
        };
        use super::wprog_engine_rtl::{WPE_HW_DEPTH, WPE_HW_HEAP};
        let row = y / GLYPH_ROWS;
        let gy = y % GLYPH_ROWS;
        let span = |col: u8, width: usize| (x >= col as usize * 8 && x < col as usize * 8 + width).then(|| x - col as usize * 8);
        let heat_box = |size: usize, hits: u32, current: bool, sub: usize| {
            let inner = sub >= 2 && sub < size - 2 && (2..14).contains(&gy);
            let rim = sub == 2 || sub == size - 3 || gy == 2 || gy == 13;
            if !inner {
                0
            } else if current && rim {
                PANEL_OUTLINE
            } else {
                HEAT[heat_level(hits)]
            }
        };
        if row == PANEL_TITLE_ROW as usize {
            let dx = span(PANEL_CYCLES_COL, 80)?;
            let place = 9 - dx / 8;
            let above = self.cycles as u64 / 10u64.pow(place as u32);
            let digit = (above % 10) as u8;
            let bits = font[(b'0' + digit) as usize * GLYPH_ROWS + gy];
            let on = (above != 0 || place == 0) && (bits >> (7 - dx % 8)) & 1 == 1;
            return Some(if on { PANEL_DIGIT } else { 0 });
        }
        if row == PANEL_BLOCK_ROW as usize {
            let tile = PANEL_BLOCK_TILE as usize * 8;
            if let Some(dx) = span(PANEL_BLOCK_COL, self.block_hits.len() * tile) {
                let b = dx / tile;
                return Some(heat_box(tile, self.block_hits[b], self.running && self.pc as usize == b, dx % tile));
            }
            if let Some(dx) = span(PANEL_STACK_COL, WPE_HW_DEPTH * 8) {
                let inner = (1..7).contains(&(dx % 8)) && (3..13).contains(&gy);
                let lit = dx / 8 < self.sp as usize;
                return Some(if !inner { 0 } else if lit { PANEL_STACK_LIT } else { PANEL_GAUGE_UNLIT });
            }
            if let Some(dx) = span(PANEL_HEAP_COL, WPE_HW_HEAP) {
                let inner = (4..12).contains(&gy);
                let lit = dx < self.hp as usize;
                return Some(if !inner { 0 } else if lit { PANEL_HEAP_LIT } else { PANEL_GAUGE_UNLIT });
            }
            return None;
        }
        if row == PANEL_CELL_ROW as usize {
            let dx = span(PANEL_CELL_COL, self.cell_hits.len() * 16)?;
            let c = dx / 16;
            return Some(heat_box(16, self.cell_hits[c], self.running && self.cell as usize == c, dx % 16));
        }
        None
    }
}

/// The picture the screen draws for `screen`: 640 × 480 pixels, row-major, each the 12-bit 0xRGB the board's DAC is
/// driven with — the text plane (the small-text window in the small font), and the activity panel over it. This is
/// the model the RTL's rendered frames are held to.
pub fn render(screen: &Screen) -> Vec<u16> {
    use super::terminal_protocol::{COLS, GLYPH_ROWS, H_TIMING, PALETTE, SMALL_COLS, V_TIMING};
    let (font, font8) = (font(), font8());
    let (w, h) = (H_TIMING.0 as usize, V_TIMING.0 as usize);
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (row, col) = ((y / GLYPH_ROWS) as u8, (x / 8) as u8);
            let in_window = screen
                .window
                .filter(|&(r, c, rs, cs)| rs > 0 && (r..r + rs).contains(&row) && (c..c + cs).contains(&col));
            let (cell, bits) = match in_window {
                Some((r, c, _, _)) => {
                    let srow = (row - r) as usize * 2 + (y % GLYPH_ROWS) / 8;
                    let cell = screen.small[srow * SMALL_COLS as usize + (col - c) as usize];
                    (cell, font8[cell.glyph as usize * 8 + y % 8])
                }
                None => {
                    let cell = screen.cells[row as usize * COLS as usize + col as usize];
                    (cell, font[cell.glyph as usize * GLYPH_ROWS + y % GLYPH_ROWS])
                }
            };
            let on = (bits >> (7 - x % 8)) & 1 == 1 && !(cell.blink && !screen.flash);
            let underline = screen.cursor == Some((row, col)) && screen.flash && y % GLYPH_ROWS >= 14;
            let lit = (on || underline) ^ (screen.pointer == Some((row, col)));
            let text = PALETTE[if lit { cell.fg } else { cell.bg } as usize];
            out.push(screen.panel.as_ref().and_then(|p| p.pixel(&font, x, y)).unwrap_or(text));
        }
    }
    out
}
