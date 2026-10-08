# GraphQL adapter guide

- `src/lib.rs`: public API forwarding (`execute`, `CommandSender`, schema discovery).
- `src/execution.rs`: query execution and JSON envelopes. Preserve partial data, error paths, and diagnostics; never hide a resolver error behind a successful CLI status.
- `src/schema.rs`: resolver routing and field projection.
- `src/response.rs`: strict host contracts for connectivity, preview, and undo.
- `src/types.rs`, `src/convert.rs`: GraphQL data shapes and snapshot conversion.

Keep host operations in the host crate. Prefer pure parser tests and actual browser integration over canned mutation-success fixtures.
