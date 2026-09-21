# opus-adversarial-consult round1: ADR-173 レビュー

対象: `docs/adr/173-scope-solo-tap-ime-action-to-tsf-native.md`（commit `495f7637`）
レビュー範囲: 読み取りのみ。ファイルは一切編集していない。

**総評（結論を先に）**: 「回避策をアプリ限定にする」という**方針そのものは妥当**だが、
**現在の決定案（`AppImeProfile::TsfNative` ガード）はそのままでは実装してはならない**。
理由は2つ——(a) この判定は ADR-173 が対象と名指しした Windows Terminal を
**構造的に取りこぼす**（リポジトリ内に、その取りこぼしを固定した名前付き回帰テストが
既に存在する）、(b) KeyDown と KeyUp の間でプロファイルが変わりうるため、
ADR-153 が M19 で塞いだ「孤立 KeyUp が GJI へ漏れる」非対称を**新たに作り直す**。
加えて「IME ON 固着」は本 ADR の根拠から切り離すべきで、独立した BUG として
扱わないと、この ADR の成否判定そのものが汚染される（B4）。

---

## Blocker

### B1. `AppImeProfile::TsfNative` は Windows Terminal を構造的に取りこぼす（決定案の前提が誤り）

ADR-173 は決定（行49-58）と根拠（行59-68、特に行63「`AppImeProfile::TsfNative`
（Windows Terminal/WezTerm 等）」）で、Windows Terminal が
`AppImeProfile::TsfNative` に分類されることを前提にしている。**これは誤り。**

`crates/awase-windows/src/focus/class_names.rs`:

- `:19-35` `IMM32_UNAVAILABLE_CLASSES` に `"CASCADIA_HOSTING_WINDOW_CLASS"`(`:34`)
  と `"PseudoConsoleWindow"` が含まれる。
- `:51-62` `is_tsf_native_window` にも `"CASCADIA_HOSTING_WINDOW_CLASS"`(`:57`) が含まれる。
- `:121-129` `from_class_name` は **IMM32 リストを先に**評価する
  → CASCADIA は `Imm32Unavailable` になり、`TsfNative` には**決してならない**。
- `:234-247` `is_effectively_tsf_native` の doc がこの落とし穴を名指しで警告している:

  > 値だけを見て `matches!(profile, AppImeProfile::TsfNative)` と判定すると、
  > Windows Terminal のような実質 TSF ネイティブなウィンドウを取りこぼす
  > （2026-07-05 実機ログで確認: フォーカス着地直後の "enforce IME OFF" ブロックが、
  > Windows Terminal を非 TSF ネイティブと誤判定して発火した）。
  > …必ずこのメソッドを使うこと。

  **ADR-173 の決定案は、この doc が名指しで禁止しているパターンそのもの。**

- `:399-406` にこの挙動を固定した名前付き回帰テストが既にある:
  `cascadia_profile_is_masked_to_imm32_unavailable`
  （`assert_eq!(from_class_name("CASCADIA_HOSTING_WINDOW_CLASS"), Imm32Unavailable)`）。

実機側の裏取り（推測ではない）:

- `docs/known-bugs/BUG-113.md` 冒頭の「アプリ」欄が、Windows Terminal のクラスを
  **`CASCADIA_HOSTING_WINDOW_CLASS` / `Windows.UI.Input.InputSite.WindowClass` の2種**
  と記録している（同欄は続けて `AppImeProfile::TsfNative` と書いているが、上記
  `from_class_name` の優先順位より、CASCADIA 側は実際には `Imm32Unavailable` になる
  ——**既存ドキュメント自体がこの2つを混同している**）。
- `crates/awase-windows/tests/journals/actuation_decision/bug-131-report-01m29kdnz.json`
  は WindowsTerminal.exe + GJI の実機ダンプ（`docs/bug-reports-triage.md:47`、
  不具合報告 `01M29KDNZ22KNY1FPXSKBGMW7V`）。37レコード中 `gate_inputs.profile` は
  **`TsfNative` 33件 / `Imm32Unavailable` 4件**で、レコード 4〜7 で
  `TsfNative → Imm32Unavailable → TsfNative` と**1セッション内で往復している**。

