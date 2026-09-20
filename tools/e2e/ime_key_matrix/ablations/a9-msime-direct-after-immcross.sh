#!/usr/bin/env bash
# A9: MS-IME では、ImmCross(IMM32が使えるプロファイル)が失敗した後のフォールバックを、非冪等な VK_KANJI トグル(KanjiToggle)
# ではなく冪等な VK_IME_ON/OFF(MsImeDirect)にする。MsImeDirect の適用条件から「IMM32クロスプロセス不可」を外す。
python3 - <<'PY'
p='crates/awase-windows/src/state/key_sequence_policy.rs'
s=open(p,encoding='utf8').read()
a="matches!(kind, ImeKindId::MsIme) && !profile.can_use_imm32_cross_process()"
assert a in s
open(p,'w',encoding='utf8').write(s.replace(a,"matches!(kind, ImeKindId::MsIme) && { let _ = profile; true }",1))
PY
