//! `eframe::run_native` の起動失敗を、可能ならフォールバックで回避し、
//! 回避できなければ診断してユーザーに示す。
//!
//! `#![windows_subsystem = "windows"]` によりコンソールが無く、`eframe::run_native`
//! が `Err` を返しても誰の目にも触れない（ダブルクリックしても一瞬で何も起きずに
//! 終了したように見えるだけ）。この `Err` は主にウィンドウ/グラフィックスコンテキスト
//! 生成失敗で発生するが、「グラフィックドライバが原因かもしれません」という
//! 当たり障りのない固定文言では、実際に何を確認・修正すればよいかユーザーには
//! 分からない。
//!
//! 対処は2段構え:
//! 1. **[`run_with_fallback`]**: 既定の Glow(OpenGL) レンダラーで起動できない
//!    環境では、wgpu(Direct3D 12 / Vulkan)レンダラーへ切り替えて再試行する。
//!    実グラフィックスドライバが無い環境でも Windows 標準のソフトウェア
//!    ラスタライザ WARP が使えるため、そもそもダイアログを出さずに起動できる
//!    ケースが多い。
//! 2. それでも両方失敗した場合のみ、`eframe::Error` の実体
//!    （`glutin::error::ErrorKind` / `egui_glow::PainterError` のメッセージ）と、
//!    実機を能動的に調べて得た事実（実際のグラフィックスアダプタ名・リモート
//!    デスクトップ接続の有無・`opengl32.dll` を今読み込めるか）を突き合わせ、
//!    「このPCでは何が起きているか」を確定的な文で [`show_dialog`] に示す。

