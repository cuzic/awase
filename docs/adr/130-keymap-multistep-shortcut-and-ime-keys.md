# ADR-130: `[[keymap]]` の `to` を複数キーの打鍵列へ一般化する

## ステータス

**実装済み（r2、Opus 2体〈architect/premortem〉の設計レビューで方向収束、
Codex CLIに実装を委譲、Opusによるコード差分の敵対的レビューを1周行い
Blocker 0件・Major指摘（`is_to_side`呼び出し側のテスト欠落・GUIのADRスコープ外
波及・doc不整合）をすべて修正して収束。IME 制御系 VK は本 ADR のスコープから
除外——理由は下記「IME 制御系 VK を本 ADR から除外する理由」参照）。
既知の制限は「実装時の既知の制限（GUI）」節を参照。**

## コンテキスト

### 出発点: `[[keymap]]` の `to` は単一 VK しか送れない

現行の `[[keymap]]`（ADR-114 ショートカット再割当て機能）は
`KeymapRule.to: Option<String>` — **単一の VK コード1つ**しか送れない
（`crates/awase-windows/src/keymap.rs::CompiledKeymap.send_vk: Option<VkCode>`）。
送信は `crates/awase-windows/src/output/held_modifiers.rs::send_keymap_target`
（Ctrl/Shift を一時解放 → `target_vk` の Down+Up を同一 `SendInput` バッチで
送信 → Ctrl/Shift 復元、`INJECTED_MARKER` 付き）。修飾キー付き送信すら
未対応。

ユーザーからの要望: 「打鍵列機能みたいな感じで、複数の打鍵を注入することは
できますか」——`[[keystroke_macro]]`（ADR-115、`.yab` 確定パイプライン専用の
複数ステップ出力機能）と同種の体験を、`[[keymap]]` のショートカット横取り
機能でも使いたい、という要望。

### r1 からの変更: IME 制御系 VK の opt-in は撤回した

r1 ドラフトは、上記の「複数キー打鍵列」に加えて「IME 制御系 VK
（`VK_DBE_HIRAGANA`/`VK_DBE_SBCSCHAR` 等）を `to` に含めることを明示
オプトインで許可する」機構も同一 ADR で扱おうとした。Opus 2体（architect/
premortem）の独立レビューが、それぞれ別の角度から**この opt-in は技術的に
成立しない**と結論した:

- **architect（C-1）**: `send_keymap_target` は `INJECTED_MARKER` 付きで
  送信するため、`hook.rs::is_self_injected` がフックの早い段階で
  `CallNextHookEx` して素通しする（ADR-114 決定6）。この結果、awase の
  `ImeModel::desired_open` を更新する経路（`UserImeSetIntent{PhysicalImeKey}`
  等）が一切発火しない。つまり `to = ["VK_DBE_SBCSCHAR"]` を押すと**実際の
  IME は OFF になるのに awase の belief は ON のまま**残り、`effective_open()`
  が Medium+ 観測で訂正されるまで awase は「開いている」と信じて NICOLA
  ローマ字を送り続ける（BUG-02 型のリテラル漏れ）。r1 の「awase 側に状態
  理解を要求しない、他のキー種別と対称」という主張は誤りだった——IME 制御系
  VK は awase が belief を持つ唯一のキー種別であり、対称ではない。
- **premortem（P1/P3）**: 同じ機構（SendInput で `VK_DBE_*` を注入して IME
  charset を変える）は `docs/experiments.md` エントリ07/08/09/17 で**既に
  3回試されて失敗し、汎用機能ごと撤去済み**——r1 ドラフトはこれを一つも
  引用していなかった。さらに実機ログ（`docs/known-bugs.md`）に、この経路の
  副作用を awase 自身の idle-conv-check/drift correction が誤って読み、
  3回連続実行後に GJI のタスクトレイ操作でしか復旧できなくなった記録がある
  ——「awase が追跡しない＝安全」という r1 の中心的な正当化は、実装（
  `note_explicit_ime_action`/`EXPLICIT_IME_SUPPRESS_MS` の未武装）によって
  裏切られている。

