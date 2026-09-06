# ADR-138: IME probe/actuation 検証用ウィットネスアプリは Opus 敵対的レビューで却下、発信源タグ計装+最小スパイクへ縮小

## ステータス

**保留（2026-09-05）。** Opus 敵対的レビュー2体（feasibility 担当・methodology 担当）
実施、両者独立に収束。フルスコープのウィットネスアプリ案は却下。決定2（発信源タグ
計装）・決定3（最小スパイク）を次に着手すべき案として記録するが、本 ADR 作成時点
では未実装（コード変更なし）。

## Context

ユーザー依頼:「特殊な検証用のアプリケーションを作って、不要な probe や actuation が
発火していないかのログを残せないか」。加えて「まず詳細な ADR を作り、Opus 2体で
敵対的レビュー」「過去の不具合を振り返り、十分な検証計画を立てた上で、その検証が
できるアプリを設計する」こと。

直接の動機は [ADR-136](136-duplicate-immcross-probe-on-focus-change.md)。
「フォーカス変更時に `read_ime_state_full_async()` が同一イベントに対し2回発行
されている＝無駄な重複」という仮説は、Opus 敵対的レビューで**反証・却下**された。
反証理由は静的コード読解では見えない実行時条件（confidence の High/Medium、
`SkipTyping` による no-op 化、epoch 照合の非対称）だった。

ここから「独立プロセスが OS レベルのグランドトゥルースを記録するウィットネス
アプリを作れば、この種の問いを実測で決着できるのでは」という設計を立て、詳細調査
の上で Opus 2体（feasibility 担当・methodology 担当）に敵対的レビューさせた。

**結論: フルスコープのウィットネスアプリ案は、2体のレビューが独立に収束する形で
棄却された。** 本 ADR は当初案を採用ではなく**却下**として記録し、代わりに実際に
価値のある縮小版を決定する。

## 検討した当初案（却下）

Windows 標準の `EDIT` コントロールを複数ペイン（`AppKind::Win32`/`TsfNative`/`Uwp`
に対応するクラス名）持つ新規クレート `awase-witness` を作り、awase が使う唯一の
クロスプロセス IME 制御チョークポイント（`imm.rs::send_ime_control`、
`WM_IME_CONTROL`）を自分の既定 IME ウィンドウでサブクラス化して記録、`dwExtraInfo`
署名（`INJECTED_MARKER`/`TSF_MARKER`/`IME_KANJI_MARKER`）で awase 由来の注入
キーを識別、シナリオ自動化（フォーカス往復・idle 待機・合成入力）付きで JSONL
ログを吐き、awase 側の journal と相関させて「不要な probe/actuation」を検出する
——という設計だった。

## Opus 敵対的レビューの結論（却下理由）

### 却下理由1（最も致命的、feasibility 担当レビュー）: ADR-136 の問いはウィットネスで原理的に決着不能

経路A（`ObserverPoll`）と経路B（`ImmCrossProbe`）は**どちらも同じ
`read_ime_state_full`（`ime.rs:655`）を呼び、同一の `IMC_GETOPENSTATUS`
(`ime.rs:549`) + `IMC_GETCONVERSIONMODE` (`ime.rs:560`) を同じ 50ms タイムアウトで
同一の IME ウィンドウへ送信する**。外部観測者には区別不能な同一メッセージ対が
見えるだけで、判定に必要な confidence（High/Medium）・`SkipTyping` による消費
有無・epoch 照合は**すべて内部情報**。当初案が「最初の実証課題」に据えた問いその
ものが、このアプローチでは原理的に答えが出ない。

### 却下理由2（設計の中核が破綻、feasibility 担当レビュー）: `AppKind` ではなく `AppImeProfile` が実挙動を決める

`AppKind`（`focus/class_names.rs::detect_app_kind`、前置マッチ）はクラス名で
自由に選べるが、実際の IME 制御戦略・物理キー Suppress・open 読み取り可否を
決めるのは**別関数** `AppImeProfile::from_class_name`（完全一致リスト、
`class_names.rs:19-35,51-60`）。衝突回避のため独自クラス名（`Chrome_AwaseWitness`
等）を名乗ると `AppImeProfile::Standard`——**Notepad と同じ扱い**になり、測りたい
GjiDirect/MsImeDirect/KanjiToggle 戦略にも物理キー Suppress 経路にも一度も入ら
ない。「衝突回避」という安全策が、そのまま「対象の分岐に一切到達しない」ことを
意味していた。

