//! ADR-193/ADR-191 スパイク: 自前の TSF テキストストア（`ITextStoreACP` + `ITfContext`）。
//!
//! IME（TIP）が composition を開始する瞬間、各コールバック、ロック要求を**すべて時刻付きで記録**し、
//! 「リテラルか composition か」を文字の見た目でなく composition イベント（`ITfContextOwnerCompositionSink` の
//! `OnStartComposition` / `OnUpdateComposition` / `OnEndComposition`）で判定できるようにする。
//! 較正（キー×状態→効果の表）と、Chrome の TSF の癖（ロックの遅延・拒否、cold-start）の模擬に使う土台。
//!
//! 仕組み: 自前の窓（子の入力欄なし）に、自前の `ITextStoreACP` を載せた `ITfDocumentMgr` を
//! `ITfThreadMgr::AssociateFocus` で関連付ける。窓にフォーカスが来ると TIP がこのストアに接続し、
//! テキストの読み書き（`GetText` / `InsertTextAtSelection` / `SetText`）と composition を行う。
//!
//! 使い方: `text_store_probe [--seq=16,4B,41,0D] [--gap=1200] [--lock-delay=MS] [--deny-sync] [--no-scan] [--class=NAME] [--tail=MS] [--panic-test] [--log=<path>]`
//!   `--seq`: 注入する VK（16進）。既定は IME ON → `k` → `a` → Enter（composition を確定）。
//!   `--lock-delay`: `RequestLock` の同期応答を指定 ms 遅らせる（Chrome の遅いロックの模擬。0で無効）。
//!   `--deny-sync`: 同期ロック要求に `TS_E_SYNCHRONOUS` を返す（非同期のみ許す模擬）。
//!   `--no-scan`: 文字キーにスキャンコードを付けずに注入する（付けたときとの差を測る）。
//!   `--class=<クラス名>`: 窓のクラス名（既定 `TextStoreProbeTop`）。awase は分類をクラス名の文字列一致で行うので、
//!     `Chrome_RenderWidgetHostHWND` にすると awase から見て TsfNative（Vk 注入・per-VK confirm・literal 回収の経路）になる。
//!   `--tail=MS`: 最後のキーの後、終了までに待つ時間（既定600）。awase の literal 回収（ESC/BS の再送）を見るには数秒に伸ばす。
//!   `--panic-test`: `InsertTextAtSelection` で意図的に panic する（panic フックがタイムラインを残すかの確認用）。
//! キーは `SendInput`（`AWASE_TEST_INJECTION=1` の awase が物理キー扱いする目印付き）で注入する。
//! awase を止めた状態（IME 単体）が基本。前面窓がプローブ窓でないときは注入しない。
//! 実行中は Windows 機のキーボード・マウスに触らない。

#![allow(unsafe_code)]