両者とも独立に「IME 制御系 VK は本 ADR のスコープから外し、別 ADR で
（実機検証を伴う、より重い設計として）扱うべき」と判定した。本 ADR はこれに
従う。IME 制御系 VK 側の検討事項は本文末尾「別 ADR へ持ち越す論点」に
まとめ、charset 軸切替そのものは別 ADR（番号未定）に切り出す。

## IME 制御系 VK を本 ADR から除外する理由（決定）

`crates/awase-windows/src/keymap.rs::forbidden_target_vk_reason` が
`ImeKeyKind::from_vk(vk).is_some()` を満たす VK（`VK_KANJI`/`VK_DBE_*`/
`VK_IME_ON`/`VK_IME_OFF` 等）を `from`/`to` 双方から禁止する既存の挙動
（ADR-114 決定5）を**そのまま維持**する。本 ADR はこの禁止に一切例外を
持ち込まない。

**IME 関連キーの可否判断に関しては**（他の禁止対象＝親指キー・Alt系・Win系・
`VK_CAPITAL` とは別基準として）、線引きの原則（architect m-2 の提案を採用）:
**「`ImeKeyKind::from_vk` が `Some`（＝ awase が open 軸の belief を持つ
キー）は禁止、それ以外は許可」**に統一する。親指キー・Alt系・Win系・
`VK_CAPITAL` の禁止は ADR-114 決定5 の理由（実行時状態依存の専用処理）の
ままで変更しない——これらは「opt-in で解決できる」種類の制約ではなく
「技術的に破綻する」種類の制約であり、IME 制御系 VK とは異なる理由で禁止
され続ける。

**決定（premortem N-2 を反映）**: 上記の判定式は `ImeKeyKind::from_vk` 単独
ではなく、**`ImeKeyKind::from_vk(vk).is_some() || vk_may_mutate_conv(vk)`
の OR** とする。理由: `vk.rs::vk_may_mutate_conv` は `VK_KANA`(0x15)・
`VK_CONVERT`(0x1C)・`0xF0..=0xF6` を conv-mutating と判定するが、
`ImeKeyKind::from_vk` は 0xF4 までしかカバーせず、`VK_DBE_ROMAN`(0xF5)/
`VK_DBE_NOROMAN`(0xF6) と `VK_CONVERT`(0x1C) が漏れる。

- `VK_DBE_ROMAN`/`VK_DBE_NOROMAN` はローマ字入力⇔JISかな入力方式を切り替え、
  `hook.rs:898-908` が「BUG-61 の実機調査で、いったん JIS かな側へ切り替わると
  `ImmSetConversionStatus`・`VK_DBE_ROMAN` 注入のどちらでも復旧不能と確定した
  （Windows にこの入力方式を外部から戻す公式 API が存在しない）」と記録する
  領域であり、`ImeKeyKind` 対象外だからと素通しすると即座に実害になる。
- `VK_CONVERT`（=「変換」）は今日、既定では右親指キーとして偶然ブロックされて
  いるに過ぎず、親指キーを変更したユーザー（`config.rs:239,352` に実在する
  設定）ではブロックが外れる。

**この OR 判定は `to` 側の判定にのみ適用し、`from` 側には適用しない**
（premortem R2-b）。危険なのは VK を**送る**ことであり、`from` は横取り・
消費するだけで送らないため、conv-mutating を理由に `from` を禁止する根拠が
ない。`from = "変換"`（親指キーを Space 等に変更したユーザーが、空いた
「変換」をホットキーとして使う設定、`config.rs:239,352` に実在するパターン）
まで丸ごと skip させるのは過剰禁止になる。このリポジトリには `from`/`to` を
非対称に扱う先例が既にある
（`is_forbidden_ctrl_or_shift_primary_key`（`keymap.rs:56-61`）は `from`
専用で `to` には適用されない、ADR-114 決定5・MA-3）。実装は
`forbidden_target_vk_reason` に `is_to_side: bool` を追加するか、`to` 専用の
判定関数（例: `forbidden_send_vk_reason`）を切り出す。

