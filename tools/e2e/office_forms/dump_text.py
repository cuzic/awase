# -*- coding: utf-8 -*-
# LibreOffice / Apache OpenOffice の Writer 文書の本文を、UNO で読んで UTF-8 のファイルへ書き出し続ける。
# 各 Office に同梱の python.exe で動かす(LibreOffice は Python 3、OpenOffice 4.1.x は 2 系のこともあるので両対応で書く)。
# 使い方: python.exe dump_text.py <ポート> <出力ファイル>
# 例外は <出力ファイル>.err に追記する(typing_stress が終了時にログへ写す)。
import io
import sys
import time
import traceback

import uno

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
