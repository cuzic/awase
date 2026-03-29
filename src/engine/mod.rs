use std::collections::HashMap;
use std::time::Duration;

use timed_fsm::{Response, TimedStateMachine};

use crate::ngram::NgramModel;
use crate::scanmap::scan_to_pos;
use crate::types::{KeyAction, KeyEventType, RawKeyEvent, Timestamp};
use crate::yab::{YabFace, YabLayout, YabValue};

/// 同時打鍵判定用タイマー ID
pub const TIMER_PENDING: usize = 1;

/// `Response` の型エイリアス
type Resp = Response<KeyAction, usize>;

/// キーの分類（フック受信時に一度だけ決定）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyClass {
    /// 文字キー（配列変換の対象）
    Char,
    /// 左親指キー
    LeftThumb,
    /// 右親指キー
    RightThumb,
    /// パススルー（修飾キー、Fキー、ナビゲーション等）
    Passthrough,
}

impl KeyClass {
    const fn is_thumb(self) -> bool {
        matches!(self, Self::LeftThumb | Self::RightThumb)
    }

    const fn is_left_thumb(self) -> bool {
        matches!(self, Self::LeftThumb)
    }
}

/// 配列の面を表す列挙型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    Normal,
    LeftThumb,
    RightThumb,
    Shift,
}

impl Face {
    /// KeyClass の親指キーから対応する Face を取得
    const fn from_thumb(key_class: KeyClass) -> Self {
        match key_class {
            KeyClass::LeftThumb => Self::LeftThumb,
            KeyClass::RightThumb => Self::RightThumb,
            _ => Self::Normal, // fallback
        }
    }

    const fn from_thumb_bool(is_left: bool) -> Self {
        if is_left {
            Self::LeftThumb
        } else {
            Self::RightThumb
        }
    }
}

/// エンジン内部の判定結果。finalize() で Response に変換される。
enum Outcome {
    /// キーが確定した（アクション列を出力 + タイマー管理）
    Confirmed(Vec<KeyAction>),
    /// 保留状態に入った（タイマー起動）
    Pending,
    /// 素通し
    PassThrough,
}

impl Outcome {
    fn confirmed_one(action: KeyAction) -> Self {
        Self::Confirmed(vec![action])
    }
}

/// 同時打鍵判定用の状態
#[derive(Debug)]
enum NicolaState {
    Idle,
    PendingChar {
        #[allow(dead_code)] // Phase 4 以降で SendInput 時に使用予定
        scan_code: u32,
        vk_code: u16,
        timestamp: Timestamp,
    },
    PendingThumb {
        #[allow(dead_code)] // Phase 4 以降で SendInput 時に使用予定
        scan_code: u32,
        vk_code: u16,
        is_left: bool,
        timestamp: Timestamp,
    },
    /// 文字キー → 親指キーの順に到着し、3 鍵目（char2）を待機中
    PendingCharThumb {
        #[allow(dead_code)] // Phase 4 以降で SendInput 時に使用予定
        char_scan: u32,
        char_vk: u16,
        char_timestamp: Timestamp,
        thumb_vk: u16,
        thumb_is_left: bool,
        thumb_timestamp: Timestamp,
    },
}

/// 修飾キー（Ctrl / Alt / Shift）の押下状態
#[derive(Debug, Default)]
struct ModifierState {
    ctrl: bool,
    alt: bool,
    shift: bool,
}

impl ModifierState {
    /// Ctrl / Alt / Shift キーの押下状態を更新する
    const fn update(&mut self, event: &RawKeyEvent) {
        let is_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );

        match event.vk_code {
            // Ctrl (generic), LCtrl, RCtrl
            0x11 | 0xA2 | 0xA3 => self.ctrl = is_down,
            // Alt (generic), LAlt, RAlt
            0x12 | 0xA4 | 0xA5 => self.alt = is_down,
            // Shift (generic), LShift, RShift
            0x10 | 0xA0 | 0xA1 => self.shift = is_down,
            _ => {}
        }
    }

    /// OS 予約キーコンビネーション用の修飾キーが押下中かどうか
    const fn is_os_modifier_held(&self) -> bool {
        self.ctrl || self.alt
    }
}

