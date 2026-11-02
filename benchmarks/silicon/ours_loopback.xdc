# ours_loopback — Nexys A7-100T (Board B) pin constraints.
#
# E3 is the 100 MHz oscillator and an MRCC (clock-capable) pin, which is what lets the clock leave
# the pad on the dedicated clock escape rather than the general interconnect. C4/D4 are the USB-UART
# bridge's RXD/TXD as seen by the FPGA.
set_property -dict { PACKAGE_PIN E3 IOSTANDARD LVCMOS33 } [get_ports clk]
set_property -dict { PACKAGE_PIN C4 IOSTANDARD LVCMOS33 } [get_ports uart_rx]
set_property -dict { PACKAGE_PIN D4 IOSTANDARD LVCMOS33 } [get_ports uart_tx]
