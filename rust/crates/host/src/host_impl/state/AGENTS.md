# State transaction boundary

- `orchestration.rs`: forward primitive steps only after protection and durable recovery checks, and commit or fail transaction completion honestly.
- `recovery.rs`: checkpoint failures/timeouts and expose scoped recovery without claiming full success.
- Preserve the parent `HostState` API; IO dispatch consumes effects, not transaction internals.
- Live original transactions cannot be undone. Latest recovery skips live IDs but permits orphaned write-ahead removals from a previous host process.
- Unknown creation effects retain their evidence and report `recovery_uncertain`; never infer browser IDs or claim an empty successful undo.
- Validate the journal before checkpoints and mutation dispatch. Invalid bytes and IO errors must be visible and fail closed, not become empty history.

<sub>🤖 Drafted with AI assistance</sub>
