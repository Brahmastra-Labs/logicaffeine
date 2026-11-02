// tb_term_ps2 — term_ps2 against a PS/2 device model on open-collector lines with pull-ups, at the protocol's
// timing (a 12.5 kHz device clock). The device sends a keyboard's make/break/extended codes, a byte with bad
// parity, a frame abandoned mid-byte, and rides out a clock glitch; the host sends the mouse's enable command
// 0xF4 and the device clocks it in and acknowledges, then a device that never answers; then keyboards clocking at
// 30 kHz (the fastest the Nexys A7's bridge clocks one) and at 50 kHz, each with the shortest data setup PS/2
// allows. The line monitor counts the device's clock edges (not the host's own inhibit) and times the shortest
// low phase. Prints one line per check, `PASS: <what>` or `FAIL: <what> …`, and `DONE` at the end.
`timescale 1ns/1ps
module tb_term_ps2;
  reg clk = 1'b0;
  always #5 clk = ~clk;
  reg rst = 1'b1;

  wire ps2_clk_w, ps2_data_w;
  pullup (ps2_clk_w);
  pullup (ps2_data_w);
  reg dev_clk_low = 1'b0, dev_data_low = 1'b0;
  wire dut_clk_low, dut_data_low;
  assign ps2_clk_w  = (dev_clk_low  || dut_clk_low)  ? 1'b0 : 1'bz;
  assign ps2_data_w = (dev_data_low || dut_data_low) ? 1'b0 : 1'bz;

  wire [7:0] rx_byte, rx_status, line_min_low;
  wire [15:0] line_falls, line_bytes;
  wire [7:0]  line_errors;
  wire       rx_valid, tx_busy, tx_done, tx_ack, line_fell;
  reg  [7:0] tx_byte = 8'd0;
  reg        tx_start = 1'b0;
  term_ps2 #(.CLK_HZ(100_000_000), .TX_TIMEOUT_US(3000)) dut (
    .clk(clk), .rst(rst), .ps2_clk_in(ps2_clk_w), .ps2_data_in(ps2_data_w),
    .ps2_clk_low(dut_clk_low), .ps2_data_low(dut_data_low),
    .rx_byte(rx_byte), .rx_status(rx_status), .rx_valid(rx_valid),
    .tx_byte(tx_byte), .tx_start(tx_start), .tx_busy(tx_busy), .tx_done(tx_done), .tx_ack(tx_ack),
    .line_falls(line_falls), .line_min_low(line_min_low), .line_fell(line_fell),
    .line_bytes(line_bytes), .line_errors(line_errors));
  integer fell_pulses = 0;
  always @(posedge clk) if (line_fell) fell_pulses <= fell_pulses + 1;

  // Everything the port reports, in order.
  reg [15:0] got [0:63];
  integer ngot = 0;
  always @(posedge clk) if (rx_valid) begin got[ngot] <= {rx_status, rx_byte}; ngot <= ngot + 1; end
  reg done_seen = 1'b0, ack_seen = 1'b0;
  always @(posedge clk) if (tx_done) begin done_seen <= 1'b1; ack_seen <= tx_ack; end

  // The device sends one frame: start, 8 data bits LSB first, odd parity (inverted when `bad`), stop. Data
  // changes `hold` ns after the clock rises and `setup` ns before it falls; the clock stays low `low` ns. The host
  // reads the data at the falling edge.
  integer hold = 20000, setup = 20000, low = 40000;
  task dev_bit(input b);
    begin
      #(hold);
      dev_data_low = !b;
      #(setup);
      dev_clk_low = 1'b1;
      #(low);
      dev_clk_low = 1'b0;
    end
  endtask
  task dev_send(input [7:0] v, input bad);
    integer i;
    begin
      dev_bit(1'b0);
      for (i = 0; i < 8; i = i + 1) dev_bit(v[i]);
      dev_bit((~^v) ^ bad);
      dev_bit(1'b1);
      dev_data_low = 1'b0;
      #100000;
    end
  endtask

  // The device takes one host-to-device byte: wait for the host's request to send (clock released with data
  // low), clock in 8 data bits, parity and stop on rising edges, then pull data low for the acknowledge clock.
  reg [10:0] dev_rx;
  task dev_receive(output [7:0] v, output parity_ok);
    integer i;
    begin
      wait (ps2_clk_w == 1'b0);
      wait (ps2_clk_w == 1'b1);
      if (ps2_data_w !== 1'b0) $display("FAIL: host-to-device: no start bit after the inhibit");
      for (i = 0; i < 10; i = i + 1) begin
        #40000; dev_clk_low = 1'b1;
        #40000; dev_clk_low = 1'b0;
        #1000;  dev_rx[i] = ps2_data_w;
      end
      #20000; dev_data_low = 1'b1;
      #20000; dev_clk_low = 1'b1;
      #40000; dev_clk_low = 1'b0;
      #20000; dev_data_low = 1'b0;
      v = dev_rx[7:0];
      parity_ok = (^dev_rx[8:0]) == 1'b1 && dev_rx[9] == 1'b1;
    end
  endtask

  integer k;
  reg [7:0] seen;
  reg       pok;
  reg [7:0] kbd [0:7];
  initial begin
    kbd[0] = 8'h1C; kbd[1] = 8'hF0; kbd[2] = 8'h1C; kbd[3] = 8'hE0;
    kbd[4] = 8'h75; kbd[5] = 8'hE0; kbd[6] = 8'hF0; kbd[7] = 8'h75;
    #200 rst = 1'b0;
    #1000;

    // 0. Before the device sends anything, a send nobody answers: the host's own inhibit is no edge and no low phase.
    @(negedge clk); tx_byte = 8'hFF; tx_start = 1'b1; @(negedge clk); tx_start = 1'b0;
    wait (done_seen);
    if (line_falls == 0 && line_min_low == 8'hFF) $display("PASS: the host's own inhibit is neither counted nor timed");
    else $display("FAIL: line monitor after the host's inhibit: %0d edges, shortest low %0d us", line_falls, line_min_low);
    done_seen = 1'b0;
    #100000;

    // 1. A keyboard: 'A' pressed and released, then the extended up-arrow pressed and released.
    for (k = 0; k < 8; k = k + 1) dev_send(kbd[k], 1'b0);
    #10000;
    for (k = 0; k < 8; k = k + 1)
      if (got[k] !== {8'h00, kbd[k]}) $display("FAIL: keyboard byte %0d: got status %02h byte %02h, sent %02h", k, got[k][15:8], got[k][7:0], kbd[k]);
    if (ngot == 8) $display("PASS: keyboard make/break/extended codes arrive in order, undamaged");
    else $display("FAIL: keyboard: %0d bytes reported for 8 sent", ngot);
    if (line_falls == 88 && fell_pulses == 88 && (line_min_low == 39 || line_min_low == 40) && line_bytes == 8 && line_errors == 0)
      $display("PASS: the line monitor counts the device's 88 clock edges and 8 bytes, and times its 40 us low phase");
    else $display("FAIL: line monitor: %0d edges (%0d pulses) for 88, %0d bytes, %0d damaged, shortest low %0d us for 40",
                  line_falls, fell_pulses, line_bytes, line_errors, line_min_low);

    // 2. A byte with the wrong parity is reported damaged.
    dev_send(8'h55, 1'b1);
    #10000;
    if (ngot == 9 && got[8] === {__V_PS2_PARITY_ERROR__, 8'h55} && line_errors == 1 && line_bytes == 8)
      $display("PASS: a parity error is reported with the byte, and counted damaged");
    else $display("FAIL: parity error: %0d bytes, last %04h, monitor %0d whole %0d damaged", ngot, got[ngot-1], line_bytes, line_errors);

    // 3. A frame abandoned after five bits, then silence past the timeout, then a whole byte.
    dev_bit(1'b0); dev_bit(1'b1); dev_bit(1'b0); dev_bit(1'b1); dev_bit(1'b1);
    dev_data_low = 1'b0;
    #600000;
    dev_send(8'h29, 1'b0);
    #10000;
    if (ngot == 10 && got[9] === {8'h00, 8'h29}) $display("PASS: a stalled frame is dropped and the next byte is read whole");
    else $display("FAIL: resync: %0d bytes, last %04h", ngot, got[ngot-1]);

    // 4. A 2 us glitch on the idle clock is not an edge.
    dev_clk_low = 1'b1; #2000; dev_clk_low = 1'b0;
    #200000;
    dev_send(8'h5A, 1'b0);
    #10000;
    if (ngot == 11 && got[10] === {8'h00, 8'h5A}) $display("PASS: a clock glitch shorter than the debounce window is ignored");
    else $display("FAIL: glitch: %0d bytes, last %04h", ngot, got[ngot-1]);
    if (line_falls == 127 && line_min_low <= 2 && line_min_low >= 1)
      $display("PASS: the line monitor counts the glitch the reader ignores, and times it under the debounce window");
    else $display("FAIL: line monitor after the glitch: %0d edges for 127, shortest low %0d us", line_falls, line_min_low);

    // 5. The host enables a mouse: 0xF4 goes out, the device acknowledges, and answers 0xFA.
    @(negedge clk); tx_byte = 8'hF4; tx_start = 1'b1; @(negedge clk); tx_start = 1'b0;
    dev_receive(seen, pok);
    wait (done_seen);
    if (seen == 8'hF4 && pok && ack_seen) $display("PASS: host-to-device 0xF4 is clocked in with odd parity and acknowledged");
    else $display("FAIL: host-to-device: device got %02h parity_ok %0d, host saw ack %0d", seen, pok, ack_seen);
    #200000;
    dev_send(8'hFA, 1'b0);
    #10000;
    if (ngot == 12 && got[11] === {8'h00, 8'hFA}) $display("PASS: the device's answer after a send is received");
    else $display("FAIL: answer after send: %0d bytes, last %04h", ngot, got[ngot-1]);
    if (line_falls == 149) $display("PASS: a send counts the device's clock edges, not the host's own inhibit");
    else $display("FAIL: line monitor after a send: %0d edges for 149", line_falls);

    // 6. A send no device answers ends unacknowledged.
    done_seen = 1'b0;
    @(negedge clk); tx_byte = 8'hFF; tx_start = 1'b1; @(negedge clk); tx_start = 1'b0;
    wait (done_seen);
    if (!ack_seen) $display("PASS: a send nobody clocks in ends unacknowledged");
    else $display("FAIL: an unanswered send reported an acknowledge");
    if (line_falls == 149) $display("PASS: the host's inhibit alone is no clock edge");
    else $display("FAIL: line monitor after an unanswered send: %0d edges for 149", line_falls);

    // 7. Keyboards at 30 kHz (the bridge's fastest) and 50 kHz: data set up 5 us before each falling edge.
    #200000;
    hold = 11666; setup = 5000; low = 16667;
    for (k = 0; k < 3; k = k + 1) dev_send(kbd[k], 1'b0);
    #10000;
    if (ngot == 15 && got[12] === {8'h00, kbd[0]} && got[13] === {8'h00, kbd[1]} && got[14] === {8'h00, kbd[2]})
      $display("PASS: a keyboard clocking at 30 kHz with a 5 us setup arrives whole");
    else $display("FAIL: 30 kHz: %0d bytes, last %04h", ngot, got[ngot-1]);
    hold = 5000; setup = 5000; low = 10000;
    for (k = 3; k < 6; k = k + 1) dev_send(kbd[k], 1'b0);
    #10000;
    if (ngot == 18 && got[15] === {8'h00, kbd[3]} && got[16] === {8'h00, kbd[4]} && got[17] === {8'h00, kbd[5]})
      $display("PASS: a keyboard clocking at 50 kHz with a 5 us setup arrives whole");
    else $display("FAIL: 50 kHz: %0d bytes, last %04h", ngot, got[ngot-1]);
    if (line_bytes == 17 && line_errors == 1 && line_falls == 215)
      $display("PASS: the line monitor's totals: 215 device clock edges, 17 bytes whole, 1 damaged");
    else $display("FAIL: line monitor totals: %0d edges for 215, %0d whole for 17, %0d damaged for 1", line_falls, line_bytes, line_errors);
    $display("DONE");
    $finish;
  end
endmodule
