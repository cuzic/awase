# ADR-186 敵対的レビュー round2（v2 = commit `d82cfe08`）

## 判定: **収束（Blocker 0）**

round1 の Blocker 3件は**すべて正しく反映**されている。決定の絞り込み（決定2のみ／3保留／4現状維持／5切り出し）は妥当で、新しい型・フィールド・variant は実際に 0 個。
残るのは **Must-fix 2件（どちらも記述の訂正・検証計画への追加で、設計変更は不要）** と Should-fix 3件。

- **M1（v2）**: 実測表の「Composition」「Conversion」行で、半角/全角のセルの**VKが入れ替わっている**（r2:142 は Conversion、r1:160 は Composition）。B3 で直したはずの Composition/Conversion の取り違えが、この1セルだけ残っている。
- **M2（v2）**: 「期待される結果」表の3行目（直接入力で無変換/変換 → **満たす**）は、**ObservedEisu が立っている場合にだけ**正しい。belief が `AssumedRomaji` のまま OFF→ON すると、eisu reset を抑止しても Engine ON になる（抑止は「正しい belief を消さない」だけで、無い belief を作らない）。この窓は決定3を保留した結果として実在し、しかも**検証計画の手順では踏めない**。

---

## 1. round1 指摘の反映状況（抜き打ち検証込み）

| # | 指摘 | v2 の反映 | 検証結果 |
|---|---|---|---|
| B1 | convは保存される／eisu reset が要件を壊す | 実測節（72-74行）、前提の訂正5(b)（96-98行）、決定2の後半（112-117行） | **正** 。`round2:147/157/284` を再確認、3件とも conv=0x10 のまま open 0→1。`session.cc:1023-1034`・`keyevent_handler.cc:700-705` の引用も正確 |
| B2 | 0xF3/0xF4 とも HANKAKU＝トグル | 前提の訂正5(a)（95-96行）、決定5（131-135行）、表の VK 明示 | **正**。`keyevent_handler.cc:315-316`、`atok.tsv:29/75/107`、`transport.rs:405-418`、`vk.rs:132-133` すべて実在を確認。別件への切り出しも妥当 |
| B3 | Composition/Conversion の取り違え／全セル一致の撤回 | 表を4状態→5状態に分割（61-67行）、78-80行で根拠（Space・STEP14の次候補） | **ほぼ正。ただし M1 参照**（半角/全角セルだけ逆） |
| M1 | `resolve_delegate_to_open_axis` は存在しない／ADR-179決定2は別物 | 決定2（106-107行）で `resolve_pending_thumb_as_single` 内の `special.delegate_to_open_axis` と正しく記述。二重actuationしないことも明記 | **正**。`nicola_fsm.rs:2130` / 2251-2273 を再確認 |
| M2 | 有効化は `gji_thumb_key_ime_toggle`、Passthrough とは別物 | 決定2（108-110行）で opt-in と親指キー前提を明記、Passthroughが効かなくなることも明記。非決定（141行）でADR-179のマージ前TODOと衝突しないことも記載 | **正**。`config.rs:427`、`gji_charset_autodetect.rs:429-472` と整合 |
| M3 | idle-conv-check の安全網評価が過大 | 期待表の最終行（153行）で TsfNative限定・500ms/1500ms を明記 | **正**。`idle_check.rs:48/54/60/66` と一致 |
| M4 | UserTurnOnEisuReset との衝突／物理0xF2の到達が未確認 | 決定3で保留、`transport.rs:197-206` を根拠に明記（119-124行）。置き換え（併存させない）も明記 | **正**。保留という結論は最も安全 |
| M5 | 決定4の前提「composingを見られない」は誤り | 決定4（126-129行）で fail-closed を正しく説明し現状維持 | **正**。`nicola_fsm.rs:2261-2273` と一致 |
| S1 | 新 variant 不要、`UserHalfWidthAlnumToggle` 再利用 | 決定3（122-124行）と非決定（139行） | **正** |
| S2 | A/B/T 全件一致は無効レコード3件を除く | 実測節 49-51行、56行に無効レコードの行番号 | **正**。`round1:94-96` / `round1:242` / `round2:294-318` を再確認 |
| S3 | 2ラウンドは別ビルド | 52-53行 | **正** |
| S4 | conv 0x09/0x19（ROMANビット） | 77行 | **正**。ただし表の各セルがどちらの conv で測られたかまでは書いていない（Should-fix S3 参照） |
| S5 | 0xF1 は未測定と未割当を書き分け | 75-76行 | **正** |
| S6 | 参照ADRがこのブランチに無い／ADR-179のTODOと衝突 | status（21-23行）と非決定（141行） | **正**。`docs/adr/` に179/181/183/184/185が無いことを再確認 |
| S7 | 決定2のみに絞る | 決定全体の構成 | **正** |

