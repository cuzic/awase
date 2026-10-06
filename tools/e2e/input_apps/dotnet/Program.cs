using System;
using System.IO;
using System.Text;

// kind: wf-textbox | wf-multiline | wf-richtextbox | wpf-textbox | wpf-multiline | webview2
// 第 2 引数があれば、本文が変わるたびに UTF-8 でそのファイルへ書き出す(UIA で読めない部品用)。
internal static class Program
{
    const string Title = "dotnet-form-input";
    static string dump;

    static void Dump(string text)
    {
        if (dump == null) return;
        try
        {
            var tmp = dump + ".tmp";
            File.WriteAllText(tmp, text, new UTF8Encoding(false));
            File.Move(tmp, dump, true);
        }
        catch (Exception e) { Console.Error.WriteLine("dump failed: " + e); }
    }

    [STAThread]
    static int Main(string[] args)
    {
        var kind = args.Length > 0 ? args[0] : "wf-textbox";
        dump = args.Length > 1 ? args[1] : null;
        Dump("");
        switch (kind)
        {
            case "wf-textbox": return WinForms(kind);
            case "wf-multiline": return WinForms(kind);
            case "wf-richtextbox": return WinForms(kind);
            case "wpf-textbox": return Wpf(false);
            case "wpf-multiline": return Wpf(true);
            case "webview2": return WebView2();
            default: Console.Error.WriteLine("unknown kind: " + kind); return 2;
        }
    }

    static int WinForms(string kind)
    {
        System.Windows.Forms.Application.EnableVisualStyles();
        var f = new System.Windows.Forms.Form { Text = Title, Width = 760, Height = 260, StartPosition = System.Windows.Forms.FormStartPosition.Manual, Left = 100, Top = 100 };
        System.Windows.Forms.TextBoxBase c = kind == "wf-richtextbox" ? new System.Windows.Forms.RichTextBox()
            : new System.Windows.Forms.TextBox { Multiline = kind == "wf-multiline" };
        c.Dock = System.Windows.Forms.DockStyle.Fill;
        c.TextChanged += (s, e) => Dump(c.Text);
        f.Controls.Add(c);
        f.Shown += (s, e) => { f.Activate(); c.Focus(); };
        System.Windows.Forms.Application.Run(f);
        return 0;
    }

    static int Wpf(bool multiline)
    {
        var app = new System.Windows.Application();
        var tb = new System.Windows.Controls.TextBox { AcceptsReturn = multiline, TextWrapping = System.Windows.TextWrapping.Wrap };
        tb.TextChanged += (s, e) => Dump(tb.Text);
        var w = new System.Windows.Window { Title = Title, Width = 760, Height = 260, Left = 100, Top = 100, Content = tb };
        w.Loaded += (s, e) => { w.Activate(); tb.Focus(); System.Windows.Input.Keyboard.Focus(tb); };
        return app.Run(w);
    }

    // WebView2(Edge/Chromium)に <textarea> を 1 つ。ページが input のたびに本文を postMessage で渡す。
    static int WebView2()
    {
        System.Windows.Forms.Application.EnableVisualStyles();
        var f = new System.Windows.Forms.Form { Text = Title, Width = 760, Height = 300, StartPosition = System.Windows.Forms.FormStartPosition.Manual, Left = 100, Top = 100 };
        var wv = new Microsoft.Web.WebView2.WinForms.WebView2 { Dock = System.Windows.Forms.DockStyle.Fill };
        f.Controls.Add(wv);
        var udf = Path.Combine(Path.GetTempPath(), "dotnetform-wv2-" + Environment.ProcessId);
        wv.CreationProperties = new Microsoft.Web.WebView2.WinForms.CoreWebView2CreationProperties { UserDataFolder = udf };
        f.Shown += async (s, e) =>
        {
            await wv.EnsureCoreWebView2Async();
            wv.CoreWebView2.WebMessageReceived += (ss, ee) => Dump(ee.TryGetWebMessageAsString());
            wv.CoreWebView2.NavigateToString("<!doctype html><meta charset=utf-8><textarea id=t autofocus rows=8 cols=80></textarea>" +
                "<script>t.addEventListener('input',()=>window.chrome.webview.postMessage(t.value));t.focus();</script>");
            f.Activate(); wv.Focus();
        };
        System.Windows.Forms.Application.Run(f);
        return 0;
    }
}