#[cfg(windows)]
// `#[implement(...)]`（windows-rs）が生成する内部コードがこのリポジトリの pedantic/nursery deny に触れるため、
// マクロ生成部分にまとめて allow する（`spike_langbar_input_mode.rs` と同じ扱い）。
#[allow(clippy::ref_as_ptr, clippy::inline_always)]
mod store {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use windows::core::{implement, Interface, Ref, BOOL, GUID, HRESULT, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{E_NOTIMPL, HWND, POINT, RECT};
    use windows::Win32::System::Com::{IDataObject, FORMATETC};
    use windows::Win32::UI::TextServices::{
        ITextStoreACP, ITextStoreACPSink, ITextStoreACP_Impl, ITfCompositionView,
        ITfContextOwnerCompositionSink, ITfContextOwnerCompositionSink_Impl, ITfRange,
        TsActiveSelEnd, TEXT_STORE_LOCK_FLAGS, TS_AE_END, TS_ATTRVAL, TS_E_NOLOCK,
        TS_E_SYNCHRONOUS, TS_IAS_QUERYONLY, TS_RT_PLAIN, TS_RUNINFO, TS_SELECTIONSTYLE,
        TS_SELECTION_ACP, TS_STATUS, TS_S_ASYNC, TS_TEXTCHANGE,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    /// ストアの窓（`GetWnd` / `GetTextExt` / `GetScreenExt` で使う）。
    pub(crate) static TOP: AtomicIsize = AtomicIsize::new(0);

    #[derive(Clone, Debug)]
    pub(crate) struct Rec {
        pub(crate) at_ms: u128,
        pub(crate) tag: &'static str,
        pub(crate) detail: String,
    }
    pub(crate) type Log = Arc<Mutex<Vec<Rec>>>;

    pub(crate) fn push(log: &Log, t0: Instant, tag: &'static str, detail: String) {
        if let Ok(mut l) = log.lock() {
            l.push(Rec {
                at_ms: t0.elapsed().as_millis(),
                tag,
                detail,
            });
        }
    }

    #[derive(Default)]
    struct State {
        text: Vec<u16>,
        sel: (i32, i32),
        /// 選択の active end（`TsActiveSelEnd` の生値）。`SetSelection` で受け取ったものを `GetSelection` で返す。
        sel_ase: i32,
        sink: Option<ITextStoreACPSink>,
        /// 現在付与中のロック（0=なし）。
        locked: u32,
        /// ロック中に来た非同期のロック要求（現在のロックが終わってから付与する）。
        pending: Option<u32>,
    }

    #[implement(ITextStoreACP, ITfContextOwnerCompositionSink)]
    pub(crate) struct Store {
        st: RefCell<State>,
        pub(crate) log: Log,
        pub(crate) t0: Instant,
        pub(crate) lock_delay_ms: u64,
        pub(crate) deny_sync: bool,
        pub(crate) panic_test: bool,
    }

    impl Store {
        pub(crate) fn new(
            log: Log,
            t0: Instant,
            lock_delay_ms: u64,
            deny_sync: bool,
            panic_test: bool,
        ) -> Self {
            Self {
                st: RefCell::new(State {
                    sel_ase: TS_AE_END.0,
                    ..State::default()
                }),
                log,
                t0,
                lock_delay_ms,
                deny_sync,
                panic_test,
            }
        }
    }

    fn text_of(v: &[u16]) -> String {
        String::from_utf16_lossy(v)
    }

    impl Store_Impl {
        fn rec(&self, tag: &'static str, detail: String) {
            push(&self.log, self.t0, tag, detail);
        }

        /// 書き込み系メソッドの前提: 書き込みロックを持っていること。
        fn need_write(&self, name: &'static str) -> windows::core::Result<()> {
            let locked = self.st.borrow().locked;
            if locked & 4 == 0 {
                self.rec(
                    "ERR",
                    format!("{name}: 書き込みロックなし (locked=0x{locked:X})"),
                );
                return Err(TS_E_NOLOCK.into());
            }
            Ok(())
        }

        fn need_read(&self, name: &'static str) -> windows::core::Result<()> {
            let locked = self.st.borrow().locked;
            if locked & 2 == 0 {
                self.rec(
                    "ERR",
                    format!("{name}: 読み取りロックなし (locked=0x{locked:X})"),
                );
                return Err(TS_E_NOLOCK.into());
            }
            Ok(())
        }

        fn clamp(&self, p: i32) -> i32 {
            let len = i32::try_from(self.st.borrow().text.len()).unwrap_or(i32::MAX);
            p.clamp(0, len)
        }

        /// `[start, end)` を文字数の範囲に丸め、逆順なら入れ替えた `(start, end)` を返す。
        /// TIP が逆順の範囲を渡しても `Vec::splice` でパニックしない（COM コールバックの外へ unwind させない）。
        fn ordered(&self, a: i32, b: i32) -> (i32, i32) {
            let (a, b) = (self.clamp(a), self.clamp(b));
            (a.min(b), a.max(b))
        }
    }

    impl ITextStoreACP_Impl for Store_Impl {
        fn AdviseSink(
            &self,
            riid: *const GUID,
            punk: Ref<windows::core::IUnknown>,
            dwmask: u32,
        ) -> windows::core::Result<()> {
            // SAFETY: riid は呼び出し元(TSF)が用意した有効な GUID ポインタ。
            let iid = unsafe { riid.as_ref() }.copied().unwrap_or_default();
            self.rec("AdviseSink", format!("iid={iid:?} mask=0x{dwmask:X}"));
            if iid != ITextStoreACPSink::IID {
                return Err(windows::core::Error::from(
                    windows::Win32::Foundation::E_INVALIDARG,
                ));
            }
            let sink = punk.ok()?.cast::<ITextStoreACPSink>()?;
            self.st.borrow_mut().sink = Some(sink);
            Ok(())
        }

        fn UnadviseSink(&self, _punk: Ref<windows::core::IUnknown>) -> windows::core::Result<()> {
            self.rec("UnadviseSink", String::new());
            self.st.borrow_mut().sink = None;
            Ok(())
        }

        fn RequestLock(&self, dwlockflags: u32) -> windows::core::Result<HRESULT> {
            let sync = dwlockflags & 1 != 0;
            let write = dwlockflags & 4 != 0;
            self.rec(
                "RequestLock",
                format!(
                    "flags=0x{dwlockflags:X} ({}{})",
                    if sync { "SYNC " } else { "ASYNC " },
                    if write { "READWRITE" } else { "READ" }
                ),
            );
            if self.deny_sync && sync {
                self.rec(
                    "RequestLock",
                    "→ TS_E_SYNCHRONOUS (--deny-sync)".to_string(),
                );
                return Ok(TS_E_SYNCHRONOUS);
            }
            {
                let mut st = self.st.borrow_mut();
                if st.locked != 0 {
                    // ロック中の要求: 非同期なら現在のロック終了後に付与、同期は拒否。
                    if sync {
                        drop(st);
                        self.rec("RequestLock", "→ TS_E_SYNCHRONOUS (ロック中)".to_string());
                        return Ok(TS_E_SYNCHRONOUS);
                    }
                    // 複数の非同期要求は上書きせず、フラグの和にする（READWRITE は READ を含むので、どちらの要求も満たす）。
                    st.pending = Some(st.pending.unwrap_or(0) | (dwlockflags & !1));
                    drop(st);
                    self.rec(
                        "RequestLock",
                        "→ TS_S_ASYNC (ロック中、後で付与)".to_string(),
                    );
                    return Ok(TS_S_ASYNC);
                }
            }
            if self.lock_delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.lock_delay_ms));
            }
            let mut flags = dwlockflags & !1;
            // 最初の（呼び出し元の要求に対する）OnLockGranted の結果。TSF の仕様上、これが RequestLock の phrSession になる。
            let mut first_hr: Option<HRESULT> = None;
            loop {
                let sink = {
                    let mut st = self.st.borrow_mut();
                    st.locked = flags;
                    st.sink.clone()
                };
                self.rec("OnLockGranted", format!("flags=0x{flags:X}"));
                if let Some(sink) = sink {
                    // SAFETY: sink は AdviseSink で受け取った有効な COM 参照。呼び出し中に RefCell を借りない。
                    let r = unsafe { sink.OnLockGranted(TEXT_STORE_LOCK_FLAGS(flags)) };
                    let hr = r.as_ref().map_or_else(|e| e.code(), |()| HRESULT(0));
                    if hr.is_err() {
                        // 失敗を握りつぶすと、TIP 側の編集セッション失敗が正常に見えて測定を誤読する。
                        self.rec(
                            "OnLockGranted-ERR",
                            format!("flags=0x{flags:X} hr=0x{:08X}", hr.0),
                        );
                    }
                    first_hr.get_or_insert(hr);
                }
                let (next, text) = {
                    let mut st = self.st.borrow_mut();
                    st.locked = 0;
                    (st.pending.take(), text_of(&st.text))
                };
                self.rec("LockReleased", format!("text={text:?}"));
                match next {
                    Some(f) => flags = f,
                    None => break,
                }
            }
            Ok(first_hr.unwrap_or(HRESULT(0)))
        }

