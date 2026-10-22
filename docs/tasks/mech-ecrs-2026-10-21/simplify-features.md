---
title: 機能・設定・診断の単位で、挙動は変わるがコードが減る撤去候補
status: 調査のみ(2026-10-22、origin/develop 6a1e2d1e)。コードは変更していない。cargo は実行していない。行数は git ls-files・wc -l・git grep で数えたもの。「推定」と書いた数は、ファイルをまたぐ配線を読んで見積もったもので、着手前に数え直すこと
created: 2026-10-22
related_adr: ["ADR-117", "ADR-158", "ADR-206", "ADR-247", "ADR-258", "ADR-261", "ADR-263", "ADR-264", "ADR-268", "ADR-271", "ADR-273"]
---

# 機能・設定・診断の単位で、挙動は変わるがコードが減る撤去候補

所有者の問い(2026-10-21):「もう撤去できるコードはないか。挙動が変わっても、シンプルになるならそちらを選ぶかどうか」。
挙動を変えない撤去は `INDEX.md`・`INDEX-dd.md`・ADR-276 でほぼ済んだ。この文書はその次の段階で、**利用者から見える動き・設定・診断・観測が変わる**候補を、機能の単位で並べる。

数え方:
- 本番とテストは、ファイル内の最初の `#[cfg(test)] mod` を境に分けた(`tests/`・`examples/` は全部をテスト側に数えた)。
- 「推定」が付いていない数は、ファイル単位でそのまま削れる行数。

## 1. 表(削れる行数が大きく、挙動の変化が小さい順)

