# LOGOS clock tree synthesis within the fanout limit (run by run_cts unless LOGOS_CTS_FANOUT is 0).
#
# TritonCTS stops subdividing its H-tree once a region's average cluster count falls under its maximum leaf
# sinks, 15 by default, so with balanced splits a leaf drives that average — anywhere up to 14 clusters, past a
# fanout limit of 10. The maximum is a CtsOptions field this OpenROAD (41a51eaf) exposes to Python and not to
# Tcl, so this driver sets it to the limit and then runs the flow's own cts.tcl in the same process, where every
# other setting lives.
import os
import sys

from openroad import Design, Tech

design = Design(Tech())
# The design limit, as logos_fanout_limit.tcl reads it: the fanout margin under the SDC's limit.
limit = int(round(float(os.environ["MAX_FANOUT_CONSTRAINT"]))) - int(os.environ.get("LOGOS_FANOUT_MARGIN", "1"))
parms = design.getTritonCts().getParms()
parms.setNumMaxLeafSinks(limit)
print(f"[LOGOS-CTS] H-tree subdivided until a leaf's average falls under {parms.getNumMaxLeafSinks()} clusters")
try:
    design.evalTclString(f"source {os.environ['SCRIPTS_DIR']}/openroad/cts.tcl")
except Exception as exc:
    print(f"[LOGOS-CTS] cts.tcl failed: {exc}", file=sys.stderr)
    sys.exit(1)
sys.exit(0)