        fn GetStatus(&self) -> windows::core::Result<TS_STATUS> {
            self.rec("GetStatus", String::new());
            Ok(TS_STATUS {
                dwDynamicFlags: 0,
                dwStaticFlags: 8, // TS_SS_NOHIDDENTEXT
            })
        }

        fn QueryInsert(
            &self,
            acpteststart: i32,
            acptestend: i32,
            cch: u32,
            pacpresultstart: *mut i32,
            pacpresultend: *mut i32,
        ) -> windows::core::Result<()> {
            self.rec(
                "QueryInsert",
                format!("{acpteststart}..{acptestend} cch={cch}"),
            );
            let (s, e) = (self.clamp(acpteststart), self.clamp(acptestend));
            // SAFETY: 出力ポインタは呼び出し元が用意した有効な領域(null なら書かない)。
            unsafe {
                if !pacpresultstart.is_null() {
                    *pacpresultstart = s;
                }
                if !pacpresultend.is_null() {
                    *pacpresultend = e;
                }
            }
            Ok(())
        }

        fn GetSelection(
            &self,
            ulindex: u32,
            ulcount: u32,
            pselection: *mut TS_SELECTION_ACP,
            pcfetched: *mut u32,
        ) -> windows::core::Result<()> {
            self.need_read("GetSelection")?;
            let (s, e) = self.st.borrow().sel;
            let ase = self.st.borrow().sel_ase;
            self.rec(
                "GetSelection",
                format!("idx=0x{ulindex:X} count={ulcount} → {s}..{e}"),
            );
            // SAFETY: 出力ポインタは呼び出し元が用意した有効な領域。
            unsafe {
                if ulcount > 0 && !pselection.is_null() {
                    *pselection = TS_SELECTION_ACP {
                        acpStart: s,
                        acpEnd: e,
                        style: TS_SELECTIONSTYLE {
                            ase: TsActiveSelEnd(ase),
                            fInterimChar: BOOL(0),
                        },
                    };
                    if !pcfetched.is_null() {
                        *pcfetched = 1;
                    }
                } else if !pcfetched.is_null() {
                    *pcfetched = 0;
                }
            }
            Ok(())
        }

        fn SetSelection(
            &self,
            ulcount: u32,
            pselection: *const TS_SELECTION_ACP,
        ) -> windows::core::Result<()> {
            self.need_write("SetSelection")?;
            // SAFETY: pselection は ulcount 個の有効な要素を指す(ulcount>0 のとき)。
            if ulcount > 0 && !pselection.is_null() {
                let sel = unsafe { *pselection };
                let (s, e) = self.ordered(sel.acpStart, sel.acpEnd);
                self.rec("SetSelection", format!("{s}..{e} ase={}", sel.style.ase.0));
                let mut st = self.st.borrow_mut();
                st.sel = (s, e);
                st.sel_ase = sel.style.ase.0;
            }
            Ok(())
        }

        fn GetText(
            &self,
            acpstart: i32,
            acpend: i32,
            pchplain: PWSTR,
            cchplainreq: u32,
            pcchplainret: *mut u32,
            prgruninfo: *mut TS_RUNINFO,
            cruninforeq: u32,
            pcruninforet: *mut u32,
            pacpnext: *mut i32,
        ) -> windows::core::Result<()> {
            self.need_read("GetText")?;
            let end = if acpend == -1 {
                i32::try_from(self.st.borrow().text.len()).unwrap_or(0)
            } else {
                acpend
            };
            let (s, e) = (self.clamp(acpstart), self.clamp(end));
            let avail = usize::try_from((e - s).max(0)).unwrap_or(0);
            let n = avail.min(cchplainreq as usize);
            self.rec(
                "GetText",
                format!("{acpstart}..{acpend} req={cchplainreq} → {n}文字"),
            );
            // SAFETY: 出力バッファは呼び出し元が cchplainreq / cruninforeq 分を用意している。
            unsafe {
                if !pchplain.is_null() && n > 0 {
                    let st = self.st.borrow();
                    let start = usize::try_from(s).unwrap_or(0);
                    std::ptr::copy_nonoverlapping(
                        st.text[start..start + n].as_ptr(),
                        pchplain.0,
                        n,
                    );
                }
                if !pcchplainret.is_null() {
                    *pcchplainret = u32::try_from(n).unwrap_or(0);
                }
                let mut runs = 0u32;
                if cruninforeq > 0 && !prgruninfo.is_null() && n > 0 {
                    *prgruninfo = TS_RUNINFO {
                        uCount: u32::try_from(n).unwrap_or(0),
                        r#type: TS_RT_PLAIN,
                    };
                    runs = 1;
                }
                if !pcruninforet.is_null() {
                    *pcruninforet = runs;
                }
                if !pacpnext.is_null() {
                    *pacpnext = s + i32::try_from(n).unwrap_or(0);
                }
            }
            Ok(())
        }

