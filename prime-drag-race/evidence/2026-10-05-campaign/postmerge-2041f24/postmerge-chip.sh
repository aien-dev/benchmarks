#!/bin/bash
# Post-merge check of the combined omega main (after #307): correctness, kernel gate, one short timed
# run. Screens for an obvious regression only; the parity claim stays on receipt 1a3eafe9.
set -u
SHA=$1; E=$2
R=$HOME/workspace/primes-race/benchmarks/prime-drag-race/target/release/prime_race
N=$E/bins/gb10_native; M=$E/bins/gb10_native_mutant
TAGS="algorithm=other,faithful=no,bits=1"
BIG="999999..1000001,1048575..1048577,2097151..2097153,4194303..4194305,8388607..8388609,10000000,16777215..16777217,127667399..127667402"
export PR_GB10_SPIN_US=2000
log(){ echo "$(date -u +%H:%M:%SZ) $*"; }
log "chip start $SHA; load $(cat /proc/loadavg)"
FAILS=""
v(){ local n=$1 b=$2 i=$3 nm=$4; shift 4
  PR_GB10_CTA_BUDGET=$b $R verify --impl $i --name $nm --declare $TAGS "$@" > $E/verify/$n.txt 2>&1; local rc=$?
  tail -1 $E/verify/$n.txt; log "$n rc=$rc"; return $rc; }
v default-b128 128 $N gb10-native || FAILS="$FAILS default-b128"
v big-b1024 1024 $N gb10-native --limits "$BIG" || FAILS="$FAILS big-b1024"
v mutant-b1024 1024 $M gb10-native-mutant --limits "100000,1000000,10000000"
grep -q 'label_mismatch\|EXEC_FAILED\|usage' $E/verify/mutant-b1024.txt && FAILS="$FAILS mutant-wrongreason"
grep -q 'FAIL limit' $E/verify/mutant-b1024.txt || FAILS="$FAILS mutant-notcaught"
( cd $E/omega && PHYSICS_DIR=$HOME/workspace/primes-race/physics sh tools/run_gpu_elementwise_chip.sh $E/eltwise ) > $E/eltwise/gate.txt 2>&1; rc=$?
log "elementwise chip gate rc=$rc: $(tail -2 $E/eltwise/gate.txt | tr '\n' ' ' | cut -c1-160)"
[ $rc -eq 0 ] || FAILS="$FAILS eltwise-gate"
log "verify summary: fails=[${FAILS}]"
[ -z "$FAILS" ] || { log "STOP: no timed run"; echo "VERDICT FAIL$FAILS" > $E/VERDICT; exit 1; }
log "short timed run (budget 128, spin 2000, table auto, audit 5, warmup 1, 5 x >=5 s)"
PR_GB10_CTA_BUDGET=128 $R run --impl gb10-native=$N --declare gb10-native=$TAGS \
  --source gb10-native=aien-dev/omega@$SHA \
  --build "gb10-native=make PHYSICS_DIR=<physics@6d7cf0d> prime-race-gb10 (cc -O3 -mcpu=native; libomega_gpu omega CFLAGS -O2); env PR_GB10_CTA_BUDGET=128 PR_GB10_SPIN_US=2000" \
  --trials 5 --warmup 1 --limit 1000000 --min-seconds 5 --audit-passes 5 \
  --evidence-dir $E/runs --runner-commit aien-dev/benchmarks@d66cd2a0d7556de2d4afff1b6c1ec368cec54693 \
  --quiet-flag-file $HOME/workspace/.spark-quiet > $E/runs/short.txt 2>&1; rc=$?
grep -E 'median|receipt:' $E/runs/short.txt | cut -c1-140; log "short run rc=$rc"
MED=$(grep -o 'median *[0-9.]*' $E/runs/short.txt | head -1 | awk '{print $2}')
REC=$(grep -o 'receipt: .*' $E/runs/short.txt | head -1 | sed 's/receipt: //')
V=FAIL; [ $rc -eq 0 ] && [ -n "$MED" ] && V=$(awk -v m=$MED 'BEGIN{print (m >= 0.95*19701.34) ? "PASS" : "REGRESSION"}')
printf '{"merge_revision": "aien-dev/omega@%s", "gb10_native_sha256": "%s", "gb10_native_mutant_sha256": "%s", "median_passes_per_s": "%s", "reference_receipt": "1a3eafe9 (19701.34)", "threshold": "0.95x", "receipt": "%s", "verdict": "%s"}\n' \
  "$SHA" "$(sha256sum $N | cut -d' ' -f1)" "$(sha256sum $M | cut -d' ' -f1)" "$MED" "$REC" "$V" > $E/summary.json
echo "VERDICT $V median $MED" > $E/VERDICT; log "VERDICT $V median $MED"
