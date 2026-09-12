---
id: ADR-165
title: |-
  TsfNative キャッシュ復元にhwnd一致を要求し、無関係な窓の誤ON復元とforce-ON誤発火を防ぐ (BUG-128)
summary: |-
  不具合報告01M27VXD4SPAD4STQ9TG1PZSCD起点。on_focus_process_changedのTsfNativeキャッシュ復元分岐（35230fd由来）が(pid,class_name)キーの粗さ（Windows.UI.Input.InputSite.WindowClassが複数の無関係なUWP窓を同一視）と重なり誤ってdesired_openをON復元、force-ONまで誤発火する経路を特定。opus-adversarial-consult round1で当初案（案A、経過時間のみのガード）が35230fdの救済シナリオを再び壊すため不採用と判明、hwnd一致を弁別子に加える案Eへ差し替え、round2で無条件hwnd一致化、round3でhwndの安定性が未検証と判明し実機ゲートを追加、round5でdragonflyg4実機検証によりround3の懸念を解消、round6の実装差分レビューでhwnd不一致時のログ欠如を指摘され追加
status: |-
  実装済み（ブランチfix/bug128-tsf-cache-restore-recency）。dragonflyg4実機で修正前後の動作を確認済み。WezTerm・仮想デスクトップ実往復・ペイン分割でのhwnd安定性はマージ後ソーク継続
related_adr:
  - "ADR-087"
  - "ADR-090"
  - "ADR-157"
---

# ADR-165: TsfNative キャッシュ復元ヒューリスティックが直近の明示 IME-OFF 意図を上書きする問題

## ステータス

**実装済み（ブランチ `fix/bug128-tsf-cache-restore-recency`）。
dragonflyg4 実機で修正後の実際の動作を確認済み。** opus-adversarial-consult
round1〜round6（設計3ラウンド・round5実機検証・round6実装差分レビュー）を
経て収束。不具合報告 `01M27VXD4SPAD4STQ9TG1PZSCD` の根本原因調査から起票。
round1 で当初案（案A、経過時間のみのガード）が不十分と判明し、案E（hwnd
を弁別子に加える）へ差し替え、round2 で時間条件を撤去し hwnd 一致を無条件化、
round3 で「hwnd の安定性」がこの ADR の範囲では未検証と判明し実機ゲートを
追加、round4 でこの対応（決定は確定・実機検証はマージ前提条件）が妥当と
確認され、round5 で dragonflyg4 実機検証（検証方針節）を行い round3 の
懸念が実証的に否定されたことを確認、round6 で実装差分をレビューし
Must-fix 2件（hwnd不一致時のログ欠如、本ステータス節の未更新——本節は
その反映）を反映した。

**マージ後ソーク継続項目（未消化のまま明示）:** WezTerm・仮想デスクトップの
実往復・ペイン分割での hwnd 安定性は今回の実機検証で個別に確認していない
（検証方針節参照）。`hwnd_cache.rs`/`focus_tracking.rs` のログに hwnd を
含めてあるため（round6 M-impl-1）、これらの操作で `35230fd` の救済が
効かなくなる「静かな後退」が起きた場合は `HwndCache: restore`/
`[focus] TsfNative/SSOT: cache restore スキップ` ログの `cache_hwnd`/
`new_hwnd`/`hwnd=` から診断できる。

## 背景 / 症状

不具合報告 `01M27VXD4SPAD4STQ9TG1PZSCD`（2026-09-11、GJI、症状カテゴリ
`ThumbKeyMisbehavior`）:

> Ctrl+無変換をおしているのに、半角全角にならず、親指シフト入力（ひらがな）の
> ままになっている。

添付 journal（`UnifiedJournal::dump_to_file`）を解析したところ、実際には
Ctrl+無変換 自体は毎回正しく機能していた（`desired_open=false` に確定し
`GjiFsm` は `OffCold` を維持、その後の入力は正しく半角でパススルーされた）。
問題はその**約3.6秒後**に発生した:

