#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! `wix/main.wxs` の不変条件を固定する grep ベース回帰テスト。
//!
//! ADR-099（バージョンアップ時の設定消失を防ぐ）決定0は、MSI メジャー
//! アップグレード時にユーザーデータ（`config.toml`/`layout/nicola.yab`）が
//! 保持されることを、`<MajorUpgrade Schedule="afterInstallExecute">` と
//! 該当コンポーネントの `NeverOverwrite="yes"` + GUID 不変の組み合わせで
//! 実現している。この不変条件が壊れると、コンパイラは何も教えてくれない
//! （XML なのでビルドは通り、実害は次のメジャーアップグレード実行時にしか
//! 顕在化しない）ため、`architecture_guard.rs` に倣ってテキスト走査で
//! 固定する。壊れたら教えてくれるためのテストであり、意図的に GUID や
//! `Schedule` を変更する場合はこのファイルの期待値も合わせて更新すること。
//!
//! いずれのテストも `content.contains(...)` のような素朴なファイル全体探索は
//! 使わない。`wix/main.wxs` 自身に「この属性が無いと壊れる」という説明
//! コメントを書いており、そのコメント文字列が偶然チェック対象のリテラルと
//! 一致すると、実際のタグから属性を消してもテストが通ってしまう
//! （コードレビューで実際に発覚：`<MajorUpgrade>` 直前のコメントに
//! `Schedule="afterInstallExecute"` という文字列がそのまま含まれていた）。
//! 必ずタグの範囲を切り出してから判定する。

use std::fs;
use std::path::Path;

fn read_repo_file(rel_path_from_workspace_root: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    // crates/awase-windows から見てリポジトリルートは2階層上。
    let path = Path::new(manifest_dir)
        .join("../..")
        .join(rel_path_from_workspace_root);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn main_wxs() -> String {
    read_repo_file("wix/main.wxs")
}

/// `needle`（例: `"<MajorUpgrade"`）から始まるタグの範囲（開始 `<` から
/// 対応する `>` まで、自己終了 `/>` を含む）を切り出す。コメント本文に
/// 同名のリテラルが出現しても、タグ開始マーカーとして `<` を含めて検索
/// するため誤爆しない（`<!-- ... Component Id="X" ... -->` のような
/// コメント内テキストは `<Component` にはマッチしない）。
fn extract_tag<'a>(content: &'a str, needle: &str) -> &'a str {
    let start = content
        .find(needle)
        .unwrap_or_else(|| panic!("wix/main.wxs に {needle} タグが見つからない"));
    let end = content[start..]
        .find('>')
        .map(|i| start + i + 1)
        .unwrap_or(content.len());
    &content[start..end]
}

/// `start_needle`（例: `"<Condition Message="`）から `end_needle`（例:
/// `"</Condition>"`）までの範囲を切り出す。`<Property>`/`<Condition>` の
/// ように子要素や CDATA 本文を含む複数行要素を、コメント本文の偶然の
/// 文字列一致から区別して判定するために使う（extract_tag は単一タグの
/// 開始 `<` から最初の `>` までしか切り出せず、本文を含む要素には使えない）。
fn extract_element<'a>(content: &'a str, start_needle: &str, end_needle: &str) -> &'a str {
    let start = content
        .find(start_needle)
        .unwrap_or_else(|| panic!("wix/main.wxs に {start_needle} が見つからない"));
    let end = content[start..].find(end_needle).map_or_else(
        || panic!("wix/main.wxs に {start_needle} に対応する {end_needle} が見つからない"),
        |i| start + i + end_needle.len(),
    );
    &content[start..end]
}

