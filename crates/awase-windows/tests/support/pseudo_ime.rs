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
//!
//! # 遷移表（ADR-277 段階 1）
//!
//! 遷移は状態を持たない `TransitionTable::step(状態, ImeInput, QuirkSwitches)` 1 つに集めてある。中身は
//! 格子（実測）+ `KEPT_STAGE_RULES`（上の段階の仮定を表にしたもの）+ 文字キー・書き込み・外部変更の規則。
//! `PseudoIme`（閉ループ）・`TransitionTable::reachable`（全列挙、`pseudo_ime_exhaustive.rs`）・ランダム探索が
//! 同じ表を引く。時刻に依るクセ（Q3 の読み戻し遅延）だけは `PseudoIme` の側に置く。

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;

/// クセの目録の1件。実 IME・アプリが「仕様に反して/表に無い形で」見せる挙動のうち、閉ループに影響するもの。
/// 擬似 IME が模しているか(`modeled`)と、根拠・CI での再現手段を残す。新しいクセはここへ足す(最小スキーマ: when/effect/evidence/ci)。
#[derive(Debug, Clone, Copy)]
pub struct Quirk {
    pub id: &'static str,
    /// どの IME・アプリ・状態で起きるか。
    pub when: &'static str,
    /// 何が起きるか。
    pub effect: &'static str,
    /// 実測の根拠(ドキュメント・数値)。
    pub evidence: &'static str,
    /// CI 実機での再現手段(構成名・ワークフロー)。
    pub ci: &'static str,
    /// 擬似 IME が模しているか(模している場合のスイッチ名)。
    pub modeled: Option<&'static str>,
}