        fn SetText(
            &self,
            dwflags: u32,
            acpstart: i32,
            acpend: i32,
            pchtext: &PCWSTR,
            cch: u32,
        ) -> windows::core::Result<TS_TEXTCHANGE> {
            self.need_write("SetText")?;
            let (s, e) = self.ordered(acpstart, acpend);
            // SAFETY: pchtext は cch 個の UTF-16 を指す(cch>0 のとき)。
            let new: Vec<u16> = if cch > 0 && !pchtext.is_null() {
                unsafe { std::slice::from_raw_parts(pchtext.0, cch as usize) }.to_vec()
            } else {
                Vec::new()
            };
            self.rec(
                "SetText",
                format!("flags=0x{dwflags:X} {s}..{e} ← {:?}", text_of(&new)),
            );
            let new_end = s + i32::try_from(new.len()).unwrap_or(0);
            let mut st = self.st.borrow_mut();
            let (us, ue) = (
                usize::try_from(s).unwrap_or(0),
                usize::try_from(e).unwrap_or(0),
            );
            st.text.splice(us..ue, new);
            st.sel = (new_end, new_end);
            Ok(TS_TEXTCHANGE {
                acpStart: s,
                acpOldEnd: e,
                acpNewEnd: new_end,
            })
        }

        fn GetFormattedText(&self, _s: i32, _e: i32) -> windows::core::Result<IDataObject> {
            self.rec("GetFormattedText", String::new());
            Err(E_NOTIMPL.into())
        }

        fn GetEmbedded(
            &self,
            _pos: i32,
            _service: *const GUID,
            _riid: *const GUID,
        ) -> windows::core::Result<windows::core::IUnknown> {
            self.rec("GetEmbedded", String::new());
            Err(E_NOTIMPL.into())
        }

        fn QueryInsertEmbedded(
            &self,
            _service: *const GUID,
            _fmt: *const FORMATETC,
        ) -> windows::core::Result<BOOL> {
            self.rec("QueryInsertEmbedded", String::new());
            Ok(BOOL(0))
        }

        fn InsertEmbedded(
            &self,
            _flags: u32,
            _s: i32,
            _e: i32,
            _obj: Ref<IDataObject>,
        ) -> windows::core::Result<TS_TEXTCHANGE> {
            self.rec("InsertEmbedded", String::new());
            Err(E_NOTIMPL.into())
        }

        fn InsertTextAtSelection(
            &self,
            dwflags: u32,
            pchtext: &PCWSTR,
            cch: u32,
            pacpstart: *mut i32,
            pacpend: *mut i32,
            pchange: *mut TS_TEXTCHANGE,
        ) -> windows::core::Result<()> {
            self.need_write("InsertTextAtSelection")?;
            // `--panic-test`: COM コールバック内の panic（非 unwind ABI の境界で abort になる）で、
            // panic フックがタイムラインを残すかを確かめるための意図的な panic。
            assert!(
                !self.panic_test,
                "--panic-test: InsertTextAtSelection で意図的に panic"
            );
            // TS_IAS_NOQUERY=0x1（通常の挿入）、TS_IAS_QUERYONLY=0x2（範囲の問い合わせのみ）。取り違えない。
            let query_only = dwflags & TS_IAS_QUERYONLY != 0;
            // SAFETY: pchtext は cch 個の UTF-16 を指す(cch>0 のとき)。
            let new: Vec<u16> = if cch > 0 && !pchtext.is_null() {
                unsafe { std::slice::from_raw_parts(pchtext.0, cch as usize) }.to_vec()
            } else {
                Vec::new()
            };
            let (s0, e0) = self.st.borrow().sel;
            let (s, e) = self.ordered(s0, e0);
            let new_end = s + i32::try_from(new.len()).unwrap_or(0);
            self.rec(
                "InsertTextAtSelection",
                format!(
                    "flags=0x{dwflags:X}{} sel={s}..{e} ← {:?}",
                    if query_only { " (QUERYONLY)" } else { "" },
                    text_of(&new)
                ),
            );
            if !query_only {
                let mut st = self.st.borrow_mut();
                let (us, ue) = (
                    usize::try_from(s).unwrap_or(0),
                    usize::try_from(e).unwrap_or(0),
                );
                st.text.splice(us..ue, new);
                st.sel = (new_end, new_end);
            }
            // SAFETY: 出力ポインタは呼び出し元が用意した有効な領域(null なら書かない)。
            unsafe {
                if !pacpstart.is_null() {
                    *pacpstart = s;
                }
                if !pacpend.is_null() {
                    *pacpend = new_end;
                }
                if !pchange.is_null() && !query_only {
                    *pchange = TS_TEXTCHANGE {
                        acpStart: s,
                        acpOldEnd: e,
                        acpNewEnd: new_end,
                    };
                }
            }
            Ok(())
        }