この OR 判定を `to` 側に追加することは、既存の単一 VK `to`（ADR-114）にも
遡って適用される安全側の強化であり、本 ADR 固有の新しい禁止ではない。

## なぜ ADR-115 の打鍵列エンジン（`[[keystroke_macro]]`）を再利用しないか

複数ステップの送信エンジンを新規実装せず、ADR-115 が6ラウンドかけて磨いた
既存エンジン（`Literal`/`KeySequence`/`Special`/`CtrlChord`）を `[[keymap]]`
の `to` から `@name` 参照する案（r1 の候補B）を検討したが、**再利用不可能と
判断し棄却する**。理由は「工数を惜しんで独自実装した」のではなく、2つの
delivery コンテキストが根本的に異なるため:

- **`Output::send_keys` は `OutputSession` を開く。** TSF warmup 設置・
  Unicode literal observer・composition cold-start カウンタ等、`.yab` の
  romaji→かな確定という文脈に紐づく副作用一式が起動する。`Ctrl+F13 → F7`
  という単純なショートカット送信のためにこれを起動するのは責務の逸脱であり、
  cold-start 検出（BUG-02 ファミリー）を誤爆させうる。
- **決定的な理由: ADR-115 のエンジンには `HeldModifiers` の release/restore
  が無い。** ADR-115 決定1 は「ユーザーが物理的に Shift/Alt を押している
  最中に `CtrlChord` が発火すると意図しない修飾キー付きコンボになりうる」
  ことを**稀な既知の限界**として明示的に受容している（`.yab` の打鍵列セルは
  通常、修飾キーを押しながら打つ位置には置かれない、という運用前提に依拠）。
  ところが `[[keymap]]` では `from = "Ctrl+F13"` がマッチした時点で**必ず
  Ctrl が物理的に押されている**。ADR-115 が「稀」として許容した限界が、
  `[[keymap]]` の delivery コンテキストでは**100%発生する**。これは
  ADR-114 決定3 が `HeldModifiers::push_release`/`push_restore` を必須にした
  理由（ADR-037「修飾キー残留問題」の再発防止）と正面から衝突する。
  「`send_keys` から低レベル部分だけ切り出せば再利用できるのでは」という
  再提案が出た場合も、この非互換は切り出しでは解消しない（delivery
  コンテキストの違いそのものが原因のため）——将来同じ再利用案が浮上した
  際にこの段落を参照できるよう、理由をここに明記しておく。
- 加えて、ADR-115 の `KeyAction::KeyUp(vk)`（独立した Up 単体ステップ）は
  `state/keymap_latch.rs::release_all()` が依存する「`to` は必ず Down+Up
  ペアで完結する」という不変条件（下記決定3）と非互換であり、`send_ctrl_chord`
  （`output/key_injector.rs`）も物理修飾キーを解放しない。

## 決定

### 1. `KeymapRule.to` を `Vec<String>` へ拡張する

```toml
[[keymap]]
from = "Ctrl+F13"
to = ["F7", "F8"]          # 複数ステップの打鍵列（実行順に送信）
```

（例に `変換`/`かな` 等を使わない理由: `変換`(`VK_CONVERT`)は既定の右親指キーで
`forbidden_target_vk_reason` により禁止される上、`VK_CONVERT`/`VK_KANA` は
`vk.rs::vk_may_mutate_conv` に含まれる conv-mutating VK であり、決定4の
「通常キーは conv-mutating ではないため単一バッチで安全」という前提の反例に
なってしまう——親指キーでも conv-mutating でもない `F7`/`F8` 等が本 ADR の
主対象である。）

- `src/config.rs::KeymapRule.to` の型を `Option<String>` → `Vec<String>`
  （空 = 消費のみ）に変更する。
- **serde は String（旧形式）/ `Vec<String>`（新形式）/ 省略の3形を受ける**
  `deserialize_with` を実装し、既存の `to = "F7"` という手書き設定を壊さない
  （architect M-3）。
- ただし `AppConfig::save()` は `toml::to_string_pretty(self)` で設定全体を
  書き戻すため、ユーザーが手書きした `to = "F7"` は awase-settings で無関係な
  項目を1つ保存しただけで `to = ["F7"]` に正規化される。これは仕様として
  ドキュメントに明記する（挙動自体は許容する——ADR-115 実装時に同型の正規化
  が既に許容されている前例に倣う）。
