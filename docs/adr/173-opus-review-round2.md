# opus-adversarial-consult round2: ADR-173 レビュー

対象: `docs/adr/173-scope-solo-tap-ime-action-to-tsf-native.md`（commit `389e8912`）
および `docs/known-bugs/BUG-142.md`。読み取りのみ、ファイルは編集していない。

**総評: Blocker ゼロ。設計判断としては収束と認める。** round1 の B1〜B4 はいずれも
適切に解消された（B1/B2 はプロセス名方式への転換で原理的に消え、B3 は決定2で
明示され、B4 は BUG-142 として分離された）。ただし**実装に入る前に決めるべき
Should-fix が6件**残っており、うち R2-1（配線先の指定が既存パターンの誤った半分を
指している）と R2-2（既定値の後方互換）は、そのまま実装すると実害が出る。
これらは設計の欠陥ではなく**記述の精度の問題**なので、ADR を直せば収束と判定してよい。

---

## round1 指摘の解消確認

| round1 | 判定 | 根拠 |
|---|---|---|
| B1 クラス判定が Windows Terminal を取りこぼす | **解消** | プロセス名方式に転換。`WindowsTerminal.exe` は CASCADIA⇔InputSite の往復で変わらない（R2-3 で実機裏取りを補強） |
| B2 KeyDown/KeyUp ペアリング不変条件の破壊 | **解消** | 同上。フォーカスがプロセス内に留まる限り Down/Up で判定が変わらない（R2-3 に残余の注記あり） |
| B3 ケース1（コア側）の扱い | **解消** | 決定2で明示。ADR の主張の妥当性は R2-4 で独立に検証した——**主張は正しい** |
| B4 「IME ON固着」の因果取り違え | **解消** | BUG-142 として分離、受け入れ基準から除外、対立仮説（ADR-172 4系統）も記録済み |
| S1 stale window | 未決（ADR 自身が round2 送り） | → R2-5 で回答 |
| S2 ログ | 未決項目として記載 | → R2-6 |
| S3 却下した代替案 | **解消**（:57-95 に新設） | |
| S4 スコープが広い | **解消**（プロセス名方式で消滅） | |
| S5 テスト置き場所 | 未決項目として記載（:197-206） | → R2-6 |
| S6 index/related_adr | **解消**（`index.md:181`、`related_adr` に ADR-172） | ただし R2-7 に誤記1件 |

また round1 で私が指摘しなかった追加の副作用を確認したところ、**問題なし**だった:
決定1でスコープ外になったアプリでは `explicit_ime_action_consumed` が立たないため、
`nicola_fsm.rs:2203` の BUG-123 早期打ち切りを素通りして優先順位3/4
（delegate / `ModeKeyConfig`）へフォールスルーする。これは ADR-153 適用前の
既存挙動そのものであり、ケース2が actuate しない以上 BUG-123 の二重送出は
成立しない。**意図どおり。**

---

## Should-fix

### R2-1. 決定1の配線先が、`input_relay_apps` パターンの「コピーしてはいけない方の半分」を指している

ADR :106-111 は「`FocusTracker` に…`input_relay_apps`（`:80-87`、
`overrides.input_relay_apps()` 経由で**プロセスグローバルにキャッシュされる**
既存パターン）と同じ配線で `solo_tap_ime_action_apps` を保持させ」と書いている。
この1文は、実際には**別々の2つの機構**を1つに混ぜている。

**コピーすべき半分（正しい配線）**:
- `AppOverrides` にフィールドを追加（`src/config.rs:728-765`）
- `ForceOverrides { inner: AppOverrides }`（`focus/classifier.rs:115-140`）にアクセサを追加
- `FocusTracker.overrides: ForceOverrides` へ委譲するアクセサを追加
  （`focus/tracker.rs:80-87` の `input_relay_apps()` がまさにこの形）
- 設定リロードは `FocusTracker::reset_overrides()`（`tracker.rs:247-249`）が
  `ForceOverrides` ごと差し替える（`runtime/mod.rs:1980-1984`）ので、**追加作業は不要**

→ ADR の「`FocusTracker` に…保持させ」は、正確には
「`AppOverrides` に持たせ、`ForceOverrides`→`FocusTracker` の既存委譲チェーンに
アクセサを1本足す」。`FocusTracker` に新フィールドを足すのは誤り
（リロード時に差し替わらない新しい経路を作ってしまう）。

