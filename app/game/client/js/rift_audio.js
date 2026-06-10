// Appended to mq_js_bundle.js by the website image build: the browser's audio backend, giving the
// pitch + pan that macroquad's quad-snd lacks. The wasm-side counterpart is the extern block in
// src/audio.rs. Mirrors the plugin shape of js/rift_ws.js.
miniquad_add_plugin({
  name: "rift_audio",
  version: "1",
  register_plugin(importObject) {
    let ctx;
    const buffers = [];

    // The context starts suspended under the browser autoplay policy; resume it on the first
    // user gesture (the player clicks/keys to start anyway).
    function context() {
      if (!ctx) {
        ctx = new (window.AudioContext || window.webkitAudioContext)();
        const resume = () => {
          ctx.resume();
          document.removeEventListener("pointerdown", resume);
          document.removeEventListener("keydown", resume);
        };
        document.addEventListener("pointerdown", resume);
        document.addEventListener("keydown", resume);
      }
      return ctx;
    }

    importObject.env.rift_audio_load = (index, pointer, length) => {
      // A view into wasm memory is only valid until the module grows it — copy out.
      const bytes = new Uint8Array(
        new Uint8Array(wasm_memory.buffer, pointer, length),
      );
      // decodeAudioData is async; plays before it resolves are dropped (the buffer is absent).
      context().decodeAudioData(bytes.buffer, (decoded) => {
        buffers[index] = decoded;
      });
    };

    importObject.env.rift_audio_play = (index, volume, pitch, pan) => {
      const buffer = buffers[index];
      if (!buffer) {
        return;
      }
      const c = context();
      const source = c.createBufferSource();
      source.buffer = buffer;
      source.playbackRate.value = pitch;
      const panner = c.createStereoPanner();
      panner.pan.value = pan;
      const gain = c.createGain();
      gain.gain.value = volume;
      source.connect(panner).connect(gain).connect(c.destination);
      source.start(0);
    };
  },
});
