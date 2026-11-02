// hart_icache — the hart's instruction fetch: 2^INDEX_BITS direct-mapped 64-byte lines of DDR2 code in block RAM, and
// the boot ROM read directly. A request is answered the cycle after it is made when it hits (or reads the ROM), so
// fetch runs a word a cycle; a miss fills its line from memory, eight words in order, then answers. One request is
// outstanding at a time, and the next may be made in the cycle the answer comes. The hart fetches only from the ROM
// and the RAM, so every other request is one of the two.
//
// `invalidate` (fence.i, after the data cache has written memory back) is held until `invalidated`, which waits for a
// fill already under way, so no line read before the write-back outlives the invalidation.
module hart_icache #(
  parameter [63:0] RAM_BASE = 64'h8000_0000,
  parameter        RAM_BITS = 27,
  parameter        INDEX_BITS = 8,
  parameter [63:0] ROM_BASE = 64'h0001_0000,
  parameter [63:0] ROM_SIZE = 64'h4000
) (
  input             clk,
  input             rst,
  input             if_req,
  input      [63:0] if_addr,
  output            if_ready,
  output     [63:0] if_rdata,
  // the boot ROM's fetch port: a word index now, its word the next cycle
  output            rom_en,
  output     [63:0] rom_index,
  input      [63:0] rom_word,
  // line fills from memory: a request held until granted, then eight words in order
  output reg        fill_req,
  output     [63:0] fill_addr,
  input             fill_gnt,
  input             fill_valid,
  input      [63:0] fill_data,
  input             invalidate,
  output reg        invalidated
);
  localparam LINES = 1 << INDEX_BITS;
  localparam TAG_BITS = RAM_BITS - 6 - INDEX_BITS;
  localparam [2:0] S_IDLE = 3'd0, S_GRANT = 3'd1, S_FILL = 3'd2, S_REREAD = 3'd3, S_ANSWER = 3'd4;
  reg [2:0]  state;
  reg        req_valid, req_rom;
  reg [63:0] req_addr;
  reg [2:0]  beat;

  (* ram_style = "block" *) reg [63:0] data [0:LINES * 8 - 1];
  (* ram_style = "block" *) reg [TAG_BITS-1:0] tags [0:LINES - 1];
  reg [LINES-1:0] valid;
  reg [63:0] data_out;
  reg [TAG_BITS-1:0] tag_out;

  wire [INDEX_BITS-1:0] req_line = req_addr[6 +: INDEX_BITS];
  wire [TAG_BITS-1:0]   req_tag = req_addr[6 + INDEX_BITS +: TAG_BITS];
  wire hit = req_valid && !req_rom && valid[req_line] && tag_out == req_tag;

  assign if_ready = (state == S_IDLE && req_valid && (req_rom || hit)) || state == S_ANSWER;
  assign if_rdata = req_rom ? rom_word : data_out;

  // A new request is taken whenever none is waiting on the cache.
  wire accept = if_req && (state == S_IDLE ? (!req_valid || req_rom || hit) : state == S_ANSWER);
  wire is_rom = (if_addr >= ROM_BASE) && (if_addr < ROM_BASE + ROM_SIZE);
  wire [63:0] rom_off = if_addr - ROM_BASE;
  assign rom_en = accept && is_rom;
  assign rom_index = {3'd0, rom_off[63:3]};
  assign fill_addr = {req_addr[63:6], 6'd0};

  wire        rd_en  = (accept && !is_rom) || state == S_REREAD;
  wire [INDEX_BITS+2:0] rd_idx = state == S_REREAD ? req_addr[3 +: INDEX_BITS + 3] : if_addr[3 +: INDEX_BITS + 3];
  wire [INDEX_BITS-1:0] rd_line = state == S_REREAD ? req_line : if_addr[6 +: INDEX_BITS];

  always @(posedge clk) begin
    if (rd_en) begin
      data_out <= data[rd_idx];
      tag_out <= tags[rd_line];
    end
    if (state == S_FILL && fill_valid) data[{req_line, beat}] <= fill_data;
    if (state == S_FILL && fill_valid && beat == 3'd7) tags[req_line] <= req_tag;
  end

  always @(posedge clk) begin
    if (rst) begin
      state <= S_IDLE;
      req_valid <= 1'b0;
      fill_req <= 1'b0;
      valid <= {LINES{1'b0}};
      invalidated <= 1'b0;
    end else begin
      invalidated <= 1'b0;
      case (state)
        S_IDLE: begin
          if (req_valid && !req_rom && !hit) begin
            state <= S_GRANT;
            fill_req <= 1'b1;
          end else if (accept) begin
            req_valid <= 1'b1;
            req_rom <= is_rom;
            req_addr <= if_addr;
          end else if (req_valid) begin
            req_valid <= 1'b0;
          end
        end
        S_GRANT: if (fill_gnt) begin
          fill_req <= 1'b0;
          beat <= 3'd0;
          state <= S_FILL;
        end
        S_FILL: if (fill_valid) begin
          beat <= beat + 3'd1;
          if (beat == 3'd7) begin
            valid[req_line] <= 1'b1;
            state <= S_REREAD;
          end
        end
        S_REREAD: state <= S_ANSWER;
        default: begin
          if (accept) begin
            req_valid <= 1'b1;
            req_rom <= is_rom;
            req_addr <= if_addr;
          end else begin
            req_valid <= 1'b0;
          end
          state <= S_IDLE;
        end
      endcase
      if (invalidate && !invalidated && state == S_IDLE) begin
        valid <= {LINES{1'b0}};
        invalidated <= 1'b1;
      end
    end
  end
endmodule
