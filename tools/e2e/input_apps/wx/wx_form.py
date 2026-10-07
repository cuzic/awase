# wxWidgets(wxPython、Windows ではネイティブの EDIT/RichEdit)の入力欄 1 つ。引数: field|multiline|rich
import sys

import wx

kind = sys.argv[1] if len(sys.argv) > 1 else "field"
app = wx.App()
frame = wx.Frame(None, title="wx-form-input", pos=(100, 100), size=(760, 260))
style = {"field": 0, "multiline": wx.TE_MULTILINE, "rich": wx.TE_MULTILINE | wx.TE_RICH2}[kind]
ctrl = wx.TextCtrl(frame, style=style)
frame.Show()
frame.Raise()
ctrl.SetFocus()
app.MainLoop()
