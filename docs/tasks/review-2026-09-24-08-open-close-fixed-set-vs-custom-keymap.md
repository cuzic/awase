---
title: ADR-189 の開閉トグル固定セット（0x19/0xF3/0xF4）とカスタムキーマップの衝突、ATOK 本体での予測/書き込みの非対称
status: 方針決定済み（(B) 案、2026-09-24）。実装は 01 の解決待ち
created: 2026-09-24
related_adr: ["ADR-189", "ADR-191", "ADR-192", "ADR-195", "ADR-186"]
source_review: 俯瞰レビュー（受動化・actuation撤去・学習/較正・config棚卸し・v2方針、2026-09-24）の C-1 / C-6（ATOK 部分のみ）/ C-7（開閉書き込み側の結論のみ）
---

# 開閉書き込みの固定セット vs カスタムキーマップ（俯瞰レビュー C-1 / C-6 / C-7）

C-7 の 08 担当分: ADR-195(A) により開閉書き込みは意図的に学習表を使わない（下記 C-1 の論点 (B) はこれを変える案）。

索引・優先度: [review-2026-09-24-11-low-priority-backlog.md](review-2026-09-24-11-low-priority-backlog.md)。
裏取り基準は worktree の `5877f982`（origin/develop、PR #296 まで。`cbae84ff` から本タスクの対象ファイル
〈`runtime/mod.rs`・`vk.rs`・`hook.rs`・`state/key_effect_predictor.rs`・`state/state_dependent_key_warning.rs`・
`runtime/transport.rs`・`src/config.rs`〉に差分なし）。着手時は `.claude/rules/worktree-per-session.md` に従い
専用 worktree/branch を切ること。

## ユーザー決定（2026-09-24）

- **(B) 案を採る**: 採用中の学習表でそのキーのセルが開閉トグル以外を示すときだけ、固定セット（0xF3/0xF4）の `shadow_action` を外す。
  ADR-195(A)「許可リストを学習で動かさない」は**拡大方向の禁止**で、縮小方向（学習を見て外す）は禁止範囲外である。
  ただし明文が無いので、ADR-195(A) に「縮小方向は許す」旨を追記して整合を取ること（本タスクの最初の作業）。
- **0x19 の非対称は現状維持**: ADR-191 が固定セットとして明示的に残した例外。ATOK 本体・未検出で予測しないのに書く非対称は、
  文書に明記するだけにする。`hook.rs` の静的 Toggle と `keys.ime_toggle` 既定は変えない。
- **前提が未成立**: 3f の実測（PR #303、run 35987424778）で、学習表は GJI+ATOK（カバレッジ 0.782）も MS-IME 本体（0.517〜0.531）も
  カバレッジ判定（0.80 未満で棄却）で棄却され、**現状どの構成でも内蔵表に戻る**。(B) は学習表が実行時に採用されることが前提なので、
  [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md) のカバレッジ判定の扱いが決まるまで実装できない（01 → 08）。
  実装するまでの間は、(C) の警告強化を暫定策にするかどうかを別途判断する。
- (B) では、学習表が無い・棄却されたときの既定は「固定セットを維持」（従来どおり）とする。未確認のカスタム構成で外すことはしない。

## 現状（裏取り済み）

### C-1: 固定セットはキーマップを見ない（ただし ADR-191 はそれを明示的に選んでいる）

**書き込み箇所は2つ**（`tests/architecture_guard.rs::ime_relevance_shadow_action_writes_are_accounted_for` が
`hook.rs` 1件 + `runtime/mod.rs` 1件の計2箇所に固定）:

- 0x19（`VK_KANJI`）: `hook.rs::classify_ime_relevance`（`:296-311`）が `vk.rs::ImeKeyKind::shadow_effect`（`:156`、
  `Self::Kanji => Some(Toggle)`）から初期値を書く。**IME 種別に依らない**（ATOK・未検出でも書く。`vk.rs:181-184` の doc）。
