# ci/* 検証ブランチの整理記録(2026-10-26)

`ci/*` の一回限りの検証ブランチを整理した。再利用できる道具(`tools/`・`examples/`)は本 PR で develop に取り込み、
それ以外は削除する。ブランチ先端のハッシュを残すので、必要なら `git fetch origin <hash>` や
GitHub 上のコミット URL で辿れる(削除後、GitHub が到達不能コミットを回収するまでの間)。

取り込んだ道具: `tools/ci/{stt-probe,bug203_chrome,indicator_menu,indicator_menu_stage1,indicator_taskbar}`、
`tools/ci/msime-*.ps1`、`tools/e2e/{firefox_addressbar,wezterm_probe,adr223,bug183}`、`tools/realmachine/daily.ps1`、
`check_immread.py`、`examples/{ime_indicator_probe,indicator_scenario,langbar_inputmode_probe,lang_switch_probe,tip_scope_probe}.rs`、
`typing_stress/{immread,switchime}.rs`。取り込まなかったもの: `diag*.ps1`(使い捨て診断)、`exp_modekey.rs`(コアに入る実験コード)、`ablations/*.sh`(変わったコードへの当て物)。

| ブランチ | 先端 | 最終更新 | 先端の件名 | 扱い |
|---|---|---|---|---|
| `ci/adr221-base` | `f91ac0607a` | 2026-10-03 | Merge remote-tracking branch 'origin/ci/msime-chrome-off-rca' into fix | 削除 |
| `ci/adr223-stage0` | `fd4a40e5f8` | 2026-10-03 | revert(lang-check): EVENT_OBJECT_FOCUS コールバックの LAST_FOCUS_HWND 更新位置の変更 | 削除 |
| `ci/adr223-stage1` | `5f59270ec9` | 2026-10-04 | ci(adr223): 出力の検出に Unicode 経路の send_keys ログも数える(Win32 窓・UWP で確認が空振りしてい | 削除 |
| `ci/adr234-s0-verify` | `c928a1f151` | 2026-10-07 | test(state): ImeBelief::eisu_candidate の mutants の生き残りをテストで殺す(ADR-234  | 削除 |
| `ci/adr244-s1-trace` | `9cd7881044` | 2026-10-07 | spike(adr-244): msimebody の compose/idle 構成に親指キー設定を足す(変換・無変換が親指キーとして消費 | 削除 |
| `ci/adr244-s2-spike` | `55252fbe88` | 2026-10-07 | spike(adr-244): 直接観測で追随したら持続トグルも手放す(使い捨て) | 削除 |
| `ci/adr252-stuck-observe` | `a359ae8550` | 2026-10-10 | docs(adr): ADR-252 の観測結果(run 38046806775)を ADR-252・ADR-249・BUG-197 に反映 | 削除 |
| `ci/adr253-diag` | `dd8c69e5a0` | 2026-10-10 | ci(adr253): 注入文に ASCII・絵文字を含め、構成名に n を入れる(診断ブランチのみ) | 削除 |
| `ci/adr264-stage1-verify` | `ebcf7cd81e` | 2026-10-17 | merge impl(診断を外す) | 削除 |
| `ci/adr278-corpus-base` | `6ee6980dee` | 2026-10-22 | test(adr-278): 基準版だけに置く MB-02 のプローブの Facts の再生(試験運転、PR にしない) | 残す(保留) |
| `ci/adr279-p0-after` | `4528a8cead` | 2026-10-23 | Merge remote-tracking branch 'origin/feat/adr279-p0-drift-follow-would | 削除 |
| `ci/adr279-p0-before` | `436d57a54a` | 2026-10-23 | ci(adr279): P0 の before/after を同じ構成で回す(ci/adr279-p0-before・after 専用) | 削除 |
| `ci/adr279-stage0` | `06819e7552` | 2026-10-23 | ci(adr279): 段階 0 の集計スクリプト(S0-1・S0-2) | 削除 |
| `ci/adr282-ime-scope` | `cc62195d6c` | 2026-10-26 | ci(adr282): ime-scope の引数を既定(全ペア・各 2 回)に戻す | 残す(保留) |
| `ci/adr282-ime-scope-b1a` | `ab6e5eb5ea` | 2026-10-26 | ci(adr282): B1a の測定用の変更(WT の変種・記録)を撤去する [skip ci] | 残す(保留) |
| `ci/b4-msime-toggle-probe` | `5357ac0319` | 2026-09-29 | diag(b4): run8 = 半角/全角の入力中を matrix に追加、awase 起動下の ctx.composing 信号を表に出 | 削除 |
| `ci/bug-207-baseline-mozc` | `54cf0a9363` | 2026-10-18 | ci: 比較用(develop)に ts-tsf-{gji,msime}-{40,10}ms を回す ONLY を足す(BUG-207) | 削除 |
| `ci/bug112-probe` | `9fff5b76a4` | 2026-10-04 | ci: BUG-112 の前提(窓生成→ImmGetDefaultIMEWnd non-NULL の実時間)を windows-latest | 削除 |
| `ci/bug114-chrome-startup-drift` | `7f701fdf72` | 2026-10-06 | ci(e2e): BUG-114 の起動時経路を実 Chrome で確かめる sc-bug114-* を足す | 削除 |
| `ci/bug114-wt-startup` | `ea798cf540` | 2026-10-06 | ci(wt): BUG-114 の相 E のきっかけを補助 WT 窓との前面移動にする | 削除 |
| `ci/bug172-pre-teardown` | `2a1847304c` | 2026-09-29 | experiment(bug172): 対照用に force-ON 機構(621bf93c)のみ復元(測定専用ブランチ、develop には | 削除 |
| `ci/bug172-realistic-close` | `f0dd1b4452` | 2026-09-29 | test(chrome_probe): --close-via で実運用に近い外部 IME OFF(SendInput/メモ帳経由)を再現す | 削除 |
| `ci/bug172-refocus-window` | `0d3bc4aa98` | 2026-09-29 | test(chrome_probe): --refocus はタスクバーでなく自前の別窓を前面にしてフォーカス変更を起こす | 削除 |
| `ci/bug172-step0-trace` | `0006221088` | 2026-09-29 | Merge branch 'fix/bug172-observe-external-close' into ci/bug172-step0- | 削除 |
| `ci/bug183-lang-switch` | `6fe7ec1a5b` | 2026-10-03 | ci(bug183): 言語切替ホットキーの読み方を測る(ImmGetHotKey 列挙・Toggle 値ごとの切替) | 削除 |
| `ci/bug184-off-composition` | `7e56e66f4d` | 2026-10-03 | style: cargo fmt (msime_native_composing_probe.rs) | 削除 |
| `ci/bug195-tip-scope` | `9ee5245041` | 2026-10-09 | ci(bug195): 既定の入力方式の上書き(MS-IME / GJI)で awase の種別同定が変わるかを観測(アプリウィンドウごと= | 削除 |
| `ci/bug203-chrome-verify` | `56be553ad7` | 2026-10-18 | ci(bug203): Chrome+GJI でメニューの Direct Input 選択後に GJI I/O 推論が ON に倒すかを測る | 削除 |
| `ci/c2-tschrome-repro` | `3666e862b1` | 2026-09-29 | test(e2e): C2 項目2〜5用の強制シナリオ(--reopen-before-trial/--awase-settle-ms、re | 削除 |
| `ci/c2-tschrome-repro-noadr200` | `df912e661d` | 2026-09-29 | test(e2e): 測定専用。ADR-200決定1(bc12ce95)をrevertして修正前のreinit挙動でclose-ime対照を | 削除 |
| `ci/config-verify` | `471dd5ed6a` | 2026-09-26 | ci(config-verify): ADR-201 の設定挙動を windows-latest の実 awase.exe で確かめるワーク | 削除 |
| `ci/config-verify-c0wiring` | `ced4115691` | 2026-10-24 | docs(adr280): C0 の eisu_threshold の配線確認の結果を足す | 削除 |
| `ci/daily-realmachine` | `808ab8de27` | 2026-10-07 | chore(realmachine): 実機の日次確認スクリプト(typing_stress + ログ集計)を追加 | 削除 |
| `ci/dd202-reach` | `f4bcce9217` | 2026-10-22 | probe(reach): DD2-02 の IdleCheck 到達用に tsfnative 入力先と idle-gate 目印を追加(挙 | 削除 |
| `ci/dd503-repro` | `366607a2b8` | 2026-10-22 | style: cargo fmt | 削除 |
| `ci/dd503-verify` | `a714777df5` | 2026-10-22 | ci(dd503-verify): hk 構成に hkclnr と待ち 32 秒を足す(観測のみ) | 削除 |
| `ci/debug-session` | `ea25cad1c9` | 2026-10-11 | ci(debug-session): OAuth をやめて auth key(TS_AUTHKEY)で入る(Tailnet Lock で l | 削除 |
| `ci/e2e-a12-probe` | `259b2711dc` | 2026-10-21 | ci(e2e): ADR-203 決定 4 の撤去実験 a12(GjiFsm への ImeOn 通知 2 か所を外す)と sc-reopen | 削除 |
| `ci/e2e-adr209-henkan-follow` | `483b40cb90` | 2026-09-30 | ci(e2e): 追随できる条件の調査用 sc-follow-chrome-*(実 Chrome、ATOK/MS-IME プリセット×各キー | 削除 |
| `ci/e2e-b2-bug18` | `df46ff60c5` | 2026-10-21 | ci(e2e): a12 の B2/BUG-18 側の対照(sc-startup-gji-tsf-on と cal-driftrec-*-t | 削除 |
| `ci/e2e-b4-keyassign` | `eb30926587` | 2026-09-29 | test(e2e): MS-IME キー割り当て(値2)の CI 検証を拡張(実機の全値の再現・開いた状態から・変換キー) | 削除 |
| `ci/e2e-default-vk` | `87c55a0642` | 2026-10-21 | ci(e2e): a13(Win32/UWP の既定を Vk にしたビルド)と -dvk 構成を足す(Unicode 注入の撤去の検証) | 削除 |
| `ci/e2e-dictation` | `8d1a89b4d4` | 2026-10-10 | Merge branch 'ci/e2e-dictation' into ci/adr249-ab | 削除 |
| `ci/e2e-eisu-composite` | `2b3ecd3b45` | 2026-10-14 | ci(e2e): sc-eisu-km-* — 英数(複合キー)を belief とずれた状態で押したときの追随の再現試行(観測のみ) | 削除 |
| `ci/e2e-focus-restore-13-17` | `c629db6b36` | 2026-10-08 | test(e2e): focus-restore に --fr-b-open(新しい窓 B の IME も開いた状態)を足す | 削除 |
| `ci/e2e-henkan-tsf-probe` | `68cf0c8210` | 2026-09-30 | ci(e2e): 調査用 sc-henkan-*(MS-IME プリセットで IME OFF から変換を押すと開くか、実 Chrome と素 | 削除 |
| `ci/e2e-ime` | `00ce0c614c` | 2026-09-26 | docs(adr-191): conv 軸の自動書き込み撤去のCI比較(A/B-2)を記録する | 削除 |
| `ci/e2e-roman-ab` | `a0483e9202` | 2026-10-09 | ci(e2e): a8・a10 の構成を外す (a8 の対象は develop で撤去済み) | 削除 |
| `ci/e2e-typing-stress` | `c6d0c6706f` | 2026-09-28 | test(e2e): ts-bugreport-* に PREVIEW_DEBOUNCE 衝突(打鍵中の一映停止→再開)構成を追加 | 削除 |
| `ci/e2e-warmup-ab` | `11a59df393` | 2026-09-28 | ci(e2e): typing_stress に --idle-before を追加し、long-cold を試行ごとに踏む ab-*-lo | 削除 |
| `ci/firefox-addressbar-observe` | `4029ab8476` | 2026-10-25 | ci: Firefox アドレスバーの窓クラスと awase の扱いを観測するワークフロー | 削除 |
| `ci/imm-read-matrix` | `eb703711a9` | 2026-10-25 | ci(imm-read): --mode=immswitch を足す。A/B 窓を素早く切り替えながら、窓ハンドル指定の読みと前面を読む a | 削除 |
| `ci/indicator-menu-verify` | `44ba9e820f` | 2026-10-17 | ci: メニュー測定に、対照(何もしない)・読み取りの遅れ・選んだ項目の検知(idChild、ホバー/キーボード)を足す | 削除 |
| `ci/indicator-taskbar-verify` | `85031ab877` | 2026-10-17 | ci: インジケーターの字形取り込みをタスクバーの状態・MS-IME ツールバーで測る検証ワークフロー(ADR-261 D0-4) | 削除 |
| `ci/measure-no-force-tsf` | `493f61aef2` | 2026-10-22 | ci(e2e): ntsf にフォーカス往復直後に待たずに打つ構成(-switch0)を足す(TsfGate の保留を当てにいく) | 削除 |
| `ci/menu-match-live` | `5e6cb65b25` | 2026-10-21 | ci(exp): Chrome 版に scoop の chrome.exe の探索を足す | 残す(保留) |
| `ci/msime-chrome-off-rca` | `b7df61ebdf` | 2026-10-03 | docs(tasks): MS-IME×実 Chrome の OFF が閉じない件の根本原因調査(composition 残りの VK_IM | 削除 |
| `ci/msime-legacy-keytable-spike` | `9c28a7405d` | 2026-10-10 | docs(adr): ADR-248 の要約と決定3の対象を互換 ON 限定に直す | 削除 |
| `ci/mutants-lang-detect-base` | `4f6c77dc28` | 2026-10-25 | ci: mutants-lang-detect (使い捨て、develop のテストのみ) | 削除 |
| `ci/mutants-lang-detect-new` | `4765d9f5e2` | 2026-10-25 | Merge branch 'feat/lang-detect-trigram-latin-background' into ci/mutan | 削除 |
| `ci/probe-mb0203` | `a57c053a3d` | 2026-10-22 | ci(probe): Windows Terminal(force_tsf=Tsf モード)の入力先を CI 構成に入れる(MB-02 到達 | 削除 |
| `ci/probe-reach` | `aa9263de94` | 2026-10-22 | probe(reach): DD5-02 に relevant フラグ | 削除 |
| `ci/probe-remove-dd1-02` | `ad53597bf7` | 2026-10-22 | experiment(dd1-02): kp_shadow_actuate の ClaimDeclined 分岐を外した版(e2e 比較用、 | 削除 |
| `ci/probe-remove-dd5-02` | `728af885fb` | 2026-10-22 | experiment(dd5-02): フォーカス時の presync(A1) を外した版(e2e 比較用、PR にしない) | 削除 |
| `ci/probe-remove-mb0203` | `a7ba8032b0` | 2026-10-22 | ci(probe): MB-02 撤去版の比較用の到達目印と MB-03 の BypassConfirmed 目印(PR にしない) | 削除 |
| `ci/qt-line-repro` | `07ae28a363` | 2026-10-04 | ci(e2e): Opusレビュー反映(fmt、Qtビルドを tsx-qt 実行時だけに限定、キャッシュキー・windeployqt・後片付 | 削除 |
| `ci/quirks-latency-model` | `e1963220bf` | 2026-10-04 | test(closed-loop): クセ Q-key-latency-gji を読み戻し遅延で模し、Q-ext-off-chrome-gj | 削除 |
| `ci/send-confirm-verify` | `3f7839b86c` | 2026-10-02 | spike(ci): 「送信=確認」誤同一視と補償破棄の検証用ログ・修正フラグ・vf-* 構成 | 削除 |
| `ci/spike-m1m2` | `2c7e801b9e` | 2026-10-23 | ci(spike-m1m2): M1 に外から閉じる対照(ctlclose)を足し、未確定の「きう」を Esc で取り消す | 削除 |
| `ci/stt-compat-probe` | `23bd159dc0` | 2026-10-16 | ci(stt-compat-probe): ps1 を UTF-8 BOM つきに直し、vb-cable のセットアップを自分で実行する | 削除 |
| `ci/v2-b3-t1` | `dbca612f39` | 2026-09-29 | ci(e2e-ime): 検証専用 T1(d) 互換モード既定状態の読み取り(develop にマージしない) | 削除 |
| `ci/v2-msime-learning-verify` | `ced60b7bf3` | 2026-09-29 | ci: v2 MS-IME本体学習の再検証WF(検証専用、developへは入れない) | 削除 |
| `ci/verify-809-m1m2` | `efde4ba42c` | 2026-10-23 | ci(e2e): WezTerm の front_end を WebGpu にする(検証専用) | 削除 |
| `ci/verify-809-m1m2-before` | `1d81ef714c` | 2026-10-23 | ci(e2e): WezTerm の front_end を WebGpu にする(検証専用) | 削除 |
| `ci/verify-809-wezterm-probe` | `7b7bf335e4` | 2026-10-23 | ci: WezTerm の --config の引用符を Lua の単引用符にする(検証専用) | 削除 |
| `ci/verify-m1-focus-hold` | `0d526b5e60` | 2026-10-23 | ci(e2e): WezTerm の front_end を WebGpu にする(検証専用) | 削除 |
| `ci/verify-rm-tsf` | `c7b0ee0960` | 2026-10-22 | ci(e2e): Tsf 撤去版(PR #809)を measure-no-force-tsf と同じ入力先で回す構成 rmtsf を足す( | 残す(保留) |
| `ci/vocab-jitter-verify` | `01eb0ac578` | 2026-10-05 | ci: 検証専用(tsj-* だけ回す trigger)。マージしない | 削除 |
