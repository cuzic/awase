# ADR-118: 注入キーイベントの取り扱い — 解釈しないものは消費もしない

## ステータス

**実装済み（2026-09、Opus 2体の敵対的議論 + Cloudflare R2実データ検証 + コードレビュー2周を経て決定）。**
[GitHub Issue #136](https://github.com/cuzic/awase/issues/136)（PowerToys「境界線のないマウス」
(Mouse Without Borders, MWB) 経由でIME ON/OFFが効かない）に対応。`docs/known-bugs.md` BUG-90
の追補でもある。リンク(a)（フックチェーン順序への依存）と、MWBの実際のプロセス名は実機未検証
（下記「未解決事項」参照）。

## コンテキスト

### 症状は独立した2つのバグの合成だった

issue #136（および内部トリアージのBUG-90、2026-08-26に一部調査済み・クローズ済みの同一
インシデント）は、同一ユーザーからの「local側・remote側両方から対になる報告」という体裁を
取っていたが、実データ（Cloudflare R2から取得した実際のjournal/ログ）を精査した結果、
症状は**構造的に異なる2つの原因**の合成だったと判明した。

1. **リモート側**: `PhysicalKeyDisposition::plan`（`crates/awase-windows/src/runtime/
   transport.rs`）の `VK_DBE_*` Suppress判定が `event.injected` を一切見ていなかった。
   これは2026-07-06のBUG-14修正が明言した不変条件「OSへの配送(passthrough)は維持し、実IME
   追従は観測経路に委ねる」を、2026-08-05のBUG-52対応（`is_dbe_mode_key_down`条件の追加）が
   後から破ったリグレッションである。MWB下では、BUG-14ガード（`kp_stage_shadow_ime_toggle`）
   によりawase自身は代理actuateせず、かつこのSuppressがOSへの配送も止めるため、「誰も何も
   しない」二重の空振り状態になっていた。
   - 実データ裏付け: R2から取得したremote側報告のjournalで、`VK_DBE_ALPHANUMERIC`
     (`injected=true`, `scan=0x3A`) が複数回到達していることを直接確認した。
2. **ローカル側**: MWB中継ウィンドウにフォーカスがある間、`ImmCrossProcessStrategy`が
   「今Win32フォーカスを持つウィンドウ＝ユーザーが実際に入力しているアプリ」という前提の
   もとに設計されているため、中継ウィンドウ自身の無意味なIMEコンテキストに向けてactuationが
   空振りする。実ログ引用（local側報告のapp.log）:
     ```
     [shadow-toggle] intent 昇格: vk=0xF0 scan=0x3A action=TurnOff kind=PhysicalImeKey injected=false false→false
     ```
     `current`（トグル前のeffective_open()）と`new_val`（TurnOff後の目標値）が共に`false`
     ——つまりawaseの信念は既に一致しており、apply-ime自体が一切呼ばれない（no-op）。
     直前10秒のログには`observed=false≠desired=true`のdrift correctionがGeneration 0〜5
     まで繰り返されており、MWB中継ウィンドウのIME観測自体が実在の意味を持たない
     （中継ウィンドウは実際のテキスト入力面ではないため）ノイズであることを示している。

### 共通する不変条件

両者は同じ不変条件の異なる適用例である:

> **awaseが意図として解釈しない/actuateしない入力を、awaseが消費してはならない。**
> Suppressは「awaseが代わりにactuateするから物理キーは食う」というバーターであり、
> awaseがactuateしない（できない）状況でSuppressだけを行うと、OS側にもawase側にも
> 誰もIMEを切り替えない「二重の空振り」になる。

これはBUG-14が確立した「解釈と消費はセットで判断する」という教訓を、注入イベント
（決定1）と入力中継ツール（決定4）という2つの異なる場面に一般化したものである。

## 検討したが却下した設計

### 決定3（撤回）: config opt-inでinjectedイベントをユーザー意図に昇格させる

`[keys.ime_detect] accept_injected` のようなconfigで、`RelayTrust`（ZST）を経由して
injectedイベントを`IntentWitness`に昇格させる案を検討したが、以下の理由で推進派自身が
撤回した:

1. 決定1（injectedを`Allow`で素通しする）と同時適用すると、injectedなsyncキーがintentに
   昇格→`shadow_toggled=true`→awaseがactuate、かつ同じキーがOSにも素通しされ実IMEもネイティブ
   にactuateする、というBUG-46型の二重actuationを再生産する。
2. config由来のトークン（`RelayTrust`）は「そのイベントについての事実」を何も証明しない
   ため、これをwitnessとして`IntentWitness`を構築するのは、削除済みの`UserIntentSource::
   Recovery`/`HwndCache`（`.claude/rules/ime-belief-architecture.md`参照）と同型の
   「推測を意図として偽装する」構造になる。

後継案`IntentWitness::from_relayed_physical(e)`（`e.injected && e.scan_code != 0`を型で
要求、config不要）はMWBが実際にscan codeを保存して中継している事実（local側/remote側
どちらの実データも`scan=0x3A`で一致）により実現可能性は上がったが、今回は不実装とする
（MS-IME自己注入側のscanが未測定のため判別子として使うにはさらに1件の実測が必要）。

### 決定5（撤回）: ImmCross分岐に`shadow_toggled`を読ませる

`transport.rs::plan`のImmCross無条件Suppressを`shadow_toggled`で条件付ける案を検討したが、
`effective_open()`という汚染された値（中継ウィンドウのIME観測がノイズであるため）に判断を
依存させることになり、「たまたまbeliefが一致していた回だけ直る」確率的な部分修正にしか
ならないと判明し撤回した。決定論的な修正のためには、belief非依存の信号（フォーカスウィンドウ
の同一性）が必要という結論に至り、決定4（プロファイル分類）に一本化した。

### `caps()`空チェーン案（撤回）

決定4のcondition (a)「IME actuationの所有権を持たない」を、`state/app_ime_policy.rs::caps()`
（(profile, IME種別)→actuation機構チェーンの唯一の宣言点）で`chain: &[]`を返すことで実現
しようとしたが、コードレビューで**設計上の重大な欠陥**が発見された。`chain: &[]`は安全な
no-opではなく、`ActuationOrder::run_chain`の`abandon()`経由で`ImeOpenOutcome::Failed`を
発生させる。`Failed`の下流（`runtime/executor.rs`/`state/platform_state.rs`/`state/
ime_model.rs`のreduce）は「全機構が失敗した＝IMEは要求の**逆**の状態にあると確信」という
**観測なしの高信頼度belief書き込み**を行う。これは`.claude/rules/ime-belief-architecture.md`
が禁じる「観測が無いのにbeliefを書く」パターンそのものであり、condition (c)の趣旨に
真っ向から反する。加えて既存テスト`caps_chains_have_no_unreachable_trailing_element`が
空チェーンを`assert!(!chain.is_empty())`で明示的に禁止しており、そもそもコンパイル/テストが
通らない設計だった。

## 決定

### 決定1: `transport.rs::plan` への injected passthrough

`VK_DBE_HIRAGANA`（F2）専用分岐の直後・`is_kanji_event`判定の前に、injectedイベントを
`Allow`にする分岐を追加した。

```rust
if event.injected {
    debug_assert!(
        !shadow_toggled,
        "injected イベントで shadow_toggled が立つのは設計違反 \
         (BUG-14 ガード kp_stage_shadow_ime_toggle が必ず false にする)"
    );
    return Self::Allow;
}
```

**ImmCrossアームも意図的に対象内**とした。ImmCross分岐のSuppressは「spurious連鎖の構造的
遮断」という別種の保護（`feedback_immcross_owns_kanji`の設計原則）だが、injectedイベントは
`shadow_toggled`を発火させないためawase自身がactuateすることはなく、spurious連鎖の前提
（awaseの自actuationと物理キー通過の競合）が成立しない。したがってこのアームを貫通させても
同種のリスクは生まれない。

**F2 (VK_DBE_HIRAGANA) 分岐は意図的に対象外**とした。そちらのSuppressは「awase自身の
warmup F2送信とのダブルF2防止」という別種の保護であり、awaseがactuateするか否かとは無関係
のため、injectedでバイパスしてはならない。この結果、**リモート側がTsfNativeかつ
`f2_warmup_owned=true`のとき、injected な `VK_DBE_HIRAGANA` は依然Suppressされる**
（既知の限界、下記参照）。

### 決定4: `AppImeProfile::InputRelay` の新設

入力中継ツール（PowerToys Mouse Without Borders等）のウィンドウを、`app_overrides.
input_relay_apps`（プロセス名リスト、既定は空配列、既存の`matches_disabled_app`を再利用）
で識別し、新プロファイル`AppImeProfile::InputRelay`に分類する。効果は3点:

- **(a) IME actuationを所有しない**: `ImeOpenOutcome::NotOwned`（新設、下記参照）を返す。
  **当初`runtime/executor.rs::dispatch_ime_set_open`の1点だけにgateを置いたが、コード
  レビューで実際のactuation呼び出し経路が5つ（`ImeController::apply`の呼び出し元3つ+
  `run_open_chain_async`の呼び出し元2つ）あり、`runtime/key_pipeline.rs`のshadow-toggle
  経路（issue #136で報告された「物理IMEキー押下→IME OFF/ON」操作そのもの）がこのgateを
  バイパスしていたことが判明した。バイパス時は物理キーが`transport.rs::plan`で`Allow`
  （素通し）されるのと同時にawase自身も actuate してしまい、BUG-46型の二重actuationという
  新規リグレッションを生んでいた。** 修正として、gateを実際の合流点4箇所に置き直した:
  1. `ime_controller.rs::ImeController::apply`の先頭（同期経路の唯一の合流点。
     `apply_ime_open_with_view`経由の全呼び出し元と、`key_pipeline.rs`が直接呼ぶ経路の
     両方を覆う）
  2. `runtime/open_chain.rs::run_open_chain_async`の先頭（非同期経路の入口。呼び出し元の
     `imm_cross_is_first_applicable`判定でInputRelayは通常ここに来ないが、判定と実行の
     間のrace防御として）
  3. `runtime/open_chain.rs::fallback_write`（非同期chainのImmCross失敗後フォールバック。
     各機構ごとにviewを作り直す設計のため、await中にフォーカスがInputRelayへ移った場合を
     ここで再検出する）
  4. `runtime/open_chain.rs::imm_cross_write`（`/code-review`指摘で追加。
     `AsyncChainWriter::is_applicable(ImmCross)`は`self.imm.is_some()`しか見ておらず
     profileを一切参照しないため、2の`run_open_chain_async`冒頭gateが`with_app`再入
     失敗でfail-openした場合、修正前はこの関数まで無条件でImmCross writeが到達して
     いた。3の`fallback_write`と同じfresh view再検出をImmCrossにも及ぼす）

  `runtime/executor.rs::dispatch_ime_set_open`の既存gateは早期exitの最適化として残す
  （上記4箇所と重複するが無害）。いずれも`ImmCrossProcessStrategy`/`GjiDirectStrategy`/
  `MsImeDirectStrategy`のいずれも試行しない。
- **(b) 物理IMEモードキーをsuppressしない**: `transport.rs::plan`に決定1と同型の分岐を
  追加。当初「condition (b)の担保」として想定していた`AppImeProfile::should_pass_physical_
  key`述語は、コードレビューで**本番呼び出し元ゼロのデッドコード**（BUG-46修正で`plan`から
  外されて以来、誰も呼んでいない）と判明したため、実装は`plan`への直接分岐に置いた。
  **この分岐は当初F2 (VK_DBE_HIRAGANA) 分岐より後ろに置かれており、`/code-review`で
  「`is_tsf_mode`/`f2_warmup_owned`がどちらも真の状態でInputRelay windowにフォーカス
  すると、F2分岐が先に評価されて物理かなキーがSuppressされうる」という理論上の抜け穴が
  指摘された。InputRelayチェックを`plan`の最上部（F2分岐より前）へ移動して修正。**
- **(c) この窓由来のopen観測をbeliefに取り込まない**: `AppImeProfile::can_read_imm32_open_
  status(InputRelay) = false`にするだけで、`state/observation_store.rs::FocusProbeOpenStatus
  ::classify`が自動的に`NotObservable`を返す（`ObservedOpenValue`はフィールドprivateで
  `Read`分岐からしか構築できないため、コンパイラが「観測を偽装したbelief書き込み」を防ぐ、
  `ime-belief-architecture.md`段1と同型の保証）。**intentの昇格は止めない**——中継ウィンドウ
  滞在中もローカルのNICOLA変換エンジンは動作しており（実データで確認: local側報告のjournal
  でConsume 22件）、エンジンのactivationは`effective_open()`に依存するため、intent自体を
  止めるとローマ字変換の誤爆という別の実害が出る。

`effective_open()`は「追跡を止める」のではなく「開ループにする」——新しい「不明」状態は
導入せず、`effective_open()`は従来どおり`bool`を返し続け、消費者約10箇所は変更不要。
「観測が入ってこない」＝書き込みが起きないだけであり、`desired_open`は入場前の値を自然に
保持する。

`ImePolicyProfile`（`state/ime_event.rs`、`caps()`の引数型）には**variantを追加しない**。
`From<AppImeProfile> for ImePolicyProfile`は`InputRelay => Self::ImmCross`に写す（`Plain`/
`Unknown`と同じ「到達しない安全既定」パターン。actuationの合流点4箇所（上記）のgateが
先に効くため、この写像先のchainは実行時に使われない）。これにより`caps()`・
`ime_profile_driver.rs`・`ALL_DRIVERS`/`ALL_PROFILES`は無変更で済んだ。

### `ImeOpenOutcome::NotOwned` の新設

`src/platform.rs`（ルートクレート）の`ImeOpenOutcome`に`NotOwned`を追加した。既存の
`UnsafeToToggle`（「送っていない」ためapplied/beliefを書かない、という既に確立された
パターン）と全く同じ扱いにする。「試して失敗した＝逆状態と確信」（`Failed`）と「送って
いない＝不明のまま」（`UnsafeToToggle`/`NotOwned`）という2クラスは既にこのコードベースに
存在しており、決定4は後者に属する。`ApplyError`（`state/ime_event.rs`）にも専用の
`NotOwned` variantを追加し、`UnsafeToToggle`とは共用しない——共用すると「shadow信頼度不足
で送らなかった」という別の理由を名乗ることになり、`ime-belief-architecture.md`が禁じる
意味論的偽装の軽い版になる。

### disable_appsとの違い（BUG-90 NO-GOとの整合性確認）

BUG-90調査当時（2026-08-26）、`powertoys.mousewithoutbordershelper.exe`を`app_overrides.
disable_apps`（既存の丸ごと無効化機構）の既定リストに追加する案がOpus敵対的レビューで
NO-GO判定されている。理由: (1) 中継ウィンドウでは実際に親指シフト入力が機能しており、
disable_appsで丸ごと無効化すると動いているワークフローを壊す (2) report2ではDBEキー
到達時にMWBがフォーカスを持っておらずdisable_appsは発火せず効果がゼロ。

`input_relay_apps`はこの2点いずれにも該当しない: (1) はエンジン全体を無効化する
disable_appsの性質に起因する懸念であり、`input_relay_apps`はIME制御の非所有のみで文字
変換自体は動き続けるため該当しない。(2) は決定1が対象とするリモート側の症状についての
指摘であり、`input_relay_apps`はローカル側の別メカニズムに対する修正なので該当しない。

## 未解決事項

### リンク(a): フックチェーン順序への依存

`hook.rs`により、awaseのLLフックは常に`LRESULT(1)`を返す（`CallNextHookEx`を呼ばない）。
決定1/決定4の`Allow`は「後段フックへ生イベントを流す」ことを意味せず、「engineスレッドが
`SendInput`で（`LLKHF_INJECTED` + awase自身のマーカー付きで）reinjectする」ことを意味する。
したがって**MWBのフックがawaseより前段か後段かで、実際に中継されるかどうかが変わりうる**。
決定1・決定4のどちらも、この構造的な外部条件に対する非依存性は主張できない。

実装前提条件として、実機1回（決定4を入れてローカルで英数を押し、リモートjournalに
`VK_DBE_ALPHANUMERIC injected=true`が届くか）の確認が必要。これはWindows実機が必要で
開発時のLinuxサンドボックスでは実行できなかった。issue #136の報告者に検証を依頼する。

### MWBの実際のプロセス名

MWBは複数プロセス（`PowerToys.MouseWithoutBorders.exe`本体/`...Helper.exe`/サービス側）で
構成されており、実際にフォアグラウンドウィンドウを持つのがどれかは実機でしか確定できない
（R2から取得したjournalでは`powertoys.mousewithoutbordershelper.exe`だったが、これが常に
正しいか未確認）。このため`input_relay_apps`の既定値は**空配列（オプトイン）**とし、
issue #136への回答では利用者に`[app_overrides] input_relay_apps = ["powertoys.
mousewithoutbordershelper.exe"]`を設定に追加するよう案内する。プロセス名の実機確認が
済んだら、既定値を変更する別コミットを起票する（本ADR・本PRのスコープには含めない）。

### かな(0xF2) × リモート側TsfNative × f2_warmup_owned=true

決定1はF2分岐に意図的に触れないため、この組み合わせでは`VK_DBE_HIRAGANA`が依然Suppress
される。R2から取得した実データにはこの組み合わせのケースが含まれておらず、実害の有無
自体が未確認である。

### `resolve_open_at`の`DesiredFallback`枝

`state/ime_model.rs::resolve_open_at`は、明示意図なし・観測ゼロの場合`desired_open`を
そのまま返す（`BaseDecision::DesiredFallback`）。中継ウィンドウ滞在中に書かれた
`desired_open`は、`FocusChanged`（PID変化）で`last_intent`が無条件クリアされるため、
通常は戻り先の観測に上書きされる（`resolve_open_at_desired_fallback_carries_relay_
desired_value_without_observations`テストで固定）。ただし戻り先に観測が一件も無い
（`derive_any()`/`most_recent_trusted()`が共に`None`）場合に限り、relay滞在中の
`desired_open`がそのまま戻り先で表面化する。これは決定4固有の欠陥ではなく
`DesiredFallback`枝の既存の性質であり、対処は本ADRのスコープに含めない。

### dwExtraInfoによる自己注入の判別が構造的に不可能

PowerToys（Microsoft OSS）のMouse Without Borders実装
（`src/modules/MouseWithoutBorders/App/Class/InputSimulation.cs::SendKey`）を確認した
結果、`dwExtraInfo`には`NativeMethods.GetMessageExtraInfo()`をそのまま渡しており、
独自の識別マーカーは一切付けていない。したがって、**MWB由来の中継イベントとMS-IME/CTF
自身の自己注入イベントを`dwExtraInfo`で区別することは構造的に不可能**である。次に誰かが
「マーカーで判別すればいい」と思いつくのを止めるため、ここに事実として記録する。

## 参考: 全打鍵中継ツールに対する`should_reprime_on_lightweight_focus_sync`の扱い

`should_reprime_on_lightweight_focus_sync`は`cannot_verify_real_ime_state && belief_
effective_open`の導出値であり、`cannot_verify_real_ime_state(InputRelay) = true`にした
結果、`InputRelay`に対して自動的に`true`になる。中継ウィンドウでの再プライムは「無害な
空振り」で済む可能性が高いという想定のもと、分割によるコード複雑化を避け、導出のまま
受け入れることにした。実機ソークで余計な再プライムの実害が観測されたら、
`cannot_verify_real_ime_state`を「belief読み取り可否」と「再プライム要否」の2つの意味に
分割することを検討する。

## 検証（evidence）

Linuxで実行可能なもの（`ime_controller.rs`/`open_chain.rs`は`#[cfg(windows)]`のため
Windows CIでのみ実行、それ以外はLinuxで`cargo test -p awase-windows`実行可能）。

- `crates/awase-windows/src/runtime/transport.rs::plan_tests` — injected×DBE系KeyDown/
  KeyUp→Allow、非injected→従来通りSuppress、injected×F2はTsfNative+f2_warmup_owned下で
  Suppressのまま（決定1がF2分岐に影響しない回帰ガード）、ImmCrossアーム貫通の明示的固定、
  InputRelay×KANJI系→Allow、`input_relay_f2_is_allowed_even_when_tsf_warmup_flags_are_
  true`（`/code-review`で発見されたF2分岐順序バグの直接の回帰テスト。`is_tsf_mode`/
  `f2_warmup_owned`を両方trueにしてもInputRelayなら`Allow`のまま）
- `crates/awase-windows/src/focus/class_names.rs` — 述語4つ+自由関数3つの`InputRelay`
  期待値固定、`from_class_and_process`の優先順、既存の`app_ime_profile_getters_truth_
  table`/`exhaustive_cluster_matches_independent_oracle`の拡張
- `crates/awase-windows/src/state/observation_store.rs` —
  `input_relay_profile_makes_open_status_not_observable_even_with_a_real_reading`
- `crates/awase-windows/src/state/ime_model.rs` —
  `matching_not_owned_failure_consumes_pending_without_writing_applied`、
  `repeated_input_relay_focus_roundtrips_do_not_leave_pending`（50往復でpending残留なし）、
  `resolve_open_at_desired_fallback_carries_relay_desired_value_without_observations`
- `crates/awase-windows/src/ime_controller.rs`（Windows-only）—
  `apply_returns_not_owned_for_input_relay_without_attempting_any_mechanism`。
  `ImeController::apply`（同期経路の唯一の合流点）がInputRelayでどの機構も試行せず
  `NotOwned`を即返すことを固定する。上記の gate バイパス修正の直接の回帰テスト。
- `crates/awase-windows/tests/architecture_guard.rs` —
  `input_relay_profile_wiring_occurrence_counts_are_pinned`（`AppImeProfile::InputRelay`
  の本番コード出現数をファイル別に固定。`production_code_only`ヘルパーが`mod tests`
  という名前しか認識せず`transport.rs`の`mod plan_tests`を素通りする欠陥がコード
  レビューで発覚したため、本テスト専用に汎用的な`strip_any_test_module`を実装）
- `src/config.rs` — `input_relay_apps`の既定値（空配列）・追加・カスタムリスト・空文字列
  警告

`/code-review`で追加指摘された下記2点は専用の回帰テストを付けていない（挙動そのものの
分岐は既存テストが間接的に踏むが、poison/dedup自体は再現困難なため、ここに文書化する）:

- `crates/awase-windows/src/focus/classifier.rs` —
  `INPUT_RELAY_APPS`（`RwLock<Vec<String>>`）の読み書きが`PoisonError`時に空配列へ
  静かにフォールバックしていた（`input_relay_apps_snapshot`）/書き込みを諦めていた
  （`ForceOverrides::new`）のを、`PoisonError::into_inner()`で中身を保持する方向に
  変更。`Vec<String>`への単純な`clone_from`はpanic時も複合的な不変条件を持たないため、
  poison直前の中身をそのまま使い続けて安全という判断。
- `crates/awase-windows/src/focus/class_names.rs::AppImeProfile::resolve` —
  `ime.rs::read_ime_state_fast`と`runtime/mod.rs::on_window_focus_event`にあった
  「relay_apps空ならprocess_name解決を省略する」という同一分岐（重複コード）を統合。
- `crates/awase-windows/src/focus/classifier.rs` — `INPUT_RELAY_APPS`プロセスグローバル
  の唯一の正当な利用者を`ime.rs::read_ime_state_fast`（`self`を持たない`pub unsafe fn`）
  に絞り、`runtime/mod.rs::on_window_focus_event`は正規ルート
  （`FocusTracker::input_relay_apps()`）に変更。あわせて`ime.rs`/`runtime/mod.rs`の
  両呼び出し元で、`input_relay_apps`が空（既定）の場合に`get_process_name`（Win32
  ハンドルを開くコスト）を呼ばない早期スキップを追加し、既定設定のユーザーへの
  レイテンシ回帰を防いだ（コードレビュー指摘）。

`docs/known-bugs.md`にBUG-90追補として記載。`tuning.rs`に差分なし（`tuning-constants.md`
非該当）。revertではない（`experiment-logging.md`非該当）。