- 0xF3/0xF4: `runtime/mod.rs::enrich_ime_relevance`（`:572-599`）が、無修飾かつ `tsf_obs().table_ime_kind()` が
  `Some`（GJI か CLSID 同定済み MS-IME 本体）で `is_open_toggle_for(ime)`（`vk.rs:189-196`、0xF3/0xF4 のみ真）なら
  `shadow_action = Some(Toggle)` を上書きする。
- 0x19 は別経路でも能動的に消費される: `keys.ime_toggle` の既定が `["VK_KANJI"]`（`src/config.rs:583`）で、
  `Engine::apply_special_key_match`（`src/engine/engine.rs:975`）が漢字キーを consume して冪等 ON/OFF を送る。
  `ImeDetectConfig::default().toggle` から VK_KANJI を外した経緯（`src/config.rs:495-507`、二重反転で「押しても動かない」
  キーになった）がある。**0x19 を触るときはこの2経路の相互作用を同時に見る必要がある**（相互作用の現状は未確認）。

**ADR 上の位置づけ**（元レビューの「実装が ADR-191 決定(2) と食い違う」は不正確、下記メモ参照）:

- ADR-191 本文の決定1-1（`docs/adr/191-ime-is-source-of-truth-observe-not-write.md:140`）は固定セット（0x19/0xF3/0xF4）と
  `keys.ime_toggle` を「静的に残す唯一の明示的な例外」とし、`:344`/`:429` で「撤去しない」と明記。実装は本文どおり。
- さらに `:156`（round3 RM3）が **「固定の例外と表が矛盾したときは固定が常に勝つ。カスタムキーマップのユーザーで固定セットが
  誤っていれば ADR-192 の検出・警告の対象になる」** と、本タスクの衝突そのものを既に判断済み。
- 食い違っているのは ADR-191 **frontmatter summary の (2)**「線引きはキーの VK でなく、表の作用分類(a〜e)で決める」と本文の間
  （summary が本文の決定1-1/RM3 を反映していない）。ADR-189 本文には「カスタム」への言及がない（grep 0件）。
- ADR-195(A)（`docs/adr/195-keymap-learn-productization.md:12`, `:85-88`, `:132`）は「actuation 許可リストは ADR-189＋ユーザー明示
  config のまま、学習結果から自動拡張しない」。縮小方向（学習/設定を見て外す）は同 ADR の禁止範囲外だが、明文もない。

**現行の緩和策（ADR-192 警告）**: `state/state_dependent_key_warning.rs` の `TARGET_VKS`（`:10`）は 0xF3/0xF4/0x19 を含み、
GJI のカスタム表が 0xF3/0xF4 の行を持てば `key_effect_table.rs:560` 経由で `CannotPredict(UserOverride)` → `WarningKind::UserOverride`
（「awaseはこのキーの効果を追随できない可能性があります」`:90`）になる。ただし (i) ダイアログは出ずログのみ
（`WarningDialogTracker::select` が `UserOverride` で `None`、`:69`）、(ii) 文言は「awase が握りつぶしてトグルを書く」ことを伝えない、
(iii) 0x19 は `mozc_tokens`（`key_effect_predictor.rs:638-652`）に無いので検出されない。

**失敗シナリオ**: GJI のカスタムキーマップで半角/全角に「ひらがなモード」等を割り当てた利用者。`shadow_action=Toggle` が付くと、
`transport.rs::PhysicalKeyDisposition::plan`（`:173-338`、`is_dbe_mode_key_down` `:327-335`）で、ImmCross では常に、それ以外では
`ime_actuation_owned`（GJI/MS-IME direct 適用可）かつ（`shadow_toggled` または 0xF3/0xF4 の KeyDown または KeyUp）のとき物理キーが Suppress され、awase が
開閉トグルを書く。割り当てた機能は動かない（injected・InputRelay は Allow）。
また、`shadow_action` を持つキーは予測経路から除外される（`runtime/key_pipeline.rs:1874`, `:1929`, `:1944`）ので、
**仮に学習表が正しくても、固定セットが先に Toggle を付けるため予測は使われない**。カスタムキーマップの 0xF3/0xF4 を GJI で学習・検証した
実績はリポジトリ内で未確認。

