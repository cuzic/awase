# ADR-126: Caps(英数) を「追加の Ctrl」にするプリセット（Ctrl を2つにする）

## ステータス

**採用（2026-09-03、r5。Opus 2体4ラウンドの敵対的レビューで収束——r1: B1-B3、
r2: B4-B5・M8-M15、r3: R1-R5、r4: 決定4項目5追加のみで収束確認。未実装、
Windows実機ソーク未実施）。** ADR-111 で採用した Caps(英数)⇔Left Ctrl
双方向入れ替えプリセットに加えて、片方向（Caps(英数) → Left Ctrl のみ、元の
Left Ctrl キーは変更しない）のプリセットを追加する設計。

## 背景

ADR-111 は「Caps(英数)⇔Left Ctrl の**双方向入れ替え**」を Windows の
Scancode Map（`HKLM\SYSTEM\CurrentControlSet\Control\Keyboard Layout\Scancode
Map`）で実現するプリセットを実装した。ユーザーから、これとは別に「Caps(英数)
キーを Ctrl にしたいが、元の Ctrl キーも Ctrl のまま使いたい（＝ Ctrl キーが
2つある状態にしたい）」という要望が出た。これは入れ替え（swap）ではなく
片方向の複製（duplicate）であり、ADR-111 のプリセットでは実現できない
（ADR-111 のプリセットは Left Ctrl 側も Caps(英数) の役割に置き換わってしまう）。

ADR-111 は「hook ベースでこの特定のキー（JIS 英数キー / CapsLock）を扱うのは
構造的に危険」と結論している。理由は次の2点:

1. `docs/experiments.md` エントリ07/08/09で、`VK_DBE_ALPHANUMERIC` の
   `SendInput` 注入がこのリポジトリで3回失敗している（フックに届かない、
   または CapsLock を物理的に点灯させる）。
2. PowerToys KeyboardManager が同じ組み合わせ（CapsLock→Ctrl + 日本語 IME）で
   2020年から issue を抱え続けている（Shift+CapsLock 等が日本語 IME の
   グローバル入力方式切替ショートカットであり、IME 側がフックより前段で
   キー状態を読むため、フック側の抑制・リマップが効かない）。

本 ADR が提案する片方向プリセットも、ADR-111 と**同じ Scancode Map 機構**の
上に実装する。hook ベースの新しい仕組みは一切導入しない。ADR-111 決定1が
確立した「スキャンコード段階での置換なので、レイアウトドライバの Shift 依存
VK 分岐（英数/CapsLock の二重人格）より手前で別のキー（Left Ctrl）にすり
替わる」という安全性の根拠は、片方向プリセットにもそのまま成り立つ
（決定1参照）。

## スコープ

- 対象は「Caps(英数) → Left Ctrl」の片方向複製のみ。Right Ctrl は対象外
  （ADR-111 と同じ理由: 英数キーは物理的に1つしかない）。
- 実現方式は ADR-111 と同じ Scancode Map（レジストリ）のみ。hook ベースの
  代替は追加しない（ADR-111 の却下理由がそのまま当てはまる）。
- ADR-111 の双方向入れ替えプリセットと、本 ADR の片方向複製プリセットは
  **両方とも Caps(英数) 側のスキャンコード（0x3A）を書き換える**ため、
  同時に有効化することはできない（決定2参照）。GUI 上は3択の排他選択にする。
- **本プリセットを有効化すると、英数(Caps) キー自体は物理的なキーとしては
  消滅する**（決定6参照）。Swap（ADR-111）は Left Ctrl キー側が英数の役割を
  引き継ぐため英数キーの機能は温存されるが、片方向複製ではその引き継ぎが
  無いため、キーボード上のどのキーを押しても英数(0x3A) スキャンコードが
  出なくなる。
- 「権限昇格を許容できないユーザー」への代替は ADR-111 同様、本 ADR でも
  提供しない。

## 決定

### 決定1: Scancode Map バイト列 — 1エントリのみ

ADR-111 決定2の2エントリ版に対し、本プリセットは次の1エントリのみを書く。

```
00 00 00 00                 ; Header: Version (常に 0)
00 00 00 00                 ; Header: Flags (常に 0)
02 00 00 00                 ; エントリ数 + null終端分 = 2
1D 00 3A 00                 ; from=0x003A(英数/CapsLock位置) → to=0x001D(LCtrl)
00 00 00 00                 ; null終端エントリ
```