**抜き打ちした行番号引用（すべて一致）**: r1:9/19/24/56/61/66/71/83/103/108/113/143/160/175/180/185/242、r2:9/19/29/39/44/54/64/75/90/95/142/152/162/294-318、`round2:147/157/284`、`round1:94-96`。

---

## 2. Must-fix（v2 で新たに検出）

### M1(v2). 表の半角/全角セルで Composition と Conversion が逆

**ADR 65行（ON・変換前(Composition)）**: 「**0xF4**: IME OFF、未確定破棄(r2:142)」
**ADR 66行（ON・変換中(Conversion)）**: 「**0xF3**: IME OFF、未確定破棄(r1:160)」

**実際は逆**:

- `round2-richedit.log:142-146`（STEP15、0xF4）: 前 `comp="下"` ＝**変換済み候補** → **Conversion**。直前の押下列も `無変換`(r2:132、press≈14:35:40.3) → `変換`(r2:137、press≈40.76、`atok.tsv:30 Composition Henkan → Convert`) → この 0xF4(press≈42.2)。変換キーを経ているので Conversion。
- `round1-edit.log:160-164`（STEP18、0xF3）: 前 `comp="か"` ＝**読みのまま**。直前に記録されているのは `14:27:54 ESC` → `14:27:55.8 半角/全角(0xF4)`（＝IME ON）だけで、監視対象である `Space`(0x20)・`変換`(0x1C) の押下記録は無い → 変換を経ていない → **Composition**。

結論は変わらない（`atok.tsv:29/75/107` により Composition でも Conversion でも `CancelAndIMEOff`）。しかし決定1で「この表を一次情報として固定する」と宣言する表である以上、B3 で直した当の取り違えが1セル残っているのは看過できない。

**修正案**: 65行の半角/全角セルを「**0xF3**: IME OFF、未確定破棄(r1:160)」、66行を「**0xF4**: IME OFF、未確定破棄(r2:142)」に入れ替える。あわせて「0xF4 が ON 中に届いた実例（＝決定5の根拠）は Conversion 状態での1件」と注記すると、決定5の根拠の所在が明確になる。

### M2(v2). 「期待される結果」表3行目の「満たす」は条件付き

**ADR 149行**: 「直接入力で無変換/変換 | IME ON(convは直前値を復元) | belief ON。eisu reset抑止によりObservedEisuが残ればEngine OFF、かななら Engine ON。**満たす**(決定2の抑止が前提)」

eisu reset の抑止は「**既にある正しい ObservedEisu を消さない**」だけで、「無い belief を作る」わけではない。belief が `AssumedRomaji` のまま OFF→ON すると、抑止してもしなくても Engine ON になる。

**踏める手順（決定3を保留した結果として実在する）**:
1. IME ON・かな（belief: open=ON, `AssumedRomaji`）。
2. ユーザーが **ひらがなキー or Shift+無変換** で半角英数へ（実測どおり conv 0x19→0x10）。決定3が保留なので **belief は動かない**（`AssumedRomaji` のまま）。しかも物理0xF2は `TurnOn` 扱いで `eisu_reset_on_turn_on_while_open`（`key_pipeline.rs:1606-1620`）が走るため、仮に直前が ObservedEisu でも **AssumedRomaji に戻される**。
3. conv 観測が走る前に（idle-conv-check は TsfNative限定＋ガード4/5＋500ms待ち）、**無変換を2回**押す（OFF → ON）。
4. 決定2により belief open=ON、input_mode は `AssumedRomaji` → **Engine ON**。実IMEは ON・半角英数（conv保存）→ NICOLA のかな出力が半角英数IMEへ流れる。

**今日との比較**: 今日は同じ手順で belief open が動かない（無変換に shadow_action が無い）ため Engine OFF のまま＝**偶然正しい**。つまりこのケースだけ決定2は**退行**になる。

**検証計画が踏めない**: 165-167行の手順は「直接入力からの無変換で Engine が誤って ON にならないこと」までで、上の「ひらがなキーで半角英数にしてから無変換2連打」が入っていない。**A/Bを通過してしまう。**

