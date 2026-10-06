# wxPython(wxWidgets)を uv で。ビルド済み wheel が取れるか確認する。
uv run --python 3.12 --with wxPython python -c "import wx; print('wxPython', wx.version())"
if ($LASTEXITCODE -ne 0) { throw 'wxPython を用意できない' }