/// 目録。Q1・Q2・Q5 は 2026-10-04 時点でリポジトリに記述が無く(別セッションの会話内のみ)、ここへは載せていない。
/// 番号を振らない項目(`Q-` 始まり)は 2026-10-04 に docs の一次資料から洗い直して足した(数値・環境は原文で確認済み)。
/// 証拠不足で載せていない候補: MS-IME の IMC write 着地遅延 ~250ms(BUG-015 追補6、撤回済み・1件)、
/// Shift 単独タップ誤判定 478ms(BUG-015、観測1件)、OFF→ON 直後の cold 窓(BUG-013、データ点2つ・測定保留)、
/// MS-IME 本体 CI の ImmCross ON write 失敗(ADR-186、CI の癖か欠陥か未切り分け)。
pub const QUIRKS: &[Quirk] = &[
    Quirk {
        id: "Q3",
        when: "ATOK プリセットのモードキー通過後(GJI)",
        effect: "実 IME の変化が IMM 再読に現れるまで 21〜62ms かかり、その間の読み取りは古い状態を返す",
        evidence: "tuning.rs MODE_KEY_PASS_REREAD_MS の注記、tools/e2e/ime_key_matrix/mode_key_pass_timeline.py(min21/median33/max62ms)",
        ci: "e2e-ime の mode-key-pass 系(timeline)",
        modeled: Some("set_readback_lag_ms"),
    },
    Quirk {
        id: "Q4",
        when: "実 Chrome(TSF 窓)× GJI で、他プロセスが 0xF3/0x1A を注入して IME を閉じたとき(言語バー・マウス経由は未測定)",
        effect: "IME は閉じる(IMC_GETOPENSTATUS 1→0)が awase の観測は 0 件で、belief が開のまま残る",
        evidence: "BUG-172(2026-09-29、CI 10/10 再現、ObserverPoll=0・Imm32Unavailable=39、メモ帳は影響なし)。ADR-205 で修正済みで v2.0.0 に入り実機確認済み。runtime 側(観測経路)の現象で、擬似 IME の状態機械からは表現できない",
        ci: "cal-driftrec 系・ADR-205 の外部クローズ検証(修正後は [external-change]×10、observed 0→10)",
        modeled: Some("Setup::with_external_close_watch(閉ループ側の切替。無効=Q4の症状、有効=ADR-205の追随。クセそのものは擬似IMEの状態機械ではなく観測経路なので、観測側の状態機械ExternalChangeWatchを本物で呼ぶ)"),
    },
    Quirk {
        id: "Q6",
        when: "MS-IME 本体 × 未確定の文字(composition)が残っている間",
        effect: "VK_IME_OFF / OFF 書き込みが IME を閉じず conv だけが半角英数(0x19→0x10)になる。入力中の文字は残る",
        evidence: "BUG-185(対応しない決定 2026-10-04)、docs/tasks/msime-chrome-off-rca-2026-10-04.md R1〜R5(n=10、composition 有り 0/10・無し 10/10 閉じる)",
        ci: "sc-offrca-*(chrome_probe --offrca=1a:typed_nc)",
        modeled: Some("set_off_ignored_while_composing(書き込み経路のみ。キー押下は ATOK の格子のまま)"),
    },
    Quirk {
        id: "Q-ext-off-chrome-gji",
        when: "実 Chrome × GJI で外部から注入した OFF(Q4 と同一事象の測定側。修正前の実測を残す)",
        effect: "0xF3・0x1A とも 3 秒後も閉じたまま、awase の観測 0 件で Engine は ON のまま `kiu` が出る",
        evidence: "BUG-172.md:65-72(run 36540419485、1 台の CI 実機、各 10 試行)。補償通知(compartment)は 2〜5ms(サンプル数の記載なし)",
        ci: "BUG-172 の外部注入構成",
        modeled: None,
    },
    Quirk {
        id: "Q-key-latency-gji",
        when: "GJI で物理キー押下から `open` 遷移が観測されるまで",
        effect: "多くは 250〜400ms、最大 2.3 秒。Q3(モードキー通過後の IMM 再読 21〜62ms)とは観測条件が違い、62ms では収まらない",
        evidence: "ADR-176:84-86(n=9: 247/277/321/341/362/391/529/1687/2295ms)。SendMessageTimeoutW の elapsed は全サンプル 20ms 未満",
        ci: "ADR-176 の較正(日付は原文に無い)",
        modeled: None,
    },
    Quirk {
        id: "Q-imm-probe-bimodal",
        when: "IMM probe(SendMessageTimeout 50ms)。MS-IME 本体の CI",
        effect: "応答時間は 50ms 境界の二峰性(成功は最大 50ms・時間切れは最小 50ms)。時間切れを「IMM 不可」と誤学習した",
        evidence: "BUG-158.md:26-28(CI、n=7183、p99=59.5ms)。実際に IMM が使えないアプリが時間切れか即拒否かは判別不能(未確認)",
        ci: "MS-IME 本体の CI 構成",
        modeled: None,
    },
];

/// 真値にする生データの格子（`tools/e2e/ime_key_matrix/grid-tables/`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grid {
    /// GJI の ATOK プリセット。
    Atok,
    /// GJI の MS-IME プリセット。
    GjiMsime,
    /// Microsoft IME 本体。
    MsimeNative,
}

impl Grid {
    /// 全格子（全列挙で使う）。
    pub const ALL: [Self; 3] = [Self::Atok, Self::GjiMsime, Self::MsimeNative];

    const fn file(self) -> &'static str {
        match self {
            Self::Atok => "atok.json",
            Self::GjiMsime => "msime.json",
            Self::MsimeNative => "msime-native.json",
        }
    }

    /// golden に書く短い名前（JSON のファイル名から拡張子を除いたもの）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Atok => "atok",
            Self::GjiMsime => "msime",
            Self::MsimeNative => "msime-native",
        }
    }
}

/// 入力中の段階（擬似 IME の真値）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TrueStage {
    None,
    Typing,
    ConvSpace,
    ConvHenkan,
    ConvMuhenkan,
}