**修正案（新しい型は不要、いずれも記述・計画の変更）**:
- 期待表3行目を「ObservedEisu が立っていれば満たす。**立っていない（直前に決定3保留のキーで半角英数へ入った等）場合は満たさない**」に訂正。
- 検証計画1に手順を追加: 「ひらがなキー（または Shift+無変換）で半角英数にした直後に 無変換 を2回押し、Engine が ON にならないこと」。
- A/Bで再現したときの**退避策も先に決めておく**: 決定2を **ON→OFF 方向だけ**に限定する（`Toggle` を belief ON のときだけ採用し、belief OFF のときは今日どおり生キーをGJIへ通す）。要件のうち安全上重要なのは「英数のとき Engine OFF」＝誤ったかな出力を出さない方向であり、「かなのとき Engine ON」の失敗は1打の遅延（＝現状）に留まるため、非対称にする価値がある。ただしこれは `nicola_fsm.rs` の共有分岐に方向条件を足す実装変更になるので、A/Bで問題が出てから。

---

## 3. 決定2の「eisu reset 抑止」の具体的な入れ場所（依頼事項2）

### 実際に走る経路（コードで追跡済み）

```
resolve_pending_thumb_as_single (nicola_fsm.rs:2251-2273)
  └ special.delegate_to_open_axis = Some(Toggle)  ← gji_thumb_key_ime_toggle=true で armed
  └ 戻り値 .1 = Some(ShadowImeAction::Toggle)、actions は空
→ 呼び出し元が self.ime_open_requested に格納（nicola_fsm.rs:630/1604/1792/1811/2854/2896/3013）
→ Engine::apply_ime_open_request (engine.rs:618-630)
   └ action.resolve(ctx.ime_on) = !belief
   └ ime_set_open_effects (engine.rs:876-895) → Effect::Ime(SetOpen{open, origin: ExplicitUserAction})
→ key_pipeline::kp_stage_post_decision の decision.find_ime_set_open_with_origin() ブロック（1791行〜）
   └ origin == ExplicitUserAction → platform_state.handle_engine_set_open(...) → applied
   └ applied なら record_explicit_intent(new_ime_on, UserIntentSource::Command, tick)（1834行付近）
   └ ★ eisu_reset_on_ime_on(applied && new_ime_on, input_mode)   ← key_pipeline.rs:1911-1915
      └ Some(AssumedRomaji) なら apply_input_mode_correction(.., PostSetOpenEisuReset, ..)  ← 1925-1929
```

**抑止すべき唯一の地点は `crates/awase-windows/src/runtime/key_pipeline.rs:1911-1915`**。
他の2つの救済呼び出し（`1607` の `eisu_reset_on_turn_on_while_open`、`1655` の `eisu_reset_on_ime_on`）は `kp_stage_shadow_ime_toggle` 内であり、**無変換/変換には shadow_action が付かない**（`ImeKeyKind::from_vk` に 0x1C/0x1D の行が無い、`vk.rs:124-135`）ため、delegate 経路では到達しない。→ **ADRの「既存分岐への条件追加」1箇所で済むという主張は正しい**。

### 条件式の選び方（ここを間違えると既知バグが再発する）

- **✗ 「GJI/ATOK なら常に抑止」にしてはならない**。`eisu_reset_on_ime_on` は最適化ではなく**デッドロック解除**である（`state/eisu_recovery.rs:3-16`）: `ObservedEisu` は engine activation を `NotRomajiInput` で塞ぎ、`transition_activation` は `NotRomajiInput` のとき `SetOpen(true)` を抑制するため、Imm32Unavailable アプリ（Chrome/Edge）では**訂正する観測経路が存在せず engine が永久に inactive** になる（2026-07-06 MS Edge で実発生）。IME種別で一律に抑止すると、この既知バグを再び開ける。
- **○ 「このイベントが無変換/変換の修飾なし単独タップのとき」で条件付ける**。必要なデータは**すべて同じスコープに既にある**: 同ブロックの 1876-1881行が `event.vk_code`・`event.modifier_snapshot.{ctrl,shift,alt,win}` を使って `is_default_ime_on_combo` を組み立てている。同じ材料で「`vk_code ∈ {VK_CONVERT, VK_NONCONVERT}` かつ修飾なし」を判定でき、IME-ONコンボ（Ctrl+変換）とは確実に区別できる。**新しい型・フィールド・witness は不要**。
  - `SetOpenOrigin` で区別する案は**採ってはならない**: delegate も IME-ON/OFFコンボも EngineOnコンボも同じ `ExplicitUserAction` であり（`engine.rs:608-617` が「新しい witness 種別は不要」と明記している経緯そのもの）、区別するには新 variant が必要になる。
