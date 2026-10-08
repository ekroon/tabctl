# Shared browser fixture

- `browser-fixture.js` owns the browser lifecycle shared by integration and smoke entrypoints.
- Configuration, native-host manifests, wrappers, extension and browser data must stay beneath the fixture root.
- Verify production setup's extension ID against the browser; do not rewrite it after loading.
- Never modify normal browser registrations. Teardown only the exact fixture-owned browser process and directory.
- `browser-control.js` seeds and cleans the isolated browser through CDP, never a production primitive bypass. Assertions use the real CLI/host path.
- Launch macOS test browsers with `--use-mock-keychain` and `--password-store=basic` to prevent keychain prompts.
