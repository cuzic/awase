//! BUG-25(GJI 半角英数トグル)の第4の候補経路、および TSF の公開 COM
//! push-notification インタフェースが「外部プロセスから他プロセスの IME
//! 状態を観測する」ための新しい情報源になり得るかを実機検証する使い捨て
//! スパイク。
//!
//! ## アクチュエーション側(`--select`)
//!
//! `ITfLangBarItemButton::OnMenuSelect` は、mozc(Google 日本語入力のオープン
//! ソース版)のソース(`src/win32/tip/tip_text_service.cc::OnMenuSelect`)で
//! `TipEditSession::SwitchInputModeAsync` を直接呼ぶ経路として確認されている
//! ——物理 Eisu キー押下が最終的に到達するのと同じ実効経路であり、
//! `SendInput` 注入とも IMC/compartment write とも異なる。過去3回
//! (`docs/known-bugs.md` BUG-25 追補1〜3)は SendInput 注入(scan 値問わず)
//! と IMC/compartment write を試して全て失敗しており、これは「今まで一度も
//! 試していない第4の経路」。
//!
//! ## 観測側(`--watch-profile` / `--watch-langbar`)
//!
//! 「外部の常駐アプリが別プロセスの IME 状態を観測できるか」を2つの独立した
//! push 通知経路で検証する。両方とも Windows XP 時代からある**公開・文書化
//! された** TSF COM インタフェースで、mozc の私的 IPC を模倣するものではない
//! (タスクバーの言語バー/IME インジケータ UI 自体がこれらを使っている)。
//!
//! - `--watch-profile`: `ITfInputProcessorProfileActivationSink` を
//!   `ITfInputProcessorProfiles`(`ITfSource::AdviseSink`)経由で購読する。
//!   これは**プロファイル(TIP/言語)の切り替え**に反応する通知であり、
//!   `crates/awase-windows/src/tsf/observer.rs` が既に2秒ポーリングで読んで
//!   いる `ITfInputProcessorProfileMgr::GetActiveProfile`(`tsf_active_kind`)
//!   と同じ情報を push 化できるか確認する。**GJI 内部のひらがな⇔英数の
//!   モード切替では発火しない見込み**(プロファイル=TIP identity の切替専用
//!   のはずなので、過度な期待をしないこと)。
//! - `--watch-langbar`: `ITfLangBarItemSink` を
//!   `ITfLangBarItemMgr::AdviseItemSink` 経由で GJI の入力モードボタン
//!   (`GJI_INPUT_MODE_BUTTON`)に対して購読する。言語バーのボタン表示自体が
//!   実際の composition mode のミラーであるため、こちらが本命——
//!   ひらがな⇔英数の切替(BUG-33/57 等の conv-mode belief 不整合の根本原因
//!   になっている領域)を push で捉えられる可能性がある。
//!
//! なお `ITfCompartmentEventSink`(GUID_COMPARTMENT_KEYBOARD_INPUTMODE_
//! CONVERSION 等)は、対象アプリ自身の `ITfThreadMgr`/`ITfDocumentMgr` に
//! 紐づくスコープのため、無関係な外部プロセスから `CoCreateInstance` で
//! 直接その compartment を掴む経路が存在しない(自分自身の空の
//! ThreadMgr が返るだけ)。本スパイクでは意図的に対象外にしている——
//! 上記2つ(langbar/profile)がどちらも cross-process で機能することが
//! 実機で先に確認できてから、compartment 側の再検討価値を判断する。
//!
//! # 実行方法(必ず実機)
//!
//! ```powershell
//! # 1. アクチュエーション: まずダンプのみ、次に --select=<uid>
//! cargo run -p awase-windows --example spike_langbar_input_mode --release
//! cargo run -p awase-windows --example spike_langbar_input_mode --release -- --select=4
//!
//! # 2. 観測: それぞれ既定30秒間 advise した状態で待機する。実行中に
//! #    物理的に IME 切替 / GJI モード切替を行い、コンソールに [profile-
//! #    activated] / [langbar-update] 行が出るか確認する。
//! cargo run -p awase-windows --example spike_langbar_input_mode --release -- --watch-profile
//! cargo run -p awase-windows --example spike_langbar_input_mode --release -- --watch-langbar=60
//! ```
//!
//! 実機結果が出たら `docs/known-bugs.md` BUG-25 追補4として記録する。

#![allow(unsafe_code)]

