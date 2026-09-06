# ADR-137: `VK_DBE_*` KeyDown 無条件 Suppress（BUG-52対策）が Shift+かな→カタカナ変換を巻き添えで殺している（BUG-116）

## ステータス

**実機検証完了・決定確定（v2）・本実装完了（v3、`fix/bug116-shift-katakana-return`
ブランチ）。**

本実装は develop の最新先端（BUG-115 マージ後）から分岐しており、BUG-115
（`docs/known-bugs.md` BUG-115 参照）が追加した `mode_key_delegate_owns_shadow_toggle`
機構との衝突を含む新たな Blocker 3件を、本実装向けの Opus 2体敵対的レビュー
（`opus-architect-bug116-impl`/`opus-premortem-bug116-impl`）で発見・全て
対処済み。詳細は下記「本実装のレビューで発見した追加の論点（v3）」参照。

`diag/bug116-shift-katakana` ブランチ（develop 非マージの診断スパイク）を
実機（TsfNative + GJI、Windows Terminal 環境相当）にビルド・投入し、
2026-09-05 に以下を確認した:

1. **B-1 は成立方向で確認**: Shift+かな押下時は `vk=0xF1 scan=0x70
   shift=true` が観測され、Shift なし連打では `vk=0xF1` は一度も出ず
   `vk=0xF0`/`vk=0xF2` のみが交互した（サンプル数は限定的だが、v1 の
   弁別仮説と矛盾する観測はゼロ件）。
2. **B-2 は該当環境では杞憂と判明**: `AWASE_BUG116_SCAN=zero`
   （`reinject()` の `wScan: 0` のまま、scan 情報を一切変更しない）でも
   `AWASE_BUG116_ALLOW=shift` により Shift+かな→カタカナが実際に動作した。
   **scan 付与（SB-1 のハザードがある危険なモード）は一度も試す必要がなく、
   本 ADR のスコープから完全に除外できる。**
3. **B-3 も解消**: 実機ログの `profile=TsfNative ime_kind=
   GoogleJapaneseInput` により、テスト環境は本 ADR の対象範囲内と確認。
   （報告者本人の環境が同一かは別途確認が望ましいが、少なくとも
   `Standard`/ImmCross のみに限定される症状ではないことが分かった。）
4. **B-4 は今回のセッションでは非再現**（`lshift_ms` は常に3桁ms オーダー
   の新鮮な値で stuck は観測されず）。恒久解決ではなく残存リスクとして
   維持する。
5. **新規発見（M-6）: 「カタカナへは入れるが、物理かなキー単独では
   ひらがなに戻せない」という副問題を実機で発見した。** 原因は
   `output/tsf_warmup_coord.rs::needs_f2_probe()`（GJI 戦略は常に
   `true`）により、GJI/TsfNative 環境では物理 `VK_DBE_HIRAGANA` (0xF2)
   KeyDown が **常に** Suppress される既存の設計（GJI cold-start warmup
   が独自に F2 を送るため、物理 F2 イベント自体は元々不要という前提）
   にあり、Shift+かなでカタカナに入れなかった間はこの副作用が無害
   だったが、v1 相当の修正はこれを露出させる。対処として、
   `PhysicalKeyDisposition::plan` の Allow 条件を緩めるのではなく、
   ADR-107（BUG-25）で実機検証済みの安全な注入経路
   `Output::send_gji_half_width_alnum_toggle(HalfWidthAlnumAction::Exit,
   ..)`（scan 付き `VK_DBE_HIRAGANA` 注入、Win/Alt 修飾キーガード、
   `effective_open()` ガード込み）を再利用して能動的に復元する方式を
   採用し、実機で成功を確認した（`ime_on=true && !shadow_toggled &&
   composition_warm` の条件でのみ発火、cold-start 進行中への誤発火は
   実機データ上ゼロ件）。
6. **BUG-52 非再発を実機で確認**: 上記の修正を有効化した状態で物理
   IME トグルキーを連打しても `vk=0xF1` は一度も観測されず、既存の
   保護は壊れていない。

この結果を受けて決定を v1 から v2 へ更新する（下記「決定（v2、確定）」）。
v1 の記録と、それを取り下げた経緯（B-1〜B-4, M-1〜M-5, SB-1, SB-2 の
指摘）は歴史的経緯として以下にそのまま残す。