### C-6（本タスクの範囲は ATOK 本体の非対称のみ）

- ATOK 本体（GJI の ATOK プリセットではないもの）: `ActiveImeKind::MicrosoftIme` 扱いで `ms_ime_native_identified()` が偽なら
  予測に入らない（`runtime/key_pipeline.rs:1967` の `ActiveImeKind::MicrosoftIme => return`）。0xF3/0xF4 は `table_ime_kind()==None` で
  書かれないが、**0x19 は hook.rs の静的 Toggle と `keys.ime_toggle` 既定で書かれる**。予測はしないのに書き込みはする。
- C-6 の MS-IME 本体（学習が `NeedsConfirmation(UnverifiedMsImeNative)`、`crates/awase-keymap-learn/src/judgement.rs:67,107`）と
  GJI（CI run 35931602595、prediction 落とし経路未通過）の項目は [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md) で扱う。
- MS-IME 本体の実行時キーマップが持つ再割り当て情報は `henkan_reassigned`/`muhenkan_reassigned` のみ（`key_effect_predictor.rs:553-565`、
  `custom_table: None`）。**MS-IME 本体で半角/全角の割り当て変更を awase.exe は検出できない**ので、本タスクのスコープは GJI のみ。

## 論点（ADR を先に、`opus-adversarial-consult`）

ADR-191 RM3 の「固定が常に勝つ＋警告」を維持するか、「カスタム割当時は固定セットを外す」に変えるか。変える場合の判定基準が要となる。

- **既存関数をそのまま流用すると誤検出の恐れ**: `custom_table_overrides(custom_table, vk)`（`key_effect_predictor.rs:656`）は
  「その VK の行が**ある**か」を見るだけで「既定と**違う**か」は見ない。Mozc/GJI のカスタム表はプリセットの複製から編集を始めるので、
  半角/全角を変えていない利用者の TSV にも `Hankaku/Zenkaku` 行がある可能性が高い（**実ファイルでは未確認**）。流用すると
  カスタムキーマップ利用者全員で固定セットが外れ、TsfNative（観測できない窓）での開閉追随が失われる。
- 比較元プリセットはカスタム表に記録されない（`is_unmodified_bundled_config` `:625-633` が `Custom` を「基準の同梱表が無い」と扱う）。
- 候補（作用ベース）:
  - (A) カスタム表の 0xF3/0xF4 行の全 status が「DirectInput→IMEOn、開状態→IMEOff」のトグル以外のコマンドを含むときだけ外す。
  - (B) 採用中の学習表でそのキーのセルが開閉トグル以外を示すときだけ外す（ADR-195(A) の「許可リストを学習で動かさない」との整合を ADR で要明記）。
  - (C) 外さず RM3 を維持し、警告を強化（UserOverride でもダイアログ、文言に「awase が開閉トグルとして扱う」旨）。ADR-191 summary (2) を本文に合わせて訂正。
- 0x19 を ATOK 本体・未検出で書き続けるか（C-6 の非対称）。縮小するなら `hook.rs` の静的 Toggle と `keys.ime_toggle` 既定の両方が対象になる。

## タスク

- [ ] 論点を ADR-189/191 への追記（または新 ADR）として起票し、`opus-adversarial-consult` で (A)/(B)/(C) を決める。併せて ADR-191
      frontmatter summary (2) と本文（決定1-1・RM3）の食い違いを訂正する（summary (2) の訂正は [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) には含まれない〈10 の ADR-191 行は status 同期のみ〉ので 08 で行う）。
