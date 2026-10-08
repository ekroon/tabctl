# Browser reliability regressions

- Exercise the production CLI, host and real Chrome/Edge APIs in the shared isolated fixture.
- Assert exact state around previews, policy enforcement and undo; counts alone hide ordering/grouping bugs.
- Serve large content locally. Use a genuine browser API failure for recovery tests, not a mocked success sequence.
