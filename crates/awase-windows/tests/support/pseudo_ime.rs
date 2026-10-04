//! 擬似 IME: 開閉・変換モード・入力中の段階を持つ小さな状態機械。
//!
//! # 真値の出所
//!
//! キー→結果の表は **`tools/e2e/ime_key_matrix/grid-tables/atok.json`**（GJI の ATOK プリセットを
//! CI 実機で `--grid` 学習した生データ。awase を完全にバイパスした注入の結果）をそのまま読む。
//! awase の予測器が引く `state/key_effect_table.rs::ATOK` は、この生データから
//! `gen_key_effect_table.py` が**非決定セルを除外して**生成した部分集合である
//! （例: `on-c10-typing|esc` は独立 walk で「破棄」と「入力中のまま」に割れたため予測表から除外、
//! `gen_key_effect_table.py:95`）。擬似 IME は除外されたセルも生データの値（格子での多数派）で動くので、
//! 「予測器には答えが無いが、実 IME は何かをする」状況（BUG-162 の起点）を再現できる。
//!
//! # 生データに無い部分（仮定。実機で全ては確かめていない）
//!
//! - **押下後の入力中の段階**: 生データが持つのは「開閉/conv/行方（保持・破棄・確定）」だけ。
//!   行方が「保持」のときの段階は、キー種別の規則で決める: Space→変換中(Space)、変換→変換中(変換)、
//!   無変換→変換中(無変換)、Esc→入力中（変換中から Esc で読みへ戻る、の仮定）、それ以外→直前の段階のまま。
//!   `crates/awase-keymap-learn/src/sample_models.rs` の ATOK 風モデルと同じく、撤去ブランチの
//!   `key_track` 規則を元にした仮定である。
//! - **文字キー**（表に無い）: 開いていれば入力中になる（変換中なら確定して新しい入力中）。閉なら何もしない。
//! - 生データに無い状態（例: ATOK の `on-c19-conv-muhenkan`）からの押下は **panic** する。
//!   シナリオは実測のある範囲だけを通ること（推測で埋めない）。

use std::collections::HashMap;
use std::path::PathBuf;

/// 入力中の段階（擬似 IME の真値）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrueStage {
    None,
    Typing,
    ConvSpace,
    ConvHenkan,
    ConvMuhenkan,
}

impl TrueStage {
    const fn grid_name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Typing => "typing",
            Self::ConvSpace => "conv-space",
            Self::ConvHenkan => "conv-henkan",
            Self::ConvMuhenkan => "conv-muhenkan",
        }
    }
}

/// 変換モード（生データが持つ2値。ROMAN ビット込みの conv 生値）。
pub const CONV_HIRAGANA: u32 = 0x19;
pub const CONV_ALNUM: u32 = 0x10;

/// 擬似 IME の真の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrueState {
    pub open: bool,
    /// conv の生値（`CONV_HIRAGANA` / `CONV_ALNUM`）。閉でも保持する（生データの `OFF/0x19`）。
    pub conv: u32,
    pub stage: TrueStage,
}

impl TrueState {
    /// かな入力系（NATIVE ビットあり）か。
    #[must_use]
    pub const fn is_native(&self) -> bool {
        self.conv & 0x01 != 0
    }
}

/// 生データの1セルの結果。
#[derive(Debug, Clone, Copy)]
struct GridOutcome {
    open: bool,
    conv: u32,
    disp: Disp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disp {
    None,
    Kept,
    Discarded,
    Committed,
}

/// 生データのキー名（`atok.json` のキー `state|key` の `key` 部分）。表に無いキーは `None`。
const fn grid_key_name(vk: u16) -> Option<&'static str> {
    Some(match vk {
        0x08 => "bs",
        0xF0 => "eisu",
        0x0D => "enter",
        0x1B => "esc",
        0xF3 | 0xF4 => "hankaku-zenkaku",
        0x1C => "henkan",
        0xF2 => "hiragana",
        0x1A => "ime-off",
        0x16 => "ime-on",
        0x19 => "kanji",
        0xF1 => "katakana",
        0x1D => "muhenkan",
        0x20 => "space",
        _ => return None,
    })
}

/// 文字を入力するキー（英数字・記号）か（予測器の `is_char_vk` と同じ範囲）。
const fn is_char_vk(vk: u16) -> bool {
    matches!(vk, 0x30..=0x39 | 0x41..=0x5A | 0xBA..=0xC0 | 0xDB..=0xDF)
}

/// 1回の押下の真の結果（検査用）。
#[derive(Debug, Clone, Copy)]
pub struct PressOutcome {
    pub before: TrueState,
    pub after: TrueState,
}

/// 擬似 IME。
#[derive(Debug)]
pub struct PseudoIme {
    state: TrueState,
    table: HashMap<String, GridOutcome>,
    /// awase からの書き込み（`VK_IME_ON`/`OFF` 相当）を無視する（書き込みが効かないアプリの模擬）。
    writes_blocked: bool,
    /// 受け取った書き込みの記録（`(open, 効いたか)`）。
    pub writes_received: Vec<(bool, bool)>,
    /// 仮想時計（ms）。`Harness::advance_ms` が進める。
    clock_ms: u64,
    /// クセ Q3（読み戻し遅延）: 状態が変わってから `Some(n)` ms の間、`read_state()` は変更前を返す。
    readback_lag_ms: Option<u64>,
    /// 直近の状態変更の `(時刻ms, 変更前の状態)`。
    last_change: Option<(u64, TrueState)>,
}