1. `09:12:44.58`: msedge.exe（`Chrome_WidgetWin_1`）上で Ctrl+無変換 →
   `UserImeSetIntent{target:false, source:Command}` → `desired_open=false`。
   `AppImeProfile::from_class_name("Chrome_WidgetWin_1")` は
   `IMM32_UNAVAILABLE_CLASSES` に該当するため **`Imm32Unavailable`**
   （`focus/class_names.rs:19-35,100-101`）。journal の `FocusTransition`
   エントリが記録する `app_kind: "TsfNative"` は、`AppImeProfile::
   from_class_name` とは**別のクラス名テーブル**（`focus_tracking.rs:270`
   → `detect_app_kind(&class_name)`、`class_names.rs:326-341`。`chrome_`
   前置クラスは `TsfNative` に分類される）による別軸分類（UIA FrameworkId
   経路 `focus/uia.rs:49-66` は `FocusKind::Undetermined` の補助でしか
   なく Chromium には効かない）であり、本 ADR が扱う `AppImeProfile`
   （`on_focus_process_changed` の分岐に使う）とは別物——**両者を混同しない
   こと**（round1 レビューで当初ドラフトがこの2軸を混同していた誤りを訂正、
   round2 で「UIA FrameworkId ベース」という説明自体も不正確と判明し
   訂正）。
2. `09:12:48.23`: フォーカスが `explorer.exe`（`Windows.UI.Input.InputSite.
   WindowClass`）の2つの異なる hwnd（132130→5048892、dwell 109ms）へ
   連続遷移。このクラス名は `is_tsf_native_window()` に該当するため
   `AppImeProfile::is_effectively_tsf_native()` が真になり、
   `on_focus_process_changed` の TsfNative 分岐（後述）を通る
3. 最初の遷移直後、`ImeEvent::HwndCacheRestored{target:true}` が発火し
   `desired_open` が **true へ強制的に上書き**される
4. その約400ms後、`apply_force_on_for_imm_broken`（`runtime/mod.rs`）が
   `is_eligible_for_ime_force_on()`（`effective_open() && is_japanese_ime()`）
   を根拠に `force-ON (ImmBrokenForceOn)` を発火、実 IME を ON に強制
5. 添付 `state_snapshot` はこの直後の状態（`desired_open: true,
   effective_open: true, applied: Confirmed{open:true}`）を記録しており、
   症状（親指シフト入力のまま）と整合する

ログ抜粋（`app_log_excerpt`）:

```
[warrant-shadow] chain=sync open=true origin=SelfActuated{strategy:"force_on_and_correct_romaji"} warranted
force-ON (ImmBrokenForceOn): apply_ime_open(true) → Applied
```

`[warrant-shadow]` が `warranted` を返していることから、[ADR-090](090-typestate-effectuation-and-adjacent-adr-closure.md)
系の `issue_open_warrant()`（現状 shadow のみ、actuation を止めない）も
**この force-ON を正当と判定していた**。`desired_open` 自体がステップ3で
既に汚染されていたため、warrant を含むどの判定点も正しい答えを出せなかった
（ADR-090 A-2 を前倒しで実配線しても本件は止まらない、詳細は「検討した
選択肢・案C」参照）。

## 原因（コード読解で確定、round1 レビューで機序を訂正・補強）

`runtime/focus_tracking.rs::on_focus_process_changed` の TsfNative 分岐
（`is_effectively_tsf` が真の場合）:

```rust
let desired_open = self.platform_state.ime.model().desired_open();
let cache_says_on = matches!(&cache_hit, Some(snap) if snap.ime_on);
if cache_says_on && !desired_open {
    self.platform_state.ime.apply_hwnd_cache_restore(cache_hit, tick_ms);
}
```

