# Browser integration fixtures

- `mod.rs` exports the test API; lifecycle, CLI I/O, HTTP pages and resources live in leaf modules.
- Use production setup with an explicit disposable `--user-data-dir`. Never patch native-host registrations or extension IDs to make tests pass.
- The fixture owns every browser tab it touches. Use `TABCTL_TEST_BROWSER=chrome|edge` to run the same contracts against either browser.
- Drain CLI stdout/stderr while waiting and preserve nonzero exits. Never automatically retry mutations.
- `seed_browser` uses the fixture's CDP control socket only for arranging browser state and cleanup; behavior under test goes through the production CLI.
