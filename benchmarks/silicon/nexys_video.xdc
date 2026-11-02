# Nexys Video (Xilinx Artix-7 XC7A200T-1SBG484C) constraints for the Board-A bring-up smoke test.
# Pins from the Digilent Nexys-Video-Master.xdc. The FPGA is 2x the XC7A100T on the Nexys A7-100T:
# 134,600 LUTs / 740 DSP / 13 Mb BRAM. JTAG is on FT2232 channel B — flash with
# `openFPGALoader -c digilent_b`, NOT `-c digilent` (that channel A is the Nexys A7's JTAG).

## Clock — 100 MHz oscillator on R4 (MRCC, BUFG-capable). /4 -> 25 MHz core, CPB 217 = 25e6/115200.
set_property -dict { PACKAGE_PIN R4  IOSTANDARD LVCMOS33 } [get_ports { clk }]
create_clock -add -name sys_clk_pin -period 10.00 -waveform {0 5} [get_ports { clk }]

## USB-UART bridge (FT2232, channel A). Digilent names the nets from the bridge side:
## uart_rx = data host->FPGA = the board's `uart_tx_in`  (V18);
## uart_tx = data FPGA->host = the board's `uart_rx_out` (AA19). 8N1, 115200 baud.
set_property -dict { PACKAGE_PIN V18  IOSTANDARD LVCMOS33 } [get_ports { uart_rx }]
set_property -dict { PACKAGE_PIN AA19 IOSTANDARD LVCMOS33 } [get_ports { uart_tx }]

## 8 user LEDs (bank 13, LVCMOS25). led[0] = ~0.75 Hz heartbeat; led[7:1] = last echoed byte[7:1].
set_property -dict { PACKAGE_PIN T14  IOSTANDARD LVCMOS25 } [get_ports { led[0] }]
set_property -dict { PACKAGE_PIN T15  IOSTANDARD LVCMOS25 } [get_ports { led[1] }]
set_property -dict { PACKAGE_PIN T16  IOSTANDARD LVCMOS25 } [get_ports { led[2] }]
set_property -dict { PACKAGE_PIN U16  IOSTANDARD LVCMOS25 } [get_ports { led[3] }]
set_property -dict { PACKAGE_PIN V15  IOSTANDARD LVCMOS25 } [get_ports { led[4] }]
set_property -dict { PACKAGE_PIN W16  IOSTANDARD LVCMOS25 } [get_ports { led[5] }]
set_property -dict { PACKAGE_PIN W15  IOSTANDARD LVCMOS25 } [get_ports { led[6] }]
set_property -dict { PACKAGE_PIN Y13  IOSTANDARD LVCMOS25 } [get_ports { led[7] }]
