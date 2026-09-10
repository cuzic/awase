/// マイクロ秒精度のタイムスタンプ（テスト容易性のため `Instant` を置換）
pub type Timestamp = u64;

/// プラットフォーム固有のキーコード（Windows VK, macOS keycode, Linux evdev keycode）
///
/// Engine はこの値を直接検査しない。再注入・ログ出力等でプラットフォーム層に返すために保持する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct VkCode(pub u16);

impl From<u16> for VkCode {
    fn from(v: u16) -> Self {
        Self(v)
    }
}
impl From<VkCode> for u16 {
    fn from(v: VkCode) -> Self {
        v.0
    }
}
impl core::fmt::UpperHex for VkCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::UpperHex::fmt(&self.0, f)
    }
}
impl core::fmt::LowerHex for VkCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::LowerHex::fmt(&self.0, f)
    }
}

/// プラットフォーム固有のスキャンコード（Windows Set 1, macOS keycode, Linux evdev keycode）
///
/// Engine はこの値を直接検査しない。プラットフォーム層が `PhysicalPos` に変換済み。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScanCode(pub u32);

impl From<u32> for ScanCode {
    fn from(v: u32) -> Self {
        Self(v)
    }
}
impl From<ScanCode> for u32 {
    fn from(v: ScanCode) -> Self {
        v.0
    }
}
impl core::fmt::UpperHex for ScanCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::UpperHex::fmt(&self.0, f)
    }
}
impl core::fmt::LowerHex for ScanCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::LowerHex::fmt(&self.0, f)
    }
}

// ── 特殊キー ──

/// プラットフォーム非依存の特殊キー種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpecialKey {
    /// Backspace
    Backspace,
    /// Escape
    Escape,
    /// Enter / Return
    Enter,
    /// Space
    Space,
    /// Delete
    Delete,
    /// Insert
    Insert,
    /// ↑
    Up,
    /// ↓
    Down,
    /// ←
    Left,
    /// →
    Right,
    /// Home
    Home,
    /// End
    End,
    /// Page Up
    PageUp,
    /// Page Down
    PageDown,
}

// ── 修飾キー ──

/// プラットフォーム非依存の修飾キー種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModifierKey {
    Ctrl,
    Shift,
    Alt,
    /// Windows / Cmd / Super
    Meta,
}

/// 修飾キー（Ctrl / Alt / Shift / Win）の押下状態
#[derive(Debug, Default, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)] // 各修飾キーの物理状態を1:1で表現
pub struct ModifierState {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

impl ModifierState {
    /// Ctrl / Alt / Shift / Meta キーの押下状態を更新する
    ///
    /// プラットフォーム層が `RawKeyEvent.modifier_key` に事前分類した情報を使用する。
    pub const fn update(&mut self, event: &RawKeyEvent) {
        let is_down = matches!(event.event_type, KeyEventType::KeyDown);

        if let Some(mk) = event.modifier_key {
            match mk {
                ModifierKey::Ctrl => self.ctrl = is_down,
                ModifierKey::Alt => self.alt = is_down,
                ModifierKey::Shift => self.shift = is_down,
                ModifierKey::Meta => self.win = is_down,
            }
        }
    }

    /// OS 予約キーコンビネーション用の修飾キーが押下中かどうか
    #[must_use]
    pub const fn is_os_modifier_held(self) -> bool {
        self.ctrl || self.alt || self.win
    }
}

// ── IME 関連 ──

/// IME 状態への影響（プラットフォーム非依存）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowImeAction {
    TurnOn,
    TurnOff,
    Toggle,
}

impl ShadowImeAction {
    /// 現在の実効IME開閉状態（`current_open`）から、このactionが要求する
    /// 新しい開閉状態を解決する（/code-review指摘、2026-09-08——
    /// `engine.rs::apply_ime_open_request`と
    /// `awase-windows::key_pipeline::kp_stage_shadow_ime_toggle`が
    /// 同じ`match`を独立に書いていたため共有ヘルパーへ統合）。
    #[must_use]
    pub const fn resolve(self, current_open: bool) -> bool {
        match self {
            Self::TurnOn => true,
            Self::TurnOff => false,
            Self::Toggle => !current_open,
        }
    }
}