この分岐は 2026-07-05（commit `35230fd`、「Chrome→TsfNative切替時の
Imm32Unavailable desired_open 汚染を修正」）に導入された。**その4日前**、
commit `37883d0`「TsfNative SSOT — フォーカス変化でのキャッシュ復元を
廃止」は、仮想デスクトップ切替中に一瞬だけ挟まる UWP シェルウィンドウへの
フォーカスがキャッシュ復元を誤発火させる desync を構造的に消すために
**キャッシュ復元を全廃**していた。`35230fd` はその4日後、「Chrome の
明示 OFF から TsfNative 窓へ戻ると親指シフトが止まる」という別の再発を
救済するため、`cache_says_on && !desired_open` という条件で復元を
一部復活させた——このとき安全論拠として

> transient 窓のキャッシュは ime_on=false → cache_says_on=false → 復元しない

と書いている。**本 bug report はこの前提が偽であることの実証である**
（explorer の InputSite 面のキャッシュがたまたま `ime_on=true` だった）。
`37883d0`→`35230fd`→本件、という反転史そのものが
`.claude/rules/experiment-logging.md` の想定する「なぜ前回それを捨てたのか
が辿れず同じ選択肢が再浮上する」パターンであり、本 ADR で対策を誤ると
4度目の反転を招く。

### 弱点1: キャッシュキーが粗く、無関係な窓を同一視する

`focus/hwnd_cache.rs::HwndImeCache` は `(pid, class_name)` をキーにする
（`HashMap<(u32, String), HwndImeSnapshot>`、TTL は `HWND_CACHE_MAX_AGE_MS`
= 1時間）。`Windows.UI.Input.InputSite.WindowClass` は Windows Shell
（`explorer.exe`）がホストする**複数の無関係な UWP 入力面**（タスクバー
検索ボックス、スタートメニュー、各種フライアウト等）が共有する汎用クラス
名である。実際、今回のログでも同一 `(pid=22656, class)` の下で hwnd が
132130→5048892 と2回変わっている。このキャッシュエントリは「このウィン
ドウが直前に ON だった」ことを保証せず、「`explorer.exe` のどこかの UWP
入力面が最大1時間以内に ON だったことがある」という弱い情報にしかならない。

なお、このキャッシュが保存しているのは**観測ではなく belief**である点にも
注意が必要（round1 指摘）——`save()` に渡る `ime_on` は `effective_open()`
（`focus_tracking.rs` の退場時 save 呼び出し）であり、実 OS IME 状態の
観測値ではない。一度誤って ON を復元すると、それが再び belief として
キャッシュへ書き戻され TTL が更新される、という自己増幅の可能性がある
（ただし退場時 save は `MIN_FOCUS_DURATION_MS`=100ms 未満の滞在ではスキップ
されるため、今回のような短時間 dwell の transient 窓では書き戻しが起きず、
古い誤りが最大1時間 生き残る側のリスクの方が大きい）。

### 弱点2: 明示 OFF 意図の新しさを見ていない、かつ既存の per-hwnd ガードは
このタイミングでは機能しない

`on_focus_process_changed` の**同じ関数内**、`else` 分岐（純粋
Imm32Unavailable、Chrome/Edge 自身への入場）には対になる関数
`should_discard_imm_broken_cache` があり、`EXPLICIT_OFF_CACHE_SUPPRESS_MS`
（10秒、`focus_tracking.rs:16` のファイルローカル定数——`tuning.rs` では
ない）以内の明示 OFF があればキャッシュの ON を破棄する。TsfNative 分岐は
これを一切参照しない。

一方、**「キャッシュ復元 vs 明示意図」を比較する仕組み自体は既に別に
存在する**: `apply_hwnd_cache_restore`（`state/platform_state.rs:1199-1252`）
は `IntentStore::invalidate_for_cache_restore`（`state/intent_store.rs`）
を呼び、「キャッシュの記録時刻」と「対象 hwnd に記録された明示意図の時刻」
を比較して、意図の方が新しければキャッシュを無効化する（BUG-51 追補 v3）。
**これが本件で機能しなかったのは、`IntentStore` が per-hwnd であるのに
対し、照合キーの `self.shadow_model.current_focus()` が、この呼び出しより
前の行（`focus_tracking.rs` 570-578行、`ImeEvent::FocusChanged` の
dispatch）で既に新しい hwnd（explorer 側）へ更新済みだからである**。
msedge の hwnd に記録された明示意図（Ctrl+無変換）は、explorer の hwnd を
キーに `IntentStore` を引いても見つからない。つまり本件は「明示意図を
見る仕組みが無い」のではなく、**既存の per-hwnd の仕組みが、フォーカス
遷移のタイミング（意図の対象 hwnd と現在の hwnd がずれた瞬間）に対応
できていない**、という既存機構の隙間である。

