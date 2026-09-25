---
title: 俯瞰レビュー（2026-09-24）の優先度一覧・依存関係と、低優先度・保留項目
status: 索引（全11ファイルの優先度・依存の一覧。B-7 は保留バックログで、各項目の状態は既存タスク文書側で追跡する）
created: 2026-09-24
related_adr: ["ADR-195", "ADR-196", "ADR-197", "ADR-186"]
source_review: 俯瞰レビュー（2026-09-24、origin/develop ae0ccfb3 基準）の B-7 / C-7 / D節 / 「確認できなかった点」。本ファイルは全11ファイルの索引
---

# 優先度の全体表と低優先度バックログ

裏取り基準は worktree の `5877f982`（origin/develop、PR #296 まで）。初版は `cbae84ff`（PR #292 まで）基準で、その後に PR #293（`d00ac8dd`、要確認表の採用経路の修正）、#294（`15d2de1e`、MS-IME 本体の初期仮説を5モードに）、#295（`39543e5d`、`KEY_EFFECT_SETTLE_MS` 100→170ms）、#296（`5877f982`、検証ウォークの記録と MS-IME 本体の隠れ状態仮説の記録）が入った。01〜10 は `5877f982` 基準で書き直し済み。

指摘 ID は「俯瞰レビュー A-1」のように書く。`docs/tasks/develop-weekly-code-review-2026-09-23.md` の A-x/B-x と番号が衝突するので混同しないこと（例: 週次レビューの A-2 は `7fa894bb`「A-2は実機測定で再現せず」で閉じたが、俯瞰レビューの A-2〈設定画面の表示、02〉とは別件で、02 は未解決）。

`related_adr` は B-7 に直接関係する ADR だけに絞っている（ADR-186 は BUG-162、ADR-197 は T5 の前提）。01〜10 が扱う ADR は各ファイルの frontmatter を参照。なお `docs/tasks/` の YAML frontmatter は `docs-frontmatter-convention.md` の規約対象外で、この系列（review-2026-09-24-*）内だけで揃えた独自書式である。

> **追随注記（ADR-199、2026-09-25）**: 08 は [ADR-199](../adr/199-derive-key-roles-from-user-ime-keymap.md) の T0〜T14 に置換。対応: 01・06 → T3（学習表による狭め）、08 → T2〜T4（判定関数・予測経路・`is_open_toggle_for` 撤去）、09 → T9〜T10・T12（F13〜F24・無変換/変換・MS-IME レジストリ）、10 → T7・T11（status・文書同期）、07 → ADR-176 撤去済み（ADR-198 決定3）で ADR-191 決定4 の見直しのみ（T7）、04 → T11・T14（`ime_toggle` 既定の文書追随）。

## 全体表（優先度と依存）

依存の矢印は「先に決める側 → 後で使う側」。各ファイルの「他ファイルとの依存」節と一致させてある。ただし、各ファイル側がまだ追随していない箇所が5つある（02・05・07・10。下の「各ファイルへの申し送り」）。その箇所はこの表のほうを正とする。

