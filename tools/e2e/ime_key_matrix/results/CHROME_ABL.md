| 構成 | 集計 | ケース3 | ケース7 |
|---|---|---|---|
| base | PASS=18 RECOVER=0 FAIL=6 INVALID=0 | 3(かな→ひらがな)=['FAIL', 'FAIL', 'FAIL'] | 7(かな→Shift+無変換)=['FAIL', 'FAIL', 'FAIL'] |
| E1-widen-guard2 | PASS=18 RECOVER=6 FAIL=0 INVALID=0 | 3(かな→ひらがな)=['RECOVER', 'RECOVER', 'RECOVER'] | 7(かな→Shift+無変換)=['RECOVER', 'RECOVER', 'RECOVER'] |
| E2-150ms | PASS=21 RECOVER=3 FAIL=0 INVALID=0 | 3(かな→ひらがな)=['PASS', 'PASS', 'PASS'] | 7(かな→Shift+無変換)=['RECOVER', 'RECOVER', 'RECOVER'] |
| E2-300ms | PASS=24 RECOVER=0 FAIL=0 INVALID=0 | 3(かな→ひらがな)=['PASS', 'PASS', 'PASS'] | 7(かな→Shift+無変換)=['PASS', 'PASS', 'PASS'] |
