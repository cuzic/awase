//! ME-3(`docs/tasks/mech-ecrs-2026-10-21/INDEX.md`): [`PhysicalKeyDisposition::plan_core`] の**決定表の全列挙 golden**。
//!
//! 撤去・統合の PR の前後で「物理 IME キーの配送判断(Allow/Suppress)が 1 つも変わらない」ことを、数秒の
//! Linux テストで判定するための道具(`crates/awase-windows/src/state/sync_actuation_exhaustive.rs`〈#808〉と同じ方式)。
//! 入力の組を列挙し、出力の全文を `tests/golden/physical_disposition_table.txt` に固定する。
//!
//! 列挙する軸(`plan_core` が読む入力のすべて):
//! - `vk_code` 19 通り: 判断が VK で分かれる IME 関連キーのすべて(KANA 0x15・IME_ON 0x16・JUNJA 0x17・KANJI 0x19・
//!   IME_OFF 0x1A・CONVERT 0x1C・NONCONVERT 0x1D・DBE 0xF0〜0xF6)、役割由来の F13〜F24 の両端(0x7C・0x87)と
//!   その外側の境界(0x7B・0x88)、IME と無関係な代表の文字キー(A 0x41)。
//! - `event_type` 2 × `was_down` 2 × `ime_relevance.shadow_action` 4(None・TurnOn・TurnOff・Toggle)。
//! - (`injected`, `shadow_toggled`) 3 通り: (F,F)・(F,T)・(T,F)。(T,T) は `plan_core` の `debug_assert!` が
//!   設計違反として落とす組(BUG-14 のガードが injected では必ず `shadow_toggled=false` にする)なので列挙しない。
//! - 1 行に `profile` 4 のセルを並べる(19 × 2 × 2 × 4 × 3 = 912 行、3,648 通り)。
//!   Suppress のセルには `suppress_reason` も書く(`S:imm-cross` など)。
//!
//! 列挙しない軸と理由:
//! - `kind`(`ImeKindId`): 現状の 2 値(Gji・MsIme)はどちらも `gji_direct_applicable || ms_ime_direct_applicable` が真
//!   (`ime_actuation_owned` が常に真)で、判断に効かない死んだ軸。golden は `Gji` だけで書き、全入力で 2 値の結果が同じ
//!   ことを `kind_axis_is_currently_irrelevant` が確かめる(効くようになったら失敗するので、そのときは軸を戻す)。
//! - `RawKeyEvent` の他のフィールド(`scan_code`・`extra_info`・`timestamp`・`key_classification`・`physical_pos`・
//!   `modifier_key`・`modifier_snapshot`・親指の押下時刻・`press_id`・`foreign_ctrl`・`impersonated`)と、`ime_relevance` の
//!   `shadow_action` 以外は、`plan_core`・`suppress_reason` のどちらも読まない。固定値にする。`RawKeyEvent`・`ImeRelevance` は
//!   `..default()` を使わず全フィールドを書くので、フィールドが増えるとコンパイルエラーになる(読み始めたら軸に足すか判断する)。
//! - VK は上の 19 個の表。表の外の VK が `plan_core` に特別扱いされ始めても、この golden は検出しない(表の外は代表の A と
//!   同じ扱いであることだけを固定する)。VK の場合分けを足す PR では、表にその VK を足すこと。
//!
//! enum に variant が増えると下の `match` がコンパイルエラーになる(列挙の取りこぼし防止)。
//!
//! 再生成: `UPDATE_GOLDEN=1 cargo test -p awase-ime-core --lib physical_disposition::exhaustive`
//! (ローカルで走らせない運用では、CI の失敗ログの `----- BEGIN physical_disposition_table -----` から
//! `----- END physical_disposition_table -----` までの間を golden に写す。)

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use awase::types::{
    ImeRelevance, KeyClassification, KeyEventType, ModifierState, RawKeyEvent, ScanCode,
    ShadowImeAction, VkCode,
};

use super::PhysicalKeyDisposition;
use crate::focus::class_names::AppImeProfile;
use crate::state::ime_kind::ImeKindId;

const VKS: [(u16, &str); 19] = [
    (0x41, "A"),
    (0x15, "KANA"),
    (0x16, "IME_ON"),
    (0x17, "JUNJA"),
    (0x19, "KANJI"),
    (0x1A, "IME_OFF"),
    (0x1C, "CONVERT"),
    (0x1D, "NONCONVERT"),
    (0x7B, "F12"),
    (0x7C, "F13"),
    (0x87, "F24"),
    (0x88, "0x88"),
    (0xF0, "DBE_ALPHANUMERIC"),
    (0xF1, "DBE_KATAKANA"),
    (0xF2, "DBE_HIRAGANA"),
    (0xF3, "DBE_SBCSCHAR"),
    (0xF4, "DBE_DBCSCHAR"),
    (0xF5, "DBE_ROMAN"),
    (0xF6, "DBE_NOROMAN"),
];

