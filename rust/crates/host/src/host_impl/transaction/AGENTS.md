# Transaction privacy

- Preflight the complete planned tab/window scope before the first mutation. Include source and destination privacy without broadening to unrelated windows.
- Reject mixed privacy before writing recovery data or dispatching mutations; private-only recovery is never persisted.
- Preserve the transaction's privacy mode across subsequent primitives and newly created browser IDs.

<sub>🤖 Drafted with AI assistance</sub>
