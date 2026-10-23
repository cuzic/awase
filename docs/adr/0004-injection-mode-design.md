---
id: ADR-0004
title: |-
  InjectionMode 三分岐設計
status: |-
  撤去済み(2026-10-22): Unicode 分岐は ADR-276(2026-10-21)、Tsf 分岐と `force_tsf` は 2026-10-22 に撤去し、`InjectionMode` は型ごと無くなった(注入は Batched VK だけ。末尾の追補)。旧: 採用・実装済み(InjectionMode の Unicode/Vk/Tsf 3分岐は state/injection_mode.rs に現存、2026-10-04 確認)。旧: 安定
related_adr: []
---

# ADR 0004: InjectionMode 三分岐設計

> **2026-10-21 追記:** Unicode 分岐は [ADR-276](276-remove-unicode-injection.md) で撤去した。現在の `InjectionMode` は Vk / Tsf の 2 つで、`force_tsf` 以外は Vk。以下は撤去前の設計記録。
>
> **2026-10-22 追記:** Tsf 分岐と `force_tsf` も撤去し、`InjectionMode` は型ごと無くなった(末尾の追補)。
>
> **2026-10-22 追記:** 所有者判断(2026-10-22)で、`config.toml` の `app_overrides.force_vk`(`InjectionHint::ForceVk`)を退役させ、`cache.toml` の `[injection_mode]`(`InjectionModeStore`)の読み込みを撤去した。ADR-276 で既定が全部 Vk になり、自動昇格の書き込み側も無くなっていたため。`InjectionHint` は `ForceTsf`/`Default` の 2 値で、`injection_mode_for` は `ForceTsf` → Tsf、`Default` → Vk。既存の設定は壊さない: `force_vk` は読み込みエラーにせず無視し、空でなければ起動時に「不要になった」と通知する(`config_load_diag::REMOVED_WITH_VALUE_NOTICE`、設定画面の保存で消える)。`[injection_mode]` は読まない(`ImmCapabilityStore` の保存は他セクションとして保つ)。**影響**: 以前の版が TSF へ自動昇格させていた窓(`force_tsf` に無いもの)は Vk で送られる。困る場合は `force_tsf` に足す。

**Status:** 安定（2026-05-19 現在）  
**関連コミット:** `3e444b4`, `50c5b4b`, `a444984`, `73b69ebb`, `73ddbb8`, `41d8fee`, `a57fc81`, `281cebd`

---

## Context

日本語入力（ひらがな→漢字変換）のキー注入方法はアプリごとに異なる。
普通の Win32 アプリ、Chrome 系、WezTerm/TSF ネイティブの3種類で
それぞれ最適な注入戦略が異なる。

---

## 選択肢と採用理由

### Unicode 直接注入（Win32 / UWP デフォルト）

`KEYEVENTF_UNICODE` で文字コードを直接送信。IME を経由しない。
ひらがな文字そのものを unicode として注入するため確実。

**採用ケース:** 通常の Win32 アプリ、UWP アプリ。

### VK Batched 注入（Chrome / Edge / Electron）

ローマ字を VK キーストロークとして送信。Chrome 内部の IME パイプラインを通す。
送信順序: **全 KeyDown → 全 KeyUp（重畳順）**

**採用理由:** Chrome の IME は WM_KEYUP でコンポジションをコミットするため、
Sequential（M↓M↑ U↓U↑）では M↑ 到達時点で IME が 'm' を単独確定してしまう。
Overlapped（M↓U↓ M↑U↑）なら一組として処理される。

### VK Sequential 注入（WezTerm / TSF ネイティブアプリ）

ローマ字を VK キーストロークとして送信。送信順序: **M↓M↑ U↓U↑（逐次順）**

**採用理由:** WezTerm の TSF 実装は Overlapped 順では "mう" になる（`73ddbb8` 2026-04-29）。
後に Batched と同じ順序に統一（`41d8fee` 2026-04-30）した経緯あり。

---

## 決定の変遷

### 初期（`3e444b4` 2026-04-01）

`Chrome → VK`、`それ以外 → Unicode` のシンプルな二分岐。

### WezTerm の分類変更を巡る試行錯誤（★★★）

```
WezTerm: Unicode → force_vk=true で Chrome 扱い (a57fc81 2026-04-21)
  → force_vk を削除し Unicode 注入へ移行 (3b69ebb 2026-04-29)
  → TSF Sequential モード追加 (73ddbb8 2026-04-29)
  → TSF Batched（Chrome と同じ重畳順）に変更 (41d8fee 2026-04-30)
```