**失敗シナリオ**: ユーザーが Windows Terminal で無変換を単独タップした瞬間、
フォーカスが CASCADIA 側（`Imm32Unavailable`）にあれば `explicit_ime_action_target`
は `Inactive` を返し、生 `VK_NONCONVERT` がそのまま GJI へ届く → BUG-113 の根本原因に
逆戻りし「@」が再現する。InputSite 側にあるときだけ効く。結果として
**「直ったり直らなかったりする」**という、最も切り分けの難しい壊れ方になる。

**最小の修正**: `self.platform.current_app_profile()
.is_effectively_tsf_native(self.platform.focus.class_name())` を使う。
なお `key_pipeline.rs:655-659` は同じファイル内の直近の前例として**既にこの正しい形**
を使っている（`idle-conv-check` の `is_tsf_native`）。ADR 行52-53 が列挙した
「既存アクセサの参照箇所 `:455, :659, ...`」のうち `:659` がまさにそれであり、
ADR はその中身を読まずに参照件数だけを根拠にしている。
ただし `is_effectively_tsf_native` に直しても S4 の範囲問題は残る。

### B2. KeyDown / KeyUp 間でプロファイルが変わると、M19 のペアリングが壊れて孤立 KeyUp が GJI へ漏れる

`explicit_ime_action_target` は2箇所から呼ばれる:

- `key_pipeline.rs:1274-1281`（KeyUp 早期分岐。`SuppressOnly` なら
  `explicit_ime_action_consumed = true` を立てて `return false`）
- `key_pipeline.rs:1353`（KeyDown 本体）

そして `:1197-1203` の doc が、ステートフルなラッチを**意図的に避けた**根拠を
こう書いている:

> ステートフルなラッチ（KeyDownで立ててKeyUpで消費）は取り出し漏れによる
> 恒久残留リスク（B14と同型の落とし穴）を持つため使わず、ケース3改が
> belief不変（既にOFF→OFFのまま）を前提にしていることを利用し、**KeyDownとKeyUpの
> 間でbeliefが変化しない限り同じ結論になる**ステートレスな再評価で代える。

この不変条件は「判定入力が belief だけ」であることに依存している。ここに
**フォーカスプロファイル**という第2の入力を足すと、不変条件が崩れる——そして
B1 で示したとおり、プロファイルは実機で1セッション内に往復することが
journal で確認済みである（GJI 候補ウィンドウ、Alt+Tab、CASCADIA⇔InputSite 往復）。

**失敗シナリオ2つ（どちらも ADR-153 が塞いだ非対称の再生産）**:

- KeyDown 時 TsfNative（Suppress）→ KeyUp 時 Imm32Unavailable（`Inactive`）
  → KeyUp にマーカーが立たず `transport.rs::plan:363-372` が `Allow`
  → **孤立 KeyUp が GJI へ漏れる**（`:1266-1273` の B7/B8 対策が防ごうとしたもの）。
- KeyDown 時 Imm32Unavailable（Allow）→ KeyUp 時 TsfNative（Suppress）
  → **GJI は Down だけ受け取り Up を受け取らない**（BUG-131 が
  `kana_mode_restore_key_down` で踏んだ Down/Up 非対称と同型の固着リスク）。

ADR は、この不変条件が壊れることに**一言も触れていない**。実装前に、
(a) プロファイル判定を KeyDown 時点の値でラッチする（doc が避けた方向なので
理由の更新が要る）、(b) Down/Up 双方が同じ結論になることを別の手段で保証する、
(c) 非対称を許容し実害が無い根拠を示す、のいずれかを ADR に明記すること。
併せて `:1197-1203` の doc（不変条件の記述）も更新が必須。

### B3. 「対象外プロファイルではこの設定自体は一切発火しない」（行55-58）は事実に反する

同じ config（`muhenkan_solo_tap_ime_action` / `henkan_solo_tap_ime_action`）は、
**コアクレート側のケース1** からも読まれる:

- `src/engine/nicola_fsm.rs:955` / `:963` が
  `ThumbSoloSpecialHandling.explicit_ime_action` に同じ値を載せる。
