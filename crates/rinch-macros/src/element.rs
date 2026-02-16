//! RSX element parsing.

use syn::parse::{Parse, ParseStream};
use syn::{Ident, Result, Token};

use crate::node::RsxNode;
use crate::prop::RsxProp;

/// An element in RSX (component or HTML tag).
pub struct RsxElement {
    pub name: Ident,
    pub props: Vec<RsxProp>,
    pub children: Vec<RsxNode>,
}

impl Parse for RsxElement {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;

        let content;
        syn::braced!(content in input);

        let mut props = Vec::new();
        let mut children = Vec::new();

        while !content.is_empty() {
            // Try to parse as a prop (name: value)
            if content.peek(Ident) && content.peek2(Token![:]) && !content.peek2(Token![::]) {
                let prop: RsxProp = content.parse()?;
                props.push(prop);

                // Consume trailing comma if present
                if content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            } else {
                // Parse as a child node
                let child: RsxNode = content.parse()?;
                children.push(child);

                // Consume trailing comma if present
                if content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            }
        }

        Ok(RsxElement {
            name,
            props,
            children,
        })
    }
}

impl RsxElement {
    /// Core rinch components defined in rinch-core.
    fn is_core_component(&self) -> bool {
        let name = self.name.to_string();
        matches!(
            name.as_str(),
            "Window"
                | "ThemeProvider"
                | "AppMenu"
                | "Menu"
                | "MenuItem"
                | "MenuSeparator"
                | "Fragment"
                | "Portal"
        )
    }

    /// Check if this is a widget (PascalCase name that isn't a core component).
    /// Widgets are third-party components that implement the Widget trait.
    fn is_widget(&self) -> bool {
        let name = self.name.to_string();
        // Must start with uppercase letter (PascalCase)
        let first_char = name.chars().next().unwrap_or('a');
        first_char.is_ascii_uppercase() && !self.is_core_component()
    }

