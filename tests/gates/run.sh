#!/usr/bin/env bash
# tests/gates/run.sh: the one entry point for the local verification gates of benchmarks.
# Every step runs the SAME command as the hosted job it mirrors. The canonical CI of this
# repository is .forgejo/workflows/ci.yml (runs on the Spark); .github/workflows/ci.yml is
# workflow_dispatch only and last ran 2026-09-19, before Cargo.toml gained path
# dependencies on ../canonical-branch-native-runtime (#10, 2026-09-20). Manifests under
# tests/gates/ name this script plus one step word, so no manifest needs shell expansion
# (ADR 0028 P6).
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
step="${1:?usage: tests/gates/run.sh <step>}"
obs() { printf 'AIEN-OBS %s %s\n' "$1" "$2"; }

# Cargo.toml depends by path on ../canonical-branch-native-runtime (an aien-sovereign-core
# branch checkout). Without it nothing compiles, on any runner; say so instead of failing
# inside cargo, and record which commit of it was used.
need_sibling() {
  if [ ! -d ../canonical-branch-native-runtime/crates ]; then
    echo "run.sh: ../canonical-branch-native-runtime is missing (Cargo.toml path dependencies)" >&2
    exit 3
  fi
  obs sibling_runtime_commit "\"$(git -C ../canonical-branch-native-runtime rev-parse HEAD 2>/dev/null || echo unknown)\""
}

case "$step" in
  fmt)
    need_sibling
    cargo fmt -p aien-benchmarks -- --check
    ;;
  check)
    need_sibling
    cargo check --all-targets
    ;;
  test)
    need_sibling
    cargo test --verbose
    ;;
  verify)
    need_sibling
    cargo run --release -- verify
    ;;
  generate)
    need_sibling
    # Hosted: cargo run --release -- generate --output assets. It rewrites the tracked
    # files under assets/; the runner's clean_tree_after check then proves the committed
    # assets are exactly what the generator produces (a drift is a FAIL here, not hidden).
    cargo run --release -- generate --output assets
    obs assets_changed "$( [ -z "$(git status --porcelain -- assets)" ] && echo false || echo true)"
    ;;
  verify-records)
    need_sibling
    # Not a hosted step. Offline evidence-integrity check (aien-architecture#233 lane P2): one
    # stdout line per defect, exit 1 on any. An absent records/ is a note on stderr, not a defect.
    out="$(cargo run --release --bin bench_verify_records -- records artifacts tests/fixtures/records/good)"
    rc=$?
    [ -n "$out" ] && printf '%s\n' "$out"
    obs record_defects "$( [ -z "$out" ] && echo 0 || printf '%s\n' "$out" | wc -l | tr -d ' ')"
    exit "$rc"
    ;;
  perf-index)
    need_sibling
    # Not a hosted step. Rewrites docs/PERF_INDEX.md; like generate, a drift from the committed
    # copy shows as a dirty tree and perf_index_changed == true.
    cargo run --release --bin bench_index
    obs perf_index_changed "$( [ -z "$(git status --porcelain -- docs/PERF_INDEX.md)" ] && echo false || echo true)"
    ;;
  zero-disk-secrets)
    # Not a hosted step in this repository; the same invariant the sibling repos enforce.
    echo "Auditing repository for prohibited plaintext secret files..."
    if find . -maxdepth 4 -name ".env*" -not -path "*/.git/*" -not -path "*/target/*" | grep -q .; then
      echo "::error::Prohibited plaintext .env file found. Use an approved AIEN SecretProvider."
      obs env_files_found true
      exit 1
    fi
    obs env_files_found false
    echo "Zero plaintext .env files verified."
    ;;
  *)
    echo "run.sh: unknown step '$step'" >&2
    exit 2
    ;;
esac
