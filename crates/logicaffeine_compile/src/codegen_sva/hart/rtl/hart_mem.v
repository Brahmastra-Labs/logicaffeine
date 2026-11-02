// hart_mem — the caches' one way to the DDR2: line transactions from the instruction cache (fills) and the data cache
// (fills and write-backs) onto one memory port, a whole transaction at a time. A request is held until granted; a
// granted write then hands over its eight words, a granted read gets its eight words back in order. The data cache
// goes first when both ask.
//
// The memory port promises in-order transactions, so a line read after a write-back of it reads what was written.
module hart_mem (
  input             clk,
  input             rst,
  input             i_req,
  input      [63:0] i_addr,
  output            i_gnt,
  output            i_valid,
  output     [63:0] i_data,
  input             d_req,
  input             d_we,
  input      [63:0] d_addr,
  output            d_gnt,
  input      [63:0] d_wdata,
  input             d_wvalid,
  output            d_wready,
  output            d_rvalid,
  output     [63:0] d_rdata,
  output            m_req,
  output            m_we,
  output     [63:0] m_addr,
  input             m_gnt,
  output     [63:0] m_wdata,
  output            m_wvalid,
  input             m_wready,
  input             m_rvalid,
  input      [63:0] m_rdata
);
  reg       busy, owner_d, writing;
  reg [2:0] beats;

  wire pick_d = d_req;
  assign m_req  = !busy && (d_req || i_req);
  assign m_we   = pick_d && d_we;
  assign m_addr = pick_d ? d_addr : i_addr;
  assign i_gnt  = m_req && m_gnt && !pick_d;
  assign d_gnt  = m_req && m_gnt && pick_d;

  assign m_wdata  = d_wdata;
  assign m_wvalid = busy && owner_d && writing && d_wvalid;
  assign d_wready = busy && owner_d && writing && m_wready;
  assign i_valid  = busy && !owner_d && m_rvalid;
  assign d_rvalid = busy && owner_d && !writing && m_rvalid;
  assign i_data   = m_rdata;
  assign d_rdata  = m_rdata;

  wire beat = writing ? (m_wvalid && m_wready) : (busy && m_rvalid);
  always @(posedge clk) begin
    if (rst) begin
      busy <= 1'b0;
    end else if (!busy) begin
      if (m_req && m_gnt) begin
        busy <= 1'b1;
        owner_d <= pick_d;
        writing <= m_we;
        beats <= 3'd0;
      end
    end else if (beat) begin
      beats <= beats + 3'd1;
      if (beats == 3'd7) busy <= 1'b0;
    end
  end
endmodule
