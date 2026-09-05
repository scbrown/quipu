// The wasm store lives HERE, not on the main thread.
//
// Loading the repository pack — verify the manifest against the payload bytes,
// adopt the bundled shapes, import, promote — takes several seconds of solid
// CPU on a 61k-triple artifact. On the main thread that is a frozen tab with no
// way to draw a progress line, so the page could not even tell you what it was
// busy doing. Queries and edits are fast (tens of ms) but ride the same channel
// so there is one place the store is touched from.
//
// Commands arrive as {id, cmd, ...args}; replies as {id, ok, result} or
// {id, ok: false, error}.
import init, { Explorer, explorerVersion } from "./pkg/quipu_wasm_explorer.js";

const ready = init();
let explorer = null;

const need = () => {
  if (!explorer) throw new Error("no pack loaded");
  return explorer;
};

onmessage = async (e) => {
  const { id, cmd, bytes, source, sparql, entity, predicate, value, episode } = e.data;
  try {
    await ready;
    switch (cmd) {
      case "version":
        return postMessage({ id, ok: true, result: JSON.parse(explorerVersion()) });

      case "load": {
        // `new Date().toISOString()` is the receiver's clock, and that is the
        // right one: the import timestamp records when THIS reader took the
        // pack in, not when the producer built it. The producer's time is in
        // the manifest, and the page shows both.
        explorer = Explorer.loadQpack(
          new Uint8Array(bytes),
          source,
          new Date().toISOString(),
        );
        return postMessage({ id, ok: true, result: JSON.parse(explorer.loadReport()) });
      }

      case "query":
        return postMessage({ id, ok: true, result: JSON.parse(need().query(sparql)) });
      case "stats":
        return postMessage({ id, ok: true, result: JSON.parse(need().stats()) });

      case "set":
        return postMessage({ id, ok: true, result: JSON.parse(need().set(entity, predicate, value)) });
      case "retract":
        return postMessage({
          id, ok: true,
          result: JSON.parse(need().retract(entity, predicate ?? "", value ?? "")),
        });
      case "episode":
        return postMessage({ id, ok: true, result: JSON.parse(need().episode(episode)) });
      case "editLog":
        return postMessage({ id, ok: true, result: JSON.parse(need().editLog()) });

      // The delta from the pack this tab loaded to its current state, as the
      // `share-delta/v1` document the CLI already reads (aegis-8fdp8d). Not
      // computed in JS: the diff, the SPARQL validation and the delta_id are
      // quipu's, and a second implementation here would be a second producer of
      // one format.
      case "delta":
        return postMessage({ id, ok: true, result: JSON.parse(need().delta()) });

      case "exportManifest":
        return postMessage({ id, ok: true, result: JSON.parse(need().exportManifest()) });
      case "exportNtriples":
        return postMessage({ id, ok: true, result: need().exportNtriples() });
      case "exportPack": {
        // TRANSFERRED, not copied: re-exporting the repository pack is several
        // megabytes and structured-cloning it would hold two of them at once.
        const out = need().exportPack();
        return postMessage({ id, ok: true, result: out }, [out.buffer]);
      }

      default:
        throw new Error(`unknown cmd: ${cmd}`);
    }
  } catch (err) {
    postMessage({ id, ok: false, error: String(err?.message ?? err) });
  }
};
