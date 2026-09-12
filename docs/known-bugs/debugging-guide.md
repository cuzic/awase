---
id: debugging-guide
title: |-
  デバッグ方法
type: reference
---

# デバッグ方法

ログ出力（`RUST_LOG=debug`）で以下のキーワードを確認する:

| ログキーワード | 意味 |
|---|---|
| `[composition] marked cold reason=X idle=Yms` | cold-start 発生。reason と idle 時間を確認 |
| `[h1-probe] cold=N long_idle=B f2_gji_long_idle=B idle_at_cold=Xms min=Yms max=Zms` | Chrome probe パラメータ |
| `[h1-warmup] cold=N eager_settle_ms=Xms probe_min_ms=Yms reason=Z` | WezTerm TSF probe パラメータ |
| `[tsf-probe] cold=N ChromeProbe 完了 → batched 送信 (Xms)` | Chrome probe 完了・経過時間 |
| `[tsf-probe] cold=N GjiProbe 完了 (Xms, gji_idle=Yms, settled=B)` | GJI probe 完了 |
| `[tsf-probe] cold=N NameChangeWait → nc_fired=B timed_out=B` | NameChangeWait 状態 |
| `[raw-tsf-literal] cold=N composition confirmed` | LiteralDetect: 正常 composition 判定 |
| `[raw-tsf-literal] cold=N raw TSF literal suspected → BS ×N` | LiteralDetect: literal 疑い → リカバリ |
| `[gji-candidate] SHOW #N` / `HIDE` | GJI 候補ウィンドウ表示/非表示 |
| `[gji-poll] GJI I/O Xms ago predates focus change` | GJI が focus change より前に静止 |
| `[composition] marked warm (epoch=N)` | probe 完了・warm 確定 |
| `[hook] IME-mode vk=0xXX dir self_injected=B injected=B scan=0xXX extra=0xXX` | IME モードキー到達診断（injected=LLKHF_INJECTED、BUG-08/BUG-14 の注入元切り分け） |
| `[hook] foreign-injected VK_KANA dir を swallow` | 外部注入 VK_KANA の遮断（BUG-08 防御。VK_KANA 以外の swallow は BUG-14 で撤回済み） |
| `[shadow-toggle] injected IME キー vk=0xXX はユーザー意図に昇格させない (BUG-14)` | 外部注入 IME モードキーの意図昇格ガードが発動（OS への配送は維持） |
| `[shift-conv-guard] Shift 押下 → IME-ON 半角英数へ切替` | Shift conv 安全網 entry（BUG-15/BUG-25。安全網ブリップか持続トグルの開始か、直後の `[shift-conv-guard] 左Shift単独タップ → 半角英数トグルON` の有無で判別） |
| `[shift-conv-guard] 左Shift単独タップ → 半角英数トグルON (conv=0x0000 維持)` | BUG-25: 左Shift単独タップで持続トグル開始（復元をスキップ） |
| `[shift-conv-guard] かな入力へ復元` | BUG-15/BUG-25: conv をかな入力へ verify-retry 復元（安全網ブリップの終了、またはトグルOFF） |
| `[tip-detect] IME kind candidate X (current=Y), awaiting confirmation next tick` | CLSID 種別フリップの1回目の観測（`ImeKindDebounce`）。次 tick も同じなら確定、元に戻れば破棄 |
| `[tip-detect] IME kind → X` | CLSID 種別変化が2 tick連続で確定し `WM_IME_KIND_CHANGED` を発行（`GjiFsm`/`MsImeStrategy` が再構築される点に注意、BUG-17） |
| `stale confirm 検出` / `epoch-fence-stale` | ADR-079/BUG-35: confirm 根拠が前世代由来と判明（追補1で SuspectedLiteral と同じ backspace+再送に変更済み。追補2で「既に可視」ショートカットの猶予漏れによる false positive も修正済み。追補3で `CompositionReset`/`NativeF2Consumed` 自体に `gji_idle_ms` observation ゲートを追加し前提条件面を根治。追補4で backspace 自体を送らない（romaji 再送のみ）方式に変更、literal の positive な証拠がない限り BS を送らない） |
| `[literal-detect] cold=N セッション確認済み → スキップ` | BUG-39: `literal_session_confirmed(N)==true`（確認済み世代が現在の `cold_seq=N` と一致）のため reactive literal-detect 自体をスキップ。修正後は世代不一致で自動的に無効化されるため、このログの `cold=N` は必ず「実際にその N で確認が取れた」世代のはず — もしこの N で一度も `[literal-detect] cold=N composition confirmed` 相当のログが無いのに出ていたら回帰を疑う |

---
