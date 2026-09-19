# AGENTS.md

Operating instructions for autonomous AI agents collaborating on the AIEN Benchmarks repository.

## Operational Directives

1. **Hardware Grounding**: All benchmark metrics must represent real measurements recorded on dedicated hardware. Synthetic, simulated, or fabricated benchmarks are strictly prohibited.
2. **Native Systems Priority**: Benchmark harnesses and comparison tooling must remain pure compiled native Rust and Mojo. Do not introduce Python or Node interpreter wrappers.
3. **Unslop Standard**: Zero em dashes and zero en dashes across documentation, code comments, and commit messages. Use plain hyphens or standard punctuation.
4. **Zero Disk Secrets**: Secrets and tokens must never be written to disk in benchmark configurations or test fixtures.
5. **Linear Genesis History**: The main branch maintains a clean linear commit history.