#[test]
fn major_upgrade_schedule_is_after_install_execute() {
    let content = main_wxs();
    let tag = extract_tag(&content, "<MajorUpgrade");
    assert!(
        tag.contains(r#"Schedule="afterInstallExecute""#),
        "wix/main.wxs の <MajorUpgrade> タグ本体に \
         Schedule=\"afterInstallExecute\" が見つからない（タグ: {tag:?}）。\
         既定値 afterInstallValidate に戻ると、新バージョンの \
         ファイル配置より前に旧バージョンが完全アンインストールされ、\
         ConfigFile/NicolaYab の NeverOverwrite=\"yes\" が無力化される \
         （ADR-099 F1・決定0）。意図的な変更なら ADR-099 を更新すること。"
    );
}

#[test]
fn config_file_and_nicola_yab_components_have_never_overwrite() {
    let content = main_wxs();
    for component_id in [
        "ConfigFile",
        "NicolaYab",
        "NicolaKeytopYab",
        "NicolaUsYab",
        "NicolaFYab",
    ] {
        let tag = extract_tag(&content, &format!(r#"<Component Id="{component_id}""#));
        assert!(
            tag.contains(r#"NeverOverwrite="yes""#),
            "wix/main.wxs の Component Id=\"{component_id}\" タグ本体に \
             NeverOverwrite=\"yes\" が見つからない（タグ: {tag:?}、ADR-099 \
             決定0）。{component_id} はユーザーが編集しうるデータのため、\
             アップグレード時に上書きされてはならない。"
        );
    }
}

#[test]
fn known_component_guids_are_unchanged() {
    let content = main_wxs();
    // (Component Id, 既知の GUID)。変更する場合は意図的な更新として
    // このテストごと書き換えること（ADR-099 決定0・決定8参照）。
    let known_guids = [
        ("MainExe", "FEE4643D-BD1C-4FBA-A6F0-3422691909C5"),
        ("SettingsExe", "83280E86-7973-43A3-84E9-A4B51E47751B"),
        ("ConfigFile", "57E95F1F-4785-40B7-A4E7-16613080C938"),
        ("NicolaYab", "9690990E-0D11-425B-B60C-AF23D5E87226"),
        ("NicolaKeytopYab", "5B75B3B2-A53D-493E-BB62-81AE2B17D8ED"),
        ("NicolaUsYab", "48AA34CA-3723-4B7D-B624-6E0E9C29032C"),
        ("NicolaFYab", "523F3EB9-8E27-4312-A6D1-822D1BF7785F"),
        ("NgramData", "DCF4BA85-03F3-4EC7-BF17-D870682FFF5E"),
    ];
    for (component_id, expected_guid) in known_guids {
        // Id と Guid を別々に確認する（round1 レビュー指摘 P1: 属性の
        // 出現順や改行位置が変わっても、GUID そのものが変わっていない
        // 限りテストが誤って落ちないようにする）。
        let tag = extract_tag(&content, &format!(r#"<Component Id="{component_id}""#));
        assert!(
            tag.contains(&format!(r#"Guid="{expected_guid}""#)),
            "wix/main.wxs の Component Id=\"{component_id}\" の GUID が \
             既知の値 {expected_guid} と一致しない（タグ: {tag:?}）。GUID を \
             変更すると Windows Installer はそのコンポーネントを別物と \
             みなし、ADR-099 決定0 が意図するアップグレード時のユーザー \
             データ保護（新旧バージョン間でのコンポーネント共有）が構造的に \
             壊れる。意図的な変更なら ADR-099 を更新した上でこのテストの \
             期待値も更新すること。"
        );
    }
}

/// 2026-08-31追加: `layout/nicola_keytop.yab`（新規インストールの既定）が
/// MSI に同梱されていることを固定する。Opusレビューで、旧 `layout/nicola.yab`
/// の内容変更（記号キートップ通り出力版への切り替え）を試みた際に、この
/// コンポーネント追加を忘れると `dist\layout\nicola_keytop.yab` が
/// インストーラに含まれず、`default_layout` をそちらに変更しても
/// ファイルが存在せず無言でフォールバックする実害が指摘された
/// （`crates/awase-windows/src/app/bootstrap.rs` の
/// `select_default_layout_matches_by_file_name_not_internal_name_line` が
/// 名指しする既知の失敗モードと同型）。
#[test]
fn nicola_keytop_yab_component_is_bundled_with_never_overwrite() {
    // 2026-08-31 Opusレビュー3巡目で訂正: NeverOverwrite はコンポーネントの
    // KeyPath（レジストリ値）が既に存在する場合の上書きだけを抑止し、
    // KeyPath が無い真の新規インストールには影響しない。「付けると新規
    // インストールにも改善が届かなくなる」という当初の判断はMSIの
    // セマンティクスとして誤りだった。nicola_keytop.yab は新規インストール
    // の既定＝最も多くのユーザーが配列編集タブで実際に触るファイルであり、
    // NicolaYab/NicolaUsYab/NicolaFYab と同じ理由で保護が必要。
    let content = main_wxs();
    let tag = extract_tag(&content, r#"<Component Id="NicolaKeytopYab""#);
    assert!(
        tag.contains(r#"Guid="5B75B3B2-A53D-493E-BB62-81AE2B17D8ED""#),
        "wix/main.wxs の Component Id=\"NicolaKeytopYab\" の GUID が \
         既知の値と一致しない（タグ: {tag:?}）。意図的な変更ならこのテストの \
         期待値も更新すること。"
    );
    assert!(
        tag.contains(r#"NeverOverwrite="yes""#),
        "wix/main.wxs の Component Id=\"NicolaKeytopYab\" タグ本体に \
         NeverOverwrite=\"yes\" が見つからない（タグ: {tag:?}）。新規\
         インストールの既定配列であり、配列編集タブでその場編集される\
         ユーザーデータのため、アップグレード時に上書きされてはならない。"
    );
    assert!(
        content.contains(r#"<File Source="dist\layout\nicola_keytop.yab" />"#),
        "wix/main.wxs の LayoutFiles ComponentGroup に \
         dist\\layout\\nicola_keytop.yab の <File> が見つからない。"
    );
}

/// 2026-08-31追加: Opusレビュー再確認で判明した残件。README.md/README.en.md
/// は US 配列・富士通純正ハード向けにそれぞれ `nicola_us.yab`/`nicola_f.yab`
/// への `default_layout` 切り替えを案内しているが、MSI には元々どちらも
/// 同梱されていなかった。案内通りに変更すると存在しないファイルへ
/// `crates/awase-windows/src/runtime/mod.rs` の `resolve_index` が無言で
/// フォールバックする（`select_default_layout_matches_by_file_name_not_internal_name_line`
/// が名指しする既知の失敗モードと同型）。NicolaYab と同様、配列編集タブで
/// その場編集されうるデータのため NeverOverwrite を付けている
/// （`config_file_and_nicola_yab_components_have_never_overwrite` 参照）。
#[test]
fn nicola_us_f_kb232_yab_are_bundled_in_msi() {
    let content = main_wxs();
    for (component_id, file) in [
        ("NicolaUsYab", r"dist\layout\nicola_us.yab"),
        ("NicolaFYab", r"dist\layout\nicola_f.yab"),
        ("NicolaKb232Yab", r"dist\layout\nicola_kb232.yab"),
    ] {
        assert!(
            content.contains(&format!(r#"<Component Id="{component_id}""#)),
            "wix/main.wxs の LayoutFiles ComponentGroup に \
             Component Id=\"{component_id}\" が見つからない。"
        );
        assert!(
            content.contains(&format!(r#"<File Source="{file}" />"#)),
            "wix/main.wxs の LayoutFiles ComponentGroup に {file} の <File> \
             が見つからない。"
        );
    }
}

// vcruntime140.dll（VC++ 再頒布可能パッケージ）が無い環境で MSI
// インストール自体を中止させる LaunchCondition の回帰テスト。壊れると、
// インストールは成功するが初回起動時に OS の分かりにくいダイアログで
// 失敗するようになる（コンパイラはもちろん教えてくれず、xmllint 等の
// XML 妥当性チェックも通ってしまう）。
#[test]
fn vcruntime_launch_condition_present() {
    let content = main_wxs();

    let property = extract_element(
        &content,
        r#"<Property Id="VCRUNTIME140FOUND">"#,
        "</Property>",
    );
    assert!(
        property.contains(r#"Path="[System64Folder]""#),
        "wix/main.wxs の VCRUNTIME140FOUND の DirectorySearch が \
         System64Folder を参照していない（要素: {property:?}）。\
         System64Folder は呼び出し元プロセスのビット数に依存せず常に \
         ネイティブ 64-bit System32 を指す標準プロパティで、これを \
         使わないと（32bit 版 MSI ランタイムが System32 をそのまま \
         参照した場合の）WOW64 リダイレクトで誤検出しうる。"
    );
    assert!(
        property.contains(r#"Name="vcruntime140.dll""#),
        "wix/main.wxs の VCRUNTIME140FOUND の FileSearch が \
         vcruntime140.dll を探していない（要素: {property:?}）。"
    );

    let condition = extract_element(&content, "<Condition Message=", "</Condition>");
    assert!(
        condition.contains("Visual C++"),
        "wix/main.wxs の <Condition> に Visual C++ 再頒布可能パッケージが \
         必要である旨のメッセージが見つからない（要素: {condition:?}）。"
    );
    assert!(
        condition.contains("Installed OR VCRUNTIME140FOUND"),
        "wix/main.wxs の LaunchCondition が \
         \"Installed OR VCRUNTIME140FOUND\" を満たしていない \
         （要素: {condition:?}）。Installed を条件に含めないと、\
         修復・アップグレード・アンインストール時にもこのチェックが \
         働いてしまい、既にインストール済みの環境での操作を壊しうる。"
    );
}
