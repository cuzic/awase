---
id: ADR-249
title: |-
  他アプリが注入した Ctrl を、注入キー自身の修飾として期限付きで数える(BUG-197、Spokenly の Ctrl+V が「ふ」になる)
summary: |-
  音声入力ソフト Spokenly は Ctrl↓→V↓↑→(約100ms 後)Ctrl↑ を SendInput で注入して貼り付ける。awase は物理押下状態(PHYSICAL_KEY_STATE)を
  非注入イベントだけで更新する(ADR-054、X サーバーの Ctrl KeyUp 欠落による stuck 対策)ため、注入 Ctrl を修飾として数えず、
  V を ctrl=false の Char として NICOLA 変換して「ふ」を出す。注入された Ctrl↓ を別枠に記録し、**注入された打鍵の modifier_snapshot にだけ**
  期限(TTL)内の ctrl を足す(案 A')。物理打鍵は別枠を読まないので、KeyUp 欠落でも ADR-054 の stuck は TTL の値と無関係に再発しない。
status: |-
  起草・改訂(2026-10-10)。Opus round1(13指摘)・round2(Should-fix 3件・Nit 4件)を反映済み、Opus round3 で収束(Blocker なし、実装に着手してよい)。実装済み(PR #580、2026-10-10)。TTL は `FOREIGN_CTRL_TTL_MS`=1000ms(pending、Spokenly の保持は報告 journal の2例で 101ms の約10倍、他ツールは未測定)。CI の実機 A/B は paste が全構成 36/36(run 38041351075)。報告 journal の replay fixture は ADR-251 で実装。未了: 「注入 Ctrl↓ だけで Up なし」後の stuck 非再発の CI 観測、0x11 の到達確認、報告者の実機確認。
related_adr:
  - "ADR-054"
  - "ADR-052"
---

# ADR-249: 他アプリが注入した Ctrl を、注入キー自身の修飾として期限付きで数える

## 背景

[BUG-197](../known-bugs/BUG-197.md)。報告 `01M4J72G985T0FFT6XPN0SWCQQ`(GJI、awase 2.0.0)で、Spokenly の音声入力を使うと
「ふ」1文字だけが入る(awase を止める・やまぶきR なら正常)。

### 事実(報告の journal・CI のログ・コードで裏取り。Opus round1 で確認済みの項目に印)

1. Spokenly の貼り付けは注入キー(`injected=True`)の列 `左Ctrl↓(scan 29) → V↓(scan 47) → V↑ → 左Ctrl↑`。Ctrl↓→Ctrl↑ は 101.1ms と 101.6ms(2例、
   journal seq 61→64、117→120)。V↓ は Ctrl↓ の 0.44ms 後・0.54ms 後(フック時刻)。(確認済み)
2. V↓ は `state_before=Idle → PendingChar(0x56)`、Consume(effect 3件)。NICOLA 配列で V は「ふ」。(確認済み。「3件」は V↓ の値で、V↑ も別に3件)
3. 診断 `[engine-input]`(CI の注入 Ctrl+V)は V↓ で `mods(c=false …) gas_ctrl=true phys_ctrl=false`。**ただし `gas_ctrl` はエンジンが処理した時点の値**
   (V の行は `delay=88ms`)で、フックは他アプリの注入も含めて一度飲み込み(`hook.rs` の `Accepted => LRESULT(1)`)、エンジンが PassThrough と判定した Ctrl↓ を
   `INJECTED_MARKER` 付きで再注入する(`lib.rs` の `RawKeyEventExt::reinject`)。つまり `gas_ctrl=true` は **awase 自身の再注入を反映した値**で、
   OS が他アプリの注入を独立に認識した証拠ではない。フック時点で読むと、V↓ が Ctrl↓ の約0.5ms 後なので、再注入が済んでおらず false になりうる(競合)。
4. 修飾の取得(`observer/focus_observer.rs::read_os_modifiers`)は、**Ctrl・Shift = `PHYSICAL_KEY_STATE`(注入を除外)、Alt = `GetAsyncKeyState` ∨ `LLKHF_ALTDOWN`
   (Alt なりすまし中は偽、`hook.rs`)、Win = `GetAsyncKeyState`**。ADR-054 の2つの問題は別コミット: 問題1(awase 自身の synthetic Ctrl↑ による
   `GetAsyncKeyState` の汚染)は `11817d1`/`effb47d`/`261f400`、問題2(X サーバーが注入した Ctrl の KeyUp 欠落による stuck)は `8351bcf9`。
   注入 Alt/Win も同じく飲み込まれてから再注入されるため、フック時点の競合は潜在的に同系統(未確認、本 ADR の対象外)。
5. 注入された V も物理キーと同じ `produce` 経路を通る。`if !is_injected` で飛ばされるのは `physical_key_state` の更新と親指ラッチだけで、
   `modifier_snapshot` の作成(`hook.rs` の `read_os_modifiers()` 直後)と `build_raw_key_event(…, is_injected, …)` は注入でも実行される。
   エンジンの `bypass_reason` は `Passthrough` → `ImeControl` → `is_os_modifier_held()`(= ctrl ∨ alt ∨ win、**Shift を含まない**)の順に見るので、
   `ctx.modifiers.ctrl=true` で V が届けば `OsModifierHeld` で素通しになる。打鍵の ctx は hook 時点の `event.modifier_snapshot` から組む
   (`key_pipeline.rs`、ADR-129。INPUT_DEFER の再生でも値は変わらない)。(確認済み。`runtime/mod.rs` の「bypass_reason が見るのは build_ctx」というコメントは古く、根拠にしない)
6. 報告では、両方の貼り付けの約 0.77〜0.91 秒前(28944→29858ms、53808→54579ms)に**物理の右 Ctrl の長押し**がある(Spokenly の押して話すホットキーと思われる、未確認)。貼り付けの時点では離れていた。
   ホットキーを押したまま貼り付けが走る設定では物理 Ctrl が重なるので、現状でも成立しうる。報告者の設定によって症状が出る/出ないが分かれる可能性がある。

### 制約

- ADR-054 の2つの問題(awase 自身の synthetic Ctrl↑ による汚染、他アプリの注入 Ctrl の KeyUp 欠落)を再発させない。
- 注入キーの通り道は他にもある。リマップ系ツールが変換結果の文字キーを注入する(PowerToys が `LLKHF_INJECTED` 付きで送るかは、このリポジトリ内では未確認。
  報告の `competing_software` は PowerToys のプロセスがあることしか示さない)。注入された文字キーの NICOLA 変換を一律に止めない。

## 選択肢

| 案 | 内容 | 判定 |
|---|---|---|
| A | 注入 Ctrl↓ を別枠に記録し、**全キー**(物理含む)の修飾に期限内だけ足す(`read_os_modifiers` を変える) | 却下。注入 Ctrl↑ が欠けると TTL の間、**物理打鍵**が全部 `OsModifierHeld` で素通しになる(ローマ字が生で出る、Ctrl+無変換の IME OFF コンボに一致しうる、親指シフトの同時打鍵が壊れる)。`read_os_modifiers` は `build_ctx` 経由でタイマー・フォーカス・refresh にも効く |
| **A'** | 注入 Ctrl↓ を別枠に記録し、**注入された打鍵の `modifier_snapshot` にだけ**、期限内の ctrl を足す。`read_os_modifiers`・`build_ctx`・`HeldModifiers` は変えない | **採用案** |
| B | 注入された文字キーを常に素通し | 却下。リマップ系の注入キーを変換できなくなる |
| C | Ctrl も `GetAsyncKeyState` で読む | 却下。ADR-054 の問題1(汚染)と問題2(stuck)を再発させる |
| D | 注入元アプリ(プロセス名)で例外 | 却下。LL フックでは注入元を特定できない |
| E | 注入 Ctrl↓ の直後の1キーだけ素通し | 却下。複数キーの組み合わせ(Ctrl+A→Ctrl+C 等)に弱く、V↓ だけ素通しして V↑ を素通ししない対の崩れの扱いも要る。必要な状態は A' とほぼ同じで、単純さの利点がない |
| F | `GetAsyncKeyState` の同意を必須にする(A' への追加) | 却下。事実3のとおりフック時点では競合で false になりうる。KeyUp 欠落時は OS 側も押下のまま固着するので、守りたい場面で役に立たない |

## 決定(案 A')

1. **対象は Ctrl のみ(`is_ctrl_variant`: 0x11・0xA2 は左スロット、0xA3 は右スロット)。Shift は扱わない。** リポジトリの Ctrl 判定は generic `VK_CONTROL`(0x11)も含む(`vk.rs` の `classify_modifier`・`is_ctrl_variant`)。`SendInput` に 0x11 を渡すツール(pyautogui 等)の入力が LL フックに 0x11 のまま届くか 0xA2 に変換されて届くかは未確認なので、CI の A/B で1ケース観測する。 貼り付けの素通しに Shift は要らず(`is_os_modifier_held` は Shift を含まない、Ctrl+Shift+V も Ctrl だけで素通し)、
   Shift は NICOLA の出力面(`shift_held`)・`mode_key_follow_admits_modifiers`・`is_default_ime_on_combo`・半角英数トグル(ADR-245)に効く再発ファミリーなので広げない。
   Alt/Win も対象外。
2. **記録**(`hook.rs` の注入分岐、`if !is_injected { … }` の else 側。`focus_app_disabled` の早期 return より前、zombie 再判定の直後、ログなし。
   順序は `architecture_guard` の `disable_apps_early_return_is_positioned_after_physical_key_state_update_and_before_vk_kana` が固定):
   他アプリの注入(`is_injected && !self_injected`)の Ctrl KeyDown で、**まだ記録がなければ**左右別の `foreign_ctrl_down_at_us[2]`(`HOOK_STATE` の `AtomicU64`、`Relaxed`)に、コールバックで1回だけ取った `ts = now_timestamp()`(µs)を書く。`HOOK_STATE` の atomic が必須なのは、書き込みがフックスレッド(記録・注入 Up)とメインスレッド(reset・Leave・watchdog の解除)、ゾンビの旧フックの3か所から起きるため。CAS・Mutex は不要(解除と競合しても記録が消える安全側に倒れるだけ)。読むのはフックスレッドだけ
   (オートリピート・押し直しで期限を延ばさない。`physical_key_down_at_ms` と同じ「最初の Down を保持」)。注入 KeyUp で 0 にする。`physical_key_state` は従来どおり更新しない。
3. **適用**(`hook.rs` の `read_os_modifiers()` 直後、`is_injected` のときだけ): `modifier_snapshot.ctrl |= (foreign_ctrl_active(ts, down_at[左], ttl) || foreign_ctrl_active(ts, down_at[右], ttl))`(左右の OR。引数は下の純粋関数の3つ)。
   純粋関数 `foreign_ctrl_active(ts_us, down_at_us, ttl_us) -> bool`(`down_at_us != 0 && ts_us.saturating_sub(down_at_us) < ttl_us`。`down_at > ts` でもアンダーフローしない)。時刻 `ts` はコールバックの中で**1回だけ**取り、記録・比較・`build_raw_key_event`(引数を足して `event.timestamp` にも同じ値を使う)で共有する。`event.timestamp` は現状 `build_raw_key_event` の中で snapshot の作成より後に作られるため、そのままでは参照できない。比較は**対象キーのフックキャプチャ時刻**で行い、
   エンジン側の遅れ(CI で delay=88ms)を TTL に含めない。`GetTickCount64`(分解能 約15.6ms)は使わない。
4. **解除**: 注入 Ctrl の KeyUp、同じスロットの物理 KeyUp(0x11 の記録は物理 0xA2 の Up で消す。OS の VK ごと1ビットと一致する、**未確認**)、`reset_physical_key_state`(画面ロック復帰・パニックリセット、BUG-023)、
   `clear_hook_latches_for_app_disable` の Leave、`clear_hook_latches_for_watchdog_reinstall`(issue #165)。後2つは `physical_key_state` の Ctrl/Shift を消す既存経路と同じ位置に足す
   (`app_disable_leave_edge_clears_only_ctrl_and_shift_not_alt_or_win` ガードの更新要否を確認)。
5. **読んではいけない場所**(`architecture_guard` で走査固定): `HeldModifiers`(物理状態を直接読む。別枠を入れると、解放→復元で他アプリの Ctrl を awase が押し直し OS 上の stuck を永続化する、ADR-054 問題2の再発)、
   `ctrl_consumed_since_down`(入れると、注入 Ctrl+V の後の物理 Ctrl+無変換を ime-off-rescue が誤って保留する)、`read_os_modifiers`。別枠を読んでよいのは hook の snapshot 作成部だけ。
6. **journal**: `KeyInput` に「注入由来の ctrl」を表す `foreign_ctrl: bool` を足す(物理の Ctrl が無いのに `ctrl=true` となる報告を後で誤読しない)。**運び方**: フックで snapshot を作るときに求めた「別枠が ctrl を足したか」の bool を `RawKeyEvent` の新しいフィールドに載せ、journal はそれを写すだけにする。エンジンスレッドが `HOOK_STATE` を読むと、決定5の前提に反し、INPUT_DEFER の再生では再生時点の値になる(ADR-129 と同種の誤り)。`injected && ctrl` からの推定は、物理 Ctrl が重なると区別できないので不可。`RawKeyEvent` はコアクレートの型で、構造体リテラルを書いているファイルが22ある(`grep -rln "RawKeyEvent {" crates src | wc -l`)。中身は bool だけで層の規則には触れないが、変更範囲として記録する。
7. **TTL**(`tuning.rs`、`#[measured(...)]`、`FOREIGN_CTRL_TTL_MS`): 物理打鍵に効かないので、長めでも失うものは「注入キーの誤った素通し」だけ。
   導出は「Spokenly の実測最大 102ms と、他ツール(AutoHotkey `Send ^v`、PowerToys 等、未測定)への余裕」。値は実装 PR で測定とともに決める(例: 1000ms を上限の目安)。
   他ツールの実測は値を決める条件ではなく追加確認とする。コミット本文に ms の実測と導出を書く(tuning-constants 規約)。
8. **対象外**(明記): 修飾キーのリマップ(AutoHotkey `CapsLock::Ctrl`、PowerToys の CapsLock→Ctrl 等、注入 Ctrl に**物理**の文字キーを組み合わせる)は直らない。
   案 A でも直るのは TTL 内だけで、押すまでの時間によって挙動が変わる不安定さを新しく作るため。必要なら別 ADR にする。

## 影響・リスク

`modifier_snapshot.ctrl` が注入 V で true になるため、素通し以外の次の経路も物理の Ctrl+V と同じ扱いに変わる(意図どおり、ただし記録しておく)。
- `[[keymap]]`: `find_match(vk, modifier_snapshot)`。`from = "Ctrl+V"` の keymap があると注入 Ctrl+V もリマップされる。`send_keymap_target` は Ctrl を解放してから復元の要否を物理状態だけで判断するので、他アプリが Ctrl を押している最中に OS 上の Ctrl が離されたままになりうる。
- composition の破棄: `cancel_composition_and_arm_post_bypass_on_ctrl`。GJI の未確定文字列がある状態で Spokenly が貼り付けると、**未確定文字列をキャンセルしてから**素通しする。`[[post_bypass]]` が V にマッチすれば latch も武装する。
- `lang_check_on_keydown`・`kp_stage_key_effect_track` は修飾付きのキーを対象外にする(素通しなので無害と思われる)。
- OS 側: awase が Ctrl↓ を再注入済みなので、注入 Ctrl↑ が欠けたときの OS 上の Ctrl 固着は今も同じで、本 ADR の範囲外。

- IME 制御キーの組み合わせ(Opus PR レビュー S2): 手動設定の `keys.ime_on/ime_off/ime_toggle`・engine_on/off は注入イベントも受け付ける。
  修正前は注入された Ctrl+変換(AutoHotkey の `Send ^{vk1C}` 等)の ctrl が false で一致しなかったが、期限内は一致して SetOpen を出し、
  `on_engine_set_open_request`・`is_default_ime_on_combo` のひらがなリセット、`panic_detect`、`kanji_shadow_action`/`passive_without_lookup` の `modified`、
  `mode_key_follow` も、注入キーについては物理キーと同じ扱いに変わる(物理 Ctrl+変換と揃う。意図的)。実害の有無は未確認。望まなければ
  `ime_relevance` が IME 制御のキーには別枠を足さない条件を足す。
- awase 自身の synthetic Ctrl↑(`send_keymap_target` の Ctrl 解放)は `self_injected` で早期 return するので記録を消さない。OS 上の Ctrl が離れた後も
  期限内は注入キーが ctrl=true のまま(物理打鍵には効かないので許容)。

失敗シナリオ(Opus round1 指摘10)と A' での扱い:

| シナリオ | A' |
|---|---|
| 注入 Ctrl↑ の欠落 | 物理打鍵は別枠を読まないので影響なし。TTL の間に注入されたキーだけが素通しになる |
| 注入 Ctrl↓ に物理 Ctrl が重なる | 物理 Up が先: 同 VK の物理 Up で記録を消す(OS と一致)。注入 Up が先: 記録は消えるが `physical_key_state` が真のまま残る。左右は別 VK |
| 他のフックが awase の自己注入を marker なしで再注入 | 注入キーにだけ影響し、TTL で上限が付く |
| オートリピート・押し直し | 期限内は最初の Down を保持して延ばさない。期限切れの記録(KeyUp 欠落の残り)は次の Down が上書きする(Opus PR レビュー S1。残すと次の貼り付けが1回失敗する) |

## 検証

- `state/foreign_modifier.rs`(`#[cfg(windows)]` なし)に状態遷移と実効 ctrl の判定を切り出し、Linux の単体テストで固定: 境界(TTL ちょうど・`down_at=0`・`down_at > ts`・物理キーには効かない・最初の Down を保持・左右別)、解除5経路。`hook.rs` は `#[cfg(windows)]` で Linux のテストには現れないため、この分離が必要。
- `src/engine/tests.rs`(ホスト実行): 注入 V↓ を `ctx.modifiers.ctrl=true` で入れると `OsModifierHeld` で PassThrough。Ctrl↑ が V↑ より先に来ても V↑ が Suppress されない(`handle_bypass` が `output_history.remove_by_scan` を呼ぶ)。
- 報告 journal の seq 61-64・117-120 を `tests/journals/` の replay fixture にする(fix-requires-evidence の (a))。
- `architecture_guard`: 上の決定5の走査固定、`foreign_ctrl` の書き込み位置の固定、**解除5か所(`reset_physical_key_state`・app-disable の Leave・watchdog reinstall・注入 Up 分岐・物理 Up 分岐)それぞれに解除の呼び出しがあること**の走査固定(純粋モジュールのテストでは配線を固定できないため)。ADR-054 の既存テスト(synthetic Ctrl↑ の汚染)が通ること。
- CI の A/B(`ci/e2e-dictation`): `tsx-dict-*-paste`。先に paste 模擬を直す(クリップボードを Win32 API で設定し、フォーカスを奪わない。現状は PowerShell が前面を奪い全試行 `focus_ok=False`)。
  注入は `dwExtraInfo=0`(`TEST_INJECTION_MARKER` では物理扱いになり意味がない)。修正前は「ふ」(FAIL)、修正後は挿入文が入る(PASS)、awase なしを対照にする。
  追加で「注入 Ctrl↓ だけで KeyUp なし」のあと物理打鍵が通常どおり変換されること(stuck 非再発)も観測する。
- 実装 PR で、事実5の古いコメント(`runtime/mod.rs` の「bypass_reason が見るのは build_ctx の戻り値」)を直す。
- 実機は報告者に確認を依頼する。物理の右 Ctrl のホットキー設定(事実6)も併せて聞く。

関連: [BUG-198](../known-bugs/BUG-198.md)(注入 `VK_PACKET` の文字が保留→再注入で消える)は同じ CI・同じリリースで直したいが、原因が別なので別の変更にする。