- `to = []` と `to` 省略（後方互換のため）は同じ「消費のみ」を意味する
  （architect m-6）。

### 2. 各ステップに修飾キーは書けない（今日の暗黙のバグを踏襲拡大しない）

現行の `to` パース（`keymap.rs:119-126`）は `parse_key_combo` フォールバック
経由で `to = "Ctrl+M"` のような入力を受理するが、**`ctrl`/`shift`/`alt` を
静かに捨てて素の VK だけを送っている**（今日から存在するサイレントバグ）。
列化するとユーザーは自然に `to = ["Ctrl+M", "F7"]` と書くため、このバグが
列全体に拡大する（architect M-2）。

**決定**: 各ステップの解決に `parse_key_combo` フォールバックを使わない。
修飾子を含む文字列が `to` の要素にあれば、そのルール全体を他のパース失敗
ルールと同様 `log::warn!` + skip する。修飾キー付きステップが将来必要に
なった場合は、ADR-115 の `CtrlChord` 相当（Ctrl 限定・単一バッチ）を専用の
ステップ種別として別途追加検討する。

**これは後方互換の破れを含む決定である**: 現行実装は `to = "Ctrl+M"` を
（意図とは異なる形だが）受理し、修飾子を捨てた素の `M` を送っている。本 ADR
適用後は同じ設定がルールごと skip され、何も送られなくなる。意図と異なる
動作を静かに続けさせるより、`log::warn!` でユーザーに気付かせる方を選ぶ。

### 3. 各ステップは必ず Down+Up ペアで完結する（新しい不変条件）

`state/keymap_latch.rs::release_all()` は「`target_vk` の KeyUp は注入しない
（テーブルを空にするだけで済む）」という設計になっており、これは
「`target_vk` は KeyDown 側で Down+Up 同一バッチとして即時完結する」という
決定3（ADR-114）の不変条件に完全に依存している（architect M-7）。

**決定**: 列化後も各ステップは独立した Down+Up ペアとして送信し、
Down のみ・Up のみの片方だけのステップは許可しない。将来「キーを押しっぱなし
にする」ステップ種別を追加する場合は、ADR-110 の
`release_all_latched_remap_targets()` 相当（KeyUp 明示注入）が必要になる
ことを、その時点の設計で必ず再検討する。この不変条件を
`state/keymap_latch.rs::release_all()` の doc コメントから本 ADR へ参照を
張る形で明記する。

### 4. 全ステップを単一 `SendInput` バッチで送る

`send_keymap_target` の「中間状態を外部に見せない」原則（Chrome cold-start
対策の VK_A+BS アトミックバッチと同じ）を踏襲し、Ctrl/Shift の release/
restore を列全体で1回ずつ、間に各ステップの Down+Up ペアを並べた単一
`SendInput` バッチとして送信する（capacity 見積りは `2 + steps.len()*2 + 2`
に拡張、architect m-3）。IME 制御系 VK が本 ADR の対象外になったことで、
premortem P2 が指摘した「バッチか間隔か」の二律背反（先頭文字破損 vs latch
不変条件破壊）は本 ADR では発生しない——通常キー（英字/OEM記号/F13-F20等）は
conv-mutating ではないため、単一バッチで安全に送れる（「通常キー」の範囲は
下記「IME 制御系 VK を本 ADR から除外する理由」の OR 判定——
`ImeKeyKind::from_vk(vk).is_some() || vk_may_mutate_conv(vk)`——を `to` 側
判定に組み込むことで確定する。`VK_CONVERT`/`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`
はこの OR により `to` から禁止されるため、ここで単一バッチが安全と言える
「通常キー」に例外は残らない）。

### 5. composition キャンセル判定は列の先頭で1回だけ行う

