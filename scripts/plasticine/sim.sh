#!/usr/bin/env bash
# The FUNCTIONAL gate (run inside the container via run-in-container.sh): Spatial's own Scala backend compiles the
# app and simulates it — every controller, every memory, and the host's check against the gold — without mapping
# it onto a fabric. PIR writes a whole accelerator as ONE JVM class, so a composed program past a few sections is
# larger than the JVM lets a class be; such a program is simulated here whole, and mapped onto the fabric section
# by section through gate.sh. No toolchain source is edited. Usage: sim.sh <app>
set -euo pipefail
APP="$1"
cd /work/spatial
# The Scala backend reads Spatial's resource index from the classpath; build.sh writes it, and a tree built before
# it did gets it here.
[ -f resources/files_list ] || bash bin/update_resources.sh
# A run is a FRESH run: a stale generated tree would replay an old app against new sources.
rm -rf "gen/Scala/$APP"
# The app is printed in the SIMULATOR flavor (`spatial_lang::SpatialFlavor::Simulator`): every fold a register scan,
# because this Spatial's Scala backend drops the first-iteration flags of an unrolled `Reduce` it splits into a kernel.
echo "=== [$(date -u +%H:%M:%S)] SpatialTest -a $APP -b Scala -p pirTest -T"
pir/env/bin/python -c "
import sys; sys.path.insert(0, 'pir/bin')
import spatial_test
spatial_test.SpatialTest(args=['-a', '$APP', '-b', 'Scala', '-p', 'pirTest', '-T']).run()
" 2>&1 | tail -150
echo "=== [$(date -u +%H:%M:%S)] done"
