# ADR-196 T1: 学習窓への外部からの書き込みを直接観測する基盤を実装する

状態: **実装済み（PR #259、develop未マージ）。実機未検証（quiet window・生存確認・
残余リスク判定が実際にA'の崩れを検出できるかは実機検証が必要）。** TSF compartment
変更通知のCOM advise sink実装は見送り、`WM_IME_NOTIFY`（IMM32互換レイヤー経由）で
代替した（項目2の一部縮退、将来の拡張余地として明記）。`check_session_interference`/
`observation_alive`/`measurement_suspicious`はAPIとして公開済みだが、実際にセッションを
中断・再測定へ接続する配線は[ADR196-T2](adr196-t2-mismatch-adjudication.md)のスコープ
として残っている。
[ADR195-T1](adr195-t1-independent-learning-process.md)（独立学習プロセス本体）完了後に
着手。ADR-196の他タスク（T2〜T5）の前提となる基盤。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定1bは、学習セッション中に
awase自身が学習窓へ横から書き込む（A'の崩れ）ことを検出するための観測基盤を定める。
round1で提案した「固定トグルキーを1回注入して開閉の反転を見る」自己診断は、リリース
ビルドのawaseが注入キーに一切反応しない（BUG-14ガード）ため原理的に検出できないと
round2で判明し撤回された。本タスクは、その代替である「外部からの書き込みの直接観測」
を実装する。

## 実装対象（ADR-196決定1bの項目番号1〜6にそのまま揃えてある。**番号を振り直さないこと**
——round4 M-A'／横断レビューM1で、番号のずれが他タスクの相互参照を壊すことが2度確認されている）

1. **注入イベントの分類規則**: 学習プロセスは自分の`SendInput`に専用の目印を付け、
   自前の`WH_KEYBOARD_LL`フックで観測したキーイベントを次の3つに分類する。
   - `LLKHF_INJECTED`が無い → ユーザーの物理入力（[ADR195-T7](adr195-t7-safety-measures.md)
     の混入検出の対象）
   - 注入されていて自分の目印がある → 自分の注入
   - それ以外の注入（目印が無い、または別の目印） → 外部からの書き込み
   （「awaseの目印を列挙して探す」規則ではなく「自分の目印が無ければ外部」という規則に
   すること。awase側は`INJECTED_MARKER`〈`tsf/output.rs:15`〉以外にも`TSF_MARKER`
   〈warmup〉・`IME_KANJI_MARKER`〈漢字キーactuation〉を使い分けており、前者だけを
   探す規則では後2つを見落とす。**自分の目印自体は[ADR195-T1](adr195-t1-independent-learning-process.md)
   実装対象2が`SendInput`側に付ける——本タスクはそれを判定する側**）。
2. **キーを伴わない書き込みの検出（TSF通知経路の生存確認を含む）**: `WM_IME_NOTIFY`
   （`IMN_SETOPENSTATUS`/`IMN_SETCONVERSIONMODE`）とTSF compartment変更通知を監視し、
   自分で注入していない期間にこれらが届いたら外部からの書き込みとして扱う。
   **生存確認**: COMの戻り値では`advise sink`が外れたことを検出できないため、フック
   （項目4）と同じ方式にする——自分の注入で`read_status`により開閉・変換モードの変化が
   観測されたのに対応する通知が届かなかった場合、通知経路が停止したとみなしセッションを
   失敗にする。
3. **静かな観測窓（quiet window）**: フォーカス移行・デバウンス待ち直後に、注入を一切
   しない期間T msを置き、その間1・2のどちらも発生しないことをセッション開始条件にする。
   Tは`tuning.rs`の該当定数から導出し、`.claude/rules/tuning-constants.md`の実測義務に
   従って確定する（**実装前に実測が必要**）。
4. **フックの生存確認とメッセージポンピング**: `ImeDriver`の6メソッドの待ちはすべて
   `MsgWaitForMultipleObjectsEx`等でメッセージを回しながら待つ（`sleep`待ちにすると
   `LowLevelHooksTimeout`超過でフックが通知なしに外れる。**メッセージポンピング自体は
   [ADR195-T1](adr195-t1-independent-learning-process.md)実装対象2の担当**）。学習
   プロセスは自分の注入1件ごとに、自分のフックで自分の目印付きとして観測されることを
   確認し、観測されなかった注入が1件でもあればセッションを失敗にする。
5. **セッション中の監視**: 測定と測定の間の待ち時間に1・2が発生したら、その試行を
   無効化する。無効化がN回（暫定値、実測で決める）を超えたら、セッション全体を失敗
   として終了し、表を書き出さない。
6. **残余リスクの緩和**: IMMを直接呼ぶ外部書き込みが測定の窓の中に入った場合は検出
   できない（上記1・2は測定と測定の**間**を対象とする）。緩和策として、1回の注入に
   対して開閉・変換モードのcompartment変更通知が2回以上、または向きが逆転して届いた
   場合はその試行を無効にする。

## 完了条件

- 分類規則（自分の目印／外部）のユニットテスト（複数の目印すべてが「外部」に分類
  されることを含む）。
- フック・TSF通知の生存確認が、意図的に観測を止めた場合にセッション失敗になることの
  テスト。
- quiet window長T・無効化回数Nの実測値と、その根拠（`.claude/rules/tuning-constants.md`
  準拠）。

## 関連

- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定1b
- [ADR-196 opus-review-round2](../adr/196-opus-review-round2.md)（A'自己診断の原理的欠陥）
- [ADR195-T1](adr195-t1-independent-learning-process.md)
- [ADR195-T7](adr195-t7-safety-measures.md)（ユーザー入力の混入検出との関係）
