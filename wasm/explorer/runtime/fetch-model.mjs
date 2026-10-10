import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile, rename } from "node:fs/promises";
import { dirname, join } from "node:path";

const manifest = JSON.parse(await readFile(new URL("model-assets.json", import.meta.url)));
const root = process.argv[2] ?? "dist/models";
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
for (const { path: name, sha256: expected } of manifest.files) {
  const target = join(root, manifest.directory, name);
  let existing;
  try { existing = await readFile(target); } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  if (existing && hash(existing) === expected) continue;
  const url = `https://huggingface.co/${manifest.model}/resolve/${manifest.revision}/${name}`;
  const response = await fetch(url, { signal: AbortSignal.timeout(60_000) });
  if (!response.ok) throw new Error(`${name}: HTTP ${response.status}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (hash(bytes) !== expected) throw new Error(`${name}: model checksum differs`);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(`${target}.tmp`, bytes);
  await rename(`${target}.tmp`, target);
  console.log(`verified ${name}: ${bytes.length} bytes`);
}
