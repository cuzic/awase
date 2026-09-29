---
id: ADR-208
title: |-
  Blind 窓(Imm32Unavailable/TsfNative)で、絶対指定の IME キー(Ctrl+変換/無変換 等)が古い `applied` を根拠に送信を省かれ続ける固着を防ぐ(草稿)
summary: |-
  ADR-205 から切り出した。GJI では `gji_direct_already_matches(shadow_on, open)` が `applied` を根拠に `VK_IME_ON/OFF` の送信を省く。読めない窓(Blind)では `applied` を観測で訂正できず、
  外部から開閉が変わって(かつ ADR-205 の watch で検出できなかった)後は、絶対指定キー(Ctrl+変換=ON 等)を何度押しても `AlreadyMatched` で握り潰され状態が変わらない(所有者定義の「固着」)。
  ADR-205 round5 の指摘: (1) 置き場所は `kp_stage_shadow_ime_toggle` の入口では不足(Ctrl+変換はエンジンのコンボ)、`kp_run_inner` で shadow toggle より前・1押下1回。`applied` は reducer 経由(新 event、`applied_direct_assignments_are_accounted_for`、ime_event_guard)。
  (2) TsfNative(WT×GJI)へ広げると BUG-124 の「@」(単発 `VK_IME_OFF`)の構成を作り直す → Imm32Unavailable かつ非 TsfNative に限るか、WT で A/B を先に取る。
  (3) MS-IME × 実 Chrome は awase 自身の `VK_IME_OFF` が効かない測定(BUG-172 対照)があり、別問題。
status: |-
  草稿(2026-09-29)。未レビュー・未着手。ADR-205 の実装後に、第0段の測定(MS-IME の VK_IME_OFF、WT×GJI の A/B)を踏まえて起草を進める。ADR-206 決定3(b) の Blind 窓例外とセットで検討。
related_adr:
  - "ADR-205"
  - "ADR-206"
  - "ADR-098"
  - "ADR-108"
---

# ADR-208: 絶対指定キーが古い `applied` で握り潰され続けないようにする(草稿)

## 固着の定義(所有者、2026-09-29)

固着 = 何度モードキーを押しても状態が変わらないこと。トグル系で belief が古く 2 回押しで直るのは許容。本 ADR の対象は、絶対指定キーが省略され続ける場合のみ。

## 論点(ADR-205 round5 の結論、未決)

1. 置き場所: 非注入の KeyDown で、この押下が IME actuation を起こしうるとき(`is_ime_mode_key` またはエンジンの特殊キー照合に一致)、`kp_run_inner` の中で shadow toggle より前に 1 回、`applied` を未確認に落とす。
   BUG-113 の同一押下での二重送信は、最初の送信が `applied` を `Optimistic(open)` にするので従来ガードで守られる。
2. 書き込み口: `applied` を落とす専用 event が必要(`ModeKeyPassedThrough` は `last_intent` も捨てるので不可)。`ime_event_guard` に登録し構築点を固定する。
3. 適用範囲: Imm32Unavailable かつ非 TsfNative。TsfNative(WT×GJI)は実 ON 時の単発 `VK_IME_OFF` が「@」を誘発するか A/B を取ってから(推奨)。だめなら既知の制限として明記する。
4. ADR-206 は決定3(b)（Consume のみ）を撤回済み（OFF 方向は常に絶対指定 `SetOpen(false)`）。代償は belief OFF での OFF キーごとの単発 `VK_IME_OFF`（WT×GJI で BUG-124 型の「@」の可能性、ADR-206 側の実機 A/B がマージ条件、代替は同じ OFF キー2連打時だけ送る案）。本 ADR の対象に「`bare_ime_action` または `forced_open_action` を持つ親指の非リピート Down」を含める（親指の S1/S2 は `shadow_action`/`sync_direction` を持たず、`kp_stage_shadow_ime_toggle` の入口だけでは漏れる）。出荷順は ADR-206 と同時、または本 ADR の後。
5. MS-IME × 実 Chrome: awase の `VK_IME_OFF` が閉じない可能性(BUG-172 対照)。実機確認が先。
