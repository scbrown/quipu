// The wasm store lives HERE, not on the main thread.
//
// Loading the repository pack — verify the manifest against the payload bytes,
// adopt the bundled shapes, import, promote — takes several seconds of solid
// CPU on a 61k-triple artifact. On the main thread that is a frozen tab with no
// way to draw a progress line, so the page could not even tell you what it was
// busy doing. Queries are fast (tens of ms) but ride the same channel so there
// is one place the store is touched from.
//
// Commands arrive as {id, cmd, ...args}; replies as {id, ok, value} or
// {id, ok: false, error}.
import init, { Explorer, explorerVersion } from "./pkg/quipu_wasm_explorer.js";

const ready = init();
let explorer = null;

onmessage = async (e) => {
  const { id, cmd, bytes, source, sparql } = e.data;
  try {
    await ready;
    let value = null;
    if (cmd === "version") {
      value = explorerVersion();
    } else if (cmd === "load") {
      // `new Date().toISOString()` is the receiver's clock, and that is the
      // right one: the import timestamp records when THIS reader took the pack
      // in, not when the producer built it. The producer's time is in the
      // manifest, and the page shows both.
      explorer = Explorer.loadQpack(
        new Uint8Array(bytes),
        source,
        new Date().toISOString(),
      );
      value = JSON.parse(explorer.loadReport());
    } else if (cmd === "query") {
      if (!explorer) throw new Error("no pack loaded");
      value = JSON.parse(explorer.query(sparql));
    } else if (cmd === "stats") {
      if (!explorer) throw new Error("no pack loaded");
      value = JSON.parse(explorer.stats());
    } else {
      throw new Error(`unknown cmd: ${cmd}`);
    }
    postMessage({ id, ok: true, value });
  } catch (err) {
    postMessage({ id, ok: false, error: String(err?.message ?? err) });
  }
};
