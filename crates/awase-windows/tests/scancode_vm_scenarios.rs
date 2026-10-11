//! Scancode Map の VM 検証(ADR-257、`.github/workflows/scancode-vm-verify.yml`)のシナリオ表の golden。
//!
//! `tools/scancode-vm/scenarios.json`(手書きの入力と期待。ここだけが正)から、本番経路
//! (`scancode_apply::decide` → `scancode_map::build_bytes`)で書き込むバイト列を作り、`tools/scancode-vm/scenario-maps.json`
//! (生成物)と比べる。VM のワークフローは、後者を読むだけで `awase-windows` をビルドしない(ADR-257 決定10)。
//!
//! 再生成: `UPDATE_GOLDEN=1 cargo test -p awase-windows --test scancode_vm_scenarios`。再生成の対象は
//! `scenario-maps.json` だけで、`scenarios.json` の手書きの期待は書き換えない(トートロジー防止、決定4)。
//! `decide` が書かない(`Stop`)シナリオはエラーにする(`expected` に既存値をそのまま入れる、決定10)。

use std::collections::BTreeMap;
use std::path::PathBuf;

use awase_windows::scancode_apply::{decide, ApplyRequest, Decision};
use awase_windows::scancode_map::build_bytes;
use awase_windows::scancode_pairs::{Cycle, Pair};
use serde_json::Value;

fn tools_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/scancode-vm")
}

fn u16_of(v: &Value) -> u16 {
    u16::try_from(v.as_u64().expect("数値")).expect("u16")
}

fn pairs_of(v: &Value) -> Vec<(u16, u16)> {
    v.as_array()
        .expect("配列")
        .iter()
        .map(|p| (u16_of(&p[0]), u16_of(&p[1])))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// シナリオ1件が VM に書くエントリ列。
fn entries_of(scenario: &Value) -> Vec<(u16, u16)> {
    let name = scenario["name"].as_str().expect("name");
    match scenario["source"].as_str().expect("source") {
        // 許可リスト外のキー(英字)や、同じ from の重複など、本番の `decide` では作れない手書きのエントリ列。
        "raw" => pairs_of(&scenario["entries"]),
        "decide" => {
            let r = &scenario["request"];
            let existing = pairs_of(&r["existing"]);
            let request = ApplyRequest {
                pairs: pairs_of(&r["pairs"])
                    .into_iter()
                    .map(|(a, b)| Pair::new(a, b))
                    .collect(),
                cycles: r["cycles"]
                    .as_array()
                    .expect("cycles")
                    .iter()
                    .map(|c| {
                        let keys: Vec<u16> = c.as_array().expect("循環").iter().map(u16_of).collect();
                        Cycle::new(&keys).unwrap_or_else(|| panic!("{name}: 循環が作れない"))
                    })
                    .collect(),
                caps_extra_ctrl: r["caps_extra_ctrl"].as_bool().expect("caps_extra_ctrl"),
                expected: existing.clone(),
                allow_displace: r["allow_displace"].as_bool().expect("allow_displace"),
            };
            let existing_raw = build_bytes(&existing);
            match decide(existing_raw.as_deref(), &request) {
                Decision::Write { entries } => entries,
                Decision::Stop(exit) => panic!("{name}: decide が書かない({exit:?})"),
            }
        }
        other => panic!("{name}: 未知の source {other}"),
    }
}

#[test]
fn scenario_maps_match_the_production_path() {
    let dir = tools_dir();
    let scenarios: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("scenarios.json")).expect("scenarios.json"))
            .expect("JSON");
    let mut generated: BTreeMap<String, String> = BTreeMap::new();
    for s in scenarios["scenarios"].as_array().expect("scenarios") {
        let name = s["name"].as_str().expect("name").to_string();
        let bytes = build_bytes(&entries_of(s)).unwrap_or_else(|| panic!("{name}: 空のエントリ列"));
        assert!(generated.insert(name.clone(), hex(&bytes)).is_none(), "{name}: シナリオ名が重複");
    }
    let golden_path = dir.join("scenario-maps.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        let mut text = serde_json::to_string_pretty(&generated).expect("JSON");
        text.push('\n');
        std::fs::write(&golden_path, text).expect("golden を書く");
        return;
    }
    let golden: BTreeMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string(&golden_path).expect("scenario-maps.json"))
            .expect("JSON");
    assert_eq!(
        generated, golden,
        "scenario-maps.json が本番経路の出力と違う。意図した変更なら UPDATE_GOLDEN=1 で再生成する"
    );
}

/// 手書きの期待・押すキーが、`control_keys` の名前だけを使っていること(typo で期待が黙って消えるのを防ぐ)。
#[test]
fn scenario_keys_refer_to_control_keys() {
    let scenarios: Value = serde_json::from_str(
        &std::fs::read_to_string(tools_dir().join("scenarios.json")).expect("scenarios.json"),
    )
    .expect("JSON");
    let names: Vec<&str> = scenarios["control_keys"]
        .as_array()
        .expect("control_keys")
        .iter()
        .map(|k| k["name"].as_str().expect("name"))
        .collect();
    for s in scenarios["scenarios"].as_array().expect("scenarios") {
        let scenario = s["name"].as_str().expect("name");
        let press = s["press"].as_array().expect("press");
        let expect = s["expect"].as_array().expect("expect");
        for key in press.iter().chain(expect.iter()) {
            let key = key.as_str().expect("キー名");
            assert!(names.contains(&key), "{scenario}: control_keys に無いキー {key}");
        }
        if s["kind"] == "verify" {
            assert_eq!(press.len(), expect.len(), "{scenario}: 押すキーと期待の数が違う");
        }
    }
}
