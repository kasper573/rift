use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{FnArg, ItemFn, LitStr, Pat, Token};

/// Declares a free fn as a terminal command and registers it at the definition site.
///
/// `#[command(name = "tp", access = role::admin)]` — both attribute args are optional: `name`
/// defaults to the fn name, `access` is a `fn(&World, Entity) -> bool` checked against the
/// sender's connection. The fn takes `(world: &mut World, ctx: &CommandCtx, ...)`; every
/// further parameter is a positional argument parsed via `CommandArg`, and the doc comment
/// becomes the command's description.
#[proc_macro_attribute]
pub fn command(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attrs = syn::parse_macro_input!(attr as Attrs);
    let func = syn::parse_macro_input!(item as ItemFn);
    expand(attrs, func)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

struct Attrs {
    name: Option<LitStr>,
    access: Option<syn::Expr>,
}

impl Parse for Attrs {
    fn parse(input: ParseStream) -> syn::Result<Attrs> {
        let mut attrs = Attrs {
            name: None,
            access: None,
        };
        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "name" => attrs.name = Some(input.parse()?),
                "access" => attrs.access = Some(input.parse()?),
                _ => return Err(syn::Error::new(key.span(), "expected `name` or `access`")),
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(attrs)
    }
}

fn expand(attrs: Attrs, func: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    let fn_ident = &func.sig.ident;
    let name = attrs
        .name
        .map(|name| name.value())
        .unwrap_or_else(|| fn_ident.to_string());
    let access = match attrs.access {
        Some(access) => quote!(Some(#access)),
        None => quote!(None),
    };
    let description = doc_string(&func);
    let params = command_params(&func)?;
    let names: Vec<String> = params.iter().map(|(ident, _)| ident.to_string()).collect();
    let idents: Vec<&syn::Ident> = params.iter().map(|(ident, _)| ident).collect();
    let types: Vec<&syn::Type> = params.iter().map(|(_, ty)| ty).collect();

    Ok(quote! {
        #func

        const _: () = {
            fn run(
                world: &mut ::bevy_terminal::macro_support::World,
                ctx: &::bevy_terminal::CommandCtx,
            ) -> Result<String, String> {
                let mut raw = ctx.split_args();
                #(
                    let #idents: #types = ::bevy_terminal::CommandArg::parse(#names, raw.next())?;
                )*
                #fn_ident(world, ctx #(, #idents)*)
            }
            ::bevy_terminal::macro_support::inventory::submit! {
                ::bevy_terminal::TerminalCommand {
                    name: #name,
                    description: #description,
                    access: #access,
                    args: &[#(
                        ::bevy_terminal::CommandArgSpec {
                            name: #names,
                            required: <#types as ::bevy_terminal::CommandArg>::REQUIRED,
                        }
                    ),*],
                    run,
                }
            }
        };
    })
}

fn command_params(func: &ItemFn) -> syn::Result<Vec<(syn::Ident, syn::Type)>> {
    if func.sig.inputs.len() < 2 {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "a command fn must take (world: &mut World, ctx: &CommandCtx, ...)",
        ));
    }
    func.sig
        .inputs
        .iter()
        .skip(2)
        .map(|arg| match arg {
            FnArg::Typed(param) => match &*param.pat {
                Pat::Ident(ident) => Ok((ident.ident.clone(), (*param.ty).clone())),
                _ => Err(syn::Error::new_spanned(
                    param,
                    "command arguments must be plain identifiers",
                )),
            },
            FnArg::Receiver(_) => Err(syn::Error::new_spanned(arg, "commands must be free fns")),
        })
        .collect()
}

fn doc_string(func: &ItemFn) -> String {
    let lines: Vec<String> = func
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(pair) => match &pair.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(doc),
                    ..
                }) => Some(doc.value().trim().to_owned()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    lines.join(" ")
}
