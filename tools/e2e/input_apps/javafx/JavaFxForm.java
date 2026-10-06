// JavaFX(Glass、Swing/AWT とは別の入力実装)の入力欄 1 つ。第 1 引数 field|area、第 2 引数: 本文の書き出し先(UTF-8)。
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import javafx.application.Application;
import javafx.scene.Scene;
import javafx.scene.control.TextArea;
import javafx.scene.control.TextField;
import javafx.scene.control.TextInputControl;
import javafx.stage.Stage;

public final class JavaFxForm extends Application {
    static synchronized void dump(Path out, String text) {
        try {
            Path tmp = Paths.get(out.toString() + ".tmp");
            Files.write(tmp, text.getBytes(StandardCharsets.UTF_8));
            Files.move(tmp, out, StandardCopyOption.REPLACE_EXISTING);
        } catch (IOException e) {
            System.err.println("dump failed: " + e);
        }
    }

    @Override
    public void start(Stage stage) {
        String kind = getParameters().getRaw().get(0);
        Path out = Paths.get(getParameters().getRaw().get(1));
        dump(out, "");
        TextInputControl c = kind.equals("area") ? new TextArea() : new TextField();
        c.textProperty().addListener((o, a, b) -> dump(out, b));
        stage.setTitle("javafx-form-input");
        stage.setScene(new Scene(c, 700, 200));
        stage.setX(100);
        stage.setY(100);
        stage.show();
        c.requestFocus();
    }

    public static void main(String[] args) {
        launch(args);
    }
}
