#!/usr/bin/env python3
"""実 Chrome の drift recovery を DRIFT_RECOVERY 規約で集計する。"""
import os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_common import DRIFT_RE as DRIFT, OBSERVED_RE as OBS, PHYS_CTRL_RE as PHYS  # noqa: E402
MUH=re.compile(r"\[engine-input\] vk=0x1D KeyDown")
def analyze_ctrl(chrome,awase):
    """`chrome_probe --close-ime=N --ctrl-muhenkan-off`: 物理 Ctrl+無変換で OFF にする変種。gap=OFF 操作の直後も実 IME が開いたまま。
    RESULT PASS=英字(ka)=OFF が効いた(または補正された)。gap あり+PASS=corrected、gap あり+FAIL=not_corrected、gap なし=gap_not_made。"""
    lines=[re.sub(r"^\[[\d:.]+Z\] ","",x) for x in chrome]
    c=dict(gap_not_made=0,corrected=0,not_corrected=0,invalid=0)
    for x in lines:
        if "RESULT " not in x: continue
        gap="gap=true" in x
        if "RESULT INVALID" in x: c["invalid"]+=1
        elif not gap: c["gap_not_made"]+=1
        elif "RESULT PASS" in x: c["corrected"]+=1
        else: c["not_corrected"]+=1
    n=sum(c.values()); gap_made=c["corrected"]+c["not_corrected"]
    muh=[x for x in awase if MUH.search(x)]; phys=sum(bool(PHYS.search(x)) for x in muh)
    observed=sum(bool(OBS.search(x)) for x in awase); drift=sum(bool(DRIFT.search(x)) for x in awase)
    if n==0 or n-c["invalid"]<c["invalid"]: verdict="INVALID"  # 有効試行が無い、または有効が無効より少ない
    elif not muh or phys!=len(muh): verdict="INVALID"  # 注入が物理 Ctrl+無変換として届いていない(前提不成立)
    elif gap_made==0: verdict="GAP_NOT_MADE"
    elif c["not_corrected"]==0: verdict="CORRECTED"
    elif c["corrected"]==0: verdict="NOT_CORRECTED"
    else: verdict="UNDETERMINED"
    return dict(verdict=verdict,trials=n,recovered=c["corrected"],not_recovered=c["not_corrected"],invalid=c["invalid"],observed=observed,drift=drift,
                ctrl=True,gap_made=gap_made,gap_not_made=c["gap_not_made"],phys_ctrl_ok=phys,muhenkan_downs=len(muh))
def analyze(chrome,awase):
    if any("CLOSE_IME method=ctrl_muhenkan" in x for x in chrome): return analyze_ctrl(chrome,awase)
    rs=[re.sub(r"^\[[\d:.]+Z\] ","",x) for x in chrome if "RESULT " in x]
    valid=[x for x in rs if "INVALID" not in x]; recovered=sum("RESULT PASS" in x or "RESULT RECOVER" in x for x in valid)
    observed=sum(bool(OBS.search(x)) for x in awase); drift=sum(bool(DRIFT.search(x)) for x in awase)
    if not valid: verdict="INVALID"
    elif observed==0 and drift==0: verdict="NOT_OBSERVED"
    elif recovered==len(valid): verdict="RECOVERED"
    elif recovered==0: verdict="NOT_RECOVERED"
    else: verdict="UNDETERMINED"
    return dict(verdict=verdict,trials=len(rs),recovered=recovered,not_recovered=len(valid)-recovered,invalid=len(rs)-len(valid),observed=observed,drift=drift)
def main(argv):
    if len(argv)!=2:return 2
    try:
        c=open(argv[0],encoding="utf-8",errors="replace").read().splitlines(); a=open(argv[1],encoding="utf-8",errors="replace").read().splitlines()
    except OSError as e: print(f"DRIFT_RECOVERY: verdict=INVALID reason={e}"); return 3
    r=analyze(c,a); extra=" method=ctrl_muhenkan gap_made={gap_made} gap_not_made={gap_not_made} phys_ctrl_ok={phys_ctrl_ok}".format(**r) if r.get("ctrl") else ""
    print("DRIFT_RECOVERY: verdict={verdict} form=chrome ime=? trials={trials} recovered={recovered} reopened_by_typing=0 typed_blind=0 unexplained=0 api_only=0 not_recovered={not_recovered} invalid_trials={invalid} observed={observed} drift={drift} conv_read=0 reinit=0 unicode=0 intent_true=0".format(**r)+extra)
    return 3 if r["verdict"]=="INVALID" else (0 if r["verdict"] in ("RECOVERED","CORRECTED") else 1)
if __name__=="__main__":sys.exit(main(sys.argv[1:]))
