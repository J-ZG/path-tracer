# Temporary dependency patches

- **wgpu-profiler 0.28.0**: Change the Tracy dependency from 0.18 to 0.19 in
  `Cargo.toml` and `Cargo.toml.orig` (including its example/dev dependency).
  The public API used here is compatible. This keeps the application and GPU
  profiler on one Tracy client/runtime instead of linking two versions. Remove
  when a published profiler release supports Tracy 0.19. In `src/tracy.rs`,
  initialization uses a compute-pass timestamp when encoder timestamps are
  unavailable (Metal). This fixes the startup validation failure tracked in
  https://github.com/Wumpf/wgpu-profiler/issues/107. Retain that change until
  upstream initializes Tracy on devices without `TIMESTAMP_QUERY_INSIDE_ENCODERS`.

Upstream: https://github.com/Wumpf/wgpu-profiler.
