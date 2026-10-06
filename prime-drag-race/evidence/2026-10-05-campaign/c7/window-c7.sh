#!/bin/bash
# Prime race campaign C7: is the C3 shared-memory table still worth it after the C5 mark-loop fix?
# One binary (omega 9dee7b1); the only variable is PR_GB10_TABLE (auto = staged table, global = never).
# Correctness on both modes before any timed run; then A-B-A-B at 64 CTAs and A-B-A at 128.
set -u
E=$HOME/workspace/evidence/prime-race/c7
mkdir -p $E/verify $E/runs
R=$HOME/workspace/primes-race/benchmarks/prime-drag-race/target/release/prime_race
BIN=$HOME/workspace/evidence/prime-race/bins
H=9dee7b1; FULL=9dee7b157ec769fae745f63b4022a1d17eb06adc
N=$BIN/gb10_native-$H; M=$BIN/gb10_native_mutant-$H
TAGS="algorithm=other,faithful=no,bits=1"
BIG="999999..1000001,1048575..1048577,2097151..2097153,4194303..4194305,8388607..8388609,10000000,16777215..16777217,127667399..127667402"
export PR_GB10_SPIN_US=2000
log(){ echo "$(date -u +%H:%M:%SZ) $*"; }
log "window start; load $(cat /proc/loadavg)"
sha256sum $N $M > $E/bins.sha256
FAILS=""
v(){ local n=$1 t=$2 b=$3 i=$4 nm=$5; shift 5
  PR_GB10_TABLE=$t PR_GB10_CTA_BUDGET=$b $R verify --impl $i --name $nm --declare $TAGS "$@" > $E/verify/$n.txt 2>&1; local rc=$?
  tail -1 $E/verify/$n.txt; log "$n rc=$rc"; return $rc; }
for t in auto global; do
  v $t-default-b64 $t 64 $N gb10-native || FAILS="$FAILS $t-default-b64"
  v $t-default-b128 $t 128 $N gb10-native || FAILS="$FAILS $t-default-b128"
  v $t-big-b1024 $t 1024 $N gb10-native --limits "$BIG" || FAILS="$FAILS $t-big-b1024"
  v $t-big-b64 $t 64 $N gb10-native --limits "$BIG" || FAILS="$FAILS $t-big-b64"
  for b in 64 1024; do
    v $t-mutant-b$b $t $b $M gb10-native-mutant --limits "100000,1000000,10000000"
    grep -q 'label_mismatch\|EXEC_FAILED\|usage' $E/verify/$t-mutant-b$b.txt && FAILS="$FAILS $t-mutant-b$b-wrongreason"
    grep -q 'FAIL limit' $E/verify/$t-mutant-b$b.txt || FAILS="$FAILS $t-mutant-b$b-notcaught"
  done
done
log "verify summary: fails=[${FAILS}]"
[ -z "$FAILS" ] || { log "STOP: no timed run"; exit 1; }
for leg in "A-auto64:auto:64" "B-global64:global:64" "C-auto64:auto:64" "D-global64:global:64" "E-auto128:auto:128" "F-global128:global:128" "G-auto128:auto:128"; do
  IFS=: read name t b <<< "$leg"
  log "T $name timed (table $t, spin 2000, budget $b, audit 5, warmup 1, 10 x >=5 s); load $(cat /proc/loadavg)"
  PR_GB10_TABLE=$t PR_GB10_CTA_BUDGET=$b $R run --impl gb10-native=$N --declare gb10-native=$TAGS \
    --source gb10-native=aien-dev/omega@$FULL \
    --build "gb10-native=make PHYSICS_DIR=<physics@6d7cf0d> prime-race-gb10 (cc -O3 -mcpu=native; libomega_gpu omega CFLAGS -O2); env PR_GB10_TABLE=$t PR_GB10_CTA_BUDGET=$b PR_GB10_SPIN_US=2000" \
    --trials 10 --warmup 1 --limit 1000000 --min-seconds 5 --audit-passes 5 \
    --evidence-dir $E/runs --runner-commit aien-dev/benchmarks@d66cd2a0d7556de2d4afff1b6c1ec368cec54693 \
    --quiet-flag-file $HOME/workspace/.spark-quiet > $E/runs/$name.txt 2>&1; rc=$?
  grep -E 'median|receipt:' $E/runs/$name.txt | cut -c1-140; log "T $name rc=$rc"
done
log "window end; load $(cat /proc/loadavg)"