        fn InsertEmbeddedAtSelection(
            &self,
            _flags: u32,
            _obj: Ref<IDataObject>,
            _s: *mut i32,
            _e: *mut i32,
            _c: *mut TS_TEXTCHANGE,
        ) -> windows::core::Result<()> {
            self.rec("InsertEmbeddedAtSelection", String::new());
            Err(E_NOTIMPL.into())
        }

        fn RequestSupportedAttrs(
            &self,
            dwflags: u32,
            cfilterattrs: u32,
            _pafilterattrs: *const GUID,
        ) -> windows::core::Result<()> {
            self.rec(
                "RequestSupportedAttrs",
                format!("flags=0x{dwflags:X} n={cfilterattrs}"),
            );
            Ok(())
        }

        fn RequestAttrsAtPosition(
            &self,
            acppos: i32,
            cfilterattrs: u32,
            _pafilterattrs: *const GUID,
            _dwflags: u32,
        ) -> windows::core::Result<()> {
            self.rec(
                "RequestAttrsAtPosition",
                format!("pos={acppos} n={cfilterattrs}"),
            );
            Ok(())
        }

        fn RequestAttrsTransitioningAtPosition(
            &self,
            acppos: i32,
            cfilterattrs: u32,
            _pafilterattrs: *const GUID,
            _dwflags: u32,
        ) -> windows::core::Result<()> {
            self.rec(
                "RequestAttrsTransitioningAtPosition",
                format!("pos={acppos} n={cfilterattrs}"),
            );
            Ok(())
        }

        fn FindNextAttrTransition(
            &self,
            _acpstart: i32,
            acphalt: i32,
            _cfilterattrs: u32,
            _pafilterattrs: *const GUID,
            _dwflags: u32,
            pacpnext: *mut i32,
            pffound: *mut BOOL,
            plfoundoffset: *mut i32,
        ) -> windows::core::Result<()> {
            // SAFETY: 出力ポインタは呼び出し元が用意した有効な領域(null なら書かない)。
            unsafe {
                if !pacpnext.is_null() {
                    *pacpnext = acphalt;
                }
                if !pffound.is_null() {
                    *pffound = BOOL(0);
                }
                if !plfoundoffset.is_null() {
                    *plfoundoffset = 0;
                }
            }
            Ok(())
        }

        fn RetrieveRequestedAttrs(
            &self,
            _ulcount: u32,
            _paattrvals: *mut TS_ATTRVAL,
            pcfetched: *mut u32,
        ) -> windows::core::Result<()> {
            // SAFETY: 出力ポインタは呼び出し元が用意した有効な領域(null なら書かない)。
            unsafe {
                if !pcfetched.is_null() {
                    *pcfetched = 0;
                }
            }
            Ok(())
        }

        fn GetEndACP(&self) -> windows::core::Result<i32> {
            self.need_read("GetEndACP")?;
            let n = i32::try_from(self.st.borrow().text.len()).unwrap_or(0);
            self.rec("GetEndACP", format!("→ {n}"));
            Ok(n)
        }

        fn GetActiveView(&self) -> windows::core::Result<u32> {
            self.rec("GetActiveView", "→ 0".to_string());
            Ok(0)
        }

        fn GetACPFromPoint(
            &self,
            _vcview: u32,
            _pt: *const POINT,
            _flags: u32,
        ) -> windows::core::Result<i32> {
            self.rec("GetACPFromPoint", "→ 0".to_string());
            Ok(0)
        }

        fn GetTextExt(
            &self,
            _vcview: u32,
            acpstart: i32,
            acpend: i32,
            prc: *mut RECT,
            pfclipped: *mut BOOL,
        ) -> windows::core::Result<()> {
            self.rec("GetTextExt", format!("{acpstart}..{acpend}"));
            let top = HWND(TOP.load(Ordering::SeqCst) as *mut core::ffi::c_void);
            let mut wr = RECT::default();
            // SAFETY: top は自プロセスの有効な窓。出力ポインタは呼び出し元が用意した有効な領域。
            unsafe {
                let _ = GetWindowRect(top, &raw mut wr);
                if !prc.is_null() {
                    *prc = RECT {
                        left: wr.left + 10,
                        top: wr.top + 40,
                        right: wr.left + 30,
                        bottom: wr.top + 60,
                    };
                }
                if !pfclipped.is_null() {
                    *pfclipped = BOOL(0);
                }
            }
            Ok(())
        }

        fn GetScreenExt(&self, _vcview: u32) -> windows::core::Result<RECT> {
            self.rec("GetScreenExt", String::new());
            let top = HWND(TOP.load(Ordering::SeqCst) as *mut core::ffi::c_void);
            let mut wr = RECT::default();
            // SAFETY: top は自プロセスの有効な窓。
            unsafe {
                let _ = GetWindowRect(top, &raw mut wr);
            }
            Ok(wr)
        }