- `src/engine/nicola_fsm.rs:2043-2076` `resolve_explicit_ime_action` が
  `resolve_pending_thumb_as_single` の優先順位2としてこれを消費する（belief ON 側）。

ADR-173 は `crates/awase-windows` 側の `explicit_ime_action_target`（ケース2/3改、
belief OFF 側）にしかガードを置かない。したがってガード適用後も、
**Chrome / メモ帳で belief ON のまま無変換を単独タップすると、awase 自身の明示 config が
IME を OFF にする**——ユーザーが「他アプリでは GJI 自身のネイティブなかな切替に
委ねたい」（行42-45）と言った、まさにその挙動が残る。

さらに構造的な制約: コアクレートは ADR-019 で OS 依存型を持てないため、
`AppImeProfile` をそのまま参照できない。層境界を守ったまま同じスコープを
ケース1にも効かせるなら、既存の `ThumbSoloSpecialHandling`（既に事前分類済みの
値を渡す入口）に真偽値を1つ載せる形になる。

ADR は最低限、**ケース1を意図的にグローバルのまま残すのか**、残すならなぜ
ユーザー要望と矛盾しないのかを明記すること。現状の「この設定自体は一切発火しない」
という記述は、次にこの ADR を読む人を確実に誤導する。

### B4. 「IME ON 固着」の因果連鎖が欠けており、ADR が根拠として挙げた事実は根拠になっていない

ADR 行25-29 は「awase.exe を完全に停止した状態でも再現する」ことを、決定の
支持材料として提示している。**これは支持材料にならない。** awase 停止時は
awase が何も抑止しないので、本 ADR の修正はその構成では**定義上ゼロ効果**である。
この観測が言えるのは「最終的な加害者は GJI である」までで、「awase 側の抑止で
連鎖が切れるか」については何も言っていない。ADR は両者を取り違えている。

より重大なのは、**awase 稼働時には物理半角/全角キーがそもそも GJI に届かない**こと:

- `docs/known-bugs/BUG-113.md` 原因1（dragonflyg4 実機）: この機体の物理半角/全角キーは
  `VK_KANJI`(0x19) ではなく **`VK_DBE_SBCSCHAR`(0xF3) / `VK_DBE_DBCSCHAR`(0xF4)**
  として awase フックに届く。
- `crates/awase-windows/src/runtime/transport.rs:405-418`: 既定
  `dbe_mode_key_policy = Suppress` かつ `ime_actuation_owned`（GJI なら
  `gji_direct_applicable` が真）のとき、0xF0/0xF1/0xF3/0xF4 の **KeyDown は無条件に
  `Suppress`**、KeyUp も `Suppress`。

つまり awase 稼働時、「半角/全角を押しても OFF に戻らない」は
**GJI が生キーをどう扱うかの問題ではなく、awase 側の actuation / belief の問題**である。
無変換/変換の抑止は、この経路に一切触れない。

本 ADR の修正が固着を解消しうる連鎖は、実質**1本しかない**:

> 生の無変換/変換 → GJI の `ITfKeyEventSink` 横取り → GJI 内部状態が壊れる →
> その後 awase が送る `VK_IME_OFF` を GJI が無視する → 固着

ADR はこの連鎖を**明示的に書くべき**。書けば反証可能な予測になる（抑止後も固着が
起きるなら原因は awase 側であり、受け入れ基準 行94-95 は不成立と判定できる）。

**ADR が検討していない対立仮説**: 並行ブランチ
`adr/172-tsf-blind-rescue-consolidation` の ADR-172
（`docs/adr/172-tsfnative-blind-rescue-four-system-consolidation.md`、
opus-adversarial-consult round1〜3 で収束済み・コード変更なし）が、TsfNative で
ON 方向へ働く**独立した4系統**を表にまとめている:

| 系統 | エントリポイント | 発火 |
|---|---|---|
| force-on | `runtime/mod.rs::apply_force_on_for_imm_broken` | 周期リフレッシュから毎tick（「TsfNative唯一のON方向救済」） |
| drift correction | `runtime/ime_refresh.rs::ir_apply_drift_correction` | 同じく毎tick |
| warmup | `output/mod.rs::eager_tsf_warmup_inner` | 複数トリガー、単一合流点なし |
| reassert | `runtime/mod.rs::reassert_explicit_physical_key`（ADR-121 D1） | **物理IMEキー検出というイベント駆動** |

