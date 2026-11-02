# ours_fifo_tx — Nexys Video (Board A) pin constraints.
#
# R4 is the 100 MHz oscillator, an MRCC pin (IO_L13P_T2_MRCC_34). The prog_* pins are the FT2232H
# asynchronous FIFO (DPTI): eight data lines plus the RXF#/RD#/OE#/TXE#/WR# handshake, and
# prog_spien selects the parallel FIFO rather than SPI.
set_property -dict { PACKAGE_PIN R4  IOSTANDARD LVCMOS33 } [get_ports clk]
set_property -dict { PACKAGE_PIN U20 IOSTANDARD LVCMOS33 } [get_ports prog_d[0]]
set_property -dict { PACKAGE_PIN P14 IOSTANDARD LVCMOS33 } [get_ports prog_d[1]]
set_property -dict { PACKAGE_PIN P15 IOSTANDARD LVCMOS33 } [get_ports prog_d[2]]
set_property -dict { PACKAGE_PIN U17 IOSTANDARD LVCMOS33 } [get_ports prog_d[3]]
set_property -dict { PACKAGE_PIN R17 IOSTANDARD LVCMOS33 } [get_ports prog_d[4]]
set_property -dict { PACKAGE_PIN P16 IOSTANDARD LVCMOS33 } [get_ports prog_d[5]]
set_property -dict { PACKAGE_PIN R18 IOSTANDARD LVCMOS33 } [get_ports prog_d[6]]
set_property -dict { PACKAGE_PIN N14 IOSTANDARD LVCMOS33 } [get_ports prog_d[7]]
set_property -dict { PACKAGE_PIN Y19 IOSTANDARD LVCMOS33 } [get_ports prog_txen]
set_property -dict { PACKAGE_PIN P19 IOSTANDARD LVCMOS33 } [get_ports prog_rdn]
set_property -dict { PACKAGE_PIN R19 IOSTANDARD LVCMOS33 } [get_ports prog_wrn]
set_property -dict { PACKAGE_PIN V17 IOSTANDARD LVCMOS33 } [get_ports prog_oen]
set_property -dict { PACKAGE_PIN P17 IOSTANDARD LVCMOS33 } [get_ports prog_siwun]
set_property -dict { PACKAGE_PIN R14 IOSTANDARD LVCMOS33 } [get_ports prog_spien]