- [ ] GJI 実機で、半角/全角を**変えていない**カスタムキーマップの TSV に `Hankaku/Zenkaku` 行が残るかを確認する（(A) の成否を決める）。
- [ ] Mozc が VK_KANJI(0x19) を TSV にどのキー名で書くかを確認し、`mozc_tokens` への追加要否を判断する（未確認）。
- [ ] (A)/(B) を採る場合: 判定を純粋関数として `state/`（`key_effect_predictor.rs` 近傍、または `is_open_toggle_for` に引数追加）に置き、
      `enrich_ime_relevance` からは `KeyEffectKeymap` のキャッシュ（`key_pipeline.rs:1955-1966` と同じ取得経路）を参照する。全打鍵経路なので
      毎打鍵のコスト（TSV 走査）を避ける前計算を用意する。`runtime/mod.rs` の代入は1回のまま（条件を前段に畳む）にし、architecture_guard の
      2箇所固定を維持する。
- [ ] 0x19 を変える場合: 上書き場所を決める（(a) hook.rs に情報を渡す＝hook.rs は config もキーマップも持たない、(b) `enrich_ime_relevance` で
      `None` に上書き＝ガード件数 or 方式の変更が要る）。`keys.ime_toggle` 既定と `Engine::apply_special_key_match` の扱いも同時に決める
      （`fix-requires-evidence.md` のキー選択ファミリー、ルート `awase` クレート側も含む）。
- [ ] 影響洗い出し: `transport.rs::plan` の `is_dbe_mode_key_down`（`:327-335`）を起点に VK 0xF0〜0xF6 全種（`fix-requires-evidence.md` の
      物理IMEキー配送ファミリー）。`plan` は `:293-296` で `shadow_action.is_none()` なら即 `Allow` を返し、`is_dbe_mode_key_down`
      （`is_open_toggle_for` を直接見る）はこのガードを通過した `shadow_action` 付きのキーにしか評価されない（同ファイル `:315-321` の
      コメントも 0xF0/0xF1 について同じ仕組みを明記）。よって 0xF3/0xF4 の `shadow_action` を外せば物理キーは Allow になり、`plan` 側の
      変更は不要。ただし外した後に `is_dbe_mode_key_down` がデッドコード化しないか（`shadow_action` 付きかつ `shadow_toggled` が立たない
      0xF3/0xF4 のケースが残るか）と、同コメント `:322-326`（「0xF3/0xF4 は `enrich_ime_relevance` で必ず `Toggle` を持つ」）の前提が
      崩れる点を確認・更新する。外した後は予測経路に入り、カスタム表ガードにより学習表なしなら予測 None になる挙動を明記する。

## 受け入れ条件

- ホスト Linux（`cargo test -p awase-windows --lib` / `cargo test --lib`）: `state/` に置いた判定関数の単体テスト
  （トグルのみの 0xF3/0xF4 行→外さない、別コマンド割当→外す、行なし→外さない）。`architecture_guard`（Linux で走る）の2箇所固定が通る。
  0x19/`keys.ime_toggle` を変える場合は `src/engine/tests.rs` にテスト。
- Windows ターゲットのコンパイル: `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`（`runtime/` の配線、
  `#[cfg(windows)]` のため Linux の単体テストには存在しない）。`ime_key_sequence_golden`（`#![cfg(windows)]`）は windows-build CI で確認。
- 実機: GJI カスタムキーマップ（半角/全角=ひらがなモード等）で物理キーが IME に届き、belief が観測に追随すること（実タイピング確認）。
  無変更構成・トグルのみのカスタム構成では従来どおり開閉トグル（TsfNative の Chrome/VS Code でも）。(C) を採る場合は警告ダイアログの表示確認。

## 他ファイルとの依存

