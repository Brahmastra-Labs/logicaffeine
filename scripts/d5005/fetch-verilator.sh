#!/usr/bin/env bash
# Fetch Verilator, the simulator the decode engine's composed RTL runs under where iverilog would take hours: Ubuntu
# 24.04's verilator 5.020 package, unpacked without root into tools/verilator (or $VERILATOR_TOOLS), with a launcher
# that gives it its root. The package must have the SHA-256 written here; any other stops the script.
#
#   scripts/d5005/fetch-verilator.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
T="${VERILATOR_TOOLS:-$ROOT/tools/verilator}"
DEB=verilator_5.020-1_amd64.deb
SHA=bb74ca8c32c73eb903ee91da64c8b58cfbe095c229967a95782108afb67bb3ca
URL=http://archive.ubuntu.com/ubuntu/pool/universe/v/verilator/$DEB
mkdir -p "$T/dl"
if [ ! -f "$T/dl/$DEB" ]; then
  curl -fL --retry 3 -o "$T/dl/$DEB.part" "$URL"
  mv "$T/dl/$DEB.part" "$T/dl/$DEB"
fi
if [ "$(sha256sum "$T/dl/$DEB" | awk '{ print $1 }')" != "$SHA" ]; then
  echo "$T/dl/$DEB is not the file this script pins (sha256 $SHA)" >&2
  exit 1
fi
rm -rf "$T/pkg"
mkdir -p "$T/pkg"
dpkg -x "$T/dl/$DEB" "$T/pkg"
# The package keeps its programs in usr/bin and its root in usr/share/verilator, where the launcher looks for them.
share="$T/pkg/usr/share/verilator"
mkdir -p "$share/bin"
for b in verilator verilator_bin verilator_bin_dbg verilator_coverage_bin_dbg; do
  ln -sfn "../../../bin/$b" "$share/bin/$b"
done
ln -sfn ../../bin/verilator_bin "$share/verilator_bin"
mkdir -p "$T/bin"
cat > "$T/bin/verilator" <<EOF
#!/usr/bin/env bash
export VERILATOR_ROOT="$share"
exec "$T/pkg/usr/bin/verilator" "\$@"
EOF
chmod +x "$T/bin/verilator"
"$T/bin/verilator" --version
echo "$T: verilator ready"
