---
id: ADR-255-companion-255-opus-review-round3
title: |-
  ADR-255（IME OFF のときだけ無変換/変換を Space にする）Opus敵対的レビュー round3
type: companion-doc
related_adr:
  - "ADR-255"
---

# ADR-255 敵対的レビュー round3(Opus)

対象: `docs/adr/255-ime-off-thumb-key-space.md`(ワークツリー keymap-ime、HEAD `cf1c5166`)。行番号はこの HEAD のもの。

## 総評

**まだ収束していない。新規の Blocker 1・Must 2。**

- 中心の問題は決定2-4。「4 源すべて None なら IME 側に開閉の役割が無い」とみなしているが、この前提が実コードと合わない。awase の「役割」(`KeyRole`)は **トグルだけ**を表す型で、**開くだけの割り当て**(GJI の「変換/無変換で IME ON/OFF」のオーバーレイ、CUSTOM 表の `DirectInput,Henkan,IMEOn` 単独行、MS-IME の「変換 = IME-オン」)では None を返す。結果として、ADR が「守る」と書いている CI の `ctl-loaded` 構成そのものが守られない(R3-B1)。
- 決定3(TsfNative 非対応)は、決定としては正しい。ただし対象範囲が想定よりずっと狭い。`cannot_verify_real_ime_state` は Chrome/Edge/UWP/コンソール(Imm32Unavailable)・TsfNative・InputRelay と、Standard でも実質 TSF ネイティブの窓をすべて含む。報告者のアプリがここに入る見込みは高いので、**報告者のアプリの確認を実装着手の条件(ゲート)にする**こと(R3-M1)。
- CI の入力先(ADR-193 の RichEdit スーパークラス化は「TsfNative 相当」)では、決定3 によって正の検証(2a)が原理的に発動しない可能性がある(R3-M2)。

---

## r2 対応の確認

| ID | 判定 | 根拠 |
| --- | --- | --- |
| R2-B1 | 決定は妥当。範囲の書き方が不足 | R3-M1 |
| R2-M1 | **満たさない** | R3-B1。4 源の網羅は「トグルの役割」の網羅で、「IME が開く」の網羅ではない |
| R2-M2 | 方針は満たす。判定の手段に注意 | R3-S1(scan のハードコードより、フックが印を付ける方が安全) |
| R2-M3 | 満たす | 2-5(a)。そもそも InputRelay は `cannot_verify_real_ime_state` に含まれる(`class_names.rs:284`)ので、決定3 と重複して安全側 |
| R2-M4 | 満たす | 末尾配置・`!was_down`・`matches_ime_set_open`/`matches_ime_off` の網羅 match・VK を Platform から渡す |
| R2-M5 | 満たす | 影響範囲の節 |
| R2-S1 | 満たす。補足は R3-S2 | `transition_activation` は `prev_activation` を更新するので、次の打鍵での二重発行は無い(`engine.rs:411-440`) |
| R2-S2 | 満たす | 2-5(b) |
| R2-S3 | 満たす | 計画2(c) を Win32 の窓で確かめる。ただし R3-M2 |
| R2-S4 | 満たす | 既存の役割経路と同じにする。実装前に調べる |
| R2-S5 | ほぼ満たす | R3-M2・R3-S3 |
| R2-S6 | 満たす | `thumb_key_when_ime_off = "unchanged" \| "space"` |
| R2-S7 | 満たす | |

---

## Blocker

### R3-B1. 決定2-4 の 4 源は「トグル」しか見ない。開くだけの割り当てを持つ人から、IME を開く手段を奪う

実コードの裏取り:

1. **`KeyRole` の variant は `ImeToggle` の 1 つだけ**(`crates/awase-gji-config/src/role.rs:28-31`)。CUSTOM 表の判定 `custom_table_has_toggle`(:153-162)は、「閉で開く」**かつ**「すべての開状態で閉じる」**かつ** ON/OFF 行が効く、のときだけ真になる。
   - CI の `ctl-loaded` 構成(`DirectInput,Muhenkan,IMEOn` / `DirectInput,Henkan,IMEOn`。開状態で閉じる行は無い)は**トグルではない**ので `None`。ADR は 2-4 の説明で「CI の `ctl-loaded` 構成がこのケース」と書き、守れる前提にしているが、実際は 4 源とも None になり、Space で上書きされる。
