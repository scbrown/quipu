import { build } from "esbuild";
import { mkdir, copyFile, readFile } from "node:fs/promises";

await mkdir("dist", { recursive: true });
const ortCommit = (await readFile("node_modules/onnxruntime-web/__commit.txt", "utf8")).trim();
if (ortCommit !== "8d85527a010e294a26b274749f74294b2a32cec5") {
  throw new Error("ONNX runtime changed; review its source license before packaging");
}
await build({
  entryPoints: ["node_modules/@huggingface/transformers/dist/transformers.web.js"],
  outfile: "dist/transformers.mjs", bundle: true, platform: "browser",
  format: "esm", minify: true,
});
// Paths are explicitly self-hosted by BrowserSemanticSearch; neither CDN
// defaults nor a cross-origin isolation requirement are implicit here.
for (const variant of ["asyncify", "jsep", "jspi"]) {
  for (const extension of ["mjs", "wasm"]) {
    const name = `ort-wasm-simd-threaded.${variant}.${extension}`;
    await copyFile(`node_modules/onnxruntime-web/dist/${name}`, `dist/${name}`);
  }
}
await copyFile("browser-runtime.mjs", "dist/browser-runtime.mjs");
await copyFile("model-assets.json", "dist/model-assets.json");
for (const [source, target] of [
  ["node_modules/@huggingface/transformers/LICENSE", "dist/TRANSFORMERS-LICENSE"],
  ["ONNX-LICENSE", "dist/ONNX-LICENSE"],
  ["ONNX-NOTICES", "dist/ONNX-NOTICES"],
]) {
  await copyFile(source, target);
}