impl TrueStage {
    /// 全段階（格子の状態名を読み戻すときに使う）。
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::Typing,
        Self::ConvSpace,
        Self::ConvHenkan,
        Self::ConvMuhenkan,
    ];

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
#[must_use]
pub const fn grid_key_name(vk: u16) -> Option<&'static str> {
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

/// 格子の状態名（`on-c19-typing`・`on-c1B-none` 形式）。閉のときの段階は常に `none`。
///
/// conv の 16 進は**大文字**（格子の JSON が `on-c1B-none` と書く）。2026-10-22 まで小文字（`{:02x}`）で引いていたため、
/// MS-IME の 2 格子で conv=0x1B の実測セル（`msime.json`・`msime-native.json` の `c1B` の状態、各 13 キー）が全て
/// 「実測なし」になり、`knows` が偽を返してランダム探索が黙って飛ばしていた（`pseudo_ime_exhaustive.rs` の到達性の
/// 全列挙で発見。ADR-277）。
#[must_use]
pub fn state_name(s: TrueState) -> String {
    format!(
        "{}-c{:02X}-{}",
        if s.open { "on" } else { "off" },
        s.conv,
        if s.open { s.stage.grid_name() } else { "none" },
    )
}

/// 格子のセル名（`on-c19-typing|esc` 形式）。
fn cell_name(before: TrueState, key: &str) -> String {
    format!("{}|{key}", state_name(before))
}

/// 格子の状態名を読み戻す（`state_name` の逆）。擬似 IME が表せない状態（`on-c19-typing-prev-esc` のように
/// 直前のキーで分けた状態）は `None`。
#[must_use]
pub fn parse_state_name(name: &str) -> Option<TrueState> {
    let (open, rest) = if let Some(r) = name.strip_prefix("on-c") {
        (true, r)
    } else {
        (false, name.strip_prefix("off-c")?)
    };
    let (conv, stage) = rest.split_once('-')?;
    let conv = u32::from_str_radix(conv, 16).ok()?;
    let stage = TrueStage::ALL
        .into_iter()
        .find(|s| s.grid_name() == stage)?;
    Some(TrueState { open, conv, stage })
}

/// 文字を入力するキー（英数字・記号）か（予測器の `is_char_vk` と同じ範囲）。
#[must_use]
pub const fn is_char_vk(vk: u16) -> bool {
    matches!(vk, 0x30..=0x39 | 0x41..=0x5A | 0xBA..=0xC0 | 0xDB..=0xDF)
}

// ---------------------------------------------------------------------------
// 宣言的な遷移表（ADR-277 段階 1）
// ---------------------------------------------------------------------------

/// 行方が「保持」のときの押下後の段階（**仮定**。生データには段階が無い。モジュール doc 参照）。
/// 表に無いキーは直前の段階のまま。全列挙・ファズ・閉ループが同じこの表を引く。
pub const KEPT_STAGE_RULES: &[(u16, TrueStage)] = &[
    (0x20, TrueStage::ConvSpace),    // Space → 変換中(Space)
    (0x1C, TrueStage::ConvHenkan),   // 変換 → 変換中(変換)
    (0x1D, TrueStage::ConvMuhenkan), // 無変換 → 変換中(無変換)
    (0x1B, TrueStage::Typing),       // Esc → 入力中（BUG-205、run 39092296146 で実測確定）
];

/// 擬似 IME への入力（全列挙・ファズ・閉ループが共有するアルファベット）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImeInput {
    /// 物理キー1回の押下（生キーが IME に届いた）。
    Press(u16),
    /// awase からの開閉の書き込み（`VK_IME_ON`/`OFF`・`IMC_SETOPENSTATUS` を区別しない単純化）。
    Write(bool),
    /// awase の見ていない経路での開閉の変更（言語バーのマウス操作等）。入力中は捨てる。
    External(bool),
}

