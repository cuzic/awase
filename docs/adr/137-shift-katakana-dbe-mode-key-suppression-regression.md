# ADR-137: `VK_DBE_*` KeyDown 無条件 Suppress（BUG-52対策）が Shift+かな→カタカナ変換を巻き添えで殺している（BUG-116）

## ステータス

**前提未検証・実機検証スパイクを作成済み（`diag/bug116-shift-katakana` ブランチ、
develop 非マージ）。設計は Opus 2体（architect/premortem 役）による4ラウンドの
敵対的レビューで収束したが、収束したのは「まだ何も確定していない、実機で
検証する必要がある」という結論であり、下記「決定（v1、取り下げ）」に書いた
最初の修正案そのものは v1 のレビューで根拠不十分と判定され取り下げられている。**

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

## 決定（v1、取り下げ）

`is_dbe_mode_key_down` の判定に `!event.modifier_snapshot.shift` を追加し、
Shift 押下中はこの追加 Suppress 条件を無効化する、という単純な1行修正を
最初に提案したが、上記 B-1 に加え B-2〜B-4・M-1〜M-5（後述）の指摘を受けて
**取り下げた**。実装前に実機データが必要という結論のみが確定している。

## レビューで発見された論点（Opus 2体、4ラウンド）

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
  `VK_DBE_ROMAN`/`VK_DBE_NOROMAN`）は、`hook.rs` が Alt 押下の有無に
  関わらず**既定で常時 swallow** する（BUG-62）。つまり **awase が稼働中は
  復旧操作そのものが物理的に効かない**。実機検証時は「awase を Exit で
  終了 → Alt+かなで復旧 → 確認後に再起動」の手順を必ず先に把握しておく。

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

## 未解決事項

- **実機検証未実施。** 上記スパイクを実機（報告者の実アプリ含む複数
  プロファイル）で走らせ、デシジョンツリーに従って ADR-137 を
  確定/破棄/別軸へ振り分ける。
- **`AppImeProfile::Standard`（ImmCross）での Shift+かな→カタカナ欠落**は
  本 ADR のスコープ外。報告者のアプリが Standard だった場合は別 BUG/ADR
  （`feedback_immcross_owns_kanji` の見直し）を起票する。
- **回帰テストは判定フローで方針が確定してから書く**（未検証の前提を
  先にテストへ固定すると、前提が誤りだった場合にテストごと書き直しに
  なるため）。`plan_tests` は `runtime/mod.rs` の `#[cfg(windows)]` により
  Linux ではコンパイルされず、実行は windows-build CI 待ち（本スパイク
  ブランチは develop へマージしないため CI 対象外、確定後の本実装で
  改めて windows-build を通す）。
