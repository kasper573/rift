// iOS/WebKit resumes a suspended AudioContext only from a user-gesture handler. cpal (via kira) resumes
// it once at startup — before any gesture — and never exposes it, so iOS stays silent while desktop
// auto-resumes on first input. We wrap the constructor to resume every context on the first gesture; as
// a wasm-bindgen snippet it loads before the wasm captures the constructor. See bevy_kira_audio#83.

const contexts = [];
for (const name of ["AudioContext", "webkitAudioContext"]) {
  const original = self[name];
  if (!original) continue;
  self[name] = new Proxy(original, {
    construct(target, args) {
      const context = new target(...args);
      contexts.push(context);
      return context;
    },
  });
}
const resume = () => contexts.forEach((c) => c.state !== "running" && c.resume());
for (const event of ["pointerdown", "touchend", "mousedown", "keydown"]) {
  document.addEventListener(event, resume);
}

// The unlock above is a load-time side effect; this marker only gives the wasm-bindgen binding
// something to import and call, so the snippet is pulled into the bundle.
export function audio_unlock() {}