/// キーの IME 関連情報（プラットフォーム層が事前分類）
#[allow(clippy::struct_excessive_bools)]
// 各フィールドは独立の判定軸を1:1で表現（enum化はwiden/意味混同のリスクを増やす）
#[derive(Debug, Clone, Copy, Default)]
pub struct ImeRelevance {
    /// このキーが IME 状態を変更する可能性がある
    pub may_change_ime: bool,
    /// Shadow IME 状態への効果（None = 影響なし）
    pub shadow_action: Option<ShadowImeAction>,
    /// ユーザー設定の IME 同期キー（ガード対象）
    pub is_sync_key: bool,
    /// IME 同期キーの方向
    pub sync_direction: Option<ShadowImeAction>,
    /// IME 制御キー（半角/全角等、保留フラッシュ必要）
    pub is_ime_control: bool,
    /// この打鍵の直後、IME 側の状態（open/conv）が遷移しうるか
    /// （＝今この瞬間に conv を読んでも信用できないか）。
    ///
    /// `may_change_ime`（awase が IME refresh をスケジュールすべきか）とも
    /// `vk_may_mutate_conv`（IMM32 の conv ワードを変えるか、`VK_NONCONVERT`
    /// は意図的に除外）とも判定軸が異なる第3の軸（BUG-113 残置課題）。
    /// GJI 既定キーマップでは 無変換=直接入力/変換=ひらがな であり、この軸が
    /// 無いと素の 変換/無変換 の直後に idle-conv-check の cross-process 読み取りが
    /// 走り、GJI の TSF composition がまだ遷移中の値を拾って drift correction の
    /// actuation を誘発しうる（読み取りと書き込みの時間的近接、実機A/Bで
    /// 「@」の独立した十分条件と確定済み、docs/known-bugs.md BUG-113参照）。
    pub is_ime_mode_key: bool,
    /// ADR-153 決定1: 無変換/変換単独タップの明示config
    /// (`muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`) による
    /// IME open 軸 actuation を、`kp_stage_shadow_ime_toggle`
    /// （プラットフォーム層）が**この物理KeyDown 1回分について既に発行済み**
    /// であることを示すマーカー。2つの独立した消費者を持つ:
    /// - ケース2（belief OFF→ON昇格）で立てた場合: `PendingThumbData`
    ///   経由で運ばれ、100ms後の `resolve_pending_thumb_as_single`
    ///   （ケース1）が同じ打鍵を二重に actuate しないためのB13/B14対策。
    /// - ケース3（belief既にOFF×"off"、エンジンが非活性で`NicolaFsm`に
    ///   到達しない）で立てた場合: `transport.rs::plan` が同じ打鍵の生キー
    ///   配送をSuppressする判定に使う（M19対策）。
    ///
    /// 常にこのイベント1回限りの値（次のKeyDownでは
    /// `RawKeyEvent::ime_relevance` が新規に構築され直す）。
    pub explicit_ime_action_consumed: bool,
    /// ADR-154: GJI/MS-IME **自動検出**由来の`delegate_to_open_axis`が armed な
    /// 親指キーについて、`kp_stage_shadow_ime_toggle`（消費点2）がこの物理
    /// KeyDown 1回分のIME open軸を**beliefをOFF→ONへ実際に動かす形で裁定済み**
    /// であることを示すマーカー。`PendingThumbData`経由で運ばれ、100ms後の
    /// `resolve_pending_thumb_as_single`（消費点1）が同じ打鍵に対して優先順位3
    /// （delegate）を二重に発火させないために使う。
    ///
    /// **`explicit_ime_action_consumed`とは別フィールドである理由**: あちらは
    /// `transport.rs::plan`という**engine外の第2の消費者**を持ち、無変換/変換の
    /// 物理配送を`Suppress`に転じさせる。本フィールドが意味を持つのは
    /// `Decision::Consume`に乗る打鍵（＝engine活性時の親指キー）だけであり、
    /// engine非活性時（`Inactive(ImeOff)`/`Inactive(UserDisabled)`）は
    /// `Decision::PassThrough`に落ちて`plan`の戻り値が実際に物理配送を左右する。
    /// この経路で流用すると、明示config用のSuppress判定が誤発火し、無変換/変換が
    /// GJIに一切届かないままawaseも何もactuateしない——ADR-119型の「二重の空振り」
    /// を新規に作る（詳細はADR-154「決定」節）。
    ///
    /// **禁止事項**: `crates/awase-windows/src/runtime/transport.rs`の production
    /// コードはこのフィールドを読んではならない（`tests/architecture_guard.rs`の
    /// grepガードで機械的に固定する）。物理配送の可否を左右させてはならず、
    /// 非活性経路では単に捨てられる値である。
    ///
    /// 常にこのイベント1回限りの値。KeyUpでは立たない（`kp_stage_shadow_ime_toggle`
    /// がKeyDown以外を早期returnするため）——ADR-153ケース3改がKeyUp側にも
    /// マーカーを立てる特別分岐を持つのとは非対称だが、本フィールドは
    /// `PendingThumb`経由で運ばれるだけで物理配送に影響しないためKeyUpペアリングは
    /// 不要（意図的な非対称）。
    pub auto_delegate_open_axis_consumed: bool,
}

