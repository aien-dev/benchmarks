# GB10 prime sieve investigation

OBSERVED: Worktree starts at d6632db. No GPU program, GPU lock, sudo, commit or push was used.
OBSERVED: This delivery is **not host-validated**. `quietlock check` returned 1 twice:
`quietlock: cannot open lock file /home/drakestapleton/workspace/.spark-quiet.lock: Read-only file system`.
The brief therefore prohibited every build and host test. Neither was attempted.
OBSERVED: Writing `astra-out/astra.patch` also failed with `Read-only file system`.
The report and patch are saved in the writable worktree's `astra-deliverables/` instead.
UNKNOWN: The requested all-checks-PASS result and output placement remain outstanding.

DOCUMENTED: Source abbreviations below: E = `src/omega_gpu_elementwise_api.c`,
C = `src/omega_blackwell_codegen.c`, H = `bench/prime_race/tests/gb10_sieve_host_test.c`,
Q = `src/omega_blackwell_qmd.c`, S = `src/omega_gpu_session.c`.
NAK = `/home/drakestapleton/workspace/.ref/mesa/src/nouveau/compiler/`,
at Mesa a08a0b93ac53dfb8dff6d2c9cd3893b80aa30f44. Existing-loop line numbers are unchanged;
new scheduling/checker references refer to this patch. Quoted code is implementation evidence,
not a public NVIDIA guarantee of SASS scheduling behavior. **DOCS SILENT** on control words.

## 1. What bounds the prime loop?

INFERRED (high confidence, conditional on coherent warp execution): Blanket stalls alone
do not explain 821 / 519 us. They impose tens of thousands of cycles per critical warp,
not the hundreds of thousands or millions needed at ordinary GPU clocks.
UNKNOWN: The dominant residual is not identified by the supplied measurements.
Memory waits, branch/warp fragmentation, issue arbitration and host timing remain candidates.

DOCUMENTED: E:57-65 sets fixed=6, variable=4, branch=5 (predicate was 13 at d6632db).
C:403 says `w[3] = 0x010fca00` for IADD3: its actual delay is 5, wait mask 0x10
(SB4), no read/write barrier, regardless of IR control. No sieve producer assigns SB4.
E:101-119 groups three loads on SB0; only the following ISETP waits for their results.
DOCUMENTED: NAK `nak/sm70_encode.rs:325-332` encodes delay at 105..109, waits at 116..122,
and writes `deps.yld` only inside `if self.sm < 120`. Omega sets bit 109 for variable ops.
UNKNOWN: A quantitative GB10 yield-bit penalty; do not add a fabricated yield cost.

DOCUMENTED: NAK `latencies/lat_rs_gen.py:48-61` selects the reader row and writer column.
`nak/sm120_instr_latencies.rs:26,42,48` maps IMAD/IMul to Fma, IADD3 to Alu,
and ISETP to Dualalu. Its lines 452/469 add one cycle to the SM100 RAW values.

| DOCUMENTED dependency | NAK bound including padding | Existing encoded policy |
|---|---:|---:|
| ALU/dual-ALU GPR -> ALU/dual-ALU | 5 | usually 6; IADD3 5 |
| IMAD/FP32 FMA-family -> same family | 5 | 6 |
| ALU GPR <-> FMA-family | 6 | usually 6 |
| ISETP P0 -> predicate guard | 14 | 13 |
| LDG/LDS result -> consumer | scoreboard, no numeric memory latency | group then SB0 wait |

DOCUMENTED: GPR values above come from `latencies/sm100/reg_raw.csv:2,3,5`.
For the guard, `nak/sm70.rs:261-263` uses `raw(write, None)`;
`nak/sm120_instr_latencies.rs:463-469` selects RedirectedFp64 as reader;
`latencies/sm100/pred_raw.csv:11`, Dualalu column, gives 13 before padding.
Thus reading the table backwards would suggest an unsafe predicate reduction.
INFERRED (high): Existing chip success at 13 is not proof that 14 is unnecessary for all
GB10 conditions; 14 is the conservative reference-derived value, not a measured minimum.

