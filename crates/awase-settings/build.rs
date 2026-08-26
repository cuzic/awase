fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        awase_build_support::embed_awase_manifest("Awase.Settings");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