// ── キーイベント ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventType {
    KeyDown,
    KeyUp,
}

/// キーの基本分類（プラットフォーム層が事前に決定）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyClassification {
    /// 文字キー（NICOLA 変換対象、PhysicalPos あり）
    Char,
    /// 左親指キー
    LeftThumb,
    /// 右親指キー
    RightThumb,
    /// パススルー（修飾キー、Fキー、ナビゲーション等）
    Passthrough,
}

/// フックから受け取る生のキーイベント
///
/// プラットフォーム層が事前分類した情報を含む。Engine は `vk_code`/`scan_code` を
/// 直接検査せず、分類済みフィールドを使用する。
#[derive(Debug, Clone, Copy)]
pub struct RawKeyEvent {
    /// プラットフォーム固有キーコード（再注入用に保持）
    pub vk_code: VkCode,
    /// プラットフォーム固有スキャンコード（再注入用に保持）
    pub scan_code: ScanCode,
    pub event_type: KeyEventType,
    pub extra_info: usize,
    pub timestamp: Timestamp,
    /// キーの基本分類（プラットフォーム層が事前に決定）
    pub key_classification: KeyClassification,
    /// 物理キー位置（Char キーの場合のみ Some）
    pub physical_pos: Option<crate::scanmap::PhysicalPos>,
    /// IME 関連の事前分類（プラットフォーム層が設定）
    pub ime_relevance: ImeRelevance,
    /// 修飾キー分類（プラットフォーム層が設定、None = 修飾キーではない）
    pub modifier_key: Option<ModifierKey>,
    /// フック時点でキャプチャした修飾キー状態スナップショット
    ///
    /// `GetAsyncKeyState` を replay 時ではなく capture 時に呼ぶことで、
    /// `crate::INPUT_DEFER` 経由の drain 時に modifier 状態が変化していても
    /// 正しい文脈でイベントを再処理できる（doc は元々 `OUTPUT_PENDING_QUEUE` と
    /// 書いていたが、実際にこの前提へ依存している経路は `INPUT_DEFER` である。
    /// ADR-129 未決定事項4）。
    pub modifier_snapshot: ModifierState,
    /// フック時点でキャプチャした左/右親指キーの押下タイムスタンプ
    /// （[ADR-129](../docs/adr/129-thumb-timestamp-live-requery-during-gate-drain-replay.md)）。
    ///
    /// **T1系**（`hook.rs` の `update_thumb` クロージャ内 `now_timestamp()`）
    /// 由来である。同一構造体の `timestamp` フィールドは **T2系**
    /// （`build_raw_key_event` 内の `now_timestamp()`）由来で、同じ物理押下
    /// でも数 µs ずれる。**両者を減算・比較してはならない。** 本フィールドの
    /// 用途は `InputContext.left/right_thumb_down` への供給（＝
    /// `NicolaFsm::phys` への供給）のみであり、そこでの比較相手は同じく
    /// T1系の `left_thumb_consumed`/`right_thumb_consumed` である。
    ///
    /// `modifier_snapshot` と同じ理由（capture 時点で埋め込み、drain replay
    /// 時にライブ再取得しない）でこのフィールドを持つ。`OUTPUT_GATE` active
    /// 中に `crate::INPUT_DEFER` へ退避されたイベントの drain replay で、
    /// このスナップショットが無ければ「replay を実行している"今"」のライブ
    /// 値を誤って読んでしまう（ADR-129 が扱う実インシデント）。
    pub left_thumb_down_snapshot: Option<Timestamp>,
    /// [`Self::left_thumb_down_snapshot`] の右親指版。
    pub right_thumb_down_snapshot: Option<Timestamp>,
    /// ソフトウェア注入イベント（Windows: `LLKHF_INJECTED`）。
    ///
    /// awase 自身の注入（marker 付き）はフック層で除外済みのため、これが true なのは
    /// 他プロセス（MS-IME/CTF・タッチキーボード・他ツール等）の SendInput 由来のみ。
    /// 注入イベントはユーザーの物理操作ではないので、ユーザー意図（PhysicalImeKey 等）
    /// に昇格させてはならない（BUG-14: 外部注入 VK_DBE_HIRAGANA を物理かなキーと誤読し
    /// ユーザーの IME OFF を Engine ON で上書きし続けた）。
    pub injected: bool,
}

