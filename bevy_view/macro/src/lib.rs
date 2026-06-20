use proc_macro::TokenStream;
use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitStr, Pat, Token, braced, parse_macro_input};

#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    let markup = parse_macro_input!(input as Markup);
    lower_roots(&markup.roots).into()
}

struct Markup {
    roots: Vec<Node>,
}

impl Parse for Markup {
    fn parse(input: ParseStream) -> syn::Result<Markup> {
        let mut roots = Vec::new();
        while !input.is_empty() {
            roots.push(input.parse()?);
        }
        Ok(Markup { roots })
    }
}

enum Node {
    Element(ElementNode),
    Text(LitStr),
    Block(Expr),
}

struct ElementNode {
    tag: Ident,
    attrs: Vec<Attr>,
    children: Vec<Node>,
}

enum Attr {
    Event(EventKind, Expr),
    Use(Expr),
    Insert(Expr),
    Cursor(Expr),
    Src(Expr),
    Field(Ident, Expr),
    When(Expr),
    Each(Expr),
    Key(Expr),
    Let(Pat),
}

#[derive(Clone, Copy)]
enum EventKind {
    Click,
    Mount,
    Cleanup,
    Over,
    Out,
    Drag,
    DragEnd,
}

impl Parse for Node {
    fn parse(input: ParseStream) -> syn::Result<Node> {
        if input.peek(Token![<]) {
            Ok(Node::Element(input.parse()?))
        } else if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            Ok(Node::Block(content.parse()?))
        } else if input.peek(LitStr) {
            Ok(Node::Text(input.parse()?))
        } else {
            Err(input.error("expected an element `<…>`, a string literal, or `{ expr }`"))
        }
    }
}

impl Parse for ElementNode {
    fn parse(input: ParseStream) -> syn::Result<ElementNode> {
        input.parse::<Token![<]>()?;
        let tag: Ident = input.parse()?;
        let mut attrs = Vec::new();
        while !input.peek(Token![>]) && !input.peek(Token![/]) {
            attrs.push(input.parse()?);
        }
        if input.peek(Token![/]) {
            input.parse::<Token![/]>()?;
            input.parse::<Token![>]>()?;
            return Ok(ElementNode {
                tag,
                attrs,
                children: Vec::new(),
            });
        }
        input.parse::<Token![>]>()?;
        let mut children = Vec::new();
        loop {
            if input.peek(Token![<]) && input.peek2(Token![/]) {
                break;
            }
            if input.is_empty() {
                return Err(input.error("unclosed element"));
            }
            children.push(input.parse()?);
        }
        input.parse::<Token![<]>()?;
        input.parse::<Token![/]>()?;
        let close: Ident = input.parse()?;
        input.parse::<Token![>]>()?;
        if close != tag {
            return Err(syn::Error::new(
                close.span(),
                format!("closing tag </{close}> does not match <{tag}>"),
            ));
        }
        Ok(ElementNode {
            tag,
            attrs,
            children,
        })
    }
}

impl Parse for Attr {
    fn parse(input: ParseStream) -> syn::Result<Attr> {
        if input.peek(Token![let]) {
            input.parse::<Token![let]>()?;
            input.parse::<Token![=]>()?;
            let content;
            braced!(content in input);
            return Ok(Attr::Let(content.call(Pat::parse_single)?));
        }
        if input.peek(Token![use]) {
            input.parse::<Token![use]>()?;
            input.parse::<Token![=]>()?;
            return Ok(Attr::Use(attr_value(input)?));
        }
        let name: Ident = input.parse()?;
        if name == "on" {
            input.parse::<Token![:]>()?;
            let event: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let kind = match event.to_string().as_str() {
                "click" => EventKind::Click,
                "mount" => EventKind::Mount,
                "cleanup" => EventKind::Cleanup,
                "over" => EventKind::Over,
                "out" => EventKind::Out,
                "drag" => EventKind::Drag,
                "dragend" => EventKind::DragEnd,
                _ => {
                    return Err(syn::Error::new(
                        event.span(),
                        "unknown event; expected click, mount, cleanup, over, out, drag, or dragend",
                    ));
                }
            };
            return Ok(Attr::Event(kind, attr_value(input)?));
        }
        input.parse::<Token![=]>()?;
        match name.to_string().as_str() {
            "insert" => Ok(Attr::Insert(attr_value(input)?)),
            "cursor" => Ok(Attr::Cursor(attr_value(input)?)),
            "src" => Ok(Attr::Src(attr_value(input)?)),
            "when" => Ok(Attr::When(attr_value(input)?)),
            "each" => Ok(Attr::Each(attr_value(input)?)),
            "key" => Ok(Attr::Key(attr_value(input)?)),
            _ => Ok(Attr::Field(name, attr_value(input)?)),
        }
    }
}