この事実は、対処の設計に直接効く: `desired_open`（グローバル）と
`persistent_explicit_off_ms()`（グローバル、`last_user_explicit_off_ms`）
という**フォーカスに紐付かない値**を新しい判定に持ち込むと、per-hwnd の
粒度をさらに一段グローバルへ後退させることになり、BUG-128 と `35230fd`
の救済シナリオを判定関数の入力レベルで区別できなくなる（次節「検討した
選択肢・案A」参照）。

## 関連する既存 ADR / known-bugs との関係

- **[ADR-087](087-open-belief-actuation-warrant-separation.md)（BUG-63）**:
  「belief を actuation の根拠に直接使うべきではない」という教訓。
  **本件は ADR-087 が予言した事例そのもの**——belief（`desired_open`）
  1箇所の汚染が `effective_open()` を経由して実 IME への強制 ON に化けた。
  ただし ADR-090 の warrant（A-2、現状 shadow）は `desired_open` 汚染後には
  同じ誤答（`warranted`）を返すため、A-2 を配線しても本件の直接の対策には
  ならない（「検討した選択肢・案C」参照）。
- **[ADR-157](157-symmetric-target-resolution-for-drift-correction-and-force-on.md)
  / known-bugs BUG-110 追補7〜9（issue #189）**: `check_drift_correction`
  （生 `desired_open()`）× `apply_force_on_for_imm_broken`（`effective_open()`）
  の「二重 SSOT」問題。**発火源は独立**（BUG-110 は `HeuristicDefault`/
  `ConvOpenInference` という観測プール経由の汚染、本件は `HwndCacheRestored`
  という `desired_open` への直接書き込み——`state/ime_model.rs` の reducer
  を見ても両者は別経路）だが、**被害を実送信に変換する増幅器は共通**:
  どちらも最終的に `apply_force_on_for_imm_broken` →
  `is_eligible_for_ime_force_on()` → `effective_open()` という同じ合流点を
  通る。「テーマは近いが独立」という単純な切り分けは半分のみ正しい。
  ADR-157 から引き継ぐべき教訓は結論ではなく**プロセス**:
  「発火源に新しい抑止機構を重ねるのではなく、既存の狭いガードを対称に
  広げる方が良い」（BUG-110 追補8〜9、調停案 `DriftBurst` を撤回し
  `ConvOpenInference` ガードに `HeuristicDefault` を1バリアント足すだけの
  修正へ縮小した経緯）。ただし本件では「対称に広げる」対象そのものの
  選び方が論点になる（案A vs 案E、次節）。
- **上記の合流点（`is_eligible_for_ime_force_on()`）自体の是正**は本 ADR の
  スコープ外とする。BUG-110/ADR-157 が既に指摘済みの独立した課題であり、
  本 ADR は `desired_open` が汚染される**入口**（`HwndCacheRestored` の
  誤発火）を塞ぐことに専念する。

## 検討した選択肢

### 案A（round1 で不採用と判定）: 明示 OFF の新しさだけで復元を抑制する
対称ガード

`else` 分岐の `should_discard_imm_broken_cache` と対称に、
`EXPLICIT_OFF_CACHE_SUPPRESS_MS` 経過前ならキャッシュ復元を抑制する
（`pre_focus_explicit_off_ms`/`desired_open`/`cache_says_on` のみを見る）。