impl RawKeyEvent {
    /// フォーカス復帰後の resync を起動する「本命の1打鍵」かどうか。
    ///
    /// report `01M0VGJ2M5KQHD1D9V7HAMBHNT`（Alt+Tab 復帰直後の最初のキーが
    /// resync 完了前に PassThrough でリテラル漏れする）の修正で、この1打鍵を
    /// resync 完了まで defer するために使う判定。リテラル漏れが起きうるのは
    /// `Char`/親指キーの `KeyDown` のみなので、それ以外は resync を起動しない:
    ///
    /// - `Passthrough`（修飾キー・Fキー・Tab 等のナビゲーション）は対象外
    ///   （Alt+Tab 連打で Tab 自体が resync を消費し、スイッチャー操作が
    ///   遅延する事故を防ぐ）。
    /// - `KeyUp` は対象外（Alt/Ctrl/Shift/Win の解放イベントが resync を
    ///   消費すると、後続の本命キーが素通りしてしまう）。
    /// - 修飾キー（Ctrl/Alt/Win）保持中は対象外（ショートカットを遅延させない）。
    ///   Shift は対象外にしない（Shift+文字は正当な通常入力のため）。
    /// - 外部注入（`injected`）は対象外。awase 自身の注入はフック層で除外済み
    ///   のためこれが true なのは他プロセス由来のみで、ユーザーの物理操作では
    ///   ない（BUG-14: 外部注入 VK_DBE_HIRAGANA を物理かなキーと誤読した前例）。
    #[must_use]
    pub const fn starts_focus_resync(&self) -> bool {
        matches!(self.event_type, KeyEventType::KeyDown)
            && matches!(
                self.key_classification,
                KeyClassification::Char
                    | KeyClassification::LeftThumb
                    | KeyClassification::RightThumb
            )
            && !self.modifier_snapshot.ctrl
            && !self.modifier_snapshot.alt
            && !self.modifier_snapshot.win
            && !self.injected
    }
}

/// 出力アクション
#[derive(Debug, Clone)]
pub enum KeyAction {
    /// 特殊キーを押下（プラットフォーム非依存）
    SpecialKey(SpecialKey),
    /// プラットフォーム固有キーコードを押下（再注入・フォールバック用）
    Key(VkCode),
    /// プラットフォーム固有キーコードをリリース
    KeyUp(VkCode),
    /// Unicode 文字を直接出力
    Char(char),
    /// 何もしない（キーを握りつぶす）
    Suppress,
    /// ローマ字文字列をキーイベントとして送信（IME ローマ字入力モード用）
    Romaji(String),
    /// キーシーケンスとして出力（IME がキーストロークを変換する）
    KeySequence(String),
    /// Ctrl+VK の単一チョード送信（ADR-115 決定1）。1回の `SendInput` バッチで
    /// press/release が自己完結するため、`OutputHistory` の KeyUp 整合性索引
    /// の対象にはならない（`Key`/`KeyUp` と違い解放すべき片割れを持たない）。
    CtrlChord(VkCode),
    /// 打鍵列（ADR-115 決定4）。1回の `send_keys` 呼び出しの中で即座に全要素を
    /// 実行する同期バッチモデル——待機（`Wait`）を含むステップは表現しない
    /// （ADR-115 決定11、将来別概念として設計する）。
    Sequence(Vec<Self>),
}

impl KeyAction {
    /// `Romaji` バリアントからローマ字文字列を返す。他のバリアントは空文字列。
    #[must_use]
    pub fn romaji(&self) -> &str {
        if let Self::Romaji(s) = self {
            s
        } else {
            ""
        }
    }
}