| # | ファイル | 担当指摘 | 優先度 | 依存・順序 |
|---|---|---|---|---|
| 01 | [adr196-adoption-and-mismatch-check](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md) | A-1 / B-2（**解消済み**、PR #293 `d00ac8dd`）/ B-3 / C-3 / C-6（MS-IME・GJI 部分）/ C-7（予測・ADR-192 警告・不具合報告は同じ `RuntimeTableCache` で揃っていることの確認） | 中〜高。最初の一手はタスク0（実測）。閉状態セルのカバレッジの疑いが実測で確かめられたら次リリース前へ戻す | 06 → 01（06 は 01 と同時か先）。01 → 02・03・08。10（B-8）→ 01。[adr196-t2-msime-learning-open-issues](adr196-t2-msime-learning-open-issues.md) → 01 |
| 02 | [settings-status-display](review-2026-09-24-02-settings-status-display.md) | A-2 / C-7（設定画面だけが独自判定で揃っていない） | **次リリース前**（D節 2） | 01 と独立に着手可（01 が採用フラグを `validate_and_convert` の外に出したら追随）。06 → 02（失効表示の部分だけ）。ADR196-T3 → 02（測定環境の表示）。案A のとき 02 → 03 |
| 03 | [release-bundle-keymap-learn-win](review-2026-09-24-03-release-bundle-keymap-learn-win.md) | B-4 | **次リリース前**（D節 3、同梱するか隠すかの判断） | 案A（学習UIを出す）のときだけ 01・02 → 03。案B/C は独立。07 → 03（persist の置き場所）。03 → 07(2) |
| 04 | [sample-config-and-user-docs](review-2026-09-24-04-sample-config-and-user-docs.md) | A-3 / A-9（**保留**、D節 14） | A-3: **次リリース前**（D節 4。T1・T2 は docs と設定ファイルだけで先行可）。A-9: 保留（文書の暫定注記だけ先に） | 04 → 07(5)（推奨モード統一が ConfirmMode 2択化の前提）。07 → 04 T4（A-9 の (b)/(c)）。T1 は docs・設定ファイルに加えて `crates/awase-windows/src/main.rs:48`・`:51` の起動エラー文言（`#[cfg(windows)]`）を含む |
| 05 | [startup-desired-open-forced-on](review-2026-09-24-05-startup-desired-open-forced-on.md) | B-5 起動時 | **次リリース前**（D節 5）。ただし「次リリース前」に求めるのは BUG-163（仮）の起票まで。修正は 09 の A/B と合わせる | 05 → 09（09 の A/B は 05 の修正有無を前提条件に持つ）。10（B-8）→ 05（B-8 で `state/platform_state.rs` を pre-push の対象に加える。今は対象外） |
| 06 | [keymap-learn-staleness-wiring](review-2026-09-24-06-keymap-learn-staleness-wiring.md) | B-1 | 中〜高（D節 6）。01 で5%判定を外すなら同時か先に必須 | 06 → 01・02・10。07(2) は 06 を待たない（06 が `ConfigFingerprint` を流用しないと決め、07 のタスク(3) に回答済み） |
| 07 | [adr176-fate-and-v2-cache-toml](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) | A-8 / A-10 / C-4 / C-5 | 中（D節 7、v2 着手前に決める） | 03 → 07(2)。04 → 07(5)。07 → 03（案A の persist の置き場所）・04 T4・10 |
| 08 | [open-close-fixed-set-vs-custom-keymap](review-2026-09-24-08-open-close-fixed-set-vs-custom-keymap.md) | C-1 / C-6（ATOK 部分）/ C-7（開閉書き込みは意図的に学習表を使わない、ADR-195(A)） | 中（D節 8） | 01 → 08（(B) 案を採る場合のみ）。06 と相互参照（どちらも先行不要）。08 → 09。10 と ADR-191 の frontmatter を両方触る（10 は status と summary の事実訂正、08 は summary (2)。順序は無く、後からマージする側が rebase する） |
| 09 | [remaining-active-writes-inventory](review-2026-09-24-09-remaining-active-writes-inventory.md) | B-5 残り / C-2 | 中（D節 9）。実機 A/B が必要 | 05・08 → 09。09 ↔ 10（09 の A/B 結果を 10 の撤去記録へ、10 の記録を 09 が参照） |
| 10 | [adr-status-and-stale-docs-sync](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) | A-4〜A-7 / B-6（削除部分だけ**保留可**、D節 11）/ B-8（D節 13 は保留だが 10 が保留を外した） | A-4〜A-7: 中（D節 10、P1 は docs のみで随時）。B-6: P2（削除部分だけ保留可）。B-8: P3 として先に入れる（01・05 の修正がルールの対象になるかがこれで決まるため） | 事実訂正は何も待たない。01・06・07 → 10（ADR-196/195/176 の status 同期）。07 → 10（ADR-176 の status）。09 ↔ 10（ADR-179 の撤去記録。09 の A/B 結果を 10 の節へ、`set_ime_mode` の死蔵コードは 09 → 10 B-6）。10（B-8）→ 01・05 |
| 11 | 本ファイル | B-7 / C-7 の割当 / D節 / 「確認できなかった点」の割当 | 保留（索引） | — |

