//! Shared between `crates/awase-windows/build.rs` and
//! `crates/awase-settings/build.rs` so the Windows manifest embedding
//! workaround (BUG-79) lives in exactly one place.

/// Embeds a Windows application manifest (`asInvoker` execution level) so
/// Windows never treats this binary as a legacy installer needing elevation
/// (see `docs/known-bugs.md` BUG-79: auto-start + UAC compatibility-flag
/// reports).
///
/// `embed_manifest::embed_manifest()` picks its embedding strategy from the
/// `TARGET` env var: on an `-msvc` target it emits linker flags
/// (`/MANIFEST:EMBED` + `/MANIFESTINPUT:...`) that `lld-link` can only
/// resolve by shelling out to `mt.exe`, which cargo-xwin's cross-compilation
/// toolchain (used by the Linux `windows-cross-check` CI job) does not
/// provide. The `-gnu` code path instead builds a self-contained `.rsrc`
/// COFF object in pure Rust and links it directly — no external tool.
///
/// **This must only run on the `-gnu` path when actually cross-compiling
/// (no `mt.exe` available at all).** A real Windows host building for
/// `-msvc` natively (`windows-latest` CI, or a contributor's own machine)
/// always has `mt.exe` next to `link.exe`/`lld-link.exe` as part of the same
/// MSVC toolchain. Forcing the `-gnu` COFF object into an `-msvc` binary
/// unconditionally (previous version of this function) produced a manifest
/// resource that a real Windows host apparently didn't treat as fully
/// equivalent to a native `/MANIFEST:EMBED` one: `awase.exe` launching
/// `awase-settings.exe` via `std::process::Command::spawn` (→
/// `CreateProcessW`) failed with `ERROR_NOT_SUPPORTED` (os error 50) on a
/// binary built this way, even though the identical `.exe` still launched
/// fine via `ShellExecute`-based paths (Explorer double-click, PowerShell
/// `Start-Process`) — those two APIs apparently don't validate the manifest
/// resource as strictly as `CreateProcessW` does. See `docs/known-bugs.md`
/// BUG-79 addendum. Detect a native Windows host via `HOST` (Cargo always
/// sets it to the build machine's own triple) and use the real `-msvc`
/// path there instead.
// SAFETY: build scripts run single-threaded, and TARGET is restored before
// returning, so this can't race with another thread reading the env.
#[allow(unsafe_code)]
pub fn embed_awase_manifest(name: &str) {
    let host = std::env::var("HOST").unwrap_or_default();
    if host.contains("windows") {
        // Native Windows build: mt.exe is genuinely available, so let
        // embed_manifest use its real -msvc /MANIFEST:EMBED path.
        embed_manifest::embed_manifest(embed_manifest::new_manifest(name))
            .expect("unable to embed manifest file");
        return;
    }
    // Cross-compiling from a non-Windows host (no mt.exe anywhere on this
    // machine, e.g. the Linux-hosted windows-cross-check CI job): force the
    // tool-independent -gnu COFF-object path.
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

#[cfg(test)]
mod tests {
    // Pins the manifest content itself (not just "the link succeeded"), so a
    // future embed-manifest upgrade that silently stops requesting
    // asInvoker (re-exposing BUG-79) fails this test instead of only
    // showing up as a customer report.
    #[test]
    fn manifest_requests_as_invoker_execution_level() {
        let xml = embed_manifest::new_manifest("Test.App").to_string();
        assert!(
            xml.contains(r#"level="asInvoker""#),
            "manifest must request asInvoker so Windows doesn't flag this exe \
             for the Program Compatibility Assistant's elevation heuristics \
             (BUG-79); got:\n{xml}"
        );
    }
}
