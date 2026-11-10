//! Compiles the Slang trace shader into WGSL with `slangc`.
//!
//! Uses `bin/slangc` in a `slang-*` directory at the repository root (an extracted Slang release) if there is one, and otherwise `slangc` from the `PATH`.

fn main() {
    // rerun whenever any file in src/render changes, which covers every Slang module the shader may import
    println!("cargo::rerun-if-changed=src/render");

    let Some(out_dir) = std::env::var_os("OUT_DIR") else {
        println!("cargo::error=OUT_DIR is not set");
        return;
    };
    let local_slangc = local_slangc();
    if let Some(local_slangc) = &local_slangc {
        println!("cargo::rerun-if-changed={}", local_slangc.display());
    }
    let slangc = local_slangc.unwrap_or_else(|| "slangc".into());
    let output = std::process::Command::new(&slangc)
        .arg("src/render/trace.slang")
        .args(["-target", "wgsl", "-entry", "compute_main"])
        .arg("-o")
        .arg(std::path::Path::new(&out_dir).join("trace.wgsl"))
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            println!(
                "cargo::error=failed to run slangc, which must be in slang-*/bin/ or on the PATH: {error}"
            );
            return;
        }
    };

    let directive = if output.status.success() {
        "warning"
    } else {
        println!("cargo::error=slangc failed to compile src/render/trace.slang");
        "error"
    };
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        println!("cargo::{directive}={line}");
    }
}

/// Returns `bin/slangc` in the `slang-*` directory at the repository root with the last name in lexicographic order that contains it, if any.
fn local_slangc() -> Option<std::path::PathBuf> {
    let mut directories: Vec<_> = std::fs::read_dir(".")
        .ok()?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("slang-"))
        })
        .collect();
    directories.sort();
    directories
        .into_iter()
        .rev()
        .map(|directory| directory.join(format!("bin/slangc{}", std::env::consts::EXE_SUFFIX)))
        .find(|slangc| slangc.is_file())
}