**round1 レビューで不採用と判定した理由**: 本件（BUG-128）と `35230fd`
が元々救済しようとしたシナリオ（Chrome で明示 OFF → 数秒後に**同じ**
TsfNative 窓へ戻る）は、`cache_says_on=true` / `desired_open=false` /
「直近に明示 OFF がある」という判定関数への入力が**完全に同一**になる。
両者を分けるのは「入場先の窓が、その ON を作った窓自身かどうか」という
情報であり、案Aの判定関数はこれを一切受け取らない。したがって
`EXPLICIT_OFF_CACHE_SUPPRESS_MS` をどんな値にしても両者を分離できず、
案Aは「精度の改善」ではなく「明示 OFF 後 N 秒間は per-window キャッシュ
より新しい方のグローバル意図を優先する」という**ポリシー反転**にしか
ならない。採用すると `35230fd` の再現手順（「Chrome で Ctrl+無変換 →
仮想デスクトップ切替で Windows Terminal へ戻る → 全キー PassThrough →
親指シフト機能停止」）が明示 OFF 後 10 秒以内という限定つきで再発する。

### 案E（採用、round2 で「直近の明示OFFがあるときだけ」の条件を撤去し
無条件化）: キャッシュに hwnd を持たせ、ON 復元は hwnd 一致を要求する

`HwndImeSnapshot`（`focus/hwnd_cache.rs`）に `hwnd: usize` を1フィールド
追加する。save 側は `FocusTracker::save_ime_state`（`focus/tracker.rs`）が
既に `self.current`（`FocusIdentity`、hwnd を保持）を見ているため追加取得
コストは無い。restore 側は `classified.hwnd`（`on_focus_process_changed`
内で既に手元にある）を渡すだけでよい。判定:

```rust
fn should_restore_tsf_cache_on(
    snap: Option<&HwndImeSnapshot>,
    desired_open: bool,
    new_hwnd: usize,
) -> bool {
    let Some(snap) = snap else { return false };
    snap.ime_on && !desired_open && snap.hwnd == new_hwnd
}
```

**round2 での訂正**: round1 時点の案Eは「直近
`EXPLICIT_OFF_CACHE_SUPPRESS_MS`（10秒）以内に明示 OFF があるときだけ
hwnd 一致を要求し、無ければ従来どおり（hwnd 不問で）復元する」という
時間条件つきだった。round2 レビューで、この時間条件が残っている限り
「明示 OFF から10秒経過後に、無関係な hwnd の cached ON を復元する」
という**本 bug report と同じ経路が依然として再現する**ことが指摘された
（TTL は `HWND_CACHE_MAX_AGE_MS`=1時間、ガードは10秒なので、塞がるのは
窓の一部でしかなかった）。**この指摘を受け、時間条件を撤去し hwnd 一致を
無条件の要件にした。** `pre_focus_explicit_off_ms`/`EXPLICIT_OFF_CACHE_
SUPPRESS_MS` は判定から消え、副次的に「N3: 10秒という値がこのシナリオ
向けに未実測」という round1 の Nice-to-have も解消する（値そのものを
使わなくなるため）。

**採用理由**:

1. **弁別子が正しい**: 本件と `35230fd` シナリオの違いは経過時間では
   なく「入場先がその ON を作った窓自身か」。案Eはこれを直接見る。
