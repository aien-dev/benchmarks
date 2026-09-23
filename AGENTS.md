# AGENTS.md

Operating instructions for autonomous AI agents collaborating on the AIEN Benchmarks repository.

## Operational Directives

1. **Hardware Grounding**: All benchmark metrics must represent real measurements recorded on dedicated hardware. Synthetic, simulated, or fabricated benchmarks are strictly prohibited.
2. **Native Systems Priority**: Benchmark harnesses and comparison tooling must remain pure compiled native Rust and Mojo. Do not introduce Python or Node interpreter wrappers.
3. **Unslop Standard**: Zero em dashes and zero en dashes across documentation, code comments, and commit messages. Use plain hyphens or standard punctuation.
4. **Zero Disk Secrets**: Secrets and tokens must never be written to disk in benchmark configurations or test fixtures.
5. **Linear Genesis History**: The main branch maintains a clean linear commit history.

## Agent Identity Standard

Commits authored or materially assisted by AI agents must carry identity
trailers per the org-wide standard (https://github.com/aien-dev/.github/blob/main/AGENT_IDENTITY.md):
`Agent-Name`, `Agent-Model`, `Agent-Provider`, `Agent-Session`, and
`Assisted-by: <name>:<model>`, so any problem can be traced to the agent,
model, version, and session that produced it. Values come from the agent
runtime and are never invented. Agents must never add `Signed-off-by`;
DCO certification is human-only.
