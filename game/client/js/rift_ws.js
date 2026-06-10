// Appended to mq_js_bundle.js by the website image build: the bundle ships a quad_net plugin,
// but minification stripped the sapp_jsutils globals it depends on, so it throws on first use.
// The wasm-side counterpart is the extern block in src/platform.rs.
miniquad_add_plugin({
  name: "rift_ws",
  version: "1",
  register_plugin(importObject) {
    let socket;
    const inbox = [];
    const outbox = [];
    importObject.env.rift_ws_open = (pointer, length) => {
      socket = new WebSocket(UTF8ToString(pointer, length));
      socket.binaryType = "arraybuffer";
      socket.onopen = () => {
        for (const message of outbox) {
          socket.send(message);
        }
        outbox.length = 0;
      };
      socket.onmessage = (event) => inbox.push(new Uint8Array(event.data));
    };
    importObject.env.rift_ws_state = () => {
      if (!socket || socket.readyState === WebSocket.CONNECTING) {
        return 0;
      }
      return socket.readyState === WebSocket.OPEN ? 1 : 2;
    };
    importObject.env.rift_ws_send = (pointer, length) => {
      // A view into wasm memory is only valid until the module grows it — copy out.
      const data = new Uint8Array(
        new Uint8Array(wasm_memory.buffer, pointer, length),
      );
      if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(data);
      } else {
        outbox.push(data);
      }
    };
    importObject.env.rift_ws_next = () =>
      inbox.length > 0 ? inbox[0].length : -1;
    importObject.env.rift_ws_read = (pointer) => {
      const message = inbox.shift();
      if (message) {
        new Uint8Array(wasm_memory.buffer, pointer, message.length).set(
          message,
        );
      }
    };
  },
});