**コピーしてはいけない半分**: `focus/classifier.rs:29` の
`static INPUT_RELAY_APPS: OnceLock<RwLock<Vec<String>>>`。
これは `ForceOverrides::new` の**副作用**として複製されるプロセスグローバルで、
その doc（`classifier.rs:20-28`）と `tracker.rs:80-86` と CLAUDE.md の3箇所が、
存在理由を明示的に限定している:

> `read_ime_state_fast` は…`offload_unsafe`（ワーカースレッド）からも直接からも
> 呼ばれる…（`tracker.rs`）正規ルート。…`FocusTracker` に到達できる呼び出し元は
> こちらを使うこと。

`explicit_ime_action_target` は `&self` を取り `self.platform.focus` に到達できる
（同ファイル `key_pipeline.rs:659` が `self.platform.focus.class_name()` を
実際に使っている）ため、**グローバル static は一切不要**。

`feedback_no_raw_global_statics_prefer_static_struct` と ADR-164（裸のグローバル
static の集約）の方向に真っ向から反するので、ADR に
**「新しいプロセスグローバル static は追加しない（`INPUT_RELAY_APPS` は
`self` を持てない `read_ime_state_fast` 専用の例外であり、本件は該当しない）」**
と明記すること。この1行が無いと、実装者が「input_relay_apps と同じ配線」を
素直に読んで2つ目の static を作る。

### R2-2. 既定値（空 = 無効）が、既存ユーザーにとってサイレントな後方互換破壊になる（未記載）

ADR :104 は `solo_tap_ime_action_apps` の既定値を「空 = 無効」としている。
これは、**現在 `muhenkan_solo_tap_ime_action = "off"` を設定しているユーザーが、
本 ADR 適用後に何も書き加えないと、全アプリで回避策を失う**ことを意味する
（＝「@」が Windows Terminal で復活する）。ADR にはこの移行についての記述が
一切無い。

「空 = 無効」が唯一の流儀でもない——同じ `AppOverrides` 内で
`default_disable_apps()` は `vec!["mstsc.exe"]`（非空既定、`config.rs:768-770`）、
`default_input_relay_apps()` は空、と両方の前例がある。

**選択肢は2つ。どちらでもよいが、選んで書くこと:**

- **(a) 空 = 従来どおり全アプリ（推奨）**: 本 ADR が純粋な機能追加になり、
  後方互換の破壊がゼロ。ユーザーは `["WindowsTerminal.exe"]` を書いた時点で
  限定される。要望（Terminal だけで効かせたい）は同じく満たせる。
- **(b) 空 = 無効（ADR の現案）**: 「限定が既定」で意味は素直だが、既存設定が
  黙って無効化される。採るなら、**移行手順（既存ユーザーは1行足す必要がある）**
  と、`CHANGELOG.md` へのユーザー向け記載を「次のアクション」に加えること。

いずれにせよ `src/config.rs` の `muhenkan_solo_tap_ime_action` doc（`:430-465`、
`explicit_ime_action_target` の doc を正本と宣言している箇所）と、正本側
（`key_pipeline.rs:1166-1203`）の両方に「どのアプリで効くか」を書く必要がある。
現在の「次のアクション」にこの doc 更新が入っていない。

### R2-3. 「構造的に回避できる」（:122-124）は過大主張。ただし実機裏取りで補強できる

プロセス名も、フォーカスがプロセスを跨げば変わる。したがって「構造的に回避」
ではなく「実質的に回避」。**ただし ADR に有利な実機証拠がリポジトリ内にある**ので、
推測ではなく観測として書き直せる:

- `docs/bug-reports-triage.md:40`（不具合報告 `01M0VGJ2M5KQHD1D9V7HAMBHNT`）に
  `HwndCache: restore [2848 InputSite] ime_on=false` とあり、**pid 2848 =
  WindowsTerminal.exe** である。つまり Windows Terminal の
  `Windows.UI.Input.InputSite.WindowClass` 子ウィンドウは**同一プロセス内**であり、
  round1 B1 で問題にした CASCADIA⇔InputSite の往復では**プロセス名は変わらない**。
  これは決定1の根拠そのもの。
- 残余: 同じ triage 行が Alt+Tab スイッチャーを「explorer.exe ホストの UWP
  InputSite」と記録している。キー押下中に Alt+Tab 等でプロセスを跨げばプロセス名も
  変わるが、**その場合フォーカスは実際にアプリを離れている**ので判定が変わること
  自体は妥当。