impl PseudoIme {
    /// ATOK プリセットの生データ（`grid-tables/atok.json`）を真値にした擬似 IME。
    #[must_use]
    pub fn atok(initial: TrueState) -> Self {
        let path = grid_path("atok.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} が読めない: {e}", path.display()));
        let raw: HashMap<String, HashMap<String, u32>> = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{} の JSON が読めない: {e}", path.display()));
        let mut table = HashMap::new();
        for (cell, outcomes) in raw {
            // 格子での多数派（件数が最大、同数なら文字列順で先）を採る。ATOK の第3版は全セルが1種類。
            let mut v: Vec<(&String, &u32)> = outcomes.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let Some((best, _)) = v.first() else { continue };
            table.insert(cell, parse_outcome(best));
        }
        Self {
            state: initial,
            table,
            writes_blocked: false,
            writes_received: Vec::new(),
            clock_ms: 0,
            readback_lag_ms: None,
            last_change: None,
        }
    }

    #[must_use]
    pub const fn state(&self) -> TrueState {
        self.state
    }

    pub fn set_writes_blocked(&mut self, blocked: bool) {
        self.writes_blocked = blocked;
    }

    /// 仮想時計を進める（`Harness::advance_ms` から呼ぶ）。
    pub fn advance(&mut self, ms: u64) {
        self.clock_ms += ms;
    }

    /// クセ Q3: 状態が変わってから `ms` の間、読み取り（`read_state`）が変更前の状態を返す。
    ///
    /// 実測（`tuning.rs` の `MODE_KEY_PASS_REREAD_MS` の注記、`mode_key_pass_timeline.py`）:
    /// ATOK プリセットのモードキー通過後、IMM 再読に変化が現れるまで min21 / median33 / p90 33 /
    /// max62ms。11ms 後の 1 回は古い状態を読んだ。真の状態（`state`）は変わらず、観測だけが遅れる。
    pub fn set_readback_lag_ms(&mut self, ms: Option<u64>) {
        self.readback_lag_ms = ms;
    }

    /// 観測（IMM 再読・poll 等）が読む状態。遅延中は変更前の状態。真の状態は `state()`。
    #[must_use]
    pub fn read_state(&self) -> TrueState {
        match (self.readback_lag_ms, self.last_change) {
            (Some(lag), Some((at, before))) if self.clock_ms.saturating_sub(at) < lag => before,
            _ => self.state,
        }
    }

    /// 状態を更新し、変わったなら読み戻し遅延の起点を記録する。
    fn set_state(&mut self, new: TrueState) {
        if new != self.state {
            self.last_change = Some((self.clock_ms, self.state));
        }
        self.state = new;
    }

    /// 物理キー1回の押下（生キーが IME に届いた）。
    pub fn press(&mut self, vk: u16) -> PressOutcome {
        let before = self.state;
        let after = match grid_key_name(vk) {
            Some(key) => {
                let cell = format!(
                    "{}-c{:02x}-{}|{key}",
                    if before.open { "on" } else { "off" },
                    before.conv,
                    if before.open {
                        before.stage.grid_name()
                    } else {
                        "none"
                    },
                );
                let o = self.table.get(&cell).copied().unwrap_or_else(|| {
                    panic!(
                        "擬似 IME の真値が無い: `{cell}`（grid-tables/atok.json に実測が無い状態・キー。\
                         シナリオを実測のある範囲に収めること）"
                    )
                });
                TrueState {
                    open: o.open,
                    conv: o.conv,
                    stage: next_stage(before.stage, vk, o),
                }
            }
            None if is_char_vk(vk) && before.open => TrueState {
                stage: TrueStage::Typing,
                ..before
            },
            None => before,
        };
        self.set_state(after);
        PressOutcome { before, after }
    }

    /// awase の見ていない経路での開閉の変更（言語バーのマウス操作等）。入力中は捨てる。
    pub fn external_set_open(&mut self, open: bool) {
        self.set_state(TrueState {
            open,
            stage: TrueStage::None,
            ..self.state
        });
    }

    /// awase からの開閉の書き込み。効いたら `true`。
    pub fn write_open(&mut self, open: bool) -> bool {
        let applied = !self.writes_blocked;
        if applied {
            // 開閉だけを変える（ATOK の VK_IME_OFF は入力中を破棄するが、ここでは書き込みの有無だけを見る）。
            self.set_state(TrueState {
                open,
                stage: TrueStage::None,
                ..self.state
            });
        }
        self.writes_received.push((open, applied));
        applied
    }
}

fn grid_path(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/e2e/ime_key_matrix/grid-tables")
        .join(file)
}

/// `"ON/0x19/保持"` 形式を読む。
fn parse_outcome(s: &str) -> GridOutcome {
    let mut parts = s.split('/');
    let open = match parts.next() {
        Some("ON") => true,
        Some("OFF") => false,
        other => panic!("開閉が読めない: {other:?} in {s}"),
    };
    let conv = parts
        .next()
        .and_then(|c| u32::from_str_radix(c.trim_start_matches("0x"), 16).ok())
        .unwrap_or_else(|| panic!("conv が読めない: {s}"));
    let disp = match parts.next() {
        None => Disp::None,
        Some("保持") => Disp::Kept,
        Some("破棄") => Disp::Discarded,
        Some("確定") => Disp::Committed,
        Some(other) => panic!("行方が読めない: {other} in {s}"),
    };
    GridOutcome { open, conv, disp }
}

/// 押下後の段階（モジュール doc の仮定）。
const fn next_stage(prev: TrueStage, vk: u16, o: GridOutcome) -> TrueStage {
    if !o.open {
        return TrueStage::None;
    }
    match o.disp {
        Disp::None | Disp::Discarded | Disp::Committed => TrueStage::None,
        Disp::Kept => match vk {
            0x20 => TrueStage::ConvSpace,
            0x1C => TrueStage::ConvHenkan,
            0x1D => TrueStage::ConvMuhenkan,
            0x1B => TrueStage::Typing,
            _ => prev,
        },
    }
}