    /// Check if this element is a rinch component (core component or widget).
    pub fn is_rinch_component(&self) -> bool {
        self.is_core_component() || self.is_widget()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::RsxNode;
    use syn::parse_str;

    // ── Empty and basic elements ─────────────────────────────────

    #[test]
    fn parse_empty_element() {
        let el: RsxElement = parse_str("div {}").unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert!(el.props.is_empty());
        assert!(el.children.is_empty());
    }

    #[test]
    fn parse_element_with_text_child() {
        let el: RsxElement = parse_str(r#"div { "hello" }"#).unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert!(el.props.is_empty());
        assert_eq!(el.children.len(), 1);
        assert!(matches!(&el.children[0], RsxNode::Text(lit) if lit.value() == "hello"));
    }

    #[test]
    fn parse_element_with_multiple_text_children() {
        let el: RsxElement = parse_str(r#"div { "Hello, " "world!" }"#).unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert_eq!(el.children.len(), 2);
    }

    // ── Props parsing ────────────────────────────────────────────

    #[test]
    fn parse_element_with_single_prop() {
        let el: RsxElement = parse_str(r#"div { class: "foo" }"#).unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert_eq!(el.props.len(), 1);
        assert_eq!(el.props[0].name.to_string(), "class");
        assert!(el.children.is_empty());
    }

    #[test]
    fn parse_element_with_multiple_props() {
        let el: RsxElement = parse_str(r#"div { class: "foo", id: "bar" }"#).unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert_eq!(el.props.len(), 2);
        assert_eq!(el.props[0].name.to_string(), "class");
        assert_eq!(el.props[1].name.to_string(), "id");
    }

    #[test]
    fn parse_prop_with_trailing_comma() {
        let el: RsxElement = parse_str(r#"div { class: "foo", }"#).unwrap();
        assert_eq!(el.props.len(), 1);
        assert!(el.children.is_empty());
    }

    // ── Mixed props and children ─────────────────────────────────

    #[test]
    fn parse_element_with_mixed_props_and_children() {
        let el: RsxElement = parse_str(r#"div { class: "foo", "hello" "world" }"#).unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert_eq!(el.props.len(), 1);
        assert_eq!(el.children.len(), 2);
    }

    #[test]
    fn parse_props_before_children() {
        let el: RsxElement =
            parse_str(r#"div { class: "container", id: "main", "content" }"#).unwrap();
        assert_eq!(el.props.len(), 2);
        assert_eq!(el.children.len(), 1);
    }

    // ── Nested elements ──────────────────────────────────────────

    #[test]
    fn parse_element_with_nested_element() {
        let el: RsxElement = parse_str(r#"div { p { "hello" } }"#).unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert_eq!(el.children.len(), 1);
        match &el.children[0] {
            RsxNode::Element(child) => {
                assert_eq!(child.name.to_string(), "p");
                assert_eq!(child.children.len(), 1);
            }
            _ => panic!("Expected Element child"),
        }
    }

    #[test]
    fn parse_deeply_nested() {
        let el: RsxElement = parse_str(r#"div { div { div { p { "deep" } } } }"#).unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert_eq!(el.children.len(), 1);
        // Verify we can traverse the nesting
        match &el.children[0] {
            RsxNode::Element(c1) => {
                assert_eq!(c1.name.to_string(), "div");
                match &c1.children[0] {
                    RsxNode::Element(c2) => {
                        assert_eq!(c2.name.to_string(), "div");
                        match &c2.children[0] {
                            RsxNode::Element(c3) => {
                                assert_eq!(c3.name.to_string(), "p");
                            }
                            _ => panic!("Expected Element"),
                        }
                    }
                    _ => panic!("Expected Element"),
                }
            }
            _ => panic!("Expected Element"),
        }
    }

    #[test]
    fn parse_multiple_sibling_elements() {
        let el: RsxElement =
            parse_str(r#"div { p { "one" } span { "two" } p { "three" } }"#).unwrap();
        assert_eq!(el.children.len(), 3);
    }

    // ── Expression children ──────────────────────────────────────

    #[test]
    fn parse_element_with_braced_expr() {
        let el: RsxElement = parse_str("div { {count.get()} }").unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert_eq!(el.children.len(), 1);
        assert!(matches!(&el.children[0], RsxNode::Expr(_)));
    }

    #[test]
    fn parse_element_with_reactive_closure() {
        let el: RsxElement = parse_str("div { {|| count.get().to_string()} }").unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert_eq!(el.children.len(), 1);
        assert!(matches!(&el.children[0], RsxNode::Expr(_)));
    }

    #[test]
    fn parse_multiple_children_types() {
        // Mix of text, elements, and expressions
        let el: RsxElement = parse_str(r#"div { "text" p { "child" } {expr} }"#).unwrap();
        assert_eq!(el.name.to_string(), "div");
        assert_eq!(el.children.len(), 3);
        assert!(matches!(&el.children[0], RsxNode::Text(_)));
        assert!(matches!(&el.children[1], RsxNode::Element(_)));
        assert!(matches!(&el.children[2], RsxNode::Expr(_)));
    }

    // ── Event handlers ───────────────────────────────────────────

    #[test]
    fn parse_event_handler() {
        let el: RsxElement =
            parse_str(r#"button { onclick: move || count.update(|n| *n += 1), "Click" }"#).unwrap();
        assert_eq!(el.name.to_string(), "button");
        assert_eq!(el.props.len(), 1);
        assert_eq!(el.props[0].name.to_string(), "onclick");
        assert_eq!(el.children.len(), 1);
    }

    #[test]
    fn parse_event_handler_no_trailing_comma() {
        // Event handler followed directly by text child (no comma after closure)
        // syn parses `move || do_thing()` as the expression, then "Click" is next
        let el: RsxElement =
            parse_str(r#"button { onclick: move || do_thing(), "Click" }"#).unwrap();
        assert_eq!(el.props.len(), 1);
        assert_eq!(el.children.len(), 1);
    }

    // ── Widget parsing ───────────────────────────────────────────

    #[test]
    fn parse_widget_with_style_and_class() {
        let el: RsxElement = parse_str(
            r#"Button { variant: "filled", style: "margin: 10px", class: "my-btn", "Click" }"#,
        )
        .unwrap();
        assert_eq!(el.name.to_string(), "Button");
        assert_eq!(el.props.len(), 3);
        assert_eq!(el.children.len(), 1);
    }

    #[test]
    fn parse_widget_with_reactive_style() {
        let el: RsxElement =
            parse_str(r#"Button { style: {|| "color: red".to_string()}, "Click" }"#).unwrap();
        assert_eq!(el.name.to_string(), "Button");
        assert_eq!(el.props.len(), 1);
        assert_eq!(el.props[0].name.to_string(), "style");
        assert_eq!(el.children.len(), 1);
    }

    #[test]
    fn parse_widget_with_reactive_class() {
        let el: RsxElement =
            parse_str(r#"Button { class: {|| if active { "on" } else { "off" }}, "Click" }"#)
                .unwrap();
        assert_eq!(el.name.to_string(), "Button");
        assert_eq!(el.props.len(), 1);
        assert_eq!(el.props[0].name.to_string(), "class");
        assert_eq!(el.children.len(), 1);
    }

    // ── is_rinch_component / is_core_component / is_widget ──────

    #[test]
    fn show_and_for_are_widgets_not_core() {
        // Show and For were removed as core components — they parse as widgets now
        let show: RsxElement = parse_str(r#"Show { when: {|| true} }"#).unwrap();
        assert!(show.is_rinch_component()); // Still PascalCase → widget
        let for_el: RsxElement = parse_str(r#"For { each: {|| vec![]} }"#).unwrap();
        assert!(for_el.is_rinch_component()); // Still PascalCase → widget
    }

    #[test]
    fn is_core_component_fragment() {
        let el: RsxElement = parse_str("Fragment {}").unwrap();
        assert!(el.is_rinch_component());
    }

    #[test]
    fn is_core_component_window() {
        let el: RsxElement = parse_str("Window {}").unwrap();
        assert!(el.is_rinch_component());
    }

    #[test]
    fn is_core_component_theme_provider() {
        let el: RsxElement = parse_str("ThemeProvider {}").unwrap();
        assert!(el.is_rinch_component());
    }

    #[test]
    fn is_widget_button() {
        let el: RsxElement = parse_str(r#"Button { variant: "filled" }"#).unwrap();
        assert!(el.is_rinch_component());
    }

    #[test]
    fn is_widget_custom_component() {
        let el: RsxElement = parse_str("MyCustomWidget {}").unwrap();
        assert!(el.is_rinch_component());
    }

    #[test]
    fn is_not_rinch_component_div() {
        let el: RsxElement = parse_str("div {}").unwrap();
        assert!(!el.is_rinch_component());
    }

    #[test]
    fn is_not_rinch_component_span() {
        let el: RsxElement = parse_str("span {}").unwrap();
        assert!(!el.is_rinch_component());
    }

    #[test]
    fn is_not_rinch_component_p() {
        let el: RsxElement = parse_str("p {}").unwrap();
        assert!(!el.is_rinch_component());
    }

    // ── Edge cases ───────────────────────────────────────────────

    #[test]
    fn parse_element_only_props_no_children() {
        let el: RsxElement = parse_str(r#"input { class: "field", id: "name" }"#).unwrap();
        assert_eq!(el.props.len(), 2);
        assert!(el.children.is_empty());
    }

    #[test]
    fn parse_element_only_children_no_props() {
        let el: RsxElement = parse_str(r#"div { "one" "two" "three" }"#).unwrap();
        assert!(el.props.is_empty());
        assert_eq!(el.children.len(), 3);
    }

    #[test]
    fn parse_path_expression_not_confused_with_prop() {
        // `foo::bar` should NOT be parsed as prop `foo:` with value `:`
        // because we check peek2(Token![:]) && !peek2(Token![::])
        let el: RsxElement = parse_str("div { {foo::bar()} }").unwrap();
        assert!(el.props.is_empty());
        assert_eq!(el.children.len(), 1);
    }
}