2. **GJI の「変換/無変換で IME ON/OFF」オーバーレイ**: `key_role`(:135-141)は `SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF` があると、`VK_CONVERT`/`VK_NONCONVERT` で**早期に `None`** を返す。GJI 利用者がよく使う設定項目で、直接入力の変換で IME が開く。2-4 は全源 None → Space → 変換で IME が開かなくなる。
3. **MS-IME 本体**: `msime_native_key_role`(`key_effect_predictor.rs:874-895` の doc)は、無変換/変換について「マスタースイッチ有効かつ値==2(トグル)のときだけ `Some(ImeToggle)`。値0/1/3・値なしは受動」。MS-IME の「キーとタッチのカスタマイズ」で「変換 = IME-オン」等にした人は None → Space で上書きされる。
4. **不明と「無い」の区別が無い**: `key_role` は `Source::Unknown`(表が読めない)で false、`thumb_forced_action`(`key_effect_runtime.rs:422-434`)は `!ime_identified`(TIP 未同定。BUG-179 のように MS-IME を Other と同定する事例がある)で役割を引かない。どちらも 2-4 では「役割なし」と読まれ、発動する。決定2-5 の原則「不確かなら素通し」と矛盾する。

決定5 の注記「IME 側の割り当てより優先される」は 3・4 の一部を「仕様」として受け入れている。しかし 1・2 は**awase が既に読んでいる表**から分かる事実なので、「知らないから上書きする」の範囲に入らない。

修正案(2-4 を置き換える):

- 新しい判定「このキーは**直接入力状態で IME を開く/IME の機能を持つ**か」を三値で作る: `Opens` / `NoEffect` / `Unknown`。**発動は `NoEffect` のときだけ**にする。
  - GJI: プリセットまたは CUSTOM 表の `DirectInput` 行にそのキーの行があれば `Opens`(または「機能あり」)。`KeyStates::opens_when_closed` 相当が既にある(role.rs:158)。オーバーレイがあれば `Opens`。行が無ければ `NoEffect`。表が読めなければ `Unknown`。
  - MS-IME 本体: マスタースイッチと値から、値ごとの効果(IME-オン/オフ/トグル/なし)を写像する。値の意味を確かめていない段階では `Unknown`。
  - TIP 未同定・`table_ime_kind` が None: `Unknown`。
  - 加えて、従来の 4 源(`keys.ime_*`・`thumb_role_open_actions`・専用 Fn キー・`sync_direction`)のいずれかがあれば発動しない(これは残す)。
- `DirectInput` 行の IMEOn 以外のコマンド(例: `Reconvert`、`InputModeHiragana` 等)も「機能あり」に数えるかを決める。数えないなら、その機能を Space で奪うことを注記に書く。
- 単体テストに、`ctl-loaded` の表・オーバーレイ・MS-IME の値 0〜3・表なし・TIP 未同定の各行を入れる。

---

## Must

### R3-M1. 決定3 の実効範囲を明記し、報告者のアプリの確認を実装の着手条件にする

- `cannot_verify_real_ime_state`(`crates/awase-windows-core/src/focus/class_names.rs:277-286`)は、`Imm32Unavailable`(Chrome/Edge/UWP/XAML/コンソール系、:109-129)・`TsfNative`(WezTerm/Windows Terminal)・`InputRelay`・Standard でも `is_effectively_tsf_native` な窓、で真になる。決定3 を「TsfNative 等」と書くと読み手は WezTerm 程度を想像するが、実際に発動するのは **IMM で状態を読める古典的な Win32 の窓だけ**になる(Windows 11 のメモ帳など XAML/RichEdit 系がどちらに分類されるかは未確認)。
- 決定3 の見出しと設定画面の注記を「Chrome・Edge・VS Code・Windows Terminal・UWP など(IME の状態を awase が読めないアプリ)では動かない」に直す。どのプロファイルが対象かを、実際の述語名(`cannot_verify_real_ime_state`)で書く。疑問1 は「述語は `cannot_verify_real_ime_state` を使う」で閉じられる。これ以外の述語を使うと、InputRelay や Standard+TSF の窓を取りこぼす。
- 報告への扱い: 報告者のアプリがこの範囲に入るなら、この ADR は報告を直さない。r1 S4 と同じ理由(消費者のいない機能を足さない)で、**計画0 の「報告者のアプリのプロファイル確認」を実装着手のゲート**にし、ADR に次のどちらかを書くこと。
  - (a) Standard の窓なら実装する。
  - (b) 対象外の窓なら実装を保留し、報告者には「GJI の直接入力状態の制約(仮説 a〜c の切り分け結果)」と「awase では現状できない理由」を返す。鮮度を問わない根拠の別 ADR を起こすかは、そこで判断する。
- 報告者への確認事項は 3 つにまとめる: (1) どのアプリで使うか、(2) タスクバーの表示が「A」(直接入力/半角英数)のどちらか、(3) 押し続けて Space が連続して出る必要があるか。今の ADR は (2)・(3) を別々の節に散らしていて、(1) は計画0 に埋もれている。

### R3-M2. CI の正の検証(計画2a)が、入力先の分類次第で原理的に発動しない

