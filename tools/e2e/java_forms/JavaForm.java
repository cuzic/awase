// Java の入力部品を 1 つだけ持つ窓。awase の「Java アプリで入力が正常にできない」報告の再現・回帰確認用。
// 読み戻しは typing_stress が行う。Java の部品は UI Automation で読めない(Java Access Bridge が要る)ので、
// 確定済みの内容が変わるたびに UTF-8 のファイル(第 2 引数)へ書き出し、typing_stress がそのファイルを読む。
// 未確定文字(composition)は部品側が持っていてドキュメントに入らないので、書き出されるのは確定済みの文字だけ。
//
// 使い方: java -Dfile.encoding=UTF-8 -cp <このディレクトリ> JavaForm <swing|swingarea|awt|awtarea> <出力ファイル>
import java.awt.BorderLayout;
import java.awt.Component;
import java.awt.Frame;
import java.awt.TextArea;
import java.awt.TextComponent;
import java.awt.TextField;
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import javax.swing.JFrame;
import javax.swing.JTextArea;
import javax.swing.JTextField;
import javax.swing.SwingUtilities;
import javax.swing.event.DocumentEvent;
import javax.swing.event.DocumentListener;
import javax.swing.text.JTextComponent;

public final class JavaForm {
    static final String TITLE = "java-form-input";
    static Path out;

    /** 一時ファイルへ書いてから置き換える(読み手が書きかけを読まないように)。 */
    static synchronized void dump(String text) {
        try {
            Path tmp = Paths.get(out.toString() + ".tmp");
            Files.write(tmp, text.getBytes(StandardCharsets.UTF_8));
            Files.move(tmp, out, StandardCopyOption.REPLACE_EXISTING);
        } catch (IOException e) {
            System.err.println("dump failed: " + e);
        }
    }

    public static void main(String[] args) throws Exception {
        String kind = args[0];
        out = Paths.get(args[1]);
        dump("");
        switch (kind) {
            case "swing":
            case "swingarea":
                SwingUtilities.invokeLater(() -> swing(kind.equals("swingarea")));
                break;
            case "awt":
            case "awtarea":
                awt(kind.equals("awtarea"));
                break;
            default:
                System.err.println("unknown kind: " + kind);
                System.exit(2);
        }
    }

    static void swing(boolean area) {
        JFrame f = new JFrame(TITLE);
        f.setDefaultCloseOperation(JFrame.EXIT_ON_CLOSE);
        JTextComponent c = area ? new JTextArea(8, 60) : new JTextField(60);
        c.getDocument().addDocumentListener(new DocumentListener() {
            void changed() { dump(c.getText()); }
            public void insertUpdate(DocumentEvent e) { changed(); }
            public void removeUpdate(DocumentEvent e) { changed(); }
            public void changedUpdate(DocumentEvent e) { changed(); }
        });
        f.getContentPane().add(c, BorderLayout.CENTER);
        f.pack();
        f.setLocation(100, 100);
        f.setVisible(true);
        c.requestFocusInWindow();
    }

    static void awt(boolean area) {
        Frame f = new Frame(TITLE);
        f.addWindowListener(new WindowAdapter() {
            @Override public void windowClosing(WindowEvent e) { System.exit(0); }
        });
        TextComponent c = area ? new TextArea("", 8, 60) : new TextField("", 60);
        c.addTextListener(e -> dump(c.getText()));
        f.add((Component) c, BorderLayout.CENTER);
        f.pack();
        f.setLocation(100, 100);
        f.setVisible(true);
        c.requestFocus();
    }
}
