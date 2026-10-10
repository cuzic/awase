---
id: ADR-255-companion-255-opus-review-round7
title: |-
  ADR-255（IME OFF のときだけ無変換/変換を Space にする）Opus敵対的レビュー round7
type: companion-doc
related_adr:
  - "ADR-255"
---

# ADR-255 敵対的レビュー round7(Opus)

対象: `docs/adr/255-ime-off-thumb-key-space.md`(ブランチ `docs/adr255-late-keymap`、HEAD `57d6c862`)。行番号はこの HEAD のもの。

## 総評

照合の位置(`engine.on_input` の直後、素通しのときだけ)は良い選択。r1 の B1 を、A1 と同じく「エンジン自身の判断を見る」形で閉じている。`kp_stage_post_decision` 以降の段も、消費に変わった `Decision` を正しく扱う(Q1)。

ただし **新規の Must が 3 件**ある。

1. **Space を `send_keymap_target` で直接送ると、executor のキューに並んでいる先行の出力を追い越す**(R7-M1)。テキストの空白を作る用途では、文字順が入れ替わる実害になる。
2. **遅いルールの照合に `!event.was_down` が無い**(R7-M2)。リピート Down に当たると、Down は OS に届き Up だけを latch が飲む。r2 R2-M4 が形を変えて再び開いている。
3. **遅いルールの `from` に許すキーの範囲が未定義**(R7-M3)。条件6(`KeyDirectInputEffect`)は無変換/変換にしか定義されていない。

いずれも ADR の記述の修正で閉じる。**3 件を直すまでは docs としてもマージを勧めない**。直した後は再レビューなしでマージしてよい(どれも局所的な仕様の追記で、他の決定に波及しない)。

---

## Q1. 照合の位置と、前後の段

### エンジンの前の段

`kp_run_inner`(`key_pipeline.rs:58-150`)の前段で、IME の機能が無い無変換(条件6 を満たすもの)に対して状態を書く箇所を確かめた。

- `enrich_thumb_key_role` は役割(この場合 None)を押した側に書くだけ。
- `kp_stage_shadow_ime_toggle` は、無変換/変換の開閉は通らないとコメントにある(:978-981 付近)。`shadow_action`/`sync_direction` が無ければ意図を作らない。
- `settle_fkey_role_latch` は F13〜F24 用。
- `kp_stage_idle_conv_check`・`kp_stage_focus_probe` は観測で、この打鍵に固有の書き込みは無い。
- `try_hold_key`(TsfGate)は保留して後で同じ `kp_run_inner` を通る。

主張は成り立つ。決定2(c) の「要確認」は、上の確認で閉じてよい。

### エンジンの後の段

- `kp_stage_post_decision`(:1433-)は、`may_change_ime` の refresh・`kp_arm_external_change_watch`(:1549-1555)を `!decision.is_consumed()` のときだけ行う。`kp_stage_mode_key_follow`(:1646-1659)も `decision.is_consumed()` で早期 return する。消費に格上げした後なら、ModeKeyPass の誤装填も refresh も起きない。
- **前提は、格上げを `kp_stage_post_decision` より前に行うこと**。ADR はそう書いている。
- `PhysicalKeyDisposition::plan`(:240 付近)は格上げの前に計算され、無変換は Allow になる。`execute_relay` の Consume アーム(`executor.rs:414-438`)は `physical` を見ないので、二重配送は起きない。
- `kp_latch_keyup_to_keydown_disposition` は無変換/変換を `excluded` にしている。

### journal

- `DecisionKind::from_decision(&decision)` は格上げ後の値を記録する(格上げを journal の記録より前に置く前提)。`physical` は Allow のまま残るが、doc のとおり「`plan()` の判定値であり、実際に届いたかではない」。矛盾はしない。
- ただし R7-M1 の修正をしないと、**送った Space が journal・境界 journal(ADR-250)のどこにも現れない**(`send_keymap_target` は executor を通らない直接の SendInput)。