2. **`35230fd` を壊さない（ただし hwnd の安定性は実機未検証、round3
   指摘）**: `classified.hwnd` の出どころは `GUITHREADINFO.hwndFocus`
   （`focus/probe.rs`→`win32.rs`、null 時 `hwndActive`→`GetForegroundWindow`
   にフォールバック）であり、**トップレベルウィンドウではなくフォーカス中の
   子コントロール**（`state/platform_state.rs:1437-1439` が BUG-91 絡みで
   同じ注意を明記済み）。`class_names.rs:44` のコメントが示すとおり、
   `35230fd` の救済対象である Windows Terminal 自身も
   `Windows.UI.Input.InputSite.WindowClass`（今回誤復元を起こした explorer
   のフライアウトと**同じクラス**）の子ウィンドウでフォーカスを受ける。
   XAML Islands の InputSite 子 hwnd はタブ生成・ペイン分割・DPI/テーマ
   変更等で作り直されうるため、「仮想デスクトップ往復で同じ hwnd に戻る」
   という前提は**この ADR の中では検証されていない仮定**である。もし
   実際には不安定なら、無条件 hwnd 一致は `35230fd` の救済を時間条件
   つき版の「10秒」ではなく**恒久的に**失う。理由3（フェイルセーフの
   向き）により致命的な後退にはならない（`37883d0` 以前の SSOT 継続へ
   単に戻るだけ）が、**実機検証（検証方針節）を経るまでは「案Eが
   `35230fd` を壊さない」と断定しない**。
3. **フェイルセーフの向きが正しい**: hwnd を頻繁に再生成するアプリ
   （今回の explorer InputSite 系列）では不一致となり復元が空振りする
   だけ——その帰結は「TsfNative SSOT を維持する」（`37883d0` 以前の
   挙動、desired_open を前窓のまま持ち越す）であり、偽の force-ON より
   実害が小さい。無条件化しても time-of-day に関わらずこの安全側の
   フェイルセーフが常に効く。
4. **反転史を止める**: `37883d0`（全廃）→`35230fd`（穴を開け直す）→
   本件、という反転に対し、時間条件つきの案Eでも「明示 OFF 後10秒だけ
   `37883d0` の挙動に戻す」という時間分割が残っていた。無条件化した
   ことで、穴の**大きさ**の調整ではなく穴の**形**（弁別子）そのものを
   恒久的に直す変更になった。
5. **`(pid, class_name)` キー自体は変えない**ため、`else` 分岐
   （Imm32Unavailable）や他のキャッシュ利用シナリオへの影響が無く、
   案Bのような広い棚卸しが不要。

**残る限界**: `(pid, class_name)` キーの粗さ自体（弱点1）は案Eでも解消
しない——hwnd 不一致時に「復元しない」という安全側へ倒すだけで、キャッシュ
に無関係な情報が混入すること自体は直っていない。BUG-107（`ImmCapability
Store` の `class_name` 単独キー問題、別キャッシュだが同型）と合わせて
別途調査する価値があるが、本 ADR のスコープ外とする。

### 案B（不採用、round1 で却下根拠を訂正）: `HwndImeCache` のキーを hwnd
単位に全面変更する

**当初ドラフトはこの却下理由として「`(pid, class_name)` キーが意図的な
設計だと示す `hwnd_cache.rs` 内のコメント」を引用していたが、round1
レビューで実ファイル（103行）にそのようなコメントが存在しないことが
判明したため、この却下根拠は撤回する。** 改めて検討すると、案Bの
フルスコープ（`(pid, class_name)` キー自体を hwnd 単位へ変更）は
`else` 分岐や他の利用箇所への影響範囲の洗い出しが必要で、本 bug report
1件の修正としてはスコープが過大——ただし必要なのはそのフルスコープでは
なく、案Eが行う「スナップショットに hwnd を追加し判定にだけ使う」という
最小部分で十分なため、案Bは不採用のまま、案Eを採用する。

### 案C（却下）: ADR-090 Phase A-2（warrant による実 actuation ブロック）
を本件のために前倒しする

背景節で示したとおり、`desired_open` 自体が汚染された後では warrant も
同じ誤った答えを返す（`warranted` ログで実証済み）。A-2 昇格は
[ADR-163](163-actuation-decision-io-separation-and-replay-harness.md) が
定める「実削除+差分ゼロ再生証明」という重い前提条件付きの独立した
大規模イニシアチブであり、本件1つのために前倒しする根拠にならない。
（この却下は ADR-087/090 の教訓自体が本件に無関係という意味ではない——
関連節で述べたとおり本件は ADR-087 が予言した事例そのものである。
「A-2 が配線されていても本件は止まらなかった」という事実のみを根拠に
却下する。）