impl From<&YabValue> for KeyAction {
    fn from(value: &YabValue) -> Self {
        match value {
            YabValue::Romaji { romaji, .. } => Self::Romaji(romaji.clone()),
            YabValue::Literal(s) => s.chars().next().map_or(Self::Suppress, Self::Char),
            YabValue::Special(sk) => Self::Key(sk.to_vk()),
            YabValue::None => Self::Suppress,
        }
    }
}

#[cfg(test)]
/// `YabValue` をエンジン出力用の `KeyAction` に変換する（`From` トレイトへの委譲）。
fn yab_value_to_action(value: &YabValue) -> KeyAction {
    KeyAction::from(value)
}

/// 配列変換エンジン（状態機械 + 同時打鍵判定）
#[allow(missing_debug_implementations)] // HashMap<u32, KeyAction> が Debug 不向き
pub struct Engine {
    /// 配列定義（.yab ベース）
    layout: YabLayout,

    /// 左親指キーの仮想��ーコード
    left_thumb_vk: u16,

    /// 右親指キーの仮想キーコード
    right_thumb_vk: u16,

    /// 同時打鍵判定用の状態
    state: NicolaState,

    /// 修飾キーの押下状態
    modifiers: ModifierState,

    /// 左親指キーが押下中か（押下時刻を保持）
    left_thumb_down: Option<Timestamp>,

    /// 右親指キーが押下中か（押下時刻を保持）
    right_thumb_down: Option<Timestamp>,

    /// 同時打鍵の判定閾値（マイクロ秒）
    threshold_us: u64,

    /// 物理キー → 注入済みキーの対応（KeyUp 時の整合性維持用）
    /// scan_code をキーとする（同一物理キーは常に同じ scan_code を持つ）
    active_keys: HashMap<u32, KeyAction>,

    /// エンジンの有効/無効
    enabled: bool,

    /// n-gram モデル（None なら固定閾値にフォールバック）
    ngram_model: Option<NgramModel>,

    /// 直近の出力文字履歴（n-gram 判定用、最大 3 文字）
    recent_output: Vec<char>,
}

impl Engine {
    #[must_use]
    pub fn new(
        layout: YabLayout,
        left_thumb_vk: u16,
        right_thumb_vk: u16,
        threshold_ms: u32,
    ) -> Self {
        Self {
            layout,
            left_thumb_vk,
            right_thumb_vk,
            state: NicolaState::Idle,
            modifiers: ModifierState::default(),
            left_thumb_down: None,
            right_thumb_down: None,
            threshold_us: u64::from(threshold_ms) * 1000,
            active_keys: HashMap::new(),
            enabled: true,
            ngram_model: None,
            recent_output: Vec::new(),
        }
    }