## Q2. KeyLifecycle と PassThroughWith の effects

- 主張は成り立つ。非活性の Phase 2(`engine.rs:510-522`)は `pass_through()`/`pass_through_with(transition_effects)` を返す。`lifecycle.on_key_down_consumed` も `phase1_held` も呼ばない。`InputTracker::process` は FSM の `on_event` 内(`fsm_adapter.rs`)でしか呼ばれず、非活性では走らない。latch が KeyUp を先に飲んでも、エンジンに宙に浮く記録は残らない。
- `force_consume`(`src/engine/decision.rs:182-187`)は `std::mem::take(self.effects_mut())` で effects を移す。`check_active_transition` の flush・`release_pending_and_reinject`・`EngineStateChanged` は保たれる。
- **ただし実行の順序に問題がある**(R7-M1)。保たれた effects は Consume アームで executor のキューに入り(`executor.rs:423-433`、Timer 以外は Queue)、`WM_EXECUTE_EFFECTS` で後から実行される。ADR の手順では Space を `send_keymap_target` でその場で送るので、Space が先、flush した保留出力が後になる。

## Q3. r1 M3・M4 の手当

- **M3(親指ラッチ)**: 成り立つ。フックの親指ラッチ(`hook.rs:1808-1830`)は Down で立ち、同じ物理キーの Up で解ける。latch が Up を飲むのは `deliver_key_event` の段で、フックの段の後なので、親指ラッチの解除は妨げない。押している間に活性化したときの窓は、ADR のとおり役割経路と同じ扱いで固定すればよい。
- **M4(古い latch)**: `was_down` を見る案は成り立つ。フックは overflow で素通しする前に `physical_key_state` を `swap` している(:1561 は :1735 より前)ので、取り逃した Up の次の Down は `was_down=false` になる。セッションのロック中に失われた Up は、アンロック時の `reset_physical_key_state` と `release_all` で処理される(`message_handlers.rs:1045-1056`)。
- **実装の前提**: `KeymapLatch`(`state/keymap_latch.rs`)は vk だけの `Vec<VkCode>` で、どのルール(早い/遅い)で積んだかを持たない。「遅いルールの latch だけ `was_down` で捨てる」には、latch に種類を持たせる必要がある。あるいは、早いルールにも同じ扱いを広げる(その方が単純で、既存の早いルールの stale latch も直る)。どちらにするかを決定4 に書くこと(S1)。
- 捨てた後は「再照合」ではなく、**ステップ1 から通常の流れへ落とす**のが正しい。遅いルールの照合は `kp_run_inner` 内で行われるので、ステップ1 で再照合すると、エンジンを通さずに遅いルールを当てることになり、B1 が再発する。ADR の「latch を捨てて再照合する」の文言を「latch を捨て、通常の流れ(エンジンの判断→遅いルール)に渡す」に直すこと。

## Q4. `from` の禁止の緩和と `find_match`

- 早い/遅いを `KeymapTable::new` で別の集合に分ければ、`find_match`(`keymap.rs:190-201`)の呼び出し元(`consume_keymap_match` の 1 箇所)は変えずに済む。ADR の「コンパイル: 遅いルールを分ける」はその形で読める。
- `warn_if_vk_conflicts`(:217 付近)と `filter_active`(:170)は、両方の集合に対して呼ぶこと。片方だけだと、遅いルールの衝突警告と `app` の絞り込みが抜ける。
- **修飾付きの `from`**: 緩和は「主キーに親指キー」だけで、`from = "Ctrl+VK_NONCONVERT"`・`ime = "off"` は書ける。条件5 で非注入しか見ていないので、Ctrl を押したまま当たると、Ctrl+Space(MS-IME 等の IME トグル)が OS に届く。R7-M1 の修正で送信を effects にすると、`send_keymap_target` の修飾の扱い(ADR-130)も失う。**遅いルールの `from` は無修飾に限る**(コンパイル時に警告して skip)を R7-M3 と合わせて書くこと。

