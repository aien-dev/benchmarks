# AIEN Prime Drag Race: benchmark contract (version 1)

This contract is frozen. Any change to a rule below is a new contract version.
The runner in this directory enforces it. Contract version string: `1`.

## What is measured

Sieve all primes up to a limit (default 1,000,000), over and over, for at least a minimum time
(default 5 seconds), and count how many complete passes finish. The score is passes per second.
Higher is faster.

## The canonical result

For limit L, only odd numbers are stored. Bit k (word k/64, bit k%64, little-endian 64-bit words,
which is the same bytes as little-endian 32-bit words) is set exactly when 2k+1 is prime, for
0 <= k < (L+1)/2. All bits past that range are zero. The prime 2 is implicit when L >= 2. The count
of primes is the number of set bits plus 1 when L >= 2. For L = 1,000,000 the count is 78,498.

The runner builds this result itself with an independent reference: deterministic Miller-Rabin on
every odd number up to L (bases 2, 3, 5, 7 below 3,215,031,751, otherwise the first 12 primes, with
128-bit multiplication). The reference is checked against trial division and the known prime counts
(10:4, 100:25, 1,000:168, 10,000:1,229, 100,000:9,592, 1,000,000:78,498, 10,000,000:664,579).

## What one pass includes

Everything needed to go from nothing to a complete result that the host can read: reset or allocate
the sieve state, compute the base primes up to sqrt(L) (for GPUs, on the host, written into a
device-visible buffer), launch the work, wait for confirmed completion, and have the result sitting
in host-readable memory. Excluded: process start, one-time setup (opening a GPU session, uploading
kernel code, allocating persistent buffers), and the final export and checking of the answer.
A CPU pass allocates a fresh zeroed sieve each time (faithful). A GPU pass that keeps persistent
buffers is marked not faithful.

## Implementation interface

`<impl> [--limit N] [--min-seconds S] [--bitmap-out PATH] [--report-out PATH] [--audit-passes K]`

- Timed mode: loop passes against a monotonic clock until S seconds have passed. Elapsed is the
  actual time spent. After the loop, outside the timing, export the last pass's canonical bitmap and
  write the report. Print exactly one line on stdout, and only on success:
  `<label>;<passes>;<elapsed %.6f>;<threads>;algorithm=..,faithful=..,bits=..`
  The label is `aien-` followed by the implementation name. Everything else goes to stderr.
- Audit mode (K passes): run K full passes and append each pass's canonical bitmap to the bitmap
  file in order. This proves each pass recomputes the answer and does not reuse an old one.
- Failure: no stdout line, report status `EXEC_FAILED`, exit code 2. Usage errors exit 64.
- Report: JSON, schema `aien-prime-race/impl-report/v1`, with fields impl, label, algorithm,
  faithful, bits, threads, environment, limit, min_seconds, mode (`timed` or `audit`), passes,
  elapsed_s, status (`COMPLETE`), error, build, and an implementation-specific detail object.

## Labels

Each implementation declares its tags to the runner (`--declare`). The runner rejects any run whose
line or report differs from the declaration or from each other. Known declarations:

- `cpu-c-base`: algorithm=base, faithful=yes, bits=1, threads=1.
- `gb10-native`: algorithm=other, faithful=no, bits=1, threads = launched GPU threads.
- CUDA mirror: same labels as gb10-native.

## Timing rules

Monotonic clock only. GPU work counts only after completion is confirmed. Kernel-only time is
recorded as a diagnostic and is never the score.

## Trial protocol

Sequential, one machine, under the owner's quiet flag with nothing else running. Per implementation:
one audit run (K=5), one discarded warm-up run, then 10 timed trials of at least 5 seconds each.
Trials are interleaved round-robin across implementations (A,B,C,A,B,C,...) to spread drift.
Per trial the runner records passes per second, the raw line, the report, and the load average
before and after. Statistics (median, min, max, mean, sample standard deviation, coefficient of
variation of passes per second) are computed only when every audit and trial passed.

## Correctness gates (before any speed number)

1. Exit code 0, exactly one valid stdout line, status COMPLETE, passes >= 1, elapsed >= min seconds.
2. Line fields equal report fields. Label and tags equal the declaration.
3. Bitmap length exact and byte-for-byte equal to the reference. A mismatch reports the first
   differing odd number, or tail garbage.
4. Audit mode: every one of the K bitmaps equals the reference.
5. Sweep (`prime_race verify`): limits 0 to 5, 8, 9, 15, 25, 31 to 34, 49, 63 to 66, 95 to 97, 121,
   127 to 130, 169, 255 to 258, 289, 361, 961, 1023 to 1025, 1999 to 2001, 4095 to 4097, word and
   block boundaries plus and minus 1, every prime square p^2 and p^2 plus or minus 1 up to p = 997,
   999,983 to 1,000,003, and 1,000,000. The 10,000,000 limit is a scaling check run separately.

A failing run produces a FAIL receipt with all evidence kept, and no statistics.

## Receipts

Schema `aien-prime-race/receipt/v1`. One JSON file named by the SHA-256 of its own bytes, mode 0444,
never overwritten, never written inside a source checkout. Raw stdout, stderr, reports and bitmaps
are stored as `blobs/<sha256>` and referenced by digest. A receipt records verdict and reason,
implementation identity (binary SHA-256, source repo and commit, build command and flags),
runner commit and binary hash, hardware (CPU model, GPU name, UUID and driver, kernel), conditions
(quiet flag contents, load averages, CPU governor), correctness evidence, every raw trial, summary
statistics (PASS only), the upstream-style lines, and reproduction commands.

## Environment classes

Results are only comparable inside one class:

1. Host CPU (`linux-host-cpu`).
2. Linux-hosted GB10, native AIEN path (`linux-hosted-gb10-native`).
3. Linux-hosted GB10, CUDA yardstick (`linux-hosted-gb10-cuda`). A yardstick outside AIEN, for comparison only.
4. AIENOS booted natively: not yet measured.

## Upstream relationship

The format follows the upstream Drag Race, PlummersSoftwareLLC/Primes `drag-race`, pinned at
commit a2899c96752b65ac1b08a0e1418ab7e708040ed0. Our entries are NONCONFORMING for upstream:
our license is AGPL-3.0-or-later where upstream requires BSD-3-Clause, and the native GPU entry
uses a language upstream does not accept. They are published as experimental, never as upstream
leaderboard entries.

## Running it

    cargo build --release        # in this directory
    prime_race verify --impl ./cpu_base --name cpu-c-base \
        --declare algorithm=base,faithful=yes,bits=1,threads=1
    prime_race run --impl cpu-c-base=./cpu_base --declare cpu-c-base=algorithm=base,faithful=yes,bits=1,threads=1 \
        --source cpu-c-base=aien-dev/omega@COMMIT --build 'cpu-c-base=cc -O3 -mcpu=native ...' \
        --evidence-dir ~/workspace/evidence/prime-race