const PROFILES: [AppImeProfile; 4] = [
    AppImeProfile::Standard,
    AppImeProfile::Imm32Unavailable,
    AppImeProfile::TsfNative,
    AppImeProfile::InputRelay,
];

const EVENT_TYPES: [KeyEventType; 2] = [KeyEventType::KeyDown, KeyEventType::KeyUp];

const ACTIONS: [Option<ShadowImeAction>; 4] = [
    None,
    Some(ShadowImeAction::TurnOn),
    Some(ShadowImeAction::TurnOff),
    Some(ShadowImeAction::Toggle),
];

/// (injected, shadow_toggled)。(true, true) は設計違反(`debug_assert!`)なので含めない。
const INJECTED_TOGGLED: [(bool, bool); 3] = [(false, false), (false, true), (true, false)];

/// variant が増えたらコンパイルエラーにして、上の列挙表の更新を強いる。
const fn covered_profile(p: AppImeProfile) {
    match p {
        AppImeProfile::Standard
        | AppImeProfile::Imm32Unavailable
        | AppImeProfile::TsfNative
        | AppImeProfile::InputRelay => {}
    }
}

const fn covered_event_type(t: KeyEventType) {
    match t {
        KeyEventType::KeyDown | KeyEventType::KeyUp => {}
    }
}

const fn covered_action(a: Option<ShadowImeAction>) {
    match a {
        None
        | Some(ShadowImeAction::TurnOn | ShadowImeAction::TurnOff | ShadowImeAction::Toggle) => {}
    }
}

const fn covered_disposition(d: PhysicalKeyDisposition) {
    match d {
        PhysicalKeyDisposition::Allow | PhysicalKeyDisposition::Suppress => {}
    }
}

const fn tf(b: bool) -> &'static str {
    if b {
        "T"
    } else {
        "F"
    }
}

fn event(
    vk: u16,
    event_type: KeyEventType,
    was_down: bool,
    shadow_action: Option<ShadowImeAction>,
    injected: bool,
) -> RawKeyEvent {
    RawKeyEvent {
        was_down,
        press_id: None,
        vk_code: VkCode(vk),
        scan_code: ScanCode(0),
        event_type,
        extra_info: 0,
        timestamp: 0,
        key_classification: KeyClassification::Passthrough,
        physical_pos: None,
        ime_relevance: ImeRelevance {
            may_change_ime: false,
            shadow_action,
            is_sync_key: false,
            sync_direction: None,
            is_ime_control: false,
            is_ime_mode_key: false,
            layout_japanese: None,
        },
        modifier_key: None,
        modifier_snapshot: ModifierState::default(),
        left_thumb_down_snapshot: None,
        right_thumb_down_snapshot: None,
        injected,
        foreign_ctrl: false,
        impersonated: false,
    }
}

/// golden に書く `kind`(死んだ軸なので 1 値。上の doc)。
const KIND: ImeKindId = ImeKindId::Gji;

const fn covered_kind(k: ImeKindId) {
    match k {
        ImeKindId::Gji | ImeKindId::MsIme => {}
    }
}

fn fmt_cell(ev: &RawKeyEvent, profile: AppImeProfile, toggled: bool) -> String {
    let d = PhysicalKeyDisposition::plan_core(ev, profile, toggled, KIND);
    covered_disposition(d);
    match d.suppress_reason(ev, profile) {
        Some(reason) => format!("S:{reason}"),
        None if d == PhysicalKeyDisposition::Suppress => "S:?".to_string(),
        None => "A".to_string(),
    }
}

