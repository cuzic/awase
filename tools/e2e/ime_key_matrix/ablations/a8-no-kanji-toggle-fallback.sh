#!/usr/bin/env bash
# A8: ImmCross失敗後の非同期フォールバックで、非冪等な VK_KANJI トグル(KanjiToggle)を送らない。
# 仮説: MS-IME本体では ImmCross(WM_IME_CONTROL)がタイムアウトで Failed になるが、メッセージ自体は遅れて処理される/
# 生キーが既にIMEを開いているため、その後の盲目的なトグルが開いたIMEを閉じてしまう(Engine ON + IME OFF)。
python3 - <<'PY'
p='crates/awase-windows/src/runtime/open_chain.rs'
s=open(p,encoding='utf8').read()
a="""        let (command, outcome) = if crate::ime_controller::mechanism_is_applicable(mechanism, &view)
        {"""
assert a in s
b="""        let (command, outcome) = if mechanism != WriteMechanism::KanjiToggle
            && crate::ime_controller::mechanism_is_applicable(mechanism, &view)
        {"""
open(p,'w',encoding='utf8').write(s.replace(a,b,1))
PY
