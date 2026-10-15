#![doc = include_str!("../README.md")]

use proc_macro2::{Group, Ident, Span, TokenStream, TokenTree};
use quote::{ToTokens, TokenStreamExt, quote, quote_spanned};
use syn::{
    Error, Expr, Token, braced,
    parse::{
        ParseStream, Parser,
        discouraged::{AnyDelimiter, Speculative},
    },
    parse_quote_spanned,
    visit_mut::{VisitMut, visit_expr_mut},
};

struct YieldVisitor;

impl VisitMut for YieldVisitor {
    fn visit_expr_mut(&mut self, i: &mut Expr) {
        visit_expr_mut(self, i);

        if let Expr::Yield(yield_expr) = i {
            let expr = yield_expr.expr.take().unwrap();
            *i = parse_quote_spanned! { Span::mixed_site() =>
                yield_tx.send(#expr).await
            }
        }
    }
}

fn transform_body(input: ParseStream) -> Result<TokenStream, Error> {
    let mut out = TokenStream::new();

    while !input.is_empty() {
        // parsed expression
        if let fork = input.fork()
            && let Ok(mut expr) = fork.parse::<Expr>()
        {
            // PollVisitor { nav }.visit_expr_mut(&mut expr);
            YieldVisitor.visit_expr_mut(&mut expr);
            expr.to_tokens(&mut out);
            input.advance_to(&fork);
            continue;
        }

        // parsed block
        if let Ok((delim, _, buf)) = input.parse_any_delimiter() {
            out.append(Group::new(delim, transform_body(&buf)?));
            continue;
        }

        // single token
        out.append(input.parse::<TokenTree>()?);
    }

    Ok(out)
}

fn transform(stream_fn: &'static str) -> impl Parser<Output = TokenStream> {
    |input: ParseStream| {
        let r#move: Option<Token![move]> = input.parse()?;

        input.parse::<Option<Token![|]>>()?;
        input.parse::<Option<Token![|]>>()?;

        let body;
        braced!(body in input);

        let body = transform_body(&body)?;

        let stream_fn = Ident::new(stream_fn, Span::mixed_site());
        Ok(quote_spanned! { Span::mixed_site() =>
            async_generator::#stream_fn(async #r#move |yield_tx| {
                #body
            })
        })
    }
}

fn macro_impl(input: proc_macro::TokenStream, stream_fn: &'static str) -> proc_macro::TokenStream {
    let input = TokenStream::from(input);

    match transform(stream_fn).parse2(input.clone()) {
        Ok(tokens) => tokens,
        Err(err) => {
            let err = err.into_compile_error();
            quote! {
                #err;
                #input
            }
        }
    }
    .into()
}

/// Creates a stream from an async body containing `yield value` expressions.
///
/// Use this macro through the `async_generator` crate. The body returns `()`;
/// an optional `move` before the block moves captures into the generator.
#[proc_macro]
pub fn generator(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    macro_impl(input, "generator")
}

/// Creates a stream of results from a fallible async body.
///
/// Use this macro through the `async_generator` crate. The body returns
/// `Result<(), E>` and supports `?`. Yielded values become `Ok(value)` and a
/// terminal error becomes one final `Err(error)` item.
#[proc_macro]
pub fn try_generator(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    macro_impl(input, "try_generator")
}