v1（Shift 押下有無だけを弁別軸にする単純な条件追加）は、コード読解ベースの
「BUG-52 は Shift 無関係、BUG-116 は Shift 併用」という前提に依存していたが、
敵対的レビューでこの前提自体が実機ログで未検証であり、しかも BUG-52 の
記述（「なぜ 0xF1/0xF2 が交互に生成されるか未解明」）と論理的に緊張関係に
あることが判明した（下記「レビューで発見された論点」B-1）。加えて
`reinject()` が常に `wScan: 0` で送るため、たとえ Shift で弁別できても
実際に MS-IME/TSF に届くかどうか自体が別の未検証事項であること（B-2）、
scan 付き DBE キー注入が JIS かな直接入力への不可逆な固着ハザードを持ち
（BUG-15 追補7 / BUG-61）、その復旧手段（Alt+かな）を awase 自身が常時
swallow する（BUG-62）ため実装前に安全設計が必須であることも判明した
（B-1〜B-4, M-1〜M-5, SB-1, SB-2 の詳細は後述）。

これらを受けて、**「確定した修正」ではなく「実機で検証するための診断的
スパイク」を先に作る**方針に転換した。スパイクの実装・安全設計・実機での
確認手順・判定フロー（デシジョンツリー）は `diag/bug116-shift-katakana`
ブランチのコミット本文と本 ADR の「実機検証スパイク」節を参照。

## 問題

### 症状

JIS配列の「カタカナ ひらがな ローマ字」キー（scan 0x70）を Shift 併用で
押しても、GJI/MS-IME の変換モードがカタカナに切り替わらない。Windows の
一般的な IME 挙動（Shift+かな→カタカナ）が awase 使用中は発火しない。
詳細な再現ログ・症状は `docs/known-bugs.md` BUG-116 参照。**「アプリ/IME」
欄の `TsfNative`/`Imm32Unavailable` はコード読解由来の推定であり、報告者が
実際にどのアプリで再現したかは未確認**（B-3、下記）。

### 根本原因（の候補——確定はしていない）

`crates/awase-windows/src/runtime/transport.rs::PhysicalKeyDisposition::plan`
は、`ime_actuation_owned`（GJI/MS-IME への直接 actuation が有効）な文脈で、
`VK_DBE_ALPHANUMERIC`/`VK_DBE_KATAKANA`/`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR`
の KeyDown を **`shadow_toggled` の値に関わらず常に Suppress** する:

```rust
let is_dbe_mode_key_down = matches!(dbe_mode_key_policy, DbeModeKeyPolicy::Suppress)
    && matches!(event.vk_code,
        crate::vk::VK_DBE_ALPHANUMERIC | crate::vk::VK_DBE_KATAKANA
            | crate::vk::VK_DBE_SBCSCHAR | crate::vk::VK_DBE_DBCSCHAR)
    && event.event_type == KeyEventType::KeyDown;
ime_actuation_owned && (shadow_toggled || is_dbe_mode_key_down || matches!(event.event_type, KeyEventType::KeyUp))
```

この無条件 Suppress は BUG-52（2026-08-05, `bdf4a139`→`9a02ce6b`）で
導入された。BUG-52 の実際の repro は次の通り（`docs/known-bugs.md` BUG-52節）:

> NICOLA の物理「IME ON」キー（scan 0x70、awase の engine トグル用に
> 割り当て）を **Shift なしで** 連打すると、IME が既に ON の状態で
> `VK_DBE_HIRAGANA` の代わりに `VK_DBE_KATAKANA` が Windows のキーボード
> レイアウト変換層によって生成され、それが素通しされて実 IME が勝手に
> カタカナへ切り替わる。

一見すると、BUG-52 が潰したかったのは「Shift を押していないのに
`VK_DBE_KATAKANA` が生成されてしまう誤爆」であり、BUG-116 は「Shift 併用の
正当な要求」だから両者は `event.modifier_snapshot.shift` で区別できる
——というのが v1 の仮説だった。**しかしこの仮説は以下の理由で確定していない
（B-1）:**

- BUG-52 の実機ログ（`[hook] IME-mode ...` 書式）には修飾キー状態が一切
  出力されておらず、「Shift なし」は語り（連打していた、という記述）からの
  推測であって、ログで直接確認された事実ではない。
- BUG-52 自身が「なぜこの物理キーで 0xF1/0xF2 が交互に生成されるか…未解明」
  と明記している。もし Windows のレイアウト変換層が Shift 併用時に 0xF1 を
  生成するという BUG-116 側の主張が正しいなら、**BUG-52 で観測された 0xF1
  も Shift 由来だった可能性**があり、その場合 `!shift` ゲートは BUG-52 が
  実際に起きたケースでこそガードを外す＝BUG-52 が再発する。
- BUG-52 追補1 は同じ scan 0x70 から **0xF0（ALPHANUMERIC）** も観測して
  おり、「Shift の有無」だけでは説明できない別の状態変数が効いている
  可能性を示唆する。