UNKNOWN: No digest-matched nvdisasm listing for 22a27c6c or 418f1295 was available in the
inspected artifacts, and the failed quiet check prevented regenerating one with the host test.
The following is a **source/encoder-derived model**, not an observed disassembly or timing.
The existing `/tmp/sieve.bin` hashes to cbac1285, the earlier ungrouped kernel; it was not used.

| INFERRED cycle model, high confidence in static arithmetic | Global 22a27c6c | Shared 418f1295 |
|---|---:|---:|
| Address setup + three loads + terminating test/branch | 12+12+13+5 = 42 | 10+12+13+5 = 40 |
| First-mark calculation: 11 fixed ops + 2 IADD3 | 76 | 76 |
| Final mark-loop test/branch, with no mark | 18 | 18 |
| Table increment + back branch | 11 | 10 |
| Productive prime, excluding active marks and extra memory waits | 147 | 144 |
| Each active mark: test, branch, shift, OR, add, back branch | 40 | 40 |
| One final sentinel/early-break entry | 42 | 40 |
| Staging full iteration: test/branch, address, load, address, store, increment, branch | none | 49 |

DOCUMENTED: E:416-457 contains these prime and mark paths; E:381-394 stages the table.
INFERRED (high): If L is issue-to-ready time for the last of three loads, its additional
wait is approximately max(0,L-4), plus any issue/queue delay. L is UNKNOWN for both LDG
and LDS here; NAK's dependency matrices do not supply it. Branch resolution is also extra.

INFERRED (high, arithmetic): At limit 1e6, the `k0 >= base+32` break means 1,838,431
productive word/prime pairs, averaging 117.6596 primes/word, not 167 for every word.
There are 811,068 scalar marks. The ideal coherent-warp model takes the maximum mark count
over the 32 adjacent words for each prime, summing 57,654 warp/prime pairs and 71,784
warp mark iterations over 489 word-warps. Formula: `147*N + 40*M + 42` per word-warp.
This arithmetic was evaluated independently of the host test; it is not chip evidence.

| INFERRED ideal-warp critical path, excluding setup/tail | 64 CTAs | 123 CTAs |
|---|---:|---:|
| Critical path productive primes / marks | 290 / 348 | 167 / 196 |
| Fixed cycles, including terminating table checks | 56,634 | 32,431 |
| Time if SM clock = 1 GHz | 56.634 us | 32.431 us |
| Time if SM clock = 2 GHz | 28.317 us | 16.216 us |
| Brief's comparison measurement | ~821 us | ~519 us |