「ユーザーが OFF にしたのに ON に戻る」は、これらのいずれかが再表明している場合の
教科書的な症状であり、特に reassert は**物理 IME キー押下そのものが引き金**である。
前例として `docs/known-bugs/index.md:31` の BUG-022（「MS Edge で Uwp⇔TsfNative
フォーカス往復後、conv=Eisu に固着」）は、まさに B1/B2 で問題にしている
プロファイル往復が固着を生んだ実例である。

**推奨**: スコープ限定の決定と固着を**切り離す**。
(1) 本 ADR の決定は「ユーザーが明示要望したアプリ限定化」だけを根拠にする
（固着が未解明でも独立に正当化できる）。
(2) 固着は `docs/known-bugs/BUG-142.md` として独立に起票し、最初の調査手順を
「**awase 稼働状態で**再現させ、半角/全角押下前後の journal `ActuationDecision`
レコードを見て awase 自身が ON を再表明していないか確認する」とする
（ADR-172 の4系統表を出発点にできる）。
(3) 固着が再現しなくなったことを、本 ADR の成功根拠として**扱わない**
——タイミング依存でマスクされただけの可能性を排除できないため
（記憶にある `feedback_dont_trust_actuation_outcome_over_observation_events` と同型）。

---

## Should-fix

### S1. `current_app_profile()` の stale 窓は仮説ではなく実害の前例がある（ADR 行85-88 への回答）

ADR は「フェンシングが必要かどうか」と問うだけで答えていない。答えは
リポジトリ内にある:

- `crates/awase-windows/src/runtime/focus_tracking.rs:139-144`:
  > BUG-114 根本原因1（ADR-134 D1c）: `advance_focus_tracking` 済み…の**後**に
  > 呼ぶこと。これより前だと `current_app_profile()` がまだ正しい値を返さない。

  **実際に不具合を1件生んだ既知の窓**である。

- フォーカス追跡は別の `spawn_local` タスクであり、フォーカス着地から
  `advance_focus_tracking` 完了までの間に届いた打鍵は**直前のアプリのプロファイル**を読む。
  「Alt+Tab で Terminal に切り替えて即座に無変換をタップ」は、まさにこの窓。

- 既存のフェンシング前例が2つある: `platform.rs::injection_hint_for(pid, class_name)`
  （doc に「フォーカス変更直後の stale 回避用」と明記）、および ADR-106 の
  `FocusFence`/`is_identity_ok`/`admit()`（`focus/current.rs:13-23` 参照）。

ADR は、どちらかを採用するか、あるいは「stale による取りこぼしは許容する」と
明記すること。**後者を選ぶ場合、fail 方向が fail-open（生キーが GJI へ漏れて
「@」が出る）である**ことも併記すべき。

### S2. ガードが弾いたときのログが無く、サイレント no-op になる

`kp_stage_shadow_ime_toggle` は他のすべての分岐で `tracing::info!` を出している
（`:1361`, `:1386`）。ガードだけ無言だと、次の不具合報告が「隠し設定が効かない」に
なったとき、app.log から「ガードで弾かれた」「belief が ON だった」
「`dedicated_fn_key` で弾かれた」(`:1236-1240`) の区別が付かない。
プロファイルと `class_name` を含む1行を（レート制限付きで）出すこと。

### S3. 既存の per-app 機構（`app_overrides`）を検討した形跡がなく、却下理由も無い

`src/config.rs:726-765` の `AppOverrides` は既に、プロセス名リストによる
per-app 指定の枠組み（`force_text` / `force_bypass` / `force_vk` / `force_tsf` /
`disable_apps` / `input_relay_apps`）と、共有の照合ロジック
（`state/app_suppression::matches_disabled_app`、大小無視・`.exe` 有無吸収）と、
バリデーション（`check_override_list`、`:1273-1300`）を持っている。