    /// エンジンの有効/無効を切り替える
    pub fn toggle_enabled(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.recent_output.clear();
        log::info!(
            "Engine {}",
            if self.enabled { "enabled" } else { "disabled" }
        );
        self.enabled
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// n-gram モデルを設定する。
    ///
    /// 設定すると、同時打鍵判定の閾値が候補文字の出現頻度に応じて動的に調整される。
    pub fn set_ngram_model(&mut self, model: NgramModel) {
        self.ngram_model = Some(model);
    }

    /// n-gram モデルに基づいて閾値を動的に調整する。
    ///
    /// モデルが未設定の場合は固定閾値 (`threshold_us`) を返す。
    fn adjusted_threshold_us(&self, candidate: char) -> u64 {
        self.ngram_model
            .as_ref()
            .map_or(self.threshold_us, |model| {
                model.adjusted_threshold(&self.recent_output, candidate)
            })
    }

    /// 直近の出力文字履歴に文字を追加する（最大 3 文字を保持）。
    fn push_recent_output(&mut self, ch: char) {
        if self.recent_output.len() >= 3 {
            self.recent_output.remove(0);
        }
        self.recent_output.push(ch);
    }

    /// 事��解決済みの仮名文字を `recent_output` に追加する。
    fn track_output(&mut self, kana: Option<char>) {
        if let Some(ch) = kana {
            self.push_recent_output(ch);
        }
    }

    /// 配列を動的に差し替える。保留中のキーがあればタイムアウトとして確定する。
    pub fn swap_layout(&mut self, layout: YabLayout) -> Resp {
        let timeout_response = self.on_timeout(TIMER_PENDING);
        self.state = NicolaState::Idle;
        self.layout = layout;
        self.active_keys.clear();
        self.recent_output.clear();
        timeout_response
    }

    /// 保留中のキーがあるかどうか
    const fn has_pending(&self) -> bool {
        !matches!(self.state, NicolaState::Idle)
    }

    /// Face 列挙値に対応する YabFace への参照を返す
    const fn get_face(&self, face: Face) -> &YabFace {
        match face {
            Face::Normal => &self.layout.normal,
            Face::LeftThumb => &self.layout.left_thumb,
            Face::RightThumb => &self.layout.right_thumb,
            Face::Shift => &self.layout.shift,
        }
    }

    /// scan_code から PhysicalPos を経由して YabFace を引き、`KeyAction` と
    /// 事前解決済みの仮名文字を返す。
    #[allow(clippy::unused_self)]
    fn lookup_face(
        &self,
        scan_code: u32,
        _vk_code: u16,
        face: &YabFace,
    ) -> Option<(KeyAction, Option<char>)> {
        let pos = scan_to_pos(scan_code)?;
        let value = face.get(&pos)?;
        let kana = match value {
            YabValue::Romaji { kana, .. } => *kana,
            YabValue::Literal(s) => s.chars().next(),
            _ => None,
        };
        Some((KeyAction::from(value), kana))
    }

    /// `PendingCharThumb` 状態で char1+thumb を同時打鍵として解決し、アクション列を返す
    fn resolve_char_thumb_as_simultaneous(
        &mut self,
        char_scan: u32,
        char_vk: u16,
        thumb_is_left: bool,
        thumb_timestamp: Timestamp,
    ) -> Vec<KeyAction> {
        let thumb_face = Face::from_thumb_bool(thumb_is_left);
        if let Some((action, kana)) =
            self.lookup_face(char_scan, char_vk, self.get_face(thumb_face))
        {
            if thumb_is_left {
                self.left_thumb_down = Some(thumb_timestamp);
            } else {
                self.right_thumb_down = Some(thumb_timestamp);
            }
            self.active_keys.insert(char_scan, action.clone());
            self.track_output(kana);
            vec![action]
        } else {
            // 親指面に定義がない場合は文字キーを単独確定
            self.resolve_pending_char_as_single(char_scan, char_vk)
        }
    }

    /// タイマー Duration を生成するヘルパー
    const fn pending_duration(&self) -> Duration {
        Duration::from_micros(self.threshold_us)
    }

    /// Outcome を Response に変換する（タイマー命令を自動付与）
    fn finalize(&self, outcome: Outcome) -> Resp {
        match outcome {
            Outcome::Confirmed(actions) => {
                let r = Response::emit(actions).with_kill_timer(TIMER_PENDING);
                if self.has_pending() {
                    r.with_timer(TIMER_PENDING, self.pending_duration())
                } else {
                    r
                }
            }
            Outcome::Pending => Response::consume()
                .with_kill_timer(TIMER_PENDING)
                .with_timer(TIMER_PENDING, self.pending_duration()),
            Outcome::PassThrough => Response::pass_through(),
        }
    }

    fn on_key_down(&mut self, event: &RawKeyEvent) -> Resp {
        let key_class = self.classify(event);

        // 変換対象外のキー（修飾キー等）はそのまま通す
        if key_class == KeyClass::Passthrough {
            return Response::pass_through();
        }

        // OS 予約ショートカット（Ctrl+C, Alt+Tab 等）は変換せずそのまま通す
        if self.modifiers.is_os_modifier_held() {
            return Response::pass_through();
        }

        // Shift 面：Shift が押下中かつ親指キーでない場合、配列の Shift 面を参照
        if self.modifiers.shift && !key_class.is_thumb() {
            return self.handle_shift(event);
        }

        // ── (state, key_class) ディスパッチテーブル ──
        let prev = std::mem::replace(&mut self.state, NicolaState::Idle);
        match (prev, key_class) {
            // Idle
            (NicolaState::Idle, _) => self.handle_idle(event, key_class),

            // PendingChar + thumb → 同時打鍵候補
            (
                NicolaState::PendingChar {
                    scan_code,
                    vk_code,
                    timestamp,
                },
                KeyClass::LeftThumb | KeyClass::RightThumb,
            ) => self.handle_pending_char_thumb(scan_code, vk_code, timestamp, event, key_class),

            // PendingChar + char → flush + 新規保留
            (
                NicolaState::PendingChar {
                    scan_code, vk_code, ..
                },
                KeyClass::Char,
            ) => self.handle_pending_char_char(scan_code, vk_code, event),

            // PendingThumb + char → 同時打鍵候補
            (
                NicolaState::PendingThumb {
                    vk_code,
                    is_left,
                    timestamp,
                    ..
                },
                KeyClass::Char,
            ) => self.handle_pending_thumb_char(vk_code, is_left, timestamp, event),

            // PendingThumb + thumb → flush + 新規保留
            (
                NicolaState::PendingThumb { vk_code, .. },
                KeyClass::LeftThumb | KeyClass::RightThumb,
            ) => self.handle_pending_thumb_thumb(vk_code, event, key_class),

            // PendingCharThumb → 3 鍵仲裁
            (
                NicolaState::PendingCharThumb {
                    char_scan,
                    char_vk,
                    char_timestamp,
                    thumb_is_left,
                    thumb_timestamp,
                    ..
                },
                _,
            ) => self.resolve_pending_char_thumb(
                event,
                key_class,
                char_scan,
                char_vk,
                char_timestamp,
                thumb_is_left,
                thumb_timestamp,
            ),

            // Passthrough は冒頭のガードで除外済み
            (_, KeyClass::Passthrough) => unreachable!(),
        }
    }

    /// Shift 面の処理（状態非依存）
    fn handle_shift(&mut self, event: &RawKeyEvent) -> Resp {
        if let Some((action, kana)) =
            self.lookup_face(event.scan_code, event.vk_code, self.get_face(Face::Shift))
        {
            self.active_keys.insert(event.scan_code, action.clone());
            self.track_output(kana);
            self.finalize(Outcome::Confirmed(vec![action]))
        } else {
            // Shift 面に定義がないキーは OS に任せる
            Response::pass_through()
        }
    }

    /// Idle 状態での新規キー押下処理
    fn handle_idle(&mut self, event: &RawKeyEvent, key_class: KeyClass) -> Resp {
        // Shift 面：Shift が押下中かつ親指キーでない場合、配列の Shift 面を参照
        if self.modifiers.shift && !key_class.is_thumb() {
            return self.handle_shift(event);
        }

        // 親指キーが既に押下中なら即時同時打鍵
        if !key_class.is_thumb() {
            if let Some(face) = self.active_thumb_face() {
                if let Some((action, kana)) =
                    self.lookup_face(event.scan_code, event.vk_code, self.get_face(face))
                {
                    self.active_keys.insert(event.scan_code, action.clone());
                    self.track_output(kana);
                    return self.finalize(Outcome::Confirmed(vec![action]));
                }
            }
        }

        // 配列定義にもなく親指キーでもないキーは即座に素通しする
        // （Enter, Backspace, Tab 等に不要な 100ms 遅延を生じさせない）
        if !key_class.is_thumb() && !self.is_layout_key(event.vk_code, event.scan_code) {
            return Response::pass_through();
        }

        // 新たに保留
        self.state = if key_class.is_thumb() {
            NicolaState::PendingThumb {
                scan_code: event.scan_code,
                vk_code: event.vk_code,
                is_left: key_class.is_left_thumb(),
                timestamp: event.timestamp,
            }
        } else {
            NicolaState::PendingChar {
                scan_code: event.scan_code,
                vk_code: event.vk_code,
                timestamp: event.timestamp,
            }
        };
        self.finalize(Outcome::Pending)
    }

    /// PendingChar + 親指キー → 同時打鍵候補（閾値内なら PendingCharThumb、超過なら flush+新規）
    fn handle_pending_char_thumb(
        &mut self,
        pending_scan: u32,
        pending_vk: u16,
        pending_ts: Timestamp,
        event: &RawKeyEvent,
        key_class: KeyClass,
    ) -> Resp {
        let elapsed_us = event.timestamp.saturating_sub(pending_ts);

        // 親指面で保留文字キーの候補を取得し閾値を調整
        let candidate_face = Face::from_thumb(key_class);
        let candidate = self.lookup_face(pending_scan, pending_vk, self.get_face(candidate_face));
        let threshold = candidate
            .as_ref()
            .and_then(|(_, kana)| *kana)
            .map_or(self.threshold_us, |ch| self.adjusted_threshold_us(ch));

        if elapsed_us < threshold {
            // 保留=文字, 到着=親指 → PendingCharThumb へ遷移（3 鍵目を待つ）
            self.state = NicolaState::PendingCharThumb {
                char_scan: pending_scan,
                char_vk: pending_vk,
                char_timestamp: pending_ts,
                thumb_vk: event.vk_code,
                thumb_is_left: key_class.is_left_thumb(),
                thumb_timestamp: event.timestamp,
            };
            return self.finalize(Outcome::Pending);
        }

        // 時間超過 → 前の保留を単独確定 + 今回のキーを新規処理
        let prev_actions = self.resolve_pending_char_as_single(pending_scan, pending_vk);
        let new_result = self.handle_idle(event, key_class);
        self.combine_prev_and_new(prev_actions, new_result)
    }

    /// PendingChar + 文字キー → 前の保留を単独確定 + 今回のキーを新規処理
    fn handle_pending_char_char(
        &mut self,
        pending_scan: u32,
        pending_vk: u16,
        event: &RawKeyEvent,
    ) -> Resp {
        let prev_actions = self.resolve_pending_char_as_single(pending_scan, pending_vk);
        let new_result = self.handle_idle(event, KeyClass::Char);
        self.combine_prev_and_new(prev_actions, new_result)
    }

    /// PendingThumb + 文字キー → 同時打鍵候補（閾値内なら即時確定、超過なら flush+新規）
    fn handle_pending_thumb_char(
        &mut self,
        pending_vk: u16,
        pending_is_left: bool,
        pending_ts: Timestamp,
        event: &RawKeyEvent,
    ) -> Resp {
        let elapsed_us = event.timestamp.saturating_sub(pending_ts);

        // 親指面で到着文字キーの候補を取得し閾値を調整
        let pending_face = Face::from_thumb_bool(pending_is_left);
        let candidate =
            self.lookup_face(event.scan_code, event.vk_code, self.get_face(pending_face));
        let threshold = candidate
            .as_ref()
            .and_then(|(_, kana)| *kana)
            .map_or(self.threshold_us, |ch| self.adjusted_threshold_us(ch));

        if elapsed_us < threshold {
            if let Some((action, kana)) = candidate {
                // 保留=親指, 到着=文字 → 同時打鍵
                if pending_is_left {
                    self.left_thumb_down = Some(pending_ts);
                } else {
                    self.right_thumb_down = Some(pending_ts);
                }
                self.active_keys.insert(event.scan_code, action.clone());
                self.track_output(kana);
                return self.finalize(Outcome::Confirmed(vec![action]));
            }
        }

        // 時間超過 or 候補なし → 前の保留を単独確定 + 今回のキーを新規処理
        let prev_actions = self.resolve_pending_thumb_as_single(pending_vk);
        let new_result = self.handle_idle(event, KeyClass::Char);
        self.combine_prev_and_new(prev_actions, new_result)
    }

    /// PendingThumb + 親指キー → 前の保留を単独確定 + 今回のキーを新規処理
    fn handle_pending_thumb_thumb(
        &mut self,
        pending_vk: u16,
        event: &RawKeyEvent,
        key_class: KeyClass,
    ) -> Resp {
        let prev_actions = self.resolve_pending_thumb_as_single(pending_vk);
        let new_result = self.handle_idle(event, key_class);
        self.combine_prev_and_new(prev_actions, new_result)
    }

    /// 前の保留のアクションと今回の結果を結合する
    fn combine_prev_and_new(&self, prev_actions: Vec<KeyAction>, new_result: Resp) -> Resp {
        if new_result.consumed {
            if new_result.actions.is_empty() {
                // new_result is Pending (consume with no actions but has timer set)
                if prev_actions.is_empty() {
                    new_result
                } else {
                    // Emit prev_actions, keep the timer commands from new_result
                    let mut r = Response::emit(prev_actions);
                    r.timers = new_result.timers;
                    r
                }
            } else {
                // new_result is Emit — combine actions
                let mut all_actions = prev_actions;
                all_actions.extend(new_result.actions);
                let mut r = Response::emit(all_actions);
                r.timers = new_result.timers;
                r
            }
        } else {
            // new_result is PassThrough
            if prev_actions.is_empty() {
                new_result
            } else {
                self.finalize(Outcome::Confirmed(prev_actions))
            }
        }
    }

    /// 保留中の文字キーを単独打鍵として解決し、アクション列を返す
    fn resolve_pending_char_as_single(&mut self, scan_code: u32, vk_code: u16) -> Vec<KeyAction> {
        if let Some((action, kana)) =
            self.lookup_face(scan_code, vk_code, self.get_face(Face::Normal))
        {
            self.active_keys.insert(scan_code, action.clone());
            self.track_output(kana);
            vec![action]
        } else {
            self.active_keys.insert(scan_code, KeyAction::Key(vk_code));
            vec![KeyAction::Key(vk_code)]
        }
    }

    /// 保留中の親指キーを単独打鍵として解決し、アクション列を返す
    fn resolve_pending_thumb_as_single(&mut self, vk_code: u16) -> Vec<KeyAction> {
        // Thumb keys use vk_code as their scan_code key since they don't have
        // standard scan codes in our map; use u32 from vk_code for consistency.
        self.active_keys
            .insert(u32::from(vk_code), KeyAction::Key(vk_code));
        vec![KeyAction::Key(vk_code)]
    }

    /// `PendingCharThumb` 状態で新しいキーが到着した場合の 3 鍵仲裁処理
    #[allow(clippy::too_many_arguments)]
    fn resolve_pending_char_thumb(
        &mut self,
        event: &RawKeyEvent,
        key_class: KeyClass,
        char_scan: u32,
        char_vk: u16,
        char_timestamp: Timestamp,
        thumb_is_left: bool,
        thumb_timestamp: Timestamp,
    ) -> Resp {
        if !key_class.is_thumb() {
            // char2 到着 → d1/d2 比較で 3 鍵仲裁
            let d1 = thumb_timestamp.saturating_sub(char_timestamp);
            let d2 = event.timestamp.saturating_sub(thumb_timestamp);

            if d1 < d2 {
                // char1+thumb = 同時打鍵、char2 は新規処理
                let prev_actions = self.resolve_char_thumb_as_simultaneous(
                    char_scan,
                    char_vk,
                    thumb_is_left,
                    thumb_timestamp,
                );
                let new_result = self.handle_idle(event, KeyClass::Char);
                return self.combine_prev_and_new(prev_actions, new_result);
            }
            // d1 >= d2: char1 = 単独、char2+thumb = 同時打鍵
            let prev_actions = self.resolve_pending_char_as_single(char_scan, char_vk);
            let thumb_face = Face::from_thumb_bool(thumb_is_left);
            if let Some((action, kana)) =
                self.lookup_face(event.scan_code, event.vk_code, self.get_face(thumb_face))
            {
                if thumb_is_left {
                    self.left_thumb_down = Some(thumb_timestamp);
                } else {
                    self.right_thumb_down = Some(thumb_timestamp);
                }
                self.active_keys.insert(event.scan_code, action.clone());
                self.track_output(kana);
                let mut all_actions = prev_actions;
                all_actions.push(action);
                return self.finalize(Outcome::Confirmed(all_actions));
            }
            // 親指面に char2 の定義がない場合はそれぞれ単独確定
            let new_result = self.handle_idle(event, KeyClass::Char);
            return self.combine_prev_and_new(prev_actions, new_result);
        }
        // 親指キーが来た場合は char1+thumb を同時打鍵として確定し、
        // 新しい親指キーを保留にする
        let prev_actions = self.resolve_char_thumb_as_simultaneous(
            char_scan,
            char_vk,
            thumb_is_left,
            thumb_timestamp,
        );
        let new_result = self.handle_idle(event, key_class);
        self.combine_prev_and_new(prev_actions, new_result)
    }

    fn on_key_up(&mut self, event: &RawKeyEvent) -> Resp {
        // 親指キーのリリース追跡
        let key_class = self.classify(event);
        if key_class.is_left_thumb() {
            self.left_thumb_down = None;
        } else if key_class == KeyClass::RightThumb {
            self.right_thumb_down = None;
        }

        // PendingCharThumb 状態での KeyUp 処理
        if let NicolaState::PendingCharThumb {
            char_vk, thumb_vk, ..
        } = self.state
        {
            if event.vk_code == char_vk || event.vk_code == thumb_vk {
                return self.handle_key_up_pending_char_thumb(event);
            }
        }

        // 保留中のキーが離された場合、保留を単独確定
        if self.is_pending_key(event.vk_code) {
            return self.handle_key_up_pending(event);
        }

        // active_keys から対応する注入済みキーを探してリリース
        self.handle_key_up_active(event)
    }

    /// 保留中キーの vk_code と一致するか判定する
    fn is_pending_key(&self, vk_code: u16) -> bool {
        match &self.state {
            NicolaState::PendingChar { vk_code: vk, .. }
            | NicolaState::PendingThumb { vk_code: vk, .. } => *vk == vk_code,
            NicolaState::Idle | NicolaState::PendingCharThumb { .. } => false,
        }
    }

    /// PendingCharThumb 状態で char1 または thumb が離された場合の処理
    fn handle_key_up_pending_char_thumb(&mut self, event: &RawKeyEvent) -> Resp {
        let NicolaState::PendingCharThumb {
            char_scan,
            char_vk,
            thumb_is_left,
            thumb_timestamp,
            ..
        } = self.state
        else {
            unreachable!()
        };
        self.state = NicolaState::Idle;
        let mut actions = self.resolve_char_thumb_as_simultaneous(
            char_scan,
            char_vk,
            thumb_is_left,
            thumb_timestamp,
        );
        if event.vk_code == char_vk {
            // char1 が離された: Char は KeyUp 不要だが Key なら KeyUp 追加
            if let Some(KeyAction::Key(vk)) = self.active_keys.remove(&event.scan_code) {
                actions.push(KeyAction::KeyUp(vk));
            }
        }
        self.finalize(Outcome::Confirmed(actions))
    }

    /// 保留中のキーが離された場合、保留を単独確定して KeyUp を処理する
    fn handle_key_up_pending(&mut self, event: &RawKeyEvent) -> Resp {
        let old_state = std::mem::replace(&mut self.state, NicolaState::Idle);
        let actions = match old_state {
            NicolaState::PendingChar {
                scan_code, vk_code, ..
            } => self.resolve_pending_char_as_single(scan_code, vk_code),
            NicolaState::PendingThumb { vk_code, .. } => {
                self.resolve_pending_thumb_as_single(vk_code)
            }
            NicolaState::Idle | NicolaState::PendingCharThumb { .. } => unreachable!(),
        };
        let mut result = actions;
        if let Some(KeyAction::Key(vk)) = self.active_keys.remove(&event.scan_code) {
            result.push(KeyAction::KeyUp(vk));
        }
        // Unicode 文字 (Char) は Down+Up 一括送信済みなので KeyUp 追加不要
        self.finalize(Outcome::Confirmed(result))
    }

    /// active_keys から対応する注入済みキーを探してリリースする
    fn handle_key_up_active(&mut self, event: &RawKeyEvent) -> Resp {
        if let Some(action) = self.active_keys.remove(&event.scan_code) {
            return match action {
                // Unicode 文字やローマ字列の場合、KeyUp は不要（押下時に入力完了）
                KeyAction::Char(_) | KeyAction::Romaji(_) => {
                    self.finalize(Outcome::confirmed_one(KeyAction::Suppress))
                }
                KeyAction::Key(vk) => self.finalize(Outcome::confirmed_one(KeyAction::KeyUp(vk))),
                _ => self.finalize(Outcome::PassThrough),
            };
        }
        self.finalize(Outcome::PassThrough)
    }

    /// PendingChar タイムアウト：文字キーを単独打鍵として確定する
    fn timeout_pending_char(&mut self, scan_code: u32, vk_code: u16) -> Resp {
        if let Some((action, kana)) =
            self.lookup_face(scan_code, vk_code, self.get_face(Face::Normal))
        {
            self.active_keys.insert(scan_code, action.clone());
            self.track_output(kana);
            Response::emit(vec![action]).with_kill_timer(TIMER_PENDING)
        } else {
            // 配列定義に含まれないキーはそのまま通す
            self.active_keys.insert(scan_code, KeyAction::Key(vk_code));
            Response::emit(vec![KeyAction::Key(vk_code)]).with_kill_timer(TIMER_PENDING)
        }
    }

    /// PendingThumb タイムアウト：親指キーを単独打鍵として確定する
    fn timeout_pending_thumb(&mut self, vk_code: u16) -> Resp {
        self.active_keys
            .insert(u32::from(vk_code), KeyAction::Key(vk_code));
        Response::emit(vec![KeyAction::Key(vk_code)]).with_kill_timer(TIMER_PENDING)
    }

    /// PendingCharThumb タイムアウト：char1+thumb を同時打鍵として確定する
    fn timeout_pending_char_thumb(
        &mut self,
        char_scan: u32,
        char_vk: u16,
        thumb_is_left: bool,
        thumb_timestamp: Timestamp,
    ) -> Resp {
        let actions = self.resolve_char_thumb_as_simultaneous(
            char_scan,
            char_vk,
            thumb_is_left,
            thumb_timestamp,
        );
        Response::emit(actions).with_kill_timer(TIMER_PENDING)
    }

    /// キーイベントを分類する
    const fn classify(&self, event: &RawKeyEvent) -> KeyClass {
        if event.vk_code == self.left_thumb_vk {
            return KeyClass::LeftThumb;
        }
        if event.vk_code == self.right_thumb_vk {
            return KeyClass::RightThumb;
        }
        if Self::is_passthrough_vk(event.vk_code) {
            return KeyClass::Passthrough;
        }
        KeyClass::Char
    }

    /// 現在押下中の親指キーに対応するシフト面を返す
    const fn active_thumb_face(&self) -> Option<Face> {
        if self.left_thumb_down.is_some() {
            Some(Face::LeftThumb)
        } else if self.right_thumb_down.is_some() {
            Some(Face::RightThumb)
        } else {
            None
        }
    }

    /// いずれかの配列面に定義されているキーかどうか
    fn is_layout_key(&self, _vk_code: u16, scan_code: u32) -> bool {
        let Some(pos) = scan_to_pos(scan_code) else {
            return false;
        };
        self.get_face(Face::Normal).contains_key(&pos)
            || self.get_face(Face::LeftThumb).contains_key(&pos)
            || self.get_face(Face::RightThumb).contains_key(&pos)
            || self.get_face(Face::Shift).contains_key(&pos)
    }

    /// 変換対象外のキー（修飾キー、ファンクションキー等）を判定する
    const fn is_passthrough_vk(vk_code: u16) -> bool {
        matches!(
            vk_code,
            // 修飾キー
            0x10 | 0x11 | 0x12 |  // Shift, Ctrl, Alt
            0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA4 | 0xA5 |  // L/R Shift, Ctrl, Alt
            // Windows キー
            0x5B | 0x5C |
            // Caps Lock
            0x14 |
            // Esc
            0x1B |
            // ファンクションキー (F1-F24)
            0x70..=0x87 |
            // ナビゲーション
            0x21..=0x28 |  // PageUp, PageDown, End, Home, Arrow keys
            // Insert, Delete
            0x2D | 0x2E |
            // Num Lock, Scroll Lock
            0x90 | 0x91 |
            // Print Screen, Pause
            0x2C | 0x13
        )
    }
}

impl TimedStateMachine for Engine {
    type Event = RawKeyEvent;
    type Action = KeyAction;
    type TimerId = usize;

