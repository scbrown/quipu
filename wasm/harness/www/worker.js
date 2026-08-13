// Dedicated worker: the opfs-sahpool VFS needs FileSystemSyncAccessHandle,
// which only exists here. Commands arrive as {id, cmd, ...args}; replies as
// {id, ok, value} or {id, ok: false, error}.
import init, {
  install_opfs,
  scenario_write,
  scenario_read,
  journal_mode,
} from "./pkg/quipu_wasm_harness.js";

const ready = init();

onmessage = async (e) => {
  const { id, cmd, path, n } = e.data;
  try {
    await ready;
    let value = null;
    if (cmd === "install_opfs") {
      await install_opfs();
    } else if (cmd === "write") {
      value = JSON.parse(scenario_write(path, n));
    } else if (cmd === "read") {
      value = JSON.parse(scenario_read(path));
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
