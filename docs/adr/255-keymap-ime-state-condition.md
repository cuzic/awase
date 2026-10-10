---
id: ADR-255
title: |-
  `[[keymap]]` に IME 状態の条件(`ime = "off" | "on"`)を足し、IME OFF のときだけ無変換/変換を Space にできるようにする
summary: |-
  顧客報告「IME OFF のとき GJI の設定が反映されず、変換/無変換を空白入力に割り当てても動かない」を GitHub Windows CI で実機検証した(ブランチ ci/e2e-direct-space、run 38058313464・38059338837)。結果: GJI の CUSTOM 表は読まれ、直接入力の無変換/変換に IMEOn を割り当てると awase の有無にかかわらず効く。一方 DirectInput 行に InsertSpace/InsertHalfSpace/InsertFullSpace を割り当てても入力欄に空白は入らなかった(awase あり/なし、単独タップの Suppress 解除でも同じ)。ただし Precomposition に同じ割り当てをした対照でも空白が入らず、「直接入力では不可」と「無変換/変換というキーでは InsertSpace が文字を出さない」を分けられていない。awase は IME OFF の無変換/変換を握っていない(素通し)。そこで GJI の仕様に頼らず、既存の `[[keymap]]`(ADR-114)に IME 状態の条件を足し、IME OFF のときだけ無変換/変換を `VK_SPACE` に再割り当てできるようにする。IME ON のときはルールが当たらず、従来どおり親指キーとして働く。
status: |-
  起草(2026-10-10)。Opus レビュー前。実装は未着手。
related_adr:
  - "ADR-114"
  - "ADR-130"
  - "ADR-206"
  - "ADR-141"
  - "ADR-230"
---

# ADR-255: `[[keymap]]` に IME 状態の条件を足す

## ステータス

起草(2026-10-10)。Opus 敵対的レビュー前。実装は未着手。

## コンテキスト

### 報告

「IME OFF のとき、Google 日本語入力の設定が反映されず、変換/無変換を空白入力に割り当てても期待どおり動作しない」。

### 実機検証の結果(GitHub Actions windows-latest、ブランチ `ci/e2e-direct-space`)

構成名 `sc-direct-space-*`(`.github/workflows/e2e-ime.yml`、判定なしの回収)。スパイクが IME OFF(`--seq` の 1A)にしてから無変換(0x1D)/変換(0x1C)を注入し、各押下の +1500ms の入力欄末尾(`tail`)と未確定(`comp`)を記録する。各セル n=1。

| 構成(CUSTOM 表の行) | awase | 結果 |
| --- | --- | --- |
| 行なし(基準) | なし/あり | 空白は入らない |
| DirectInput の無変換/変換 = InsertSpace | なし/あり/あり(単独タップ Suppress 解除) | 空白は入らない |
| 同 InsertHalfSpace / InsertFullSpace | なし/あり | 空白は入らない |
| 対照: DirectInput の無変換/変換 = IMEOn | なし/あり | IME が閉→開(open 0→1)。CUSTOM 表は読まれ、直接入力の行は効く |
| 対照: Precomposition の無変換/変換 = InsertSpace | なし/あり/あり(Suppress 解除) | IME は開くが空白は入らない |

(run 38058313464 は `sc-direct-space-*` 10 構成、run 38059338837 は対照を足した 13 構成。成果物は 7 日で失効する。)

### 分かったことと分かっていないこと

- 分かった: (1) awase は IME OFF(エンジン非活性)の無変換/変換を握らず GJI へ素通しする(`state/physical_disposition.rs` の無変換/変換は物理配送を常に Allow)。(2) GJI は直接入力の行を読み、IMEOn のような**状態を変えるコマンド**は効く。(3) 直接入力の InsertSpace 系は空白を出さなかった。
- **分かっていない**: 対照(Precomposition)でも空白が出ないので、「直接入力に空白入力は割り当てられない」のか「無変換/変換というキーでは InsertSpace が文字を出さない」のかを区別できていない。入力欄の観測が空白を捉えているかも未確認(スペースキー自体を注入する正の対照が無い)。したがって「GJI の仕様で不可」とは書かない。本 ADR の決定は、どちらでも成り立つように、GJI の振る舞いに依存しない。

### 既存の `[[keymap]]`(ADR-114)