## Q5. 閉じた指摘のうち再び開くもの

| ID | 判定 |
| --- | --- |
| r1 B1 | 閉じたまま。エンジン自身の Decision を見る |
| r1 B2 | 閉じたまま。条件8 で composition があれば当たらず、`cancel_composition` を呼ばない。TSF での有効範囲が未検証な点は ADR に明記済み |
| r1 M3 | 閉じたまま(Q3) |
| r1 M4 | 方針は閉じた。実装の前提(latch の種類、再照合の文言)が S1 と Q3 |
| r1 M5 | 閉じたまま(条件5) |
| R2-S1 | **形を変えて再び開く**(R7-M1)。遷移の effects は保たれるが、Space がそれを追い越す |
| R2-M4 | **再び開く**(R7-M2)。リピートの扱い |
| R3-S2 | 閉じたまま。`check_active_transition` は `emit_set_open=false` |

決定8(案 A1 への引き返し条件)は妥当。引き返しの条件に「(4) 送信を effects に載せたとき、`send_keymap_target` の修飾の扱いや出力層の warm/cold 判断との整合が重い」を足すとよい(R7-M1 の修正の重さが、K と A1 の比較で最も効く点)。

---

## Must

### R7-M1. Space は Decision の effects に載せる(`send_keymap_target` で直接送らない)

- 遅いルールが当たる打鍵は、エンジンが素通しを返した打鍵。直前の文字も素通し(`ReinjectKey`)で、**executor のキューに並んでいることがある**。relay の PassThroughWith は「flush 出力 + キー再注入を FIFO」(`executor.rs:343-349` のコメント、:395-412)、Consume の effects は Queue(:423-433)。
- `send_keymap_target` はその場で SendInput する(`message_handlers.rs:299-306` と同じ呼び方)。キューが空でないとき(OUTPUT_GATE・Park・drain 中・遷移の flush)、Space が先行の文字を追い越す。「foo bar」と速く打つと「fo obar」になりうる。既存の早いルール(ショートカット用途)では目立たなかったが、空白を打つ用途では文字順の入れ替わりがそのまま実害になる。
- 修正: `force_consume` の後、`decision.push_effect(Effect::Input(InputEffect::SendKeys(...)))` で `to` を effects の末尾に積む。遷移の effects・先行のキューの後に、FIFO で実行される。
  - journal(`DecisionKind`)にも送信が残る。
  - VK は Platform が持つ値を使う(ADR-019。エンジンではなくシェルで積むので、シェルの `crate::vk::VK_SPACE` でよい)。
  - 修飾の扱いは R7-M3 で無修飾に限れば要らない。
- 出力層(`output/vk_send.rs`)が `SendKeys` を IME ON 前提の warm/cold 判断(`state/warm_send_plan.rs`)に通すかは、実装時に確かめること。エンジンの Space 親指フォールバック(`ThumbRawVkEmission`)が生の機能 VK を effects で送る前例があるので、同じ effect の型を使うのが安全。決定2 と決定4 の「`send_keymap_target` で送る」を書き換え、決定8 の引き返し条件に足す。

### R7-M2. 遅いルールの照合は `!event.was_down` のときだけ

- 失敗シナリオ:
  1. 無変換の最初の Down は条件8(composition あり)や条件1(belief が ON)で当たらず、素通しで IME/OS に届く。
  2. 押したまま状態が変わり(composition が消える、belief が OFF になる)、次の自動リピート Down(`was_down=true`)で遅いルールが当たる。
  3. Space を送り、latch に積む。物理 Up は latch が飲む(`deliver_key_event` ステップ1)。
  4. **OS には無変換の Down だけが届き、Up が届かない**。OS 側で無変換が押されたままになる(BUG-131/132 と同じ Down/Up 非対称)。
