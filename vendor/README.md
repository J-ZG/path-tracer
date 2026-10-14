# Temporary dependency patches

- **egui-winit 0.36.2**: In `src/lib.rs`, compile `dropped_file` and its
  `NativeFile` use/push only on native targets. `NativeFile` implements synchronous
  `bytes`, while egui's wasm `DroppedFile` trait requires `bytes_async`. Winit's
  web backend does not deliver filesystem drop events or browser `File` handles,
  so a path-only wrapper cannot implement browser file reading. This restores
  wasm compilation without pretending to support web drag-and-drop. Native drops
  and the application's browser file picker remain available. Remove the patch
  when an upstream release supports wasm compilation with egui-winit.
- **wgpu-profiler 0.28.0**: Change the Tracy dependency from 0.18 to 0.19 in
  `Cargo.toml` and `Cargo.toml.orig` (including its example/dev dependency).
  The public API used here is compatible. This keeps the application and GPU
  profiler on one Tracy client/runtime instead of linking two versions. Remove
  when a published profiler release supports Tracy 0.19. In `src/tracy.rs`,
  initialization uses a compute-pass timestamp when encoder timestamps are
  unavailable (Metal). This fixes the startup validation failure tracked in
  https://github.com/Wumpf/wgpu-profiler/issues/107. Retain that change until
  upstream initializes Tracy on devices without `TIMESTAMP_QUERY_INSIDE_ENCODERS`.

Upstreams: https://github.com/emilk/egui and https://github.com/Wumpf/wgpu-profiler.
