import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

test("README test links reference existing tests", () => {
  // Collect all test names from source test files
  const srcTestDir = path.resolve(__dirname, "../../../src/tests/unit");
  const testFiles = fs
    .readdirSync(srcTestDir)
    .filter((f) => f.endsWith(".test.ts") && f !== "readme-links.test.ts");

  const allTestNames = new Set<string>();
  const testNamePattern = /^test\("([^"]+)"/gm;

  for (const file of testFiles) {
    const content = fs.readFileSync(path.join(srcTestDir, file), "utf-8");
    let match: RegExpExecArray | null;
    while ((match = testNamePattern.exec(content)) !== null) {
      allTestNames.add(match[1]);
    }
  }

  assert.ok(allTestNames.size > 0, "Should find at least one test name in test files");

  // Parse README for <!-- test: "..." --> comments
  const readmePath = path.resolve(__dirname, "../../../README.md");
  const readme = fs.readFileSync(readmePath, "utf-8");

  const commentPattern = /<!-- test: ((?:"[^"]+",?\s*)+) -->/g;
  const namePattern = /"([^"]+)"/g;

  const referencedNames: string[] = [];
  let commentMatch: RegExpExecArray | null;
  while ((commentMatch = commentPattern.exec(readme)) !== null) {
    const inner = commentMatch[1];
    let nameMatch: RegExpExecArray | null;
    while ((nameMatch = namePattern.exec(inner)) !== null) {
      referencedNames.push(nameMatch[1]);
    }
  }

  assert.ok(referencedNames.length > 0, "README should contain at least one <!-- test: --> comment");

  const missing = referencedNames.filter((name) => !allTestNames.has(name));
  assert.deepStrictEqual(
    missing,
    [],
    `README references tests that don't exist:\n${missing.map((n) => `  - "${n}"`).join("\n")}`,
  );
});
