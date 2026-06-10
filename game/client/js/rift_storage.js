// Appended to mq_js_bundle.js by the website image build: bridges the browser's localStorage to
// the wasm client so user settings survive a reload. The wasm-side counterpart is the extern
// block in src/user_settings.rs; the load/read split mirrors rift_ws's next/read so the Rust side
// can size its buffer before the copy.
miniquad_add_plugin({
  name: "rift_storage",
  version: "1",
  register_plugin(importObject) {
    const KEY = "rift.user_settings";
    let staged = null;
    importObject.env.rift_storage_load = () => {
      const value = localStorage.getItem(KEY);
      if (value === null) {
        staged = null;
        return -1;
      }
      staged = new TextEncoder().encode(value);
      return staged.length;
    };
    importObject.env.rift_storage_read = (pointer) => {
      if (staged) {
        new Uint8Array(wasm_memory.buffer, pointer, staged.length).set(staged);
        staged = null;
      }
    };
    importObject.env.rift_storage_save = (pointer, length) => {
      const value = UTF8ToString(pointer, length);
      try {
        localStorage.setItem(KEY, value);
      } catch (_) {
        // Private mode or an exhausted quota: persistence is best-effort, so swallow it.
      }
    };
  },
});