ユーザーの要望は文字どおり「このアプリだけ」であり、
`app_overrides.solo_tap_ime_action_apps` 形式なら
(a) プロセス名なので**決定的**（往復するクラス名に依存しない＝B1・B2 を丸ごと回避）、
(b) 既存の照合・検証を再利用、(c) 将来 WezTerm だけ／別アプリだけという要望にも
ADR 改訂なしで対応できる。

ADR には**「却下した代替案」節が1つも無い**。上記と、`is_effectively_tsf_native`
案（B1）を併記し、なぜ選ばなかったかを残すこと。

**関連する罠**: 不具合報告 `01M29KDNZ22KNY1FPXSKBGMW7V` の環境が
「`force_tsf` 設定で WindowsTerminal.exe を強制 TSF 化」（`docs/bug-reports-triage.md:47`）
であるとおり、このユーザーは既に `force_tsf` を設定している。しかし `force_tsf` は
**ローマ字出力モード（`is_tsf_mode`）の軸であり、`AppImeProfile` には一切影響しない**
（`AppImeProfile` は `class_name` + `relay_apps` のみから決まる。
`focus/current.rs:79-80`）。「force_tsf を入れてあるから TsfNative のはず」という
読み違えが起きやすいので、ADR に明記すること。

### S4. `is_effectively_tsf_native` に直しても、スコープは「PowerShell だけ」より広い

`class_names.rs:51-62` の `is_tsf_native_window` の対象は
{`Windows.UI.Core.CoreWindow`(UWP/WinUI), `XamlExplorerHostIslandWindow`(エクスプローラ/
タスクバー), `Windows.UI.Input.InputSite.WindowClass`, `CASCADIA_HOSTING_WINDOW_CLASS`,
`org.wezfurlong.wezterm`}。

特に `Windows.UI.Input.InputSite.WindowClass` は **Windows Terminal 固有ではない**:

- `CHANGELOG.md:450` が、GJI の候補ポップアップ自体が
  `Windows.UI.Input.InputSite.WindowClass` であり `Chrome_WidgetWin_1` との間で
  フォーカスが往復すると記録している。
- `docs/design/journal-diagnostic-fidelity-fixes.md:393` が
  「Chrome 内 InputSite⇔本体、Edge の Uwp⇔TsfNative」と記録している。

つまりガードは **Chrome / Edge の中でも時々通る**。ADR 行96-97 の回帰確認基準
（「他アプリで挙動が変わらないこと」）は、落ち着いた手動テストでは通り、実運用で
落ちるタイプの基準になる。

**緩和要因（ADR に明記すべき）**: `explicit_ime_action_target:1217-1220` は
`!effective_open()`（belief OFF）を要求するので、composing 中に出る GJI 候補
ポップアップがフォーカスを持っている瞬間とは重なりにくい。これは**実際に効く緩和**
だが、暗黙の前提のままにせず書くこと。

### S5. 回帰テストの置き場所（ADR 行98-100）が Linux CI で走らない

ADR は「`explicit_ime_action_target` の単体テスト、プロファイル別の期待値」を
計画しているが、CLAUDE.md が明記するとおり
`crates/awase-windows/src/runtime/mod.rs` はモジュールツリー全体に
`#[cfg(windows)]` が掛かっているため、`runtime/key_pipeline.rs` 内の
`#[cfg(test)]` テストは **Linux のテストバイナリに存在すらしない**
（`cargo nextest list` にも出ず、エラーも skip メッセージも出ない）。
`cargo nextest run --workspace --lib` では一切検証されず、`windows-build` ジョブ
だけが頼りになる。

これを許容するなら ADR に明記すること。あるいは
`crates/awase-windows/tests/architecture_guard.rs`（Linux で走るソーススキャン型）に
1本足すこと。**なお既存ガードは本変更で壊れない**が、それ自体が穴である:

- `kp_stage_shadow_ime_toggle_never_reintroduces_case3_forced_actuate`
  （`:3764-3833`）は `explicit_ime_action_case3_off` タグの不在、`SuppressOnly`
  アーム内の actuation 呼び出しの不在、`explicit_ime_action_target(` の
  呼び出し回数 `>= 2` を見る。
