# zcu104.xdc — pins for zcu104_top.v on the ZCU104 (XCZU7EV-FFVC1156-2-E).
# Source: Xilinx/XilinxBoardStore boards/Xilinx/zcu104/1.1/part0_pins.xml + board.xml (branch 2025.2).

# 300 MHz programmable differential clock (SI5341B), PL bank shared with the DDR4 SODIMM.
set_property -dict {PACKAGE_PIN AH18 IOSTANDARD DIFF_SSTL12} [get_ports clk_300_p]
set_property -dict {PACKAGE_PIN AH17 IOSTANDARD DIFF_SSTL12} [get_ports clk_300_n]
create_clock -name clk_300 -period 3.333 [get_ports clk_300_p]

# uart2_PL: the FT4232's PL channel (host side /dev/ttyUSB3). TX is the FPGA's output.
set_property -dict {PACKAGE_PIN C19 IOSTANDARD LVCMOS18} [get_ports uart_tx]
set_property -dict {PACKAGE_PIN A20 IOSTANDARD LVCMOS18} [get_ports uart_rx]

# GPIO_PB_SW0 as reset, GPIO_LED_0..3.
set_property -dict {PACKAGE_PIN B4 IOSTANDARD LVCMOS33} [get_ports rst_btn]
set_property -dict {PACKAGE_PIN D5 IOSTANDARD LVCMOS33} [get_ports {led[0]}]
set_property -dict {PACKAGE_PIN D6 IOSTANDARD LVCMOS33} [get_ports {led[1]}]
set_property -dict {PACKAGE_PIN A5 IOSTANDARD LVCMOS33} [get_ports {led[2]}]
set_property -dict {PACKAGE_PIN B5 IOSTANDARD LVCMOS33} [get_ports {led[3]}]

# Slow, asynchronous board I/O: no timing relationship to the fabric clock.
set_false_path -from [get_ports {uart_rx rst_btn}]
set_false_path -to   [get_ports {uart_tx led[*]}]
