use std::fs;
use std::path::Path;

#[test]
fn real_driver_does_not_call_awase_actuation_pipeline() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in fs::read_dir(src).expect("srcを読める") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(&path).expect("ソースを読める");
            for forbidden in ["send_input_safe", "set_ime_open"] {
                assert!(
                    !text.contains(forbidden),
                    "{}からawase actuation関数{forbidden}を呼んではならない",
                    path.display()
                );
            }
        }
    }
}