## 決定（v2、確定）

実機検証（上記ステータス参照）を踏まえ、以下の2点を実装する。

### 決定1: `VK_DBE_KATAKANA` 単体・Shift 押下時のみ Allow（scan は変更しない）

`transport.rs::PhysicalKeyDisposition::plan` の `is_dbe_mode_key_down` に
`&& !event.modifier_snapshot.shift` を追加する形は v1 と同じだが、
スパイクでの実機確認により以下が確定した:

- 対象は **`VK_DBE_KATAKANA` (0xF1) 単体のみ**（M-1）。0xF0/0xF3/0xF4 は
  対象外のまま。
- **scan は変更しない**（`reinject()` の `wScan: 0` のままで動作確認済み、
  B-2 は該当なし）。SB-1 のハザード（scan 付き注入によるかな入力ロック）
  はこの決定では一切踏まない。
- `half_width_alnum_toggle_active`（M-3）が true の間はこの Allow を
  適用しない——`kp_stage_shadow_ime_toggle` の no-op 分岐が既に
  `kp_restore_kana_from_half_width` へ委譲しており、そちらが独自に
  scan 付き `VK_DBE_HIRAGANA` を注入する経路と、素通しした物理 0xF1 が
  競合するのを避ける。**実装時に発覚した重大な訂正（本実装レビュー
  B-2）**: このフラグを `plan()` 呼び出し時点でライブに読むと、
  `kp_stage_shadow_ime_toggle` が同じイベント処理内で
  `kp_restore_kana_from_half_width` へ委譲した場合に同期的に `false` へ
  落ちてしまい、ガードが常にすり抜ける。`kp_stage_shadow_ime_toggle`
  実行**前**のスナップショットを渡す必要がある（`key_pipeline.rs` に
  `half_width_alnum_toggle_before` として実装）。
- **`is_configured_thumb_key`（本実装レビュー B-1、新規追加）**: この VK
  が NICOLA 親指キーとして設定されている場合、この KeyDown は IME
  モードキーではなく同時打鍵入力である。BUG-115 が追加した
  `mode_key_delegate_owns_shadow_toggle` 機構（ひらがな/カタカナキーを
  親指キーに設定し `hiragana_delegate_to_open_axis`/
  `katakana_delegate_to_open_axis` が armed の構成）では、この構成下で
  `delegate_owned=true` となり `shadow_toggled` が常に false を返す
  ため、ガードが無いと Shift（小指シフト面）併用の打鍵ごとに実 IME を
  カタカナへ飛ばしてしまう。`gji_charset_autodetect::is_configured_thumb_key`
  を再利用する。

引数の増加は `DbeModeKeyContext { policy, half_width_alnum_toggle_active,
is_configured_thumb_key }` という struct + `impl Into<DbeModeKeyContext>`
（`DbeModeKeyPolicy` からの `From` 実装込み）で吸収し、`plan()` の引数
個数を増やさず、既存の回帰テスト（`DbeModeKeyPolicy` を直接渡す24箇所）を
無改造のまま通す。

### 決定2: GJI 環境で物理かなキー単独によるひらがな復元を能動的に注入する

`needs_f2_probe()` が `true`（GJI 戦略）の場合、物理 `VK_DBE_HIRAGANA`
KeyDown は `is_tsf_mode` の下で常に Suppress される（既存仕様）。決定1で
カタカナへ入れるようになった以上、戻す経路も対で必要（M-6、実機確認済み）。

`key_pipeline.rs` で、物理 `VK_DBE_HIRAGANA` KeyDown が
`shift=false && physical=Suppress` で到達し、かつ

```
effective_open() && !shadow_toggled && is_composition_warm()
```

が真であれば（実機データでは `open` 条件と `warm` 条件が常に一致して
いたが、cold-start 進行中への誤発火をより確実に避けるため保守的な
`warm` 条件を採用する——スパイクの2候補 `WhenOpen`/`WhenWarm` のうち
`WhenWarm` を選択）、`Output::send_gji_half_width_alnum_toggle(
HalfWidthAlnumAction::Exit, effective_open(), false)`
（ADR-107/BUG-25 で実機検証済みの安全な注入経路、Win/Alt 修飾キー
ガード・`effective_open()` ガード込み）を呼んでひらがなへの復元を
能動的に送る。`ActiveImeKind::GoogleJapaneseInput` にスコープを限定する
（**訂正**: 「MS-IME は `needs_f2_probe()=false` のため発生しない」は
不正確——`ActiveImeKind` は GJI 検出/非検出の2値であり、GJI 未検出時は
「MS-IME と推定」されるだけで `MicrosoftIme` への切替は
`set_active_ime_kind` 経由の実検出後にしか起きない。正確には「MS-IME
**検出済みの場合**は `needs_f2_probe()=false` により物理 F2 が元々
Allow されこの副問題自体が発生しない」。ATOK 等の第三 IME や種別未確定
時の扱いはこのスコープ限定により未検証のまま残る）。

