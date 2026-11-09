mod gpu;
#[cfg(not(target_arch = "wasm32"))]
pub(super) mod headless;
pub(super) mod interactive;
mod path_tracer;
mod ui;
pub mod world; // pub for criterion benchmarks
mod world_resources;

/// The path of the glTF 2.0 .glb file to use by default.
const DEFAULT_SCENE: &str = "assets/glb/box.glb";

/// The path of the skybox file to use by default.
const DEFAULT_SKYBOX: &str = "assets/skybox/kloppenheim_06_puresky_2k.hdr"; // https://polyhaven.com/a/kloppenheim_06_puresky
