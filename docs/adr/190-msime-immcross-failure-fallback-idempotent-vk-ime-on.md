---
id: ADR-190
title: |-
  Microsoft IMEでImmCross(WM_IME_CONTROL)が失敗したとき、非冪等なVK_KANJIトグルではなく冪等なVK_IME_ON/OFF(MsImeDirect)へフォールバックする
summary: |-
  CI実機E2E(`sc-*`)で、Microsoft IME本体(Win32 Edit、ImmCrossプロファイル)にawaseを起動すると、直接入力からの最初のひらがなキーでIMEが開かず
  Engineだけ ON になる(実IME OFF + Engine ON、3/3)。awaseなしなら開く。原因: (1)MS-IME本体への最初のImmCross set-open(0x0006)がCIで約150msのタイム
  アウトで`success=false`、(2)`imm_cross_write`は事後読み取りが`None`(不明)でも`Failed`にする(`Some(open)`のときだけ`AlreadyMatched`)、
  (3)`ImmCross × MsIme`のチェーンは`[ImmCross, KanjiToggle]`(ADR-089)で、再注入されたF2が既に開いたIMEを非冪等なVK_KANJIが閉じる。
  検証: KanjiToggle撤去(a8)で各3/3 ALL PASS、MsImeDirectへ差し替え(a9)で実IMEが全手順で正しい。決定: `ImmCross × MsIme`のチェーンを
  `[ImmCross, MsImeDirect]`にする(`MsImeDirect`は2026-08-06以降VK_IME_ON/OFFの冪等キー、TsfNativeでは既に使用中)。
status: |-
  **ドラフト(未実装)**。CI検証済み(a8: run 35515406371、a9: run 35516320434)。実機(dragonflyg4)未検証。opus-adversarial-consult未実施。
related_adr:
  - "ADR-063"
  - "ADR-089"
  - "ADR-117"
  - "ADR-189"
---

# ADR-190: MS-IMEのImmCross失敗後は冪等なVK_IME_ON/OFFへフォールバックする

関連: [BUG-152](../known-bugs/BUG-152.md)。

## 背景と症状

CI実機E2E(`.github/workflows/e2e-ime.yml`の`sc-*`構成、Microsoft IME本体+awase)で、直接入力からの**最初のひらがなキー(0xF2)**を押すと、
実IMEは閉じたまま(`open=0`)なのにawaseはEngine ON(`Engine activated`)になり、続く`k`がNICOLAのかな(`き`)として直接入力に出た
(`sc-dbe/kanji/shift-msime-native` 各3/3、全構成で再現)。awaseなし(`sc-*-noawase`)では同じキー列でF2はIMEを開き、F0/F3/F4もトグルする。

## 原因(ログとablationで確定)

1. Microsoft IME本体への**最初のImmCross set-open**(`WM_IME_CONTROL 0x0006`)が、CIランナーで約150msの`SendMessageTimeout`に収まらず
   `set_ime_open_for_target ... success=false send_elapsed=154ms`。GJIでは同じ呼び出しが12〜25msで成功する。
2. `runtime/open_chain.rs::imm_cross_write`は`Failed`のとき`read_ime_state_fast().ime_on`を読み直すが、これも失敗して`None`になる。
   **`Some(open)`のときだけ`AlreadyMatched`、`None`(不明)は`Failed`**として次の機構へ落とす(`open_chain.rs`の`ActuationOutcome::Failed`腕)。
   `fallback_write`のdocは「`Failed`は実際に確認した場合だけ」と書くが、`None`も含むので実装と食い違う。
3. `ImmCross × MsIme`のチェーンは`[ImmCross, KanjiToggle]`(`state/app_ime_policy.rs:65`の`CHAIN_IMM_CROSS_THEN_KANJI`、ADR-089 §2.8の表)。
   `MsImeDirectStrategy::is_applicable`が`!can_use_imm32_cross_process()`を要求する(`state/key_sequence_policy.rs:60`)ため、ImmCrossプロファイルでは
   適用されず(`fallback_write: mechanism=MsImeDirect not applicable → Failed`)、最後の`KanjiToggle`が非冪等な`VK_KANJI`を送る。
   ひらがなキーの再注入(`[reinject] vk=0xf2`)が既にIMEを開いていると、このトグルが閉じる。

検証した仮説(すべてCI実機、各3回):

| 実験 | 結果 |
|---|---|
| 再注入の`scan=0`が原因か(`sc-probe-vk-msime-native`) | **否定**。MS-IME本体はscan=0のF2でも開く(open 0→1) |
| a8: `fallback_write`でKanjiToggleを送らない | `sc-dbe`/`sc-kanji-msime-native` 各3/3 **ALL PASS** |
| a9: `ms_ime_direct_applicable`の`!can_use_imm32_cross_process()`を外す | `sc-dbe`/`sc-shift` 各3/3 ALL PASS、`sc-kanji`は実IMEが全手順で正しい(2/3のFAILはCIのスパイク側タイマー遅延で`k`プローブが判定窓を超えた「?」) |

`MsImeDirect`は`VK_IME_ON`(0x16)/`VK_IME_OFF`(0x1A)を`SendInput`する冪等キーで、`conv`を変えない(`ime_controller.rs` `MsImeDirectStrategy`のdoc、
2026-08-06〜)。ADR-063の「`VK_DBE_HIRAGANA`/`VK_DBE_ALPHANUMERIC`」の記述は古い。CIでは、awaseを経由しないパススルーで`VK_IME_ON/OFF`が
Microsoft IME本体を押すたびに開閉した(`sc-kanji-msime-native-a8`手順4〜7)。Chromeが`VK_IME_ON/OFF`を受け付けない(`docs/experiments.md` 2026-05-22)のは
Chrome × GJIの話で、ImmCross(Win32 Edit)プロファイルの対象ではない。

