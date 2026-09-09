//! ADR-158 TE3前提サブタスク: `#[actuation_choke_point(callers = "...")]`属性マクロ。
//!
//! [ADR-161](../../docs/adr/161-single-source-spec-generation.md)実証実験5
//! （`spike/syn-xtask-prototype`ブランチの`crates/macro-spike`のスパイク。このブランチには
//! 存在しない）を本実装化したもの。「まず記録・可視化してから
//! dylintの許可リストへ昇格する」という[ADR-158](../../docs/adr/158-complexity-reduction-north-star.md)
//! 「育て方」の考え方をコードとして具体化する——強制（コンパイルエラー）ではなく、実行時に
//! 呼び出し元の`file:line`と期待される許可呼び出し元リストを`tracing::debug!`で記録する。
//!
//! # 使い方
//!
//! ```ignore
//! #[actuation_choke_point(callers = "some_caller_a, some_caller_b")]
//! pub fn can_use_imm32_cross_process(&self) -> bool { ... }
//! ```
//!
//! 実機セッションのログに`[actuation-record]`タグで出力が現れる。数セッション分のログを
//! 集めて「実際の呼び出し元が`callers`欄と一致するか」を確認した上で、確信が持てた範囲から
//! `lints/actuation_call_guard`の`RESTRICTED_CALLS`へ昇格する（TA2で確立した運用パターン）。
//!
//! # 既知の制約（実証実験5から引き継ぎ）
//!
//! `#[track_caller]`が取得できるのは`file:line`のみで、呼び出し元の**関数名**は直接得られ
//! ない（ADR-161 round4 TJ1 S5が指摘した「file:line→関数名の変換工程」が別途必要）。昇格
//! 判断を行う際は、記録されたfile:lineを`crates/xtask-adr-evidence`のようなsynベースの
//! ツールか目視で関数名へ変換すること。

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

#[proc_macro_attribute]
pub fn actuation_choke_point(attr: TokenStream, item: TokenStream) -> TokenStream {
    let name_value = parse_macro_input!(attr as syn::MetaNameValue);
    let mut func = parse_macro_input!(item as ItemFn);

    let fn_name = func.sig.ident.to_string();
    let callers_str = if let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(s),
        ..
    }) = &name_value.value
    {
        s.value()
    } else {
        return syn::Error::new_spanned(&name_value, "callers = \"...\" の形式で書くこと")
            .to_compile_error()
            .into();
    };
    let block = &func.block;

    let new_block: syn::Block = syn::parse_quote! {
        {
            let __loc = ::std::panic::Location::caller();
            tracing::debug!(
                "[actuation-record] {} called from {}:{} (想定呼び出し元: {})",
                #fn_name, __loc.file(), __loc.line(), #callers_str
            );
            #block
        }
    };
    *func.block = new_block;
    func.attrs.push(syn::parse_quote!(#[track_caller]));

    quote! { #func }.into()
}
