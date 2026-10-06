#!/bin/bash
# Post-merge sanity pass on omega main: build + host checks + postmerge-chip.sh, all inside one quiet hold.
set -u
SHA=$1; E=$2; P=$HOME/workspace/primes-race
log(){ echo "$(date -u +%H:%M:%SZ) $*"; }
( cd $E/omega && export PHYSICS_DIR=$P/physics && make -s prime-race-gb10 prime-race-gb10-host-test test-gpu-elementwise ) > $E/host.txt 2>&1; rc=$?
grep -E 'checks=|VERDICT|PASS|FAIL' $E/host.txt | tail -4; log "build + host checks rc=$rc"
[ $rc -eq 0 ] || { echo "VERDICT FAIL host" > $E/VERDICT; exit 1; }
cp $E/omega/bench/prime_race/build/gb10_native $E/omega/bench/prime_race/build/gb10_native_mutant $E/bins/
sha256sum $E/bins/* > $E/bins.sha256
$P/postmerge-chip.sh $SHA $E
