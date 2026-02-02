//! Procedural macros for defining Rinch editor extensions.
//!
//! Provides the `extension!` macro for declaring nodes, marks,
//! commands, keyboard shortcuts, and input rules.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, format_ident};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token, Result as SynResult, braced, bracketed, Expr};

struct ExtensionDef {
    name: LitStr,
    priority: Option<syn::LitInt>,
    marks: Vec<Expr>,
    nodes: Vec<Expr>,
    shortcuts: Vec<(LitStr, Ident)>,
    commands: Vec<(Ident, Expr)>,
}

impl Parse for ExtensionDef {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let mut name = None;
        let mut priority = None;
        let mut marks = Vec::new();
        let mut nodes = Vec::new();
        let mut shortcuts = Vec::new();
        let mut commands = Vec::new();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![:]>()?;

            match key.to_string().as_str() {
                "name" => {
                    name = Some(input.parse::<LitStr>()?);
                }
                "priority" => {
                    priority = Some(input.parse::<syn::LitInt>()?);
                }
                "marks" => {
                    let content;
                    bracketed!(content in input);
                    while !content.is_empty() {
                        marks.push(content.parse::<Expr>()?);
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                }
                "nodes" => {
                    let content;
                    bracketed!(content in input);
                    while !content.is_empty() {
                        nodes.push(content.parse::<Expr>()?);
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                }
                "shortcuts" => {
                    let content;
                    bracketed!(content in input);
                    while !content.is_empty() {
                        let inner;
                        syn::parenthesized!(inner in content);
                        let key_str: LitStr = inner.parse()?;
                        inner.parse::<Token![,]>()?;
                        let cmd_name: LitStr = inner.parse()?;
                        shortcuts.push((key_str, format_ident!("{}", cmd_name.value())));
                        if !content.is_empty() {
                            let _ = content.parse::<Token![,]>();
                        }
                    }
                }
                "commands" => {
                    let content;
                    braced!(content in input);
                    while !content.is_empty() {
                        let cmd_name: Ident = content.parse()?;
                        content.parse::<Token![=>]>()?;
                        let handler: Expr = content.parse()?;
                        commands.push((cmd_name, handler));
                        if !content.is_empty() {
                            let _ = content.parse::<Token![,]>();
                        }
                    }
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("Unknown extension field: {}", key),
                    ));
                }
            }

            // Optional trailing comma
            if !input.is_empty() {
                let _ = input.parse::<Token![,]>();
            }
        }

        let name = name.ok_or_else(|| input.error("Missing required field: name"))?;

        Ok(ExtensionDef {
            name,
            priority,
            marks,
            nodes,
            shortcuts,
            commands,
        })
    }
}

/// Define an editor extension with nodes, marks, commands, and shortcuts.
///
/// # Example
///
/// ```ignore
/// extension! {
///     name: "bold",
///     marks: [MarkSpec::simple("bold")],
///     shortcuts: [("Mod-b", "toggle_bold")],
///     commands: {
///         toggle_bold => |editor| FormattingCommands::toggle_mark(editor, "bold")
///     }
/// }
/// ```
///
/// This generates a struct named after the extension (e.g., `BoldExtension`)
/// that implements the `Extension` trait.
#[proc_macro]
pub fn extension(input: TokenStream) -> TokenStream {
    let def = syn::parse_macro_input!(input as ExtensionDef);

    let ext_name_str = def.name.value();
    // Convert "bold" -> "BoldExtension"
    let struct_name = format_ident!(
        "{}Extension",
        ext_name_str
            .chars()
            .enumerate()
            .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
            .collect::<String>()
    );

    let name_lit = &def.name;

    let priority_impl = if let Some(ref p) = def.priority {
        quote! {
            fn priority(&self) -> i32 { #p }
        }
    } else {
        quote! {}
    };

    let marks_impl = if def.marks.is_empty() {
        quote! {}
    } else {
        let mark_exprs = &def.marks;
        quote! {
            fn marks(&self) -> Vec<rinch_editor::schema::MarkSpec> {
                vec![#(#mark_exprs),*]
            }
        }
    };

    let nodes_impl = if def.nodes.is_empty() {
        quote! {}
    } else {
        let node_exprs = &def.nodes;
        quote! {
            fn nodes(&self) -> Vec<rinch_editor::schema::NodeSpec> {
                vec![#(#node_exprs),*]
            }
        }
    };

    let shortcuts_impl = if def.shortcuts.is_empty() {
        quote! {}
    } else {
        let shortcut_entries: Vec<TokenStream2> = def.shortcuts.iter().map(|(key, cmd)| {
            let cmd_str = cmd.to_string();
            quote! {
                (rinch_editor::input::KeyboardShortcut::new(#key, #cmd_str), #cmd_str.to_string())
            }
        }).collect();
        quote! {
            fn keyboard_shortcuts(&self) -> Vec<(rinch_editor::input::KeyboardShortcut, String)> {
                vec![#(#shortcut_entries),*]
            }
        }
    };

    let commands_impl = if def.commands.is_empty() {
        quote! {}
    } else {
        let cmd_entries: Vec<TokenStream2> = def.commands.iter().map(|(name, handler)| {
            let name_str = name.to_string();
            quote! {
                rinch_editor::extensions::CommandRegistration::new(#name_str, #handler)
            }
        }).collect();
        quote! {
            fn commands(&self) -> Vec<rinch_editor::extensions::CommandRegistration> {
                vec![#(#cmd_entries),*]
            }
        }
    };

    let expanded = quote! {
        #[derive(Debug)]
        pub struct #struct_name;

        impl rinch_editor::extensions::Extension for #struct_name {
            fn name(&self) -> &str { #name_lit }
            #priority_impl
            #marks_impl
            #nodes_impl
            #shortcuts_impl
            #commands_impl
        }
    };

    expanded.into()
}