D節「保留でよいもの」との対応: B-6（10。削除部分だけ保留可）、B-7（下記）、A-9（04）、ConfirmMode 2択化（04 の推奨モード統一 → 07）。B-8 も D節では保留だったが、10 は保留にしないと決めた（`.githooks/pre-push` の正規表現と rules の表の数行だけの変更で、01・05 の修正がルールの対象になるかがこれで決まるため）。

C-7 は新規作業なし。3つの結論をそれぞれ 01（揃っている経路の確認）・02（設定画面だけ揃っていない = A-2）・08（開閉書き込みは意図的に学習表を使わない = C-1）に吸収した。01（`source_review` と本文「C-7」節）と 08（`source_review` と冒頭）には記載がある。02 は frontmatter に C-7 が無く、A-2 そのものとして扱っている（C-7 の 02 担当分は A-2 と同じ内容なので、割当は索引上の整理として扱う）。これで俯瞰レビューの A-1〜A-10・B-1〜B-8・C-1〜C-7 はすべてどれかのファイルに割り当てられている。

### D節の優先度からの変更点

- 01 は D節 1（今すぐ直すべき筆頭）から「中〜高」に下げた。理由は 01 に記載のとおり: A-1 の根拠（30%⊂5%）が誤りで、B-2 は PR #293 で解消済み。棄却されても内蔵表に戻るだけで belief を壊さない。ただしタスク0の実測で閉状態セルの疑いが確かめられたら、次リリース前の筆頭へ戻す。
- 04（A-3）と 05 は D節どおり「次リリース前」とした（初版で「高」に下げていたのは根拠がなかったので戻した）。
- 04 の A-9 と 10 の B-6（削除部分）は D節どおり「保留」とし、表の担当指摘に明記して同じファイル内の他の指摘と優先度を分けた。
- 10 の B-8 は D節の「保留」から外した（10 の「作業単位」の判断に合わせた。理由は上記）。

### 推奨の着手順

1. 並行して先に進められるもの: 01 タスク0（実測）、02（01 と独立。失効表示の部分だけ 06 の後）、05 の BUG-163 起票、04 の T1・T2（docs と設定ファイル。T1 は `main.rs` の起動エラー文言1箇所を含むので Windows target の compile check が要る）、10 の P1（docs の事実訂正）と P3（B-8、pre-push と rules）。
2. 01 で5%判定を外す方向になったら、06（preset を含む指紋の照合）を同時か先に入れる。
3. 03 の判断（同梱／隠す／案C）→ 次リリース。案A なら 01・02 の完了が条件。
4. その後に 07（v2 前）、08、09（05 の扱いを固定して A/B）、10 の status 同期。

## 裏取りで変わった点・解消済みと判明した点（`5877f982` 時点）

