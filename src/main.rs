#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    path_tracer::run()
}

// on the web, lib.rs starts the app with wasm_bindgen(start)
// this empty main function allows compiling this binary for wasm32
#[cfg(target_arch = "wasm32")]
const fn main() {}