**呼び出し位置の制約**: `kp_stage_execute` より**前**で評価する必要が
ある。`kp_stage_execute` は `composition_native_f2_down()` を呼び、
`is_tsf_mode=true` の場合は warm/cold に関わらず `MarkCold` するため
（本実装レビューで発覚）、後に置くと `is_composition_warm()` が常に
false になり決定2 が永久に発火しない。`plan()` 呼び出し直後（journal
記録より前）が唯一の正解。

**追加のガード（本実装レビューで発見、決定2 に実装済み）**:

- `!event.injected`（BUG-14 の規律）: `plan()` の F2 分岐は
  `event.injected` チェックより前にあるため、外部プロセス由来の注入
  （MWB / MS-IME 自身の SendInput）も `physical == Suppress` に到達
  しうる。ユーザーの物理操作でない入力を actuation の根拠にしない。
- `!is_configured_thumb_key`（決定1 と同じ BUG-115 対策。**本実装
  レビューで判明した本命の衝突**）: ひらがなキーを親指キーに設定し
  `hiragana_delegate_to_open_axis` が armed の構成（BUG-115 が救おうと
  した当のシナリオ）では、`delegate_owned=true` → `shadow_toggled=false`
  かつ `physical=Suppress`（GJI 戦略）かつ `effective_open()=true`
  （delegate の発火前提）かつ `is_composition_warm()=true`（連続打鍵中）
  が揃うため、このガードが無いと **NICOLA の親指キーを押すたびに**
  `send_gji_half_width_alnum_toggle(Exit)` が発火し、GJI へ
  `VK_DBE_HIRAGANA` を SendInput し続けることになる。BUG-115 で出荷
  したばかりの機能に対する明確な回帰であり、決定1・決定2 の**両方**に
  このガードを置く（`.claude/rules/fix-requires-evidence.md` の
  「IME actuation 合流点」が言う「1箇所直しただけでは再発する領域」の
  典型）。
- repeat latch（M-2 対策）: `WH_KEYBOARD_LL` は auto-repeat の KeyDown も
  配送するため、かなキーを押しっぱなしにすると KeyDown ごとに
  SendInput バッチが飛ぶ。`GateStore::kana_mode_restore_key_down` を
  `half_width_alnum_toggle_active` と同型の latch として追加し、対応する
  KeyUp が来るまで再発火しないようにする。
- `conv_mutation_allowed`（M-4 対策）: `send_gji_half_width_alnum_toggle`
  自体はこのチェックを持たない（既存呼び出し元が全て engine 有効文脈
  からしか来ないため元々穴になっていなかった）。決定2 は
  `effective_open()` が真なら awase engine が user-disabled（無変換
  3連打等）でも発火しうるため、`ConvModeAuthority::UserOwned`＝
  「conv mode に一切触らない」契約に反しないよう、呼び出し前に
  `self.platform.output.conv_mutation_allowed.get()` を確認する。
- `read_kana_lock()` によるアボート（B-3 の部分的緩和）: この注入は
  scan 付き `VK_DBE_HIRAGANA` を使うため、BUG-15 追補7/BUG-61 の
  「JIS かな固着ハザード」を実際に踏む経路である（下記「SB-1 の記述
  訂正」参照）。唯一のゲートが `effective_open()`（belief であり
  保証ではない）だけでは不十分なため、OS のかな入力ロックが既に
  On になっていないかを追加確認し、On なら注入を見送る。

### 決定に含めないもの

- **M-2（`shadow_toggled=true` 経路）**: IME OFF からの Shift+かな、
  または belief 乖離時は本 ADR のスコープ外のまま。実害が報告された
  場合に別途対応する。
- **M-4（`PassthroughQueue` の KeyUp 非対称）**: 決定1のスコープ
  （0xF1 の KeyDown のみ Allow、KeyUp は既存どおり Suppress）により
  `deferred_vks` に 0xF1 が入りうるようになる。実装時に
  `transport.rs:47-60` の doc コメントを「0xF1 は例外」と明記して
  更新し、実害（残留による誤動作）が無いことをコードレビューで
  確認する。挙動そのものは変更しない（別途 KeyUp も Allow にする
  等の対称化は本 ADR のスコープ外）。
