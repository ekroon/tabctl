import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

type JsonSchema = {
  type?: string | string[];
  required?: string[];
  properties?: Record<string, JsonSchema>;
};

type ProtocolContract = {
  version: number;
  requestEnvelope: JsonSchema;
  responseEnvelope: JsonSchema;
  actions: { local: string[]; forwarded: string[] };
};

function readContract(): ProtocolContract {
  const protocolPath = path.resolve(__dirname, "../../../config/protocol/host-protocol.v1.json");
  const raw = fs.readFileSync(protocolPath, "utf-8");
  return JSON.parse(raw) as ProtocolContract;
}

test("host protocol contract includes expected envelope invariants", () => {
  const contract = readContract();
  assert.equal(contract.version, 1);
  assert.equal(contract.requestEnvelope.type, "object");
  assert.deepEqual(contract.requestEnvelope.required, ["id", "action", "params"]);
  assert.equal(contract.requestEnvelope.properties?.id?.type, "string");
  assert.equal(contract.requestEnvelope.properties?.action?.type, "string");
  assert.equal(contract.requestEnvelope.properties?.params?.type, "object");
  assert.equal(contract.requestEnvelope.properties?.client?.type, "object");
  assert.deepEqual(contract.requestEnvelope.properties?.client?.required, ["component", "version"]);
  assert.equal(contract.requestEnvelope.properties?.client?.properties?.component?.type, "string");
  assert.equal(contract.requestEnvelope.properties?.client?.properties?.version?.type, "string");

  assert.equal(contract.responseEnvelope.type, "object");
  assert.deepEqual(contract.responseEnvelope.required, ["ok", "component", "version"]);
  assert.equal(contract.responseEnvelope.properties?.ok?.type, "boolean");
  assert.equal(contract.responseEnvelope.properties?.component?.type, "string");
  assert.equal(contract.responseEnvelope.properties?.version?.type, "string");
  assert.deepEqual(contract.responseEnvelope.properties?.requestId?.type, ["string", "null"]);
  assert.equal(contract.responseEnvelope.properties?.error?.type, "object");
  assert.deepEqual(contract.responseEnvelope.properties?.error?.required, ["message"]);
  assert.equal(contract.responseEnvelope.properties?.error?.properties?.message?.type, "string");
});

test("host protocol contract keeps local actions exact and disjoint", () => {
  const contract = readContract();
  assert.deepEqual([...contract.actions.local].sort(), ["history", "undo", "version"]);
  const local = new Set(contract.actions.local);
  assert.equal(local.size, contract.actions.local.length, "local actions must be unique");
  const forwarded = new Set(contract.actions.forwarded);
  const overlap = [...local].filter((action) => forwarded.has(action));
  assert.deepEqual(overlap, [], `local actions overlap forwarded actions: ${overlap.join(", ")}`);
});

test("host protocol contract keeps critical forwarded actions", () => {
  const contract = readContract();
  const forwarded = new Set(contract.actions.forwarded);
  for (const action of ["ping", "list", "analyze", "inspect", "open", "archive", "close", "report", "screenshot", "reload"]) {
    assert.ok(forwarded.has(action), `missing forwarded action: ${action}`);
  }
});