さらに `focus/uia.rs::resolve_app_kind` が UIA 結果着弾後に AppKind を非同期に
上書きするため、素の Win32 ウィンドウはクラス名詐称後も `AppKind::Win32` へ
戻される。

### 却下理由3: 内部計装は既に相当配線済みで、ウィットネスは下位互換になる

`state/event_origin.rs` の `EventOrigin`/`EventSource::{Physical, Injected,
SelfActuated}` は ADR-082 で「非同期系 journal entry は必須フィールドとして
持つ」と既に決定済み。`state/ime_actuation.rs::ActuationRecord`
（origin/epoch/target/policy/attempts/action）が `journal.rs::JournalEntry::
ImeActuation` として全 actuation で記録される。`tests/drift_correction_replay.rs`
は BUG-43 の無限再送有界化を**実機ログを fixture 化し CI で毎回**検証済み。
ウィットネスが出せるのは「実機で1回走らせたときの JSONL」であり、**CI で鳴らない
＝既存資産の下位互換**。

### 却下理由4: `send_ime_control` は「全 IME read/write」の合流点ではない

`imm.rs::send_ime_control`（`WM_IME_CONTROL`）は `ImmCrossProcessStrategy` の
1経路に過ぎない。以下はメッセージにも `WH_KEYBOARD_LL` にも一切現れない:

- `ImmNotifyIME(NI_COMPOSITIONSTR, CPS_CANCEL)`（`runtime/mod.rs:1871-1913`、
  メッセージでも SendInput でもない）
- `GjiDirectStrategy`/`MsImeDirectStrategy`/`KanjiToggleStrategy` の `SendInput`
  は `WH_KEYBOARD_LL` では見えるが `WM_IME_CONTROL` には映らない
- `detect_kana_for_hwnd`/`check_tsf_composition_active`/
  `capture_composition_snapshot` 等の `ImmGetContext`/`ImmGetCompositionStringW`
  直読み（痕跡ゼロ）

→ ウィットネスは**肯定的検出専用**であり、「不要な actuation は無かった」という
**否定的結論を単独では出せない**。これを見落としたまま運用すると、最も恐れて
いた誤報（見えていないだけなのに「問題なし」と報告する）を生む。

### 却下理由5: このリポジトリで「恒久化した検証ツール」は一度も定着していない

`examples/*.rs` の実績: `two_imm32_windows_probe.rs`（185行）ほか6本中6本が
2〜3コミットで放棄（唯一の例外 `spike_langbar_input_mode.rs` は963行・14コミット）。
3ペイン×3クラス名＋`ITextStoreACP` 自前実装（前例より遥かに重い21メソッドの
COM インタフェース）＋シナリオランナーという規模のツールを、実際に誰が維持
するのかという問いに答えがない。

### 却下理由6: シナリオ自動化は10日前の教訓に正面から逆行する

`docs/experiments.md` エントリ22（2026-09-05）: 自動条件ローテーションが
「単体トグルでは起きない新しい失敗モード（エイリアシング）」を持ち込み、実機
ラウンドを2回無駄にした。同エントリはさらに「ユーザーの『奇数回目で必ず@が出る』
という言い換えが真因への最短経路だった」と総括している。自動シナリオランナーは
まさにこの「人間の言い換え」を系から除去する方向の投資であり、直近の教訓に反する。

### 副次的な技術的指摘（採用しない設計に対する記録として残す）

- `dwExtraInfo` マーカーはサブシステム単位であり、呼び出し点単位の一意性はない
  （`INJECTED_MARKER` は romaji/Unicode/BS/Escape/Ctrl chord 等を全部兼ねる）
- `ImmGetDefaultIMEWnd` のサブクラス化はタイムアウト予算内で走る観測者効果が
  あり、ロックフリー以外禁止という制約が必要
- `CoreTextEditContext` は素の Win32 デスクトップでは実質使えない
  （`CoreTextServicesManager::GetForCurrentView()` が CoreWindow 前提）
- バグった自前 `ITextStoreACP` 実装は、awase 由来と区別のつかない IME 異常を
  自ら生み出し、グランドトゥルース性を破壊しうる

