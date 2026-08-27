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
//!   (`candidate_buttons()` の候補 GUID を順に試す)に対して購読する。
//!   言語バーのボタン表示自体が
//!   実際の composition mode のミラーであるため、こちらが本命——
//!   ひらがな⇔英数の切替(BUG-33/57 等の conv-mode belief 不整合の根本原因
//!   になっている領域)を push で捉えられる可能性がある。
//!
//! ## 実機での訂正(重要): `ITfLangBarItemMgr`/`ITfSource` は独立
//! `CoCreateInstance` では取得できない
//!
//! 当初 `CoCreateInstance(CLSID_TF_LangBarItemMgr, ...)` /
//! `CoCreateInstance(CLSID_TF_InputProcessorProfiles, ...).cast::<ITfSource>()`
//! で直接取得を試みたが、実機で前者は OS 標準の汎用4項目(言語/修正/
//! キーボード/ヘルプ)しか見えず GJI 固有の項目に届かず、後者は
//! `AdviseSink` が `CONNECT_E_CANNOTCONNECT`(0x80040202)で失敗した。
//! Microsoft Learn の該当ページ(`ITfInputProcessorProfileActivationSink`
//! Remarks、`ITfLangBarItemMgr` Remarks)を確認したところ、**どちらも
//! 「`ITfThreadMgr::QueryInterface` で取得すること」と明記**されていた
//! ——独立した `CoCreateInstance` は誤り。本スパイクは
//! `CoCreateInstance(CLSID_TF_ThreadMgr, ...)` → `ITfThreadMgr::Activate()`
//! → `.cast::<ITfLangBarItemMgr>()` / `.cast::<ITfSource>()` の順に修正した
//! (`with_thread_mgr`)。
//!
//! なお `ITfCompartmentEventSink`(GUID_COMPARTMENT_KEYBOARD_INPUTMODE_
//! CONVERSION 等)も同じ `ITfThreadMgr`(の `ITfCompartmentMgr`/グローバル
//! compartment)経由で成立する可能性が、この訂正によって再浮上している。
//! ただし対象は「無関係な外部プロセスの `ITfThreadMgr` を自分で `Activate`
//! した場合に、他プロセスの実際の compartment 値まで見えるか」という
//! 別の未検証の疑問(langbar/profile が広域ブロードキャストである保証は
//! あるが、per-document compartment が同様に共有されるかは別問題)のため、
//! 本スパイクでは上記2つの実機確認結果が出るまで着手しない。
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
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
    use windows::Win32::UI::TextServices::{
        CLSID_TF_ThreadMgr, ITfInputProcessorProfileActivationSink,
        ITfInputProcessorProfileActivationSink_Impl, ITfLangBarItem, ITfLangBarItemButton,
        ITfLangBarItemMgr, ITfLangBarItemSink, ITfLangBarItemSink_Impl, ITfMenu, ITfMenu_Impl,
        ITfSource, ITfThreadMgr, GUID_LBI_INPUTMODE, TF_LANGBARITEMINFO,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    /// GJI ビルドの「クラシック言語バー」入力モードボタン GUID(ポップアップ
    /// メニュー式、`InitMenu`/`ITfMenu` 経由でテキストが取れる)。
    /// `google/mozc` `src/win32/tip/tip_lang_bar.cc::kTipLangBarItem_Button`
    /// の `#ifdef GOOGLE_JAPANESE_INPUT_BUILD` 分岐値(2026-08 時点 master)。
    /// `{D8C8D5EB-8213-47CE-95B7-BA3F67757F94}`
    const GJI_INPUT_MODE_BUTTON: GUID = GUID::from_values(
        0xd8c8_d5eb,
        0x8213,
        0x47ce,
        [0x95, 0xb7, 0xba, 0x3f, 0x67, 0x75, 0x7f, 0x94],
    );

    /// 候補 GUID を優先順位付きで並べたもの。1つ目はクラシック言語バー
    /// ボタン(`kTipLangBarItem_Button`, メニュー式)、2つ目は Windows 8+
    /// の標準タスクバー入力モードアイコン(`GUID_LBI_INPUTMODE`, 全 TIP
    /// 共通の標準 GUID)。後者は mozc 側で `is_menu=false` として登録される
    /// ため `InitMenu` は no-op になるが、`OnMenuSelect` 自体は
    /// `IsMenuButton()` を見ずに動くため `--select` は両方で機能しうる
    /// (mozc ソース `TipLangBarToggleButton::OnMenuSelect` で確認済み)。
    fn candidate_buttons() -> [(&'static str, GUID); 2] {
        [
            (
                "kTipLangBarItem_Button(GJI固有・クラシック言語バー)",
                GJI_INPUT_MODE_BUTTON,
            ),
            (
                "GUID_LBI_INPUTMODE(標準・Win8+タスクバー入力モードアイコン)",
                GUID_LBI_INPUTMODE,
            ),
        ]
    }

    /// `mgr` から候補 GUID を順に試し、最初に見つかった
    /// `(名前, ITfLangBarItemButton)` を返す。
    fn find_button(mgr: &ITfLangBarItemMgr) -> Option<(&'static str, ITfLangBarItemButton)> {
        for (name, guid) in candidate_buttons() {
            // SAFETY: mgr は呼び出し元が生成した有効な COM 参照。
            if let Ok(item) = unsafe { mgr.GetItem(&raw const guid) } {
                if let Ok(button) = item.cast::<ITfLangBarItemButton>() {
                    return Some((name, button));
                }
            }
        }
        None
    }

    /// COM 初期化 → `ITfThreadMgr` 生成・`Activate()` → `f` 実行 →
    /// `Deactivate()`/`CoUninitialize()` の一連の面倒を見る。
    ///
    /// `ITfLangBarItemMgr`/`ITfSource`(profile activation sink 用)は
    /// Microsoft Learn によれば独立した `CoCreateInstance` ではなく
    /// **`ITfThreadMgr::QueryInterface`** で取得することが公式に指定されて
    /// いる。当初これを無視して独立 `CoCreateInstance` を試み、実機で
    /// (1) `ITfLangBarItemMgr` は OS 標準の汎用4項目しか見えず GJI 固有
    /// 項目に届かない、(2) `ITfSource::AdviseSink` が
    /// `CONNECT_E_CANNOTCONNECT` で失敗、の2件の実機失敗を確認した後に
    /// この訂正へ至った(モジュール doc 参照)。
    fn with_thread_mgr<F: FnOnce(&ITfThreadMgr) -> anyhow::Result<()>>(f: F) -> anyhow::Result<()> {
        // SAFETY: プロセス起動直後、他に COM 呼び出しが走っていない状態での
        // 単発初期化。
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;

        let result = (|| -> anyhow::Result<()> {
            // SAFETY: 標準的な in-proc/local COM オブジェクト生成。
            let thread_mgr: ITfThreadMgr =
                unsafe { CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_ALL) }?;
            // SAFETY: thread_mgr は上で生成した有効な COM 参照。Activate は
            // ITfLangBarItemMgr/ITfSource 経由の以降の呼び出し全てに必要。
            let client_id = unsafe { thread_mgr.Activate() }?;
            println!("ITfThreadMgr::Activate: OK (client_id={client_id})");

            let inner_result = f(&thread_mgr);

            // SAFETY: thread_mgr は有効な COM 参照。Activate の成功に対して
            // 1回だけ呼ぶ(エラーでも後片付けとして必ず試みる)。
            let _ = unsafe { thread_mgr.Deactivate() };

            inner_result
        })();

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
        with_thread_mgr(|thread_mgr| {
            let mgr: ITfLangBarItemMgr = thread_mgr.cast()?;
            println!("ITfLangBarItemMgr(via ITfThreadMgr::cast): OK");

            if let Some((name, button)) = find_button(&mgr) {
                println!("GetItem: OK ({name})");
                let menu: ITfMenu = MenuRecorder.into();
                // SAFETY: button/menu は共に有効な COM 参照。InitMenu 自体は
                // メニュー項目を列挙するだけで実モードには触れない(副作用が
                // ないことは mozc ソース `TipLangBarButton::InitMenu`/
                // `TipLangBarToggleButton::InitMenu` で確認済み)。
                // GUID_LBI_INPUTMODE 側は is_menu=false のため no-op になり
                // [menu] 行が1つも出ない可能性がある(それ自体は異常ではない)。
                match unsafe { button.InitMenu(&menu) } {
                    Ok(()) => println!(
                        "InitMenu: OK (上記 [menu] 行の uid/text 対応を確認すること。\
                         0行ならこのボタンは is_menu=false の可能性が高い)"
                    ),
                    Err(e) => println!("InitMenu failed: {e:?}(is_menu=false なら想定内)"),
                }

                if let Some(uid) = select_uid {
                    println!("--select={uid} 指定あり。OnMenuSelect({uid}) を実行します。");
                    // SAFETY: button は上で取得した有効な COM 参照。
                    let r = unsafe { button.OnMenuSelect(uid) };
                    println!("OnMenuSelect({uid}) -> {r:?}");
                    // `SwitchInputModeAsync`(mozc 側)は edit session 経由の非同期
                    // 実行のため、OnMenuSelect の戻り値が返った時点ではまだ実際の
                    // モード切替が完了していない可能性がある。プロセスをすぐ終了
                    // (CoUninitialize)すると、対象アプリ側の edit session が完了
                    // する前に呼び出し元 COM アパートメントが消え、非同期処理が
                    // 中断される懸念があるため、数秒間メッセージポンプを回してから
                    // 終了する。
                    println!(
                        "edit session の非同期完了を待つため3秒間メッセージポンプを回します..."
                    );
                    pump_messages_for(Duration::from_secs(3));
                    println!(
                        "→ 半角英数にしたい入力欄で実際に打鍵して確認してください(読み返しは信用しない)。"
                    );
                } else {
                    println!(
                        "--select=<uid> 未指定のためダンプのみで終了します。半角英数の uid を確認したら再実行してください。"
                    );
                }
            } else {
                println!("候補 GUID どちらも GetItem 失敗");
                dump_all_items(&mgr)?;
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
        with_thread_mgr(|thread_mgr| {
            let source: ITfSource = thread_mgr.cast()?;
            println!("ITfSource(via ITfThreadMgr::cast): OK");
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
        with_thread_mgr(|thread_mgr| {
            let mgr: ITfLangBarItemMgr = thread_mgr.cast()?;
            println!("ITfLangBarItemMgr(via ITfThreadMgr::cast): OK");
            let Some((name, button)) = find_button(&mgr) else {
                anyhow::bail!("候補 GUID どちらも GetItem 失敗(dump モードで先に確認すること)");
            };
            println!("GetItem: OK ({name})");
            let guid = candidate_buttons()
                .into_iter()
                .find(|(n, _)| *n == name)
                .map(|(_, g)| g)
                .expect("find_button が返した name は candidate_buttons 由来");
            let sink: ITfLangBarItemSink = LangBarUpdateLogger { item: button }.into();
            let mut cookie = 0u32;
            // SAFETY: mgr は有効な COM 参照。cookie はこのスコープの
            // ローカル変数。
            unsafe { mgr.AdviseItemSink(&sink, &raw mut cookie, &raw const guid) }?;
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
