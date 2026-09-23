# ADR-192 T1: 状態依存キーの判定関数（決定1）を実装する

状態: 未着手（2026-09-22起票）
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md)（rev8、
opus-adversarial-consult 7ラウンドで収束済み）決定1は、新しい解釈器を作らず、develop に
既にある実測表（`crates/awase-windows/src/state/key_effect_table.rs`）の上に判定関数を
定義する。この判定は決定2（警告）・決定3（案内）の入力になる、本ADR実装の基礎部分。

**このタスクの成果物は判定関数と回帰テストのみ**。警告UIやawase-settings連携は含まない
（[ADR192-T2](adr192-t2-warning-detection-and-dispatch.md)/
[T3](adr192-t3-awase-settings-guided-replacement.md)が別タスク）。ADR-192の「受益範囲の
申告」節が示唆する段階実装の第一段階に相当する。

## 実装対象（ADR-192決定1を読んで実装すること。詳細・根拠・実測データはADR本文参照）

対象VKは次の6キーのみ: `Henkan`(0x1C)・`Muhenkan`(0x1D)・`HankakuZenkaku`(0xF3/0xF4)・
`Kanji`(0x19)・`ImeOn`(0x16)・`ImeOff`(0x1A)。

1. **判定関数の配置**: `crates/awase-windows/src/state/key_effect_table.rs`のセル横断
   ロジック上に`pub fn`として実装する（`key_effect_table`モジュール自体は`state/mod.rs`で
   `mod`=privateだが、この判定関数だけを`pub`にすれば`awase-settings`からも呼べる）。
   `KeyEffectKeymap::predict`が持つ実行時向け補正（`mode == Unknown`の既定値補正等）は
   使わず、セル表を直接横断すること。
2. **(A) 開閉軸の状態依存性**: 対象VKごとに、到達可能な全セルの`(open, open_after)`から
   `Set(true)`/`Set(false)`/`Toggle`/`Identity`の4仮説のいずれかに矛盾なく一致するか判定する。
   一致しなければ状態依存。ADR本文の検算表（`ImeOn`/`ImeOff`/`Kanji`/`HankakuZenkaku`は
   非状態依存、ATOKの`Henkan`/`Muhenkan`は状態依存）と一致することを単体テストで固定する。
3. **(B) 未確定文字列の行方**: 対象VKのうち「実キーボードに存在する物理キー」（`HankakuZenkaku`・
   `Kanji`・`Henkan`・`Muhenkan`。`ImeOn`/`ImeOff`は除く——合成キーであり実キーボードに存在
   しない）に限り、かつ(A)が`Identity`でないものについて、`Disposition`が一部のセルにだけ
   `Discarded`/`Committed`を持つか判定する。この「実キーボードに存在する物理キーか」を
   判定する述語は`crates/awase-windows/src/vk.rs`に新設する（既存の`is_ime_control`は
   `0x16`/`0x19`/`0x1A`を同列に扱うため流用できない、ADR本文参照）。ADR本文の検算表
   （`HankakuZenkaku`はプリセット依存でDiscarded/Committedが分かれる、`Kanji`は常に
   Committed、`Henkan`/`Muhenkan`は非該当）と一致することを単体テストで固定する。
4. **CannotPredictの三分割**: 判定結果の型は
   `enum Classification { StateIndependent, StateDependent(Axis), CannotPredict(Reason) }`
   とし、`Reason`は`AmbiguousKeymap`（キーマップ解釈が不確か、例: BUG-143型の食い違い・
   ATOKの古い`custom_keymap_table`〈ADR-186決定(c)〉・overlay適用構成）・`UserOverride`
   （カスタム表にそのVKの行があり`predict`が`None`を返す構成）・`InsufficientData`
   （`MSIME_NATIVE`の試行数不足）の3値に分ける。`AmbiguousKeymap`/`InsufficientData`は
   最終的に「沈黙」、`UserOverride`は「伝える」という帰結になるが、**この判定関数自体は
   3値の`Reason`を返すところまでを担当し、沈黙/伝えるの分岐はT2（決定2）の責務とする**。
5. **MS-IME本体（`MSIME_NATIVE`）**: プリセット単位で`CannotPredict(InsufficientData)`
   として扱う（セル単位の信頼度判定ロジックは新設しない）。`KeyEffectKeymap::for_msime_native`
   がレジストリの再割り当てを検出した場合も同じ帰結にする。

## 完了条件・テスト

- `cargo test -p awase-windows`（Linuxで走る）に、ADR本文の検算表を再現する単体テストを
  追加する。少なくとも: `ImeOn`/`ImeOff`が(A)で非状態依存、`Enter`/`Esc`/`Bs`/`Space`/
  `Eisu`/`Hiragana`/`Katakana`が対象VK範囲外（判定を回さない）、ATOKの`Henkan`/`Muhenkan`が
  (A)で状態依存、`HankakuZenkaku`/`Kanji`が(B)で警告対象になり`ImeOn`/`ImeOff`は(B)対象外、
  `MSIME_NATIVE`がプリセット単位で`CannotPredict(InsufficientData)`になること。
- `mozc_tokens`と`awase-gji-config::MOZC_KEY_ALIASES`が同じ`"Hankaku/Zenkaku"`トークンを
  別のVKへ解決している食い違い（ADR-192決定1「既知の食い違い」節）は、このタスクでは
  解消しない（判定関数はVK単位で`key_effect_table.rs`のセルを引くだけなので直接の影響は
  無い）。将来カスタムキーマップのトークン解決を実装する際の宿題として記録に残すこと
  （このタスクファイルへのリンクで十分、新規docは不要）。
- `cargo clippy -p awase-windows -- -A clippy::cargo`が通る。

## 関連

- [ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md) 決定1
- [ADR192-T2](adr192-t2-warning-detection-and-dispatch.md)（このタスクの判定関数を消費する側）
- [ADR192-T3](adr192-t3-awase-settings-guided-replacement.md)（同上）