- r2 R2-M4 で A1 に付けた `!was_down` が、案 K の発動条件(条件1〜8)から抜けている。条件に「9. `!event.was_down`(リピートの Down には当たらない。最初の Down が素通しなら、そのキーの Up まで素通しのまま)」を足し、回帰テスト(物理キー押下ラッチ・ファミリー)に入れる。

### R7-M3. 遅いルールの `from` の範囲を決める

- 決定1 の構文は任意のキーに `ime = "off"` を書ける。しかし条件6(IME の機能を持たないこと)の 4 源と `KeyDirectInputEffect` は、**無変換/変換にしか定義されていない**(評価場所も `enrich_thumb_key_role`、無変換/変換の Down だけ)。
  - `from = "VK_F13"`・`ime = "off"` のようなルールは、条件6 が評価できず `Unknown` で常に発動しない(黙って効かない)か、実装しだいで評価を素通りして IME の機能(例: MS-IME/MOBILE プリセットの `DirectInput\tF13\tIMEOn`)を奪う。
- どちらかを決定1・決定5 に書く。
  - (a) **遅いルールの `from` は、無修飾の親指キー(無変換/変換)に限る**(それ以外は警告して skip)。推奨。今回の消費者はこれだけ。
  - (b) 任意のキーに広げ、条件6 を全キーに定義する(`KeyStates::of` は任意のキー名を読めるが、4 源は無変換/変換専用なので別の整理が要る)。
- (a) なら、Q4 の「修飾付きの `from`」も同時に閉じる。

---

## Should

### S1. latch の種類と「再照合」の文言

Q3 のとおり。`KeymapLatch` に早い/遅いの区別を持たせるか、`was_down` の扱いを早いルールにも広げるかを決める。stale latch を捨てた後は、通常の流れに落とす(ステップ1 では再照合しない)。

### S2. 遅いルールの発動の記録

R7-M1 で effects に載せれば `DecisionKind` に残る。加えて、なぜ消費したか(遅いルールが当たった)を区別できるように、境界 journal(ADR-250)か debug ログに「late keymap」の印を残す。そうしないと、journal 上は「非活性のエンジンが無変換を Consume した」と読めてしまい、エンジン側の不具合と取り違える。

### S3. `filter_active`・`warn_if_vk_conflicts` は両方の集合に

Q4 のとおり。影響範囲に一行足す。

### S4. 条件3(`ctx.is_japanese_ime`)は条件1 に含まれる

`compute_state`(`engine.rs:269-283`)は `UserDisabled` → `NotJapaneseIme` → `ImeOff` の順に判定するので、`Inactive(ImeOff)` なら日本語 IME で、エンジンも有効。条件3 は冗長(害は無い)。消すか「条件1 に含まれる」と注記すると、読み手が別の判定だと誤解しない。

---

## マージの判断

- 新規の Must 3 件(R7-M1〜M3)。**直すまで docs としてのマージは勧めない**(実装者がそのまま `send_keymap_target` を使う指示を読む危険がある)。
- 3 件はどれも ADR の数行の追記・書き換えで閉じる。直した後は、Should を残したまま docs としてマージしてよい。実装は従来どおり決定3b のゲートで保留。

---

## 確認に使ったコマンド(HEAD `57d6c862`)

- `sed -n 150,330p crates/awase-windows/src/runtime/key_pipeline.rs`(on_input の後の journal・post_decision・execute の順序)
- `sed -n 1433,1560p; 1646,1690p crates/awase-windows/src/runtime/key_pipeline.rs`(`is_consumed` での分岐)
- `sed -n 170,190p src/engine/decision.rs`(`force_consume`)
- `sed -n 405,440p crates/awase-windows/src/runtime/executor.rs`(Consume の effects はキューへ)
- `sed -n 460,546p src/engine/engine.rs`(非活性の Phase 2 は lifecycle を触らない)
- `grep -rn "\.process(" src/engine/*.rs`(InputTracker は FSM の中だけ)
- 未確認: `output/vk_send.rs` が `SendKeys` の VK_SPACE を IME OFF でどう扱うか(R7-M1 の実装時)。