`0x001D → 0x003A`（Left Ctrl → 英数/CapsLock 方向）のエントリは**書かない**。
元の Left Ctrl キーのスキャンコードは awase から見て一切変更されないため、
物理的に「英数(Caps) キー」と「Ctrl キー」の**両方**が Left Ctrl として機能する
（ユーザーの言う「Ctrl が2つになる」状態）。

決定1（ADR-111）が確立した「スキャンコード置換はレイアウトドライバの Shift
依存 VK 分岐より手前で起きる」という事実は片方向でも変わらないため、
Shift+(旧英数キー、今は Ctrl) は素直に Shift+Ctrl のチョードとして解釈され、
Shift+CapsLock の日本語 IME グローバルショートカットとは衝突しない。この
「区別できない」の一次的な根拠は決定6で改めて明示する。

### 決定2: `ScancodeMapPreset` / `ScancodeMapSelection` によるプリセットの一般化

`crates/awase-windows/src/scancode_map.rs` の `preset_entries()` /
`is_preset_active()` / `merge_for_enable()` / `remove_preset()` は
「Caps⇔Ctrl 双方向入れ替え」1種類だけを前提にハードコードされている。これを
2つの型に分けて一般化する。

```rust
/// Scancode Map に書き込む具体的な内容を持つプリセット
/// （「無効」はこの型では表現しない — 下記 `ScancodeMapSelection` 参照）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScancodeMapPreset {
    /// ADR-111: Caps(英数)⇔Left Ctrl 双方向入れ替え
    Swap,
    /// 本ADR: Caps(英数)→Left Ctrl 片方向のみ（Ctrl が2つになる）
    CapsAsExtraCtrl,
}

impl ScancodeMapPreset {
    /// このプリセットが書き込むエントリ。
    #[must_use]
    pub const fn entries(self) -> &'static [(u16, u16)] {
        match self {
            Self::Swap => &[(SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL),
                             (SCANCODE_LEFT_CTRL, SCANCODE_CAPS_EISU)],
            Self::CapsAsExtraCtrl => &[(SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL)],
        }
    }

    /// このプリセットが有効なとき「自分が書いた」と主張できる scancode
    /// （`from` 側）の集合。Swap は 0x3A と 0x1D の両方を書くので両方を
    /// 主張するが、CapsAsExtraCtrl は 0x3A しか書かない（決定3のASYMMETRY参照）。
    const fn owned_from_codes(self) -> &'static [u16] {
        match self {
            Self::Swap => &[SCANCODE_CAPS_EISU, SCANCODE_LEFT_CTRL],
            Self::CapsAsExtraCtrl => &[SCANCODE_CAPS_EISU],
        }
    }
}

/// GUI のラジオボタン・CLI 引数が表す「ユーザーが選んだ状態」。`Off` を
/// 独立した variant として持つことで `Option<ScancodeMapPreset>` を使わず
/// 済ませる（`Option<Option<_>>` を要求する API を作らないため——実装時の
/// 注意点として、`from_cli_arg` はパース失敗を `Option<Self>` の `None` で
/// 表現でき、`Option<Option<Self>>` は不要になる）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScancodeMapSelection {
    Off,
    Swap,
    CapsAsExtraCtrl,
}

impl ScancodeMapSelection {
    #[must_use]
    pub const fn preset(self) -> Option<ScancodeMapPreset> {
        match self {
            Self::Off => None,
            Self::Swap => Some(ScancodeMapPreset::Swap),
            Self::CapsAsExtraCtrl => Some(ScancodeMapPreset::CapsAsExtraCtrl),
        }
    }
}

/// 現在のエントリ列から、有効なプリセットを検出する（排他判定）。
/// Swap は自分の2エントリの**包含判定**（`entries().iter().all(...)`）で
/// 検出し、CapsAsExtraCtrl も同型に `ScancodeMapPreset::CapsAsExtraCtrl
/// .entries()` の包含判定で検出する（マジック値のタプルを直接書かない —
/// r2 で `current_preset` だけが `entries.contains(&(SCANCODE_CAPS_EISU,
/// SCANCODE_LEFT_CTRL))` という手書きタプルを使っており、
/// `compute_new_entries` のコメントが謳う「マジック値の直接比較はしない」
/// と矛盾していた。両方とも `ScancodeMapPreset::entries()` を単一の SSOT
/// として使う）。Swap の2エントリは CapsAsExtraCtrl の1エントリを包含する
/// ため、判定順は Swap が先。
#[must_use]
pub fn current_preset(entries: &[(u16, u16)]) -> Option<ScancodeMapPreset> {
    if ScancodeMapPreset::Swap.entries().iter().all(|e| entries.contains(e)) {
        Some(ScancodeMapPreset::Swap)
    } else if ScancodeMapPreset::CapsAsExtraCtrl.entries().iter().all(|e| entries.contains(e)) {
        Some(ScancodeMapPreset::CapsAsExtraCtrl)
    } else {
        None
    }
}