/// Bare values must be braced if they contain `<`/`>`/`/` (turbofish, generics, comparisons) to avoid
/// being mistaken for tag boundaries or attribute separators.
fn attr_value(input: ParseStream) -> syn::Result<Expr> {
    if input.peek(syn::token::Brace) {
        let content;
        braced!(content in input);
        content.parse()
    } else {
        let mut tokens = TokenStream2::new();
        while !input.is_empty()
            && !input.peek(Token![>])
            && !input.peek(Token![/])
            && !input.peek(Token![<])
            && !next_attribute_starts(input)
        {
            let token: TokenTree = input.parse()?;
            tokens.extend(std::iter::once(token));
        }
        syn::parse2(tokens)
    }
}

/// `on:` event boundary checks peek for the literal `on`, so a `::` path separator (e.g. `Val::Px`)
/// inside a bare value is not mistaken for it.
fn next_attribute_starts(input: ParseStream) -> bool {
    if (input.peek(Ident) || input.peek(Token![use]) || input.peek(Token![let]))
        && input.peek2(Token![=])
    {
        return true;
    }
    if input.peek(Ident) && input.peek2(Token![:]) {
        let fork = input.fork();
        if let Ok(ident) = fork.parse::<Ident>() {
            return ident == "on";
        }
    }
    false
}

