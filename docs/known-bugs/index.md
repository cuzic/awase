# awase 既知の不具合 — 索引

> 1件1ファイルに分割済み。各ファイル先頭の frontmatter に完全なタイトル文字列・関連コミット・関連ADRを保持する。
> ここでの「概要」列は元タイトルの機械的な先頭切り出し（意味的な要約ではない）。判断が必要な場合は必ずファイル本文を開くこと。

## BUG一覧

| BUG | 概要 |
|---|---|
| [BUG-001](BUG-001.md) | TSF cold-start — probe バジェット超過で1文字目がリテラルになる (WezTerm) |
| [BUG-002](BUG-002.md) | Chrome cold-start — probe タイミング想定外で1文字目がリテラルになる |
| [BUG-003](BUG-003.md) | LiteralDetect 偽陽性（false positive CompositionConfirmed） |
| [BUG-004](BUG-004.md) | GJI モニター切断時のフォールバック |
| [BUG-005](BUG-005.md) | SessionExpired 閾値 (2000ms) が任意値 |
| [BUG-006](BUG-006.md) | focus_epoch のオーバーフロー ~~（解消済み）~~ |
| [BUG-007](BUG-007.md) | Edge/Chrome フォーカス約500ms後に Engine が必ず OFF になる（偽 FocusProbe 観測） |
| [BUG-008](BUG-008.md) | 外部注入 VK_KANA によるかなロックトグルで JIS かな入力化（GJI/Windows Terminal） |
| [BUG-009](BUG-009.md) | post_to_main_thread の誤配送 — WM_IME_KIND_CHANGED / WM_FOCUS_KIND_UPDATE がワーカースレッドから main に届か… |
| [BUG-010](BUG-010.md) | MS-IME で物理ひらがなキー（VK_DBE_HIRAGANA）が食い逃げされ IME ON にならない |
| [BUG-011](BUG-011.md) | UIA 結果のキャッシュキー取り違えで Edge が永久 NonText（全キーがエンジン素通し） |
| [BUG-012](BUG-012.md) | UIA 非同期 focus 分類の適用を無効化（(pid,class) キャッシュ粒度がブラウザと構造的に不一致） |
| [BUG-013](BUG-013.md) | MS-IME cold start — IME ON 遷移直後の送信で先頭文字がリテラル化（「を」→「wお」） |
| [BUG-014](BUG-014.md) | 外部注入 VK_DBE_HIRAGANA を物理かなキーと誤読し、ユーザーの IME OFF を Engine ON で上書きし続ける |
| [BUG-015](BUG-015.md) | Shift 面使用後の Shift 解放で MS-IME が英数モードに落ち、かな入力が数秒壊れる |
| [BUG-016](BUG-016.md) | フォーカス遷移の settle スキップに再試行がなく、belief ON × 実 IME OFF が放置される |
| [BUG-017](BUG-017.md) | CLSID ベース IME 種別の単発フリップで GjiFsm が丸ごと再構築され、Chrome 入力中に cold が単語ごとに発火し続ける |
| [BUG-018](BUG-018.md) | 無操作中の AppKind (TsfNative⇔Uwp/InputSite) 往復後、再開直後の入力が部分欠落する（修正済み） |
| [BUG-019](BUG-019.md) | 一発だけのカタカナ conv 誤読を warmup が鵜呑みにし、GJI が実際にカタカナへ固定される（修正済み） |
| [BUG-020](BUG-020.md) | ドリフト補正の再送が non-ImmCross アプリで no-op のため IME ON / Engine OFF が固定化する（修正済み・実機検証待ち） |
| [BUG-021](BUG-021.md) | Chrome の cold-start 復帰処理が重症度 (Short/Medium/Long) を無視し、確定キー/IME再有効化のたびに過剰発火する |
| [BUG-022](BUG-022.md) | MS Edge で Uwp⇔TsfNative フォーカス往復後、conv=Eisu(英数) に固着し nicola が入力できなくなる |
| [BUG-023](BUG-023.md) | 画面ロック中に離された修飾キーの KeyUp が失われ、Shift/Ctrl が恒久的に stuck する（修正済み・実機再現確認待ち） |
| [BUG-024](BUG-024.md) | `is_partial_literal()` が romaji 自体の compose 結果ではなく warmup F2 への |
| [BUG-025](BUG-025.md) | 左Shift単独タップによる「IME-ON 半角英数」持続トグル（BUG-15 hold方式の置換） |
| [BUG-026](BUG-026.md) | FocusChanged 直後 conv が既に NATIVE の場合、idle-conv-check の steady-state 分岐が engine 復帰を永久に見送る |
| [BUG-027](BUG-027.md) | per-VK confirm ループが `vk_sent 未設定` を検出すると、リカバリなしで romaji（と巻き込んだ後続文字）を丸ごと失う |
| [BUG-028](BUG-028.md) | `flush_raw_tsf_literal_recovery` が `pending_gji_key_responses` を drain せず、`StartProbe` が数秒… |
| [BUG-029](BUG-029.md) | Chrome per-VK confirm が VK1 以降を誤って `SuspectedLiteral` 判定し、 |
| [BUG-030](BUG-030.md) | `LiteralDetectCore::poll`（`run_per_vk_confirm` 以外の literal-detect 経路）が候補ウィンドウ可視でも SHOW イベン… |
| [BUG-031](BUG-031.md) | `NativeF2Down`（非 TSF）が warm 中でも無条件に cold-mark し、連続 typing の1文字を無用な per-VK confirm レースに晒す |
| [BUG-032](BUG-032.md) | `send_vk_dbe_hiragana_pair` が Win キー押下中のスキップを送信成功と |
| [BUG-033](BUG-033.md) | `Imm32Unavailable` プロファイルでは drift correction が構造的に一度も発火し得ない（belief 自身を「観測」として書き戻す循環） |
| [BUG-034](BUG-034.md) | `SendMessageTimeoutW(SMTO_ABORTIFHUNG)` の `timeout_ms` 未保証により、エンジンスレッド上の同期 IME 読み取りが数秒ブロック… |
| [BUG-035](BUG-035.md) | per-VK confirm が世代をまたいだ stale な confirm 根拠を現世代の証拠として |
| [BUG-036](BUG-036.md) | `RawTsfLiteralRecovery` give-up が Chrome GJI reinit を backspace flush より先に送り、未確定 preedit が… |
| [BUG-037](BUG-037.md) | Ctrl+T 等の同一プロセス内フォーカス移動で IME belief が実状態と乖離しても、唯一の訂正手段（物理 IME キー）が no-op に握り潰される |
| [BUG-038](BUG-038.md) | `RawTsfLiteralRecovery` の give-up 分岐が `pending_deferred` を flush しないため、probe 実行中に届いた別の打鍵が消… |
| [BUG-039](BUG-039.md) | `literal_session_confirmed` が FocusChange・長時間 idle・アプリ切替をまたいで持ち越され、新しい cold セッションの先頭文字が li… |
| [BUG-040](BUG-040.md) | `nc_for_plan` が `gji_settled`（GJI probe の実測結果）を見ずに confirm-key ヒントだけで `nc_fired` を昇格し、genu… |
| [BUG-041](BUG-041.md) | `decide_alt_impersonation` が KeyUp 時点で「なりすまし発動中」フラグを stuck true のまま持ち越し、後続の無関係な Alt 押下まで m… |
| [BUG-042](BUG-042.md) | IME ON・Engine OFF から一切復旧できない（Ctrl+Shift+変換 が no-op、トレイ「状態をリセット」が誤ったウィンドウを対象にする） |
| [BUG-043](BUG-043.md) | `ir_apply_drift_correction`（Blacklist/TsfNative パス）が observation store を更新しないため、同じ IME-OFF… |
| [BUG-044](BUG-044.md) | `tray_wnd_proc` の「到達不能」判断が逆で、トレイ右クリックのコンテキストメニューが一切表示されなくなった |
| [BUG-045](BUG-045.md) | per-VK confirm の literal 判定が「代理指標のタイムアウト」に基づく belief であり、actual な TSF composition 状態と乖離しても… |
| [BUG-047](BUG-047.md) | `Vk`/`Tsf` 注入モードで記号（句読点「。」「、」・長音「ー」等）を送ると、cold-start ウォームアップ保護が無いため半角のまま出力される |
| [BUG-048](BUG-048.md) | `Engine::check_active_transition` の対称 `SetOpen` echo がユーザーの明示的な IME OFF 意図（`last_intent`）を… |
| [BUG-049](BUG-049.md) | 小指シフト面（物理 Shift）の全角記号が `shift-conv-guard` の conv 書き込みと競合し半角化する（BUG-47 とは別原因、Phase 1・Phase … |
| [BUG-050](BUG-050.md) | 一度カタカナに入ると IME-ON コンボを押しても永久に復旧できない（デッドロック解消・トリガーとも解消済み、詳細は追補参照） |
| [BUG-051](BUG-051.md) | TsfNative の drift correction が `TIMER_IME_REFRESH` の恒久停止で再起動されず、IME OFF で Engine ON のまま最大8… |
| [BUG-052](BUG-052.md) | `PhysicalKeyDisposition::plan` が `VK_DBE_KATAKANA` の KeyDown を「shadow_toggle 不発なら安全」として素通し… |
| [BUG-053](BUG-053.md) | Win キー押下時に検索UIが開くと KeyUp が失われ `PHYSICAL_KEY_STATE[VK_LWIN]` が恒久的にスタックし、以後 IME ON/OFF の実送信が… |
| [BUG-054](BUG-054.md) | `apply_force_on_for_imm_broken` の `conv_mode_policy=force` 経路が20msごとのVK_IME_ON無限再送ループに縮退し、… |
| [BUG-055](BUG-055.md) | `get_ime_wnd`/`set_ime_romaji_mode` が `GetForegroundWindow()`（トップレベル）基準の `ImmGetDefaultIME… |
| [BUG-056](BUG-056.md) | `learn_imm_capability_on_focus` が `ImmGetDefaultIMEWnd`=NULL を1回観測しただけで `Unavailable` を確定し… |
| [BUG-057](BUG-057.md) | `classify_ime_snapshot` の `OsPoll` 観測が `ime_on` を見ずに `conv` だけで英数(`ObservedEisu`)判定するため、一瞬… |
| [BUG-058](BUG-058.md) | 小指シフト面のチョード（Shift+数字等）が `OutputActiveGuard` と `shift-conv-guard` 復元の循環待ちに陥り、通常速度の打鍵でも毎回 ~5… |
| [BUG-059](BUG-059.md) | `ImeModeFsm::on_conversion_mode_read` が FocusChange 直後の cold 判定用ポーリング（1回読み）だけで `confirmed=… |
| [BUG-060](BUG-060.md) | `conv_mode_policy = force` 運用中に LINE で全打鍵が「い」になる／IME が JIS かなになる（**クローズ**: 前提機構が ADR-094 で… |
| [BUG-061](BUG-061.md) | Windows Terminal + MS-IME で JIS かな入力に固定され復旧できない（**解決不能と確定**: Win32 にローマ字/かな入力方式を外部から切り替える公… |
| [BUG-062](BUG-062.md) | 物理 Alt+VK_KANA（MS-IME の「ローマ字/JIS かな入力方式切替」ショートカット）を swallow して JIS かな固着を未然に防止（BUG-61 の根本原因… |
| [BUG-063](BUG-063.md) | 仮想デスクトップ切替後 Windows Terminal で半角のつもりが「くした」とかな変換される（IME belief と actuation の根拠が未分離） |
| [BUG-064](BUG-064.md) | config1.db に旧 awase 実験由来の残骸バインドが実在する（F13/F14/F21/F22、バグではなく既知の事実の記録） |
| [BUG-065](BUG-065.md) | `TSF_OBS_TEST_LOCK` 共有ロックが `.lock().unwrap()` で non-poison-resilient なため、1テストの真の失敗が無関係な10テ… |
| [BUG-066](BUG-066.md) | 全角ハイフンマイナス「－」が Chrome/Firefox 等（VK/TSF 送信経路）で長音「ー」に化ける（`build_symbol_to_vk` の VK_OEM_MINUS… |
| [BUG-067](BUG-067.md) | Alt 押下中の合成 `VK_DBE_HIRAGANA` 注入で MS-IME が JIS かな直接入力へ切り替わる（`kp_restore_kana_from_half_widt… |
| [BUG-068](BUG-068.md) | `Blind` drift correction の give-up 後再武装が「鮮度」を「新情報」の代理指標として使うため、TsfNative で短周期に再武装し VK_IME_… |
| [BUG-069](BUG-069.md) | `ir_post_focus_change_snapshot` が belief を `applied=Confirmed` へ偽装し、TsfNative の force-on /… |
| [BUG-070](BUG-070.md) | GJI 候補確定タイミングで eager warmup（`ConfirmKeyUp`）が GJI の `EndComposition` と競合し、`@` がリテラルとして漏れる |
| [BUG-071](BUG-071.md) | バージョンアップ時に `config.toml`/`layout/*.yab` が失われる（MSI の `MajorUpgrade` スケジューリング欠落 + ZIP アンインスト… |
| [BUG-072](BUG-072.md) | タスクトレイ「不具合を報告」ウィンドウの日本語が文字化け（トーフ表示）する |
| [BUG-073](BUG-073.md) | BUG-72修正の副作用で「不具合を報告」ウィンドウが背面のまま開き「一瞬表示されてすぐ消える」ように見える |
| [BUG-074](BUG-074.md) | `RawTsfLiteralRecovery` の give-up（2連続 raw-tsf-literal）で文字が痕跡なく完全に失われる — BUG-29 が予告していた「次回の… |
| [BUG-075](BUG-075.md) | `StaleConfirm` 回収が「先頭 VK は着弾していない」と無条件に仮定して romaji 全体を再送するため、着弾済みの子音が二重になり促音が増える |
| [BUG-077](BUG-077.md) | TsfNative でフォーカス復帰直後の最初のキーが resync 完了前に PassThrough でリテラル出力される（Alt+Tab 復帰直後の「rの」化） |
| [BUG-078](BUG-078.md) | リモートデスクトップ接続後にローカル側 Ctrl が押しっぱなしになる（Excel/iTunes で入力が壊れる） |
| [BUG-079](BUG-079.md) | awase.exe / awase-settings.exe にアプリケーションマニフェストが無いため、Windows のプログラム互換性アシスタント(PCA)が「管理者として実行… |
| [BUG-080](BUG-080.md) | 起動時・モーダルポンプ中のフックキー配送で打鍵が消える/順序が壊れる可能性 |
| [BUG-081](BUG-081.md) | bootstrap直後の初回フォーカスだけ定常のprocess_changed判定を通らない |
| [BUG-082](BUG-082.md) | トレイメニュー表示中のCtrl+C/--exit-afterでアプリが終了しない可能性 |
| [BUG-083](BUG-083.md) | /code-review(Opus敵対的レビュー)によるADR-105/102実装の追加是正5件 |
| [BUG-084](BUG-084.md) | Ctrl+prefix後のpost-bypass latchが別の前景窓の最初の1キーへ誤適用される |
| [BUG-085](BUG-085.md) | `dispatch_probe_actions` の早期returnがdeferred VKフラッシュとGjiFsm通知の両方を飛ばし、`pending_gji_warmup` が… |
| [BUG-086](BUG-086.md) | `EndComposition` が `ColdKind`/`ProbeParams` を固定値で再構築し、Medium/Long probe の `forces_prepend_… |
| [BUG-087](BUG-087.md) | `send_romaji_as_tsf_warm` の `LiteralDetectFsm` install が直前の段の検出窓を無警告で破棄しうる（ADR-103の対象外、事前存… |
| [BUG-088](BUG-088.md) | `HOOK_KEYS` リング overflow時にキーが無警告で消える（配送経路、ADR-102/105コードレビュー指摘2） |
| [BUG-089](BUG-089.md) | gate中にdeferされたCtrl+key（tmux prefix等）ではGJI composition キャンセルが効かない（ADR-102/105コードレビュー指摘4、未対応… |
| [BUG-090](BUG-090.md) | PowerToys「マウスなしでコンピューターを制御」(Mouse Without Borders) 使用中に物理「英数」キーが効かない（「かな」は効く、**追補で根本原因を特定・… |
| [BUG-091](BUG-091.md) | ネイティブ Win32 マルチフィールドダイアログでのフィールド間 Tab 直後、進行中の FocusProbe/ImmCrossProbe/idle-conv-check の観測… |
| [BUG-092](BUG-092.md) | BUG-33 追補 — `Imm32Unavailable`/`TsfNative` の shadow フォールバック観測 laundering を型で閉じた（ADR-106 決定… |
| [BUG-093](BUG-093.md) | MS-IME の無変換単独タップ delegate が変換中 composition を破棄する |
| [BUG-094](BUG-094.md) | 親指キーを無変換/変換に選び直すと設定画面のドロップダウンが消える |
| [BUG-095](BUG-095.md) | `.yab`のクォート崩れリテラルが無警告で受理される（レイアウト検証不足） |
| [BUG-097](BUG-097.md) | IME apply pending 上書き後の旧成功完了が stale 扱いされ applied が固着する |
| [BUG-098](BUG-098.md) | generation なし非同期 shadow toggle OFF 完了は focus epoch ゲートを通らない |
| [BUG-100](BUG-100.md) | `key_remap` の latch (`LATCHED_TARGET`) が KeyUp 消失や一部の swallow 経路で stuck する |
| [BUG-101](BUG-101.md) | `Engine::on_input` の Phase 0 が Consume 済み KeyDown に対応する KeyUp を FSM に一切届けていない（2026-03-31 混… |
| [BUG-102](BUG-102.md) | 起動直後にフォーカスしていたアプリの `ImmCrossProbe`（High）観測が導出から外れ、Medium の定期ポーリングに負ける（bootstrap フェンス desyn… |
| [BUG-103](BUG-103.md) | `[[post_bypass]]` は `reload_config()` で反映されない（設定変更に再起動が必要） |
| [BUG-104](BUG-104.md) | 独自 `.yab` レイアウトが UTF-8 でないと起動時に無言でバンドル版へ差し替わる |
| [BUG-105](BUG-105.md) | NICOLA 3鍵仲裁が char1 解放済みなら無条件で char2 側を優先し、タイトな重なりでも無視する |
| [BUG-106](BUG-106.md) | Teams(WebView2/MS-IME) で送信 romaji VK が JIS かな配列として解釈される |
| [BUG-107](BUG-107.md) | `ImmCapabilityStore` の学習キャッシュが `class_name` のみをキーにしており、winitの汎用クラス名を介して無関係なプロセスの誤学習が `awas… |
| [BUG-108](BUG-108.md) | タスクトレイの「学習キャッシュをクリア」メニュー項目が完全な no-op になっている |
| [BUG-109](BUG-109.md) | `drain_pending_deferred_before_send_if_queue_only`（ADR-123 決定4-3）が recovery resend 自身の送信より… |
| [BUG-110](BUG-110.md) | 物理IMEキー1回の低確度な検出で、NICOLA変換エンジンがフォーカス変更まで無期限停止する |
| [BUG-111](BUG-111.md) | `run_ime_refresh` の 500ms 周期リフレッシュが実フォーカス変更の有無に関わらず `[imm-learning] profile 降格` ログを毎ティック再発… |
| [BUG-112](BUG-112.md) | `ImmCapabilityStore` が `awase-settings.exe` を稀に `Unavailable` と誤学習し恒久化する（BUG-107 の「あ混入」の残存… |
| [BUG-113](BUG-113.md) | Windows Terminal + GJI で、Engine 有効時に物理半角/全角キー（`VK_DBE_SBCSCHAR`）を押すと余分な「@」が出力される（**二重actua… |
| [BUG-114](BUG-114.md) | Windows Terminal（TsfNative プロファイル）の `FocusChanged` 分類が `Standard`/`ImmCross` にフォールバックし、dri… |
| [BUG-115](BUG-115.md) | `awase-gji-config` の `session_keymap` フィールド番号が誤っており、GJI が無変換/変換キーでIME ON/OFFを制御する overlay … |
| [BUG-116](BUG-116.md) | Shift+物理かなキー（JIS配列 `VK_DBE_KATAKANA`）でカタカナ変換に切り替わらない（BUG-52修正のリグレッション、**決定1/2実装・実機確認済み**） |
| [BUG-117](BUG-117.md) | `UserImeSetIntent{source: PhysicalImeKey}` が発生源を検証せず `desired_open` を無条件上書きし、Edge(TsfNativ… |
| [BUG-118](BUG-118.md) | 無変換/変換 delegate-to-open-axis の `TurnOn` 方向が構造的に発火できず、GJI 自身が IME を ON にしても NICOLA 変換が起動しない… |
| [BUG-119](BUG-119.md) | GJI自動検出の無変換/変換 `delegate_to_open_axis` が、ユーザーが明示的に選んだ「常に送出する（パススルー）」設定を無視して物理キーを握りつぶす（**`T… |
| [BUG-120](BUG-120.md) | Windows Defenderが`Behavior:Win32/Persistence.A!.ml`としてawase.exeを誤検知（対策は補助的、未確認・恒久対策はコード署名） |
| [BUG-121](BUG-121.md) | `Ctrl+無変換`（`keys.ime_off`既定ホットキー）が、実IME状態と belief がズレた直後に稀に「@」を誘発する（既存の独立バグ、develop回帰ではない・… |
| [BUG-122](BUG-122.md) | ADR-153決定1「ケース2」（無変換/変換単独タップの明示config、`"on"`方向）が、`IntentWitness::from_physical` の witness … |
| [BUG-123](BUG-123.md) | ADR-153決定1「ケース2」修正（BUG-122）後、`*_solo_tap_always_suppress = false`環境で無変換/変換キー単独タップがGJIへ二重の信… |
| [BUG-124](BUG-124.md) | ADR-153決定1「ケース3」の"off"×belief既にOFFを全面撤回したところ、GJI自身のTSFキー横取りによる「@」再現に逆戻りした（設計の見直し不足、同日中に「抑止… |
| [BUG-125](BUG-125.md) | 明示config対象VKが現在のNICOLA親指キー設定と一致しない場合、GJI自動検出由来のactuationがマスクされず二重actuationしうる（/code-review… |
| [BUG-126](BUG-126.md) | （未確認・理論的リスクとして調査しクローズ）タイマー経路の親指タイムスタンプがdrain replay時にライブ再取得され、別の押下の値と誤って比較されうる懸念——実機未再現、失敗… |
| [BUG-127](BUG-127.md) | `OUTPUT_GATE` drain replay 中、親指キー押下タイムスタンプがイベント捕捉時点ではなくリプレイ実行時点のライブ値で再構築され、既に消費済みの押下と無関係な後… |
| [BUG-128](BUG-128.md) | Chrome で Ctrl+無変換 直後に explorer.exe 内の別 UWP 入力面へフォーカスが移ると、無関係な cached ON が復元され force-ON まで誤発火する |
| [BUG-129](BUG-129.md) | 【解決済み・仕様と判定】`flush_pending`の`PendingCharThumb`腕が`ComposingHint`（現`ThumbRawVkEmission`）を参照しない件、根本原因はコード見… |
| [BUG-130](BUG-130.md) | `tsf::probe::tests::check_now_show_only_confirm_becomes_stale_after_grace_expires` がwindows-build CIで稀にflake（テスト自体の不具合、実装バグではない） |
| [BUG-131](BUG-131.md) | `kana_mode_restore_key_down`（ADR-137決定2のM-2ラッチ）の解除条件がDBEキーのDown/Up vk非対称で成立せず固着する |
| [BUG-132](BUG-132.md) | `hook.rs`の`LEFT_THUMB_DOWN_AT_US`がDBEキーのDown/Up vk非対称で親指キー押下中ラッチしうる（設定リロードで自然回復、未修正） |

## その他の資料

| 資料 | ファイル |
|---|---|
| 実装アーキテクチャ概要（2026-06-02 時点） | [architecture-overview.md](architecture-overview.md) |
| デバッグ方法 | [debugging-guide.md](debugging-guide.md) |
| 2026-07-25: Windows実機での`cargo test --lib -p awase-windows`初回実行で判明したテスト自体の不具合（実装バグではない） | [NOTE-2026-07-25-test-infra.md](NOTE-2026-07-25-test-infra.md) |
| FEATURE-115: 打鍵列機能（ADR-115）実装状況・既知の限界 | [FEATURE-115.md](FEATURE-115.md) |
