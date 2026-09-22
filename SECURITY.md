# Security Policy

## Zero Disk Secrets Invariant

This repository maintains a clean disk state. API keys, hardware tokens, and credentials must never reside on disk in plaintext. All secrets resolve in-memory from hardware TPM-bound key vaults.

## Reporting Vulnerabilities

To report a vulnerability or telemetry leak, contact:
`aien@aienos.com`