### 案D（却下）: 新規の調停・抑止機構を設計する

ADR-157 が実装・実機ソークまで済ませた調停機構をユーザー指摘で全面撤回
した前例（発火源の上に抑止機構を重ねる設計パターン）そのもの。案Eで
弁別子レベルの修正として十分説明できる問題に対し、新規の型やタイマーを
導入する理由がない。（なお「差分が小さいから安全」という理由付けは
妥当でない——差分の大きさではなく「既存の正当な救済シナリオを壊さない
か」で測るべきであり、案Aはまさに差分は小さいが既存シナリオを壊す例
だった。）

## 決定

**案E（`HwndImeSnapshot` に hwnd を追加し、ON 復元は hwnd 一致を無条件に
要求する。時間条件は設けない）を採用する。**

## 残課題（決定に付随する既知の限界、対処しないことを明記）

- **`(pid, class_name)` キーの粗さ自体（弱点1）は解消しない。** hwnd 不一致
  時に「復元しない」という安全側へ倒すだけで、キャッシュに無関係な情報が
  混入すること自体は直っていない。BUG-107（`ImmCapabilityStore` の
  `class_name` 単独キー問題、別キャッシュだが同型）と合わせて別途調査する
  価値があるが、本 ADR のスコープ外とする。
- キャッシュ復元を抑止した際、そのキャッシュエントリ自体は無効化しない
  （TTL＝1時間で自然失効するまで残る）。hwnd 不一致で復元をスキップした
  場合、同じキャッシュエントリは以後も残り続けるが、次に同じ hwnd へ
  正当に戻ってきたときには引き続き有効な情報として使える可能性がある
  ため、積極的な無効化はしない。ただし退場時 save が `MIN_FOCUS_
  DURATION_MS`（100ms）未満の滞在でスキップされる関係で、今回のような
  短時間 dwell の transient 窓では古い belief が最大1時間残り得る
  （弱点1と同じ限界の別の現れ）。
- `is_eligible_for_ime_force_on()` の合流点自体（BUG-110/ADR-157 が既に
  指摘）は本 ADR のスコープ外。
- **hwnd 一致は必要条件であって十分条件ではない**（round3 Nice-to-have）:
  HWND 値は OS が破棄済みハンドルを再利用しうる。誤一致には「同一 pid」
  「同一クラス名」「破棄済み値の再利用」「TTL=1時間以内」が同時に必要で
  確率は低いが無視できない。ただしその場合の帰結は「無関係な cached ON
  を復元する」＝**案E導入前の今日の挙動そのもの**であり、案Eが新たに
  作る失敗ではなく、単に塞ぎ損ねが残るだけ（悪化はしない）。

## 検証方針

`focus/hwnd_cache.rs`・`runtime/focus_tracking.rs` はいずれも
`#[cfg(windows)]` ゲート配下（`runtime/mod.rs` のモジュールツリー全体、
CLAUDE.md 参照）のため、Linux では `cargo check --target
x86_64-pc-windows-msvc -p awase-windows --tests --lib` によるコンパイル
確認までで、ユニットテスト自体の実行は `windows-build` CI に委ねる。
新規テストは `should_restore_tsf_cache_on` に対し、少なくとも次のケースを
固定する: hwnd一致×cache=ON×desired_open=false→復元／hwnd不一致×同条件
→抑止／`desired_open=true`→復元不要（hwnd一致でも）／`cache_says_on=false`
→復元不要（hwnd一致でも）。

**実機ゲート（round3 Must-fix）→ 実機検証済み（round5 確認）**: 採用理由2
の「Windows Terminal 等では hwnd が安定する」という主張について、
`on_focus_process_changed` の TsfNative 分岐に一時的な診断ログ
（`[adr165-spike] tsf-enter hwnd=... pid=... class=... process=...`）を
追加し、dragonflyg4 実機（`clipwire-exec` 経由、ブランチ
`fix/bug128-tsf-cache-restore-recency` コミット `34bdab40`）で検証した。