WezTerm は IMM32 ブリッジを持つが TSF で IME を処理するため、
Unicode 直接注入では変換できない。かといって Chrome と全く同じ挙動でもない。
結果として `Tsf` という専用モードが生まれた。

### 現在の三分岐（`281cebd` 2026-04-01 〜）

```rust
enum InjectionMode {
    Unicode,  // Win32/UWP デフォルト
    Vk,       // Chrome/Edge/Electron — IME composition 経由
    Tsf,      // WezTerm — TSF 直結アプリ向け
}

fn resolve_injection_mode() -> InjectionMode {
    // 1. config の focus_overrides.force_tsf → Tsf
    // 2. config の focus_overrides.force_vk  → Vk
    // 3. AppKind::Chrome                     → Vk
    // 4. それ以外                            → Unicode
}
```

`force_tsf` / `force_vk` config オーバーライドにより、
誤検出アプリをユーザーが手動で修正できる。

---

## Consequences

**良い点:**
- アプリごとの IME パイプラインの違いを吸収する適切な抽象
- config override により edge case をユーザーが修正可能

**トレードオフ:**
- AppKind の自動検出が誤った場合に出力が壊れる
- Tsf モードと Vk モードの境界が明確ではなく、新しいアプリで分類が不確か

## 追補(2026-10-22): Tsf 注入経路(`force_tsf`)の撤去

所有者判断(2026-10-22)で、Tsf 注入経路(VK Sequential、`InjectionMode::Tsf`)と `force_tsf` を撤去した。注入は Batched VK の 1 つだけになり、`InjectionMode`・`InjectionHint` は型ごと無くなった。

**Tsf が必要だった理由は消えていた:** `force_tsf` は、当時の既定(Unicode 注入)が IME の composition を通らず、WezTerm・Windows Terminal で漢字変換(候補窓)が出なかったために VK で送る指定として入れた(`1268878d`・`4e49ee1`)。ADR-276 で既定が Vk になり、この理由は無くなった。

