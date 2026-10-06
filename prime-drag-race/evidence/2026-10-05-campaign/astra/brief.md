# Brief for GPT-6 Astra: why is our native GB10 prime-sieve kernel slow, and why did shared memory make it slower?

You are a precision investigator. ANALYSIS + HOST-ONLY experiments. Report back; the orchestrator runs any chip test.

## Hard rules
- NEVER launch anything on the GPU: do not run gb10_native, gb10_native_mutant, gb10_launch_cost, run_gb10_chip.sh,
  or anything with GB10_CHIP_RUN=1. Do not touch /tmp/aien-gb10.lock. No sudo.
- Work only inside your worktree: /home/drakestapleton/.claude/jobs/ab7b59e1/tmp/astra (detached at omega d6632db). Do not edit any other checkout. Do not push. No git commits needed;
  deliver changes as a patch file in /home/drakestapleton/.claude/jobs/ab7b59e1/tmp/astra-out.
- Before any build or host test: run `quietlock check`; only proceed if it exits 0 (another agent may be timing the chip).
  Build: `cd /home/drakestapleton/.claude/jobs/ab7b59e1/tmp/astra && make -s prime-race-gb10-host-test PHYSICS_DIR=/home/drakestapleton/workspace/primes-race/physics` (CPU only, ~1-2 min).
- No Python in the repo (scratch scripts outside the repo are fine). No CUDA in the product. C only.
- Tag every claim OBSERVED / DOCUMENTED (quote file:line) / INFERRED (confidence) / UNKNOWN. NVIDIA does not document SASS
  control words; say "DOCS SILENT" where so.

## The code
- Kernel generator: /home/drakestapleton/.claude/jobs/ab7b59e1/tmp/astra/src/omega_gpu_elementwise_api.c. Emitter policy lines ~30-70: CW(stall,wbar,rbar,wait);
  K_FIXED stall 6, K_PRED stall 13, K_VAR stall 4 + write scoreboard 0 and the NEXT instruction waits on it
  (fully serialized), K_BRA/K_EXIT stall 5. ld_group() issues several loads then one wait. gen_prime_sieve() ~line 369.
  A comment notes the IADD3 encoder ignores the control word (fixed 0x010fca00).
- Encoders: /home/drakestapleton/.claude/jobs/ab7b59e1/tmp/astra/src/omega_blackwell_codegen*.c (grep for control). IR: omega_blackwell_codegen.h.
- Host test with an IR simulator + scoreboard hazard checker + nvdisasm listing:
  /home/drakestapleton/.claude/jobs/ab7b59e1/tmp/astra/bench/prime_race/tests/gb10_sieve_host_test.c (nvdisasm at /usr/local/cuda/bin/nvdisasm, -b SM121).
- Host side: /home/drakestapleton/.claude/jobs/ab7b59e1/tmp/astra/bench/prime_race/gb10_native.c; launch path src/omega_gpu_session.c.
- Reference for real Blackwell latencies/scheduling (open source, Mesa NAK):
  ~/workspace/.ref/mesa/src/nouveau/compiler/nak/sm120_instr_latencies.rs, calc_instr_deps.rs,
  opt_instr_sched_*.rs, and ~/workspace/.ref/mesa/src/nouveau/compiler/latencies/sm100. GB10 is SM 12.1.
- Algorithm: limit 1e6 -> 15625 32-bit words of odd-number bits; one thread per word (grid-stride), 128 threads/CTA.
  Per word: loop over 167 base primes (table {p, magic, k0} 12 B/entry), compute first multiple offset with a
  magic-number division, then an inner mark loop sets bits (j += p while j < 32). Then one store.

## Measurements on the chip (all OBSERVED, medians, 10 x 5 s each, correctness verified every time)
Frozen baseline: 971.48 passes/s (64 CTAs). CUDA yardstick 19591.67 passes/s, kernel-only 35.8 us. CPU 2069.
- C1 CTA count 64 -> 123 (same kernel): 966 -> 1605 passes/s; device time per launch ~931 -> ~519 us (1.79x for 1.92x threads).
- C2a group the 3 table loads into one round trip (ld_group): 966 -> 1207 at 64 CTAs (1.25x); 1940 at 123 CTAs.
- C3 table staged once per CTA into shared memory, loop reads LDS instead of uncached LDG (same instruction count
  per iteration otherwise): 1210 -> 1082 at 64 CTAs (SLOWER, device time per launch 821 -> 918 us);
  1915 at 123 CTAs vs 1940 for C2a. So uncached table loads are NOT the bottleneck; my earlier inference that the
  loop was bound by the count of uncached requests is refuted.
- C4 host-side marker wait spin: empty-kernel launch floor 103 -> 6 us; sieve launch 823 -> 785 us. (host-side, done)
- GB10 has 48 SMs (verify from docs if you can). 64 CTAs x 4 warps = 256 warps -> ~5 warps per SM, ~1.3 per scheduler.

## Questions (answer in this order)
1. What bounds the prime loop? Estimate cycles per prime-loop iteration and per mark-loop iteration from the actual
   control words (stall counts, waits, yields) in the nvdisasm listing of the default kernel (sha 22a27c6c) vs the latency
   tables in NAK. Compare the predicted device time at 64 and 123 CTAs with the measured 821 / ~519 us. Leading hypothesis
   (INFERRED by orchestrator, unverified): fixed stall counts (6 for every ALU op, 13 for every ISETP) with ~1 warp per
   scheduler make the loop issue-latency bound. Confirm or refute with numbers.
2. Why is C3 (staged, sha 418f1295) slower at 64 CTAs by ~97 us/launch? Candidates: staging loop cost, BAR.SYNC,
   LDS latency/control words, shared memory config/carveout in the QMD (word 36; see src/omega_blackwell_qmd.c and
   ~/workspace/.ref/mesa/src/nouveau/headers/nvidia/classes/clcec0qmd.h), something else.
3. Propose the smallest scheduling change that is safe: correct per-op stall counts from NAK's latency table instead of the
   blanket 6/13, letting independent instructions overlap, not waiting on scoreboards that aren't needed, dual issue, etc.
   It must keep the host test's scoreboard hazard checker at 0 hazards (and extend the checker if the change introduces
   a new hazard class, e.g. fixed-latency RAW with too-short stall). Implement it in your worktree as a GENERAL emitter
   change if possible (benefits every Omega elementwise kernel), host-test it (all checks PASS), save `git diff` to
   /home/drakestapleton/.claude/jobs/ab7b59e1/tmp/astra-out/astra.patch.
4. For each claim, give the one-variable chip experiment that would confirm it (what to run, what number would confirm,
   what would refute). Keep CTA count fixed at 64 for the scheduling A/B.

## Deliverable
Write /home/drakestapleton/.claude/jobs/ab7b59e1/tmp/astra-out/REPORT.md: findings with tags, cycle model table, the patch summary + host test output summary, proposed chip
experiments. Keep it under ~250 lines. Plain, precise.
