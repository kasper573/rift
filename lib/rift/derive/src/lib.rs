//! Generates the encode/decode impl behind `#[derive(Wire)]`; the trait and the wire format
//! live in `rift`. Only field and variant names are extracted — the generated code lets each
//! field's declared type pick its `Wire` implementation.

use proc_macro::{Delimiter, TokenStream, TokenTree};
use std::iter::Peekable;

#[proc_macro_derive(Wire)]
pub fn wire(input: TokenStream) -> TokenStream {
    let mut tokens = input.into_iter().peekable();
    skip_attributes(&mut tokens);
    skip_visibility(&mut tokens);
    let kind = ident(&mut tokens);
    let name = ident(&mut tokens);
    let code = match (kind.as_str(), tokens.next()) {
        ("struct", Some(TokenTree::Group(group))) if group.delimiter() == Delimiter::Brace => {
            for_named_struct(&name, &fields(group.stream()))
        }
        ("struct", Some(TokenTree::Group(group)))
            if group.delimiter() == Delimiter::Parenthesis =>
        {
            for_tuple_struct(&name, tuple_arity(group.stream()))
        }
        ("enum", Some(TokenTree::Group(group))) if group.delimiter() == Delimiter::Brace => {
            for_enum(&name, &unit_variants(&name, group.stream()))
        }
        _ => panic!("Wire supports plain structs and fieldless enums; {name} is neither"),
    };
    code.parse().expect("Wire generated invalid code")
}

fn for_named_struct(name: &str, fields: &[String]) -> String {
    let encodes: String = fields
        .iter()
        .map(|field| format!("::rift::Wire::encode(&self.{field}, out);"))
        .collect();
    let decodes: String = fields
        .iter()
        .map(|field| format!("{field}: ::rift::Wire::decode(bytes)?,"))
        .collect();
    format!(
        "impl ::rift::Wire for {name} {{
            fn encode(&self, out: &mut ::std::vec::Vec<u8>) {{ {encodes} }}
            fn decode(bytes: &mut &[u8]) -> ::core::option::Option<Self> {{
                ::core::option::Option::Some(Self {{ {decodes} }})
            }}
        }}"
    )
}

fn for_tuple_struct(name: &str, arity: usize) -> String {
    let encodes: String = (0..arity)
        .map(|index| format!("::rift::Wire::encode(&self.{index}, out);"))
        .collect();
    let decodes: String = (0..arity)
        .map(|_| "::rift::Wire::decode(bytes)?,".to_owned())
        .collect();
    format!(
        "impl ::rift::Wire for {name} {{
            fn encode(&self, out: &mut ::std::vec::Vec<u8>) {{ {encodes} }}
            fn decode(bytes: &mut &[u8]) -> ::core::option::Option<Self> {{
                ::core::option::Option::Some(Self({decodes}))
            }}
        }}"
    )
}

fn for_enum(name: &str, variants: &[String]) -> String {
    let arms: String = variants
        .iter()
        .map(|variant| {
            format!(
                "if want == {name}::{variant} as u32 {{
                    return ::core::option::Option::Some({name}::{variant});
                }}"
            )
        })
        .collect();
    format!(
        "impl ::rift::Wire for {name} {{
            fn encode(&self, out: &mut ::std::vec::Vec<u8>) {{
                ::rift::Wire::encode(&(*self as u32), out);
            }}
            fn decode(bytes: &mut &[u8]) -> ::core::option::Option<Self> {{
                let want = <u32 as ::rift::Wire>::decode(bytes)?;
                {arms}
                ::core::option::Option::None
            }}
        }}"
    )
}

fn fields(stream: TokenStream) -> Vec<String> {
    let mut names = Vec::new();
    let mut tokens = stream.into_iter().peekable();
    loop {
        skip_attributes(&mut tokens);
        skip_visibility(&mut tokens);
        match tokens.next() {
            Some(TokenTree::Ident(name)) => names.push(name.to_string()),
            None => break,
            Some(other) => panic!("Wire: unexpected {other} in a field list"),
        }
        // The field's type runs to the next comma outside angle brackets.
        let mut depth = 0;
        for token in tokens.by_ref() {
            match token {
                TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
                TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
                TokenTree::Punct(p) if p.as_char() == ',' && depth == 0 => break,
                _ => {}
            }
        }
    }
    names
}

fn tuple_arity(stream: TokenStream) -> usize {
    let mut arity = 0;
    let mut depth = 0;
    let mut in_field = false;
    for token in stream {
        match token {
            TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
            TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
            TokenTree::Punct(p) if p.as_char() == ',' && depth == 0 => in_field = false,
            _ if !in_field => {
                in_field = true;
                arity += 1;
            }
            _ => {}
        }
    }
    arity
}

fn unit_variants(name: &str, stream: TokenStream) -> Vec<String> {
    let mut variants = Vec::new();
    let mut tokens = stream.into_iter().peekable();
    loop {
        skip_attributes(&mut tokens);
        match tokens.next() {
            Some(TokenTree::Ident(variant)) => variants.push(variant.to_string()),
            None => break,
            Some(other) => panic!("Wire: unexpected {other} in {name}'s variants"),
        }
        match tokens.next() {
            None => break,
            Some(TokenTree::Punct(p)) if p.as_char() == ',' => {}
            Some(_) => panic!("Wire supports fieldless enums; {name} has variants with fields"),
        }
    }
    variants
}

fn ident(tokens: &mut Peekable<impl Iterator<Item = TokenTree>>) -> String {
    match tokens.next() {
        Some(TokenTree::Ident(name)) => name.to_string(),
        other => panic!("Wire: expected an identifier, got {other:?}"),
    }
}

fn skip_attributes(tokens: &mut Peekable<impl Iterator<Item = TokenTree>>) {
    while matches!(tokens.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '#') {
        tokens.next();
        tokens.next();
    }
}

fn skip_visibility(tokens: &mut Peekable<impl Iterator<Item = TokenTree>>) {
    if matches!(tokens.peek(), Some(TokenTree::Ident(word)) if word.to_string() == "pub") {
        tokens.next();
        if matches!(
            tokens.peek(),
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis
        ) {
            tokens.next();
        }
    }
}
