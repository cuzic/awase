# ADR-109: `.yab` 句読点確定サフィックス（やまぶき `CV4D` 相当）の実現機構

## ステータス

**保留（本ADR単独での実装は行わない、2026-08-28）。** [GitHub Issue #118](https://github.com/cuzic/awase/issues/118)
（タスクトレイ「不具合を報告」経由、report 内部ID `01M13EACMQ7D2VETW75N0BTZ9C`）を受けて、
「句読点キーを押した瞬間に直前の変換候補を自動確定する」機能をどう実装するかを検討した。

検討の結果、本機能を `ConfirmThenSend` という**専用**の `YabValue`/`KeyAction` variant で
個別実装するのではなく、**将来実装予定の汎用「打鍵列機能」（1つの `.yab` セルに対して
複数のキーアクションの列を定義できるようにする機能、未着手）の一特殊ケース**として
位置づけ直すことにした。CV4D 相当の挙動は「句読点キー1つに対して『確定キー→句読点』
という2アクションの列を割り当てる」という形で、汎用打鍵列機能の上に自然に載る。
単独の variant として先取り実装すると、汎用機能を設計する際に型を二重化・再設計する
コストが生じるため、汎用機能側の設計に統合されるまで本ADRとしての実装は行わない。

本ファイルはその判断に至るまでの調査結果（`.yab`/エンジン側の現状構造、composing
観測の到達範囲、確定手段の技術的選択肢とその根拠、却下した代替案）を**汎用打鍵列機能を
設計する際の入力資料として**そのまま残す。決定1〜6・却下した代替案・未解決の疑問は、
汎用機能の設計時に読み替えて再利用することを想定している。実機検証は未実施。
**決定1（`.yab` 側の具体的なトークン表記）は Issue 報告者への確認が取れるまで暫定**
のままである（後述「未解決の疑問」1）。

## コンテキスト

### 要望

Issue #118 は、やまぶき（旧来の親指シフトエミュレータ）の配列定義にある `CV4D` 指定
（句読点キーの定義に付与すると、文字入力→変換→句読点入力の流れで句読点を打った瞬間に
直前の変換候補が自動確定される）相当の機能を awase の `.yab` にも実装してほしい、という
要望である。現状 `.yab` パーサ（`src/yab/mod.rs`）にはこの機能が無く、ユーザーが独自に
`'。CV4D'` のような値を書いても単なるクォート付きリテラル文字列として解釈される
（`YabValue::parse` の `strip_paired_quote` 分岐、mod.rs:103-105）。

### 実在の yamabuki コミュニティでの前例（Web 調査）

`CV4D` というトークン自体を裏付ける一次資料は見つからなかった（Web 検索では
ヒットせず）。ただし同種の機能を実現する別の手法は実例として確認できた:
配列定義ファイルとして「-確定」がファイル名に含まれるものを使うと、**句読点キーの
マッピング自体を「句読点を出力した後に Ctrl+M（Enter 相当）を出力する」という
複合定義にすることで実現している**という記述が見つかった。これは
`SpecialKey::Enter` を composition 確定に使うという本ADRの決定2の技術的な妥当性を
裏付ける傍証だが、**`CV4D` という語そのものの一次資料ではない**ため、決定1の
表記は暫定扱いのままとする。

### 現状のコード構造（関連する事実）

1. **`.yab` の1セルは単一の `YabValue` に対応する**。`YabFace`
   （`src/yab/mod.rs:19-34,48`）は `PhysicalPos` → `Option<YabValue>` の固定配列で、
   複合値（サフィックス付き）を表す既存のセル文法は無い（面レベルの `<x>` ブロックが
   唯一の複合構文だが無関係、mod.rs:435-450）。
2. `YabValue::parse`（mod.rs:84-112）は 空/`無` → `None`、キーワード表 → `Special`、
   `V`+16進数/`機`+数値 → `Vk`、クォート → `Literal`、全角ASCII → `classify_fullwidth`
   （`Romaji` または `KeySequence`）、それ以外 → `Literal` の順で判定する。句読点
   `、`/`。` は現状 unquoted で書けば `classify_fullwidth` の else 分岐を経て
   `YabValue::KeySequence("、")` になる。
3. `YabValue → KeyAction` の変換（`impl From<&YabValue> for KeyAction`、
   `src/engine/nicola_fsm.rs:42-58`）は**コンテキストを持たない純粋関数**であり、
   composing 状態にアクセスできない。
4. **composing 状態は既にコア（`awase`、OS非依存）まで到達している。**
   `InputContext.composing: bool`（`src/engine/decision.rs:279`）が
   `ComposingHint::{Trusted(bool), Unknown}`（`src/engine/fsm_types.rs:277-282`）を
   介して `NicolaFsm` に渡り、無変換/変換/Enter/Space 親指キーの単独タップガード
   （`GeneralConfig::*_ignore_composing_guard`、config.rs:216-318）が既にこれを
   使っている。プラットフォーム非依存トレイト `ImeDetector::is_composing(&self) -> bool`
   （`src/platform.rs:99`）も存在し、Windows 実装は
   `tsf/observer.rs::ime_composition_active_now()`（`EVENT_OBJECT_IME_SHOW`/`HIDE`
   由来の `AtomicBool`、ライブ計算値であり `ImeModel` のような蓄積 belief ではない
   ため [ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md)
   の Observe→classify_*→reduce() 規律の対象外）。
5. **ただし即時キー押下の経路には composing がまだ渡っていない。**
   `NicolaFsm::on_event(event, phys)`（nicola_fsm.rs:2124）は `composing` を
   受け取らない。渡っているのは `on_timeout(timer_id, phys, composing)`
   （nicola_fsm.rs:2143-2148、投機出力の差し替え判定用）だけである。一方
   `crates/awase-windows/src/runtime/key_pipeline.rs:106-114` の
   `build_input_context(..)` は毎キーイベントで `ctx.composing` を構築済みで、
   `self.engine.on_input(event, &ctx)`（key_pipeline.rs:190）に渡している——
   **`Engine::on_input` レベルでは composing は既に手元にある**。句読点は単発物理
   キーで同時打鍵の曖昧性が無いため通常 `on_event`（timeout 非経由）で解決される、
   という点が本ADRの主要な実装ギャップになる。
6. **確定に使う VK 送信の副作用面での前例**: `crates/awase-windows/src/runtime/mod.rs:1717`
   `cancel_ime_composition()` は `ImmNotifyIME(NI_COMPOSITIONSTR=0x15,
   CPS_CANCEL=0x04)` で未確定文字列をキャンセルする（Ctrl-bypass 機能用）。対の
   `CPS_COMPLETE=0x02` を使えば同じ関数形で「確定」もできるが、GJI に対して
   IMC read/write を成否判定に使わない（[[feedback... GJI は IMC が信用できない]]、
   ADR-107 追補）という repo の既定方針があり、また TSF-native アプリ
   （WezTerm 等）では `ImmGetContext` が正しい HIMC を返すために子 hwnd 解決の
   ワークアラウンドが必要（`cancel_ime_composition` 自体がそれを実装済み）。
   一方、**確定キーとして物理 Enter を送る**という手法（決定2）は、Web 調査で
   確認した実在のコミュニティ手法と一致し、IMM32/TSF のどちらの経路でも
   「composing 中に Enter は確定に使われる」という IME UX の一般的な契約に乗るため、
   IMC read/write より広いアプリ互換性が期待できる。
7. **確定キーの受動的認識は既に存在するが、能動送信の前例は無い**。
   `crates/awase-windows/src/vk.rs:314-317` `is_composition_confirm_key(vk)`
   （Space/Enter/Escape）は**ユーザー自身の**打鍵が確定/キャンセルを引き起こした
   ことを認識し `ColdReason::PassthroughConfirmKey`/`ReinjectConfirmKey`
   （output/mod.rs:1497-1501、tsf/composition_fsm.rs:190-244）の warm/cold
   ブックキーピングを駆動するために使われる。awase 自身が能動的に確定キーを注入する
   機能は今回が初めてになる。

### 制約

- [layer-boundaries](../layer-boundaries.md) A-1: コア `awase` クレートは OS 非依存を
  保つ。`.yab` パーサ・`YabValue`/`KeyAction` の拡張はコアに置いてよいが、
  composing の「鮮度が高い」観測（`is_composing()`）に基づく実際の確定要否判定は
  プラットフォーム層（`output/`）が担うべきという判断が決定3の根拠になる。
- composing はライブ計算値であり `ImeModel` の belief ではないため
  [ime-belief-architecture](../../.claude/rules/ime-belief-architecture.md) の
  3段防御（Observe→classify_*→reduce()）は本ADRには適用されない。ただし
  「awase 自身が能動的に確定キーを送る」という決定2は、IME を能動的に操作する点で
  [fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) の
  「キー選択（IME ON/OFF に送る VK）」ファミリーに準じた証拠義務を課す（決定6）。
- GJI に対して IMC read/write を成否判定に使わない（追補2の教訓、ADR-107 でも踏襲）。
  本ADRは確定手段として IMC write（`CPS_COMPLETE`）を主経路に採らない理由の一つ。

---

## 決定1（暫定）: `.yab` 側の表記は `CV4D` サフィックスをそのまま踏襲するが、報告者確認まで確定しない

Issue 本文の例（`'。CV4D'`、クォート付き）は**ユーザー自身の試行錯誤**であり、実在の
yamabuki 一次資料で裏付けが取れていない。本ADRは以下を暫定案として採用するが、
**実装着手前に Issue 報告者（または実在の yamabuki レイアウトファイル現物）で
正確な表記を確認すること**を決定0 相当の前提条件とする（未解決の疑問1）。

暫定案: セルの生テキスト（クォート判定より前）の**末尾に `CV4D` が直接連結されている
場合**（区切り文字なし、例 `。CV4D`、大文字固定）、これを剥がして残りを通常の
`YabValue::parse` に再帰させ、結果を `YabValue::ConfirmThenSend(Box<YabValue>)`
（決定2）で包む。クォート付き（`'。CV4D'` のような、ユーザーが実際に書いた形）は
**リテラル文字列 `。CV4D` として扱う現状維持**とし、特別扱いしない——クォートは
「ローマ字変換させない生文字列」を明示する既存の意味を持つため、その中の文字列を
魔法のキーワードとして横取りするのは既存文法の意味を壊す。

もし報告者確認の結果、実際の yamabuki 表記がこの暫定案と異なる場合（例: 区切り文字
あり、大文字小文字を問わない、末尾ではなく別行等）、本決定のみを差し替え、
決定2以降（`YabValue`/`KeyAction` の拡張・確定ロジック）はそのまま流用できる設計に
しておく（パーサ層とセマンティクス層を分離するのはこのため）。

---

## 決定2: `YabValue::ConfirmThenSend(Box<YabValue>)` / `KeyAction::ConfirmThenSend(Box<KeyAction>)` を新設する

`YabValue`（`src/yab/mod.rs`）と `KeyAction`（`src/types.rs:258-273`）の両方に
`ConfirmThenSend(Box<Self>)` を追加する。`impl From<&YabValue> for KeyAction`
（nicola_fsm.rs:42-58）に

```rust
YabValue::ConfirmThenSend(inner) => Self::ConfirmThenSend(Box::new(Self::from(inner.as_ref()))),
```

を足すだけで済み、**この変換は今日と同じくコンテキストフリーのままでよい**
（composing 判定は決定3で別の場所に置く。ここでは「意図」だけを運ぶ）。

`serialize()`（mod.rs:116-138）には
`Self::ConfirmThenSend(inner) => format!("{}CV4D", inner.serialize())`
を足す（決定1が確定した表記に合わせて更新）。

**確定の実体を Enter（`SpecialKey::Enter`）にする**理由: 「未解決の疑問」で確認した
実在の yamabuki コミュニティ手法（句読点+Ctrl+M の複合定義）と一致し、composing 中の
Enter が確定に使われるのは IMM32/TSF 双方に共通する IME UX の一般契約であって、
本 repo が既に不信任している IMC read/write（GJI）に依存しない。新しい `KeyAction`
variant を作らず既存の `SpecialKey::Enter` 送信をそのまま呼べるため、`is_composition_confirm_key`
等の既存ブックキーピングとも自然に整合する（決定5参照）。

---

## 決定3: composing 判定と実際の展開は出力に一番近い層（プラットフォームの `Output`）で行う

`ConfirmThenSend` を**その場で** `[Enter, inner]`/`[inner]` に展開せず、**意図を運ぶ
値のまま** `Response<KeyAction, _>` → 既存の送信パイプラインを素通しし、実際に
SendInput する直前（`crates/awase-windows/src/output/mod.rs::send_keys()`。この関数は
既に `self.composition`/TSF observer にアクセス可能、ログ用に
`ime_composition_active_now()` を既に呼んでいる）で展開する。

理由:

1. **鮮度**: `ctx.composing`（`Engine::on_input` に渡る値、事実5参照）はフック時点の
   スナップショットであり、キューイング・drain 等を経て実際に SendInput するまでの
   間に古くなりうる。`send_keys()` はその直前なので最も新しい composing 観測に
   基づいて確定要否を判断できる。
2. **クロスプラットフォーム性を壊さない**: `KeyAction::ConfirmThenSend` 自体は
   `awase` コアに置く OS 非依存の型（layer rule A-1 適合）。展開判定に使う
   composing 観測は各プラットフォームの `ImeDetector::is_composing()`
   （`src/platform.rs:99`、Windows/macOS/Linux 各実装）を経由すればよく、
   `awase-windows` 固有の話に閉じない設計になる。ただし Windows 実装は
   `ImeDetector::is_composing()` 単体ではなく ADR-107 決定5 が採った
   `ime_composition_active_now() || gji_candidate_visible_now()`（OR 合成）を
   踏襲する方が、preedit と候補ウィンドウの両方を取りこぼさない（同じ理由で
   ADR-107 が既にこの合成を採用済み）。
3. `Engine::on_input` レベルで展開すると、決定1が変わった場合の再設計コストが
   `send_keys()` に置くより大きい（呼び出し元が増える）。

**composing が不明/観測不能な場合は確定キーを送らない**（`inner` のみ送る）。
ADR-107 の INV-D（実証されるまで機能を無効化する）と同じ考え方——確定漏れは
「今までどおり手動確定が要る」という既知の劣化にとどまるが、composing でないのに
Enter を送ると実アプリに改行/フォーム送信という実害が発生しうるため、非対称に
安全側へ倒す。

---

## 決定4: キルスイッチは既定 Off の隠し設定にする

ADR-107 決定8 と同じパターンを踏襲する。`GeneralConfig`
（`src/config.rs`、`confirm_mode`/`HalfWidthAlnumTogglePolicy` と同じ並び）に

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PunctuationAutoConfirmPolicy {
    #[default]
    Off,
    On,
}
```

を追加し、`GeneralConfig::punctuation_auto_confirm` として持つ。**既定 `Off`
＝本ADR実装後もマージ直後の既定動作は無変化**。実機ソークが積み上がるまで
config.toml 手動編集のみで有効化できるオプトインとする（設定 GUI には出さない）。
決定1（表記）と決定3（composing 判定の信頼性）の両方に実機未検証の要素が残るため、
[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) の精神に沿い
デフォルト配布はしない。

---

## 決定5: 注入する Enter は既存の `SpecialKey::Enter`（`'入'`）送信経路をそのまま再利用する

新しいマーカーや専用送信関数は作らない。`YabValue::Special(SpecialKey::Enter)`
（`.yab` の `入` キーワード）が今日通っている送信経路をそのまま呼ぶ。

根拠: `is_composition_confirm_key(vk)`（vk.rs:314-317）や `ColdReason::PassthroughConfirmKey`
/`ReinjectConfirmKey` のブックキーピングは「確定キーが押された後は warm/cold 状態を
こう扱う」という**意味的に正しい**ロジックであり、CV4D が送る Enter も実際に
composition を確定させる行為である以上、同じ扱いを受けるべきである。ADR-107 決定2
（`IME_KANJI_MARKER` で自己注入を隠す）とは事情が異なる——あちらは「shadow toggle に
誤って乗ると belief を汚染する」ことを避けるためにマーカーで経路を変えたが、Enter の
送信には shadow toggle 効果が無く、隠す理由が無い。**実機検証で warm/cold
ブックキーピングとの相互作用に問題が見つかった場合のみ**、専用マーカーの導入を
再検討する（未解決の疑問3）。

---

## 決定6: 証拠義務

[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md) の
「キー選択（IME ON/OFF に送る VK）」ファミリーに準じて扱う。

### (a) Linux で回せる自動テスト

| 対象 | 置き場所 | 固定する内容 |
|---|---|---|
| `.yab` パース（決定1確定後） | `src/yab/mod.rs` `#[cfg(test)]` | `。CV4D` → `ConfirmThenSend(KeySequence("。"))` の往復（parse→serialize→parse）。クォート付き `'。CV4D'` は素の `Literal("。CV4D")` のまま変化しないこと（決定1の非対称性） |
| `YabValue → KeyAction` 変換 | `src/engine/nicola_fsm.rs` 既存テスト群に追加 | `ConfirmThenSend` が再帰的に内側を変換すること |
| 展開ロジック（決定3、プラットフォーム非依存部分があれば） | 展開判定を純粋関数として切り出せるなら `state/` 等 `#[cfg(test)]` | composing=true→`[Enter, inner]`、composing=false→`[inner]` |

### (b) 自動テストで代替できないもの（記録で担保する）

- Enter 確定が MS-IME / GJI 双方で composition を正しく確定させ、後続文字が正しく
  入力されること（IMM32 経路・TSF-native 経路の両方）。
- composing=false（何も確定すべきものが無い）状態で誤って Enter が実アプリに
  届かないこと（改行やフォーム送信が起きないこと）。
- warm/cold ブックキーピング（`PassthroughConfirmKey`/`ReinjectConfirmKey`）との
  相互作用に異常がないこと。

`docs/known-bugs.md` に本機能専用のエントリを立て、上記の実機確認結果を記録する
（着手前に `docs/experiments.md` へ事前登録エントリも立てる）。

---

## 却下した代替案

- **`ImmNotifyIME(CPS_COMPLETE)` を主経路にする**（決定2の対案）: `cancel_ime_composition()`
  と対称な実装は容易だが、GJI に対する IMC read/write 不信任という repo の既定方針
  （追補2の教訓）と、TSF-native アプリでの hwnd 解決の複雑さを考えると、Enter
  ベースより広く安全に倒せる根拠が無い。実機検証で Enter 方式が特定アプリで
  不発と判明した場合の**補完手段**として温存する。
- **composing 判定を `Engine::on_input` レベル（コア側）で確定して展開する**
  （決定3の対案）: 実装は簡単だが、鮮度の低い composing スナップショットに基づく
  ため、SendInput 直前に判定するより Enter 誤送信のリスクが高い。
- **確定キー注入に専用マーカーを新設する**（決定5の対案）: shadow toggle 汚染の
  リスクが無いため、ADR-107 決定2 のような隔離が要る根拠が今のところ無い。
  過剰設計。

## 未解決の疑問（実装着手前後で確認すること）

1. **`CV4D` の正確な yamabuki 表記**（区切り文字の有無、大文字小文字、クォート要否）
   は未確認。Issue 報告者への確認、または実在の yamabuki レイアウトファイル現物の
   入手が必要（決定1参照）。
2. **composing=false 時に確定キーを送らない、という非対称設計で十分か**。
   ユーザーが「変換確定前提」で連続入力するケースでは、composing の観測が
   1 tick 遅れて false→本来送るべき確定を送り損ねる可能性がある。実害が出た場合、
   決定3 の判定タイミングを見直す。
3. **Enter の能動送信が `PassthroughConfirmKey`/`ReinjectConfirmKey` の
   warm/cold ブックキーピングと衝突しないか**。決定5 の前提（同一経路の再利用で
   問題ない）を実機で確認する。
4. **GJI/MS-IME 双方で実際に候補が確定されるか**（IMC read-back を成否判定に
   使わない、実際に打鍵した文字で確認する、という追補2の教訓を踏襲する）。