UNKNOWN: Actual benchmark SM frequency and resident-warp placement were not recorded in
the inspected receipts. The 48-SM count is a supplied assumption, not verified here from
a vendor specification. At 48 SMs and four schedulers/SM, launch-wide averages are 1.333
and 2.5625 warps/scheduler; CTA distribution, the second grid-stride round and early exits
make those averages misleading for the tail. No 1/(warp-count) latency scaling is assumed.
DOCUMENTED: NVIDIA's [scheduler explanation](https://developer.nvidia.com/docs/drive/drive-os/7.0.3/public/nsight/nsight-graphics/AdvancedLearning/index.html)
describes four schedulers per SM; the [Blackwell tuning guide](https://docs.nvidia.com/cuda/blackwell-tuning-guide/)
gives occupancy limits, not a GB10 SASS delay table or this machine's SM count.

INFERRED (medium): Divergent execution deserves an explicit experiment. E:443-452 has
lane-dependent branches and no emitted structured reconvergence instructions. If lanes
were fully serialized, summing their same static paths gives 1,504,208 / 856,472 cycles,
or 752.1 / 428.2 us at an assumed 2 GHz. This is an illustrative opposite extreme, not
proof of serialization or a calibrated prediction. The single-thread simulator cannot
distinguish these extremes. Do not multiply scalar instruction counts by 32 without evidence.

OBSERVED (existing receipts, read-only): Medians of ten per-trial submit-to-marker averages:
`/home/drakestapleton/workspace/evidence/prime-race/c2/runs/c680704c*.json`: grouped/123 = 510.05 us;
`.../c3/runs/15310126*.json`: grouped/64 = 820.25 us;
`.../c3/runs/c1c17c02*.json`: grouped/64 repeat = 821.16 us;
`.../c3/runs/bdea5434*.json`: staged/64 = 918.16 us;
`.../c3/runs/fa0577a3*.json`: staged/123 = 515.65 us.
The brief's ~519 is from C1, so it is not a matched grouped-kernel comparison.
DOCUMENTED: S:224-236 starts a host clock before submit and ends it after both marker waits;
S:218-223 includes L2 flush and the second release. These are not kernel-only timestamps.
INFERRED (high): The reported scaling supports latency hiding/work distribution, but cannot
uniquely identify fixed stalls. A fitted residual per prime would not independently prove memory latency.

## 2. Why did shared staging lose about 97 us?

UNKNOWN: No isolated measurement determines the cause. Static code does not justify attributing
the entire loss to BAR.SYNC, LDS, or a carveout change.
DOCUMENTED: E:377-398 adds the staging prologue and one barrier before any output-thread exit.
At 1e6 there are 168 entries including sentinel: 504 words / 2016 bytes per CTA.
INFERRED (high): Four staging iterations, a final test/branch and BAR cost about
`4*49 + 18 + 6 = 220` nominal cycles, plus setup and memory/barrier waits. At 1 GHz,
97 us would require ~97,000 extra cycles. The instruction count alone is inadequate.

DOCUMENTED: C:693 emits LDS control from the IR; E:101-119 applies the same grouped
scoreboard policy to LDS and LDG. Shared addresses replace three IMAD.WIDE operations with
IADD3, so they save three nominal cycles per productive prime, not add them.
INFERRED (high): Equal scoreboard policy does not imply equal issue throughput or wait latency.
Same-address lanes need not imply bank conflicts; synchronized lanes request the same table word.
UNKNOWN: Actual LDS latency, conflicts under fragmented execution, and barrier arrival skew.

DOCUMENTED: Q:114-118 changes only the low 11 bits of word 36; min/max/target stay 9/26/9.
The exact values are global `0x04b44808` (1024 bytes/CTA) and staged `0x04b44810`
(2016 rounded to 2048 bytes/CTA). NVIDIA's `clcec0qmd.h:252-255` in the Mesa headers names
these fields. NAK `nak/qmd.rs:551-553` maps size to `(size_kb / 4) + 1`:
the unchanged settings correspond to min/target 32 KiB, max 100 KiB.
INFERRED (high): An explicitly changed carveout setting is ruled out. At 2 KiB/CTA even
the 32 KiB target accommodates 16 CTAs, above this launch's average 1.33 or 2.56 CTAs/SM.
UNKNOWN: Hardware-selected carveout and scheduler placement; the header defines fields,
not all selection behavior. An allocation-triggered effect needs its own A/B.

INFERRED (high): C3 refutes the simple prediction that staging necessarily accelerates this
kernel. It does NOT logically exonerate memory latency: it also changes initialization,
synchronization, address instructions and the memory pipeline. C2a's improvement remains
evidence that serialized requests were costly. The much smaller 123-CTA penalty (~5.6 us
against the separate grouped campaign) favors an occupancy-sensitive effect over a universal
97-us barrier tax, but that comparison is not interleaved and has unknown clock differences.

## 3. Patch and host validation

DOCUMENTED: E:498-552 adds a general local scheduling pass across every elementwise family.
It tightens supported adjacent ALU/dual-ALU or FMA/FMA pairs from six to five cycles.
Mixed classes, IMAD.WIDE producers, IADD3's historical encoding, instruction order, register
allocation, waits and CTA count remain unchanged. E:58 uses predicate delay 14 from NAK.
INFERRED (high): These pair bounds permit every immediate scalar dependency in scope;
later consumers retain at least nine cycles. Scalar WAR/WAW table values are <=2 here.
This is a conservative candidate, not a chip-qualified scheduling policy.
INFERRED (high, static): The productive-prime model becomes 143 global / 140 shared cycles,
and an active mark becomes 39. Savings are deliberately modest: about 1,500 cycles on the
64-CTA ideal critical path, less than 2 us at 1 GHz. A large measured gain would falsify
the simple delay-sum model and require investigating changed issue/branch behavior.

DOCUMENTED: H:238 onward adds a fixed-RAW checker using encoded control words, allocated
registers, forward/back branches and predicate guards. It adds failing mutations for ALU
delay 4 and predicate delay 13. H:464 onward checks all seven families and both mutants.
The existing SB0 checker, oracle sweeps, shared-race and short-allocation controls remain.
UNKNOWN: This checker is not a complete ISA verifier: variable latency and wide-result
dependencies retain existing coverage; register-read barriers and warp reconvergence are
not fully modeled. Passing it would not replace chip correctness checks.

OBSERVED: `git diff --check` passed. No compiler, nvdisasm regeneration, simulator, or new
hazard result was obtained. **There is no all-checks-PASS claim and no measured speedup.**
DOCUMENTED: Required validation command once the real quiet check is permitted and exits 0:
`quietlock check && make -s prime-race-gb10-host-test PHYSICS_DIR=/home/drakestapleton/workspace/primes-race/physics`
Run from this worktree. Required result: final `VERDICT PASS`, zero SB0 and fixed-RAW hazards,
and all negative controls detected. Do not substitute a different quiet lock.

## 4. One-variable experiments for the orchestrator

INFERRED (high, experimental design): Use correctness-checked, interleaved A/B windows,
limit 1e6, 128 threads/CTA, **64 CTAs for scheduling A/B**, identical spin setting, and
record clocks and kernel hashes. Capture both kernel-only time and submit-to-marker time.
The existing launch-cost command shape is `gb10_launch_cost sieve N 64 128 1000000`;
it always selects the global kernel (source: `bench/prime_race/tests/gb10_launch_cost.c:68`).
The benchmark currently selects shared automatically (`bench/prime_race/gb10_native.c:150-151`).
Thus compare the same harness/kernel mode on both sides. These are proposed chip runs only.

| INFERRED experiment, one changed variable | Confirms / expected number | Refutes or limits claim |
|---|---|---|
| Predicate padding alone: 13 -> 14, same kernel otherwise | Same bitmap; small timing cost supports safe padding | Mismatch refutes candidate; a large speedup exposes missing scheduling behavior |
| Five-cycle pair pass off/on, both with predicate 14, 64 CTAs | Zero host hazards; correct bitmap; modest repeatable kernel-only saving (static ideal ~2 us at 1 GHz) | No resolved gain means stalls optimized here are hidden; mismatch rejects patch |
| Fixed ALU delays 6 -> 12 only, all else original | Delay-dominated model predicts proportional added time from counted ALUs | Little kernel-only change rejects fixed stalls as dominant |
| Structured reconvergence around mark loop only, same body and arithmetic | Large speedup plus higher active lanes per issued instruction supports fragmentation | <5% change with matched clocks weakens it; require new control-flow correctness qualification |
| Global kernel: reserve 1024 -> 2048 shared bytes only | ~97 us loss supports allocation/configuration mechanism | <5 us difference rules out most of that mechanism |
| Global kernel: add the C3 staging prefix/barrier, still read global table | Added time near 97 us supports prefix/barrier explanation | <5 us limits prefix cost; remaining C3 loss lies elsewhere |
| Same staged prefix and both address forms computed in both kernels: switch only three table loads LDG -> LDS | ~97 us loss localizes recurring shared read path | Near equality rejects that recurring-path explanation |
| With identical prefix, emit paired LDG/LDS latency probes with identical SB waits | LDS penalty sufficient to explain ~97 us / 290 primes supports wait-path hypothesis | Small LDS wait gap rules it out; do not infer from throughput alone |
| One bit only: variable-op bit 109 off/on | Repeatable timing change gives empirical meaning on SM121 | No effect supports treating it as unquantified, not a known yield tax |
| Marker spin 0 -> 2000 us only, collect kernel timestamps too | Marker-time change with stable kernel time isolates host polling | Kernel time also shifts: check clock/load coupling |

UNKNOWN: Dual issue and precise LDS/LDG/branch latency cannot be certified from public
control-word documentation. NAK `nak/sm70.rs:198` still says `TODO: co-issue`.
Do not use zero-delay dual issue or remove dependency waits as the first candidate.
