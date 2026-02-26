# host_impl Module Guide

- `protocol.rs`: native-message framing, host metadata/version helpers, shared action sets.
- `state.rs`: `HostState` request handling, protocol routing, analyze/close apply, undo recording.
- `undo.rs`: undo log read/append/find with retention filtering.
- `dispatch.rs`: client IO handling and native message dispatch.
- `runtime.rs`: socket/pipe/tcp runtime bootstrap and `run()` host entry wiring.