- **解消済み**: 俯瞰レビュー B-2（要確認表の採用経路）。`4f3291a8` は PR #293（`d00ac8dd`）で develop に入った（`git branch -a --contains 4f3291a8` に `remotes/origin/develop` を確認）。01 で「解消済み」に変更済み。残りは A-1 の5%判定との組み合わせ（C-3）だけ。
- **前提が変わった点**: PR #294（`408ef6ba`、5モード仮説モデル）以降、MS-IME 本体の学習は10回中9回が正答率0.95以上で要確認まで到達する（[adr196-t2-msime-learning-open-issues](adr196-t2-msime-learning-open-issues.md) 未解決2）。01 のシナリオ2（MS-IME 本体での要確認→採用）は現在起こりうる。01 は反映済み。08 は対象ファイルに差分が無いことを確認済み（08 冒頭）。
- **その他の差分**: PR #295（`aac67a4c`）で `KEY_EFFECT_SETTLE_MS` が 100→170ms（`tuning.rs:523`）。打鍵後の予測の fence（`state/ime_model.rs:614`）に使う定数で、01〜10 の対象箇所には影響しない（05・09 で確認済み）。
- 初版から引き継ぐ点: (1) `config.sample.toml` の `speculative` は「廃止」と注記済み（04）。(2) `ime.rs:1742` の `set_ime_mode_for_target` 呼び出しは独立した conv 軸の書き込み経路ではない。呼び出し元の無い `set_ime_mode`（`ime.rs:1733`）の中で委譲しているだけで、死蔵コードとして 10 の B-6 で扱う（09。初版の「conv 軸の書き込みに含む」は誤りだった）。(3) 実行される pre-push は `.githooks/pre-push`（`git config core.hooksPath` が `/home/cuzic/rust-nicola/.githooks`）。対象の正規表現は `:36`。未追跡の `.git/hooks/pre-push:28` にも同じ `state/(ime|conv_mode|observation_store)` があるが、実行されない（10。初版の「実行されるのは `.git/hooks` 側」は誤りだった）。(4) 週次レビュー B-2（閉状態セルの潰れ、`ef76bf9a`）は別件で修正済み。(5) 新規 BUG の次の連番は BUG-163（05。`docs/known-bugs/` の最大は BUG-162）。
- 上記以外の 01〜10 の中核指摘（A-1 の5%判定 `key_effect_runtime.rs:488-491`、A-2 の `status_line(None)` `awase-settings/src/main.rs:3433`、B-4 の同梱漏れ、`desired_open: true` `ime_model.rs:309`、トレイ `ClearImmCache` の空処理 `message_handlers.rs:1303`、`staleness::check` の本番未配線など）は `5877f982` でも現存する（各ファイルで再確認済み）。

## 各ファイルへの申し送り（11 の担当外。表のほうが正）

| ファイル | 箇所 | 直す内容 |
|---|---|---|
| 02 | `:127` | 「次リリース前に 01・03 とセットで片付ける（索引 11 の推奨順）」は古い。「01 とは独立に次リリース前に片付ける。03 で案A を選ぶ場合は 01・02 の完了が条件」に直す |
| 05 | `:85`・`:115` | 「実行される `.git/hooks/pre-push:28`」は誤り。実行されるのは `.githooks/pre-push`（`:36`） |
| 07 | `:117` | 「06 → 07(2)の順」は古い。06 `:48`・`:84` が `ConfigFingerprint` を流用しないと決めたので、タスク(3) は回答済みで、07(2) は 06 を待たない |
| 10 | `:164` | 「本タスク → 01・06・07」は待つ側が始点になっていて、矢印の向きの定義と逆。「01・06・07 → 本タスク」に直す（`:167`・`:168` の「本タスク → 05／01」は B-8 の話で、向きは正しい） |
| 01 | `:168` | 対応は任意。「06 → 01」は向きが正しく、11 と一致している |

## 「確認できなかった点」の行き先

俯瞰レビュー末尾の5項目は次のファイルで追跡する。

| 確認できなかった点 | 担当 |
|---|---|
| 領域A撤去後、drift correction 単独で TsfNative の ON 回復を代替できるか（実機 A/B） | [09](review-2026-09-24-09-remaining-active-writes-inventory.md)（下の「保留事項」にも記載） |
| 既定構成の MS-IME 本体で学習した表と `MSIME_NATIVE` の実際の不一致率 | [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md)（タスク0。06 の依存の向きの根拠でもある） |
| `a07e681e` 以降の MS-IME 本体の `verify_accuracy` | [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md)（[adr196-t2-msime-learning-open-issues](adr196-t2-msime-learning-open-issues.md) の正答率記録を参照）、[07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) |
| 起動時の drift correction が ImmCross の実機で IME を開けてしまうか（BUG-157 は CI の件数だけ） | [05](review-2026-09-24-05-startup-desired-open-forced-on.md) |
| CI の `windows-build` が awase-keymap-learn-win をビルドしているか | [03](review-2026-09-24-03-release-bundle-keymap-learn-win.md)（ビルドしていないことを確認済み: `ci.yml:282`・`:378` は `-p awase-windows` と `-p awase-settings` だけ） |