**実測結果（2026-09-11、dragonflyg4）**: Windows Terminal（pid=20280、
`windowsterminal.exe`）へのタブ切替・フォーカス往復を繰り返しながら
23:07:09〜23:22:32（約15分間、6回の入場）観測したところ:

- トップレベル（`CASCADIA_HOSTING_WINDOW_CLASS`）: hwnd=197908 で
  23:22:14・23:22:21 の2回とも一致
- InputSite 子ウィンドウ（`Windows.UI.Input.InputSite.WindowClass`、
  round3 が懸念した「explorer のフライアウトと同じクラス」の当のクラス）:
  hwnd=263890 で 23:07:09・23:19:41・23:22:00・23:22:07・23:22:27・
  23:22:32 の**6回すべて**一致

一方、explorer 側の別 UWP フライアウト（pid=22656、`process=None` ——
UAC 保護等でプロセス名が取得できない別プロセス）は、同一セッション内
（23:22:00〜23:22:31）では hwnd=3345678 で安定していたが、本 ADR の
発端になった不具合報告では**別の機会**に hwnd=132130→5048892 と変化して
いた（背景節参照）。「同じ機会（同じウィンドウインスタンス）内では安定、
別の機会に作られたウィンドウは別 hwnd」という案Eが依拠する前提と一致する。

**round3 の懸念（Windows Terminal 自身も InputSite 子ウィンドウを使うため
hwnd が不安定かもしれない）はこの実測により直接否定された**——懸念の
対象そのものである InputSite 子 hwnd が6回の入場すべてで一致したため。
`GetAncestor(GA_ROOT)` への切替は不要と判断する。

**今回の実機検証で確認できていない項目（未消化のまま明示する、暗黙に
閉じない）**: ペイン分割・仮想デスクトップの実往復（Win+Ctrl+←/→）・
WezTerm は今回のセッションで個別に切り分けて確認していない。ただし
タブ切替と約15分の時間差を挟んだ複数回のフォーカス往復で hwnd が一貫して
安定したという事実から、これらの操作でも同様に安定する公算が高いと判断し、
残りは `develop` マージ後の通常の実機ソークに委ねる。崩れていることが
判明した場合の対処は上記の `GetAncestor(GA_ROOT)` 案。

## 残課題（追加、round5）

- **案Eが保証するのは per-window の正しさであり、「明示 OFF 後は一切
  復元しない」ではない。** 上記の実測が示すとおり、同一の flyout hwnd を
  再訪した場合、他所（別ウィンドウ）での明示 OFF があってもその hwnd の
  cached ON は復元される——これは `35230fd` の意図どおりの設計上の挙動で
  あり、回帰ではない。将来「明示 OFF 後に同じウィンドウへ戻ったら ON に
  戻った」という報告が来た場合、本 ADR の regression と即断せず、まず
  「本当に同じ hwnd への回帰か」を確認すること。

## 関連

不具合報告 `01M27VXD4SPAD4STQ9TG1PZSCD`、`docs/known-bugs.md` BUG-63・
BUG-107・BUG-110（追補7〜9）、[ADR-087](087-open-belief-actuation-warrant-separation.md)、
[ADR-090](090-typestate-effectuation-and-adjacent-adr-closure.md)、
[ADR-157](157-symmetric-target-resolution-for-drift-correction-and-force-on.md)。
opus-adversarial-consult round1（Must-fix 6件反映）・round2（Must-fix 1件、
時間条件つき案Eを無条件化して反映）・round3（Must-fix 1件、hwnd 安定性の
未検証性を明記し実機ゲートを追加、Nice-to-have 1件反映）・round4
（設計判断そのものへの Must-fix ゼロ、収束確認）・round5（dragonflyg4
実機検証結果を反映、Must-fix ゼロ、収束確認）・round6（実装差分レビュー、
Must-fix 2件——hwnd 不一致時のログ欠如とステータス節の未更新——反映）実施済み。
