// term_vga — the Fabric Terminal's screen: 640 x 480 at 60 Hz, one pixel every __N_CLOCKS_PER_PIXEL__ board clocks, drawing
// __N_COLS__ x __N_ROWS__ text cells of 8 x 16 pixels from the text buffer through the font ROM, with a text cursor, a pointer, and
// an overlay input that paints over the text wherever `ov_on` is set (the activity panel).
//
// A cell is 16 bits {bg[3:0], fg[3:0], blink, glyph[6:0]} at address row << 7 | col. Each cell's glyph row is
// fetched during the cell before it — text buffer, then font ROM, two registered reads in the 32 board clocks a
// cell lasts — so the pixel path is a bit select and a palette lookup. Blink and the cursor flash together, each
// phase 2^FLASH_BIT frames (32, about half a second).
//
// The small-text window (`win_*`, in text cells; `win_rows` 0 closes it) shows the small plane instead: buffer rows
// from __N_SMALL_FIRST_ROW__, two of them to each text row, each 8 pixels tall in the small font. A cell in the window
// fetches its small-plane cell and small-font row in the same two reads, so the timing is the same.

// The text buffer: one write/read port for the link, one read port for the screen. The attribute and glyph bytes
// are block RAMs of their own, eight bits wide, so no bit of a cell rides a RAMB's parity lane. Rows 0..31 are the
// screen's, rows 32..63 the small plane's.
module term_textbuf (
  input             clk,
  input             a_we,
  input      [12:0] a_addr,
  input      [15:0] a_wdata,
  output reg [15:0] a_rdata,
  input      [12:0] b_addr,
  output reg [15:0] b_rdata
);
  (* ram_style = "block" *) reg [7:0] attr [0:8191];
  (* ram_style = "block" *) reg [7:0] glyph [0:8191];
  integer i;
  initial for (i = 0; i < 8192; i = i + 1) begin attr[i] = 8'h07; glyph[i] = 8'h20; end
  always @(posedge clk) begin
    if (a_we) begin attr[a_addr] <= a_wdata[15:8]; glyph[a_addr] <= a_wdata[7:0]; end
    a_rdata <= {attr[a_addr], glyph[a_addr]};
  end
  always @(posedge clk) b_rdata <= {attr[b_addr], glyph[b_addr]};
endmodule

// The font: 128 glyphs x 16 rows, a glyph row per byte, MSB the leftmost pixel.
module term_font (
  input            clk,
  input      [10:0] addr,
  output reg [7:0] data
);
  (* ram_style = "block" *) reg [7:0] rom [0:2047];
  initial $readmemh("term_font.hex", rom);
  always @(posedge clk) data <= rom[addr];
endmodule

// The small font: 128 glyphs x 8 rows, made from the font above (terminal_rtl::font8).
module term_font8 (
  input            clk,
  input      [9:0] addr,
  output reg [7:0] data
);
  (* ram_style = "block" *) reg [7:0] rom [0:1023];
  initial $readmemh("term_font8.hex", rom);
  always @(posedge clk) data <= rom[addr];
endmodule

