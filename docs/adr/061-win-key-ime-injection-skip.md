# ADR-061: Win キー押下中の IME キー注入スキップ

## ステータス

採用済み（2026-06-27 実装）

## コンテキスト

### 問題

Win+A などの Win ショートカット操作中に、awase がフォーカス変化を検知して
F21 / F22 / `VK_DBE_HIRAGANA` を `SendInput` で注入すると、
Windows がそれを「Win キーを押しながらのキー入力」として受け取り、
Win キーを離した瞬間にスタートメニューを誤起動させる問題が発生した。

### 発生経路

1. ユーザが Win+A（アクセシビリティ設定を開く等）を押す
2. フォーカス変化により awase がエンジン状態の同期を試みる
3. `send_engine_state_ime_key` または TSF ウォームアップの `send_vk_dbe_hiragana_pair` が呼ばれる
4. `SendInput(F21)` / `SendInput(VK_DBE_HIRAGANA)` が Win キー押下中に到達する
5. OS が Win+F21 等の未認識ショートカットとして処理し、Win↑ 時にスタートメニューが起動する

### 既存の Alt スキップとの対称性

`send_ime_mode_key` 内の `HeldModifiers::read()` は Ctrl / Shift / Alt が押下中の場合、
「Alt+Tab が確定する」などの副作用を避けるために Alt の KeyUp を先行注入する設計があった。
Win キーについては「Win を `SendInput` で解放してもスタートメニューが開く」という挙動があるため、
同様に注入自体をスキップする方式が適切と判断した。

## 決定

### 変更箇所 1: `send_ime_mode_key`（`crates/awase-windows/src/ime.rs`）

```rust
pub unsafe fn send_ime_mode_key(vk: awase::types::VkCode) {
    use crate::vk::{VK_LWIN, VK_RWIN};

    // Win キー押下中は注入をスキップする。
    // Win+F21/F22 は OS に未認識ショートカットとして届き、Win↑ のタイミングで
    // スタートメニューを誤起動させる原因になる。
    if crate::hook::is_physical_key_down(VK_LWIN) || crate::hook::is_physical_key_down(VK_RWIN) {
        log::debug!(
            "[ime-mode] skipped vk=0x{vk:02X} (Win key held — Win+F21/F22 triggers Start Menu on Win↑)"
        );
        return;
    }
    // ... 以降は HeldModifiers 処理と SendInput
}
```

### 変更箇所 2: `send_vk_dbe_hiragana_pair`（`crates/awase-windows/src/tsf/send.rs`）

```rust
pub(crate) fn send_vk_dbe_hiragana_pair() -> u64 {
    use crate::vk::{VK_DBE_HIRAGANA, VK_LWIN, VK_RWIN};

    // Win キー押下中は送信をスキップする。
    if crate::hook::is_physical_key_down(VK_LWIN) || crate::hook::is_physical_key_down(VK_RWIN) {
        log::debug!("[tsf-warmup] skipped VK_DBE_HIRAGANA (Win key held)");
        return crate::hook::current_tick_ms();
    }
    // ... 以降は SendInput
}
```

押下状態の確認には `hook::PHYSICAL_KEY_STATE`（ハードウェア由来イベントのみで更新する
256 要素の `AtomicBool` 配列）の `is_physical_key_down` を使用する。
注入キーイベントで PHYSICAL_KEY_STATE が汚染されないため、Win キーの実際の物理押下状態を
正確に読み取ることができる。

## なぜこの設計か / 検討した代替案

### 代替案 A: Win キーを `SendInput` で一時解放してから注入

Alt の Ctrl/Shift 対策と同様に Win を先に KeyUp 注入し、IME キーを送信後に再押下する方法。
ただし Win キーを `SendInput(VK_LWIN, up)` で解放してもスタートメニューが開いてしまうため
採用不可。

### 代替案 B: フォーカス変化後の注入タイミングを Win キー解放まで遅延

Win 解放後に pending キューを処理する FSM を追加する案。
ただし Win キー解放の検知を PHYSICAL_KEY_STATE で行うためにポーリングまたは追加フックが必要となり、
実装コストが高い。また Win+A → 別アプリにフォーカスが移った場合は awase が
キーイベントを受け取れなくなる可能性がある。

### 採用した設計の理由

Win キー押下中の IME キー注入は「スタートメニュー誤起動」以外の副作用もなく、
フォーカス変化由来の IME 同期はわずかに遅延しても実害がない。
最もシンプルで副作用のない「スキップして何もしない」が正解と判断した。

## 結果

- Win+A / Win+R 等の Win ショートカット中にスタートメニューが誤起動しなくなった
- `send_ime_mode_key` と `send_vk_dbe_hiragana_pair` の両エントリポイントをカバーしており、
  F21 / F22 / `VK_DBE_HIRAGANA` の全注入パスが保護される
- `PHYSICAL_KEY_STATE` が注入キーで汚染されない設計（ADR-054）を前提とするため、
  Win キーの物理押下を正確に判定できる

## 関連 ADR

- ADR-054: `PHYSICAL_KEY_STATE` の注入キーフィルタリング（物理押下状態 SSOT の設計）
- ADR-057: GJI キーバインド F13/F14 → F21/F22 移行（F21/F22 が IME キーとして使われる経緯）
- ADR-048: SacrificialWarmup による Chrome コールドスタート修正（`send_vk_dbe_hiragana_pair` の役割）
