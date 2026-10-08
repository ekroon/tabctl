# Undo restoration

- `entries.rs`: normalize historical and write-ahead recovery payloads.
- `restore.rs`: reconcile browser IDs, restore placement/group metadata, and clean up only recorded created tabs.
- Never recreate a tab merely because a primitive was planned. Recovery uses issued targets and current browser existence.

<sub>🤖 Drafted with AI assistance</sub>