`runtime/message_handlers.rs` の既存の単一 VK 前提のキャンセル判定
（`:282-286`）を、列全体の送信前に1回だけ行う形に変更する。理由（architect
M-6）: (a) `ime_composition_active_now()` は atomic 読み取りであり、送信直後
に再度読むと値が食い違う既知の罠がある、(b) 全ステップを単一バッチで送る
以上、判定を挟む物理的な場所がない。

**`to` が空（消費のみ）の場合はこのキャンセル判定自体を行わない**（現行の
`if let Some(target_vk) = matched { ... }` の内側で判定している挙動を維持、
architect R2-4）。ADR-114 決定3 が composition 破棄を正当化した論理は
「ユーザーが明示設定したキー送信を優先する」であり、何も送らないルールには
及ばない。

### 6. GUI（`awase-settings`）の `from`/`to` 候補を `forbidden_target_vk_reason`
と SSOT 化する

前回の修正（`KEYMAP_VIRTUAL_ONLY_KEYS`、物理キーが存在しない VK を `from`
から除外）は IME 制御系 VK の7つだけを対象にしており、`かな`
（`VK_KANA`=0x15、`ImeKeyKind::Kana`）・`漢字`（`VK_KANJI`=0x19、
`ImeKeyKind::KanjiToggle`）・`変換`/`無変換`（既定の親指キー）が同じ穴を
持ったまま残っている——しかもこれをテスト
（`physical_ime_keys_remain_in_from_options`）が「仕様」として固定していた
（architect M-4）。

**決定**: `crates/awase-windows/src/keymap.rs::forbidden_target_vk_reason` を
`pub` 化し、`awase-settings`（既に `awase-windows` に依存済み）から直接呼ぶ。
GUI 側の手書きリスト `KEYMAP_VIRTUAL_ONLY_KEYS` は廃止し、単一の判定関数を
SSOT にする。親指キーは実行時値なので GUI は現在の設定値
（`left_thumb_key`/`right_thumb_key`）を渡す（`is_muhenkan_thumb_key` と同型
の既存パターンを踏襲）。`physical_ime_keys_remain_in_from_options` テストは
削除または反転する。`to` 側も同じ関数で複数選択 UI から候補を絞り込む。

**`to` 側 UI は VK 選択のみを提供し、修飾子（Ctrl/Shift）チェックボックスは
出さない**（決定2 と対応、architect R2-5）。`forbidden_target_vk_reason` は
VK 単位の判定であり、決定2 の「修飾子付きステップは skip」の管轄外——UI に
修飾子チェックボックスを付けると、SSOT 化で塞いだはずの「選べるのに動かない」
が別ルートで再発する。

### 7. `to` の型変更に追随が必要な既存コード

`find_match`（`keymap.rs:179-190`）・`warn_if_vk_conflicts`
（`keymap.rs:220-223`、直し忘れると GJI 専用 Fn キー衝突検出が機能しなくなる）
・`runtime/mod.rs::recompute_active_keymaps`（`:1594`）の3箇所は `to` の型
変更に同時追随する（architect m-4）。

### 既知の限界（変更しない）

`runtime/message_handlers.rs:200-204` の通り、`[[keymap]]` は
`FocusKind::NonText` では一切効かない（v1 スコープからの既知の限界）。複数
ステップ化後もこの限界は変わらない（premortem N-3）。

## 別 ADR へ持ち越す論点（IME 制御系 VK・charset 軸切替）

本 ADR のスコープからは外すが、ユーザーの要望第2項（半角英数/全角英数/
半角カナへのショートカット割り当て）自体を諦めるものではない。別 ADR
起票時、以下を必ず踏まえること（architect/premortem 双方の指摘の要約）:

1. **既存の `keys.ime_on`/`ime_off`/`ime_toggle` との重複を避ける
   （architect C-2）。** open 軸（IME ON/OFF）は既にこれらが belief 統合済み
   で実現している。新しい経路が必要なのは **conv 軸**（半角カナ/全角英数、
   `keys.ime_*` の語彙では表現できない）に限られる。この壁を正面から扱うのが
   別 ADR の主題であり、ADR-094（charset 軸の追跡撤去）を巻き戻さない形の
   設計が要る。
