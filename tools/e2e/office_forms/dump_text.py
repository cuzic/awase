# -*- coding: utf-8 -*-
# LibreOffice / Apache OpenOffice の Writer 文書の本文を、UNO で読んで UTF-8 のファイルへ書き出し続ける。
# 各 Office に同梱の python.exe で動かす(LibreOffice は Python 3、OpenOffice 4.1.x は 2 系のこともあるので両対応で書く)。
# 使い方: python.exe dump_text.py <ポート> <出力ファイル>
# 例外は <出力ファイル>.err に追記する(typing_stress が終了時にログへ写す)。
import io
import os
import sys
import time
import traceback

# 同梱 python.exe から素で起動すると uno を見つけられない(OpenOffice 4.1.x で ImportError)ので、
# python.exe のあるフォルダ(= Office の program\)を検索パス・PATH・URE_BOOTSTRAP に足してから読み込む。
def _find_program_dir():
    # LibreOffice は program\python.exe、OpenOffice は program\python-core-X\bin\python.exe が実体。pyuno.pyd/uno.py のある親を探す。
    d = os.path.dirname(os.path.abspath(sys.executable))
    for _ in range(4):
        if os.path.exists(os.path.join(d, "pyuno.pyd")) or os.path.exists(os.path.join(d, "uno.py")):
            return d
        d = os.path.dirname(d)
    return os.path.dirname(os.path.abspath(sys.executable))


_prog = _find_program_dir()
sys.path.insert(0, _prog)
os.environ["PATH"] = _prog + os.pathsep + os.environ.get("PATH", "")
os.environ.setdefault("URE_BOOTSTRAP", "vnd.sun.star.pathname:" + os.path.join(_prog, "fundamental.ini"))

try:
    import uno  # noqa: E402
except ImportError:
    # 診断: どこに uno があるか(typing_stress が helper.log をログへ写す)
    sys.stderr.write("sys.executable=%s\nsys.version=%s\nsys.path=%s\n" % (sys.executable, sys.version, sys.path))
    for root, dirs, files in os.walk(os.path.dirname(os.path.dirname(os.path.abspath(sys.executable)))):
        for n in files:
            if "uno" in n.lower() and n.lower().endswith((".py", ".pyd", ".pyc")):
                sys.stderr.write("found: %s\n" % os.path.join(root, n))
    raise

port = int(sys.argv[1])
out = sys.argv[2]
err = out + ".err"


def note(msg):
    with io.open(err, "a", encoding="utf-8") as f:
        f.write(u"%s\n" % msg)


def connect():
    local = uno.getComponentContext()
    resolver = local.ServiceManager.createInstanceWithContext("com.sun.star.bridge.UnoUrlResolver", local)
    url = "uno:socket,host=127.0.0.1,port=%d;urp;StarOffice.ComponentContext" % port
    deadline = time.time() + 90
    last = None
    while time.time() < deadline:
        try:
            return resolver.resolve(url)
        except Exception as e:  # 起動待ち
            last = e
            time.sleep(0.5)
    raise RuntimeError("UNO に接続できない: %s" % last)


def main():
    ctx = connect()
    smgr = ctx.ServiceManager
    desktop = smgr.createInstanceWithContext("com.sun.star.frame.Desktop", ctx)
    note(u"connected")
    last = None
    while True:
        try:
            doc = desktop.getCurrentComponent()
            if doc is not None and doc.supportsService("com.sun.star.text.TextDocument"):
                text = doc.getText().getString()
                if text != last:
                    tmp = out + ".tmp"
                    with io.open(tmp, "w", encoding="utf-8", newline="") as f:
                        f.write(text)
                    # 置き換え(Windows の os.rename は既存ファイルがあると失敗する)
                    import os
                    if os.path.exists(out):
                        os.remove(out)
                    os.rename(tmp, out)
                    last = text
                elif not __import__("os").path.exists(out):
                    with io.open(out, "w", encoding="utf-8", newline="") as f:
                        f.write(text)
        except Exception:
            note(traceback.format_exc())
        time.sleep(0.05)


try:
    main()
except Exception:
    note(traceback.format_exc())
    raise