/// コンテキスト無効化の理由（ログ・デバッグ用）
#[derive(Debug, Clone, Copy)]
pub enum ContextChange {
    /// IME がオフになった
    ImeOff,
    /// 入力言語が変更された
    InputLanguageChanged,
    /// エンジンが無効化された（ホットキー等）
    EngineDisabled,
    /// レイアウトが差し替えられた
    LayoutSwapped,
    /// フォーカスが別のコントロールに移動した
    FocusChanged,
    /// バイパスキーイベント（Passthrough / IME制御 / OSモディファイア）
    BypassKey,
}

#[cfg(test)]
mod tests {
    use itertools::Itertools as _;

    use super::*;

    // ── ShadowImeAction::resolve ──

    #[test]
    fn shadow_ime_action_resolve_matches_expected_truth_table() {
        // /code-review指摘（2026-09-08）で追加した共有ヘルパー。
        // engine.rs::apply_ime_open_request と
        // key_pipeline.rs::kp_stage_shadow_ime_toggle が独立に実装していた
        // 同じ真理値表をここに固定する。
        assert!(ShadowImeAction::TurnOn.resolve(false));
        assert!(ShadowImeAction::TurnOn.resolve(true));
        assert!(!ShadowImeAction::TurnOff.resolve(false));
        assert!(!ShadowImeAction::TurnOff.resolve(true));
        assert!(ShadowImeAction::Toggle.resolve(false));
        assert!(!ShadowImeAction::Toggle.resolve(true));
    }

    // ── RawKeyEvent::starts_focus_resync ──

