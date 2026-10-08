#!/usr/bin/env node
"use strict";

require("../lib/browser-fixture").run({
  browser: "chrome",
  profile: "it",
  root: process.env.TABCTL_FIXTURE_ROOT,
});