## B-7: そのほか未着手（重要度が下がるもの）

新しいタスクファイルは作らない。各項目は既存の文書で追跡し、本ファイルは一覧と優先度だけを持つ。「検証」はその項目の完了を何で確かめるか（Linux テスト / windows-latest CI / 物理実機）。

- [ ] **ADR196-T2 項目9: `tag_mismatches` の配線。** 純粋ロジック（`crates/awase-keymap-learn/src/mismatch_tag.rs`、PR #284）だけで、`mismatch_tag.rs` 以外からの参照は `lib.rs:28` の `pub mod` 宣言のみ。学習フローと不具合報告へ接続する。→ [adr196-t2-mismatch-adjudication](adr196-t2-mismatch-adjudication.md)（`:11`・`:126` の残作業）で追跡。検証: Linux テスト（学習フローのユニットテストと不具合報告 JSON の golden）。**C-3 次第で優先度が上がる**（01 の結論待ち。B-3 のとおり MS-IME 本体では 1b-2/1b-4 の生存確認が成り立たないので、版ずれ／パイプライン疑いのタグは学習表の正しさを守る代わりの防護として重みが増す）。
- [ ] **再測定の結果と ADR196-T1 の外部書き込み観測の永続化**（不具合報告にも付ける）。→ 同じく [adr196-t2-mismatch-adjudication](adr196-t2-mismatch-adjudication.md)（`:11`・`:131`）で追跡。検証: 永続化と添付は Linux テスト、`RealImeDriver` での再測定の動作確認は windows-latest CI または物理実機。**C-3 次第で優先度が上がる**（理由は上と同じ。依存先は 01）。
- [ ] **ADR196-T5: Microsoft IME 側の版情報（指紋）と採点日時フィールド。** 元レビューと T5 の状態欄の「ADR-197 待ち」は古い。ADR-197 は決定4（互換モードフラグの読み取り）だけを採用して「決定済み・実装済み」になっている。読み取り関数 `msime_legacy_keymap::read_legacy_compat_mode_enabled`（`crates/awase-windows/src/msime_legacy_keymap.rs:452`、`NoTsf3Override2`）は実装済みだが、呼び出し元は不具合報告（`runtime/message_handlers.rs:1646`）だけ。T5 の指紋・既知構成判定にはまだ接続されていない。したがって残作業は「ADR-197 決定4の読み取り関数を T5 の指紋に接続する」ことと、採点日時フィールドの追加。採点日時は `crates/awase-keymap-learn/src` に該当するフィールドが見当たらなかった（フィールド名は未確認）。→ [adr196-t5-revalidation-not-invalidation](adr196-t5-revalidation-not-invalidation.md) で追跡（同文書の「ADR-197待ち」の表現の更新は T5 側に申し送る）。検証: 判定の純粋部分は Linux テスト、レジストリ読み取りは物理実機または windows-latest CI。
- [ ] **ADR195-T7 項目3**（セキュリティソフトの検知、TsfNative への副作用）。項目5（MSI 同梱と署名）は [03](review-2026-09-24-03-release-bundle-keymap-learn-win.md)。→ [adr195-t7-safety-measures](adr195-t7-safety-measures.md)（`:15`・`:144` で未着手）で追跡。検証: 物理実機のみ。
- [ ] **`QUIET_WINDOW_MS`（T）と `SESSION_INVALIDATION_LIMIT`（N）を実測で確定する。** `crates/awase-keymap-learn-win/src/driver.rs:69`（`QUIET_WINDOW_MS = 200`）と `:73`（`SESSION_INVALIDATION_LIMIT = 3`）はどちらも「暫定値」のまま。ADR-196 は T を「`tuning.rs` の該当定数から導出し、`.claude/rules/tuning-constants.md` の実測義務に従って確定する」（`196-keymap-learn-truth-priority.md:114`）とし、T と N を「tuning-constants 規約に従い実測で確定」（`:31`）としている。ファイルは `tuning.rs` の外にあるが、ADR 自身が規約の適用を義務づけているので、**コミット本文に「測ったもの・ms・導出」を書く**（任意ではない）。T の導出元は awase のフォーカス起因処理（warmup・conv force・drift correction）の時間。PR #295 で変わった `KEY_EFFECT_SETTLE_MS` は打鍵後の予測の fence 用で、導出元ではない。→ [adr196-t1-external-write-observation](adr196-t1-external-write-observation.md)（`:53` で「項目3（quiet window）は未検証」）で追跡。検証: Windows の物理実機または windows-latest CI での測定が必須（Linux のテストでは代わりにならない）。**C-3 次第で優先度が上がる**（生存確認の代替として重要度が上がる。依存先は 01）。
- [ ] **`adr195-remaining-work-2026-09-23.md` の「## 2.」節（見送った低優先度の指摘）は全件見送りのまま。** 棚卸しして期限を切るか閉じる。→ [adr195-remaining-work-2026-09-23](adr195-remaining-work-2026-09-23.md) で追跡。検証: 文書のみ。
- [ ] **BUG-162**（ADR-186 撤去実験の `baseline` 構成が `outcome=Unwarranted` で FAIL）は未修正。`docs/known-bugs/index.md:166` が「未修正」、`BUG-162.md` の `fix_commits: []`。ADR-191 統合後の再検証メモ（`abef7194`）は追記済み。→ [BUG-162](../known-bugs/BUG-162.md) で追跡。検証: windows-latest CI の撤去実験（`baseline` が PASS に戻ること）。