- [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md): (B) を採るなら 01 の「学習表の採用が効く」ことが前提（01 → 08）。C-6 の MS-IME/GJI 部分は 01 側。
- [06](review-2026-09-24-06-keymap-learn-staleness-wiring.md): キーマップ指紋・カスタム表の読み取り経路を共有（相互参照、どちらも先行不要）。
- [09](review-2026-09-24-09-remaining-active-writes-inventory.md): 能動書き込み棚卸しの一部（08 の結論を 09 の一覧に反映、08 → 09）。
- [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md): ADR-191 summary (2) の訂正は 10 には含まれず 08 で行う（10 は ADR-191 の status 同期のみ）。

## 未確認点

- 半角/全角を変えていないカスタム TSV に `Hankaku/Zenkaku` 行が残るか（実ファイル）。
- Mozc における VK_KANJI の TSV キー名。
- hook.rs の 0x19 静的 Toggle と `keys.ime_toggle` 既定（Engine 消費）が同一押下で現在どう調停されているか。
- 0xF3/0xF4 の書き込みを外したときの TsfNative での belief 追随の退行（実機未検証）。
- 外した後も ADR-192 UserOverride 警告の文言が「追随できない可能性」のままである（学習表で外した構成では実態と合わない。今回は未対応）。

## レビュー反映メモ（Opus レビュー、2026-09-24）

- 反映: 1（情報源は awase.exe 側に既存: `KeyEffectKeymap`/`custom_table_overrides`/`key_pipeline.rs:1955-1966`。未確認点から削除）、
  2（「既定と違う」判定の穴→論点 (A)/(B)/(C) と実ファイル確認タスク）、3（`keys.ime_toggle` 既定と Engine 消費を追加）、
  4（書き込み2箇所・0x19 は hook.rs）、6（スコープを GJI に限定）、7（0x19 キー名確認）、8（予測経路除外で学習表は使われない）、
  9（Suppress 条件を明記）、10（行番号訂正）、11（frontmatter・書式を 01 に合わせ、ADR-187 を外し ADR-192/ADR-186 を追加）、
  12（C-6 の MS-IME/GJI は 01 へ）、13（受け入れ条件を Linux/Windows check/CI/実機に分離）。
- 5 は一部訂正して反映: 「ADR はカスタムキーマップを考慮していない」は誤り。ADR-191 本文 `:156`（RM3）が「固定が常に勝つ、
  カスタムで誤れば ADR-192 で警告」と既に判断している。食い違いは summary (2) と本文の間。よって論点は「RM3 を維持するか変えるか」とした。
- 9 の後半「0xF3/0xF4 の `shadow_action` を外すと Allow になり変更はそれで閉じる」は正しかった（初回反映時に「誤り」として
  `plan` 側の変更をタスクに加えたが、再確認レビューの指摘どおり撤回）。`transport.rs:293-296` の `is_kanji_event` ガードで
  `shadow_action` なしは即 Allow になり、`is_dbe_mode_key_down`（`:327-335`）はその後段でしか評価されないことを `5877f982` で確認した。
  タスクは「`plan` 変更不要、デッドコード化とコメント前提の確認のみ」に改めた。
- 元レビュー提案の「ADR-191 決定(2) の文言と実装を合わせる」は、本文が実装と一致しているため採らず、summary の訂正に置き換えた。

## レビュー反映メモ（再確認レビュー、2026-09-24）

- 新たな誤り1（`plan` 変更は不要）: 反映。上記 9 の訂正とタスクの影響洗い出し項目を修正。VK 0xF0〜0xF6 全種の洗い出しは残した。
- 2（C-7 表記）: 反映。frontmatter `source_review` と H1 に C-7 を追加し、08 担当分を1行で明記した。
- 3（01 :161 への条件付き依存の追記）: 08 は変更不要との指摘どおり 08 は触らない。01 の追記は 01 側の担当（本作業は 08 のみ編集）。
- 4（10 との相互参照）: 08 側の案を採り、タスクの「相互参照」を「10 には含まれないので 08 で行う」に改めた（10 :35 は status 同期のみと確認）。
  09 :53 の表記揃えは 09 側の担当。
