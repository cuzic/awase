// LINE(Qt 6.6.3)の入力欄を模した最小アプリ。QLineEdit 1つだけの窓。
// CI では line.exe の名前でビルドし、awase の学習キャッシュ((プロセス名,クラス名)キー)が
// 実 LINE と同じ経路(Qt663QWindowIcon → Imm32Unavailable)になることを狙う。
// 読み戻しは typing_stress --form=qt が UI Automation で行う(窓内の唯一の Edit を読む)。
#include <QApplication>
#include <QLineEdit>
#include <QVBoxLayout>
#include <QWidget>

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QWidget w;
    w.setWindowTitle("qt-line-input");
    auto *lay = new QVBoxLayout(&w);
    auto *edit = new QLineEdit;
    edit->setAccessibleName("stress-input");
    lay->addWidget(edit);
    w.resize(700, 80);
    w.show();
    edit->setFocus();
    return app.exec();
}