## 保留事項（判断待ち）

- 領域A撤去後の drift correction 単独での TsfNative ON 回復の実機 A/B（[09](review-2026-09-24-09-remaining-active-writes-inventory.md)）。
- ConfirmMode 2択化の実装（04 の推奨モード統一の後、07 で扱う）。

## 受け入れ条件（本ファイル）

- B-7 の各項目は、リンク先の既存文書で完了したら本ファイルのチェックを付ける。新しいタスクファイルへは切り出さない。
- 01〜10 の優先度・依存が変わったら、全体表の該当行を同じコミットで直す（各ファイルの「他ファイルとの依存」節と矢印の向きが一致していること）。「各ファイルへの申し送り」の表の行は、該当ファイルが直ったら消す。
- 本ファイル中の review-2026-09-24 系列へのリンクがすべて実在する。検証（Linux、ドキュメントのみ）:
  ```sh
  cd <worktree> && grep -o 'review-2026-09-24-[0-9a-z-]*\.md' docs/tasks/review-2026-09-24-11-low-priority-backlog.md | sort -u | while read f; do test -f "docs/tasks/$f" || echo "missing: $f"; done
  ```
  出力が空なら合格。

## 未確認点

- 優先度は俯瞰レビュー D節の判断に、01〜10 の書き直しで変わった点（01 の格下げ）を加えたもの。ユーザーの日程・リリース予定は反映していない。
- T5 の採点日時フィールドが別名で実装済みかどうか（`scored_at`／「採点」で grep して見当たらなかっただけ）。
- `QUIET_WINDOW_MS` の導出元になる `tuning.rs` の定数がどれか（ADR-196 は「該当定数」とだけ書いている）。

## レビュー反映メモ

Opus レビュー（`cbae84ff` 基準、12件）を `5877f982` で1件ずつ裏取りした結果。

