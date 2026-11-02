#!/usr/bin/env bash
# The flat build's IP cache. A FIM work tree just set up holds the same IP sources from one build to the next (the
# FIM's own; the accelerator adds none), and the FIM's IP generation (quartus_ipgenerate, whose generation server dies
# or deadlocks at random) writes each IP's outputs beside its source. The cache keeps what a generation added to or
# changed in a tree since its setup — outside the project directory syn/, whose project files, database and reports are
# each build's own — under a key of the tree's IP sources (every .ip, .qsys and _hw.tcl, by path and SHA-256) and the
# Quartus version, so a build whose sources hash alike restores the outputs and generates nothing. The compile's own IP
# generation skips an IP whose generation directory holds its outputs (its regeneration policy), however old they are.
#
#   ip_cache.sh begin   <fim> <state>          after the setup: the tree's manifest (state/manifest) and key (state/key)
#   ip_cache.sh restore <fim> <state> <cache>  the key's entry put into the tree once every file it holds checks against
#                                              the entry's manifest (exit 0); with no key, no entry, or an entry that does
#                                              not check (set aside as <key>.refused), the tree as it was (exit 1)
#   ip_cache.sh store   <fim> <state> <cache>  after a generation: the files it added or changed outside syn/ since begin,
#                                              with their manifest, kept under the key, whole or not at all (exit 0); with
#                                              no begin, nothing kept (exit 1)
#
# QUARTUS_VERSION_FILE names the Quartus version (default the container's /opt/q234/quartus/version.txt).
set -euo pipefail
cmd=${1:-}
fim=${2:-}
state=${3:-}
cache=${4:-}
usage() {
  echo "usage: ip_cache.sh begin <fim> <state> | restore <fim> <state> <cache> | store <fim> <state> <cache>" >&2
  exit 2
}
[ -n "$fim" ] && [ -d "$fim" ] && [ -n "$state" ] || usage
version=${QUARTUS_VERSION_FILE:-/opt/q234/quartus/version.txt}

# Every file under a directory, by SHA-256 and path from it, in byte order.
manifest() { (cd "$1" && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 -r sha256sum); }
# The IP sources under a directory, the same way.
sources() { (cd "$1" && find . -type f \( -name '*.ip' -o -name '*.qsys' -o -name '*_hw.tcl' \) -print0 | LC_ALL=C sort -z | xargs -0 -r sha256sum); }

case "$cmd" in
  begin)
    [ -r "$version" ] || { echo "ip_cache.sh: no Quartus version at $version" >&2; exit 2; }
    mkdir -p "$state"
    manifest "$fim" > "$state/manifest.part"
    { cat "$version"; sources "$fim"; } | sha256sum | cut -c1-64 > "$state/key.part"
    mv "$state/manifest.part" "$state/manifest"
    mv "$state/key.part" "$state/key"
    echo "ip_cache.sh: the tree's IP sources hash to $(cat "$state/key")"
    ;;
  restore)
    [ -n "$cache" ] || usage
    [ -r "$state/key" ] || { echo "ip_cache.sh: this tree's setup kept no key; generating"; exit 1; }
    key=$(cat "$state/key")
    entry="$cache/$key"
    [ -r "$entry.tar" ] && [ -r "$entry.files" ] || { echo "ip_cache.sh: no IP kept for $key; generating"; exit 1; }
    stage=$(mktemp -d "$state/restore.XXXXXX")
    trap 'rm -r -f "$stage"' EXIT
    held=$(wc -l < "$entry.files")
    if ! tar -xf "$entry.tar" -C "$stage" --touch 2>/dev/null \
      || [ "$(cd "$stage" && find . -type f | wc -l)" != "$held" ] \
      || ! (cd "$stage" && sha256sum --quiet --strict -c "$entry.files" > /dev/null 2>&1); then
      mv -f "$entry.tar" "$entry.refused.tar"
      mv -f "$entry.files" "$entry.refused.files"
      echo "ip_cache.sh: the IP kept for $key does not check against its manifest; set aside, generating"
      exit 1
    fi
    (cd "$stage" && cp -a . "$fim/")
    echo "ip_cache.sh: $held files of IP restored from $key"
    ;;
  store)
    [ -n "$cache" ] || usage
    [ -r "$state/key" ] && [ -r "$state/manifest" ] || { echo "ip_cache.sh: this tree's setup kept no key; nothing kept"; exit 1; }
    key=$(cat "$state/key")
    entry="$cache/$key"
    if [ -r "$entry.tar" ] && [ -r "$entry.files" ]; then echo "ip_cache.sh: the IP for $key is kept already"; exit 0; fi
    mkdir -p "$cache"
    staging=$(mktemp -d "$cache/.store.XXXXXX")
    trap 'rm -r -f "$staging"' EXIT
    manifest "$fim" > "$staging/after"
    # The files added or changed since the setup, outside the project directory.
    LC_ALL=C comm -13 <(LC_ALL=C sort "$state/manifest") <(LC_ALL=C sort "$staging/after") | grep -v '^[0-9a-f]\{64\}  \./syn/' > "$staging/files" || true
    [ -s "$staging/files" ] || { echo "ip_cache.sh: the generation changed nothing outside syn/; nothing kept"; exit 1; }
    cut -c67- "$staging/files" | tr '\n' '\0' > "$staging/paths"
    (cd "$fim" && tar -cf "$staging/tar" --null -T "$staging/paths")
    mv -f "$staging/tar" "$entry.tar"
    mv -f "$staging/files" "$entry.files"
    echo "ip_cache.sh: $(wc -l < "$entry.files") files of IP kept as $key"
    ;;
  *) usage ;;
esac