- **M-5**: Shift の多重の意味（小指シフト面・shift-conv-guard）は
  コード上は独立した軸として成立するため変更不要。コメントで
  結合関係を記録するのみ。
- **`AppImeProfile::Standard`（ImmCross）**: 引き続きスコープ外
  （`feedback_immcross_owns_kanji` の設計原則、別 BUG/ADR）。

## 本実装のレビューで発見した追加の論点（v3、Opus 2体2ラウンド）

develop の最新先端（BUG-115 マージ後）で本実装を設計する際、architect/
premortem 役それぞれ独立に発見した Blocker 3件は決定1・決定2 の本文に
直接反映済み（上記の「本実装レビュー」注記）。ここでは Major/Minor の
対応状況を一覧にする（新ラウンドの通し番号は下記「v1 時点の記録」の
番号と衝突するため区別して読むこと）。

**SB-1 の記述訂正（重要）**: 上記ステータス節・決定1 節は「SB-1 のscan
ハザードは一切踏まない」と書いていたが、これは**決定1 にのみ**当てはまる。
**決定2 は scan 付き `VK_DBE_HIRAGANA` 注入を使うため、SB-1/BUG-61 が
警告する JIS かな固着ハザードを実際に踏む。** さらにこの注入パターンは
ADR-100 決定2（2026-08-22）・ADR-098 F4・BUG-50 が意図的に置き換え/
受容管理していたもの（eager warmup の送信キーを `VK_DBE_HIRAGANA` から
`VK_IME_ON` へ変更した理由そのもの）を、別経路から復活させる。唯一の
安全策は上記の `read_kana_lock()` アボートと `effective_open()` ガード
（belief であり保証ではない）であり、`WhenWarm` 条件による発火頻度の
抑制と合わせて残存リスクとして受容する。

| # | 論点 | 対応 |
|---|---|---|
| 実装M-1 | 決定2 と `composition_native_f2_down`（`kp_stage_execute`）が1打鍵で2発（scan付きF2 → cold化 → `VK_IME_ON`）送る二重 actuation。真因は「代替送信のキー選択」自体を直す方が筋が良い可能性 | **対応せず、既知のトレードオフとして記録。** 代替送信のキー選択変更は ADR-100 の対象領域を再設計することになり本 ADR のスコープを超える。両送信とも冪等（`VK_IME_ON` は反復無害、scan付きF2は状態を確定的に設定）なため実害は未確認 |
| 実装M-2 | auto-repeat KeyDown での重複発火 | **対応済み**: `kana_mode_restore_key_down` latch |
| 実装M-3 | Shift stuck が出荷後リスクに格上げ、対策は低コスト | **対応せず、既存の B-4 記録のまま残存**（下記「決定に含めないもの」）。`is_held_fresh` 型の鮮度ガード追加は新規 tuning 定数の実測が必要で本 PR のスコープ外 |
| 実装M-4 | `conv_mutation_allowed`/`ConvModeAuthority::UserOwned` を素通りする | **対応済み**: 呼び出し前に `conv_mutation_allowed.get()` を確認 |
| 実装M-5 | GJI cold 時、決定2 が無言で no-op になる新症状（idle-conv-check とは別の非決定性） | **ログ出力のみ対応**（`[kana-mode-restore] 見送り`）。既知の限界として known-bugs.md に記録 |
| 実装M-6 | 「MS-IME なら発生しない」の根拠記述が不正確 | **対応済み**: 決定2 本文を訂正 |
| 実装M-7 | 決定2 の見送りログが `[shift-conv-guard]` プレフィックスを流用し半角英数トグルのトリアージを誤誘導しうる | **対応済み**: `[kana-mode-restore]` という専用プレフィックスを使う |
| 実装M-8 | ADR/known-bugs.md が v1 のまま矛盾 | **対応済み**: 本更新で反映 |
| 実装m-1 | 決定2 の条件判定はテーブル駆動テスト化しづらい（`Runtime` 全体が要る） | **静的ガードで代替**: `tests/architecture_guard.rs` にトークン固定テストを追加（Linux で実行可能） |
| 実装m-2 | `deferred_vks` 残留（旧 M-4 と同一論点） | **対応済み**: doc コメント更新 |
| 実装m-3 | `plan()` の引数数 | **対応済み**: `DbeModeKeyContext` 導入 |
| 実装m-4 | `docs/known-bugs.md:3073`/`ime.rs:365` の ADR-107 M4 記述が「Shift 押下中は DBE キー自体が配送されない」と過度に一般化されており、BUG-116（scan=0x70/0xF1 の物理配送を実機確認済み）と矛盾して見える | **未対応（残存）**。ADR-107 M4 の実測対象は「awase 自身が注入する scan=0 の `VK_DBE_ALPHANUMERIC`」であり BUG-116 とは対象が異なるため矛盾はしないが、将来の誤読を防ぐ限定句の追記が望ましい |