- 抑止した場合の**復帰手段を ADR に書くこと**: stale な `ObservedEisu` に嵌ったときは Ctrl+変換（IME-ONコンボ）が従来どおり reset するうえ、`kp_reset_to_hiragana_romaji_capsoff`（1895-1900行）でひらがな＋ローマ字へ寄せるので、ユーザー側の脱出口は残る。

### 別の副作用の検査（依頼事項2の後半）

| 観点 | 結果 |
|---|---|
| **ADR-090 A-2 warrant（`issue_open_warrant`）** | 問題なし。唯一の発行点は `state/actuation_chain.rs:349`。delegate 経路は `handle_engine_set_open` 内で `record_explicit_intent(.., UserIntentSource::Command, ..)` を通るため、warrant は **Step 1（`ExplicitUserIntent`）** で授権される（`open_warrant.rs:150-157`）。`requested` と intent が一致するので `finalize` も通る。拒否されて空振りする経路は無い |
| **同・副作用** | `EXPLICIT_ON_INTENT_TTL_MS = 10_000`（`tuning.rs:467`、OFF側はさらに長い）。**無変換/変換の単独タップ1回ごとに、以後10秒間 open 意図が IntentStore に固定され、drift correction より優先される**。今日は Passthrough で生キーを渡すだけなので intent は記録されない。NICOLA では親指キーの単独タップが日常的に起きるため、これは新しい露出。ADR のリスク節に1行足すべき（Should-fix S1） |
| **ADR-153 / BUG-113（「@」）** | **むしろ改善方向**。(a) delegate 発火時は生の `VK_NONCONVERT`/`VK_CONVERT` を GJI に渡さない（`nicola_fsm.rs:2263-2271` + `Decision::Consume`）ため、BUG-113 の根本原因である「GJI の `ITfKeyEventSink` 横取り」の入口が塞がる。(b) probe 側も `is_ime_mode_key_for_ime` が 0x1C/0x1D を含む（`vk.rs:180-185`）ので、この打鍵では `should_run_idle_conv_check` のガード5で probe を出さない（`idle_check.rs:66-68`）＝「読み取りと書き込みの時間的近接」も発生しない |
| **フォーカス遷移直後** | **新しい空振り窓がある**。`strip_ime_set_open_if_settling`（`executor.rs:156-172`）と `handle_engine_set_open` の2段フィルタ（`platform_state.rs:289-309`）が settling 中の `SetOpen` を落とす。このとき物理キーは既に `Decision::Consume` で中継されないため、**OS側にも awase側にも誰も切り替えない**（ADR-119 が名付けた「二重の空振り」と同型）。今日は Passthrough で生キーが GJI に届くので、少なくとも IME は切り替わる。窓は短い（settle 期間）が、Alt+Tab 直後に無変換を押す操作は普通に起きる。ADR の「残る限界」表に1行足すこと（Should-fix S2） |
| **`ime_open_requested` ワンショットチャネルの漏れ** | **検査したが問題なし**（誤検出を避けるため明記する）。`flush_pending` が値をセットするのは `ThumbRawVkEmission::Allowed` のときだけ（`nicola_fsm.rs:611-631`。`Denied` は `(空, None)` を返す）。本番の `flush`/`flush_to_effects` 呼び出しは `engine.rs:413/593/690/750` の4箇所すべてが `Denied`。`Allowed` を渡す本番経路は `toggle_enabled`(`nicola_fsm.rs:711`)・`swap_layout`(1090)・`handle_bypass`(3295/3319) の3つで、前2つは `on_command` が `discard_ime_open_request()` 済み、`handle_bypass` は `on_input` 経路なので末尾で必ず `apply_ime_open_request` が回収する。`PendingCharThumb` の flush 腕は `resolve_char_thumb_as_simultaneous` を呼ぶのでチャネルに触れない（`nicola_fsm.rs:638-661`）。**決定2でこの経路が新たに armed になっても、stale 発火は起きない** |
| **`architecture_guard` の保護範囲** | `user_ime_on_paths_are_paired_with_eisu_reset`（`tests/architecture_guard.rs:662-674`）が数えているのは `write_sync_key(` / `write_physical_key(` / `write_set_open_request(` の**出現数**であって、eisu reset の呼び出しではない。したがって **今回の「条件追加」はどのガードテストにも引っかからない**。ADR 116-117行の「対応表とガードテストに例外を明記する」は良いが、**回帰を守るのは検証計画2の回帰テストだけ**である点を明記すること（Should-fix S3） |

