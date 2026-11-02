# Nexys Video constraints with the UART TX/RX pins SWAPPED vs nexys_video.xdc — a diagnostic to test
# whether the FPGA-TX line is V18 (uart_tx_in) rather than AA19 (uart_rx_out). Clock + LEDs identical.
set_property -dict { PACKAGE_PIN R4  IOSTANDARD LVCMOS33 } [get_ports { clk }]
create_clock -add -name sys_clk_pin -period 10.00 -waveform {0 5} [get_ports { clk }]

## SWAPPED: drive uart_tx (FPGA output) on V18 instead of AA19 (txprobe has no uart_rx port)
set_property -dict { PACKAGE_PIN V18  IOSTANDARD LVCMOS33 } [get_ports { uart_tx }]

set_property -dict { PACKAGE_PIN T14  IOSTANDARD LVCMOS25 } [get_ports { led[0] }]
set_property -dict { PACKAGE_PIN T15  IOSTANDARD LVCMOS25 } [get_ports { led[1] }]
set_property -dict { PACKAGE_PIN T16  IOSTANDARD LVCMOS25 } [get_ports { led[2] }]
set_property -dict { PACKAGE_PIN U16  IOSTANDARD LVCMOS25 } [get_ports { led[3] }]
set_property -dict { PACKAGE_PIN V15  IOSTANDARD LVCMOS25 } [get_ports { led[4] }]
set_property -dict { PACKAGE_PIN W16  IOSTANDARD LVCMOS25 } [get_ports { led[5] }]
set_property -dict { PACKAGE_PIN W15  IOSTANDARD LVCMOS25 } [get_ports { led[6] }]
set_property -dict { PACKAGE_PIN Y13  IOSTANDARD LVCMOS25 } [get_ports { led[7] }]