        fn GetWnd(&self, _vcview: u32) -> windows::core::Result<HWND> {
            self.rec("GetWnd", String::new());
            Ok(HWND(TOP.load(Ordering::SeqCst) as *mut core::ffi::c_void))
        }
    }

    impl ITfContextOwnerCompositionSink_Impl for Store_Impl {
        fn OnStartComposition(
            &self,
            _pcomposition: Ref<ITfCompositionView>,
        ) -> windows::core::Result<BOOL> {
            self.rec(
                "COMPOSITION-START",
                format!("text={:?}", text_of(&self.st.borrow().text)),
            );
            Ok(BOOL(1))
        }

        fn OnUpdateComposition(
            &self,
            _pcomposition: Ref<ITfCompositionView>,
            _prangenew: Ref<ITfRange>,
        ) -> windows::core::Result<()> {
            self.rec(
                "COMPOSITION-UPDATE",
                format!("text={:?}", text_of(&self.st.borrow().text)),
            );
            Ok(())
        }

        fn OnEndComposition(
            &self,
            _pcomposition: Ref<ITfCompositionView>,
        ) -> windows::core::Result<()> {
            self.rec(
                "COMPOSITION-END",
                format!("text={:?}", text_of(&self.st.borrow().text)),
            );
            Ok(())
        }
    }
}