---

## 4. ユーザーの現在の設定で何が変わるか（依頼事項3）

前提: `gji_thumb_key_ime_toggle=false`、`muhenkan_solo_tap_always_suppress=false`（＝非composing時 Passthrough、`config.rs:301`）、無変換/変換が親指キー、GJI ATOKプリセット。

| | 今日（opt-in=false） | 決定2実装後（opt-in=true） |
|---|---|---|
| 無変換 単独タップ（非composing） | delegate は `None`（`gate_thumb_key_ime_actions` が Toggle を落とす）→ 優先順位4 の Passthrough → **生 `VK_NONCONVERT` が GJI へ** → GJI が開閉トグル。awase の belief は動かない | **優先順位3 の delegate が発火** → 生キーは送られず、awase が `SetOpen(!belief)` を actuate。belief は押下時点で動く |
| **変換 単独タップ** | 同上（Passthrough） | **同じく Toggle になる**（ATOK は両方 Toggle）。右親指キーの単独タップでも IME が切り替わる。`henkan_solo_tap_always_suppress` の設定値に関わらず delegate が優先する |
| composing 中の無変換 | Passthrough（`muhenkan_solo_tap_ignore_composing_guard` 次第）→ GJI が ToggleAlphanumericMode | **変わらない**（delegate は composing で fail-closed。決定4どおり） |
| Passthrough 設定の意味 | 有効 | **非composing の単独タップでは無効化される**（ADR 109-110行に記載済み） |
| BUG-113「@」 | 生キーが GJI へ届くため、GJI の TSF横取りが「@」を出す条件が残る | 生キーを渡さないので**改善** |
| IntentStore | 記録されない | 単独タップごとに `UserIntentSource::Command` で **10秒間 open 意図が固定**（drift correction より優先） |
| フォーカス遷移直後の無変換 | 生キーが GJI へ届き IME は切り替わる | settling 中は SetOpen が落ちる＋生キーも出ないので**完全な空振り** |
| belief がズレているとき | GJI が実状態に基づいて正しくトグル | awase が**自分の belief に基づいて逆方向へ** actuate しうる（`config.rs:452-455`、TsfNative/Blind で読み戻し不可） |
| ADR-179 の実験コミット（Passthrough前提） | — | 決定2は Passthrough を前提にしないので、実験の撤去と両立する（ADR 141行の記載は正しい） |

**見落としやすい点の総括**: (a) 変換キーも同時に Toggle になる、(b) Passthrough 設定が事実上死ぬ、(c) IntentStore の10秒固定が新設、(d) フォーカス遷移直後の空振り、(e) M2(v2) の退行窓。(a)(b) は ADR に記載済み、(c)(d)(e) は未記載。

---

## 5. Should-fix（3件、いずれも1〜2行の追記）

- **S1(v2)**: リスク節に「単独タップごとに `record_explicit_intent`（`UserIntentSource::Command`）が走り、`EXPLICIT_ON_INTENT_TTL_MS = 10_000`（`tuning.rs:467`）の間 drift correction より優先される」を追記。
- **S2(v2)**: 「残る限界」表に「フォーカス遷移直後（settling 中）は `SetOpen` が2段フィルタで落ち、生キーも中継されないため完全な空振りになる（`executor.rs:156-172`、`platform_state.rs:289-309`）」を追記。今日は生キーが届くぶん退行にあたる。
- **S3(v2)**: 116-117行の「ガードテストに例外を明記」について、`user_ime_on_paths_are_paired_with_eisu_reset` は `write_*(` の出現数を数えるだけで eisu reset の条件変更を検出しないことを明記し、**検証計画2の回帰テストが唯一の保護**であることを書く。あわせて S4(round1) の残り（表の各セルがどちらの conv で測られたか）を、必要なら表に conv 列として足す。

---

## 6. 結論

**Blocker は残っていない ＝ 収束**。決定2（+抑止）の設計は、コードを追った限り実装可能で、二重actuation・warrant拒否・ワンショットチャネル漏れ・BUG-113のいずれも新たな問題を起こさない。抑止は `key_pipeline.rs:1911-1915` 1箇所への条件追加（条件式は同スコープの `event.vk_code` + `event.modifier_snapshot`、新しい型は不要）で足りる。

マージ前に反映してほしいのは、**M1(v2) の表の入れ替え**と、**M2(v2) の期待表の訂正＋検証手順の追加**の2点だけ。どちらも設計変更ではない。
