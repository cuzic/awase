# ADR196-T2: MS-IME本体の学習に残る未解決問題（2026-09-24）

状態: **未着手（記録のみ）**。windows-latest実機検証（PR #281の検証、run 35947329002 / 35947596061 / 35947850606、
配線なしの完走run 35945955606）で判明した事項。検証ブランチ`ci/adr196-t2-msime-closedconv-verify`は
developへマージしない。

## 経緯の要点

- MS-IME本体で学習初期化が`未知の変換モード値 0x0001`で失敗した件の**真因はCIの言語設定**
  （既存リストへja-JP追加だとen-USが既定のままMS-IMEがアクティブにならない）。ja-JPのみ＋MS-IME TIPに
  直すと解消し、学習は完走する（run 35945955606: presses=1891、judgement=rejected、verify_accuracy=0.940）。
  「open=falseならconvを0x00扱い」の正規化は、必要性が証明できず撤去した。
- `observation_alive`/`measurement_suspicious`の配線と`ARRIVAL_LOG`は、ARRIVAL_LOG撤去のみPR #281で実施（2026-09-24にdevelop統合済み。0x0001正規化と生存確認配線は同PR内で`0d4d9f80`により撤去）。
- 再測定の押下数上限・リセット間隔の実測調整（PR #285）はGJI+ATOKでの値。MS-IME本体では未測定。

## 未解決1: 生存確認（`observation_alive` / `measurement_suspicious`）が未配線

`RealImeDriver::press()`から呼ぶと、MS-IME本体で学習が失敗する（実測）。

- MS-IME本体は開閉・変換モードが変わっても`WM_IME_NOTIFY`(`IMN_SETOPENSTATUS`/`IMN_SETCONVERSIONMODE`)を
  EDITへ送らない。`status_changed=true, hook_alive=true, notify_alive=false`で全試行が無効化される
  （run 35947329002/35947596061）。
- 通知不着を警告のみにしても、1回の注入で通知が2件以上届き`measurement_suspicious=true`となって
  4試行が無効化、`reason=interference`で失敗（run 35947850606）。配線なしは完走。
- フックの生存確認（`hook_monitor.liveness()`）は累計判定で、一度の取りこぼしで以降すべて無効になる設計。

検討事項: 通知が届かないIME(TSFのみ)でも成り立つ生存確認の設計（通知経路を持つIMEにだけ適用する、
TSF compartment通知を別経路の生存指標にする、等）。GJI/ATOKで配線した場合の挙動も未検証。
配線するなら本タスクで、MS-IME・GJI・ATOKの3構成で実機確認すること。

## 未解決2: 半角カタカナ（conv=0x0013）が学習モデルに無い（**対応済み**、2026-09-23）

対応: `Status::mode_from_raw_conv`（`awase-keymap-learn/src/model.rs`）で`raw & 0x0B`をそのまま`mode`に保持し、
`Conv`に表せない値（0x13→0x03、0x18→0x08）も復号失敗にしない。予測側`convert_cell`は表せないセルを
読み飛ばすので予測への影響なし。`verify_accuracy`の実機での改善（0.95以上か）は未確認（CI待ち）。

以下は対応前の記録。

MS-IME本体で`未知の変換モード値 0x0013`が多発し復号失敗する（run 35945955606: 683回、decode_errors=220。
run 35947850606: decode_errors=95）。`Conv`(C10/C19/C1B)に半角カタカナ(0x13)が無いモデル欠落で、
精度低下（verify_accuracy=0.940<0.95）の一因。`--adopt-pending-judgement`による採用経路は
精度≥0.95でないと通らないため、要確認→採用の実機通過は未検証のまま。

検討事項: `Conv`に0x13を追加するか、未知convを「復号失敗」でなく別状態として扱うか
（ADR-195「誤りに強い分類は未実装」との関係）。

## 参考

- 診断runで「IMEが開かず'k'が入力される」現象は、`--activate-gji`(+`--msime`)の付け忘れによる診断構成の不備で実バグではない。
- 関連: [adr195-remaining-work-2026-09-23.md](adr195-remaining-work-2026-09-23.md)、
  [adr195-t10-realimedriver-ci-observation-failure.md](adr195-t10-realimedriver-ci-observation-failure.md)
