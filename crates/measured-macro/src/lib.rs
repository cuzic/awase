//! ADR-158 TE1: `#[measured(...)]`属性マクロ。
//!
//! [`.claude/rules/tuning-constants.md`](../../.claude/rules/tuning-constants.md)が求める
//! 「タイミング定数の変更には実測msをコミット本文に書け」という規約を、人力レビュー頼みから
//! マクロ展開時のコンパイルエラーへ格上げする（ADR-161実証実験6のスパイクを本実装化）。
//!
//! # 使い方
//!
//! 実測済みの定数:
//! ```ignore
//! #[measured(value_ms = 500, commit = "a6b4c0dd")]
//! pub const RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE: u64 = 500;
//! ```
//!
//! まだ実測（git考古学）を済ませていない定数（段階導入の猶予、ADR-158 TE1 round2 M-6）:
//! ```ignore
//! #[measured(pending = true)]
//! pub const SOME_CONST: u64 = 300;
//! ```
//!
//! `value_ms`/`commit`のペアと`pending = true`のどちらか一方が必須。両方欠けている場合、
//! または`value_ms`のみ・`commit`のみのように片方だけ指定した場合はコンパイルエラーになる。
//! `margin_ms`は任意（マージン込みの値の場合、実測最大値との差分を記録する目的、値の検証は
//! 行わない）。未知のキー（タイポ等）はコンパイルエラーにする。
//!
//! # `value_ms`と定数の実値との一致検証（2026-09-09、opus code review M2で追加）
//!
//! 当初の実装は`value_ms`を構文解析するだけで、対象の`const`宣言が持つ実際のリテラル値とは
//! 一度も突き合わせていなかった——つまり`value_ms = 500`と書いたまま定数の値を
//! `300`に変えても、コンパイルは変わらず通っていた。本バージョンは対象アイテムを`ItemConst`
//! として解析し、初期化式が単純な整数リテラルの場合に限り`value_ms`と一致するか検証する
//! （複雑な式・他定数参照の場合は検証をスキップし、値の変更自体は妨げない——この属性の
//! 目的は「値を動かすときに実測を強制する」ことであり、静的評価できない式を無理に評価
//! しようとはしない）。
//!
//! 属性自体は実行時には何もしない——生成される`const`宣言そのものは変更しない。将来的には
//! `crates/xtask-adr-evidence`型のsynベースのxtaskがこのメタデータを読み取り、
//! `.claude/rules/tuning-constants.md`の表を生成する用途を想定する。

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Expr, ExprLit, ItemConst, Lit, Token};

struct MeasuredArgs {
    value_ms: Option<syn::LitInt>,
    #[allow(dead_code)] // 検証はしないが、記録用メタデータとして受理する
    margin_ms: Option<syn::LitInt>,
    commit: Option<syn::LitStr>,
    pending: bool,
}

impl syn::parse::Parse for MeasuredArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut args = MeasuredArgs {
            value_ms: None,
            margin_ms: None,
            commit: None,
            pending: false,
        };
        let pairs: Punctuated<syn::MetaNameValue, Token![,]> = Punctuated::parse_terminated(input)?;
        for pair in pairs {
            let key = pair
                .path
                .get_ident()
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            match key.as_str() {
                "value_ms" => {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Int(i), ..
                    }) = pair.value
                    {
                        args.value_ms = Some(i);
                    } else {
                        return Err(syn::Error::new_spanned(
                            &pair,
                            "value_ms は整数リテラルで指定すること",
                        ));
                    }
                }
                "margin_ms" => {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Int(i), ..
                    }) = pair.value
                    {
                        args.margin_ms = Some(i);
                    } else {
                        return Err(syn::Error::new_spanned(
                            &pair,
                            "margin_ms は整数リテラルで指定すること",
                        ));
                    }
                }
                "commit" => {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }) = pair.value
                    {
                        args.commit = Some(s);
                    } else {
                        return Err(syn::Error::new_spanned(
                            &pair,
                            "commit は文字列リテラルで指定すること",
                        ));
                    }
                }
                "pending" => {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Bool(b), ..
                    }) = pair.value
                    {
                        args.pending = b.value;
                    } else {
                        return Err(syn::Error::new_spanned(
                            &pair,
                            "pending は真偽値リテラルで指定すること",
                        ));
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        &pair,
                        format!(
                            "未知のキー `{other}` です（value_ms/margin_ms/commit/pendingの \
                             いずれかのタイポではないか確認すること）"
                        ),
                    ));
                }
            }
        }
        Ok(args)
    }
}

#[proc_macro_attribute]
pub fn measured(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as MeasuredArgs);
    let item2: TokenStream2 = proc_macro2::TokenStream::from(item.clone());

    if args.pending {
        return quote! { #item2 }.into();
    }

    let Some(value_ms) = &args.value_ms else {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[measured(...)] には value_ms が必須です（.claude/rules/tuning-constants.md: \
             実測msを書かずに定数を変更してはならない）。まだ実測していない場合は \
             #[measured(pending = true)] を使うこと。",
        )
        .to_compile_error()
        .into();
    };
    if args.commit.is_none() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[measured(...)] には commit が必須です（実測がどのコミットで行われたかを \
             追跡できるようにする）。まだ実測していない場合は #[measured(pending = true)] \
             を使うこと。",
        )
        .to_compile_error()
        .into();
    }

    // value_ms と対象constの実際のリテラル値を突き合わせる（2026-09-09 M2）。
    // ItemConstとして解析できず、または初期化式が単純な整数リテラルでない場合は
    // 検証をスキップする（過剰な静的評価は行わない）。
    if let Ok(item_const) = syn::parse::<ItemConst>(item) {
        if let Expr::Lit(ExprLit {
            lit: Lit::Int(actual),
            ..
        }) = item_const.expr.as_ref()
        {
            let attr_value: i128 = value_ms
                .base10_parse()
                .expect("value_ms was validated as an integer literal during parsing");
            let actual_value: i128 = actual
                .base10_parse()
                .expect("const initializer literal must parse as an integer");
            if attr_value != actual_value {
                let const_name = &item_const.ident;
                return syn::Error::new_spanned(
                    &item_const,
                    format!(
                        "#[measured(value_ms = {attr_value}, ...)] が `{const_name}` の実際の \
                         値（{actual_value}）と一致しません。定数の値を変更した場合は \
                         value_ms も実測し直して更新すること（.claude/rules/\
                         tuning-constants.md）。"
                    ),
                )
                .to_compile_error()
                .into();
            }
        }
    }

    quote! { #item2 }.into()
}