fn render() -> (String, usize) {
    let mut out = String::new();
    out.push_str(
        "# PhysicalKeyDisposition::plan_core の決定表(全列挙) golden (ME-3)\n\
         #\n\
         # 生成元: crates/awase-ime-core/src/state/physical_disposition_exhaustive_tests.rs\n\
         # このファイルは自動生成される。更新は UPDATE_GOLDEN=1 で再生成するか、CI の失敗ログの BEGIN/END の間を写すこと。\n\
         # 撤去・統合の PR でこのファイルが変わらなければ、plan_core の配送判断は列挙した全入力で不変。\n\
         # セル: A = Allow、S:<理由> = Suppress(理由は suppress_reason)。\n\
         # kind は Gji だけ(現状 2 値とも ime_actuation_owned が真で効かない。kind_axis_is_currently_irrelevant が確かめる)。\n\
         #\n",
    );
    let mut header = String::from("# [vk type was_down action injected toggled]");
    for profile in PROFILES {
        covered_profile(profile);
        let _ = write!(header, " {profile:?}");
    }
    out.push_str(&header);
    out.push_str("\n\n");

    let mut count = 0usize;
    for (vk, name) in VKS {
        for event_type in EVENT_TYPES {
            covered_event_type(event_type);
            for was_down in [false, true] {
                for action in ACTIONS {
                    covered_action(action);
                    for (injected, toggled) in INJECTED_TOGGLED {
                        let ev = event(vk, event_type, was_down, action, injected);
                        let mut cells = Vec::new();
                        for profile in PROFILES {
                            count += 1;
                            cells.push(fmt_cell(&ev, profile, toggled));
                        }
                        let action =
                            action.map_or_else(|| "None".to_string(), |a| format!("{a:?}"));
                        let _ = writeln!(
                            out,
                            "[0x{vk:02X} {name} {event_type:?} was_down={} action={action} injected={} toggled={}] {}",
                            tf(was_down),
                            tf(injected),
                            tf(toggled),
                            cells.join(" ")
                        );
                    }
                }
            }
        }
    }
    let _ = writeln!(out, "\n# {count} 通り");
    (out, count)
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("physical_disposition_table.txt")
}

#[test]
fn input_space_size_is_as_documented() {
    let (text, count) = render();
    assert_eq!(count, 19 * 2 * 2 * 4 * 3 * 4);
    assert_eq!(count, 3_648);
    assert!(
        !text.contains("S:?"),
        "Suppress なのに suppress_reason が None"
    );
}

/// `kind` は現状の全 variant で結果が同じ(golden から外した根拠)。効くようになったら golden に軸を戻す。
#[test]
fn kind_axis_is_currently_irrelevant() {
    let mut differ = Vec::new();
    for (vk, name) in VKS {
        for event_type in EVENT_TYPES {
            for was_down in [false, true] {
                for action in ACTIONS {
                    for (injected, toggled) in INJECTED_TOGGLED {
                        let ev = event(vk, event_type, was_down, action, injected);
                        for profile in PROFILES {
                            let base =
                                PhysicalKeyDisposition::plan_core(&ev, profile, toggled, KIND);
                            for kind in ImeKindId::ALL {
                                covered_kind(kind);
                                if PhysicalKeyDisposition::plan_core(&ev, profile, toggled, kind)
                                    != base
                                {
                                    differ.push(format!(
                                        "{name} {event_type:?} was_down={was_down} {action:?} injected={injected} \
                                         toggled={toggled} {profile:?} {kind:?}"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        differ.is_empty(),
        "kind で結果が変わる入力がある。golden に kind の軸を戻すこと: {differ:?}"
    );
}

#[test]
fn physical_disposition_table_matches_golden() {
    let (actual, _) = render();
    let path = golden_path();
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().expect("golden の親"))
            .expect("golden の親を作れない");
        std::fs::write(&path, &actual).expect("golden を書けない");
        eprintln!("golden を更新しました: {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if actual == expected {
        return;
    }
    let first_diff = actual
        .lines()
        .zip(expected.lines())
        .enumerate()
        .find(|(_, (a, e))| a != e)
        .map_or_else(
            || {
                format!(
                    "行数が違う(現状 {} 行 / golden {} 行)",
                    actual.lines().count(),
                    expected.lines().count()
                )
            },
            |(n, (a, e))| format!("{} 行目\n  現状:   {a}\n  golden: {e}", n + 1),
        );
    eprintln!("----- BEGIN physical_disposition_table -----");
    eprint!("{actual}");
    eprintln!("----- END physical_disposition_table -----");
    panic!(
        "plan_core の決定表が golden({}) と一致しません。最初の差: {first_diff}\n\
         撤去・統合で決定を変えないはずの PR なら、変えた箇所を見直すこと。意図した変更(tuning の定数の変更を含む)なら\n\
         `UPDATE_GOLDEN=1 cargo test -p awase-ime-core --lib physical_disposition::exhaustive` で再生成するか、\n\
         上の BEGIN/END の間(現状の全文)を golden に写して、差分を PR で説明すること。",
        path.display()
    );
}