/// 状態遷移に効くクセのスイッチ（観測側のクセ Q3〈読み戻し遅延〉は時刻に依るので `PseudoIme` 側に置く）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuirkSwitches {
    /// awase からの書き込みを無視する（書き込みが効かないアプリの模擬）。
    pub writes_blocked: bool,
    /// クセ Q6: 入力中の OFF 書き込みが閉じず conv だけ半角英数にする（BUG-185、MS-IME）。
    pub off_ignored_while_composing: bool,
}

/// 1 回の遷移の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// 遷移先。`write_applied` は `ImeInput::Write` のときだけ `Some`（書き込みが効いたか）。
    Next {
        after: TrueState,
        write_applied: Option<bool>,
    },
    /// 生データに実測の無いセル（推測で埋めない）。値は格子のセル名。
    Unmeasured(String),
}

/// 到達性の全列挙の結果。
#[derive(Debug, Clone, Default)]
pub struct Reachability {
    /// 開始状態から到達しうる状態（開始状態を含む）。
    pub states: BTreeSet<TrueState>,
    /// 到達しうるのに実測の無いセル: 状態名 → キー名の集合。
    pub holes: BTreeMap<String, BTreeSet<&'static str>>,
}

/// 擬似 IME の遷移表: 格子（実測）+ `KEPT_STAGE_RULES`（仮定）+ 文字キー・書き込み・外部変更の規則。
/// 状態を持たない純粋な表で、`step` が唯一の遷移関数。`PseudoIme` もこれを呼ぶだけ。
#[derive(Debug, Clone)]
pub struct TransitionTable {
    grid: Grid,
    cells: HashMap<String, GridOutcome>,
}