## 決定

### 決定1: 新規クレート `awase-witness` は作らない

上記却下理由により、フルスコープのウィットネスアプリは造らない。

### 決定2: awase 側に発信源タグ計装を追加する（優先・単独で価値あり、未実装）

`imm.rs::send_ime_control` は呼び出し元識別子を持たないため、ここへの一括ログ
では ADR-136 の経路A/B を区別できない（両方 50ms タイムアウトで見分けが
つかない）。**各呼び出し元**（`observer/ime_observer.rs::poll_and_classify_ime`
系、`ime.rs::read_ime_state_full_async` 系）に、evidence 型・confidence・
`SkipTyping` 消費有無を journal に出す数十行の計装を追加する。これは新規決定
ではなく ADR-082 の `EventOrigin` 規律の**拡張**であり、実装コストは小さい。

これにより ADR-136 の問い、副産物の `disable_apps` 非対称、検証計画クラスBの
大半（belief 乖離・学習キャッシュ誤学習・probe 冗長性判定）が**外部ツール無しで**
決着可能になる見込み。

### 決定3: 外部観測ツールは「物理キー実配送確認」1問に絞った最小スパイクのみ（未実装）

内部ログでは原理的に測れない、外部観測が本当に効く問いは2つだけ:

1. **物理キーが実際にウィンドウへ届いたか**（`transport.rs::plan` の
   Suppress/Allow、BUG-52/116/46/90）
2. **awase が「送った」と信じたキー/メッセージが実際に届いていないか**
   （BUG-32/53）

このうち①を最初の実証課題に選ぶ（②は①の副産物として同時に観測できる）。
理由: `diag/bug116-shift-katakana` ブランチが今まさに必要としている問い
（`docs/experiments.md` エントリ23 の B-2 指摘: `reinject()` が常に `wScan:0` で
送るため Allow 判定でも実 IME に届いていない疑いが未解決）に直結し、ADR-136 と
異なり**外部観測だけで答えが出る**。

**形式**: `crates/awase-windows/examples/witness_probe.rs`、目安 ~200行。新規
クレートではなく既存スパイク流儀（`two_imm32_windows_probe.rs` 等と同じ
`#[cfg(windows)] mod xxx_probe { ... }` + Linux 側 no-op `fn main()`）を踏襲する。

**構成**:
- Win32 ペイン **1枚のみ**（素の `EDIT`、独自クラス名。2枚構成は今回の目的には
  構造的に効かない——後述）
- 既定 IME ウィンドウをサブクラス化 → `WM_IME_CONTROL` の wParam/lParam/時刻を
  ロックフリーでリングバッファに積み、stdout へ JSONL 出力
- 自スレッドに `WH_KEYBOARD_LL` を張り、`LLKHF_INJECTED` + `dwExtraInfo` 署名で
  到達キーを記録（物理キー到達＝マーカー無し到達、awase 注入＝マーカー付き到達）
- **シナリオランナーは持たない**（決定6 の教訓により人間が操作する）
- **セルフテストは別スレッド/別プロセスから撃つ**（同一スレッド `SendMessage`
  は awase が実際に使う `SendMessageTimeoutW` クロスプロセス経路と異なる可能性
  があり、テストは通るのに本番経路が見えないという BUG-113 診断コード事故
  （`docs/experiments.md` エントリ21）と同型の罠になる）
- 相関は既存 journal（`ActuationRecord.origin` / `TsfProbeStarted.source`、
  決定2 で追加するタグ）と手作業で突き合わせる。自動相関スクリプトは作らない

**欠測の誤読に関する注意（feasibility 担当レビューの追加指摘）**: `send_health.rs`
のサーキットブレーカは、同期 IMM32 呼び出しが 100ms 超過を2回連続で観測すると
2000ms のクールダウンに入り、**その間は同期サイト自体の発行を見送る**
（トリップ時のみ `[send-health]` warn ログ）。決定2/決定3 いずれの相関作業でも、
「ログに probe/actuation が記録されていない」を即座に「その経路は発火しなかった」
と解釈してはならない——`awase.log` の `[send-health]` 行を必ず突き合わせ、
サーキットブレーカによる見送りではないことを確認すること。