→ :122-124 を「構造的に回避できる」から「実機ログで確認済み（同一プロセス内の
クラス往復ではプロセス名は変わらない、`docs/bug-reports-triage.md:40`）。
プロセスを跨ぐフォーカス移動では変わりうるが、その場合フォーカスが実際に
アプリを離れているため妥当」に書き換えること。

### R2-4. 決定2の理由は**正しい**が、ADR に根拠が無い。加えて、このユーザーに限れば矛盾は発生しない可能性が高い

ADR :154-156 の「ケース1は明示的に IME を操作する設計であり、GJI の生キー横取りを
経由しない」は、round1 時点では未検証の機序主張だったので疑ったが、
**コードを追った結果この主張は正しい**:

- `nicola_fsm.rs:2175-2185`: ケース1が発火すると `actions: SmallVec::new()`
  （生キーを一切送出しない）+ `Some(explicit_action)` を返す。
- 元の KeyDown は既に PendingThumb として `Decision::Consume` 済み
  （`transport.rs:349-352` がケース2について同じ構造を明記している）。

→ ケース1では生の `VK_NONCONVERT`/`VK_CONVERT` が GJI に届かない。
BUG-123 型の二重送出も起きない。**この2行の根拠を ADR に足すこと**（無根拠の
断定のままだと、次に読む人が疑って調査をやり直す／逆に誤って一般化する）。

さらに、**このユーザーに限れば決定2の「残る矛盾」（:151-158）は実際には発生しない
可能性が高い**: ケース1は M13 を維持しており（`resolve_explicit_ime_action:2066-2071`
の `mode_key_config.is_some_and(is_passthrough)` フィルタ）、
`key_pipeline.rs:1222-1228` が記録するとおり
「実機のユーザー設定 `muhenkan_solo_tap_always_suppress = false`（ADR-153 以前からの
legacy 設定）が常に Passthrough へ解決され、明示 config 機能が恒久的に無効化されて
いた」。つまり報告者の config が今もこの値なら、**ケース1はそもそも一度も発火しない**。
ADR に「実機 config の `*_solo_tap_always_suppress` を確認すること」と一行足せば、
決定2の残余リスクを実質ゼロと確定できる（未確認なので条件付きで書くこと）。

### R2-5. S1（stale window）への回答 — 新しい窓は生まれない。そう書いて閉じてよい

ADR :185-193 が round2 送りにした項目。結論は「**フェンシングは足さない。ただし
その理由を書く**」でよい。根拠:

- `CurrentFocus::update_with_process_name`（`focus/current.rs:57-83`）は
  `process_name`・`class_name`・`app_profile` を**同一の代入ブロックで同時に**更新する。
  したがって `process_name()` の stale 窓は `current_app_profile()` のそれと
  **完全に同一**であり、プロセス名方式に変えても S1 は縮まない（が、**広がりもしない**）。
- その窓は既に `transport.rs::PhysicalKeyDisposition::plan` を含む**全ての
  プロファイル依存判定が共有している既存の窓**であり、本 ADR が新設するものではない。
- 窓の上限も限定的: `FocusTracker::update_with_process_name` は実フォーカス変更時
  だけでなく `ir_stage_focus` の 500ms 周期リフレッシュからも毎ティック呼ばれる
  （`tracker.rs:126-134` の BUG-111 コメント）。
- fail 方向は**両方向にありうる**: スコープ外と誤判定 → fail-open（生キーが GJI へ
  漏れて「@」）、スコープ内と誤判定 → 直前アプリで生キーを抑止。前者が主。
- フェンシングを足すなら `injection_hint_for(pid, class_name)`
  （`tracker.rs:104-113`、「フォーカス変更直後の stale 回避用」）と同型になるが、
  `explicit_ime_action_target` は打鍵時点で pid/process を独自に持たないため、
  呼び出し側の改造が必要になり本 ADR のスコープを超える。

→ ADR の未確定項目1を、上記を根拠に「**新しい stale 窓は作らない。既存の
プロファイル依存判定と同一の窓を共有するのみ**」と結論づけて閉じること。

### R2-6. 未確定項目2（ログ）・3（テスト）は round2 で決め切ってよい

どちらも判断材料が揃っており、round3 を待つ理由が無い:

- **ログ（:194-196）**: 出すべき。`kp_stage_shadow_ime_toggle` は他の全分岐で
  `tracing::info!` を出している（`:1361`, `:1386`）。スコープ外で無言 `Inactive` を
  返すと、次の「設定が効かない」報告時に app.log から
  「スコープ外」「belief ON」「`dedicated_fn_key` で弾かれた」(`:1236-1240`) の
  区別が付かない。プロセス名を含む1行を、レート制限付き（打鍵ごとに出ると
  journal を圧迫する）で。