impl TransitionTable {
    /// 指定した格子の生データを読む（セルごとに多数派。同数なら文字列順で先）。
    #[must_use]
    pub fn load(grid: Grid) -> Self {
        let path = grid_path(grid.file());
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} が読めない: {e}", path.display()));
        let raw: HashMap<String, HashMap<String, u32>> = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{} の JSON が読めない: {e}", path.display()));
        let mut cells = HashMap::new();
        for (cell, outcomes) in raw {
            // 格子での多数派（件数が最大、同数なら文字列順で先）を採る。ATOK の第3版は全セルが1種類。
            let mut v: Vec<(&String, &u32)> = outcomes.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let Some((best, _)) = v.first() else { continue };
            cells.insert(cell, parse_outcome(best));
        }
        Self { grid, cells }
    }

    #[must_use]
    pub const fn grid(&self) -> Grid {
        self.grid
    }

    /// 生データに実測のある状態名（`state|key` の `state` 部分）すべて。
    #[must_use]
    pub fn measured_state_names(&self) -> BTreeSet<String> {
        self.cells
            .keys()
            .filter_map(|c| c.split_once('|').map(|(s, _)| s.to_owned()))
            .collect()
    }

    /// 実測のある状態のうち、擬似 IME が表せるもの（全列挙の開始状態）。
    #[must_use]
    pub fn measured_states(&self) -> BTreeSet<TrueState> {
        self.measured_state_names()
            .iter()
            .filter_map(|n| parse_state_name(n))
            .collect()
    }

    /// 唯一の遷移関数。
    #[must_use]
    pub fn step(&self, before: TrueState, input: ImeInput, quirks: QuirkSwitches) -> Transition {
        match input {
            ImeInput::Press(vk) => match grid_key_name(vk) {
                Some(key) => {
                    let cell = cell_name(before, key);
                    match self.cells.get(&cell) {
                        Some(&o) => Transition::Next {
                            after: TrueState {
                                open: o.open,
                                conv: o.conv,
                                stage: next_stage(before.stage, vk, o),
                            },
                            write_applied: None,
                        },
                        None => Transition::Unmeasured(cell),
                    }
                }
                None if is_char_vk(vk) && before.open => Transition::Next {
                    after: TrueState {
                        stage: TrueStage::Typing,
                        ..before
                    },
                    write_applied: None,
                },
                None => Transition::Next {
                    after: before,
                    write_applied: None,
                },
            },
            ImeInput::Write(open) => {
                let after = if quirks.writes_blocked {
                    before
                } else if !open
                    && quirks.off_ignored_while_composing
                    && before.stage != TrueStage::None
                {
                    // クセ Q6: 閉じず conv だけ変わる。入力中の段階も残る。
                    TrueState {
                        conv: CONV_ALNUM,
                        ..before
                    }
                } else {
                    // 開閉だけを変える（ATOK の VK_IME_OFF は入力中を破棄するが、ここでは書き込みの有無だけを見る）。
                    TrueState {
                        open,
                        stage: TrueStage::None,
                        ..before
                    }
                };
                Transition::Next {
                    after,
                    write_applied: Some(!quirks.writes_blocked),
                }
            }
            ImeInput::External(open) => Transition::Next {
                after: TrueState {
                    open,
                    stage: TrueStage::None,
                    ..before
                },
                write_applied: None,
            },
        }
    }

    /// `starts` から `inputs` で到達しうる状態と、到達しうるのに実測の無いセルを全列挙する（幅優先）。
    #[must_use]
    pub fn reachable(
        &self,
        starts: impl IntoIterator<Item = TrueState>,
        inputs: &[ImeInput],
        quirks: QuirkSwitches,
    ) -> Reachability {
        let mut r = Reachability::default();
        let mut queue: VecDeque<TrueState> = VecDeque::new();
        for s in starts {
            if r.states.insert(s) {
                queue.push_back(s);
            }
        }
        while let Some(s) = queue.pop_front() {
            for &input in inputs {
                match self.step(s, input, quirks) {
                    Transition::Next { after, .. } => {
                        if r.states.insert(after) {
                            queue.push_back(after);
                        }
                    }
                    Transition::Unmeasured(cell) => {
                        let ImeInput::Press(vk) = input else {
                            unreachable!("実測なしは押下だけ")
                        };
                        let key = grid_key_name(vk).expect("実測なしは表のキーだけ");
                        let (state, _) = cell.split_once('|').expect("セル名");
                        r.holes.entry(state.to_owned()).or_default().insert(key);
                    }
                }
            }
        }
        r
    }
}

/// 1回の押下の真の結果（検査用）。
#[derive(Debug, Clone, Copy)]
pub struct PressOutcome {
    pub before: TrueState,
    pub after: TrueState,
}

