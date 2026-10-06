using System;
using System.IO;
using System.Text;
using Microsoft.UI.Xaml;

namespace WinUiForm
{
    public sealed partial class MainWindow : Window
    {
        public MainWindow()
        {
            InitializeComponent();
            Title = "winui-form-input";
            var args = Environment.GetCommandLineArgs();
            var dump = args.Length > 1 ? args[1] : null;
            if (dump != null) File.WriteAllText(dump, "", new UTF8Encoding(false));
            Box.TextChanged += (s, e) =>
            {
                if (dump == null) return;
                var tmp = dump + ".tmp";
                File.WriteAllText(tmp, Box.Text, new UTF8Encoding(false));
                File.Move(tmp, dump, true);
            };
            Activated += (s, e) => Box.Focus(FocusState.Programmatic);
        }
    }
}