`[[keymap]]` は `app`・`from`・`to` の3項目。`runtime/message_handlers.rs::consume_keymap_match` が NICOLA エンジンより前、`NonText` 早期 return の後、`[[post_bypass]]` の前で照合し、`keymap_latch` で KeyUp と自動リピートを回収する(Down で送れば Up は latch が消費する)。有効ルールは `recompute_active_keymaps` がフォーカス変更のたびに `filter_active(process_name)` で絞る。

## 決定

### 決定1: `KeymapRule` に `ime` を足す

```toml
[[keymap]]
from = "VK_NONCONVERT"
to = ["VK_SPACE"]
ime = "off"
```

- `ime`: `"off"` | `"on"`、省略可(省略 = 従来どおり IME 状態を問わない。**既存のルールの挙動は変えない**)。
- `"off"` は「IME が確実に OFF のときだけ」、`"on"` は「IME が確実に ON のときだけ」当てる。**確実でないとき(未知・矛盾)はどちらも当てず、素通し**にする(決定3)。

### 決定2: 条件は照合のたびに評価する

`active_keymaps` はフォーカス変更時にしか作り直されないので、`ime` の条件を集合の構築時に評価してはならない。`KeymapTable::find_match` に IME 状態(後述の三値)を引数で渡し、照合時に比べる。`filter_active` は従来どおり `app` だけで絞る。

### 決定3: 「確実に OFF/ON」の定義(三値)

`ImeStateHub::effective_open()` は `bool` を返し、未知と確認済み false を区別しない(`fix-requires-evidence.md` の `shadow_on` の項と同型の罠)。そこで照合に渡す値は次の三値とする:

```text
ImeOpenView = Open | Closed | Unknown
```

`Closed`/`Open` にするのは、`resolve_open_at` の判定根拠(`DecidedBy`)が**信頼できる観測または明示のユーザー意図**のときに限る。`HeuristicDefault`(Low)や `desired_open` の既定値だけが根拠のときは `Unknown`。具体の写像は実装時に `state/` の純粋関数(`classify_*` の流儀)として置き、Linux の単体テストで表を固定する。**belief は読むだけで、書かない**(`ime-belief-architecture.md` の書き込み点の規約に触れない)。

- 日本語 IME でない(`is_japanese_ime == false`)ときは `Unknown`(ルールを当てない)。
- 学習した表・予測(ADR-191)は belief を通して間接に効くが、ここでは追加の根拠を足さない。

### 決定4: `from` の禁止対象を `ime` 付きルールで緩める

`keymap.rs::forbidden_target_vk_reason` は `from` に**親指キー**を禁じる(ADR-114 決定5)。既定の親指キーは無変換/変換なので、そのままでは `from = "VK_NONCONVERT"` は警告で skip される。

- `ime = "off"` が付いたルールに限り、`from` の主キーに親指キーを許す。理由: IME OFF(エンジン非活性)では親指キーに役割がなく、ADR-114 決定5 が守ろうとした「親指キーの held 判定との二重管理」は、ルールが `Closed` のときしか当たらないので起きない。
- **許さないもの**: `ime = "on"` や `ime` 省略のルールでは従来どおり親指キーを禁じる(IME ON ではエンジンが親指として使うため)。IME 制御系 VK(`ImeKeyKind::from_vk` が `Some`)・Alt/Win 系・Ctrl/Shift の主キー・`VK_CAPITAL` の禁止は変えない。無変換(0x1D)/変換(0x1C)は `ImeKeyKind` に含まれない(`vk.rs` の列挙は 0x15〜0x1A と 0xF0〜0xF4)。
- `to` の禁止は変えない(`VK_SPACE` は禁止対象でない)。

### 決定5: Down/Up の非対称を作らない

- KeyDown で照合が当たったら `keymap_latch` に積み、KeyUp とリピートは latch が消費する(既存)。KeyUp 時には `ime` を**再評価しない**。Down が `Closed` で当たったあとに belief が `Open` に変わっても、Up は latch が回収する。
- 逆に Down が当たらず(`Open`/`Unknown`)エンジンへ渡った親指キーは、Up の前に `Closed` になっても `[[keymap]]` の対象にしない(latch が無いので Up はエンジンへ行く。従来と同じ)。
- これは `fix-requires-evidence.md` の「物理キー押下ラッチ」ファミリーに触れるので、回帰テストを必須とする(下記)。

### 決定6: 競合の警告