    fn resync_probe_event(
        event_type: KeyEventType,
        key_classification: KeyClassification,
        modifier_snapshot: ModifierState,
        injected: bool,
    ) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: VkCode(0),
            scan_code: ScanCode(0),
            event_type,
            extra_info: 0,
            timestamp: 0,
            key_classification,
            physical_pos: None,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
            modifier_snapshot,
            left_thumb_down_snapshot: None,
            right_thumb_down_snapshot: None,
            injected,
        }
    }

    #[test]
    fn alt_keyup_does_not_start_resync() {
        let e = resync_probe_event(
            KeyEventType::KeyUp,
            KeyClassification::Passthrough,
            ModifierState {
                alt: true,
                ..Default::default()
            },
            false,
        );
        assert!(!e.starts_focus_resync());
    }

    #[test]
    fn ctrl_shift_win_keyup_do_not_start_resync() {
        for snapshot in [
            ModifierState {
                ctrl: true,
                ..Default::default()
            },
            ModifierState {
                shift: true,
                ..Default::default()
            },
            ModifierState {
                win: true,
                ..Default::default()
            },
        ] {
            let e = resync_probe_event(
                KeyEventType::KeyUp,
                KeyClassification::Passthrough,
                snapshot,
                false,
            );
            assert!(!e.starts_focus_resync());
        }
    }

    #[test]
    fn tab_keydown_does_not_start_resync() {
        // Tab は KeyClassification::Passthrough（ナビゲーション）。
        // Alt+Tab 連打で Tab 自体が resync を消費しないことの回帰。
        let e = resync_probe_event(
            KeyEventType::KeyDown,
            KeyClassification::Passthrough,
            ModifierState {
                alt: true,
                ..Default::default()
            },
            false,
        );
        assert!(!e.starts_focus_resync());
    }

    #[test]
    fn char_keydown_with_alt_held_does_not_start_resync() {
        let e = resync_probe_event(
            KeyEventType::KeyDown,
            KeyClassification::Char,
            ModifierState {
                alt: true,
                ..Default::default()
            },
            false,
        );
        assert!(!e.starts_focus_resync());
    }

    #[test]
    fn injected_char_keydown_does_not_start_resync() {
        // BUG-14 の踏み直し防止: 外部注入イベントはユーザーの物理操作ではない。
        let e = resync_probe_event(
            KeyEventType::KeyDown,
            KeyClassification::Char,
            ModifierState::default(),
            true,
        );
        assert!(!e.starts_focus_resync());
    }

    #[test]
    fn char_keyup_does_not_start_resync() {
        let e = resync_probe_event(
            KeyEventType::KeyUp,
            KeyClassification::Char,
            ModifierState::default(),
            false,
        );
        assert!(!e.starts_focus_resync());
    }

    #[test]
    fn plain_char_keydown_starts_resync() {
        let e = resync_probe_event(
            KeyEventType::KeyDown,
            KeyClassification::Char,
            ModifierState::default(),
            false,
        );
        assert!(e.starts_focus_resync());
    }

    #[test]
    fn thumb_keydown_starts_resync() {
        for kc in [KeyClassification::LeftThumb, KeyClassification::RightThumb] {
            let e = resync_probe_event(KeyEventType::KeyDown, kc, ModifierState::default(), false);
            assert!(e.starts_focus_resync());
        }
    }

    #[test]
    fn shift_held_char_keydown_starts_resync() {
        // Shift+文字は正当な通常入力のため除外条件に入れない。
        let e = resync_probe_event(
            KeyEventType::KeyDown,
            KeyClassification::Char,
            ModifierState {
                shift: true,
                ..Default::default()
            },
            false,
        );
        assert!(e.starts_focus_resync());
    }

    // ── KeyClassification ──

    #[test]
    fn key_classification_variants_exist() {
        let variants = [
            KeyClassification::Char,
            KeyClassification::LeftThumb,
            KeyClassification::RightThumb,
            KeyClassification::Passthrough,
        ];
        for ((i, a), (j, b)) in variants
            .iter()
            .enumerate()
            .cartesian_product(variants.iter().enumerate())
        {
            assert_eq!(i == j, a == b);
        }
    }

    // ── KeyAction::romaji ──

    /// `Romaji` からは中身を、それ以外は空文字列を返す。mutants: 固定値
    /// (`""`/`"xyzzy"`) への置換が既存テストでは検知できなかった。
    #[test]
    fn key_action_romaji_extracts_string_from_romaji_variant() {
        assert_eq!(KeyAction::Romaji("ka".to_string()).romaji(), "ka");
    }

    #[test]
    fn key_action_romaji_is_empty_for_non_romaji_variants() {
        assert_eq!(KeyAction::Suppress.romaji(), "");
        assert_eq!(KeyAction::KeySequence("x".to_string()).romaji(), "");
    }

    // ── ImeRelevance ──

    #[test]
    fn ime_relevance_default() {
        let d = ImeRelevance::default();
        assert!(!d.may_change_ime);
        assert!(d.shadow_action.is_none());
        assert!(!d.is_sync_key);
        assert!(d.sync_direction.is_none());
        assert!(!d.is_ime_control);
    }

    // ── KeyEventType ──

    #[test]
    fn key_event_type_equality() {
        assert_eq!(KeyEventType::KeyDown, KeyEventType::KeyDown);
        assert_eq!(KeyEventType::KeyUp, KeyEventType::KeyUp);
        assert_ne!(KeyEventType::KeyDown, KeyEventType::KeyUp);
    }

    // ── SpecialKey ──

    #[test]
    fn special_key_all_variants() {
        let variants = [
            SpecialKey::Backspace,
            SpecialKey::Escape,
            SpecialKey::Enter,
            SpecialKey::Space,
            SpecialKey::Delete,
        ];
        assert_eq!(variants.len(), 5);
        for ((i, a), (j, b)) in variants
            .iter()
            .enumerate()
            .cartesian_product(variants.iter().enumerate())
        {
            assert_eq!(i == j, a == b);
        }
    }

    // ── ModifierKey ──

    #[test]
    fn modifier_key_all_variants() {
        let variants = [
            ModifierKey::Ctrl,
            ModifierKey::Shift,
            ModifierKey::Alt,
            ModifierKey::Meta,
        ];
        assert_eq!(variants.len(), 4);
        for ((i, a), (j, b)) in variants
            .iter()
            .enumerate()
            .cartesian_product(variants.iter().enumerate())
        {
            assert_eq!(i == j, a == b);
        }
    }

    // ── ShadowImeAction ──

    #[test]
    fn shadow_ime_action_all_variants() {
        let variants = [
            ShadowImeAction::TurnOn,
            ShadowImeAction::TurnOff,
            ShadowImeAction::Toggle,
        ];
        assert_eq!(variants.len(), 3);
        for ((i, a), (j, b)) in variants
            .iter()
            .enumerate()
            .cartesian_product(variants.iter().enumerate())
        {
            assert_eq!(i == j, a == b);
        }
    }
}
