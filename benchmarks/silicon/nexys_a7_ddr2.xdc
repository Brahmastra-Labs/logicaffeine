# Nexys A7-100T DDR2 pin constraints (Micron MT47H64M16, 1.8V bank) + clk/uart/led.
# Pin map from the LiteX digilent_nexys4ddr platform (canonical open-flow source).

# 100 MHz system clock
set_property -dict { PACKAGE_PIN E3  IOSTANDARD LVCMOS33 } [get_ports clk]
create_clock -add -name sys_clk -period 10.00 -waveform {0 5} [get_ports clk]

# USB-UART TX + a status LED
set_property -dict { PACKAGE_PIN D4  IOSTANDARD LVCMOS33 } [get_ports uart_tx]
set_property -dict { PACKAGE_PIN H17 IOSTANDARD LVCMOS33 } [get_ports led]

# ---- DDR2 address / command (SSTL18_II) ----
set_property -dict { PACKAGE_PIN M4 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[0]}]
set_property -dict { PACKAGE_PIN P4 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[1]}]
set_property -dict { PACKAGE_PIN M6 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[2]}]
set_property -dict { PACKAGE_PIN T1 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[3]}]
set_property -dict { PACKAGE_PIN L3 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[4]}]
set_property -dict { PACKAGE_PIN P5 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[5]}]
set_property -dict { PACKAGE_PIN M2 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[6]}]
set_property -dict { PACKAGE_PIN N1 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[7]}]
set_property -dict { PACKAGE_PIN L4 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[8]}]
set_property -dict { PACKAGE_PIN N5 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[9]}]
set_property -dict { PACKAGE_PIN R2 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[10]}]
set_property -dict { PACKAGE_PIN K5 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[11]}]
set_property -dict { PACKAGE_PIN N6 IOSTANDARD LVCMOS18 } [get_ports {ddr2_addr[12]}]
set_property -dict { PACKAGE_PIN P2 IOSTANDARD LVCMOS18 } [get_ports {ddr2_ba[0]}]
set_property -dict { PACKAGE_PIN P3 IOSTANDARD LVCMOS18 } [get_ports {ddr2_ba[1]}]
set_property -dict { PACKAGE_PIN R1 IOSTANDARD LVCMOS18 } [get_ports {ddr2_ba[2]}]
set_property -dict { PACKAGE_PIN N4 IOSTANDARD LVCMOS18 } [get_ports ddr2_ras_n]
set_property -dict { PACKAGE_PIN L1 IOSTANDARD LVCMOS18 } [get_ports ddr2_cas_n]
set_property -dict { PACKAGE_PIN N2 IOSTANDARD LVCMOS18 } [get_ports ddr2_we_n]
set_property -dict { PACKAGE_PIN K6 IOSTANDARD LVCMOS18 } [get_ports ddr2_cs_n]
set_property -dict { PACKAGE_PIN M1 IOSTANDARD LVCMOS18 } [get_ports ddr2_cke]
set_property -dict { PACKAGE_PIN M3 IOSTANDARD LVCMOS18 } [get_ports ddr2_odt]

# ---- DDR2 data / mask (SSTL18_II) ----
set_property -dict { PACKAGE_PIN T6 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dm[0]}]
set_property -dict { PACKAGE_PIN U1 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dm[1]}]
set_property -dict { PACKAGE_PIN R7 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[0]}]
set_property -dict { PACKAGE_PIN V6 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[1]}]
set_property -dict { PACKAGE_PIN R8 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[2]}]
set_property -dict { PACKAGE_PIN U7 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[3]}]
set_property -dict { PACKAGE_PIN V7 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[4]}]
set_property -dict { PACKAGE_PIN R6 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[5]}]
set_property -dict { PACKAGE_PIN U6 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[6]}]
set_property -dict { PACKAGE_PIN R5 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[7]}]
set_property -dict { PACKAGE_PIN T5 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[8]}]
set_property -dict { PACKAGE_PIN U3 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[9]}]
set_property -dict { PACKAGE_PIN V5 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[10]}]
set_property -dict { PACKAGE_PIN U4 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[11]}]
set_property -dict { PACKAGE_PIN V4 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[12]}]
set_property -dict { PACKAGE_PIN T4 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[13]}]
set_property -dict { PACKAGE_PIN V1 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[14]}]
set_property -dict { PACKAGE_PIN T3 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dq[15]}]

# ---- DDR2 differential strobes + clock (DIFF_SSTL18_II) ----
set_property -dict { PACKAGE_PIN U9 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dqs_p[0]}]
set_property -dict { PACKAGE_PIN V9 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dqs_n[0]}]
set_property -dict { PACKAGE_PIN U2 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dqs_p[1]}]
set_property -dict { PACKAGE_PIN V2 IOSTANDARD LVCMOS18 } [get_ports {ddr2_dqs_n[1]}]
set_property -dict { PACKAGE_PIN L6 IOSTANDARD LVCMOS18 } [get_ports ddr2_ck_p]
set_property -dict { PACKAGE_PIN L5 IOSTANDARD LVCMOS18 } [get_ports ddr2_ck_n]