- **テスト（:197-206）**: 未確定項目4が既に答えを出している——
  `state/app_suppression.rs` の `matches_disabled_app` は Linux で走る純粋関数。
  ただしそこに足せるのは**照合ロジック**のテストだけで、
  「`explicit_ime_action_target` がそのガードを実際に呼んでいる」ことは固定できない。
  round1 S5 で指摘したとおり、既存の `architecture_guard.rs` の2本
  （`kp_stage_shadow_ime_toggle_never_reintroduces_case3_forced_actuate`、
  `explicit_ime_action_case1_keeps_m13_but_case2_3_does_not`）は本変更に一切反応
  しないため、**ガードが後日削除されても誰も気付かない**。
  `architecture_guard.rs` に「`explicit_ime_action_target` の本体が
  `solo_tap_ime_action_in_scope` を含む」というソーススキャン1行を足すのが、
  Linux で走る唯一の実効的な回帰。**「検討する」ではなく「足す」と書くこと。**

### R2-7. `BUG-142.md` の frontmatter に存在しない ADR への参照がある

`docs/known-bugs/BUG-142.md:6`:
```yaml
related_adr: ["ADR-113", "ADR-153", "ADR-172", "ADR-173"]
```

**ADR-113 は存在しない**（`docs/adr/` は `112-keyup-lifecycle-fsm-delivery.md` の次が
`114-...` で欠番。`grep -rn "ADR-113" docs/` のヒットはこの1行のみ）。
意図は **BUG**-113 のはず。`.claude/rules/docs-frontmatter-convention.md` の
`related_adr` は ADR 専用のフィールドで、関連 BUG は本文に書く流儀
（`BUG-124.md` が `related_adr: ["ADR-153"]` とし BUG-113 は本文で参照している）。
`"ADR-113"` を削除すること（BUG-113 は既に本文「関連」節に書かれている）。

---

## Nit（任意）

- **N1. ファイル名が却下案のまま**: `title`/H1 は「プロセス名指定のアプリ限定にする」に
  更新されたが、ファイル名は `173-scope-solo-tap-ime-action-to-**tsf-native**.md`。
  却下した案の名前が残り grep を汚す。番号が正本なので実害は小さいが、
  リネームするなら `index.md:181` のリンクも同時に直すこと。
- **N2. 受け入れ基準のプロファイル表記**（:216-218「メモ帳 = Standard、Chrome =
  Imm32Unavailable」）: 決定1がクラス名ベースをやめた以上、条件は
  「`solo_tap_ime_action_apps` に載っていないこと」であり、`AppImeProfile` は
  もう関係ない。誤導的なので削るか、「（参考）」と明示すること。
- **N3. config バリデーションの3重化**: `config.rs:1282-1292` には
  `check_disable_apps_list` / `check_input_relay_apps_list` という、メッセージ文字列
  だけが違うほぼ同一の関数が既に2本ある。3本目を足すより
  `check_process_name_list(list, "solo_tap_ime_action_apps", w)` に一般化する
  タイミング（ADR-158 の複雑さ削減方針とも整合）。
- **N4. ADR-172 側からの相互参照**: ADR-172 は別ブランチ
  （`adr/172-tsf-blind-rescue-consolidation`）なので今は触れなくてよいが、
  マージ時に ADR-172 の `related_adr` へ `"ADR-173"` を足すこと
  （現状は ADR-173 → ADR-172 の片側のみ）。

---

## 収束判定

**Blocker ゼロ。設計判断（決定1・決定2・BUG-142 の分離）は収束と認める。**

実装着手前に、少なくとも次の3件は ADR 本文に反映すること
（いずれも記述の修正のみで、設計の再検討は不要）:

1. **R2-1**: 配線先を「`AppOverrides` → `ForceOverrides` → `FocusTracker` の
   委譲チェーン」と正確に書き、**新しいグローバル static は作らない**と明記する。
2. **R2-2**: 既定値（空 = 全アプリ / 空 = 無効）を選び、後者なら移行手順と
   CHANGELOG 記載を「次のアクション」に加える。
3. **R2-7**: `BUG-142.md` の `related_adr` から存在しない `"ADR-113"` を削除する。

R2-3〜R2-6 は ADR の精度・追跡可能性を上げるもので、反映すればそのまま実装に
進んでよい。round3 を回す必要は無いと判断する。
