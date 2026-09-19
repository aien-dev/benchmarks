# Contributing to AIEN Benchmarks

We welcome reproducible benchmark contributions from sovereign operators.

## Submission Requirements

1. **Measurement Environment**: Document exact CPU/GPU model, kernel version, memory architecture, and compiler flags.
2. **Reproducible Code**: Include CLI flags and measurement scripts so any operator can reproduce the results.
3. **Verification**: Run `cargo test --verbose` and ensure all tests pass with zero warnings.
4. **License & Provenance**: All contributions are accepted under the Apache License, Version 2.0 and the Developer Certificate of Origin (DCO 1.1).