**2窓構成を採らない理由（methodology 担当レビューの指摘）**: 経路Bの前提条件
`process_changed == true` は同一プロセス内の2窓往復では立たない。この問いに
対する2窓構成は「メモ帳などの別プロセスとの往復」でしか意味を持たず、Phase 1
のスコープ（物理キー実配送確認）には不要。

### 決定4: プロファイル網羅は行わない（AppImeProfile 詐称もオーバーライドも今回は追加しない）

決定3 のペインは `AppImeProfile::Standard` のまま使う。物理キー Suppress の
挙動はプロファイルごとに変わりうる（BUG-116 の doc 自身が「repro app 未確認」と
注記）ため、将来 `TsfNative`/`Imm32Unavailable` での検証が要ると分かった時点で、
feasibility 担当レビューが指摘した4つの落とし穴（ホットパスのプロセス名解決
コスト、`input_relay_apps_snapshot` の唯一のロック例外の拡張、
`is_tsf_native_window` を profile 経由せず直接呼ぶ2箇所、exhaustive oracle
テストの拡張）を踏まえた config 駆動オーバーライドを別途検討する。**今回は
追加しない。**

### 決定5: `.claude/rules/fix-requires-evidence.md` の「第3の選択肢」という位置づけは採らない

このリポジトリは既に「実機ログ→fixture→CI」という変換路（`journal_replay.rs`/
`drift_correction_replay.rs`/`docs/journal-replay-guide.md`）を持つ。ウィット
ネス的スパイクの正しい位置づけは、(a) 回帰テストに転記する前の**一次観測
ツール**であって、(a)(b) と並ぶ独立の第3の選択肢ではない。

## 検証計画（過去バグからの逆算、決定3のスコープに合わせて再整理）

### 決定3 のスパイクで直接検証できるもの

| BUG | 検証内容 |
| --- | --- |
| BUG-52 | 物理 `VK_DBE_KATAKANA` KeyDown が Suppress されているか（到達しないことを確認） |
| BUG-116 | Shift+物理かなキーが Allow され実際に届くか（BUG-52 のリグレッション確認） |
| BUG-90 | 外部注入（helper プロセスからの `SendInput`）が Allow され、かつ awase が二重 actuation しないか |
| BUG-32/53 | 修飾キー stuck 時、期待される IME モードキーが本当に一度も届いていないか |

### 決定2（発信源タグ計装）で内部的に決着させるもの

ADR-136 の問い、`disable_apps` 非対称（副産物）、BUG-16/20/33/51/63（belief
乖離）、BUG-56/107/112（学習キャッシュ誤学習）、BUG-102（bootstrap fence
desync）。

### どちらでも決着しない（実アプリでしか再現しない、明記すべき限界）

BUG-113 の最終真因は **PSReadLine（Windows Terminal 内の PowerShell モジュール）
と GJI の相互作用**（`docs/experiments.md` エントリ21）。BUG-02（Chrome
cold-start ~326ms）、BUG-106（Teams/WebView2）、BUG-78（`mstsc`）も同様に
アプリ固有実装が原因で、合成ペインでは模倣できない。**この限界を明記し、期待値を
釣り上げない。**

## 実装時の手続き（着手する際の注意、リポジトリ規約）

- worktree を分ける（`.claude/rules/worktree-per-session.md`）。並行セッション
  との衝突を避ける
- `develop` から feature ブランチを切る（`.claude/rules/main-develop-branch-flow.md`）
- ADR 番号は実装直前に再確認（並行セッションの番号衝突実績あり）
- 決定2/決定3 の実装はそれぞれ `.claude/rules/fix-requires-evidence.md` の
  「IME belief」「物理キー配送判断」ファミリーに触れるため、実装時は回帰テスト
  か known-bugs.md 追記を伴わせる

## 関連

[ADR-136](136-duplicate-immcross-probe-on-focus-change.md)（本 ADR の直接の動機・
反証の教訓）、[ADR-082](082-journal-structured-replay-and-event-origin.md)（`EventOrigin` 必須化の
由来）、ADR-119（actuation 合流点の洗い出し）、
`.claude/rules/ime-belief-architecture.md`、`.claude/rules/fix-requires-evidence.md`、
`docs/experiments.md` エントリ21・22・23、`examples/two_imm32_windows_probe.rs`
（当初案が参照した前身）
