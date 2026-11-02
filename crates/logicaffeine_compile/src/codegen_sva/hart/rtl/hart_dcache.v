// hart_dcache — the hart's data cache over the DDR2: 2^INDEX_BITS direct-mapped 64-byte lines in block RAM,
// write-back and write-allocate. It answers only accesses inside the RAM; the SoC routes every other address elsewhere.
//
// When E hands a memory access to M, the cache reads the line's tag and the word at that same edge, so M learns a hit
// and has its data in the cycle it presents the access. A store hit writes its bytes at the end of that cycle. When a
// store and the next access's read land on the same word at the same edge, the read is completed with the stored
// bytes, whatever the block RAM returns for a read during a write. A miss writes a dirty victim back, fills the line,
// reads the word again and answers.
//
// `flush` (fence.i, a restart) is held until `flushed`: every dirty line is written back, a clean line costs a cycle.
module hart_dcache #(
  parameter [63:0] RAM_BASE = 64'h8000_0000,
  parameter        RAM_BITS = 27,
  parameter        INDEX_BITS = 8
) (
  input             clk,
  input             rst,
  input             pre_valid,
  input      [63:0] pre_addr,
  input             req,
  input      [63:0] addr,
  input             we,
  input      [7:0]  be,
  input      [63:0] wdata,
  output            ready,
  output     [63:0] rdata,
  // line transactions with memory: a request held until granted, then eight words in order either way
  output reg        mem_req,
  output reg        mem_we,
  output reg [63:0] mem_addr,
  input             mem_gnt,
  output     [63:0] mem_wdata,
  output reg        mem_wvalid,
  input             mem_wready,
  input             mem_rvalid,
  input      [63:0] mem_rdata,
  input             flush,
  output reg        flushed
);
  localparam LINES = 1 << INDEX_BITS;
  localparam TAG_BITS = RAM_BITS - 6 - INDEX_BITS;
  localparam [3:0] S_IDLE = 4'd0, S_WB_GRANT = 4'd1, S_WB_READ = 4'd2, S_WB_SEND = 4'd3, S_FILL_GRANT = 4'd4,
                   S_FILL = 4'd5, S_REREAD = 4'd6, S_FLUSH = 4'd7, S_FLUSH_TAG = 4'd8;

  reg [3:0]  state;
  (* ram_style = "block" *) reg [TAG_BITS-1:0] tags [0:LINES - 1];
  reg [LINES-1:0] valid, dirty;
  wire [63:0] data_out;
  reg [TAG_BITS-1:0] tag_out;

  wire [INDEX_BITS-1:0] line = addr[6 +: INDEX_BITS];
  wire [TAG_BITS-1:0]   tag = addr[6 + INDEX_BITS +: TAG_BITS];
  wire [INDEX_BITS+2:0] idx = addr[3 +: INDEX_BITS + 3];
  wire [INDEX_BITS+2:0] pre_idx = pre_addr[3 +: INDEX_BITS + 3];

  // The bytes the last store wrote to the word the current access read at the same edge.
  reg        merge;
  reg [7:0]  merge_be;
  reg [63:0] merge_data;
  genvar mi;
  generate for (mi = 0; mi < 8; mi = mi + 1) begin : m
    assign rdata[8 * mi +: 8] = (merge && merge_be[mi]) ? merge_data[8 * mi +: 8] : data_out[8 * mi +: 8];
  end endgenerate

  wire hit = state == S_IDLE && req && valid[line] && tag_out == tag;
  wire store_hit = hit && we;
  assign ready = hit;

  // The walker that serves a miss or a flush: the line, the word, whether it fills (else it flushes), and the
  // missing line's own tag — the walker finishes a fill without the requester.
  reg [INDEX_BITS-1:0] w_line;
  reg [2:0]  w_word;
  reg        w_filling;
  reg [TAG_BITS-1:0] w_tag;
  wire [63:0] w_fill_addr = RAM_BASE | {{(64 - RAM_BITS){1'b0}}, w_tag, w_line, 6'd0};
  wire       wb_next = state == S_WB_SEND && mem_wready && w_word != 3'd7;

  wire        rd_en = (state == S_IDLE && pre_valid) || state == S_WB_READ || wb_next || state == S_REREAD
                    || state == S_FLUSH;
  wire [INDEX_BITS+2:0] rd_idx = (state == S_IDLE) ? pre_idx
                               : (state == S_REREAD) ? idx
                               : {w_line, (state == S_WB_SEND) ? w_word + 3'd1 : w_word};
  wire [INDEX_BITS-1:0] rd_line = (state == S_IDLE) ? pre_idx[3 +: INDEX_BITS] : w_line;
  wire        fill_write = state == S_FILL && mem_rvalid;
  wire        wr_en = store_hit || fill_write;
  wire [INDEX_BITS+2:0] wr_idx = fill_write ? {w_line, w_word} : idx;
  wire [7:0]  wr_be = fill_write ? 8'hff : be;
  wire [63:0] wr_data = fill_write ? mem_rdata : wdata;

  // The words in eight byte-wide block RAMs, so a store's bytes are each a plain write.
  genvar bl;
  generate for (bl = 0; bl < 8; bl = bl + 1) begin : lane
    (* ram_style = "block" *) reg [7:0] bytes [0:LINES * 8 - 1];
    reg [7:0] out;
    always @(posedge clk) begin
      if (rd_en) out <= bytes[rd_idx];
      if (wr_en && wr_be[bl]) bytes[wr_idx] <= wr_data[8 * bl +: 8];
    end
    assign data_out[8 * bl +: 8] = out;
  end endgenerate

  always @(posedge clk) begin
    if (rd_en) tag_out <= tags[rd_line];
    if (fill_write && w_word == 3'd7) tags[w_line] <= w_tag;
  end

  always @(posedge clk) begin
    if (rst) merge <= 1'b0;
    else if (state == S_IDLE && pre_valid) begin
      merge <= store_hit && pre_idx == idx;
      merge_be <= be;
      merge_data <= wdata;
    end else if (state != S_IDLE) merge <= 1'b0;
  end

  assign mem_wdata = data_out;

  always @(posedge clk) begin
    if (rst) begin
      state <= S_IDLE;
      valid <= {LINES{1'b0}};
      dirty <= {LINES{1'b0}};
      mem_req <= 1'b0;
      mem_wvalid <= 1'b0;
      flushed <= 1'b0;
    end else begin
      flushed <= 1'b0;
      case (state)
        S_IDLE: begin
          if (store_hit) dirty[line] <= 1'b1;
          if (req && !hit) begin
            // A miss: the victim's tag is the one read with this access, so a dirty victim goes back first.
            w_line <= line;
            w_tag <= tag;
            w_filling <= 1'b1;
            mem_req <= 1'b1;
            if (valid[line] && dirty[line]) begin
              mem_we <= 1'b1;
              mem_addr <= RAM_BASE | {{(64 - RAM_BITS){1'b0}}, tag_out, line, 6'd0};
              state <= S_WB_GRANT;
            end else begin
              mem_we <= 1'b0;
              mem_addr <= {addr[63:6], 6'd0};
              state <= S_FILL_GRANT;
            end
          end else if (flush && !flushed && !req) begin
            w_line <= {INDEX_BITS{1'b0}};
            w_filling <= 1'b0;
            state <= S_FLUSH;
          end
        end
        S_WB_GRANT: if (mem_gnt) begin
          mem_req <= 1'b0;
          w_word <= 3'd0;
          state <= S_WB_READ;
        end
        S_WB_READ: begin
          mem_wvalid <= 1'b1;
          state <= S_WB_SEND;
        end
        S_WB_SEND: if (mem_wready) begin
          w_word <= w_word + 3'd1;
          if (w_word == 3'd7) begin
            mem_wvalid <= 1'b0;
            dirty[w_line] <= 1'b0;
            if (w_filling) begin
              mem_req <= 1'b1;
              mem_we <= 1'b0;
              mem_addr <= w_fill_addr;
              state <= S_FILL_GRANT;
            end else if (w_line == {INDEX_BITS{1'b1}}) begin
              flushed <= 1'b1;
              state <= S_IDLE;
            end else begin
              w_line <= w_line + 1'b1;
              state <= S_FLUSH;
            end
          end
        end
        S_FILL_GRANT: if (mem_gnt) begin
          mem_req <= 1'b0;
          w_word <= 3'd0;
          state <= S_FILL;
        end
        S_FILL: if (mem_rvalid) begin
          w_word <= w_word + 3'd1;
          if (w_word == 3'd7) begin
            valid[w_line] <= 1'b1;
            dirty[w_line] <= 1'b0;
            state <= S_REREAD;
          end
        end
        S_REREAD: state <= S_IDLE;
        // The flush walks every line: a dirty one has its tag read (this cycle), then goes back.
        S_FLUSH: begin
          if (valid[w_line] && dirty[w_line]) begin
            state <= S_FLUSH_TAG;
          end else if (w_line == {INDEX_BITS{1'b1}}) begin
            flushed <= 1'b1;
            state <= S_IDLE;
          end else begin
            w_line <= w_line + 1'b1;
          end
        end
        S_FLUSH_TAG: begin
          mem_req <= 1'b1;
          mem_we <= 1'b1;
          mem_addr <= RAM_BASE | {{(64 - RAM_BITS){1'b0}}, tag_out, w_line, 6'd0};
          state <= S_WB_GRANT;
        end
        default: state <= S_IDLE;
      endcase
    end
  end
endmodule
