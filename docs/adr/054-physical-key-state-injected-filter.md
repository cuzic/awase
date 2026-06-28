# ADR-054: PHYSICAL_KEY_STATE と LLKHF_INJECTED フィルタリング

## ステータス

採用済み（2026-06-06 実装）

## コンテキスト

### 問題1: GetAsyncKeyState の汚染

awase は IME キー（VK_KANJI 等）を送信する前後に修飾キーを一時解放・復元する
`send_ime_mode_key` を持つ。この実装では `GetAsyncKeyState` で修飾キーの押下状態を
読み取っていたが、`GetAsyncKeyState` は `SendInput` で注入した synthetic KeyUp の
影響を即座に受ける。

```
1. ユーザーが Ctrl を物理押下
2. awase が synthetic Ctrl↑ を SendInput で注入（修飾解放）
3. GetAsyncKeyState(VK_CONTROL) → false を返す ← 汚染
4. push_restore が Ctrl を復元しない → Chrome の Ctrl+W が届かない
```

### 問題2: 外部アプリによる stuck modifier

VcXsrv 等の X サーバーが synthetic Ctrl↓ を注入し、KeyUp を送らずに終了すると
`PHYSICAL_KEY_STATE` に stuck modifier が残り、IME 制御が壊れる。

### 既存の仕組み: PHYSICAL_KEY_STATE

`WH_KEYBOARD_LL` フックコールバックで VK ごとの押下状態を追跡する
`static PHYSICAL_KEY_STATE: [AtomicBool; 256]` が既に存在していた。
ただし、このテーブルは注入イベントの有無を区別せず全イベントで更新していた。

## 決定

### 1. LLKHF_INJECTED でフィルタ（commit 8351bcf）

`WH_KEYBOARD_LL` フックコールバックで `KBDLLHOOKSTRUCT.flags` の
`LLKHF_INJECTED`（0x10）ビットを確認し、外部 synthetic イベントは
`PHYSICAL_KEY_STATE` および `PHYSICAL_KEY_DOWN_AT_MS` を更新しないようにした。

```rust
// hook.rs
const LLKHF_INJECTED: u32 = 0x10;

let is_injected = (kb.flags.0 & LLKHF_INJECTED) != 0;
if !is_injected {
    if let Some(slot) = PHYSICAL_KEY_STATE.get(vk.0 as usize) {
        slot.store(is_keydown, Ordering::Relaxed);
    }
    // PHYSICAL_KEY_DOWN_AT_MS も同様にスキップ
}
```

自前の synthetic（`is_self_injected` で検出）はこのコードより前に
`CallNextHookEx` へ転送して返るため、ここには到達しない。

### 2. HeldModifiers::push_restore を is_physical_key_down に変更（commit 11817d1）

修飾復元の判断を `GetAsyncKeyState` から `is_physical_key_down()` に切り替えた。

```rust
// ime.rs: push_restore
let still = Self {
    ctrl: self.ctrl
        && (crate::hook::is_physical_key_down(VK_LCONTROL)
            || crate::hook::is_physical_key_down(VK_RCONTROL)),
    shift: self.shift
        && (crate::hook::is_physical_key_down(VK_LSHIFT)
            || crate::hook::is_physical_key_down(VK_RSHIFT)),
    // alt も同様
};
```

### 3. HeldModifiers::read と read_os_modifiers も統一（commits effb47d, 261f400）

`send_ime_mode_key` の read()、および `focus_observer.rs` の `read_os_modifiers()` も
Ctrl について `GetAsyncKeyState` → `is_physical_key_down` に統一した。

## なぜこの設計か / 検討した代替案

**代替案: INJECTED_MARKER でフィルタ**
awase 自身の synthetic には `dwExtraInfo` に `INJECTED_MARKER` を付与しており、
これで自己注入を除外する仕組みは既に動作していた。しかし外部アプリ（VcXsrv 等）の
synthetic は `INJECTED_MARKER` を持たないため除外できない。`LLKHF_INJECTED` は
OS が自動的にセットするため、外部 synthetic も漏れなくフィルタできる。

**代替案: GetKeyState（メッセージキュー処理済み）**
`GetKeyState` はメッセージキューの処理結果を返すが、
フックスレッドのコンテキストでは必ずしも最新の物理状態を反映しない。

## 結果

- `PHYSICAL_KEY_STATE` が真の「物理キー状態」を保持するようになり、
  外部 synthetic による stuck modifier の汚染を防止
- `HeldModifiers` の read/push_restore/read_os_modifiers が一貫して
  `is_physical_key_down()` を使うようになり、CTRL MISMATCH が構造的に解消
- Ctrl を押しながら IME キーが注入されても Ctrl が正しく restore され、
  Chrome の Ctrl+W 等のショートカットが届くようになった
- `HeldModifiers::read()` から `unsafe` が除去され安全性が向上

## 関連 ADR

- ADR-032: レイヤー境界（WH_KEYBOARD_LL フックコールバックの責務分離）
- ADR-040: ImmCross と VK_KANJI の所有権（ImmCross アプリへの物理 IME キー非露出）
- ADR-053: Ctrl+無変換 IME-OFF バリア化（Ctrl stuck 問題の上位コンテキスト）
