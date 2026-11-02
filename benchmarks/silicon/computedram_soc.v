// computedram_soc — the ComputeDRAM engine on REAL SILICON (Nexys A7-100T, open flow).
//
// The proven dram_executor replays a raw sub-JEDEC ComputeDRAM AND stream (computedram_and.hex) into
// the behavioral ComputeDRAM device (computedram_model), which computes bitwise AND via triple-row
// activation (the sense-amp majority MAJ(a,b,0)) over an ON-CHIP array. That on-chip array stands in
// for the external DDR2 die: the analog DDR2 PHY (SSTL18/DQS/IDELAY) is the remaining hardware gate.
// The AND result is compared on-chip to the oracle (A & B) and streamed over UART as an 11-byte record
// {0xA5, result[63:0] MSB-first, status, 0x5A}, status=1 iff result == A & B. 100 MHz clk, 8N1 115200.
module computedram_soc #(parameter CLKS_PER_BIT = 868) (
  input clk, output reg uart_tx, output [15:0] led
);
  // Self (power-on) reset — hold reset for the first 64 cycles after configuration, then run.
  // Independent of any board switch (SW0), so the design self-starts on flash.
  reg [7:0] por_cnt = 8'd0;
  wire rst = (por_cnt < 8'd64);
  always @(posedge clk) if (por_cnt < 8'd64) por_cnt <= por_cnt + 8'd1;
  localparam [63:0] A_VAL = 64'hF0F0F0F0F0F0F0F0;
  localparam [63:0] B_VAL = 64'hFF00FF00FF00FF00;
  localparam [63:0] EXP   = 64'hF000F000F000F000; // A & B
  localparam [2:0] ROW_A = 3'd1, ROW_B = 3'd2, ROW_Z = 3'd3;
  localparam NW = 7; // AND stream: ACT,PRE,ACT,PRE,ACT,RD,PRE
  reg [63:0] rom [0:NW-1];
  initial $readmemh("computedram_and.hex", rom);

  localparam PH_PRE=2'd0, PH_RUN=2'd1, PH_SETTLE=2'd2, PH_REPORT=2'd3;
  reg [1:0] phase;
  reg [3:0] pl;
  integer idx;
  reg [15:0] settle;
  reg [63:0] result; reg passed;
  reg pre_we; reg [2:0] pre_addr; reg [63:0] pre_data;

  wire exec_rst = (phase==PH_PRE) | rst;
  wire cmd_ack, issue; wire [2:0] eop; wire [4:0] ebank; wire [31:0] erow;
  wire cmd_valid = (phase==PH_RUN) && (idx < NW);
  wire [63:0] cmd_word = (idx < NW) ? rom[idx] : 64'd0;
  wire [63:0] rd_out;

  dram_executor exec(.clk(clk), .rst(exec_rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                     .cmd_ack(cmd_ack), .issue(issue), .op(eop), .bank(ebank), .row(erow));
  computedram_model #(.AW(3), .DW(64)) memdev(.clk(clk), .rst(exec_rst), .issue(issue), .op(eop),
    .bank(ebank), .row(erow), .pre_we(pre_we), .pre_addr(pre_addr), .pre_data(pre_data),
    .rd_addr(ROW_A), .rd_out(rd_out));

  // Control: preload A,B,0 → run the stream → settle → capture + compare.
  always @(posedge clk) begin
    if (rst) begin
      phase<=PH_PRE; pl<=0; idx<=0; settle<=0; pre_we<=0; pre_addr<=0; pre_data<=0; result<=0; passed<=0;
    end else begin
      pre_we<=1'b0;
      case (phase)
        PH_PRE: case (pl)
          4'd0: begin pre_we<=1'b1; pre_addr<=ROW_A; pre_data<=A_VAL; pl<=4'd1; end
          4'd1: begin pre_we<=1'b1; pre_addr<=ROW_B; pre_data<=B_VAL; pl<=4'd2; end
          4'd2: begin pre_we<=1'b1; pre_addr<=ROW_Z; pre_data<=64'd0;  pl<=4'd3; end
          4'd3: pl<=4'd4;
          4'd4: pl<=4'd5;
          default: phase<=PH_RUN;
        endcase
        PH_RUN: begin
          if (cmd_ack) idx<=idx+1;
          if (idx>=NW) begin phase<=PH_SETTLE; settle<=16'd0; end
        end
        PH_SETTLE: if (settle>=16'd200) begin result<=rd_out; passed<=(rd_out==EXP); phase<=PH_REPORT; end
                   else settle<=settle+16'd1;
        PH_REPORT: ; // UART TX below streams the record
      endcase
    end
  end

  // UART TX: ALWAYS-ON. Each record latches the LIVE ROW_A word (= A & B once the compute finishes),
  // so transmission never depends on the control FSM's phase (robust + self-diagnosing).
  localparam S_IDLE=3'd0, S_LOAD=3'd1, S_START=3'd2, S_DATA=3'd3, S_STOP=3'd4, S_GAP=3'd5;
  reg [2:0] tstate; reg [15:0] clkc; reg [2:0] bidx; reg [7:0] tbyte; reg [3:0] pos; reg [15:0] gap;
  reg [63:0] snap; reg snap_pass;
  reg [7:0] recbyte;
  always @(*) begin
    case (pos)
      4'd0: recbyte = 8'hA5;
      4'd1: recbyte = snap[63:56];
      4'd2: recbyte = snap[55:48];
      4'd3: recbyte = snap[47:40];
      4'd4: recbyte = snap[39:32];
      4'd5: recbyte = snap[31:24];
      4'd6: recbyte = snap[23:16];
      4'd7: recbyte = snap[15:8];
      4'd8: recbyte = snap[7:0];
      4'd9: recbyte = snap_pass ? 8'h01 : 8'h00;
      default: recbyte = 8'h5A;
    endcase
  end
  always @(posedge clk) begin
    if (rst) begin
      tstate<=S_IDLE; uart_tx<=1'b1; clkc<=16'd0; bidx<=3'd0; pos<=4'd0; gap<=16'd0; tbyte<=8'd0;
      snap<=64'd0; snap_pass<=1'b0;
    end else case (tstate)
      S_IDLE:  begin uart_tx<=1'b1; snap<=rd_out; snap_pass<=(rd_out==EXP); pos<=4'd0; tstate<=S_LOAD; end
      S_LOAD:  begin tbyte<=recbyte; clkc<=16'd0; tstate<=S_START; end
      S_START: begin uart_tx<=1'b0; if (clkc==CLKS_PER_BIT-1) begin clkc<=16'd0; bidx<=3'd0; tstate<=S_DATA; end else clkc<=clkc+16'd1; end
      S_DATA:  begin uart_tx<=tbyte[bidx];
                 if (clkc==CLKS_PER_BIT-1) begin clkc<=16'd0; if (bidx==3'd7) tstate<=S_STOP; else bidx<=bidx+3'd1; end
                 else clkc<=clkc+16'd1; end
      S_STOP:  begin uart_tx<=1'b1;
                 if (clkc==CLKS_PER_BIT-1) begin clkc<=16'd0;
                   if (pos==4'd10) begin tstate<=S_GAP; gap<=16'd0; end
                   else begin pos<=pos+4'd1; tstate<=S_LOAD; end
                 end else clkc<=clkc+16'd1; end
      S_GAP:   begin uart_tx<=1'b1; if (gap>=16'd60000) begin gap<=16'd0; tstate<=S_IDLE; end else gap<=gap+16'd1; end
      default: tstate<=S_IDLE;
    endcase
  end
  assign led = {passed, phase, 5'd0, idx[7:0]};
endmodule