/// `SettingsApp` / `BugReportApp` 等の `eframe::App` を生成するファクトリ。
/// Glow → wgpu の2回、別々の `eframe::run_native` 呼び出しへ渡すために
/// `Rc` で共有する（`AppCreator` 自体は `FnOnce` だが、生成ロジックそのもの
/// は複数回呼び出せる必要がある）。
///
/// コードレビュー指摘: 元の単発 `eframe::run_native` 呼び出しは `FnOnce` を
/// 渡していたため「生成ロジックは高々1回しか走らない」ことが型で保証
/// されていたが、この `Fn` 化でその保証が消える。Glow の試行がウィンドウ/GL
/// コンテキスト生成後（アプリ生成後）に失敗するケースでは、この関数が実際に
/// 同一プロセス内で2回呼ばれうる。`SettingsApp::new`/`BugReportApp::new` は
/// 現状ファイル読み込みのみで副作用が冪等なため実害は無いが、今後どちらかに
/// 一度きりの副作用（ファイルロック取得・IPC登録等）を足す場合は、この
/// ファクトリが複数回呼ばれても安全であることを確認すること。
type AppFactory = std::rc::Rc<dyn Fn(&eframe::CreationContext<'_>) -> Box<dyn eframe::App>>;

/// Glow(OpenGL) → wgpu(Direct3D 12 / Vulkan) の順で起動を試みる。
///
/// なぜ2段構えが要るか: awase 設定画面（eframe/egui）は既定で Glow(WGL 経由の
/// OpenGL)を使うが、実グラフィックスドライバが無い環境（Basic Display Adapter
/// へのフォールバック等）では OpenGL 2.0 未満のソフトウェアラスタライザしか
/// 手に入らず起動できない（`egui_glow::PainterError` "requires opengl 2.0+"）。
/// wgpu の DX12 バックエンドには、実 GPU ドライバが無くても常に使える
/// Windows 標準のソフトウェアラスタライザ WARP（`d3d10warp.dll`、Windows 10
/// 以降 OS 標準搭載）があるため、Glow が失敗した場合のフォールバックとして
/// 機能する（[`select_wgpu_adapter`] 参照）。両方失敗した場合のみ診断
/// ダイアログを出す。
pub(crate) fn run_with_fallback(
    app_name: &str,
    viewport: eframe::egui::ViewportBuilder,
    app_creator: impl Fn(&eframe::CreationContext<'_>) -> Box<dyn eframe::App> + 'static,
) -> eframe::Result<()> {
    run_with_fallback_impl(app_name, viewport, std::rc::Rc::new(app_creator))
}

#[cfg(target_os = "windows")]
fn run_with_fallback_impl(
    app_name: &str,
    viewport: eframe::egui::ViewportBuilder,
    app_creator: AppFactory,
) -> eframe::Result<()> {
    let glow_options = eframe::NativeOptions {
        viewport: viewport.clone(),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let creator = app_creator.clone();
    let glow_result =
        eframe::run_native(app_name, glow_options, Box::new(move |cc| Ok(creator(cc))));
    let Err(glow_err) = glow_result else {
        return glow_result;
    };
    tracing::warn!(
        "{app_name}: glow(OpenGL) レンダラーでの起動に失敗、wgpu(Direct3D/Vulkan) \
         へフォールバックします: {glow_err}"
    );

    let wgpu_options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew {
                native_adapter_selector: Some(std::sync::Arc::new(select_wgpu_adapter)),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let wgpu_result = eframe::run_native(
        app_name,
        wgpu_options,
        Box::new(move |cc| Ok(app_creator(cc))),
    );
    match &wgpu_result {
        Ok(()) => tracing::info!("{app_name}: wgpu フォールバックで起動に成功しました"),
        Err(wgpu_err) => {
            let detail = describe_fallback_failure(app_name, &glow_err, wgpu_err);
            tracing::error!(
                "{app_name}: eframe::run_native failed even with wgpu fallback:\n{detail}"
            );
            show_dialog(&detail);
        }
    }
    wgpu_result
}

#[cfg(not(target_os = "windows"))]
fn run_with_fallback_impl(
    app_name: &str,
    viewport: eframe::egui::ViewportBuilder,
    app_creator: AppFactory,
) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let result = eframe::run_native(app_name, options, Box::new(move |cc| Ok(app_creator(cc))));
    if let Err(e) = &result {
        let detail = describe(app_name, e);
        tracing::error!("{app_name}: eframe::run_native failed:\n{detail}");
        show_dialog(&detail);
    }
    result
}

/// `app_name`（`eframe::run_native` の第1引数、"awase-settings" /
/// "awase-bug-report"）を、ダイアログ文言に使う日本語のウィンドウ名に変える。
/// 未知の名前が来た場合はそのまま返す（未対応のウィンドウが増えても
/// パニックしない）。
fn app_label(app_name: &str) -> &str {
    match app_name {
        "awase-settings" => "awase 設定画面",
        "awase-bug-report" => "awase 不具合報告画面",
        other => other,
    }
}

/// wgpu のネイティブアダプタ選択。実グラフィックスアダプタ（Discrete/Integrated/
/// Virtual GPU）があればそれを優先し、無ければソフトウェアアダプタ
/// （`DeviceType::Cpu`、Windows では DX12 の WARP）へ最終フォールバックする。
///
/// 素の `wgpu::Instance::request_adapter`（`force_fallback_adapter: false`）は
/// ソフトウェアアダプタしか無い環境では単に失敗を返す仕様（wgpu 自身のコメント
/// 参照）で、`force_fallback_adapter: true` は逆に実 GPU があってもソフトウェア
/// アダプタを強制してしまう。「実GPUがあればそれを、無ければソフトウェアへ」
/// という優先順位は `egui_wgpu::WgpuSetupCreateNew::native_adapter_selector` で
/// 全アダプタを見て自前で選ぶしかない。
#[cfg(target_os = "windows")]
fn select_wgpu_adapter(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<wgpu::Adapter, String> {
    let compatible =
        |adapter: &&wgpu::Adapter| surface.is_none_or(|s| adapter.is_surface_supported(s));

    if let Some(adapter) = adapters
        .iter()
        .filter(compatible)
        .find(|a| a.get_info().device_type != wgpu::DeviceType::Cpu)
    {
        return Ok(adapter.clone());
    }
    if let Some(adapter) = adapters.iter().find(compatible) {
        tracing::warn!(
            "wgpu: 実グラフィックスアダプタが見つからず、ソフトウェアアダプタへ \
             フォールバックします: {:?}",
            adapter.get_info()
        );
        return Ok(adapter.clone());
    }
    Err("互換性のある wgpu アダプタが見つかりませんでした".to_owned())
}

#[cfg(target_os = "windows")]
struct EnvDiagnosis {
    remote_session: bool,
    /// デスクトップに接続中のプライマリアダプタ名（`EnumDisplayDevicesW`）。
    /// 取得できなければ `None`。
    primary_adapter: Option<String>,
    /// `opengl32.dll` を今このプロセスから読み込めるか。`eframe::Error::Glutin`
    /// が `ErrorKind::NotFound` を報告した場合の裏取りに使う。
    opengl32_loadable: bool,
    /// `wgpu` が列挙できるアダプタがソフトウェア実装のみか（ロケールに
    /// 依存しない判定、[`is_effectively_fallback_driver`] 参照）。
    only_software_wgpu_adapters: bool,
}

#[cfg(target_os = "windows")]
fn env_report_lines(diag: &EnvDiagnosis) -> Vec<String> {
    vec![
        "── 検出した環境情報 ──".to_owned(),
        format!(
            "実行方式: {}",
            if diag.remote_session {
                "リモートデスクトップ接続"
            } else {
                "ローカル（コンソール）セッション"
            }
        ),
        format!(
            "グラフィックスアダプタ: {}",
            diag.primary_adapter
                .as_deref()
                .unwrap_or("取得できませんでした")
        ),
        format!(
            "opengl32.dll の読み込み: {}",
            if diag.opengl32_loadable {
                "成功"
            } else {
                "失敗"
            }
        ),
    ]
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn describe(app_name: &str, err: &eframe::Error) -> String {
    format!(
        "{}の起動に失敗しました。\n\nエラー詳細: {err}",
        app_label(app_name)
    )
}

/// Glow・wgpu どちらのレンダラーでも起動に失敗した場合の説明文字列を作る。
/// 対処法の中心は Glow(OpenGL) 側のエラーから [`advice_for`] で判別する
/// （wgpu は主に Glow 失敗時のフォールバックとして試すもので、直接の
/// エラー種別ごとの判別は実装していない）。ソフトウェアレンダラー
/// （wgpu の WARP フォールバック）まで失敗しているという事実自体が、
/// 通常のグラフィックドライバ不足を超えた、より深刻な環境異常
/// （Direct3D/Vulkan 関連のシステムファイル破損等）を示唆するため、
/// その旨も付記する。
#[cfg(target_os = "windows")]
fn describe_fallback_failure(
    app_name: &str,
    glow_err: &eframe::Error,
    wgpu_err: &eframe::Error,
) -> String {
    let diag = diagnose_environment();
    let mut lines = vec![
        format!(
            "{}の起動に失敗しました（OpenGL・wgpu 両方のレンダラーで失敗）。",
            app_label(app_name)
        ),
        String::new(),
        format!("OpenGL(glow) でのエラー: {glow_err}"),
        format!("wgpu(Direct3D/Vulkan、ソフトウェアフォールバック込み) でのエラー: {wgpu_err}"),
        String::new(),
    ];
    lines.extend(env_report_lines(&diag));
    lines.push(String::new());
    lines.push("── 対処方法 ──".to_owned());
    lines.push(advice_for(glow_err, &diag));
    lines.push(String::new());
    lines.push(
        "上記に加えて、実グラフィックスドライバが無くても通常は動作するはずの\n\
         ソフトウェアレンダラー(wgpu の WARP フォールバック)でも初期化に失敗して\n\
         います。上記の対処で解決しない場合は、通常のグラフィックドライバ不足を\n\
         超えた、より深刻な環境異常（Direct3D 12 関連のシステムファイル破損等）\n\
         の可能性があります。管理者権限のコマンドプロンプトで `sfc /scannow` を\n\
         実行してから再度お試しください。解決しない場合は awase-settings.log の\n\
         内容を添えて GitHub Issue でご報告いただけると助かります:\n\
         https://github.com/cuzic/awase/issues"
            .to_owned(),
    );
    lines.push(String::new());
    lines.push(
        "詳しいログは awase-settings.exe と同じフォルダの awase-settings.log に記録されています。"
            .to_owned(),
    );

    lines.join("\n")
}

/// 実際の原因ごとに対処法を出し分ける。`eframe::Error` の中身（`glutin`/`egui_glow`
/// が報告した理由）と、能動的に調べた実機の状況の両方を根拠にする。単一の
/// 固定文言に頼らず、根拠が無いケースでは「特定できなかった」ことを正直に示す。
#[cfg(target_os = "windows")]
fn advice_for(err: &eframe::Error, diag: &EnvDiagnosis) -> String {
    let remote_note = diag.remote_session.then(|| {
        "\n\nこの PC は現在リモートデスクトップ接続で操作されています。RDP 経由では\n\
         グラフィックス機能が制限される環境があるため、可能であれば PC の画面に\n\
         直接（コンソールセッションで）ログオンして再度お試しください。"
            .to_owned()
    });

    let body = match err {
        eframe::Error::Glutin(e)
            if e.error_kind() == glutin::error::ErrorKind::NotFound && !diag.opengl32_loadable =>
        {
            "opengl32.dll を読み込めませんでした。これは Windows に標準搭載されて\n\
             いるはずのファイルです。システムファイルが破損している可能性があります。\n\
             管理者権限のコマンドプロンプトで `sfc /scannow` を実行するか、\n\
             設定アプリの「Windows Update」→「トラブルシューティング」を試して\n\
             ください。"
                .to_owned()
        }
        eframe::Error::OpenGL(e) if e.to_string().contains("opengl 2.0") => driver_advice(diag),
        eframe::Error::NoGlutinConfigs(..) => driver_advice(diag),
        _ if is_effectively_fallback_driver(diag) => driver_advice(diag),
        _ => "上記のエラー内容から確定的な原因を特定できませんでした。\n\
              awase-settings.log の内容を添えて GitHub Issue でご報告いただけると\n\
              助かります: https://github.com/cuzic/awase/issues"
            .to_owned(),
    };

    match remote_note {
        Some(note) => format!("{body}{note}"),
        None => body,
    }
}

/// 検出したグラフィックスアダプタ名に応じた対処法を返す。
#[cfg(target_os = "windows")]
fn driver_advice(diag: &EnvDiagnosis) -> String {
    match (is_effectively_fallback_driver(diag), diag.primary_adapter.as_deref()) {
        (true, Some(name)) => format!(
            "検出されたグラフィックスアダプタは「{name}」で、実際の GPU ドライバー\n\
             ではなく Windows 標準のフォールバック表示ドライバーです。実際の GPU\n\
             ドライバーが読み込まれていないことが原因です。PC メーカーまたは GPU\n\
             メーカー（Intel / NVIDIA / AMD）の公式サイトから、お使いの機種向けの\n\
             最新のディスプレイドライバーをダウンロード・インストールしてください。"
        ),
        (true, None) => "実際の GPU ドライバーが読み込まれておらず、ソフトウェア\n\
                          レンダラーしか使えない状態です（wgpu で列挙できるアダプタが\n\
                          ソフトウェア実装のみでした）。PC メーカーまたは GPU メーカー\n\
                          （Intel / NVIDIA / AMD）の公式サイトから、お使いの機種向けの\n\
                          最新のディスプレイドライバーをダウンロード・インストールして\n\
                          ください。"
            .to_owned(),
        (false, Some(name)) => format!(
            "検出されたグラフィックスアダプタ「{name}」の描画機能が、awase 設定画面が\n\
             必要とする OpenGL 2.0 以上を満たしていません。Windows Update で\n\
             グラフィックスドライバーを更新するか、GPU メーカーの公式サイトから\n\
             最新版をインストールしてください。"
        ),
        (false, None) => "グラフィックスドライバーに問題がある可能性がありますが、アダプタ名を\n\
                           取得できませんでした。デバイスマネージャーの「ディスプレイ アダプター」\n\
                           を確認し、ドライバーを最新版に更新してください。"
            .to_owned(),
    }
}

/// 実 GPU ドライバーではなくフォールバック表示ドライバーだと判断できるか。
///
/// `EnumDisplayDevicesW` が返すアダプタ名の英語部分文字列一致
/// （`is_fallback_adapter_name`）だけに頼ると、日本語ロケールの Windows で
/// アダプタ名がローカライズされていた場合に判定を誤りうる。`wgpu` が
/// 列挙するアダプタの `DeviceType`（`Cpu` = ソフトウェア実装）はロケールに
/// 依存しない事実であるため、こちらを優先し、文字列一致は補助的な根拠として
/// OR で組み合わせる。
#[cfg(target_os = "windows")]
fn is_effectively_fallback_driver(diag: &EnvDiagnosis) -> bool {
    diag.only_software_wgpu_adapters
        || diag
            .primary_adapter
            .as_deref()
            .is_some_and(is_fallback_adapter_name)
}

/// Windows 標準のソフトウェア/仮想フォールバックドライバの既知の名称
/// （英語ロケール向けの補助的な判定、[`is_effectively_fallback_driver`] 参照）。
#[cfg(target_os = "windows")]
fn is_fallback_adapter_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("basic render")
        || lower.contains("basic display")
        || lower.contains("remote display")
}

/// `wgpu` が列挙できるアダプタが、ソフトウェア実装（`DeviceType::Cpu`、Windows
/// では DX12 の WARP）しか無いかどうか。ロケールに依存しない
/// （[`is_effectively_fallback_driver`] 参照）。1つもアダプタが無い場合は
/// 「ソフトウェアのみ」とは言い切れない（アダプタ列挙自体が失敗している別の
/// 問題の可能性がある）ため `false` を返す。
///
/// コードレビュー指摘: この関数は「Glow・wgpu 両方の実試行が既に失敗した後」
/// にのみ、診断専用の新しい `wgpu::Instance` を作って呼ばれる。つまり
/// 呼び出す時点で Direct3D/Vulkan 周りが壊れていることが分かっている環境
/// でのみ動く、いわば「壊れていると分かっている物をもう一度突く」処理で
/// あり、`wgpu::Instance::new`/`enumerate_adapters` 自体が panic する余地が
/// 万一あった場合、この関数を呼び出した [`diagnose_environment`] の
/// 呼び出し元（[`describe_fallback_failure`]）まで巻き込んで [`show_dialog`]
/// が一度も呼ばれずに終了しかねない。それはこのファイル全体の目的
/// （起動失敗を無言終了させずユーザーに見せる）を裏切ることになるため、
/// `catch_unwind` で隔離し、panic した場合は「ソフトウェアのみとは
/// 言い切れない」（＝ `false`、文字列ヒューリスティックへフォールバック）
/// として扱う。
#[cfg(target_os = "windows")]
fn only_software_wgpu_adapters_available() -> bool {
    std::panic::catch_unwind(|| {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            ..Default::default()
        });
        let adapters = instance.enumerate_adapters(wgpu::Backends::PRIMARY | wgpu::Backends::GL);
        !adapters.is_empty()
            && adapters
                .iter()
                .all(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
    })
    .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn diagnose_environment() -> EnvDiagnosis {
    EnvDiagnosis {
        remote_session: is_remote_session(),
        primary_adapter: primary_display_adapter_name(),
        opengl32_loadable: can_load_library("opengl32.dll"),
        only_software_wgpu_adapters: only_software_wgpu_adapters_available(),
    }
}

/// リモートデスクトップ接続で実行されているか（`GetSystemMetrics(SM_REMOTESESSION)`）。
#[cfg(target_os = "windows")]
fn is_remote_session() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};
    // SAFETY: 引数を取らない単純な照会 API。
    unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
}

/// デスクトップに接続中（`DISPLAY_DEVICE_ATTACHED_TO_DESKTOP`）のプライマリ
/// グラフィックスアダプタ名を `EnumDisplayDevicesW` で取得する。
#[cfg(target_os = "windows")]
fn primary_display_adapter_name() -> Option<String> {
    use windows::Win32::Graphics::Gdi::{
        DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICEW, EnumDisplayDevicesW,
    };
    use windows::core::PCWSTR;

    for index in 0..8u32 {
        let mut device = DISPLAY_DEVICEW {
            cb: u32::try_from(std::mem::size_of::<DISPLAY_DEVICEW>()).unwrap_or(0),
            ..Default::default()
        };
        // SAFETY: device.cb はこの構造体のサイズに設定済み。lpdevice=null により
        // ローカルマシンのグラフィックスアダプタを列挙する（ディスプレイモニタ側の
        // 列挙ではない）。
        let ok = unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &raw mut device, 0) };
        if !ok.as_bool() {
            break;
        }
        if device.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0 {
            let len = device
                .DeviceString
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(device.DeviceString.len());
            return Some(String::from_utf16_lossy(&device.DeviceString[..len]));
        }
    }
    None
}

/// 指定した DLL を今このプロセスから読み込めるかを確認する（読み込めた場合は
/// 直ちに解放し、DLL の参照カウントを変化させない）。
#[cfg(target_os = "windows")]
fn can_load_library(name: &str) -> bool {
    use windows::Win32::Foundation::FreeLibrary;
    use windows::Win32::System::LibraryLoader::LoadLibraryW;
    use windows::core::PCWSTR;

    let wide = awase_windows::win32::to_wide(name);
    // SAFETY: wide は NUL 終端済み UTF-16 で呼び出し中有効。
    unsafe {
        match LoadLibraryW(PCWSTR(wide.as_ptr())) {
            Ok(handle) => {
                let _ = FreeLibrary(handle);
                true
            }
            Err(_) => false,
        }
    }
}

/// 診断結果をメッセージボックスで表示する。
#[cfg(target_os = "windows")]
pub(crate) fn show_dialog(detail: &str) {
    awase_windows::win32::show_error_dialog("awase設定 - 起動エラー", detail);
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn show_dialog(_detail: &str) {}