### BUG-115 delegate 機構との衝突確認（architect による詳細分析）

`plan()`（配送判断）と NICOLA FSM のチョード処理は独立レイヤーであり
（`transport.rs` の doc どおり）、`kp_stage_execute` の1箇所でのみ交わる。
問題は「独立している」こと自体で、同じ物理キー押下が FSM 側では NICOLA
入力として、transport 側では IME モードキーとして二重に意味を持ちうる。

- **Phase 2**（非親指キーへの `shadow_action` オーバーライド）は自動的に
  安全: `resolve_mode_key_shadow_override_for_event` は親指キーに対して
  `None` を返すため Phase 2/3 は排他。
- **決定1 × delegate**: `delegate_owned=true` のとき `shadow_toggled` は
  常に false になるため、決定1 の Allow 条件だけが残る。「カタカナキーを
  親指キーに設定 + Shift 併用」で FSM が open 軸 delegate として処理する
  のと同時に物理 0xF1 が実 IME へ抜ける。実害頻度は低い（Shift 併用時の
  み）が構造的には BUG-46 型の二重 actuation。
- **決定2 × delegate（本命）**: ひらがなキーを親指キーに設定し
  `hiragana_delegate_to_open_axis` が armed の場合、`delegate_owned=true`
  → `shadow_toggled=false`、`physical=Suppress`（GJI 戦略）、
  `effective_open()=true`（delegate の発火前提）、`is_composition_warm()=true`
  （連続打鍵中）が揃い、**NICOLA の親指キーを押すたびに** 決定2 が発火する。
  `is_configured_thumb_key` ガードが必須。

## レビューで発見された論点（Opus 2体、4ラウンド、v1 時点の記録）

以下は実機検証前（v1 決定を取り下げた時点）の記録。上記「ステータス」で
実機確認により解消/該当なしと判明したものも含め、歴史的経緯として残す。

### Blocker

- **B-1**: 上記「根本原因（の候補）」参照。Shift の押下有無が本当に
  弁別軸になるかが実機で未検証。
- **B-2**: `crate::RawKeyEventExt::reinject()`（`lib.rs`）は常に `wScan: 0`
  で `SendInput` する。`key_pipeline.rs` には「`scan=0x0` の
  `send_ime_mode_key` では MS-IME (TSF) がモードキーとして処理しない」
  という2026-07-07の実機所見が既に記録されており、`plan()` が Allow を
  返しても実際に IME 側で受理されない可能性がある。しかもフック
  (`hook.rs::hook_proc`) は通常時つねに `LRESULT(1)` で元イベントを消費し、
  `CallNextHookEx` で OS に直接届く経路は存在しない（すべて
  `enqueue_reinject` 経由の `SendInput` に統一されている——この誤解を
  生んでいた `executor.rs::enqueue_reinject` の doc コメントは本 PR で
  別途訂正済み）。つまり「Allow にすれば OS に届く」という前提自体が
  scan 情報を失う経路を経由する。
- **B-3**: BUG-116 報告者が実際に使っていたアプリが `AppImeProfile::Standard`
  （ImmCross）だった場合、`transport.rs` は `dbe_mode_key_policy` も
  `modifier_snapshot` も一切見ずに無条件 Suppress するため、本 ADR の
  修正はユーザーの症状を一切改善しない。しかも Standard は 2026-05-28
  （`0e364eaa`）以降さらに長く壊れている。
- **B-4**: `modifier_snapshot.shift` の出所 `PHYSICAL_KEY_STATE` は左右 OR
  合成のため、片側の KeyUp 消失で恒久的に `true` に stuck した実績がある
  （2026-07-09 実機、BUG-48/BUG-62 と同型）。Win/Alt にある `is_held_fresh`
  相当の鮮度ガードが Shift には無いため、stuck 一回で `!shift` ゲートが
  セッション中ずっと無効化され BUG-52 が全面復活しうる。
- **SB-1**: JIS かな固着ハザード（BUG-15 追補7）は `always-scan` 相当の
  モードだけでなく、Shift 併用限定の scan 付与でも同じ経路を踏む。危険の
  実体は「always/shift の別」ではなく「scan 付き DBE キーが実 IME OFF の
  瞬間に着弾すること」であり、BUG-116 の対象状況（belief と実 IME が
  乖離しうる）はこの着弾確率を平常時より高める。scan 付与は
  `VK_DBE_KATAKANA` 単体に限定し、実 IME ON の前提ゲートと `read_kana_lock()`
  による自動 abort が必須（0xF0 は物理 CapsLock 位置で、追補7 が実IME OFF
  文脈への着弾で CapsLock をトグルすると実機確認済みのため対象外）。
