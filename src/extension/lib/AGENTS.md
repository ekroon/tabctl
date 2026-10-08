# Extension primitive utilities

- `content.ts`: injected content extraction and quiescence probes.
- `screenshot.ts`: image capture and tiling.
- `native-message.ts`: the only outbound native-message boundary. It measures the entire JSON envelope as UTF-8 against the host's 10 MiB cap, bounds HTML with explicit truncation metadata, and rejects oversized indivisible results.
- `native-message.test.ts`: pure byte-budget regression tests; no browser or profile registrations.

Keep orchestration in the Rust host. All background sends must use `postNativeMessage`.