/// 有効化・無効化・切替のすべてをこの1関数に統一する
/// （旧 `merge_for_enable`/`remove_preset` はこの関数へ統合・削除する）。
///
/// r1 では「Swap の逆方向エントリ (0x1D→0x3A) は自分たちしか書かない値
/// だから無条件に消してよい」という個別ルールだったが、これは
/// **既存の (0x1D→0x3A) が第三者ツール由来だった場合にそれを無警告で
/// 破壊する**（Opus r1 指摘 B1）。正しくは「現在検出されたプリセットが
/// 所有していた scancode」だけを掃除対象にする——マジック値の直接比較は
/// しない。
///
/// 戻り値の順序は決定的（既存の保持エントリを既存順のまま先に、
/// `target` のエントリを末尾に追加）。この順序保存は決定3が説明する
/// 読み戻し検証の前提条件である。
#[must_use]
pub fn compute_new_entries(
    existing: &[(u16, u16)],
    target: ScancodeMapSelection,
) -> Vec<(u16, u16)> {
    let current = current_preset(existing);
    let target_preset = target.preset();
    let mut clear: Vec<u16> = Vec::new();
    if let Some(p) = current {
        clear.extend_from_slice(p.owned_from_codes());
    }
    if let Some(p) = target_preset {
        clear.extend_from_slice(p.owned_from_codes());
    }
    let mut out: Vec<_> = existing
        .iter()
        .copied()
        .filter(|&(from, _)| !clear.contains(&from))
        .collect();
    if let Some(p) = target_preset {
        out.extend_from_slice(p.entries());
    }
    out
}
```

この設計により、各遷移が正しく振る舞う:

- **Off → Swap**: `current=None` なので clear は Swap の `{0x3A, 0x1D}` のみ。
  既存の無関係エントリ（`from` が 0x3A/0x1D 以外）は保持される
  （ADR-111 決定3と同じ）。
- **Off → CapsAsExtraCtrl**: clear は `{0x3A}` のみ。既存に第三者の
  `(0x1D→何か)` エントリがあっても**触らない**（B1で問題になったケースは
  ここで発生しない——そもそも消す理由がない）。
- **Swap → CapsAsExtraCtrl**: `current=Swap` の `{0x3A, 0x1D}` と
  `target=CapsAsExtraCtrl` の `{0x3A}` の和集合 `{0x3A, 0x1D}` を clear。
  Swap が書いた `(0x1D→0x3A)` は確実に消え、CapsAsExtraCtrl の1エントリのみ
  残る。
- **CapsAsExtraCtrl → Swap**: `current=CapsAsExtraCtrl` の `{0x3A}` と
  `target=Swap` の `{0x3A, 0x1D}` の和集合 `{0x3A, 0x1D}` を clear。
  もし CapsAsExtraCtrl 有効化中に第三者が `0x1D` に何か書き足していても、
  Swap は 0x1D 側を完全に所有する必要がある（ADR-111 決定3）ため、ここで
  上書きされるのは正しい。
- **CapsAsExtraCtrl → Off**: clear は `{0x3A}` のみ。第三者の `0x1D` エントリ
  は保持される。

`build_bytes()` / `parse_entries()`（バイナリフォーマットの生成・パース）は
プリセット非依存の汎用ロジックのままで変更不要。

### 決定3: 既存 Scancode Map 値との共存 — 0x3A/0x1D の非対称な所有、読み戻し検証の順序依存

ADR-111 決定3の方針（有効化時に自分が所有する scancode のみ上書き、無関係な
エントリは保持、書き込み後に読み戻し検証）を維持する。決定2の
`compute_new_entries` が両プリセット・全遷移で一貫してこの方針を守る。

**0x3A と 0x1D の所有は非対称であり、意図的である（Opus r2 指摘 M10）。**
`0x3A`（英数/CapsLock 位置）は、Swap・CapsAsExtraCtrl どちらのプリセットが
有効な場合も awase が排他的に所有する scancode として扱う——ユーザーが
どちらかのプリセットを選んだ時点で、既存の `from=0x3A` エントリ（awase 以外
のツールが書いたものである可能性を含む）は無条件に awase のものへ置き換わる。
一方 `0x1D`（Left Ctrl）は、**CapsAsExtraCtrl は所有しない**（決定2の
`owned_from_codes()`）。この非対称の理由は次の通り: CapsAsExtraCtrl は
「Left Ctrl キーは一切変更しない」ことがユーザーへの約束の核心であり
（スコープ節・決定6）、もし 0x1D も所有・上書きしてしまうと、第三者ツールが
Left Ctrl に設定していた別のリマップを黙って壊すことになり、この約束と
直接矛盾する。0x3A 側は逆に「英数(Caps) キーの役割を変える」こと自体が
両プリセットの目的そのものなので、既存の 0x3A エントリを保護する理由がない
（保護すると、そもそも有効化できなくなる）。将来この非対称の解消（0x3A も
保護する等）を検討する際は、この理由が今も成り立つか確認すること。

**読み戻し検証は順序に依存する（Opus r2 指摘 M13）。**
`compute_new_entries` の戻り値は決定的な順序（保持したエントリを既存順の
まま先に、`target` のエントリを末尾に追加）を返す。`awase-settings` 側の
読み戻し検証（決定4）はこの順序保存に依存した `Vec` の完全一致比較である
ため、`compute_new_entries` の内部実装を変える際（例: 重複除去のために
`HashSet`/`BTreeMap` を経由する等）は、検証ロジック側の順序非依存化
（多重集合比較への変更）を同時に検討すること。順序だけが変わって内容は
正しいのに読み戻し検証が失敗すると、レジストリは正しく書き変わっている
のにユーザーには「処理に失敗しました。」とだけ表示され、原因不明のまま
再試行を繰り返させることになる。

### 決定4: 昇格フロー — 呼び出し元の改修一覧・CLI引数・削除の冪等化

`crates/awase-settings/src/scancode_map_admin.rs` の
`run_elevated_worker(enable: bool)` / `request_elevated_change(enable: bool)`
を、`ScancodeMapSelection` を引数に取る形に変更する
（`Option<ScancodeMapPreset>` ではなく決定2の3値 enum を使う——理由は
下記 CLI引数の項参照）。

**この変更で改修が必要な既存呼び出し元（Opus r1 指摘 B2、実装時に必ず反映
すること）:**

1. `crates/awase-settings/src/scancode_map_admin.rs:102`
   `sm::is_preset_active(&entries)` → `sm::current_preset(&entries)` に置換。
2. 同 `:104` `entries.len() - sm::preset_entries().len()` の余剰エントリ数
   計算は、決定2の `current_preset`/`ScancodeMapPreset::entries().len()`
   を使う形に置き換える。この計算自体は**レジストリ非依存の純粋関数として
   `crates/awase-windows/src/scancode_map.rs` 側に切り出す**（下記 M15
   対応の項参照）。実装時は `Option::map(..).unwrap_or(..)` ではなく
   `Option::map_or(..)` を使うこと（`clippy::map_unwrap_or` は pedantic
   収録で3クレートとも `deny`、下記コードスケッチ参照）。
3. `ScancodeMapStatus::Active { extra_entries }` を
   `Active { preset: ScancodeMapPreset, extra_entries: usize }` に変更する
   （決定5のGUIが「今どちらのプリセットが有効か」を必要とするため）。
4. `crates/awase-settings/src/main.rs:1711-1752`（`scancode_map_section`
   関数内、状態表示 `match` からメッセージ表示までの本体）を決定5の
   3択UIに全面置換する。
5. `crates/awase-settings/src/main.rs:1759-1778`
   （`apply_scancode_map_change(&mut self, enable: bool)`）のシグネチャを
   `apply_scancode_map_change(&mut self, selection: ScancodeMapSelection)`
   に変更し、`request_elevated_change(selection)` を渡す。成功時メッセージ
   （現行 `1764-1769` の `if enable {"有効にしました。…"} else {"無効に
   しました。…"}`）も3値に拡張し、「入れ替えを有効にしました」「Ctrl として
   追加する設定にしました」「無効にしました」のようにどのプリセットへ
   切り替わったかが分かる文言にする（Opus r4 指摘、決定5のスケッチが
   `self.apply_scancode_map_change(selection)` を呼ぶ前提と対応させる）。

**`read_status()` の計算部分を `awase-windows::scancode_map` の純粋関数へ
切り出す（Opus r2 指摘 M15）。** `scancode_map.rs` は冒頭の doc コメントで
「パース・生成・マージのロジックは純粋関数として全プラットフォームで
コンパイル・テストできる。レジストリ I/O のみ `#[cfg(windows)]`」という
方針を既に宣言している。現行 `read_status()` は `scancode_map_admin.rs` の
`#[cfg(windows)]` ブロック内にあり、CLAUDE.md が明示する罠——
`#[cfg(windows)]` ゲート配下の `#[cfg(test)]` は Linux のテストバイナリに
**存在すらしない**（`cargo test --list` にも出ず、エラーも skip メッセージ
も出ない）——にそのまま該当する。決定2 の `current_preset` を使い、
余剰エントリ数まで含めて計算する次の関数を `scancode_map.rs` に追加する:

```rust
/// エントリ列から検出したプリセットと、無関係な余剰エントリ数を返す。
/// レジストリ非依存の純粋関数（Linux でも `cargo test -p awase-windows
/// --lib` で検証できる）。
#[must_use]
pub fn detect_status(entries: &[(u16, u16)]) -> (Option<ScancodeMapPreset>, usize) {
    let preset = current_preset(entries);
    // `.map(..).unwrap_or(0)` ではなく `.map_or(0, ..)` を使う
    // （`clippy::map_unwrap_or` は pedantic 収録、3クレートとも deny）。
    let owned_len = preset.map_or(0, |p| p.entries().len());
    (preset, entries.len() - owned_len)
}
```

`scancode_map_admin.rs::read_status()` は「レジストリを読む →
`sm::detect_status()` に渡す → `ScancodeMapStatus` に詰め替える」だけの
薄い関数にする。`ScancodeMapStatus`（`Active`/`Inactive`/`ReadError`）型
自体は GUI 表示専用の語彙なので `awase-settings` 側に残す。

**CLI引数の SSOT 化・3値 enum を選ぶ理由（Opus r2 指摘 B4）**: r2 では
`Option<ScancodeMapPreset>` を目標型とし `from_cli_arg(&str) ->
Option<Option<Self>>` を提案したが、これは `clippy::option_option` に
該当する。`Cargo.toml:66`・`crates/awase-windows/Cargo.toml:86`・
`crates/awase-settings/Cargo.toml:72` の3クレートすべてが
`pedantic = { level = "deny", priority = -1 }` を宣言しており
`option_option` は pedantic 収録のため、`cargo clippy`（`mise run clippy`）
でビルドが落ちる。決定2 で導入した `ScancodeMapSelection { Off, Swap,
CapsAsExtraCtrl }` を使えば `from_cli_arg(&str) -> Option<Self>`（パース
失敗のみ `None`）で素直に表現でき、`Option<Option<_>>` が不要になる。

プリセット名文字列は送信側（`scancode_map_admin.rs` の
`request_elevated_change_windows`、現行 `"--scancode-map on"`/
`"--scancode-map off"` を組み立てる箇所）と受信側（`main.rs` の
`--scancode-map` パース箇所）の2箇所に文字列として重複して現れる。タイポで
「昇格したのに違うプリセットが書かれる」事故を防ぐため、
`ScancodeMapSelection` に次の2関数を実装し、両側から使う（他の`pub fn`と
同様に `#[must_use]` を付ける）:

```rust
impl ScancodeMapSelection {
    #[must_use]
    pub const fn as_cli_arg(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Swap => "swap",
            Self::CapsAsExtraCtrl => "caps-extra-ctrl",
        }
    }

    #[must_use]
    pub fn from_cli_arg(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "swap" => Some(Self::Swap),
            "caps-extra-ctrl" => Some(Self::CapsAsExtraCtrl),
            _ => None,
        }
    }
}
```

`--scancode-map` に値が渡されなかった
場合（`arg_value` が `None` を返す場合）は、非昇格の通常 GUI 起動に
フォールバックせず**必ず終了コード1で終了する**（BUG-79 対策で確立した
「`awase-settings.exe` の通常起動は常に非昇格」という前提を守るため。
値なし引数のまま昇格状態で GUI が起動してしまう経路を作らない）。

**削除の冪等化（Opus r1 指摘 M5）**: `target=ScancodeMapSelection::Off` かつ
`compute_new_entries` の結果が空のとき、`sm::delete()` を呼ぶ。値が
そもそも存在しない場合 `RegDeleteKeyValueW` は `ERROR_FILE_NOT_FOUND` を
返すが、これは「既に無効」という意味で失敗ではない。`scancode_map.rs::
registry::delete()` を、同ファイルの `registry::read()` が既に
`ERROR_FILE_NOT_FOUND` を `Ok(None)` として扱っているのと同じパターンで、
`Ok(())` を返すように変更する（Opus r2 指摘 M11: r2 では参照先を「決定1
参照コード」と誤記していたが、正しくは `registry::read()` 内の
`ERROR_FILE_NOT_FOUND → Ok(None)` 分岐）。3択UIでは「既に無効なのに無効を
選ぶ」操作が2ボタン時代より起こりやすくなるため（決定5のガードで多くは
防げるが、UAC取り消し後の再試行等で発生しうる）、この冪等化は必須とする。

`ShellExecuteExW`+`SEE_MASK_NOCLOSEPROCESS` による昇格・終了コード判定の
フロー自体（ADR-111 決定4）は変更しない。

### 決定5: GUI 変更 — 2ボタンから3択排他選択へ

`crates/awase-settings/src/main.rs::scancode_map_section` の
「有効にする」/「無効にする」ボタン2つを、3択のラジオボタンに置き換える。
`ScancodeMapSelection`（決定2）をそのまま `ui.radio_value(&mut selection,
ScancodeMapSelection::Swap, "…")` の値として使う。

```
○ 無効
○ Caps(英数) ⇔ Ctrl を入れ替える
○ Caps(英数) を Ctrl として追加する（Ctrl が2つになります。元の Ctrl
   キーはそのまま。英数キー自体は使えなくなります）
```

**選択状態は `SettingsApp` のフィールドとしては持たない（Opus r1 指摘
M4）。** `scancode_map_status`（`read_status()` の結果、決定4の
`Active { preset, .. }` を含む）から毎フレーム導出したフレームローカルな
変数を `radio_value` に渡し、フレームの最後で「導出値と変わっていたら
適用する」という形にする（Opus r2 指摘 R3、具体的な実装パターン）:

```rust
let derived: ScancodeMapSelection = /* self.scancode_map_status から導出 */;
let mut selection = derived; // フレームローカル。SettingsAppのフィールドにしない
let is_read_error = matches!(self.scancode_map_status,
    Some(scancode_map_admin::ScancodeMapStatus::ReadError(_)));
ui.add_enabled_ui(!is_read_error, |ui| {
    ui.radio_value(&mut selection, ScancodeMapSelection::Off, "無効");
    ui.radio_value(&mut selection, ScancodeMapSelection::Swap, "…");
    ui.radio_value(&mut selection, ScancodeMapSelection::CapsAsExtraCtrl, "…");
});
if selection != derived {
    self.apply_scancode_map_change(selection);
}
```

理由: UAC キャンセルや書き込み失敗（`ElevationOutcome::Cancelled`/`Failed`）
のとき、もしラジオがローカル状態（`SettingsApp` のフィールド）で「新しい
選択」のままだと、実際のレジストリは元のプリセットのままなのに GUI 上は
変更後に見える不整合が起きる。2ボタン時代はボタンが状態を持たなかったため
存在しなかった失敗モードであり、3択化にあたって新規に注意が必要。

この形にすると、**「既に選択中のプリセットを再選択したときは昇格フローを
起動しない」は追加のガードコードなしに自動的に成立する**（同じラジオを
クリックしても `selection == derived` のままなので `if` に入らない。旧
2ボタン実装の `is_active` ガード、`main.rs:1742`/`1745` の
「有効にする」/「無効にする」ボタンクリック時チェックに相当する動作を、
3値ラジオでは値比較そのものが代替する——別途 `is_active` 相当の分岐を
3値化して書き足す必要はない）。

**この「毎フレーム導出する」はキャッシュ済み `scancode_map_status` から
上記 `derived`/`selection` を計算するだけの話であり、レジストリの読み取り
頻度そのものは ADR-111 決定7のまま変更しない**（タブを開いた時と操作直後
にのみ `RegGetValueW` を呼ぶ。Opus r2 指摘 M8: r2 の文言は「毎フレーム
read_status() から導出」と書いており、`read_status()`＝レジストリ読み取り
そのものだと誤読されかねなかったため、ここで明確に分離する）。操作直後は
既存コード（`main.rs:1776-1777`、`apply_scancode_map_change` 末尾の
`self.scancode_map_status = Some(scancode_map_admin::read_status())`）と
同様に必ず `read_status()` で読み直す。

**`ScancodeMapStatus::ReadError` のときはラジオ全体を無効化する（Opus r2
指摘 B5）。** 既存値が読めていない状態は「今どのプリセットが有効か」も
「他に何が書かれているか」も分からない状態であり、この状態のまま
昇格・書き込みを許すと ADR-111 決定3 が忌避した「無条件上書き」と同じ
リスクを負う。上記コードスケッチの `ui.add_enabled_ui(!is_read_error,
...)` がこれを実装する（選択の描画は `Off` 相当の見た目で構わないが、
クリックでは何も起きない）。下記の状態表示自体（読み取りエラーメッセージ）
は無効化とは独立に常に表示する。

現在の状態表示（未設定/有効/awase 以外の設定と混在/読み取りエラー）・
「反映には再起動が必要」・「全ユーザーに影響、RDP では動作しない」の注記
（ADR-111 決定5・6）はどちらのプリセットにも共通するため、文言のみ
「入れ替えプリセット」→「選択中のプリセット」に一般化して流用する。

選択を変更したときは決定2の `compute_new_entries` を直接プリセット単位で
呼ぶ1回の昇格操作で切り替える（Swap→CapsAsExtraCtrl 等の切替時に UAC を
2回出さない）。

### 決定6: NICOLA エンジン・`[[keymap]]` との相互作用、英数キー喪失について

CapsAsExtraCtrl モードでは、awase（および他のすべてのアプリケーション）から
見て「Caps(英数) キー由来の Left Ctrl」と「本来の Ctrl キー由来の Left Ctrl」は
スキャンコード・VK ともに区別できない。根拠は ADR-111 決定2が引く
`ntinput.c::MapScancode`（キーボードクラスドライバでの置換）であり、
`WH_KEYBOARD_LL` フックに届く `KBDLLHOOKSTRUCT.scanCode` には既にこの段階で
置換後の値（0x1D）が入っている。つまり awase の hook 層はもちろん、OS の
どのアプリケーション層のコードも「どちらの物理キーが押されたか」を知る手段
がない（hook 側で見分けて分岐する、という実装は原理的に不可能であり、
将来そのような実装を試みないための明示）。

awase の core エンジン（`awase` crate、ADR-019）はこの変更の対象外である
——プラットフォーム非依存境界の外側（`crates/awase-windows`）で完結する
レジストリ操作であり、`KeyClassification`/`ImeRelevance` 等の分類入力は
一切変わらない。Ctrl は親指シフトの同時打鍵判定対象キーではないため、
CapsAsExtraCtrl モード自体が core の判定ロジックに影響することもない。

ただし2つの実利用上の帰結を明記する:

- ユーザーが `[[keymap]]`（アプリスコープショートカット再割当、ADR-114）
  等で Ctrl 位置に何らかのチョードを割り当てている場合、Caps(英数) キー
  側からも同じチョードが発火する。これは「Ctrl を2つにする」という要望の
  意図通りの挙動。
- **CapsAsExtraCtrl 有効時、英数(Caps) キーそのものが物理的に使えなくなる**
  （スコープ節参照）。Swap（ADR-111）では Left Ctrl キー側が英数の役割を
  引き継ぐため IME 切替手段としての英数キーは温存されるが、片方向複製では
  この引き継ぎが無い。日本語 IME ユーザーにとって英数キーは主要な IME OFF
  手段であるため、GUI 文言（決定5）で明示し、代替の IME 切替手段
  （かな/カタカナキー、IME 側のトグルショートカット等）はユーザー自身の
  IME 設定に委ねる旨を注記する。

### 決定7: ADR-111 側の記述との整合（Opus r2 指摘 M12）

ADR-111 は「採用・実装済み」のまま残るが、その決定2・決定3が名指しする
`preset_entries()` / `is_preset_active()` / `merge_for_enable()` /
`remove_preset()` は本 ADR の決定2により `ScancodeMapPreset` /
`ScancodeMapSelection` / `current_preset()` / `compute_new_entries()` へ
全面的に置き換わり、旧関数は削除される。本 ADR の実装 PR で、ADR-111 の
ステータス欄に「Scancode Map のプリセット関数群は ADR-126 により
`ScancodeMapPreset`/`compute_new_entries()` へ一般化・改名された」旨を
一行追記する。`docs/adr/index.md` の ADR-111 行の要約は変更不要（機能自体の
説明は変わらないため）。あわせて、`docs/adr/index.md` の ADR-126 自身の行
（本 ADR 採用時点で「検討中」表記のままになっている）のステータス列を、
本 ADR のステータス欄の更新（下記参照）と同じ文言に同期させる
（Opus r2 指摘 M12・R5、ADR-111 決定9と同じ粒度）。

## 却下した代替案

- **hook ベースでの片方向複製実装**: ADR-111 が確立した「JIS 英数キー位置を
  hook で扱うのは構造的に危険」という結論は、双方向・片方向を問わず
  当てはまる。Scancode Map という既に安全性が確認された機構がある以上、
  新しいリスクを負う理由がない。
- **Swap と CapsAsExtraCtrl の同時有効化**: 両者とも 0x3A の書き込み先
  （to）が同じ 0x1D であるため技術的には矛盾しないが、Swap が追加で
  0x1D→0x3A を書くため、CapsAsExtraCtrl が「Left Ctrl キーは変更しない」と
  約束している前提が壊れる。GUI 上排他にすることで、この矛盾した状態が
  発生しないようにする。
- **Right Ctrl も対象にした「Ctrl を3つにする」等の拡張**: 要望の範囲外。
  英数キーは物理的に1つしかなく、ADR-111 のスコープ限定方針を踏襲する。
- **「Swap の逆方向エントリという特定の値を無条件に消す」個別ルール
  （r1 で検討し撤回）**: Opus r1 指摘 B1 の通り、第三者ツールが偶然同じ値
  `(0x1D→0x3A)` を書いていた場合にそれを無警告で破壊し、無効化しても
  復元されない。決定2の `compute_new_entries`（検出されたプリセットの
  所有 scancode のみを掃除する一般化）に置き換えた。
- **`Option<ScancodeMapPreset>` を有効化/無効化/切替の共通引数型にする
  （r2 で検討し撤回）**: `clippy::option_option`（3クレート共通で
  `pedantic = deny`）に抵触しコンパイル不能（Opus r2 指摘 B4）。
  `ScancodeMapSelection { Off, Swap, CapsAsExtraCtrl }` の専用3値 enum に
  置き換えた。
- **`ReadError` 時もラジオ操作を許可する（r2 で検討し撤回）**: 既存値が
  読めていない状態での無条件書き込みは ADR-111 決定3 が忌避した
  「無条件上書き」と同種のリスクを負う（Opus r2 指摘 B5）。ラジオ自体を
  無効化する方針にした。

## 未解決の疑問

1. **実機検証**: ADR-111 同様、JIS キーボード実機での動作確認
   （Caps(英数) キーと本来の Ctrl キーの両方が Left Ctrl として機能するか、
   Shift+(旧英数位置) が正常な Shift+Ctrl チョードとして機能するか、
   英数キー喪失後の IME 切替に実用上の支障がないか）は設計段階では未実施。
2. **単体テストで必須カバーすべきケース**（Linux 上で `cargo test -p
   awase-windows --lib` により実行可能。決定4の切り出しにより
   `scancode_map_admin.rs` 側の `#[cfg(windows)]` に隠れず全ケースが
   走ることを確認する、Opus r2 指摘 M15）:
   - `compute_new_entries` の5遷移（Off→Swap、Off→CapsAsExtraCtrl、
     Swap→CapsAsExtraCtrl、CapsAsExtraCtrl→Swap、CapsAsExtraCtrl→Off）
     それぞれが期待通りのエントリ列になること。
   - **第三者の `(0x1D→0x3A)` エントリが、CapsAsExtraCtrl の有効化・
     無効化を通じて保持されること**（B1 の回帰テスト）。
   - `current_preset` が Swap を CapsAsExtraCtrl より優先して検出すること、
     および空/無関係エントリのみのとき `None` を返すこと。
   - `detect_status` が CapsAsExtraCtrl 有効時に `extra_entries` を正しく
     数えること（Swap=2/CapsAsExtraCtrl=1 で減算数が異なる、決定4）。
   - `delete()` が `ERROR_FILE_NOT_FOUND` を成功として扱うこと。

## 関連ファイル

`crates/awase-windows/src/scancode_map.rs`、
`crates/awase-settings/src/scancode_map_admin.rs`、
`crates/awase-settings/src/main.rs`。
関連: ADR-111（Caps⇔Ctrl 双方向入れ替え、本 ADR が一般化する基盤。決定7参照）。
