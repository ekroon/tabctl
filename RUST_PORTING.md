# Rust Porting Investigation

This document evaluates how much of `tabctl` can be ported from TypeScript/Node.js to Rust with minimal product risk.

## Current architecture and portability

| Component | Current implementation | Rust port feasibility | Notes |
|---|---|---|---|
| CLI (`src/cli`) | TypeScript + Node | **High** | Mostly argument parsing, request shaping, JSON output formatting, and local file/config access. |
| Native host (`src/host`) | TypeScript + Node | **High** | Socket server + native messaging bridge + undo/history logic are a good Rust fit. |
| Shared runtime/config (`src/shared`) | TypeScript + Node | **High** | Path resolution, profiles, wrapper health, version checks are straightforward to port. |
| Browser extension (`src/extension`) | MV3 background service worker | **Low (direct)** / **Medium (partial WASM)** | Heavily coupled to `chrome.*` extension APIs and script injection. Main orchestration must stay JS/TS. |
| Windows launcher (`src/host/launcher/main.go`) | Go | **Optional** | Already native and tiny; can stay Go or be replaced with Rust later for language consolidation. |

## Why the extension is hard to port fully

The extension entrypoint (`src/extension/background.ts`) and extension modules rely directly on Chrome Extension APIs (`chrome.runtime`, `chrome.tabs`, `chrome.windows`, `chrome.tabGroups`, `chrome.scripting`, `chrome.storage`, alarms/events). Those APIs are JavaScript-first and event-driven in the service worker.

A full Rust rewrite of that layer is not practical today. A hybrid can work:

- Keep background orchestration in TypeScript.
- Move compute-heavy pure logic to Rust/WASM only where it clearly pays off.
- Avoid putting browser API calls inside WASM boundaries.

## Rust-first target areas (recommended order)

### 1) Port native host first (best ROI)

Port `src/host/host.ts` + `src/host/lib/handlers.ts` + undo file helpers to a Rust binary:

- Native messaging stdio framing (4-byte little-endian length prefix).
- Local socket/named-pipe server for CLI requests.
- Request routing/proxying to extension.
- Undo/history retention logic.
- Version metadata in host responses.

Why first:

- Isolated boundary with explicit JSON protocol.
- Performance/safety improvements (memory safety, robust I/O handling).
- No extension store/runtime constraints.

### 2) Port CLI next

Port `src/cli/tabctl.ts` and command builders to Rust:

- Argument parsing and validation.
- Scope/option mapping to existing request schema.
- Output formatting (JSON, markdown/csv report formatting if retained in CLI).
- Setup/doctor/profile commands and local filesystem operations.

This removes Node as a runtime dependency for the two local binaries users interact with most.

### 3) Keep extension in TypeScript; selectively add WASM

Potential WASM candidates:

- URL normalization/dedupe-heavy comparison paths.
- Pure analysis transforms that do not call Chrome APIs.

Keep in TS:

- All browser API interaction.
- Service worker lifecycle and event listeners.
- Script injection/orchestration.

## Migration strategy

1. **Stabilize protocol contract first**
   - Freeze request/response envelopes between CLI ↔ host ↔ extension.
   - Add/keep regression tests around protocol behavior.
2. **Ship Rust host behind compatibility parity**
   - Preserve action names and response fields.
   - Reuse existing extension unchanged.
3. **Ship Rust CLI against same host protocol**
   - Preserve command UX and JSON shapes.
4. **Optional: evaluate targeted WASM in extension**
   - Only for measured hotspots.

## Key compatibility constraints to preserve

- Socket path and profile resolution behavior (`src/shared/config.ts` semantics).
- Undo transaction and retention behavior (`history`, `undo`, txid flow).
- Version mismatch handling (host/extension/cli metadata fields).
- Existing command and output shapes used by tests and agent workflows.

## Risks and mitigations

- **Risk:** Protocol drift between Rust and extension TS.  
  **Mitigation:** Generate shared JSON schema fixtures and run compatibility tests across implementations.

- **Risk:** Cross-platform IPC differences (Unix sockets vs Windows named pipes).  
  **Mitigation:** Implement platform integration tests equivalent to current integration suite behavior.

- **Risk:** Setup/wrapper behavior regressions.  
  **Mitigation:** Keep wrapper/manifest output byte-compatible where possible; validate with current setup and doctor test scenarios.

## Concrete next steps for an implementation phase

1. Define and check in a protocol contract (JSON schema or typed fixtures).
2. Implement Rust host MVP supporting: `ping`, `version`, `list` proxy, `analyze` proxy, `history`, `undo`, `close --apply`.
3. Add an A/B switch (env flag) to run Node host vs Rust host in integration tests.
4. Reach full host action parity.
5. Start Rust CLI with parity for read-only commands first, then mutating commands.

## Current execution status in this repository

- Protocol contract fixture added: `config/protocol/host-protocol.v1.json`
- Rust host MVP crate scaffold added: `rust/tabctl-host-mvp/`
  - Implements native messaging framing.
  - Implements local MVP actions: `ping`, `version`, `history`, `undo` (placeholder semantics).
  - Returns explicit "not implemented" for forwarded extension actions until proxy wiring is added.
- Unit guard added to keep contract action sets stable: `src/tests/unit/protocol-contract.test.ts`

## Recommendation

Porting the **native host + CLI** to Rust is practical and high-value.  
Porting the **extension core** to Rust is not practical as a full rewrite; keep TypeScript there and only consider selective WASM for isolated pure logic after profiling.