    fn on_event(&mut self, event: RawKeyEvent) -> Resp {
        if !self.enabled {
            return Response::pass_through();
        }

        // 修飾キー（Ctrl / Alt）の押下状態を追跡
        self.modifiers.update(&event);

        match event.event_type {
            KeyEventType::KeyDown | KeyEventType::SysKeyDown => self.on_key_down(&event),
            KeyEventType::KeyUp | KeyEventType::SysKeyUp => self.on_key_up(&event),
        }
    }

    fn on_timeout(&mut self, _timer_id: usize) -> Resp {
        let old_state = std::mem::replace(&mut self.state, NicolaState::Idle);

        match old_state {
            NicolaState::Idle => Response::consume().with_kill_timer(TIMER_PENDING),
            NicolaState::PendingChar {
                scan_code, vk_code, ..
            } => self.timeout_pending_char(scan_code, vk_code),
            NicolaState::PendingThumb { vk_code, .. } => self.timeout_pending_thumb(vk_code),
            NicolaState::PendingCharThumb {
                char_scan,
                char_vk,
                thumb_is_left,
                thumb_timestamp,
                ..
            } => {
                self.timeout_pending_char_thumb(char_scan, char_vk, thumb_is_left, thumb_timestamp)
            }
        }
    }
}

#[cfg(test)]
mod tests;
