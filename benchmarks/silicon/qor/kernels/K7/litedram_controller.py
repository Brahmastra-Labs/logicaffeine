#!/usr/bin/env python3
"""LiteDRAM's controller core for the K7 comparison, as Verilog: LiteDRAMController and its crossbar with one
native port — the part of a LiteDRAM core that LOGOS's ddr2_ctrl corresponds to — for the Nexys A7's DDR2
(Micron MT47H64M16HR-25, x16) behind an A7DDRPHY at a 1:2 rate. The PHY is built only for the settings it hands the
controller (phases, read and write latencies); its I/O, the clock generator, the DFI injector the
initialisation CPU drives, and that CPU are not part of the module.

    litedram_controller.py <out.v> [--sys-clk-freq HZ] [--upstream-module]
    litedram_controller.py --phy-model --row-bits N <out.v> [--sys-clk-freq HZ] [--upstream-module]

The module is MT47H64M16 carrying every minimum its -25 grade's datasheet gives (Micron 1Gb DDR2, Rev. V,
Table 12). LiteDRAM's own MT47H64M16 entry leaves tRRD, tFAW and tRAS unset, and with them unset its controller
enforces none of them, nor tRC; --upstream-module builds with that entry as it ships.

--phy-model writes `litedram_phy_model`: LiteDRAM's SDRAMPHYModel, the PHY and the part as LiteDRAM simulates them,
at the A7DDRPHY's settings, storing --row-bits row-address bits behind the full 13-bit DFI address. Its ports are the
DFI phases, the controller's mirrored, so a testbench wires the controller as synthesised to it and logs the DFI.

Runs in the LiteX environment scripts/qor/fetch-qor-tools.py litex installs (tools/qor/litex-venv).
"""
import argparse

from migen import Module, Record
from migen.fhdl import verilog

from litedram.common import GeomSettings
from litedram.core.controller import LiteDRAMController
from litedram.core.crossbar import LiteDRAMCrossbar
from litedram.modules import MT47H64M16, _SpeedgradeTimings, _TechnologyTimings
from litedram.phy.model import SDRAMPHYModel
from litedram.phy.s7ddrphy import A7DDRPHY


class MT47H64M16_25(MT47H64M16):
    """MT47H64M16 at speed grade -25 with every datasheet minimum: tRRD 10 ns, tFAW 45 ns, tRAS 40 ns (LiteDRAM takes
    tRC as tRP + tRAS = 55 ns), the commercial tREFI of 7.8 us, and the two-clock floors of tWR, tWTR and tRRD."""
    technology_timings = _TechnologyTimings(tREFI=7.8e3, tWTR=(2, 7.5), tCCD=(2, None), tRRD=(2, 10))
    speedgrade_timings = {"default": _SpeedgradeTimings(tRP=15, tRCD=15, tWR=(2, 15), tRFC=(None, 127.5),
                                                        tFAW=(None, 45), tRAS=40)}


class ControllerCore(Module):
    def __init__(self, phy, module, clk_freq):
        self.submodules.controller = LiteDRAMController(
            phy_settings=phy.settings, geom_settings=module.geom_settings,
            timing_settings=module.timing_settings, clk_freq=clk_freq)
        self.submodules.crossbar = LiteDRAMCrossbar(self.controller.interface)
        self.port = self.crossbar.get_port()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out")
    ap.add_argument("--sys-clk-freq", type=float, default=100e6)
    ap.add_argument("--upstream-module", action="store_true")
    ap.add_argument("--phy-model", action="store_true")
    ap.add_argument("--row-bits", type=int)
    a = ap.parse_args()
    if a.phy_model != (a.row_bits is not None):
        ap.error("--phy-model and --row-bits go together")
    part = MT47H64M16 if a.upstream_module else MT47H64M16_25
    module = part(a.sys_clk_freq, "1:2")
    pads = Record([("a", 13), ("ba", 3), ("ras_n", 1), ("cas_n", 1), ("we_n", 1), ("cs_n", 1), ("dm", 2), ("dq", 16),
                   ("dqs_p", 2), ("dqs_n", 2), ("clk_p", 1), ("clk_n", 1), ("cke", 1), ("odt", 1), ("reset_n", 1)])
    phy = A7DDRPHY(pads, memtype="DDR2", nphases=2, sys_clk_freq=a.sys_clk_freq, iodelay_clk_freq=200e6)
    ios = set()
    if a.phy_model:
        if not 1 <= a.row_bits <= module.geom_settings.rowbits:
            ap.error("--row-bits is 1 to %d" % module.geom_settings.rowbits)
        stored = part(a.sys_clk_freq, "1:2")
        stored.geom_settings = GeomSettings(bankbits=module.geom_settings.bankbits, rowbits=a.row_bits,
                                            colbits=module.geom_settings.colbits)
        stored.geom_settings.addressbits = module.geom_settings.addressbits
        top, name = SDRAMPHYModel(stored, settings=phy.settings, clk_freq=a.sys_clk_freq), "litedram_phy_model"
        dfi = top.dfi
    else:
        top, name = ControllerCore(phy, module, a.sys_clk_freq), "litedram_ctrl"
        dfi = top.controller.dfi
        for endpoint in (top.port.cmd, top.port.wdata, top.port.rdata):
            ios |= set(endpoint.flatten())
    for phase in dfi.phases:
        ios |= set(phase.flatten())
    verilog.convert(top, ios=ios, name=name).write(a.out)


if __name__ == "__main__":
    main()
