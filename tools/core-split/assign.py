# ADR-274 D1: モジュール → クラスタ(層)の割り当て。deps.py が読む。
# モジュール名は `crates/awase-ime-core/src/` 以下の相対パス(`mod.rs` は親の名前)。
# 新しいモジュールを足したら、ここにも足す(`deps.py --check` が未割当で落ちる)。
G = {}


def put(name, mods):
    for m in mods:
        G[m] = name


# L0 土台: 定数・仮想キー・キーマップ表・世代・時計・IME 種別・アプリ分類の型。他のどれにも依存しない
put('base', [
    'tuning', 'vk', 'keymap', 'state', 'state::generation', 'state::event_origin', 'state::hub_clock',
    'state::foreground_scope', 'state::injection_mode', 'state::layout_language', 'state::ime_kind',
    'state::focus_gen', 'state::app_suppression', 'state::scoped_latch',
    'focus', 'focus::kinds', 'focus::class_names', 'tsf', 'tsf::converter_target',
])
# L1 キー効果: 学習済みのキー効果の表と予測、キー配置の仮説
put('keyeffect', [
    'state::key_effect_predictor', 'state::key_effect_rules', 'state::key_effect_runtime',
    'state::key_effect_table', 'state::keymap_initial_hypothesis', 'state::keymap_latch',
    'state::late_keymap_plan', 'state::state_dependent_key_warning',
])
# L2 IME の核: 状態(belief)・観測(証拠を含む)・書き込み(actuation)の判断。belief と actuation は辺が 8/10 本で
# 実質 1 つの塊。観測源(mozc_shm・インジケーターの証拠)は journal の記録型ではなく、ここに置く
put('imecore', [
    'state::ime_model', 'state::ime_event', 'state::ime_update', 'state::observation_store',
    'state::evidence', 'state::imm_evidence', 'state::belief', 'state::intent_store',
    'state::probe_admission', 'state::explicit_press', 'state::press_ledger', 'state::open_warrant',
    'state::applied_mirror_plan', 'state::external_change_watch', 'state::drift_correction',
    'state::drift_plan', 'state::conv_mode', 'state::conv_classify', 'state::snapshot_input_mode',
    'state::eisu_candidate', 'state::eisu_recovery', 'state::resume_observation', 'state::transition',
    'state::actuation_chain', 'state::ime_actuation', 'state::ime_actuation_decision',
    'state::ime_set_open_plan', 'state::force_guard', 'state::gji_direct_mechanism',
    'state::ime_read_strategy', 'state::ime_profile_driver', 'state::conv_after_open',
    'state::input_barrier', 'state::app_ime_policy',
    'state::key_sequence_policy', 'state::physical_disposition', 'state::mode_key_pass',
    'state::mozc_shm', 'state::indicator_menu', 'state::indicator_glyph', 'state::indicator_glyph_seeds', 'state::indicator_glyph_store', 'indicator_evidence',
    # journal と、IME の判断を記録する型。`journal` は imecore のイベント型(DriftEdge・ConvTransition ほか)を記録し、
    # 観測源(mozc_shm・indicator_*)が `*Record` を返すので、依存が相互になる(測定済み)。分けずに imecore に含める
    'journal', 'journal_policy', 'state::ime_event_log', 'state::actuation_decision_record', 'tsf::literal_facts',
])
# L3 葉(core のどこからも使われない): 判断の計画(Facts → Plan)。TSF の送信・退避・回収と、フォーカスの分類・解決
put('plans', [
    'state::warm_send_plan', 'state::deferred_gate_plan', 'state::raw_recovery_plan', 'state::relay_plan',
    'state::post_bypass', 'state::focus_classify_plan', 'state::focus_probe_plan',
    'state::focus_resolve_plan', 'state::focus_resync_policy', 'state::msaa_role_plan', 'focus::hwnd_cache',
])
# L3 葉: フック・修飾キー・半角英数トグルの状態
put('hookinput', [
    'state::hook_state', 'state::hook_watchdog', 'state::alt_impersonation', 'state::foreign_modifier',
    'state::win_key_guard', 'state::half_width_alnum', 'state::half_width_resume_hold',
])
# 独立の葉: Scancode Map（キー入れ替え）。外部の依存を持たない（core の他のクラスタに依存しない）
put('scancode', [
    'scancode_apply', 'scancode_diagram', 'scancode_editor', 'scancode_map', 'scancode_pairs',
])

# 葉: 不具合報告のペイロードの組み立て。keyeffect の表・imecore の証拠（インジケーター）を読む
put('bugreport', ['bug_report'])

# 許可する依存の向き(A → B = A が B を使ってよい)。これ以外の本番コードの参照は `deps.py --check` が落とす
ALLOWED = {
    'base': [],
    'keyeffect': ['base'],
    'imecore': ['base', 'keyeffect'],
    'plans': ['base', 'imecore'],
    'hookinput': ['base'],
    'scancode': [],
    'bugreport': ['base', 'keyeffect', 'imecore'],
}

# コードの移動で消す予定の辺(`impl` や関数を別のモジュールへ移すまでの間だけ許す)。(参照元, 参照先, 理由)。
# ADR-274 段階 0 の 1(class_names.rs の `impl From<AppImeProfile> for ImePolicyProfile` を ime_event.rs へ)は済んだので空。
PLANNED_EDGE_REMOVALS = []
