# Transaction privacy

- Preflight the complete planned tab/window scope before the first mutation. Include source and destination privacy without broadening to unrelated windows.
- Reject mixed privacy before writing recovery data or dispatching mutations; private-only recovery is never persisted.
- Preserve the transaction's privacy mode across subsequent primitives and newly created browser IDs.
- Policy admission checks all planned tab IDs and existing group members before mutation. Window IDs describe privacy context only, not blanket mutation of unrelated tabs.

<sub>🤖 Drafted with AI assistance</sub>