- 1（#293 で B-2 解消）: 反映。`git branch -a --contains 4f3291a8` と `d00ac8dd` で確認。裏取り基準を `5877f982` に更新し、#294 の影響（01 のシナリオ2）も記載した。01 側は既に書き直し済みなので申し送りは不要になった。
- 2（C-7 の割当なし）: 反映。表の担当指摘列と、C-7 の吸収先を示す段落を追加した。
- 3（「確認できなかった点」の行き先）: 反映。対応表を追加した。`windows-build` の件は 03 で確認済みなので、その旨も書いた。
- 4（D節からの格下げ）: 04・05 は「次リリース前」に戻した。01 は 01 本文の根拠（A-1 の失敗シナリオの誤り、B-2 解消）があるので格下げを維持し、理由を明記した。保留項目（A-9・B-6・B-8）は表に「保留」と注記した。
- 5（個別ファイルへの切り出しは重複を生む）: 反映。各項目に既存文書へのリンクを付け、受け入れ条件を「既存文書で完了したらチェック」に変えた。
- 6（T/N と実測義務）: 根拠1・2は反映（ADR-196 `:114`・`:31`、`driver.rs:73` を確認）。根拠3は反映しなかった。`KEY_EFFECT_SETTLE_MS` は打鍵後の予測の fence 用（`state/ime_model.rs:614`）で、フォーカス起因処理の時間ではないので、T の導出元ではない。その旨を本文に書いた。
- 7（検証手段の区別）: 反映。各項目に「検証:」を付けた。
- 8（C-3 依存の範囲）: 反映。項目9の配線と永続化にも「C-3 次第（01）」を付けた。
- 9（frontmatter）: 反映。`status` を索引であることが分かる値にし、`related_adr` は B-7 に直接関係するもの（ADR-186・ADR-197 を追加、ADR-191・ADR-176 は外した）と範囲を本文に明記した。独自書式であることも本文に書いた。
- 10（「ADR-197 待ち」は曖昧）: 反映し、さらに訂正した。レビューは決定4の実装状況を「未確認」としていたが、ADR-197 の status は「実装済み（決定4のみ）」で、読み取り関数（`msime_legacy_keymap.rs:452`）もある。残りは T5 の指紋への接続。
- 11（リンク検証の手段）: 反映。コマンドを受け入れ条件に付けた（`xargs test -f` だと欠落したファイル名が出ないので、名前を表示する形にした）。
- 12（週次 A-2 との ID 衝突）: 反映。冒頭の ID 衝突の注意に `7fa894bb` の例を書いた。

### 再確認レビュー（`5877f982` 基準）への対応と、01〜10 の修正後の再整合

- A（02 が「01・03 とセット」のまま）: 11 の表は「01 と独立」のままにした（02 `:124-125` の本文も「独立に着手可」）。02 `:127` の古い記述は申し送り表に載せた。
- B（10 の依存の矢印が逆）: 11 の表の 10 行を「01・06・07 → 10」の形に書き直し、10 `:164` の訂正は申し送り表に載せた。
- C（C-7 の吸収先）: 01（`:7`・`:98`）と 08（`:6`・`:11`）には記載があることを確認した。02 にだけ無いので、C-7 の段落にその旨を書いた。
- 参考（01 の「01 ↔ 06」）: 01 `:168` は既に「06 → 01」になっているので対応不要。
- 01〜10 の修正で生じた食い違いも再整合した。09 `:110`（`ime.rs:1742` は `set_ime_mode` 内の委譲。`set_ime_mode` の呼び出し元が無いことを grep で確認した）、10 `:167`（実行される hook は `core.hooksPath` から `.githooks/pre-push`）、10 `:169`（B-8 は保留にしない）、04 `:227`（T1 は `main.rs:48`・`:51` を含む）、06 `:84`（07(2) は 06 を待たない。`msime_key_assignment.rs:281` の `DefaultHasher` を確認した）を、それぞれ表・「裏取りで変わった点」・推奨順に反映した。