/// 擬似 IME（遷移表 + 現在の状態 + 時刻に依るクセ）。
#[derive(Debug)]
pub struct PseudoIme {
    state: TrueState,
    table: TransitionTable,
    quirks: QuirkSwitches,
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
        Self::from_grid(Grid::Atok, initial)
    }

    /// 指定した格子の生データを真値にした擬似 IME。
    ///
    /// MS-IME の格子（`msime.json`=GJI の MS-IME プリセット、`msime-native.json`=MS-IME 本体）は conv の生値が
    /// `0x13`/`0x1B` 等も取り、状態に `conv-muhenkan` 等が増える。ATOK と同じく生データに無い状態からの押下は panic する。
    /// クセ Q6（MS-IME の入力中 OFF 無視）は格子に含まれないので、別途 `set_off_ignored_while_composing` で足す。
    #[must_use]
    pub fn from_grid(grid: Grid, initial: TrueState) -> Self {
        Self {
            state: initial,
            table: TransitionTable::load(grid),
            quirks: QuirkSwitches::default(),
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

    /// 遷移表（全列挙が同じ表を使うため）。
    #[must_use]
    pub const fn table(&self) -> &TransitionTable {
        &self.table
    }

    pub fn set_writes_blocked(&mut self, blocked: bool) {
        self.quirks.writes_blocked = blocked;
    }

    /// 【経緯 2026-10-04】スイッチと単体テスト(下の `q6_*`)はあるが、閉ループのシナリオ(`closed_loop_scenarios.rs`)からはまだ
    /// 呼ばれていない。BUG-185 は「対応しない」決定で、awase 側に検知したい挙動の修正が無く、シナリオにする根拠が無かったため。
    /// MS-IME の入力中 OFF 書き込みを扱う修正・回帰が出たときの足場として残している。消費者が現れないまま長く残るなら削ってよい。
    /// クセ Q6: 入力中の OFF 書き込み(`write_open(false)`)が IME を閉じず、conv だけ半角英数にする(BUG-185、MS-IME)。
    /// 書き込み自体は「受理」される(`write_open` は true を返す)が、開閉は変わらない。入力中でなければ通常どおり閉じる。
    pub fn set_off_ignored_while_composing(&mut self, on: bool) {
        self.quirks.off_ignored_while_composing = on;
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
    ///
    /// 【経緯 2026-10-04】これを使う現在のシナリオ(Q3 の3本)が確かめるのは「明示意図が無ければ古い読みで drift 補正が書かない」だけで、
    /// 遅れの値(33ms でも 362ms でも 2295ms でも。Q-key-latency-gji の実測、ADR-176 n=9)を変えても通る分岐は同じ。
    /// 値だけ変えたシナリオは新しい回帰検知にならないため足さなかった(PR #477 を閉じた)。遅れが意味を持つのは
    /// 「明示意図がある状態で古い読みが来る」(BUG-162/163 系)シナリオを書くとき。これはその足場。
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

    /// 遷移表を 1 回引いて状態を更新する。実測の無いセルは panic（シナリオを実測の範囲に収めること）。
    fn apply(&mut self, input: ImeInput) -> Option<bool> {
        match self.table.step(self.state, input, self.quirks) {
            Transition::Next {
                after,
                write_applied,
            } => {
                if after != self.state {
                    self.last_change = Some((self.clock_ms, self.state));
                }
                self.state = after;
                write_applied
            }
            Transition::Unmeasured(cell) => panic!(
                "擬似 IME の真値が無い: `{cell}`（grid-tables/{} に実測が無い状態・キー。\
                 シナリオを実測のある範囲に収めること）",
                self.table.grid.file()
            ),
        }
    }

    /// いまの状態から `vk` を押したときの真値が生データにあるか。無ければ `press` は panic する。
    /// ランダムなキー列が、実測のある範囲だけを通るための問い合わせ。
    #[must_use]
    pub fn knows(&self, vk: u16) -> bool {
        !matches!(
            self.table
                .step(self.state, ImeInput::Press(vk), self.quirks),
            Transition::Unmeasured(_)
        )
    }

    /// 物理キー1回の押下（生キーが IME に届いた）。
    pub fn press(&mut self, vk: u16) -> PressOutcome {
        let before = self.state;
        self.apply(ImeInput::Press(vk));
        PressOutcome {
            before,
            after: self.state,
        }
    }

    /// awase の見ていない経路での開閉の変更（言語バーのマウス操作等）。入力中は捨てる。
    pub fn external_set_open(&mut self, open: bool) {
        self.apply(ImeInput::External(open));
    }

    /// awase からの開閉の書き込み。効いたら `true`。
    pub fn write_open(&mut self, open: bool) -> bool {
        let applied = self
            .apply(ImeInput::Write(open))
            .expect("書き込みは効いたかを返す");
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

/// 押下後の段階（モジュール doc の仮定。保持のときは `KEPT_STAGE_RULES` を引く）。
fn next_stage(prev: TrueStage, vk: u16, o: GridOutcome) -> TrueStage {
    if !o.open {
        return TrueStage::None;
    }
    match o.disp {
        Disp::None | Disp::Discarded | Disp::Committed => TrueStage::None,
        Disp::Kept => KEPT_STAGE_RULES
            .iter()
            .find(|(k, _)| *k == vk)
            .map_or(prev, |(_, s)| *s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TYPING: TrueState = TrueState {
        open: true,
        conv: CONV_HIRAGANA,
        stage: TrueStage::Typing,
    };

    /// 格子の違いが擬似 IME に現れること: 同じ「IME OFF・半角英数」で英数キーを押すと、ATOK は閉のまま、MS-IME 本体は開いてかなに戻る
    /// （`off-c10-none|eisu`: atok.json=`OFF/0x10`、msime-native.json=`ON/0x19`）。
    #[test]
    fn grids_differ_on_eisu_from_closed_alnum() {
        let start = TrueState {
            open: false,
            conv: CONV_ALNUM,
            stage: TrueStage::None,
        };
        let mut atok = PseudoIme::from_grid(Grid::Atok, start);
        atok.press(0xF0);
        assert!(!atok.state().open);
        let mut native = PseudoIme::from_grid(Grid::MsimeNative, start);
        native.press(0xF0);
        assert!(native.state().open);
        assert_eq!(native.state().conv, CONV_HIRAGANA);
    }

    /// 状態名は格子の JSON と同じ大文字の 16 進で、読み戻せる（`c1B` を小文字で引いて全セルを取りこぼした退行の防止）。
    #[test]
    fn state_name_matches_grid_spelling_and_round_trips() {
        let s = TrueState {
            open: true,
            conv: 0x1B,
            stage: TrueStage::ConvMuhenkan,
        };
        assert_eq!(state_name(s), "on-c1B-conv-muhenkan");
        assert_eq!(parse_state_name("on-c1B-conv-muhenkan"), Some(s));
        assert_eq!(parse_state_name("on-c19-typing-prev-esc"), None);
        let msime = TransitionTable::load(Grid::GjiMsime);
        assert!(
            msime.measured_states().contains(&TrueState {
                open: true,
                conv: 0x1B,
                stage: TrueStage::None,
            }),
            "msime.json の on-c1B-none を擬似 IME の状態として読める"
        );
        let mut ime = PseudoIme::from_grid(
            Grid::GjiMsime,
            TrueState {
                open: true,
                conv: 0x1B,
                stage: TrueStage::None,
            },
        );
        assert!(ime.knows(0x20), "on-c1B-none|space は実測あり");
        ime.press(0x20);
    }

    #[test]
    fn q6_off_write_while_composing_keeps_open_and_changes_only_conv() {
        let mut ime = PseudoIme::atok(TYPING);
        ime.set_off_ignored_while_composing(true);
        assert!(ime.write_open(false), "書き込みは受理される");
        let s = ime.state();
        assert!(s.open, "閉じない");
        assert_eq!(s.conv, CONV_ALNUM, "conv だけ半角英数");
        assert_eq!(s.stage, TrueStage::Typing, "入力中の文字は残る");
    }

    #[test]
    fn q6_off_write_without_composition_closes() {
        let mut ime = PseudoIme::atok(TrueState {
            stage: TrueStage::None,
            ..TYPING
        });
        ime.set_off_ignored_while_composing(true);
        assert!(ime.write_open(false));
        assert!(!ime.state().open, "入力中でなければ閉じる(10/10)");
    }

    #[test]
    fn q6_control_without_switch_closes_even_while_composing() {
        let mut ime = PseudoIme::atok(TYPING);
        assert!(ime.write_open(false));
        assert!(!ime.state().open);
    }

    #[test]
    fn quirk_ids_are_unique_and_modeled_ones_name_a_switch() {
        for (i, q) in QUIRKS.iter().enumerate() {
            assert!(
                QUIRKS[i + 1..].iter().all(|o| o.id != q.id),
                "{} が重複",
                q.id
            );
            assert!(!q.evidence.is_empty() && !q.when.is_empty());
        }
    }
}
