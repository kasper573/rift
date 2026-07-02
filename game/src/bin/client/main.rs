mod assets;
mod boot;
mod platform;
mod testing;

// wasm-bindgen runs main during the page's `init()`; on native targets the browser imports panic.
fn main() {
    boot::run();
}