#[cfg(windows)]
mod app {
    use std::io::Write as _;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use windows::core::{w, IUnknown, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY,
    };
    use windows::Win32::UI::TextServices::{
        CLSID_TF_InputProcessorProfiles, CLSID_TF_ThreadMgr, ITfContext,
        ITfInputProcessorProfileMgr, ITfThreadMgr, GUID_TFCAT_TIP_KEYBOARD,
        TF_INPUTPROCESSORPROFILE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow,
        GetMessageW, GetWindowThreadProcessId, PostMessageW, PostQuitMessage, RegisterClassExW,
        SetForegroundWindow, ShowWindow, TranslateMessage, CW_USEDEFAULT, MSG, SW_SHOW, WM_CHAR,
        WM_CLOSE, WM_DESTROY, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_NOTIFY,
        WM_IME_STARTCOMPOSITION, WM_INPUTLANGCHANGE, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
        WNDCLASSEXW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    use super::store::{push, Log, Rec, Store, TOP};

    /// 窓に届いた「素の」入力の集計（IME が composition にせず素通しした = リテラル、および awase の回収キー）。
    static LIT_CHARS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static BS_KEYS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static ESC_KEYS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// 窓に届いたメッセージを記録するための、ログと起点時刻(ウィンドウプロシージャから参照する)。
    static WND_LOG: std::sync::OnceLock<(Log, Instant)> = std::sync::OnceLock::new();

    /// `--no-scan` 指定時は文字キーにもスキャンコードを付けない（付けたときとの差を測るため）。
    static NO_SCAN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    fn scan_for(vk: u32) -> u16 {
        if NO_SCAN.load(Ordering::SeqCst) {
            return 0;
        }
        match vk {
            0x4B => 0x25, // K
            0x41 => 0x1E, // A
            0x0D => 0x1C, // Enter
            _ => 0,
        }
    }

    /// スパイク/chrome_probe と同じ目印。`AWASE_TEST_INJECTION=1` の awase は、この目印の注入を物理キーとして扱う。
    const AUTO_MARKER: usize = awase_windows::hook::TEST_INJECTION_MARKER;

    fn sleep(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn arg_value(args: &[String], key: &str) -> Option<String> {
        args.iter()
            .find_map(|a| a.strip_prefix(key).map(str::to_string))
    }

    fn send_key(vk: u32, down: bool) {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(u16::try_from(vk).unwrap_or(0)),
                    wScan: scan_for(vk),
                    dwFlags: if down {
                        KEYBD_EVENT_FLAGS(0)
                    } else {
                        KEYEVENTF_KEYUP
                    },
                    time: 0,
                    dwExtraInfo: AUTO_MARKER,
                },
            },
        };
        // SAFETY: input は有効な INPUT 1件。
        unsafe {
            let _ = SendInput(&[input], size_of::<INPUT>() as i32);
        }
    }

    fn bring_to_front(hwnd: HWND) -> bool {
        // SAFETY: Win32 の前面化 API。hwnd は自プロセスの有効なウィンドウ。
        unsafe {
            let fg = GetForegroundWindow();
            let fg_tid = if fg.0.is_null() {
                0
            } else {
                GetWindowThreadProcessId(fg, None)
            };
            let my_tid = GetCurrentThreadId();
            let attached = fg_tid != 0
                && fg_tid != my_tid
                && AttachThreadInput(my_tid, fg_tid, true).as_bool();
            let _ = BringWindowToTop(hwnd);
            let ok = SetForegroundWindow(hwnd).as_bool();
            if attached {
                let _ = AttachThreadInput(my_tid, fg_tid, false);
            }
            ok || GetForegroundWindow() == hwnd
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        // 届いたキー/IME メッセージを記録する(キーが窓に届いているか、IMM 経路の composition が来るかの切り分け用)。
        if let Some((log, t0)) = WND_LOG.get() {
            let name = match msg {
                WM_KEYDOWN => Some("WM_KEYDOWN"),
                WM_KEYUP => Some("WM_KEYUP"),
                WM_SYSKEYDOWN => Some("WM_SYSKEYDOWN"),
                WM_CHAR => Some("WM_CHAR"),
                WM_IME_STARTCOMPOSITION => Some("WM_IME_STARTCOMPOSITION"),
                WM_IME_COMPOSITION => Some("WM_IME_COMPOSITION"),
                WM_IME_ENDCOMPOSITION => Some("WM_IME_ENDCOMPOSITION"),
                WM_IME_NOTIFY => Some("WM_IME_NOTIFY"),
                WM_INPUTLANGCHANGE => Some("WM_INPUTLANGCHANGE"),
                _ => None,
            };
            if let Some(name) = name {
                push(
                    log,
                    *t0,
                    "WNDMSG",
                    format!("{name} wp=0x{:X} lp=0x{:X}", wp.0, lp.0),
                );
            }
            if msg == WM_CHAR && u8::try_from(wp.0).is_ok_and(|c| c.is_ascii_alphabetic()) {
                LIT_CHARS.fetch_add(1, Ordering::SeqCst);
            }
            if msg == WM_KEYDOWN && wp.0 == 0x08 {
                BS_KEYS.fetch_add(1, Ordering::SeqCst);
            }
            if msg == WM_KEYDOWN && wp.0 == 0x1B {
                ESC_KEYS.fetch_add(1, Ordering::SeqCst);
            }
        }
        // SAFETY: ウィンドウプロシージャ。DefWindowProcW/PostQuitMessage は任意の引数で安全。
        unsafe {
            if msg == WM_DESTROY {
                PostQuitMessage(0);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }

    /// panic フック: COM コールバック内の panic は非 unwind ABI（`extern "system"`）の境界でプロセスごと abort し、
    /// 終了時のタイムライン出力に到達しない。abort の前に走るこのフックで、panic の内容と直前までのタイムラインを
    /// ログファイルへ書き出し、測定データを失わないようにする。
    fn install_panic_hook(path: String, log: Log) {
        std::panic::set_hook(Box::new(move |info| {
            let mut s = format!("PANIC: {info}\n--- panic 直前までのタイムライン ---\n");
            // panic が push 中に起きた場合にデッドロックしないよう try_lock を使う。
            if let Ok(l) = log.try_lock() {
                for r in l.iter() {
                    s.push_str(&format!("+{:>6}ms {:<24} {}\n", r.at_ms, r.tag, r.detail));
                }
            } else {
                s.push_str("(ログのロックを取得できなかった)\n");
            }
            eprintln!("{s}");
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&path)
            {
                let _ = f.write_all(s.as_bytes());
            }
        }));
    }

    /// アクティブなキーボード TIP を「GJI / MS-IME / その他」で返す（測定結果がどの IME のものかを残すため）。
    fn active_tip() -> String {
        use windows::core::GUID;
        const GJI: GUID = GUID::from_u128(0xD5A86FD5_5308_47EA_AD16_9C4EB160EC3C);
        const MSIME: GUID = GUID::from_u128(0x03B5835F_F03C_411B_9CE2_AA23E1171E36);
        // SAFETY: STA スレッドで CoInitializeEx 済みの後に呼ぶ。GetActiveProfile は out 構造体に書き込む。
        unsafe {
            let mgr: windows::core::Result<ITfInputProcessorProfileMgr> =
                CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER);
            let Ok(mgr) = mgr else {
                return "取得失敗(ProfileMgr)".to_string();
            };
            let mut p = TF_INPUTPROCESSORPROFILE::default();
            if mgr
                .GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &raw mut p)
                .is_err()
            {
                return "取得失敗(GetActiveProfile)".to_string();
            }
            let kind = if p.clsid == GJI {
                "GJI"
            } else if p.clsid == MSIME {
                "MS-IME"
            } else {
                "その他"
            };
            format!("{kind} clsid={:?} langid=0x{:X}", p.clsid, p.langid)
        }
    }

    pub(crate) fn main() {
        let args: Vec<String> = std::env::args().collect();
        // 不正な要素や空の列は、測定と誤読されないよう黙って捨てずに終了する。
        let seq_arg = arg_value(&args, "--seq=").unwrap_or_else(|| "16,4B,41,0D".to_string());
        let mut seq: Vec<u32> = Vec::new();
        for tok in seq_arg.split(',') {
            match u32::from_str_radix(tok.trim(), 16) {
                Ok(v) => seq.push(v),
                Err(_) => {
                    eprintln!(
                        "--seq の要素 {tok:?} は16進のVKとして解釈できません(--seq={seq_arg})"
                    );
                    std::process::exit(2);
                }
            }
        }
        if seq.is_empty() {
            eprintln!("--seq が空です");
            std::process::exit(2);
        }
        let gap_ms: u64 = arg_value(&args, "--gap=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1200);
        let lock_delay_ms: u64 = arg_value(&args, "--lock-delay=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let deny_sync = args.iter().any(|a| a == "--deny-sync");
        let panic_test = args.iter().any(|a| a == "--panic-test");
        let class_name =
            arg_value(&args, "--class=").unwrap_or_else(|| "TextStoreProbeTop".to_string());
        let tail_ms: u64 = arg_value(&args, "--tail=")
            .and_then(|v| v.parse().ok())
            .unwrap_or(600);
        NO_SCAN.store(args.iter().any(|a| a == "--no-scan"), Ordering::SeqCst);
        let log_path = arg_value(&args, "--log=").unwrap_or_else(|| "text_store_probe.log".into());
        let mut file = std::fs::File::create(&log_path).expect("log");
        let mut out = |s: &str| {
            println!("{s}");
            let _ = writeln!(file, "{s}");
        };

        let t0 = Instant::now();
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let _ = WND_LOG.set((Arc::clone(&log), t0));
        install_panic_hook(log_path.clone(), Arc::clone(&log));

        // SAFETY: メインスレッド(STA)で COM/TSF と窓を初期化する。以降の COM 呼び出しは同じスレッドから行う。
        let (thread_mgr, _doc, _ctx) = unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .expect("CoInitializeEx");
            let thread_mgr: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)
                    .expect("ThreadMgr");
            let client_id = thread_mgr.Activate().expect("Activate");

            let instance = GetModuleHandleW(None).expect("module");
            let cls = wide(&class_name);
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(cls.as_ptr()),
                ..Default::default()
            };
            RegisterClassExW(&raw const wc);
            let top = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
                PCWSTR(cls.as_ptr()),
                w!("text store probe"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                600,
                200,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .expect("top");
            TOP.store(top.0 as isize, Ordering::SeqCst);
            let _ = ShowWindow(top, SW_SHOW);

            // 自前のテキストストアを載せたドキュメントを作り、窓に関連付ける。
            let store: IUnknown =
                Store::new(Arc::clone(&log), t0, lock_delay_ms, deny_sync, panic_test).into();
            let doc = thread_mgr.CreateDocumentMgr().expect("CreateDocumentMgr");
            let mut ctx: Option<ITfContext> = None;
            let mut edit_cookie = 0u32;
            doc.CreateContext(client_id, 0, &store, &raw mut ctx, &raw mut edit_cookie)
                .expect("CreateContext");
            let ctx = ctx.expect("ITfContext");
            doc.Push(&ctx).expect("Push");
            let _prev = thread_mgr
                .AssociateFocus(top, &doc)
                .expect("AssociateFocus");
            let _ = thread_mgr.SetFocus(&doc);
            push(
                &log,
                t0,
                "SETUP",
                "DocumentMgr/Context を作成し窓に関連付けた".to_string(),
            );
            (thread_mgr, doc, ctx)
        };
        out(&format!("アクティブTIP: {}", active_tip()));

        let worker_log = Arc::clone(&log);
        let worker = std::thread::spawn(move || {
            let top = HWND(TOP.load(Ordering::SeqCst) as *mut core::ffi::c_void);
            sleep(1500);
            let fronted = bring_to_front(top);
            sleep(800);
            'run: {
                // キー注入の前に、前面窓がプローブ窓であることを確かめる。
                if !fronted || unsafe { GetForegroundWindow() } != top {
                    push(
                        &worker_log,
                        t0,
                        "ABORT",
                        "前面化に失敗したためキーを注入しない".to_string(),
                    );
                    break 'run;
                }
                for vk in &seq {
                    if unsafe { GetForegroundWindow() } != top {
                        push(
                            &worker_log,
                            t0,
                            "ABORT",
                            format!("vk=0x{vk:02X} の前に前面が外れた"),
                        );
                        break 'run;
                    }
                    push(&worker_log, t0, "KEY", format!("0x{vk:02X}"));
                    send_key(*vk, true);
                    sleep(40);
                    send_key(*vk, false);
                    sleep(gap_ms);
                }
                sleep(tail_ms);
            }
            // SAFETY: top は有効な窓。WM_CLOSE でメインのメッセージループを終わらせる。
            unsafe {
                let _ = PostMessageW(Some(top), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        });

        // SAFETY: メインスレッドのメッセージループ。TSF のコールバックはここで配送される。
        unsafe {
            let mut msg = MSG::default();
            while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
        }
        let _ = worker.join();
        // SAFETY: Activate と対で、同じスレッドから呼ぶ。
        let _ = unsafe { thread_mgr.Deactivate() };

        let recs: Vec<Rec> = log.lock().map(|l| l.clone()).unwrap_or_default();
        out("--- タイムライン(ms は起動からの経過) ---");
        for r in &recs {
            out(&format!("+{:>6}ms {:<24} {}", r.at_ms, r.tag, r.detail));
        }
        out("--- 集計 ---");
        let count = |tag: &str| recs.iter().filter(|r| r.tag == tag).count();
        out(&format!(
            "KEY={} WNDMSG={} COMPOSITION-START={} COMPOSITION-UPDATE={} COMPOSITION-END={} RequestLock={} InsertTextAtSelection={} SetText={}",
            count("KEY"),
            count("WNDMSG"),
            count("COMPOSITION-START"),
            count("COMPOSITION-UPDATE"),
            count("COMPOSITION-END"),
            count("RequestLock"),
            count("InsertTextAtSelection"),
            count("SetText")
        ));
        out(&format!(
            "窓に届いた素の英字(WM_CHAR)={} BS(WM_KEYDOWN)={} ESC(WM_KEYDOWN)={}  ※IME が composition にせず素通しした=リテラル、BS/ESC は awase の回収の可能性",
            LIT_CHARS.load(Ordering::SeqCst),
            BS_KEYS.load(Ordering::SeqCst),
            ESC_KEYS.load(Ordering::SeqCst)
        ));
        let final_text = recs
            .iter()
            .rev()
            .find(|r| r.tag == "LockReleased")
            .map_or_else(String::new, |r| r.detail.clone());
        out(&format!("最後のロック解放時のテキスト: {final_text}"));
        out("=== 完了 ===");
    }
}

#[cfg(windows)]
fn main() {
    app::main();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows 専用のプローブです。");
}
