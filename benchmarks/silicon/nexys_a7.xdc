# Nexys A7-100T (Xilinx Artix-7 XC7A100T-1CSG324C) constraints for the full 113-op LOGOS soft-core.
# Pins from the Digilent Nexys-A7-100T master XDC. The full CPU (soft_core.v + alu.v + shared_divider.v)
# maps to ~38,400 LUT6 / 63,400 (60%), 1,503 FF, 3 DSP48E1, 13k CARRY4 — it FITS with ~40% headroom.
#
# Bitstream flow (needs Vivado for place-and-route on 7-series):
#   yosys -p "read_verilog soft_core.v alu.v shared_divider.v; \
#             chparam -set HEAP 64 soft_core; synth_xilinx -family xc7 -top soft_core -edif soft_core.edif"
#   ... then Vivado: read_edif + this XDC -> place_design -> route_design -> write_bitstream
# (or synth the RTL directly in Vivado with this XDC). No ECP5-style open ecppack path exists for 7-series.

## Clock — 100 MHz oscillator on E3
set_property -dict { PACKAGE_PIN E3  IOSTANDARD LVCMOS33 } [get_ports { clk }]
create_clock -add -name sys_clk_pin -period 10.00 -waveform {0 5} [get_ports { clk }]

## Reset — slide switch SW0 (active-high, matching the core's synchronous reset)
set_property -dict { PACKAGE_PIN J15 IOSTANDARD LVCMOS33 } [get_ports { rst }]

## Show output (low 16 bits) -> the 16 green LEDs
set_property -dict { PACKAGE_PIN H17 IOSTANDARD LVCMOS33 } [get_ports { out[0]  }]
set_property -dict { PACKAGE_PIN K15 IOSTANDARD LVCMOS33 } [get_ports { out[1]  }]
set_property -dict { PACKAGE_PIN J13 IOSTANDARD LVCMOS33 } [get_ports { out[2]  }]
set_property -dict { PACKAGE_PIN N14 IOSTANDARD LVCMOS33 } [get_ports { out[3]  }]
set_property -dict { PACKAGE_PIN R18 IOSTANDARD LVCMOS33 } [get_ports { out[4]  }]
set_property -dict { PACKAGE_PIN V17 IOSTANDARD LVCMOS33 } [get_ports { out[5]  }]
set_property -dict { PACKAGE_PIN U17 IOSTANDARD LVCMOS33 } [get_ports { out[6]  }]
set_property -dict { PACKAGE_PIN U16 IOSTANDARD LVCMOS33 } [get_ports { out[7]  }]
set_property -dict { PACKAGE_PIN V16 IOSTANDARD LVCMOS33 } [get_ports { out[8]  }]
set_property -dict { PACKAGE_PIN T15 IOSTANDARD LVCMOS33 } [get_ports { out[9]  }]
set_property -dict { PACKAGE_PIN U14 IOSTANDARD LVCMOS33 } [get_ports { out[10] }]
set_property -dict { PACKAGE_PIN T16 IOSTANDARD LVCMOS33 } [get_ports { out[11] }]
set_property -dict { PACKAGE_PIN V15 IOSTANDARD LVCMOS33 } [get_ports { out[12] }]
set_property -dict { PACKAGE_PIN V14 IOSTANDARD LVCMOS33 } [get_ports { out[13] }]
set_property -dict { PACKAGE_PIN V12 IOSTANDARD LVCMOS33 } [get_ports { out[14] }]
set_property -dict { PACKAGE_PIN V11 IOSTANDARD LVCMOS33 } [get_ports { out[15] }]

## Status flags -> the two tri-colour LEDs (red channels)
set_property -dict { PACKAGE_PIN N15 IOSTANDARD LVCMOS33 } [get_ports { halted    }]  # LD16_R
set_property -dict { PACKAGE_PIN N16 IOSTANDARD LVCMOS33 } [get_ports { out_valid }]  # LD17_R

## The upper Show bits [31:16] are not pinned (16 LEDs); a UART-readout wrapper (cf. coproc_soc for
## the ULX3S) streams the full 32-bit words — the board-bring-up step. Unconstrained upper bits are OK
## for synthesis/fit; add set_property SEVERITY on unconstrained pins only for a final bitstream.
