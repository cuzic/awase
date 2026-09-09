//! ADR-161「機構の選定指針」実証実験3・4（恒久化しない）。
//!
//! - 実験3: deriveマクロによる「宣言→生成」(Markdown表の自動生成)の実現可能性を検証する。
//! - 実験4: 関数形式の手続きマクロによる「宣言のDSL化」の実現可能性を検証する。

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Ident, LitStr, Token};

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