- `ime = "off"` の無変換/変換ルールが、`keys.ime_on/ime_off/ime_toggle`、`muhenkan_solo_tap_dedicated_fn_key`、`muhenkan_solo_tap_ime_action`(ADR-206)と重なる場合は、`recompute_active_keymaps` の既存の衝突警告(ADR-114 未解決の疑問 4・5)に載せて警告する(動作は変えない)。IME OFF で無変換を IME ON のキーとして使う人は、`[[keymap]]` が先に消費すると IME が開けなくなる。
- 設定画面(ショートカット再割り当てタブ)に「IME の状態」の列(指定なし/ON のとき/OFF のとき)を足し、`ime = "off"` の無変換/変換が IME ON のキーと重なる場合は、その場で警告する。

### 決定7: 範囲外

- GJI の CUSTOM 表の書き換え・生成(ADR-231 の領域)はしない。
- `NonText` のフォーカスでは従来どおり効かない(ADR-114 の既知の限界を継承)。
- Scancode Map(ADR-230)で入れ替えたキーには、入れ替え後のキーで効く(フックに届く VK が基準)。両機能の同時リリースは避ける(別の判断、リリース計画で扱う)。

## 検証計画

1. **Linux 単体**(`cargo test --lib` / `-p awase-windows-core`): `KeymapRule` の `ime` の deserialize(省略・`"on"`・`"off"`・不正値)、`find_match` の三値の表(Closed/Open/Unknown × off/on/省略)、`forbidden_target_vk_reason` の `ime` 付き緩和(`off` のとき親指キー可、`on`・省略のとき不可)、三値の写像(`DecidedBy` ごと)。
2. **Windows CI**(`e2e-ime.yml`): `sc-direct-space-*` の流用。awase あり・`[[keymap]]` で `VK_NONCONVERT` → `VK_SPACE`(`ime = "off"`)を設定し、(a) IME OFF の無変換で入力欄に空白が入ること、(b) IME ON(親指キー)では入らず従来の挙動であること、(c) Down 後に IME が切り替わっても Up が回収されて固着しないこと。**正の対照として、スペースキー(VK 0x20)自体を注入して入力欄の観測が空白を捉えることを先に確認する**。
3. **実機**: 報告者の構成(GJI の設定、親指キー、`keys.ime_*`)で確認する。

## 未解決の疑問(レビューで見てほしい点)

1. 決定3の三値の写像: `resolve_open_at` の `DecidedBy` のどれを `Closed` とみなすか。TSF ネイティブのアプリで IME が状態を偽る場合に、誤って `Closed` になる経路が残らないか(`ime-belief-architecture.md` の「観測が乏しいときは `HeuristicDefault`」との整合)。
2. 決定4の緩和が、`keymap.rs` の禁止の根拠(held 判定の二重管理)を本当に壊さないか。特に、エンジンが非活性なのに IME が ON(ユーザーがエンジンを切った、アプリ別の抑制)のとき、親指キーが `[[keymap]]` に当たらないことの確認(`Closed` でないので当たらないはずだが、`compute_active` との関係を確かめる)。
3. フックの物理配送の決定(`physical_disposition.rs`)が無変換/変換を常に Allow にするため、`[[keymap]]` が消費しても元の無変換が GJI へ届いて二重になる経路がないか。`consume_keymap_match` は `deliver_key_event`(フックが Suppress して非同期に再投入する経路か、同期か)のどこで走るかを実コードで確認する。
4. 二つの CI 結果が「無変換/変換というキーでは InsertSpace が文字を出さない」可能性を残している。この ADR の決定は GJI に依存しないが、報告者への説明(GJI の設定では実現できないのか)には、正の対照が要る。
5. 設定名(`ime` / `when_ime`)と値(`"off"`/`"on"`)。既存の設定名の流儀との整合。
6. 将来 `ime` を `app` と同じ粒度の条件(`composing` など)へ広げる余地を残すか、今は `ime` だけにするか(今は `ime` だけにする方針)。

## 影響範囲(実装時)

`src/config.rs`(`KeymapRule`)、`crates/awase-windows-core/src/keymap.rs`(`find_match`・禁止判定)、`state/` に三値の純粋関数、`runtime/message_handlers.rs`(`consume_keymap_match` に三値を渡す)、`runtime/mod.rs::recompute_active_keymaps`(警告)、`crates/awase-settings`(列の追加)。再発ファミリー(物理キー押下ラッチ)に触れるため、`fix-requires-evidence.md` の (a) 回帰テストを同じ PR に含める。
