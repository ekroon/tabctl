# shared Crate Agent Guide

## Responsibility
- Shared protocol/config/profile types for tabctl + host.

## Constraints
- Maintain serialization compatibility (`serde` shape is a contract).
- Prefer additive, backward-compatible changes to shared structs/enums.