**CI の測定(`docs/tasks/mech-ecrs-2026-10-21/measure-no-force-tsf-result.md`、ブランチ `docs/measure-no-force-tsf`、PR #807):** 実 Windows Terminal と、窓クラスを Windows Terminal にした自前の RichEdit(`typing_stress --form=tsfnative`)で、現状(Tsf)・Vk(`force_tsf = []`)・Vk + TsfGate の保留なし の 3 通りを、GJI・MS-IME、連続打鍵・cold・12 秒停止・打鍵途中の停止・フォーカス往復で並べた。崩れ(欠け・リテラル化・入れ替わり・置き換わり)は実 WT で 240/240/240 試行とも 0、tsfnative で 1,044/1,044/1,008 試行で 0/0/1(保留なしの 1 件は IdleCheck の一過性の読み取り。因果は未確認で、12 回の再実行で再現せず)。composition(未確定文字)は 3 通りとも同じに作られた(`sc-ntsf-preedit-*`)。

**消したもの(`crates`・`src`・`tools`・`.cargo` で約 2,600 行の純減):** `tsf/tsf_gate.rs`(TsfGate、フォーカス直後のキー保留、`TIMER_TSF_GATE`)、`tsf/warmup/gji_warmup_coro.rs`(`GjiWarmupCoro`)、`tsf/warmup/cold_warmup.rs`(`ColdWarmupSequence`、Tsf の cold 開始と ROMAN ビットの保護書き込み)、`output/vk_send.rs` の `send_romaji_as_tsf*`・`send_char_as_tsf`・`TsfSendPipeline`、`output/sender.rs` の `TsfSender`(送信口のトレイトごと)、`output/types.rs`・`state/injection_mode.rs`、`probe_fsm.rs` の `is_tsf_mode`・`decide_transmit_plan`・`ProbeObservations`・`TransmitTarget`、`ProbeIo` の `gate_is_bypass`・`transmit_tsf`・`send_single_tsf_vk`、`GjiEvent::NativeF2Consumed`・`ColdReason::NativeF2Consumed`・`StageEndReason::GateBypass`、`GjiFsm` の注入モード(`LongIdle` は常に `CHROME_LONG_IDLE_MS`)、`tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE`、`AppOverrides::force_tsf`。

**挙動の変化:**
- WezTerm・Windows Terminal(既定の `config.toml` が `force_tsf` に入れていた)も、ほかの窓と同じ Batched VK で送る。cold 開始は `TsfProbeCoro::new_chrome` が一手に担う。
- フォーカス直後(最大 500ms)のキーの保留(TsfGate の PendingWarmup)が無くなった。**これは Tsf 窓だけでなく全アプリの挙動変更**(TsfGate は窓の種類に関係なくフォーカスイベントのたびに PendingWarmup になり、新しい窓の IME リフレッシュが済むまで打鍵を保留していた。PR #809 レビュー M1)。撤去後は、この間(典型 60〜150ms)の打鍵が前の窓の belief のまま処理されうる。所有者判断(2026-10-22)で今回は戻さず、実機(dragonflyg4)で窓切替直後の打鍵を測って要否を決める。
- raw リテラルの回収再送(`send_romaji_recovery`、旧 `send_romaji_dispatching_on_gate`)は、ゲートの状態によらず常に Batched VK で送る。
- GJI 系では、物理 F2(ひらがな)は warm のとき cold にしない(旧 Tsf だけは warm でも `NativeF2Consumed` で cold にしていた。BUG-31 の方針に揃った)。
- Windows Terminal・WezTerm の `GjiFsm` の `LongIdle`(warm から cold へ落ちるまでの無操作時間)が 10 秒(`LONG_IDLE_MS`)から 5 秒(`CHROME_LONG_IDLE_MS`、`measured(pending)`)になった。あわせて cold 開始は `TsfProbeCoro`(`needs_literal = gji_active`)なので、GJI では cold のたびに 1 VK ずつ確認する(旧 `GjiWarmupCoro` は NameChange を確認できたら一括送信していた)。測定の「12 秒停止」は両方の閾値を超えるので、5〜10 秒の停止の差は見ていない(PR #809 レビュー m3)。
- ADR-270(Mozc の最初の送信の確認)は Tsf の送信からしか呼ばれていなかったので、Batched VK の `Transmit`(`gji_active` でないとき = Mozc)へ移した。**張る窓は旧来の範囲(前景窓が Windows Terminal・WezTerm、`is_mozc_compose_check_window`)に限る**(PR #809 レビュー M3): 期限 0.3 秒の根拠は Windows Terminal の 1 台だけで、Chrome は TSF composition context の再初期化に ~326ms かかる実測(`79134f5`)があり、遅れてできた入力中の文字を Backspace が消して確定済みの文字まで消しうる。Chrome 等へ広げるのは、その窓で `[mozc-compose] ... confirmed after Nms` の分布を取ってから。期限の定数は実測が無いので動かしていない。旧来は利用者が `force_tsf` に足した任意の窓(Alacritty など)でも確認が張られたが、今は固定の 2 クラスだけで、旧 `force_tsf` に足した独自の窓は対象外になる(PR #809 再レビュー N4)。物理 F2 で warm でも cold にするのも同じ述語(`mozc_compose_check_applies`、Mozc 系 × この 2 窓)に揃え、確認の張られない窓には無益な保持を入れない(N1)。
- 物理 F2(ひらがな)は、warm でも Mozc 系(`gji_evidence_absent`)で前景窓が Windows Terminal・WezTerm なら cold にする(`tsf::cold_reason::physical_f2_marks_cold`)。BUG-209 の再現経路はひらがなキーの直後で、ADR-270 の確認は cold の送信でしか張られないため(旧 Tsf は `NativeF2Consumed` で cold にしていた。PR #809 レビュー M2)。
- Tsf の cold 開始が行っていた変換モードの ROMAN ビットの保護書き込み(`set_ime_conv_for_target(target, None)`、`conv_mutation_allowed` のときだけ)は無くなった。
- 既存の設定は壊さない: `force_tsf` は読み込みエラーにせず無視し、空でなければ起動時に「不要になった」と通知する(`config_load_diag::REMOVED_WITH_VALUE_NOTICE`。`force_vk` と同じ形)。`cache.toml` は触らない。

**確かめていないこと:** WezTerm(CI に無い)。TsfGate の保留が効く状況(フォーカス直後 500ms 以内の打鍵)は測定で 1 件も作れていない(`tsf-gate-hold` 0 件)。候補窓の表示は観測できていない(composition ができることで代えた)。窓切替直後(典型 60〜150ms)の打鍵の扱い(M1)。Mozc Readable + Windows Terminal のひらがなキー直後の確認(M2 で cold にするようにしたが実機は未確認)。実機(dragonflyg4)の Windows Terminal・WezTerm での確認が要る。