| ID | 名前・場所 | 削れる行数(本番 / テスト) | 挙動の変化(利用者から見て) | 依存 | リスクと必要な確認 | 推奨 |
|---|---|---|---|---|---|---|
| SF-01 | どこからも呼ばれないスパイク用の examples 12 本(`crates/awase-windows/examples/` の `gji_composition_probe`・`ime_observation_spike`・`msime_key_assignment_settings_probe`・`msime_native_composing_probe`・`richedit_tsf_probe`・`spike_calibration_decisive_v2`・`spike_egui_himc_reassociation_probe`・`spike_egui_ime_control_probe`・`spike_kana_lock_probe`・`spike_langbar_input_mode`・`text_store_probe`・`two_imm32_windows_probe`) | 0 / **6,278**(12 ファイルの wc -l の合計) | 無し。awase.exe にも設定画面にも入らない。過去の実験は手元で再実行できなくなる(git の履歴には残る) | `.github`・`tools`・`scripts`・`mise.toml` からの `--example` の呼び出しは 0 件(grep で確認)。`windows-core` の dev 依存は `spike_langbar_input_mode` 専用(`crates/awase-windows/Cargo.toml` 73 行目のコメント)なので一緒に外せる。docs から 24 ファイルがファイル名で参照している(リンク切れになる) | main の CI は examples をビルドしない(`ci.yml` の clippy に `--all-targets` が無い)。cargo-mutants の baseline は examples もビルドする(`ci.yml` 652〜660 行のコメント)。撤去すると mutants が少し速くなる。CI だけで確かめられる | **撤去** |
| SF-02 | 入力インジケーターの右クリックメニューを読む機能(ADR-264)。`awase-ime-core/src/state/indicator_menu.rs`・`awase-windows/src/observer/indicator_menu.rs`・`runtime/indicator_menu_shadow.rs` と、`indicator_evidence.rs` のカウンタ、journal の `IndicatorMenu`、不具合報告の要約、設定画面の選択肢 | ファイル単位で 875 / 404。配線を含めて **約 1,100 / 約 530(推定)** | 既定は `off`。`observe` を選んだ人だけ、メニューを開いたときの読みが journal と不具合報告に残る(`ui_max_level()` が `Observe` 固定なので belief には入らない)。撤去後は `[indicator_evidence] menu` が読み捨てられ、設定画面からメニューの選択肢が消える | ADR-273(2026-10-20)で「メニューは実機で信頼できず、belief への配線は断念」と決まっている。`architecture_guard::indicator_menu_shadow_is_observe_only_and_opt_in` を消す。`config.rs` に `deny_unknown_fields` は無いので、既存の `config.toml` に `menu = ...` が残っても読み込みは失敗しない。ADR-264 の status は「実装なし」のまま古い(実装はある) | 低い。全体で WinEvent のフックが 1 本減る。CI(guard・設定の読み込みテスト)で足りる | **撤去** |
| SF-03 | 引き金のブランチが既に無い CI の実験。`crates/e2e-uwp-inputsite-probe`(workspace のメンバー)と `e2e-uwp-inputsite-hook-watchdog-probe.yml`、`b1-notify-probe-verify.yml` と `tools/e2e/b1-notify-probe.ps1`、`adr195-t7-focus-loss-verify.yml` と `tools/e2e/t7-focus-loss-check.ps1` | crate 607(Rust 569 と Cargo.toml 38)と、workflow・ps1 で 358。計 **965**(本番には入らない) | 無し | 引き金のブランチ `feat/e2e-uwp-inputsite-hook-watchdog`・`fix/b1-ime-notify-external-write`・`diag/adr195-t7-t10-realmachine-verify` は origin に無い(`git ls-remote --heads` で確認)。`Cargo.toml` の members と `Cargo.lock` から外す。ほかの `.rs` での言及はコメントだけ | 無し(workspace のビルドが 1 crate 減るだけ)。CI で足りる | **撤去** |
| SF-04 | フォーカスが変わるたびに出す `[ime-diag]` の診断(`ime_diagnostic.rs` の `ImeDiagnosticSnapshot::capture("focus_changed")`、ADR-117 の前の「Phase 1 観測強化」) | **約 250 / 0(推定)**(`ime_diagnostic.rs` 370 行のうち `[comp-probe]` 以外) | `awase.log` に debug の `[ime-diag]` 行が出なくなる。`[ime-diag]` を参照する docs は 0 件。**副作用**: この関数は、ログの水準に関係なく、メインスレッドで `GetGUIThreadInfo`(上限 100ms)と、`run_with_timeout`(上限 150ms)の中でプロセスをまたぐ `IMC_GETOPENSTATUS`/`IMC_GETCONVERSIONMODE` を、フォーカスが変わるたびに呼んでいる(`ime_refresh.rs::ir_post_focus_change_snapshot`)。撤去すると、debug ログのためだけのこの読みが無くなる | `lints/actuation_call_guard` の `RESTRICTED_CALLS` で、`probe_ime_control` の許可された呼び出し元のうち `capture_imc` が 1 つ減る(complexity-budget の返済に数えられる) | 低い。実際の待ち時間(ms)は未確認で、実機の awase.log で測れる | **撤去**(`[comp-probe]` は SF-11 で別に判断) |
| SF-05 | `#[actuation_choke_point]` 属性マクロ(ADR-158 TE3 の観測フェーズ、2026-09-09 開始)。`crates/actuation-choke-point-macro` と、`AppImeProfile::can_use_imm32_cross_process` への付与 | crate 78、付与とコメントが約 15。**約 95 / 0(推定)** | debug ログ `[actuation-record]` が出なくなる。`can_use_imm32_cross_process` を `const fn` に戻せる | 付与しているのは 1 関数だけ(`git grep`)。dylint の許可リストへの昇格は、`docs/adr/158-implementation-tasks.md` の TE3 の見出しに「昇格フェーズは性質上ここでは完了不能」とある | 無し。CI で足りる | **撤去** |
| SF-06 | Linux・macOS 用の stub(`crates/awase-linux`・`crates/awase-macos`)と、設定 `general.linux_input_backend`/`linux_evdev_device` | **2,588 / 80** と、設定の項目(参照 34 箇所) | Windows の利用者には無し。Linux・macOS 向けのバイナリが作れなくなる(配布はしていない、未確認) | ADR-019(core を OS に依存させない)を、別の OS 向けにビルドすることで示している面がある。core の API を変えるたびに、この 2 crate も直す手間がある(最後の変更は 2026-10-10 の ADR-255) | core の OS 非依存は `layer_boundary_guard` と dylint でも守られている。CI で足りる | **所有者判断**(将来 Linux 版を出すつもりがあるか) |
| SF-07 | タスクバーのインジケーターの字形を読む機能(ADR-261・271・273)。`state/indicator_glyph*.rs`・`observer/indicator_glyph.rs`・`runtime/indicator_glyph_shadow.rs`・同梱の見本 `indicator_glyph_seeds.bin` と、ADR-273 段階 1・2a の belief 側の核(`ime_model`・`observation_store`・`platform_state`) | ファイル単位で 2,690 / 1,118。ADR-273 の核を含めると **約 3,800 / 約 2,300(推定。aa979d58 は +1,217、0be73ae4 は +984)** | 既定は `off`。`ui_max_level()` が `Observe` なので、今は誰の belief にも入らない(段階 2a は「まだ動かない」)。撤去すると、観測の無い窓(一太郎など)で IME 状態を知る計画が無くなる | **進行中**(2a は #746、`GLYPH_KEY_FENCE_MS` の調整は #734、BMP ダンプの実験は #781 で今日マージ)。SF-02 と一緒に消すなら `indicator_evidence.rs`(217 / 182)と、設定の `[indicator_evidence]`・`EvidenceLevel` も消せる | 高い(方針の撤回になる)。消すこと自体は CI で確かめられる | **所有者判断**(続けるなら残す) |
| SF-08 | 窓切替直後の打鍵から日本語/英数を判定する機能(ADR-263、`[auto_detect]`)。`src/lang_detect.rs`・`runtime/lang_detect_shadow.rs`、設定画面の 5 択、journal の `LangDetect` | ファイル単位で 1,367 / 732。配線を含めて **約 1,850 / 約 900(推定)** | **既定の `observe` が全員の環境で動いている**(n-gram を読み、エピソードごとに影の判定を journal に残す。`app/mod.rs::init_lang_detect_validated`)。`ime_on`・`ime_on_eisu` は既定オフで、実機(一太郎・GJI)では未確認(ADR-263 の status)。撤去すると、設定画面の「自動判定」が消え、`[auto_detect]` は読み捨てになる | 打鍵を取り込み口で保持する経路(`LangDetectIntake::Consumed`)を、`app/mod.rs`・`key_pipeline.rs`・`message_handlers.rs` から外す。guard の参照は 14 箇所。直近も修正が入っている(#782) | 中くらい。保持の経路を外すので、打鍵の順序を見る golden・e2e の確認が要る。実機は要らない(既定の挙動に戻るだけ) | **所有者判断**(`observe` で集めたデータを使う予定があるか) |
| SF-09 | 非推奨の `general.muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action` を読み替える処理(ADR-206 決定 4) | **約 200 / 約 150(推定)**(`config.rs` の項目・`legacy_thumb_solo_tap_actions`・`validate_legacy_solo_tap_action`、`runtime/mod.rs`・`engine.rs` の移行、設定画面での消去と、それぞれのテスト) | 古い `config.toml` にこの項目が残っている人は、無変換・変換の単独タップで IME が開閉しなくなる(今は読み込み時に `keys.ime_*` へ読み替えて、警告を出している)。設定画面で一度保存した人は、もう影響を受けない(保存時に消している) | ADR-217(後方互換の名目だけのコードの撤去)と同じ種類。`state_dependent_key_warning.rs` の doc も直す | 実機で 1 回確認したい(古い設定で警告が出て、単独タップが効かなくなることだけ) | **所有者判断**(v2.0.0 から 2.5 か月たった。告知の期間は足りたか) |
| SF-10 | mozc-readable の共有メモリの診断の記録(`[diagnostics] mozc_shm_shadow`、ADR-265 段階 1)。belief への接続(ADR-268)は別で、それは残す | **約 210 / 約 130(推定)**(`state/mozc_shm.rs` の `Trigger`・`RecordContext`・`to_record`、journal の `MozcShm`、`runtime/mozc_shm_shadow.rs` の後半、設定の項目) | 既定はオフ。オンにした人だけ、journal と awase.log の `[mozc-shm]` 行が出なくなる。belief と IME の動きは変わらない | ADR-268 の status に「TTL の最終値は未決(専用タイマーの間隔の実測後に決める)」とあり、その測定にこの記録を使う。guard の `mozc_shm_shadow` の項目を直す | 低い。ただし TTL を決める前に消すと、測る手段を作り直すことになる | **所有者判断**(TTL を決めてから撤去) |
| SF-11 | `[comp-probe]` の 1 行ログ(`ime_diagnostic.rs::log_composition_probe`、「部分リテラル検出の実験用」) | **約 120 / 0(推定)**(`TSF_PROBE_SNAP` の受け渡しを含む) | info の水準で、組成中の文字列(`comp=`・`result=`)を awase.log に出している。awase.log は不具合報告に gzip で付く(ADR-222)。撤去すると、literal 化の調査の手がかりが 1 つ減る | 直近の調査で使われている(BUG-200・BUG-207 の docs で参照、6 ファイル)。不具合報告への送信前に、この行を伏せる処理は見つからなかった(`awase-bug-report`・`report-worker` を grep、未確認) | 低い。調査の手がかりが減るのが代償 | **所有者判断**(残すなら、組成の文字列を出さない形にするかどうか) |
| SF-12 | ADR-258 段階 0(1 回目の literal 疑いで、「ここで arm していたら」の読みを journal に残すだけ) | **約 130 / 約 100(推定)**(3901337c は 7 ファイルで +194。ほかに journal の項目) | journal の `first-literal observe` が残らなくなる。挙動は変わらない | BUG-200(最初の 1 打鍵が消える)は未修正で、「実利用のデータ待ち」のまま 2026-10-12 から 10 日たっている。`external_change_watch.rs`・`literal_facts.rs`・`platform.rs`・`platform_state` に分かれて配線されている | 低い。消すと BUG-200 の直し方(ADR-258 案 D5')を、データ無しで決めることになる | **所有者判断**(データが集まっているかどうか) |
| SF-13 | ADR-247 案 1(GJI の CUSTOM 表の F13〜F24 の行から、開閉と入力モードを予測する) | **約 70〜115 / 6 本**(ADR-247 本文は「約 70 行と単体テスト 6 本」。`key_effect_predictor.rs::custom_f_key_prediction` を含めると約 115、推定) | 観測が届かない窓で、F キーのモード切替に Engine が追随しなくなる。ただし CI(実 Chrome・Windows Terminal)では、そういう窓が見つからなかった | ADR-247 に「読めない窓で F キーの追随が欠ける報告が出なければ、棚卸しで撤去を再検討する」とある。案 2(直接観測)は残す | 低い。CI で足りる | **撤去寄りの所有者判断** |
| SF-14 | 安全弁の設定 3 つ(`diagnostics.hook_self_heal`・`general.predict_henkan_open_in_unreadable_windows`・`general.use_learned_keymap_table`)。機能は残し、オフにするスイッチだけを消す | 各 **約 15 / 約 15(推定)** | 不具合が出たときに、ビルドし直さずに機能を止める手段が無くなる | どれも既定は `true` | 減る行数は小さく、失うものの方が大きい | **残す** |
| SF-15 | IME のキー効果の学習(ADR-191/195/196、`awase-keymap-learn`・`awase-keymap-learn-win`) | 2 crate で 12,805(本番・テストの内訳は数えていない) | 設定画面の学習が無くなり、同梱の表と GJI の設定の読み取りだけになる | 本番の予測(`awase-key-effect`)が学習済みの表を読む。比較のために表に載せた | 高い | **残す** |

## 2. 撤去するときの順序の案

1. **SF-01・SF-03・SF-05**: 利用者から見て何も変わらない。1 PR にまとめられる(docs のリンク切れの修正を含む)。
2. **SF-02・SF-04**: 既定の挙動は変わらない(SF-02 は既定オフ、SF-04 は debug ログだけ)。それぞれ 1 PR。SF-02 では ADR-264 の status も直す。
3. **SF-13**: ADR-247 の「再検討する」の条件にそのまま当たる。
4. それ以外は下の質問への答え次第。

## 3. 所有者に聞くべき質問

1. **字形(SF-07)を続けますか。** やめるなら、メニュー(SF-02)と `[indicator_evidence]` の設定もまとめて消せて、本番で約 5,000 行(推定)減ります。続けるなら、メニューだけ消します。
2. **`[auto_detect]` の既定の `observe`(SF-08)で集めているデータを使う予定はありますか。** 使わないなら、少なくとも既定を `off` にするか、機能ごと消すかを決めたいです。
3. **Linux・macOS の stub(SF-06)は、将来の移植のために残しますか。**
4. **非推奨の `*_solo_tap_ime_action` の読み替え(SF-09)は、もう打ち切ってよいですか。**(古い設定のままの人は、単独タップで IME が開閉しなくなります)
5. **計測を待っている観測(SF-10 の mozc の TTL、SF-12 の BUG-200)は、いつまで待ちますか。** 期限を決めておけば、期限を過ぎたら撤去する、で揃えられます。

## 4. 確かめていないこと

- 「推定」と書いた行数(配線・テストの分)。
- SF-04 の、フォーカス変更のたびの待ち時間の実測値。
- SF-06 で、Linux・macOS 向けのバイナリを配布していないこと。
- SF-11 の `[comp-probe]` の行が、不具合報告の送信前に伏せられていないこと(grep で見つからなかっただけ)。
