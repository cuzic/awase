# ADR-057: GJI キーバインド F13/F14 → F21/F22 への移行

## ステータス

採用済み（2026-06-17 実装、2026-06-18 DirectInput バグ修正）

## コンテキスト

### 問題: F13/F14 が terminal でエスケープシーケンスを生成する

awase は Google 日本語入力（GJI）の `config1.db` にカスタムキーバインドを書き込み、
F13→IMEOn / F14→IMEOff として使用していた。

F13（VK=0x7C）と F14（VK=0x7D）は物理キーとして存在する拡張ファンクションキーで、
WezTerm や xterm 等の terminal エミュレータに対して `\e[25~` / `\e[26~` という
エスケープシーケンスを生成するリスクがあった。awase が IME 制御のために注入した
キーストロークが terminal に漏れると、意図しない文字列が入力に混入する。

加えて、一部の DirectInput ゲームや CAD ソフトウェアでは F13/F14 に
デフォルトのアクションが割り当てられており、注入キーが誤動作を引き起こす可能性もあった。

## 決定

### F21/F22（VK=0x84/0x85）へ移行

F21（VK=0x84）と F22（VK=0x85）は Windows VK 仕様上の予約コードで、
物理キーボードに対応するキーが存在しない。WezTerm での実測によりエスケープシーケンスを
生成しないことを確認した上で採用した。

`vk.rs` の定数を更新:

```rust
// Before
pub const VK_F13: VkCode = VkCode(0x7C);
pub const VK_F14: VkCode = VkCode(0x7D);

// After
pub const VK_F21: VkCode = VkCode(0x84);
pub const VK_F22: VkCode = VkCode(0x85);
```

`gji.rs` の `ENTRIES` を更新（commit 7f8291f）:

```rust
pub const ENTRIES: &[&str] = &[
    "DirectInput\tF21\tIMEOn\n",      // ← 後日追加（9d11cd7 参照）
    "Precomposition\tF21\tIMEOn\n",
    "Precomposition\tF22\tIMEOff\n",
    "Composition\tF21\tIMEOn\n",
    "Composition\tF22\tIMEOff\n",
    "Conversion\tF21\tIMEOn\n",
    "Conversion\tF22\tIMEOff\n",
];
```

同時に `ime.rs`（`post_gji_ime_on/off` の送信キー）、`tuning.rs`（定数名
`F14F13_WAIT_MS` → `F22F21_WAIT_MS`）、`tsf/probe_fsm.rs`（KeySeq）も更新した。

### バグ修正: DirectInput エントリの追加漏れ（commit 9d11cd7）

F21/F22 移行直後に「IME-ON になれない」バグが発生した。根本原因は `ENTRIES` に
`DirectInput\tF21\tIMEOn` が含まれていなかったこと。

F22 を送ると GJI は Conversion/Composition 状態から **DirectInput**（IME-OFF）に遷移する。
その後 F21 を送っても DirectInput 状態に対応するバインドが存在しないため
GJI が F21 を無視し、IME-ON できなかった。

`ime_controller.rs` のコメントに「DirectInput はデフォルト登録済み」という誤記があり、
ENTRIES から意図的に外されていたのが原因だった。修正後、完全なバインドセットは
7 エントリ（DirectInput×1 + Precomposition/Composition/Conversion それぞれ×2）となった。

## なぜこの設計か / 検討した代替案

### VK_IME_ON (0x16) / VK_IME_OFF (0x1A) を直接使う案

これらは Chrome では機能しない（ADR-034 参照）。GJI 経由のカスタムバインドが必要。

### F15〜F20 を使う案

F15〜F20 も物理キーが存在しないが、VK_F21/F22 と同様に扱える。
F21/F22 を選んだのは VK コードの連続性よりも「より高い番号ほど実使用例が少ない」
という経験則による。実測で問題がないことを確認できた F21/F22 を採用した。

## 結果

- terminal への IME 制御キー漏れ問題が解消された
- DirectInput 系アプリとの競合リスクが大幅に低減した
- `gji.rs` の `ENTRIES` は 7 エントリが完全セットとして確定した
- `patch` / `unpatch` 関数は差分適用方式のため、既存インストールは
  トレイメニューから「GJI セットアップ」を再実行することで自動的に F21/F22 へ移行される

## 関連 ADR

- ADR-034: GJI DirectStrategy（GJI バインドによる IME 制御の全体設計）
- ADR-046: GJI FSM warm/cold SSOT（F22→F21 シーケンスを使う TSF probe の設計）