- **SB-2**: JIS かな固着からの復旧に使う Alt+かな（物理的には
  `VK_DBE_ROMAN`/`VK_DBE_NOROMAN`）は、`hook.rs:1066-1068` が
  `CACHED_SWALLOW_ALT_KANA_MODE_SWITCH` のみで判定しており、Alt 押下の
  有無（`alt_key_held()`）はログ出力にしか使われず判定に入らない——つまり
  **既定では Alt 押下の有無に関わらず常時 swallow** する（BUG-62）。
  awase が稼働中は復旧操作そのものが物理的に効かない。

  **推奨する実機手順（事前武装、premortem 提案）**: scan モード
  （`AWASE_BUG116_SCAN=real`）に入る**前**に、設定
  `GeneralConfig::swallow_alt_kana_input_method_switch`（`src/config.rs:384`、
  既定 `true`）を `false` にして設定リロード（`runtime/mod.rs:1486-1487`
  経由でライブ反映、または `awase-settings` のトグル UI）しておく。こうする
  と万一 JIS かな固着を踏んでも、Alt+かなのワンアクションだけで復旧できる
  ——「awase を Exit → 復旧 → 再起動」という確実だが重い手順（`abort_scan()`
  のラッチや各種 belief が初期化され、そのフェーズを測り直しになる）は
  **フォールバック**（事前武装を忘れた場合・設定リロードが効かなかった
  場合）として温存する。事前武装中はテスター自身の誤爆 Alt+かなも素通し
  されるが、監視下の短時間であり、かつ「ハザードが実際に起きるか」自体が
  DT-4 で観測したい事象なので実験目的と整合する。

### Major（要旨のみ、詳細は `diag/bug116-shift-katakana` のコミット本文参照）

- **M-1**: 4つの `VK_DBE_*` 全部に `!shift` を適用する根拠はなく、
  Windows 標準トリガーの根拠があるのは `VK_DBE_KATAKANA` (0xF1) だけ。
  スコープを 0xF1 単独に絞る（スパイクではこの通り実装済み）。
- **M-2**: `shadow_toggled=true`（IME OFF からの Shift+かな、または belief
  乖離時）は修正後も直らないまま残る。
- **M-3**: 半角英数持続トグル ON 中に Shift+かなを押すと、awase 自身が
  scan 付き `VK_DBE_HIRAGANA` を注入する経路（`kp_restore_kana_from_half_width`）
  と素通しされた物理 0xF1 が競合し、二重 actuation になりうる。
- **M-4**: KeyDown だけ Allow・KeyUp は Suppress のままだと、
  `PassthroughQueue` の「KANJI 系 KeyUp は常に Suppress されるため
  `deferred_vks` は inert」という既存 doc の前提が崩れる。
- **M-5**: Shift は `should_use_shift_plane`（NICOLA 小指シフト面）や
  shift-conv-guard（半角英数トグル、BUG-15/25）など、awase 内で既に
  複数の意味を持つ軸。「独立した軸」という主張はコード上は成立するが、
  この結合関係を記録しておく必要がある。
- **SM-1**: `tsf::output::make_scan_key_input` の `MapVirtualKeyW` 逆引きは
  レイアウト次第で 0 を返しうる。scan は `event.scan_code`（フックが
  実際に受け取った物理 scan）を使う（スパイクではこの通り実装済み）。
- **SM-2**: 環境変数は `OnceLock` で起動時1回固定し、`setx` は使わない
  （スパイクではこの通り実装済み）。
- **SM-3**: IME 切替直後の1本目のデータは切替自体の副作用（BUG-37 の
  「GJI→MS-IME 切替が真因では」仮説）を含みうるため、捨て打ちしてから
  計測する。

### デシジョンツリーの欠落（DT-1〜DT-4）

実機データが「判定不能」（scan が実際には 0 で送られていた等）なのか
「判定可能で結果が出た」のかを区別する分岐、Phase 1（スパイク無効相当）
の時点で既に症状が再現した場合に報告者環境との差分を先に潰す分岐、
観測された VK/scan が `VK_DBE_KATAKANA`/`0x70` 以外だった場合に根本原因
記述自体を疑う分岐、実機でハザードが実際に発現した場合の中断・復旧・
記録・モード封印の手順、をデシジョンツリーに含める（詳細は
`diag/bug116-shift-katakana` のコミット本文）。

