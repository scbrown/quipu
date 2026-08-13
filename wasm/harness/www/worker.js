// Dedicated worker: the opfs-sahpool VFS needs FileSystemSyncAccessHandle,
// which only exists here. Commands arrive as {id, cmd, ...args}; replies as
// {id, ok, value} or {id, ok: false, error}.
import init, {
  install_opfs,
  scenario_write,
  scenario_read,
  scenario_bench,
  scenario_export,
  scenario_import,
  scenario_pack,
  journal_mode,
} from "./pkg/quipu_wasm_harness.js";

const ready = init();

onmessage = async (e) => {
  const { id, cmd, path, n, read_model, bytes } = e.data;
  try {
    await ready;
    let value = null;
    if (cmd === "install_opfs") {
      await install_opfs();
    } else if (cmd === "write") {
      value = JSON.parse(scenario_write(path, n));
    } else if (cmd === "read") {
      value = JSON.parse(scenario_read(path));
    } else if (cmd === "bench") {
      value = JSON.parse(scenario_bench(path, n, read_model));
    } else if (cmd === "export") {
      value = Array.from(scenario_export(path));
    } else if (cmd === "import") {
      value = JSON.parse(scenario_import(new Uint8Array(bytes)));
    } else if (cmd === "pack") {
      value = Array.from(scenario_pack(path));
    } else if (cmd === "journal_mode") {
      value = JSON.parse(journal_mode(path));
    } else {
      throw new Error(`unknown cmd: ${cmd}`);
    }
    postMessage({ id, ok: true, value });
  } catch (err) {
    postMessage({ id, ok: false, error: String(err?.message ?? err) });
  }
};
