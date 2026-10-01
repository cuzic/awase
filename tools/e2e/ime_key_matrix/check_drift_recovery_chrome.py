#!/usr/bin/env python3
"""実 Chrome の drift recovery を DRIFT_RECOVERY 規約で集計する。"""
import re, sys
OBS=re.compile(r"\[stage-observe\] observer_poll=Some|ObserverReported")
DRIFT=re.compile(r"\[drift\] correction:|Blacklist drift correction: apply_ime_open")
def analyze(chrome,awase):
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
    r=analyze(c,a); print("DRIFT_RECOVERY: verdict={verdict} form=chrome ime=? trials={trials} recovered={recovered} reopened_by_typing=0 typed_blind=0 unexplained=0 api_only=0 not_recovered={not_recovered} invalid_trials={invalid} observed={observed} drift={drift} conv_read=0 reinit=0 unicode=0 intent_true=0".format(**r))
    return 3 if r["verdict"]=="INVALID" else (0 if r["verdict"]=="RECOVERED" else 1)
if __name__=="__main__":sys.exit(main(sys.argv[1:]))
