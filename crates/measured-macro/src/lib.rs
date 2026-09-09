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
//! `margin_ms`は任意（マージン込みの値の場合、実測最大値との差分を記録する目的）。
//!
//! 属性自体は実行時には何もしない——値はコンパイル時にのみ検証され、生成される`const`宣言は
//! 変更しない。将来的には`crates/xtask-adr-evidence`型のsynベースのxtaskがこのメタデータを
//! 読み取り、`.claude/rules/tuning-constants.md`の表を生成する用途を想定する。

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Token};

struct MeasuredArgs {
    value_ms: Option<syn::LitInt>,
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
                    if let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Int(i),
                        ..
                    }) = pair.value
                    {
                        args.value_ms = Some(i);
                    }
                }
                "margin_ms" => {
                    if let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Int(i),
                        ..
                    }) = pair.value
                    {
                        args.margin_ms = Some(i);
                    }
                }
                "commit" => {
                    if let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(s),
                        ..
                    }) = pair.value
                    {
                        args.commit = Some(s);
                    }
                }
                "pending" => {
                    if let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Bool(b),
                        ..
                    }) = pair.value
                    {
                        args.pending = b.value;
                    }
                }
                _ => {}
            }
        }
        Ok(args)
    }
}

#[proc_macro_attribute]
pub fn measured(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as MeasuredArgs);
    let item2: TokenStream2 = proc_macro2::TokenStream::from(item);

    if args.pending {
        return quote! { #item2 }.into();
    }

    if args.value_ms.is_none() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[measured(...)] には value_ms が必須です（.claude/rules/tuning-constants.md: \
             実測msを書かずに定数を変更してはならない）。まだ実測していない場合は \
             #[measured(pending = true)] を使うこと。",
        )
        .to_compile_error()
        .into();
    }
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

    quote! { #item2 }.into()
}