- `explicit_ime_action_case1_keeps_m13_but_case2_3_does_not`（`:3913-3935`）は
  `explicit_ime_action_target` 本体に `.is_passthrough(` / `.mode_key_config` が
  無いことを見る。

いずれも `current_app_profile()` ガードの追加には反応しない
→ **後日このガードが消されても、どのテストも気付かない。**

### S6. 規約・番号・整合

- **`docs/adr/index.md` に ADR-173 の行が無い**
  （`.claude/rules/docs-frontmatter-convention.md`「新規ADRを起票したら index.md にも
  短い1行を追加する」違反）。
- **ADR-172 への参照が無い**。`related_adr`（行7-10）に `"ADR-172"` を追加すべき
  ——ADR-172 は TsfNative の ON 方向救済 4 系統そのものを扱い、その `related_adr` には
  既に `"ADR-153"` が入っている（隣接領域で相互参照が片側だけ欠けている状態）。
  コード衝突リスクは低い（ADR-172 は「コード変更なし」で収束、
  `state/platform_state.rs`・`state/ime_model.rs` の doc コメント追記のみ）。
- **BUG-142 の番号**: 全ref を確認したところ `495f7637` 時点で衝突は無い
  （`git log --all --diff-filter=A --name-only -- 'docs/known-bugs/BUG-142*'` が空）。
  ただし `feedback_bug_number_collision_on_branch_merge` の前例どおり、起票直前に
  もう一度確認すること。
- **SSOT の二重更新**: `src/config.rs` の `muhenkan_solo_tap_ime_action` doc（`:430-465`）は
  「`explicit_ime_action_target` の doc comment を正本とする」と明記している。
  スコープ限定を入れるなら、正本側（`key_pipeline.rs:1166-1203`）に
  「どのアプリで効くか」と B2 の不変条件更新を書き、config 側からはそれを指すこと。
  現在の ADR の「次のアクション」にはこの doc 更新が入っていない。
- **`.claude/rules/complexity-budget.md`**: 本 ADR は「gate を1つ足す」変更だが、
  同ルールは (i) 未発効（TH1e 未達成）かつ (ii) 「gate 数」を対象外と明記しているため、
  1-in-1-out の適用対象ではない。**阻害要因ではない**ことを確認済み。

---

## 「この不確実性を残したまま実装に進んでよいか」への回答

**ADR を2つに割れば進んでよい。今の形では進めない。**

- **進めてよい部分**: 「グローバルな回避策をアプリ限定にする」という決定は、
  ユーザーの明示要望だけで独立に正当化でき、固着の機序が未解明でも成立する。
- **進めてはいけない理由**: 固着の解消が未検証だからではなく、**スコープ判定の
  実装方法が壊れているから**（B1 で対象アプリを外し、B2 で新しいキー漏れを作る）。
  この2点は実機検証を待つまでもなく、リポジトリ内の既存テスト・doc・journal 実データ
  だけで確定する。
- **固着の扱い**: BUG-142 として独立に起票し、本 ADR の受け入れ基準から外す。
  固着を本 ADR の成否に紐づけると、(a) 固着が直れば誤った因果を確定させ、
  (b) 直らなければ独立に正当なスコープ限定まで巻き添えで撤回される——
  `docs/known-bugs/BUG-124.md` が「教訓」として記録した
  「revert する際は対象を絞れ、無関係な副産物まで巻き込むな」の再演になる。

### 実装前に埋めるべき最小セット

1. B1: `is_effectively_tsf_native(class_name)` に差し替える、または S3 の
   `app_overrides` 方式へ切り替える（後者なら B1・B2・S1・S4 が同時に消える）。
2. B2: KeyDown/KeyUp のペアリング不変条件をどう保つか決め、
   `key_pipeline.rs:1197-1203` の doc を更新する。
3. B3: ケース1（コア側）をどうするか明記する。
4. B4: 固着を ADR の根拠から外し、BUG-142 を起票する。
5. S3: 「却下した代替案」節を新設する。
6. S6: `docs/adr/index.md` に1行、`related_adr` に ADR-172。