## 実機検証スパイク

ブランチ `diag/bug116-shift-katakana`（`develop` の先端から専用 worktree
で作成、develop へマージしない）。2コミット構成:

1. `bug116_spike.rs` の新設（モード定義・かなロック abort ラッチ、
   挙動変更なし）
2. `transport.rs`/`platform.rs`/`key_pipeline.rs`/`lib.rs` への配線
   （唯一の挙動変更、環境変数が既定値なら本番と完全に同じ挙動）

環境変数2本（`AWASE_BUG116_ALLOW`: off/shift/always、`AWASE_BUG116_SCAN`:
zero/real）を直交させ、1ビルドのまま複数モードを実機で切り替えて検証
できる。scan 付与は `VK_DBE_KATAKANA` 単体・実 IME ON 判定・かな入力
ロック検出による自動 abort の3段ゲート付き（上記 SB-1/SB-2 反映済み）。
診断ログ（`[bug116] ...`）で判定入力・配送経路の全てを1行に集約し、
1回の実機セッションで前提の真偽・配送経路・安全上のハザード発現有無を
まとめて確認できるよう設計した。詳細な実機チェックリストとデシジョン
ツリーはコミット本文および Opus レビューのやり取り（このセッションの
`opus-architect-adr137`/`opus-premortem-adr137` エージェントとの
往復）を参照——将来的にこの ADR 自体に転記するか、`docs/experiments.md`
のエントリから参照する形に整理する。

## 検討した代替案

### 案B: `dbe_mode_key_policy` の既定値を `Passthrough` に変える

却下。BUG-52 の誤爆（Shift なし連打でのカタカナ化）が全面的に復活する。

### 案C: `shadow_action` の事前分類（`ime_relevance.shadow_action`）側で
Shift 押下時は「IME トグル関連キーではない」と再分類する

保留。`ime_relevance` の分類は `should_use_shift_plane` 等、他のロジックとも
共有されている可能性があり影響範囲の洗い出しが必要。スパイクの結果、
弁別軸として `shift` が成立しないと判明した場合の次善案として温存する。

## 未解決事項（v3 実装後に残るもの）

- **報告者本人の環境確認**: 実機検証はテスト環境（TsfNative+GJI）で
  行ったもので、BUG-116 の報告者本人のアプリ/IME が同一かは未確認の
  まま。`Standard`/ImmCross だった場合は本 ADR は無関係（上記「決定に
  含めないもの」参照）。
- **M-2（`shadow_toggled=true` 経路）・M-4（KeyUp 非対称）・B-4/実装M-3
  （Shift stuck 鮮度ガード）**は実装のスコープ外として残す（上記
  「決定に含めないもの」参照）。実害が報告されたら別途対応する。
- **実装M-1（決定2 と `composition_native_f2_down` の二重 actuation・
  代替送信キー選択の再検討）**: 対応せず既知のトレードオフとして記録
  （上記「本実装のレビューで発見した追加の論点」参照）。
- **実装M-5（GJI cold 時の無言 no-op）**: ログ出力のみで対応、
  「たまに効かない」という新しい非決定的症状として known-bugs.md に
  記録済み。
- **MS-IME 環境での検証は未実施**: 決定2は `ActiveImeKind::
  GoogleJapaneseInput` にスコープを限定している。「MS-IME 検出済みの
  場合は `needs_f2_probe()=false` によりこの副問題自体が発生しない」が
  正確な言い方（実装M-6 で訂正）。MS-IME 実機での回帰確認が望ましい。
- **実装m-4（ADR-107 M4 記述の限定句追記）**: `docs/known-bugs.md:3073`/
  `crates/awase-windows/src/ime.rs:365` に「（awase が注入する scan=0 の
  DBE キーについて）」等の限定句を追記し、将来のセッションが BUG-116 と
  矛盾すると誤読するのを防ぐ。
- **回帰テスト**: `crates/awase-windows/src/runtime/transport.rs::
  plan_tests` に決定1 のケース（Shift+0xF1 Allow、hw トグル中/親指キー
  設定時は不適用、KeyUp 非対称、ImmCross スコープ外、Passthrough policy
  でも成立、0xF1 以外は緩めない）を実装済み。決定2 は `Runtime` 全体を
  要するためユニットテスト化できず、`tests/architecture_guard.rs` の
  静的トークン固定テストで代替（Linux で実行可能、`fix-requires-evidence.md`
  の要件充足）。`plan_tests` 自体は `runtime/mod.rs` の `#[cfg(windows)]`
  により Linux ではコンパイルされず、実行は windows-build CI 待ち。
