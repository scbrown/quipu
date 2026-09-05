import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, dirname, join } from "node:path";
import { spawnSync } from "node:child_process";

const root = resolve(import.meta.dirname, "..");
function fixture(t) {
  const dir = mkdtempSync(join(tmpdir(), "contributor-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const registry = JSON.parse(readFileSync(join(root, "docs/knowledge/contributor-stories.json")));
  const write = (path, text) => {
    mkdirSync(dirname(join(dir, path)), { recursive: true }); writeFileSync(join(dir, path), text);
  };
  write("queries/repository-share-quipu.rq", readFileSync(join(root, "queries/repository-share-quipu.rq")));
  for (const story of [registry.vision, ...registry.stories]) {
    write(story.source, readFileSync(join(root, story.source)));
    for (const code of story.code ?? []) write(code, "// code witness\n");
  }
  write("docs/book/src/SUMMARY.md", "# Concepts\n- [Example](concepts/example.md)\n");
  write("docs/book/src/concepts/example.md", "# Example\n");
  const run = () => {
    write("docs/knowledge/contributor-stories.json", JSON.stringify(registry));
    return spawnSync(process.execPath, [join(root, "scripts/build-contributor-knowledge.mjs"), dir, join(dir, "out.ttl")], { encoding: "utf8" });
  };
  return { dir, registry, run, write };
}
test("deterministic source-backed graph includes book order and real code edges", (t) => {
  const { dir, run } = fixture(t);
  assert.equal(run().status, 0);
  const first = readFileSync(join(dir, "out.ttl"), "utf8");
  assert.match(first, /knowledge\/vision> <https:\/\/quipu.dev\/knowledge\/guides>/);
  assert.match(first, /knowledge\/governs> .*src%2Fprovider%2Fmod.rs/);
  assert.match(first, /knowledge\/chapter\/concepts%2Fexample.md/);
  assert.equal(run().status, 0);
  assert.equal(readFileSync(join(dir, "out.ttl"), "utf8"), first);
});
test("a changed source passage fails instead of publishing stale evidence", (t) => {
  const { registry, run } = fixture(t);
  registry.stories[0].quote = "a passage that is not in the source";
  const result = run(); assert.notEqual(result.status, 0); assert.match(result.stderr, /source excerpt changed/);
});
test("a missing code witness fails instead of publishing a dangling claim", (t) => {
  const { registry, run } = fixture(t);
  registry.stories[0].code = ["src/missing.rs"];
  assert.match(run().stderr, /missing code witness/);
});
test("private literals fail before any output is published", (t) => {
  const { registry, run } = fixture(t);
  registry.vision.summary = "Connect to " + ["private", "svc"].join(".");
  assert.match(run().stderr, /private identifier/);
});