module term_vga #(parameter FLASH_BIT = 5) (
  input             clk,
  input             rst,
  output reg [12:0] tb_addr,
  input      [15:0] tb_data,
  input      [6:0]  win_row,
  input      [6:0]  win_col,
  input      [6:0]  win_rows,
  input      [6:0]  win_cols,
  input      [6:0]  cursor_row,
  input      [6:0]  cursor_col,
  input             cursor_on,
  input      [6:0]  pointer_row,
  input      [6:0]  pointer_col,
  input             pointer_on,
  input             ov_on,
  input      [11:0] ov_rgb,
  output     [9:0]  px,
  output     [9:0]  py,
  output            pix,
  output     [1:0]  phase,
  output            frame_start,
  output reg [3:0]  r,
  output reg [3:0]  g,
  output reg [3:0]  b,
  output reg        hs,
  output reg        vs
);
  localparam HV = __N_H_VISIBLE__, HF = __N_H_FRONT__, HS = __N_H_SYNC__, HB = __N_H_BACK__;
  localparam VV = __N_V_VISIBLE__, VF = __N_V_FRONT__, VS = __N_V_SYNC__, VB = __N_V_BACK__;
  localparam HT = HV + HF + HS + HB, VT = VV + VF + VS + VB;
  localparam CPP = __N_CLOCKS_PER_PIXEL__;

  // The pixel clock enable, and the beam position.
  reg [1:0] pdiv = 2'd0;
  assign pix = (pdiv == CPP - 1);
  assign phase = pdiv;
  always @(posedge clk) pdiv <= pix ? 2'd0 : pdiv + 1'b1;
  reg [9:0] hx = 10'd0, vy = 10'd0;
  assign px = hx;
  assign py = vy;
  assign frame_start = pix && hx == 10'd0 && vy == 10'd0;
  always @(posedge clk) if (pix) begin
    if (hx == HT - 1) begin
      hx <= 10'd0;
      vy <= (vy == VT - 1) ? 10'd0 : vy + 1'b1;
    end else hx <= hx + 1'b1;
  end

  reg [5:0] frames = 6'd0;
  always @(posedge clk) if (frame_start) frames <= frames + 1'b1;
  wire flash = frames[FLASH_BIT];

  // Prefetch: during the cell at column c, fetch the cell after it (the next line's first cell during the
  // line's last one). `nx_col`/`nx_line` name it; the fetch runs in four steps off the pixel enable.
  wire [6:0] col = hx[9:3];
  wire       last_cell = (hx[9:3] == (HT / 8) - 1);
  wire [6:0] nx_col = last_cell ? 7'd0 : col + 1'b1;
  wire [9:0] nx_line = last_cell ? ((vy == VT - 1) ? 10'd0 : vy + 1'b1) : vy;
  wire       nx_text = (nx_col < __N_COLS__) && (nx_line < VV);

  reg [10:0] font_addr = 11'd0;
  wire [7:0] font_data;
  term_font font (.clk(clk), .addr(font_addr), .data(font_data));
  reg [9:0]  font8_addr = 10'd0;
  wire [7:0] font8_data;
  term_font8 font8 (.clk(clk), .addr(font8_addr), .data(font8_data));

  // The next cell in the small-text window: its small-plane row (two to a text row, the top one first) and column.
  wire [6:0] nx_row = {2'b0, nx_line[8:4]};
  wire       nx_win = nx_text && win_rows != 7'd0 && nx_row >= win_row && nx_row < win_row + win_rows &&
                      nx_col >= win_col && nx_col < win_col + win_cols;
  wire [6:0] nx_srow = __N_SMALL_FIRST_ROW__ + ((nx_row - win_row) << 1) + nx_line[3];
  wire [6:0] nx_scol = nx_col - win_col;
  reg        nx_win_r = 1'b0;

  reg [2:0]  fetch = 3'd0;
  reg [15:0] nx_cell = 16'd0;
  reg [7:0]  nx_bits = 8'd0;
  reg        nx_valid = 1'b0;
  reg [6:0]  nx_row_r = 7'd0, nx_col_r = 7'd0;
  always @(posedge clk) begin
    if (pix && hx[2:0] == 3'd0) begin
      tb_addr <= nx_win ? {nx_srow[5:0], nx_scol} : {1'b0, nx_line[8:4], nx_col};
      nx_win_r <= nx_win;
      nx_row_r <= {2'b0, nx_line[8:4]};
      nx_col_r <= nx_col;
      nx_valid <= nx_text;
      fetch <= 3'd1;
    end else if (fetch == 3'd1) fetch <= 3'd2;
    else if (fetch == 3'd2) begin
      nx_cell <= tb_data;
      font_addr <= {tb_data[6:0], nx_line[3:0]};
      font8_addr <= {tb_data[6:0], nx_line[2:0]};
      fetch <= 3'd3;
    end else if (fetch == 3'd3) fetch <= 3'd4;
    else if (fetch == 3'd4) begin
      nx_bits <= nx_win_r ? font8_data : font_data;
      fetch <= 3'd0;
    end
  end

  // The cell being drawn: taken from the prefetch at the first pixel of each cell.
  reg [15:0] here = 16'd0;
  reg [7:0]  bits = 8'd0;
  reg        valid = 1'b0;
  reg        is_cursor = 1'b0, is_pointer = 1'b0;
  reg [3:0]  glyph_row = 4'd0;
  always @(posedge clk) if (pix && hx[2:0] == 3'd7) begin
    here <= nx_cell;
    bits <= nx_bits;
    valid <= nx_valid;
    glyph_row <= nx_line[3:0];
    is_cursor <= cursor_on && nx_row_r == cursor_row && nx_col_r == cursor_col;
    is_pointer <= pointer_on && nx_row_r == pointer_row && nx_col_r == pointer_col;
  end

  // The pixel: glyph bit, blink, cursor (an underline on the cell's last two rows, flashing), pointer (inverted).
  wire [2:0] bitpos = hx[2:0];
  wire       on = bits[3'd7 - bitpos] && !(here[7] && !flash);
  wire       underline = is_cursor && flash && glyph_row >= 4'd14;
  wire       lit = (on || underline) ^ is_pointer;
  wire [3:0] colour = lit ? here[11:8] : here[15:12];
  function [11:0] palette;
    input [3:0] index;
    case (index)
__PALETTE_CASE__
      default: palette = 12'h000;
    endcase
  endfunction
  wire [11:0] rgb = palette(colour);

  wire visible = (hx < HV) && (vy < VV);
  always @(posedge clk) if (pix) begin
    hs <= !((hx >= HV + HF) && (hx < HV + HF + HS));
    vs <= !((vy >= VV + VF) && (vy < VV + VF + VS));
    if (!visible) {r, g, b} <= 12'h000;
    else if (ov_on) {r, g, b} <= ov_rgb;
    else if (valid) {r, g, b} <= rgb;
    else {r, g, b} <= 12'h000;
  end
endmodule
