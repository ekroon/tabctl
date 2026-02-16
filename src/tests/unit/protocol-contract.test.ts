import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

test("host protocol contract includes expected action sets", () => {
  const protocolPath = path.resolve(__dirname, "../../../config/protocol/host-protocol.v1.json");
  const raw = fs.readFileSync(protocolPath, "utf-8");
  const contract = JSON.parse(raw) as {
    version: number;
    actions: { local: string[]; forwarded: string[] };
  };

  assert.equal(contract.version, 1);
  assert.deepEqual(contract.actions.local, ["history", "undo", "version"]);

  const forwarded = new Set(contract.actions.forwarded);
  for (const action of ["ping", "list", "analyze", "close", "reload"]) {
    assert.ok(forwarded.has(action), `missing forwarded action: ${action}`);
  }
});
