fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_awase_manifest("Awase.Awase");
    }
    println!("cargo:rerun-if-changed=build.rs");
}

/// Embeds a Windows application manifest (`asInvoker` execution level) so
/// Windows never treats this binary as a legacy installer needing elevation
/// (see docs/known-bugs.md, auto-start + UAC compatibility-flag reports).
///
/// `embed_manifest::embed_manifest()` picks its embedding strategy from the
/// `TARGET` env var: on an `-msvc` target it emits linker flags
/// (`/MANIFEST:EMBED` + `/MANIFESTINPUT:...`) that `lld-link` can only
/// resolve by shelling out to `mt.exe`, which cargo-xwin's cross-compilation
/// toolchain (used by the Linux `windows-cross-check` CI job) does not
/// provide. The `-gnu` code path instead builds a self-contained `.rsrc`
/// COFF object in pure Rust and links it directly — no external tool, and
/// the object carries no ABI-specific content, so it links cleanly into an
/// `-msvc` binary too. Spoofing `TARGET` for the duration of this call
/// forces that path on every target, keeping both the real (`windows-latest`)
/// build and the Linux cross-check build tool-independent.
// SAFETY: build scripts run single-threaded, and TARGET is restored before
// returning, so this can't race with another thread reading the env.
#[allow(unsafe_code)]
fn embed_awase_manifest(name: &str) {
    let real_target = std::env::var("TARGET").unwrap_or_default();
    let gnu_target = real_target
        .strip_suffix("-msvc")
        .map_or_else(|| real_target.clone(), |prefix| format!("{prefix}-gnu"));
    unsafe {
        std::env::set_var("TARGET", &gnu_target);
    }
    let result = embed_manifest::embed_manifest(embed_manifest::new_manifest(name));
    unsafe {
        std::env::set_var("TARGET", real_target);
    }
    result.expect("unable to embed manifest file");
}