2. **GJI `config1.db` 書き込み方式は既に撤去済み（事実訂正）。**
   `fc5898ff`（2026-09-02）で `awase-gji-config::write` 一式が撤去された
   ——protobuf のデフォルト値省略により `session_keymap` 判定が誤診断し、
   起動時ポップアップで同意を得た直後に失敗ダイアログを出す UX が問題視
   された。ADR-091 が「実機検証済みで堅牢」と評価していたのは撤去前の話で
   あり、別 ADR はこれを撤去後の状態として正しく引用すること。
3. **BUG-50・2026-08-11 危険度分類との方向対立を明示すること。**
   `VK_DBE_*` は「開く」と「特定 charset に強制する」を1キーに束ねる複合
   副作用キーであり、2026-08-11 の危険度分類はこれを「複合副作用・危険」に
   分類しロードマップで「設定候補から除去」を掲げている。charset 軸
   ショートカット機能はこの方向と正反対であり、別 ADR はこの対立を隠さず
   扱うこと。
4. **`docs/experiments.md` エントリ07/08/09/17/19 を読んでから設計すること。**
   エントリ17 は「将来アプリごとに動的キー割当てを変更する機能が IME 制御
   キーを対象にする場合、エントリ07/08/09 を先に読め」と本件そのものを
   名指しで警告している。実機で動いた唯一のレシピは `IME_KANJI_MARKER`
   （`INJECTED_MARKER` ではない）+ `wScan=0` 固定 + synthetic Shift↑ 前置
   であり、`wScan` を付けると CapsLock 物理トグル等の副作用がある
   （エントリ05/07）。
5. **`note_explicit_ime_action`/`EXPLICIT_IME_SUPPRESS_MS` の武装、または
   同等の防御を設計に含めること。** これが無いと idle-conv-check が過渡状態
   を誤読し、drift correction が介入して実機で復旧不能になった記録がある。
6. **連続発火時の最小間隔ガード。** 実機ログに「3回連続実行後に GJI タスク
   トレイ操作でしか復旧できなかった」記録がある。

## 実装時の既知の制限（GUI）→ 解消済み（2026-09-04 追補）

コード差分の敵対的レビュー（Opus）で発見された、実装時点でのGUI側の既知の
制限。**以下2点は追補コミットで解消した**（`main_key_combo_to` の絞り込み
自体は変更していないため、機能・禁止規則には影響しない）:

- ~~既存ルールの `to` にドロップダウンからステップを追加する手段が無い~~
  → 各ステップ行に「＋」ボタンを追加した。`rule.to.push(String::new())`
  で空ステップを追加し、`main_key_combo_to` がそれを「（未選択）」として
  表示、ドロップダウンから選ぶ流れになる。
- ~~⌨ キャプチャの挙動が新規ルールと既存ルールで非対称（新規=置換、
  既存=追記）~~ → `CaptureTarget::ExistingTo` にステップ index を追加し
  （`ExistingTo(usize, usize)`）、⌨ キャプチャは常にそのステップを
  **置換**するよう統一した。「＋＝追加／x＝削除／⌨＝置換」という直交した
  操作になり、新規ルール側と対称。
- 副次的に m7（キャプチャが禁止キーだったとき UI が無反応だった問題）も
  同時に修正した: `keymap_forbidden_reason`（理由文字列を返す版）を新設し、
  拒否時は `self.status` に理由を表示する（`ExistingFrom`/`ExistingTo`/
  `NewFrom`/`NewTo` の4経路すべてに適用）。

**残る制限**: ⌨ キャプチャは `egui` が変換できるキーに限られ、
PrintScreen や IME 関連キーはキャプチャ不可（ドロップダウン専用、
`egui_key_to_internal` の doc 参照）——これはドロップダウンから選べば
問題なく設定できるため、実用上の制約ではない。

## 非目標

- IME 制御系 VK の `[[keymap]]` への opt-in は本 ADR の非目標（上記の通り
  別 ADR へ）。
- charset 軸・IME 状態全般の belief 化・観測・追跡は本 ADR の対象外
  （ADR-094 の決定を維持）。
- eisu（英数）/かな の二値境界（`state/eisu_recovery.rs`）には触れない。
