//! ADR-161「機構の選定指針」実証実験3〜6（恒久化しない）。
//!
//! - 実験3: deriveマクロによる「宣言→生成」(Markdown表の自動生成)の実現可能性を検証する。
//! - 実験4: 関数形式の手続きマクロによる「宣言のDSL化」の実現可能性を検証する。
//! - 実験5: 属性マクロ(実行時記録版)——関数に付けると呼び出し元のfile:lineを実行時に記録する。
//!   dylintで強制する前の「まず可視化する」段階を想定。
//! - 実験6: 属性マクロ(メタデータ強制版)——`#[measured(...)]`のように、必須フィールドが
//!   欠けているとコンパイルエラーにする。tuning.rsの実測値記録義務(tuning-constants.md)を
//!   人力レビューではなくマクロ展開時に強制できるかを検証する。

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Ident, ItemFn, LitInt, LitStr, Token};

/// 実験3: `#[derive(MarkdownRow)]`。
///
/// 名前付きフィールドを持つ構造体に付けると、各フィールドを `Debug` 表示で
/// 並べた Markdown テーブル行を返す `to_markdown_row(&self) -> String` を生成する。
/// deriveマクロは型定義（構造体のフィールド一覧）は見えるが、「その型のインスタンスが
/// 実際に何個あるか」（＝`ACTUATION_CHOKE_POINTS`のようなconst配列の中身）は見えない
/// ——生成できるのは「1件をどう描画するか」というテンプレートまでで、全件を集めて
/// 表全体を組み立てる処理は呼び出し側が別途書く必要がある、という制約を確認する。
#[proc_macro_derive(MarkdownRow)]
pub fn derive_markdown_row(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(&input, "MarkdownRow は構造体にのみ付けられる")
            .to_compile_error()
            .into();
    };
    let Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(&input, "MarkdownRow は名前付きフィールドが要る")
            .to_compile_error()
            .into();
    };

    let field_idents: Vec<&Ident> = fields
        .named
        .iter()
        .map(|f| f.ident.as_ref().expect("named field"))
        .collect();

    // `| field1 | field2 | ... |` という1行を組み立てる。
    let cell_exprs = field_idents.iter().map(|ident| {
        quote! { format!("{:?}", self.#ident) }
    });

    let expanded = quote! {
        impl #name {
            /// deriveマクロ(実験3)が生成したMarkdownテーブル行。
            pub fn to_markdown_row(&self) -> String {
                let cells: Vec<String> = vec![ #( #cell_exprs ),* ];
                format!("| {} |", cells.join(" | "))
            }
        }
    };
    expanded.into()
}

/// 実験4: `choke_points! { ... }`。
///
/// DSL:
/// ```ignore
/// choke_points! {
///     "callee_name" => ["caller1", "caller2"], adr: "ADR-119, ADR-121";
///     ...
/// }
/// ```
/// を `pub static CHOKE_POINTS: &[ChokePoint] = &[ ChokePoint { .. }, .. ];` に展開する。
/// 宣言のDSL化が実際に`syn`のカスタム`Parse`実装だけで書けるか、そしてこれが
/// `ChokePoint`型自体の定義（`callee`/`callers`/`adr`フィールド）と整合するかを確認する。
struct ChokePointEntry {
    callee: LitStr,
    callers: Vec<LitStr>,
    adr: LitStr,
}

impl Parse for ChokePointEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let callee: LitStr = input.parse()?;
        input.parse::<Token![=>]>()?;
        let content;
        syn::bracketed!(content in input);
        let callers: Punctuated<LitStr, Token![,]> =
            content.parse_terminated(|s: ParseStream| s.parse::<LitStr>(), Token![,])?;
        input.parse::<Token![,]>()?;
        let adr_kw: Ident = input.parse()?;
        if adr_kw != "adr" {
            return Err(syn::Error::new(adr_kw.span(), "`adr:` を期待した"));
        }
        input.parse::<Token![:]>()?;
        let adr: LitStr = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(ChokePointEntry {
            callee,
            callers: callers.into_iter().collect(),
            adr,
        })
    }
}

struct ChokePointsInput {
    entries: Vec<ChokePointEntry>,
}

impl Parse for ChokePointsInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut entries = Vec::new();
        while !input.is_empty() {
            entries.push(input.parse()?);
        }
        Ok(ChokePointsInput { entries })
    }
}

#[proc_macro]
pub fn choke_points(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as ChokePointsInput);

    let entries: Vec<TokenStream2> = parsed
        .entries
        .iter()
        .map(|e| {
            let callee = &e.callee;
            let callers = &e.callers;
            let adr = &e.adr;
            quote! {
                ChokePoint {
                    callee: #callee,
                    callers: &[ #( #callers ),* ],
                    adr: #adr,
                }
            }
        })
        .collect();

    let count = parsed.entries.len();
    let const_name = format_ident!("CHOKE_POINTS");

    let expanded = quote! {
        pub static #const_name: [ChokePoint; #count] = [ #( #entries ),* ];
    };
    expanded.into()
}

/// 実験5: `#[actuation_choke_point(callers = "set_ime_open_ordered, ...")]`。
///
/// 関数に付けると、`#[track_caller]`を自動付与し、呼び出されるたびに
/// 呼び出し元の`file:line`と、期待される許可呼び出し元リストを`println!`で記録する
/// （実行時、dylintのようなコンパイルエラーにはならない）。「まず記録・可視化してから
/// dylintの許可リストへ昇格する」というADR-158「育て方」の考え方を、実際に動くコードで
/// 検証する。
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
            println!(
                "[actuation-record] {} called from {}:{} (許可呼び出し元: {})",
                #fn_name, __loc.file(), __loc.line(), #callers_str
            );
            #block
        }
    };
    func.block = Box::new(new_block);
    func.attrs.push(syn::parse_quote!(#[track_caller]));

    quote! { #func }.into()
}

/// 実験6: `#[measured(value_ms = 181, margin_ms = 169, commit = "9a7e699")]`。
///
/// `const`宣言に付ける。`value_ms`と`commit`が両方揃っていない場合はコンパイルエラーに
/// する——`.claude/rules/tuning-constants.md`が求める「実測msをコミット本文に書け」という
/// 人力レビュー頼みの規約を、マクロ展開時の強制に置き換えられるかを検証する。属性自体は
/// 実行時には何もせず（値はコンパイル時にのみ検証され、生成される`const`宣言自体は変更しない）、
/// 将来的にはこのメタデータをsynベースのxtaskが読み取って
/// `.claude/rules/tuning-constants.md`の表を生成する、という使い方を想定する。
struct MeasuredArgs {
    value_ms: Option<LitInt>,
    margin_ms: Option<LitInt>,
    commit: Option<LitStr>,
}

impl Parse for MeasuredArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = MeasuredArgs {
            value_ms: None,
            margin_ms: None,
            commit: None,
        };
        let pairs: Punctuated<syn::MetaNameValue, Token![,]> =
            Punctuated::parse_terminated(input)?;
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

    if args.value_ms.is_none() {
        return syn::Error::new(
            Span::call_site(),
            "#[measured(...)] には value_ms が必須です \
             (.claude/rules/tuning-constants.md: 実測msを書かずに定数を変更してはならない)",
        )
        .to_compile_error()
        .into();
    }
    if args.commit.is_none() {
        return syn::Error::new(
            Span::call_site(),
            "#[measured(...)] には commit が必須です \
             (実測がどのコミットで行われたかを追跡できるようにする)",
        )
        .to_compile_error()
        .into();
    }

    quote! { #item2 }.into()
}