fn lower_roots(roots: &[Node]) -> TokenStream2 {
    match roots {
        [] => quote!(::bevy_view::View::empty()),
        [one] => {
            let lowered = lower(one);
            quote!(::bevy_view::View::from(#lowered))
        }
        many => {
            let members = many.iter().map(|node| {
                let lowered = lower(node);
                quote!(::bevy_view::View::from(#lowered))
            });
            quote!(::bevy_view::View::fragment([#(#members),*]))
        }
    }
}

fn lower(node: &Node) -> TokenStream2 {
    match node {
        Node::Block(expr) => quote!(#expr),
        Node::Text(lit) => quote!(::bevy_view::text(#lit)),
        Node::Element(element) => lower_element(element),
    }
}

fn lower_element(element: &ElementNode) -> TokenStream2 {
    let tag = element.tag.to_string();
    match tag.as_str() {
        "node" => lower_node_like(element, quote!(::bevy_view::node())),
        "button" => lower_node_like(element, quote!(::bevy_view::button())),
        "image" => lower_image(element),
        "text" => lower_text(element),
        "Show" => lower_show(element),
        "Hide" => lower_hide(element),
        "For" => lower_for(element),
        _ if tag.starts_with(|c: char| c.is_uppercase()) => lower_component(element),
        other => syn::Error::new(
            element.tag.span(),
            format!(
                "unknown element <{other}>; use a lowercase intrinsic, a component, or `{{ expr }}`"
            ),
        )
        .to_compile_error(),
    }
}

fn lower_node_like(element: &ElementNode, ctor: TokenStream2) -> TokenStream2 {
    let attrs = intrinsic_attrs(element);
    let children = element.children.iter().map(|child| {
        let lowered = lower(child);
        quote!(.child(#lowered))
    });
    quote!(#ctor #attrs #(#children)*)
}

fn lower_image(element: &ElementNode) -> TokenStream2 {
    let Some(src) = element.attrs.iter().find_map(|attr| match attr {
        Attr::Src(expr) => Some(expr),
        _ => None,
    }) else {
        return syn::Error::new(element.tag.span(), "<image> requires `src={…}`")
            .to_compile_error();
    };
    lower_node_like(element, quote!(::bevy_view::image(#src)))
}

fn lower_text(element: &ElementNode) -> TokenStream2 {
    let ctor = match element.children.as_slice() {
        [] => quote!(::bevy_view::text("")),
        [Node::Text(lit)] => quote!(::bevy_view::text(#lit)),
        [Node::Block(expr)] if matches!(expr, Expr::Closure(_)) => {
            quote!(::bevy_view::dyn_text(#expr))
        }
        [Node::Block(expr)] => quote!(::bevy_view::text(#expr)),
        _ => {
            return syn::Error::new(
                element.tag.span(),
                "a <text> element holds a single string literal or `{ |w| … }` content closure",
            )
            .to_compile_error();
        }
    };
    let attrs = intrinsic_attrs(element);
    quote!(#ctor #attrs)
}

/// Bareword fields collapse into one partial `Node` setter so that retained fields (like a drag
/// system's position) survive the render.
fn intrinsic_attrs(element: &ElementNode) -> TokenStream2 {
    let mut fields = Vec::new();
    let mut chain = TokenStream2::new();
    for attr in &element.attrs {
        match attr {
            Attr::Field(name, value) => fields.push(quote!(node.#name = #value;)),
            Attr::Event(kind, handler) => {
                let method = event_method(*kind);
                chain.extend(quote!(.#method(#handler)));
            }
            Attr::Use(bind) => chain.extend(quote!(.bind(#bind))),
            Attr::Insert(bundle) => chain.extend(quote!(.insert(#bundle))),
            Attr::Cursor(icon) => chain.extend(quote!(.cursor(#icon))),
            Attr::Src(_) => {}
            Attr::When(_) | Attr::Each(_) | Attr::Key(_) | Attr::Let(_) => {
                chain.extend(
                    syn::Error::new(
                        element.tag.span(),
                        "when/each/key/let are only valid on <Show>/<Hide>/<For>",
                    )
                    .to_compile_error(),
                );
            }
        }
    }
    let field_setter = if fields.is_empty() {
        quote!()
    } else {
        quote!(.attr(move |entity| {
            if let Some(mut node) = entity.get_mut::<::bevy_view::Node>() {
                #(#fields)*
            }
        }))
    };
    quote!(#field_setter #chain)
}

fn event_method(kind: EventKind) -> TokenStream2 {
    match kind {
        EventKind::Click => quote!(on_click),
        EventKind::Mount => quote!(on_mount),
        EventKind::Cleanup => quote!(on_cleanup),
        EventKind::Over => quote!(on_over),
        EventKind::Out => quote!(on_out),
        EventKind::Drag => quote!(on_drag),
        EventKind::DragEnd => quote!(on_drag_end),
    }
}

/// Component styling/behavior attributes are collected into one decorator and applied to the root
/// via `.modify(Bind)` — so a headless component can be wired at its use site like an intrinsic.
fn lower_component(element: &ElementNode) -> TokenStream2 {
    let tag = &element.tag;
    let mut expr = quote!(#tag::default());
    let mut decorations = TokenStream2::new();
    for attr in &element.attrs {
        match attr {
            Attr::Field(name, value) => expr = quote!(#expr.#name(#value)),
            Attr::Insert(bundle) => decorations.extend(quote!(.insert(#bundle))),
            Attr::Event(kind, handler) => {
                let method = event_method(*kind);
                decorations.extend(quote!(.#method(#handler)));
            }
            Attr::Use(bind) => decorations.extend(quote!(.bind(#bind))),
            Attr::Cursor(icon) => decorations.extend(quote!(.cursor(#icon))),
            Attr::Src(_) | Attr::When(_) | Attr::Each(_) | Attr::Key(_) | Attr::Let(_) => {
                return syn::Error::new(
                    tag.span(),
                    "a component takes `prop=value`, `insert`/`on:…`/`use`/`cursor`, and children",
                )
                .to_compile_error();
            }
        }
    }
    if !decorations.is_empty() {
        expr = quote!(#expr.modify(::bevy_view::Bind::new(move |element| element #decorations)));
    }
    for child in &element.children {
        let lowered = lower(child);
        expr = quote!(#expr.child(#lowered));
    }
    expr
}

fn lower_show(element: &ElementNode) -> TokenStream2 {
    let Some(when) = find_when(element) else {
        return syn::Error::new(element.tag.span(), "<Show> requires `when={…}`")
            .to_compile_error();
    };
    let body = lower_body(&element.children);
    quote!(::bevy_view::show(#when, #body))
}

fn lower_hide(element: &ElementNode) -> TokenStream2 {
    let Some(when) = find_when(element) else {
        return syn::Error::new(element.tag.span(), "<Hide> requires `when={…}`")
            .to_compile_error();
    };
    let body = lower_body(&element.children);
    quote!(::bevy_view::hide(#when, #body))
}

fn find_when(element: &ElementNode) -> Option<&Expr> {
    element.attrs.iter().find_map(|attr| match attr {
        Attr::When(expr) => Some(expr),
        _ => None,
    })
}

fn lower_for(element: &ElementNode) -> TokenStream2 {
    let mut each = None;
    let mut key = None;
    let mut binding = None;
    for attr in &element.attrs {
        match attr {
            Attr::Each(expr) => each = Some(expr),
            Attr::Key(expr) => key = Some(expr),
            Attr::Let(pat) => binding = Some(pat),
            _ => {}
        }
    }
    let (Some(each), Some(key), Some(binding)) = (each, key, binding) else {
        return syn::Error::new(
            element.tag.span(),
            "<For> requires `each={…}`, `key={…}`, and `let={binding}`",
        )
        .to_compile_error();
    };
    let body = lower_body(&element.children);
    quote!(::bevy_view::each(#each, #key, move |#binding| #body))
}

fn lower_body(children: &[Node]) -> TokenStream2 {
    match children {
        [] => quote!(::bevy_view::View::empty()),
        [one] => {
            let lowered = lower(one);
            quote!(::bevy_view::View::from(#lowered))
        }
        many => {
            let members = many.iter().map(|node| {
                let lowered = lower(node);
                quote!(::bevy_view::View::from(#lowered))
            });
            quote!(::bevy_view::View::fragment([#(#members),*]))
        }
    }
}