## なぜ`MsImeDirect`が今まで入っていなかったか

ADR-089 §2.8の表は「`ImmCross × MsIme`に`MsImeDirect`を足すと、現行が到達しない経路を新設することになる」を理由に`KanjiToggle`を置いた。
これは**当時の実装の書き写し**で、「`VK_IME_ON`がImmCrossプロファイルで危険」という実測ではない(この組で試した記録は`docs/experiments.md`にも無い)。
ADR-089自身が「将来`MsImeDirectStrategy`が`Failed`を返すようになったら`KanjiToggle`を足すか判断」と書いており、`KanjiToggle`が到達するのは
`ImmCross × MsIme`の1組だけと明記していた——今回のバグはその1組そのもの。

## 決定

1. **`ImmCross × MsIme`(Standard/Plain/Unknown)のチェーンを`[ImmCross, MsImeDirect]`にする**(`CHAIN_IMM_CROSS_THEN_KANJI`を置き換え)。
2. **`MsImeDirectStrategy::is_applicable`(`ms_ime_direct_applicable`)を`ImeKindId::MsIme`だけで判定する**(`!can_use_imm32_cross_process()`を外す)。
   非同期チェーン(`run_open_chain_async`→`fallback_write`)は機構ごとに`is_applicable`を再評価するので、述語側の変更が必要。
   他の呼び出し元は`runtime/transport.rs:386`だけで、そこは`can_use_imm32_cross_process()`が真の腕を先に処理する`else`内のため、判定結果は変わらない。
3. **`KanjiToggle`は削除しない**(今回は到達不能にするだけ)。`ActiveImeKind`はGJI非検出時の**推定**値で、MS-IME以外/互換IMEに`VK_IME_ON/OFF`が
   効く保証は無い。到達しなくなったことをCI/実機で確認してから、`KanjiToggleStrategy`とその構造(`WriteMechanism::KanjiToggle`)の削除を別ADRで扱う
   (`.claude/rules/complexity-budget.md`の1-in-1-outは未発効だが、削除の方向は望ましい)。
4. 変更しないもの: `imm_cross_write`の`None`=`Failed`の扱い(不明を「開いていない」と決めつける点)。`MsImeDirect`は冪等なので、不明でも誤って逆転させない。
   `AlreadyMatched`の判定条件は触らない。

## 検討して採らなかった案

- **a8: `KanjiToggle`を送らない(`None`のときは何もしない)。** 止血にはなる(実験でALL PASS)が、ImmCrossが失敗したとき開く/閉じる手段が
  何も残らず、IMEを確実には操作できない。冪等キーが使える以上、送らない理由が無い。
- **`imm_cross_write`の`None`を`Failed`と区別する新しいoutcomeを足す。** 型・分岐が増える(ADR-184の教訓: 敵対的指摘に型/フィールドで答えると複雑化する)。
  冪等キーへ替えれば`None`の扱いを変えずに解決する。
- **ImmCrossのタイムアウトを延ばす。** CIランナーの遅さへの対症で、`docs/adr/`の「定数を実測なしに釣り上げない」規約(tuning-constants)に反する。
  本当の問題は「失敗を非冪等キーで補う」ことにある。

## 影響範囲(再発ファミリー)

`fix-requires-evidence.md`の「キー選択」「IME actuation合流点」ファミリー(`ime_controller.rs`、`runtime/open_chain.rs`、`state/app_ime_policy.rs`)。
同期チェーン(`ime_controller.rs::apply` → `run_chain`)と非同期チェーン(`fallback_write`)の**両方**が同じ`is_applicable`/チェーン定義を見るので、片方だけの
修正にならないよう、実装時に両経路のgoldenを揃える。

## 検証計画

- 回帰テスト: `tests/ime_key_sequence_golden.rs`の`characterize_strategy`に`ImmCross × MsIme`の失敗後が`MsImeDirect`(`VK_IME_ON/OFF`)になる期待値を追加。
  `key_sequence_policy.rs`の`ms_ime_direct_applicable`のテスト(ImmCrossプロファイル×MsImeが真になる)を更新。`app_ime_policy.rs`のチェーン表のテストを更新。
- CI実機E2E: `sc-dbe/kanji/shift-msime-native`を`observe`から`pass`へ固定(a9相当が本体に入ったこと)。`sc-kanji`のプローブ判定窓(`check_consistency.engine_after`の+2500ms)は
  CIの遅延で超えることがあるので、別途広げる。
- 実機(dragonflyg4、Microsoft IME): ImmCrossが失敗する状況(cold直後のフォーカス等)でひらがな/英数キーを押し、実IMEとEngineが一致するか。
  `MsImeDirect`が`VK_IME_ON`を送ったとき、既に開いているIMEのconv(カタカナ等)が変わらないか。

## 残る限界

- MS-IME本体の半角/全角(0xF3/0xF4)は、awaseが静的モデル(F3=OFF、F4=ON)のままなので、同じVKの連続で反転しない(ADR-189が対象外にした範囲、別件)。
- Microsoft IME以外(ATOK等のTSF互換IME)が`ImeKindId::MsIme`と推定された場合に`VK_IME_ON/OFF`が効くかは未確認(`KanjiToggle`を残す理由)。