#[cfg(windows)]
// `#[implement(...)]`(windows-rs)が生成する内部コード
// (`from`/`as_interface_ref`/`as_impl_ptr` の `#[inline(always)]`、
// `&raw` ではなく `as *const _` を使う箇所)がこのリポジトリの
// pedantic/nursery deny 設定に触れるため、マクロ生成部分にまとめて allow する。
#[allow(clippy::ref_as_ptr, clippy::inline_always)]
mod langbar_probe {
    use std::time::{Duration, Instant};

    use windows::core::{implement, Interface, GUID, PCWSTR};
    use windows::Win32::Graphics::Gdi::HBITMAP;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
    use windows::Win32::UI::TextServices::{
        CLSID_TF_InputProcessorProfiles, CLSID_TF_LangBarItemMgr,
        ITfInputProcessorProfileActivationSink, ITfInputProcessorProfileActivationSink_Impl,
        ITfInputProcessorProfiles, ITfLangBarItem, ITfLangBarItemButton, ITfLangBarItemMgr,
        ITfLangBarItemSink, ITfLangBarItemSink_Impl, ITfMenu, ITfMenu_Impl, ITfSource,
        TF_LANGBARITEMINFO,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    /// GJI ビルドの入力モードボタン GUID。
    /// `google/mozc` `src/win32/tip/tip_lang_bar.cc::kTipLangBarItem_Button`
    /// の `#ifdef GOOGLE_JAPANESE_INPUT_BUILD` 分岐値(2026-08 時点 master)。
    /// `{D8C8D5EB-8213-47CE-95B7-BA3F67757F94}`
    const GJI_INPUT_MODE_BUTTON: GUID = GUID::from_values(
        0xd8c8_d5eb,
        0x8213,
        0x47ce,
        [0x95, 0xb7, 0xba, 0x3f, 0x67, 0x75, 0x7f, 0x94],
    );

    fn with_com<F: FnOnce() -> anyhow::Result<()>>(f: F) -> anyhow::Result<()> {
        // SAFETY: プロセス起動直後、他に COM 呼び出しが走っていない状態での
        // 単発初期化。
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
        let result = f();
        // SAFETY: 直前の CoInitializeEx 成功に対して1回だけ呼ぶ。
        unsafe { CoUninitialize() };
        result
    }

    /// メッセージポンプ(STA COM のコールバック配送に必須)を `duration` の
    /// 間だけ回す。50ms 刻みでポーリングしつつ、来ているメッセージは即座に
    /// 処理する。
    fn pump_messages_for(duration: Duration) {
        let deadline = Instant::now() + duration;
        let mut msg = MSG::default();
        while Instant::now() < deadline {
            // SAFETY: msg はこのスコープでのみ生存するローカル変数。
            // hwnd=None でこのスレッドの全ウィンドウ/スレッドメッセージを対象にする。
            while unsafe { PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                let _ = unsafe { TranslateMessage(&raw const msg) };
                unsafe { DispatchMessageW(&raw const msg) };
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    // ---------------------------------------------------------------
    // アクチュエーション側: ITfMenu 経由のダンプ + OnMenuSelect
    // ---------------------------------------------------------------

    /// `ITfMenu` を実装し、`InitMenu` から渡ってくる `(uid, text)` を
    /// そのまま標準出力へ記録するだけの受け皿。副作用は一切持たない。
    #[implement(ITfMenu)]
    struct MenuRecorder;

    impl ITfMenu_Impl for MenuRecorder_Impl {
        fn AddMenuItem(
            &self,
            uid: u32,
            dwflags: u32,
            _hbmp: HBITMAP,
            _hbmpmask: HBITMAP,
            pch: &PCWSTR,
            cch: u32,
            _ppmenu: windows::core::OutRef<'_, ITfMenu>,
        ) -> windows::core::Result<()> {
            // SAFETY: pch/cch は呼び出し元(mozc の TIP DLL が実装する
            // ITfLangBarItemButton::InitMenu)が用意した有効な UTF-16
            // バッファで、この COM コールバックの実行中のみ有効。
            let text = unsafe {
                if pch.0.is_null() || cch == 0 {
                    String::new()
                } else {
                    String::from_utf16_lossy(std::slice::from_raw_parts(pch.0, cch as usize))
                }
            };
            println!("[menu] uid={uid} flags=0x{dwflags:X} text={text:?}");
            Ok(())
        }
    }

    fn dump_all_items(mgr: &ITfLangBarItemMgr) -> anyhow::Result<()> {
        println!("--- known GUID 不一致 → EnumItems 全件ダンプ ---");
        // SAFETY: mgr は呼び出し元で生成済みの有効な COM 参照。
        let en = unsafe { mgr.EnumItems() }?;
        loop {
            let mut buf: [Option<ITfLangBarItem>; 1] = [None];
            let mut fetched = 0u32;
            // SAFETY: buf/fetched はこのスコープでのみ生存するローカル変数。
            let _ = unsafe { en.Next(&mut buf, &raw mut fetched) };
            if fetched == 0 {
                break;
            }
            let Some(item) = buf[0].take() else { break };
            let mut info = TF_LANGBARITEMINFO::default();
            // SAFETY: info はこのスコープでのみ生存するローカル変数。
            if unsafe { item.GetInfo(&raw mut info) }.is_ok() {
                let desc_len = info
                    .szDescription
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(info.szDescription.len());
                let desc = String::from_utf16_lossy(&info.szDescription[..desc_len]);
                println!(
                    "guidItem={{{:08X}-{:04X}-{:04X}-...}} desc={desc:?}",
                    info.guidItem.data1, info.guidItem.data2, info.guidItem.data3
                );
            }
        }
        Ok(())
    }

    pub(crate) fn run(select_uid: Option<u32>) -> anyhow::Result<()> {
        with_com(|| {
            // SAFETY: 標準的な in-proc COM オブジェクト生成。
            let mgr: ITfLangBarItemMgr =
                unsafe { CoCreateInstance(&CLSID_TF_LangBarItemMgr, None, CLSCTX_ALL) }?;
            println!("ITfLangBarItemMgr: OK");

            // SAFETY: mgr は上で生成した有効な COM 参照。
            match unsafe { mgr.GetItem(&GJI_INPUT_MODE_BUTTON) } {
                Ok(item) => {
                    println!("GetItem(GJI_INPUT_MODE_BUTTON): OK");
                    let button: ITfLangBarItemButton = item.cast()?;
                    let menu: ITfMenu = MenuRecorder.into();
                    // SAFETY: button/menu は共に有効な COM 参照。InitMenu 自体は
                    // メニュー項目を列挙するだけで実モードには触れない(副作用が
                    // ないことは mozc ソース `TipLangBarButton::InitMenu`/
                    // `TipLangBarToggleButton::InitMenu` で確認済み)。
                    unsafe { button.InitMenu(&menu) }?;
                    println!("InitMenu: OK (上記 [menu] 行の uid/text 対応を確認すること)");

                    match select_uid {
                        Some(uid) => {
                            println!("--select={uid} 指定あり。OnMenuSelect({uid}) を実行します。");
                            // SAFETY: button は上で取得した有効な COM 参照。
                            let r = unsafe { button.OnMenuSelect(uid) };
                            println!("OnMenuSelect({uid}) -> {r:?}");
                            println!(
                                "→ 半角英数にしたい入力欄で実際に打鍵して確認してください(読み返しは信用しない)。"
                            );
                        }
                        None => {
                            println!(
                                "--select=<uid> 未指定のためダンプのみで終了します。半角英数の uid を確認したら再実行してください。"
                            );
                        }
                    }
                }
                Err(e) => {
                    println!("GetItem(GJI_INPUT_MODE_BUTTON) failed: {e:?}");
                    dump_all_items(&mgr)?;
                }
            }

            Ok(())
        })
    }

    // ---------------------------------------------------------------
    // 観測側その1: ITfInputProcessorProfileActivationSink
    // ---------------------------------------------------------------

    #[implement(ITfInputProcessorProfileActivationSink)]
    struct ProfileActivationLogger;

    impl ITfInputProcessorProfileActivationSink_Impl for ProfileActivationLogger_Impl {
        fn OnActivated(
            &self,
            dwprofiletype: u32,
            langid: u16,
            clsid: *const GUID,
            catid: *const GUID,
            guidprofile: *const GUID,
            hkl: HKL,
            dwflags: u32,
        ) -> windows::core::Result<()> {
            // SAFETY: clsid/catid/guidprofile はこのコールバックの実行中のみ
            // 有効な、呼び出し元(TSF ランタイム)が用意したポインタ。
            let clsid = unsafe { clsid.as_ref() };
            let catid = unsafe { catid.as_ref() };
            let guidprofile = unsafe { guidprofile.as_ref() };
            println!(
                "[profile-activated] type=0x{dwprofiletype:X} langid=0x{langid:X} \
                 clsid={clsid:?} catid={catid:?} guidprofile={guidprofile:?} \
                 hkl={hkl:?} flags=0x{dwflags:X}"
            );
            Ok(())
        }
    }

    pub(crate) fn watch_profile(seconds: u32) -> anyhow::Result<()> {
        with_com(|| {
            // SAFETY: 標準的な in-proc COM オブジェクト生成。
            let profiles: ITfInputProcessorProfiles = unsafe {
                CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_ALL)
            }?;
            let source: ITfSource = profiles.cast()?;
            let sink: ITfInputProcessorProfileActivationSink = ProfileActivationLogger.into();
            // SAFETY: source は上で取得した有効な COM 参照。IID は静的定数。
            let cookie = unsafe {
                source.AdviseSink(
                    &<ITfInputProcessorProfileActivationSink as Interface>::IID,
                    &sink,
                )
            }?;
            println!("AdviseSink(ITfInputProcessorProfileActivationSink): cookie={cookie}");
            println!(
                "{seconds}秒間待機します。この間に IME プロファイルを切り替えて \
                 ください(GJI⇔MS-IME、日本語⇔英語キーボード等)。GJI 内部の \
                 ひらがな⇔英数モード切替では発火しない可能性が高い点に注意。"
            );
            pump_messages_for(Duration::from_secs(u64::from(seconds)));
            // SAFETY: cookie は上で取得した有効な advise cookie。
            unsafe { source.UnadviseSink(cookie) }?;
            println!("UnadviseSink done.");
            Ok(())
        })
    }

    // ---------------------------------------------------------------
    // 観測側その2: ITfLangBarItemSink(本命——composition mode の push 通知)
    // ---------------------------------------------------------------

    #[implement(ITfLangBarItemSink)]
    struct LangBarUpdateLogger {
        item: ITfLangBarItemButton,
    }

    impl ITfLangBarItemSink_Impl for LangBarUpdateLogger_Impl {
        fn OnUpdate(&self, dwflags: u32) -> windows::core::Result<()> {
            println!("[langbar-update] flags=0x{dwflags:X}");
            // SAFETY: self.item は保持している有効な COM 参照。
            if let Ok(text) = unsafe { self.item.GetText() } {
                println!("[langbar-update] current text={text}");
            }
            Ok(())
        }
    }

    pub(crate) fn watch_langbar(seconds: u32) -> anyhow::Result<()> {
        with_com(|| {
            // SAFETY: 標準的な in-proc COM オブジェクト生成。
            let mgr: ITfLangBarItemMgr =
                unsafe { CoCreateInstance(&CLSID_TF_LangBarItemMgr, None, CLSCTX_ALL) }?;
            // SAFETY: mgr は上で生成した有効な COM 参照。
            let item = unsafe { mgr.GetItem(&GJI_INPUT_MODE_BUTTON) }?;
            let button: ITfLangBarItemButton = item.cast()?;
            let sink: ITfLangBarItemSink = LangBarUpdateLogger { item: button }.into();
            let mut cookie = 0u32;
            // SAFETY: mgr は有効な COM 参照。cookie はこのスコープの
            // ローカル変数。
            unsafe { mgr.AdviseItemSink(&sink, &raw mut cookie, &GJI_INPUT_MODE_BUTTON) }?;
            println!("AdviseItemSink(ITfLangBarItemSink): cookie={cookie}");
            println!(
                "{seconds}秒間待機します。この間に GJI の入力モードを \
                 物理 Eisu キー等で切り替えてください。"
            );
            pump_messages_for(Duration::from_secs(u64::from(seconds)));
            // SAFETY: mgr/cookie は上で取得した有効な組。
            unsafe { mgr.UnadviseItemSink(cookie) }?;
            println!("UnadviseItemSink done.");
            Ok(())
        })
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let watch_arg = |flag: &str| -> Option<u32> {
        args.iter().find_map(|a| {
            a.strip_prefix(flag).map(|rest| {
                rest.strip_prefix('=')
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(30)
            })
        })
    };

    if let Some(secs) = watch_arg("--watch-profile") {
        return langbar_probe::watch_profile(secs);
    }
    if let Some(secs) = watch_arg("--watch-langbar") {
        return langbar_probe::watch_langbar(secs);
    }

    let select_uid = args
        .iter()
        .find_map(|a| a.strip_prefix("--select=").map(str::to_owned))
        .and_then(|s| s.parse::<u32>().ok());
    langbar_probe::run(select_uid)
}

#[cfg(not(windows))]
fn main() {}