- ADR-193 で採用した CI の決定的な入力先は「RichEdit スーパークラス化で TsfNative 相当」。この窓が `cannot_verify_real_ime_state` で真になるなら、決定3 によって計画2 の (a)「IME OFF の無変換で空白が 1 つ入る」は**仕様どおり入らない**。「機能が効かない」のか「入力先が対象外」なのかが区別できない。
- 計画2 に、入力先の窓として **Standard に分類される IMM の窓**(素の Edit コントロール等)を明記し、まずその窓の `AppImeProfile` と `cannot_verify_real_ime_state` の値をログで確かめる手順を入れる。対照として、TsfNative 相当の窓で「入らない」(決定3 の確認)も 1 構成足す。
- 同じ理由で、計画2(c) の「composition 中は入らない」も Standard の窓で確かめる必要がある(R2-S3 で述べたとおり)。

---

## Should

### R3-S1. なりすまし由来の判定は scan のハードコードではなく、フックが印を付ける

- 決定2-3 の「scan code(Alt 0x38 / 無変換 0x7B)で判定」には 3 つ問題がある。右 Alt のなりすましは変換(scan 0x79)になるが、0x79 が書かれていない。Scancode Map(ADR-230)や Interception 系のリマッパで scan が変わる。scan を許可リストにするか拒否リストにするかで結果が変わる。
- フックは `apply_alt_impersonation`(`hook.rs:1762`)で書き換えたかを知っているので、`RawKeyEvent` に `impersonated: bool` を足して運ぶ方が確実。ADR では「印を足すか scan で」と両論併記になっているので、印に決めること。印は journal の境界(ADR-250)にも乗る。

### R3-S2. 前置する遷移の effects の中身を確認する

- `transition_activation`(`engine.rs:411-440`)は、活性→非活性で `SetOpen{open:false}`(`emit_set_open` が真のとき)と `EngineStateChanged{false}` を出す。Space 経路で前置すると、IME が既に閉じている窓に `SetOpen(false)` が 1 回届く(冪等なはずだが actuation の 1 件)。`check_active_transition` がどの `emit_set_open` で呼ばれるかを確かめ、actuation の合流点の規約(`fix-requires-evidence.md` の IME actuation 合流点ファミリー)に触れるかを ADR に一行書く。触れるなら、押下 ID(ADR-208 L1)を付けない `press=None` で良いかも書く。

### R3-S3. 検証計画の補足

1. 計画1 の表に R3-B1 の軸を足す: GJI のプリセット/CUSTOM の `DirectInput` 行の有無・オーバーレイ・MS-IME の値 0〜3・表なし・TIP 未同定。
2. 計画2(c) の負の対照に足す: GJI の「変換/無変換で IME ON/OFF」オーバーレイ構成で、変換が IME を開き Space にならないこと。
3. 計画0 の「割合を数える」は、Standard の窓では観測が 500ms 周期で入るのでほぼ 100% のはず。数える意味があるのは TsfNative ではなく Standard の窓で `Unknown` が出る条件(フォーカス直後・プローブ失敗)の頻度。計測の目的を「決定3 の範囲で、発動すべき場面で発動するか」に直す。

### R3-S4. 事実関係

- 決定3 の「Chrome・VS Code・WezTerm」: コードでは Chrome は `Imm32Unavailable`、WezTerm/Windows Terminal は `TsfNative` に分類される(class_names.rs:109-113 の doc)。どちらも `cannot_verify_real_ime_state` が真になるので結論は同じだが、ADR に「TsfNative のプロファイル」と書くと Chrome が抜けて読める。R3-M1 の言い換えで直る。
- 計画0 の「基準構成の……は成果物で確認済み(各 n=1)」: r2 の時点では未確認と書いた点を ADR 側で確認したのなら、run ID と、確認した KEY 行(またはジョブ名)を添えること(`agent-handoff.md` 3 の書き方)。

---

## 確認に使ったコマンド(HEAD `cf1c5166`)

- `sed -n 28,31p; 126,162p crates/awase-gji-config/src/role.rs`(`KeyRole` はトグルのみ、オーバーレイで早期 None、CUSTOM のトグル判定)
- `sed -n 855,895p crates/awase-windows-core/src/state/key_effect_predictor.rs`(MS-IME 本体は値==2 のときだけ役割)
- `sed -n 422,434p crates/awase-windows-core/src/state/key_effect_runtime.rs`(TIP 未同定で役割を引かない)
- `sed -n 796,836p crates/awase-windows/src/runtime/mod.rs`(`enrich_thumb_key_role` が打鍵ごとに `thumb_role_open_actions` を書く)
- `sed -n 105,130p; 277,286p crates/awase-windows-core/src/focus/class_names.rs`(プロファイルと `cannot_verify_real_ime_state`)
- 未確認: CI の入力先の窓が `cannot_verify_real_ime_state` で真になるか(R3-M2)。MS-IME の値 0/1/3 の意味。Windows 11 のメモ帳の分類。
