# host_impl Module Guide

- `protocol.rs`: native-message framing, host metadata/version helpers, shared action sets.
- `state.rs`: `HostState` request handling and protocol routing; `state/orchestration.rs` enforces mutation boundaries and completion.
- `undo.rs`: undo log read/append/find with retention filtering.
- `transaction.rs`: scoped write-ahead recovery and durable mutation checkpoints; never issue destructive primitives before recovery is persisted.
- `policy.rs`: typed protection evaluation shared by mutation plans and the final primitive boundary.
- `dispatch.rs`: client IO handling and native message dispatch.
- `runtime.rs`: macOS Unix-domain socket bootstrap and `run()` host entry wiring.
- `orchestrate/`: command orchestrations — each sequences extension primitives per CLI request. See `orchestrate/AGENTS.md`.
