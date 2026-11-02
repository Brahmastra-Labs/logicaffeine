// nexys_video_wpe_stream — board top for the WHOLE STREAMED TRANSFORMER's config-driven engine on Board A
// (Nexys Video, XC7A200T). Identical to nexys_video_wpe EXCEPT it instantiates the ConfigBram streamed engine
// `wpe_soc_stream` (cell_* in registered-read BLOCK RAM, heap/sfile block RAM, WPE_STREAM caps) instead of the
// small `wpe_soc_fpga`. The host streams the whole net's flat config image + the pre-loaded weight blob (the
// 0x48 heap-preload segment) + params over the DPTI FIFO; the engine interprets the whole autoregressive
// generation on the fabric and returns the packed result 80113 = "mat" over the wire (the video board has no
// 7-seg, so the result comes back over UART — the host prints the tokens). Flash: openFPGALoader -c digilent_b.
// XDC: nexys_video_fifo.xdc.
module nexys_video_wpe_stream #(
  // The IEEE-754 FPU is a TOP-LEVEL knob so the build (`scripts/ser-build-200t.sh FPU=1|0`) sets it with one
  // `chparam` on this wrapper: an instance parameter written here would override any chparam on the engine
  // module itself — which is exactly how the deployed 200T image came to have no FPU while the stream fit
  // profile admitted float programs. WITH_FPU=1 is the LOGOS-language image (floats run: pi_leibniz, nbody,
  // `Show 1.5 + 2.25`); WITH_FPU=0 is the int8 transformer image (the float lane pruned as dead code).
  parameter WITH_FPU = 1
) (
  input  clk,                    // R4 100 MHz
  inout  [7:0] prog_d,           // FT245 async FIFO (DPTI) — bank 14
  input  prog_rxen, input prog_txen,
  output prog_rdn, output prog_wrn, output prog_oen, output prog_siwun, output prog_spien,
  output [7:0] led
);
  // /128 -> 781.25 kHz core clock (same deeply-conservative clock as nexys_video_wpe: the remaining
  // combinational config-memory reads — liv/konst/v — are a long path the open router's 7-series model
  // underestimates; cell_*/wr_*/blk_* are now registered BRAM reads so they are off the critical path).
  reg [6:0] divcnt = 7'd0; always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk; BUFG bufg_core (.I(divcnt[6]), .O(coreclk));

  reg [3:0] por = 4'd0; always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire soc_rst = ~por[3];

  reg [25:0] hb = 26'd0; always @(posedge coreclk) hb <= hb + 1'b1;
  assign led = {hb[25:19], ~por[3]};   // heartbeat + reset-active

  wire eng_rx, eng_tx;
  ft245_uart_bridge #(.CLKS_PER_BIT(81), .W(4)) bridge (
    .clk(coreclk), .rst(soc_rst),
    .eng_rx(eng_rx), .eng_tx(eng_tx),
    .prog_d(prog_d), .prog_rxen(prog_rxen), .prog_txen(prog_txen),
    .prog_rdn(prog_rdn), .prog_wrn(prog_wrn), .prog_oen(prog_oen),
    .prog_siwun(prog_siwun), .prog_spien(prog_spien)
  );

  // The whole-program engine (the streamed transformer AND every LOGOS program); the FPU follows the wrapper knob.
  wpe_soc_stream #(.CLKS_PER_BIT(81), .WITH_FPU(WITH_FPU)) engine (
    .clk(coreclk), .rst(soc_rst), .uart_rx(eng_rx), .uart_tx(eng_tx)
  );
endmodule
