//! CSS stylesheet for class-based styling.
//!
//! Parses a subset of CSS (class selectors, `:root` variables) and resolves
//! class attributes to property maps. Supports `var(--name)` resolution and
//! `rem` to `px` conversion.
//!
//! Uses Mozilla's `cssparser` for tokenization and rule parsing, and the
//! `selectors` crate for proper selector parsing and matching.

mod parser;
pub mod selector_impl;
mod var_resolution;

pub use selector_impl::*;
pub use var_resolution::{compute_merged_styles, compute_merged_styles_with_state};

use std::collections::HashMap;

use cssparser::{Parser, ParserInput, StyleSheetParser};

use selectors::context::{
    MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode,
    SelectorCaches,
};
use selectors::matching::matches_selector_list;
use selectors::parser::{ParseRelative, SelectorList};

use parser::RinchRuleParser;
use selector_impl::RinchSelectorParser;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A parsed CSS rule: selectors + property map.
#[derive(Debug, Clone)]
pub struct CssRule {
    /// The raw selector text (kept for :root detection and debugging).
    pub selector_text: String,
    /// Parsed selector list from the selectors crate.
    pub selectors: Option<SelectorList<RinchSelectorImpl>>,
    /// Property name -> (value, is_important).
    pub properties: HashMap<String, (String, bool)>,
}

/// A stylesheet that stores CSS rules and variables.
#[derive(Debug, Clone)]
pub struct Stylesheet {
    /// CSS rules in source order.
    pub rules: Vec<CssRule>,
    /// CSS custom properties from `:root` blocks.
    pub variables: HashMap<String, String>,
}

impl Default for Stylesheet {
    fn default() -> Self {
        Self::new()
    }
}

impl Stylesheet {
    /// Create an empty stylesheet.
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            variables: HashMap::new(),
        }
    }

    /// Parse a CSS string and return a new Stylesheet.
    pub fn parse(css: &str) -> Self {
        let mut stylesheet = Self::new();
        stylesheet.add_css(css);
        stylesheet
    }

    /// Parse CSS and merge into this stylesheet.
    pub fn add_css(&mut self, css: &str) {
        let css = css.trim();
        if css.is_empty() {
            return;
        }

        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        let mut rule_parser = RinchRuleParser;

        for result in StyleSheetParser::new(&mut parser, &mut rule_parser) {
            let rule = match result {
                Ok(r) => r,
                Err(_) => continue,
            };

            if rule.properties.is_empty() {
                continue;
            }

            // Handle :root selector — extract variables
            if rule.selector_text.trim() == ":root" {
                for (k, (v, _)) in &rule.properties {
                    if k.starts_with("--") {
                        self.variables.insert(k.clone(), v.clone());
                    }
                }
                continue;
            }

            // Try to parse selector with the selectors crate
            let selector_text = rule.selector_text.clone();
            let parsed = {
                let mut sel_input = ParserInput::new(&selector_text);
                let mut sel_parser = Parser::new(&mut sel_input);
                SelectorList::parse(&RinchSelectorParser, &mut sel_parser, ParseRelative::No).ok()
            };

            if parsed.is_none() {
                tracing::warn!("Failed to parse selector: {:?}", selector_text);
                // Still store the rule without parsed selectors for debugging
                continue;
            }

            self.rules.push(CssRule {
                selector_text,
                selectors: parsed,
                properties: rule.properties,
            });
        }
    }

    /// Recursively resolve `var(--name)` references in a value.
    /// Returns the resolved value. Handles cycles via max depth.
    pub fn resolve_var(&self, value: &str) -> String {
        self.resolve_var_depth(value, 0)
    }

    fn resolve_var_depth(&self, value: &str, depth: usize) -> String {
        if depth > 10 || !value.contains("var(") {
            return value.to_string();
        }

        let mut result = String::with_capacity(value.len());
        let mut remaining = value;

        while let Some(start) = remaining.find("var(") {
            result.push_str(&remaining[..start]);
            let after_var = &remaining[start + 4..];

            // Find matching closing paren, handling nested parens
            let mut depth_paren = 1;
            let mut end = 0;
            for (i, ch) in after_var.char_indices() {
                match ch {
                    '(' => depth_paren += 1,
                    ')' => {
                        depth_paren -= 1;
                        if depth_paren == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if depth_paren != 0 {
                // Malformed — just append the rest
                result.push_str(&remaining[start..]);
                return result;
            }

            let inner = &after_var[..end].trim();
            // inner might be `--name` or `--name, fallback`
            let (var_name, fallback) = if let Some(comma_pos) = inner.find(',') {
                (
                    inner[..comma_pos].trim(),
                    Some(inner[comma_pos + 1..].trim()),
                )
            } else {
                (*inner, None)
            };

            if let Some(resolved) = self.variables.get(var_name) {
                let resolved = self.resolve_var_depth(resolved, depth + 1);
                result.push_str(&resolved);
            } else if let Some(fb) = fallback {
                let resolved = self.resolve_var_depth(fb, depth + 1);
                result.push_str(&resolved);
            } else {
                // Unresolved, keep original
                result.push_str(&remaining[start..start + 4 + end + 1]);
            }

            remaining = &after_var[end + 1..];
        }

        result.push_str(remaining);
        result
    }

    /// Convert `rem` units to `px` (1rem = 16px) in potentially compound values.
    pub fn resolve_unit(value: &str) -> String {
        let value = value.trim();
        // Quick check — if no 'rem' anywhere, return as-is
        if !value.contains("rem") {
            return value.to_string();
        }
        // Split on whitespace, convert each part
        let parts: Vec<String> = value
            .split_whitespace()
            .map(|part| {
                if let Some(rem_str) = part.strip_suffix("rem")
                    && let Ok(num) = rem_str.parse::<f64>()
                {
                    return format!("{}px", num * 16.0);
                }
                part.to_string()
            })
            .collect();
        parts.join(" ")
    }

    /// Re-parse CSS and update only `:root` variables (no rule duplication).
    /// Use this when theme CSS changes at runtime to update CSS custom properties
    /// without re-adding all the non-`:root` rules.
    pub fn update_variables_from_css(&mut self, css: &str) {
        let css = css.trim();
        if css.is_empty() {
            return;
        }

        self.variables.clear();

        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        let mut rule_parser = RinchRuleParser;

        for result in StyleSheetParser::new(&mut parser, &mut rule_parser) {
            let rule = match result {
                Ok(r) => r,
                Err(_) => continue,
            };
            if rule.selector_text.trim() == ":root" {
                for (k, (v, _)) in &rule.properties {
                    if k.starts_with("--") {
                        self.variables.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }

    /// Resolve all units in a value: var() first, then rem.
    /// Viewport units (vh/vw) are left as-is for the layout engine to resolve.
    pub fn resolve_value(&self, value: &str) -> String {
        let resolved = self.resolve_var(value);
        Self::resolve_unit(&resolved)
    }

    /// Match a space-separated class string against all rules.
    /// Returns merged properties (later rules override earlier ones).
    /// Compound selectors `.a.b` only match if all classes are present.
    /// Respects !important: an !important value won't be overridden by a non-!important value.
    pub fn match_classes(&self, class_attr: &str) -> HashMap<String, (String, bool)> {
        self.match_classes_with_tag(class_attr, None)
    }

    /// Match with both classes and an optional tag name.
    pub fn match_classes_with_tag(
        &self,
        class_attr: &str,
        tag: Option<&str>,
    ) -> HashMap<String, (String, bool)> {
        let classes: Vec<&str> = class_attr.split_whitespace().collect();
        let state = ElementState {
            classes: classes.iter().map(|s| s.to_string()).collect(),
            tag: tag.map(|t| t.to_string()),
            ..Default::default()
        };
        self.match_element(&state, &[])
    }

    /// Match rules against an element with full state (pseudo-classes, ancestors, attributes).
    pub fn match_element(
        &self,
        element: &ElementState,
        ancestors: &[ElementState],
    ) -> HashMap<String, (String, bool)> {
        // Note: we don't early-return on empty classes/tag because universal selectors (*)
        // and attribute selectors can still match.

        // Build the chain: [element, parent, grandparent, ...]
        let mut chain = Vec::with_capacity(1 + ancestors.len());
        chain.push(element.clone());
        chain.extend_from_slice(ancestors);

        let elem = RinchElement {
            index: 0,
            chain: &chain,
        };

        // Track (value, important, specificity, source_order) per property
        // to implement correct CSS cascade: !important > specificity > source order
        let mut result: HashMap<String, (String, bool, u32, usize)> = HashMap::new();
        let mut caches = SelectorCaches::default();

        for (rule_index, rule) in self.rules.iter().enumerate() {
            if let Some(ref selector_list) = rule.selectors {
                let mut context = MatchingContext::new(
                    MatchingMode::Normal,
                    None,
                    &mut caches,
                    QuirksMode::NoQuirks,
                    NeedsSelectorFlags::No,
                    MatchingForInvalidation::No,
                );

                if matches_selector_list(selector_list, &elem, &mut context) {
                    // Compute max specificity of matching selectors in this rule
                    let specificity = selector_list
                        .slice()
                        .iter()
                        .map(|s| s.specificity())
                        .max()
                        .unwrap_or(0);

                    for (k, (v, imp)) in &rule.properties {
                        let should_insert =
                            if let Some((_, existing_imp, existing_spec, existing_order)) =
                                result.get(k)
                            {
                                if *imp && !existing_imp {
                                    // New is !important, existing is not -> new wins
                                    true
                                } else if !imp && *existing_imp {
                                    // Existing is !important, new is not -> existing wins
                                    false
                                } else {
                                    // Same importance level: higher specificity wins,
                                    // equal specificity: later source order wins
                                    specificity > *existing_spec
                                        || (specificity == *existing_spec
                                            && rule_index >= *existing_order)
                                }
                            } else {
                                true
                            };

                        if should_insert {
                            result.insert(k.clone(), (v.clone(), *imp, specificity, rule_index));
                        }
                    }
                }
            }
        }

        // Strip specificity/order tracking from result
        result
            .into_iter()
            .map(|(k, (v, imp, _, _))| (k, (v, imp)))
            .collect()
    }

    /// Resolve class-based styles for a node, with var() and rem resolution.
    pub fn resolve_class_styles(&self, class_attr: &str) -> HashMap<String, String> {
        let props = self.match_classes(class_attr);
        let mut result = HashMap::new();
        for (k, (v, _)) in props {
            let resolved = self.resolve_value(&v);
            result.insert(k, resolved);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_rules() {
        let css = ".foo { color: red; font-size: 14px; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 1);
        let props = ss.match_classes("foo");
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("red"));
        assert_eq!(
            props.get("font-size").map(|(v, _)| v.as_str()),
            Some("14px")
        );
    }

    #[test]
    fn test_compound_selectors() {
        let css = ".a.b { color: blue; }";
        let ss = Stylesheet::parse(css);
        // Both classes present
        let props = ss.match_classes("a b");
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("blue"));
        // Only one class — no match
        let props = ss.match_classes("a");
        assert!(!props.contains_key("color"));
        let props = ss.match_classes("b");
        assert!(!props.contains_key("color"));
    }

    #[test]
    fn test_var_resolution() {
        let css = r#"
            :root { --rinch-primary-color: #339af0; --rinch-alias: var(--rinch-primary-color); }
            .btn { color: var(--rinch-primary-color); background: var(--rinch-alias); }
        "#;
        let ss = Stylesheet::parse(css);
        let props = ss.resolve_class_styles("btn");
        assert_eq!(props.get("color").unwrap(), "#339af0");
        assert_eq!(props.get("background").unwrap(), "#339af0");
    }

    #[test]
    fn test_rem_conversion() {
        assert_eq!(Stylesheet::resolve_unit("1.5rem"), "24px");
        assert_eq!(Stylesheet::resolve_unit("1rem"), "16px");
        assert_eq!(Stylesheet::resolve_unit("0.25rem"), "4px");
        assert_eq!(Stylesheet::resolve_unit("10px"), "10px");
    }

    #[test]
    fn test_inline_overrides_class() {
        let css = ".foo { color: red; font-size: 14px; }";
        let ss = Stylesheet::parse(css);
        let merged = compute_merged_styles(&ss, Some("foo"), Some("color: blue"), None);
        assert_eq!(merged.get("color").unwrap(), "blue");
        assert_eq!(merged.get("font-size").unwrap(), "14px");
    }

    #[test]
    fn test_pseudo_selectors_stored() {
        let css = ".foo:hover { color: red; } .bar { color: blue; }";
        let ss = Stylesheet::parse(css);
        // Both rules stored
        assert_eq!(ss.rules.len(), 2);
        // match_classes (no state) doesn't match :hover
        assert!(!ss.match_classes("foo").contains_key("color"));
        assert_eq!(
            ss.match_classes("bar")
                .get("color")
                .map(|(v, _)| v.as_str()),
            Some("blue")
        );
        // match_element with hover state matches
        let state = ElementState {
            classes: vec!["foo".into()],
            is_hovered: true,
            ..Default::default()
        };
        let props = ss.match_element(&state, &[]);
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("red"));
    }

    #[test]
    fn test_skip_at_rules() {
        let css = "@keyframes spin { from { transform: rotate(0); } to { transform: rotate(360deg); } } .foo { color: red; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 1);
        assert_eq!(
            ss.match_classes("foo")
                .get("color")
                .map(|(v, _)| v.as_str()),
            Some("red")
        );
    }

    #[test]
    fn test_multiple_css_loads() {
        let mut ss = Stylesheet::new();
        ss.add_css(":root { --color: red; }");
        ss.add_css(".btn { color: var(--color); }");
        let props = ss.resolve_class_styles("btn");
        assert_eq!(props.get("color").unwrap(), "red");
    }

    #[test]
    fn test_later_rules_override() {
        let css = ".foo { color: red; } .foo { color: blue; }";
        let ss = Stylesheet::parse(css);
        let props = ss.match_classes("foo");
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("blue"));
    }

    #[test]
    fn test_comma_separated_selectors() {
        let css = ".a, .b { color: red; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(
            ss.match_classes("a").get("color").map(|(v, _)| v.as_str()),
            Some("red")
        );
        assert_eq!(
            ss.match_classes("b").get("color").map(|(v, _)| v.as_str()),
            Some("red")
        );
    }

    #[test]
    fn test_var_with_fallback() {
        let css = ".foo { color: var(--undefined, green); }";
        let ss = Stylesheet::parse(css);
        let props = ss.resolve_class_styles("foo");
        assert_eq!(props.get("color").unwrap(), "green");
    }

    #[test]
    fn test_css_comments_handled() {
        let css = "/* Comment at start */ .foo { color: red; /* inline comment */ font-size: 14px; } /* end */";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 1);
        let props = ss.match_classes("foo");
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("red"));
        assert_eq!(
            props.get("font-size").map(|(v, _)| v.as_str()),
            Some("14px")
        );
    }

    #[test]
    fn test_descendant_selectors() {
        let css = ".a .b { color: red; } .c { color: blue; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 2);
        // match_classes without ancestors doesn't match descendant selector
        assert!(!ss.match_classes("b").contains_key("color"));
        assert_eq!(
            ss.match_classes("c").get("color").map(|(v, _)| v.as_str()),
            Some("blue")
        );
        // match_element with proper ancestor chain matches
        let element = ElementState::from_classes("b");
        let parent = ElementState::from_classes("a");
        let props = ss.match_element(&element, &[parent]);
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("red"));
    }

    #[test]
    fn test_strip_important_from_values() {
        let css = ".btn { color: red !important; background: blue!important; font-size: 14px; }";
        let ss = Stylesheet::parse(css);
        let props = ss.match_classes("btn");
        // !important should be stripped from all values, but tracked
        assert_eq!(
            props.get("color").map(|(v, imp)| (v.as_str(), *imp)),
            Some(("red", true))
        );
        assert_eq!(
            props.get("background").map(|(v, imp)| (v.as_str(), *imp)),
            Some(("blue", true))
        );
        assert_eq!(
            props.get("font-size").map(|(v, imp)| (v.as_str(), *imp)),
            Some(("14px", false))
        );
    }

    #[test]
    fn test_strip_important_from_inline() {
        let css = ".foo { color: red; }";
        let ss = Stylesheet::parse(css);
        let merged = compute_merged_styles(
            &ss,
            Some("foo"),
            Some("color: green !important; padding: 10px!important"),
            None,
        );
        // Inline styles with !important should also have it stripped and override class styles
        assert_eq!(merged.get("color").unwrap(), "green");
        assert_eq!(merged.get("padding").unwrap(), "10px");
    }

    #[test]
    fn test_important_overrides() {
        let css = r#"
            .base { display: flex; color: red; }
            .hidden { display: none !important; }
        "#;
        let ss = Stylesheet::parse(css);

        // Both classes: !important display wins
        let merged = compute_merged_styles(&ss, Some("base hidden"), None, None);
        assert_eq!(merged.get("display").unwrap(), "none");

        // Inline can't override !important without !important
        let merged = compute_merged_styles(&ss, Some("hidden"), Some("display: flex"), None);
        assert_eq!(merged.get("display").unwrap(), "none");

        // Inline !important CAN override class !important
        let merged =
            compute_merged_styles(&ss, Some("hidden"), Some("display: flex !important"), None);
        assert_eq!(merged.get("display").unwrap(), "flex");
    }

    #[test]
    fn test_important_prevents_later_override() {
        let css = r#"
            .first { color: red !important; }
            .second { color: blue; }
        "#;
        let ss = Stylesheet::parse(css);

        // !important in first rule prevents second rule from overriding
        let props = ss.match_classes("first second");
        assert_eq!(
            props.get("color").map(|(v, imp)| (v.as_str(), *imp)),
            Some(("red", true))
        );

        // But if second also has !important, it wins (later rule)
        let css2 = r#"
            .first { color: red !important; }
            .second { color: blue !important; }
        "#;
        let ss2 = Stylesheet::parse(css2);
        let props2 = ss2.match_classes("first second");
        assert_eq!(
            props2.get("color").map(|(v, imp)| (v.as_str(), *imp)),
            Some(("blue", true))
        );
    }

    #[test]
    fn test_universal_selector() {
        let css = "* { box-sizing: border-box; margin: 0; padding: 0; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 1, "Universal selector should parse");
        // Check that all 3 properties are stored
        let rule_props = &ss.rules[0].properties;
        eprintln!("Universal rule props: {:?}", rule_props);
        assert!(
            rule_props.contains_key("box-sizing"),
            "should have box-sizing"
        );
        assert!(rule_props.contains_key("margin"), "should have margin");
        assert!(rule_props.contains_key("padding"), "should have padding");
        // Universal selector matches any element with a tag
        let props = ss.match_classes_with_tag("", Some("div"));
        assert_eq!(
            props.get("box-sizing").map(|(v, _)| v.as_str()),
            Some("border-box")
        );
        assert_eq!(props.get("margin").map(|(v, _)| v.as_str()), Some("0"));
        assert_eq!(props.get("padding").map(|(v, _)| v.as_str()), Some("0"));
    }

    #[test]
    fn test_universal_plus_class_merge() {
        let css = r#"
            * { box-sizing: border-box; margin: 0; padding: 0; }
            .main-content { padding: 20px; }
        "#;
        let ss = Stylesheet::parse(css);
        let merged = compute_merged_styles(&ss, Some("main-content"), None, Some("div"));
        eprintln!("Merged main-content: {:?}", merged);
        assert_eq!(
            merged.get("box-sizing").map(|v| v.as_str()),
            Some("border-box")
        );
        assert_eq!(merged.get("margin").map(|v| v.as_str()), Some("0"));
        assert_eq!(merged.get("padding").map(|v| v.as_str()), Some("20px"));
    }

    #[test]
    fn test_element_type_selectors() {
        let css = "h1 { font-size: 2em; } a { color: blue; } code { font-family: monospace; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 3);
        let props = ss.match_classes_with_tag("", Some("h1"));
        assert_eq!(props.get("font-size").map(|(v, _)| v.as_str()), Some("2em"));
        let props = ss.match_classes_with_tag("some-class", Some("a"));
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("blue"));
    }

    #[test]
    fn test_borderless_window_matching() {
        let css = r#"
            .rinch-borderlesswindow { display: flex; flex-direction: column; height: 100vh; }
            .rinch-borderlesswindow--radius-md { border-radius: 8px; }
            .rinch-borderlesswindow__content { flex: 1; min-height: 0; }
            .rinch-borderlesswindow__titlebar { display: flex; height: 40px; }
        "#;
        let ss = Stylesheet::parse(css);
        assert_eq!(
            ss.rules.len(),
            4,
            "Expected 4 rules, got {}",
            ss.rules.len()
        );

        // Element with two classes
        let props = ss.match_classes("rinch-borderlesswindow rinch-borderlesswindow--radius-md");
        assert_eq!(
            props.get("display").map(|(v, _)| v.as_str()),
            Some("flex"),
            "display missing"
        );
        assert_eq!(
            props.get("height").map(|(v, _)| v.as_str()),
            Some("100vh"),
            "height missing"
        );
        assert_eq!(
            props.get("border-radius").map(|(v, _)| v.as_str()),
            Some("8px"),
            "border-radius missing"
        );

        // Content child
        let props2 = ss.match_classes("rinch-borderlesswindow__content");
        assert_eq!(
            props2.get("flex").map(|(v, _)| v.as_str()),
            Some("1"),
            "flex missing"
        );
    }

    #[test]
    fn test_inline_custom_property_resolution() {
        // Test that inline custom properties can reference global variables
        let css = r#"
            :root { --rinch-color-cyan-6: #0c8599; }
        "#;
        let ss = Stylesheet::parse(css);

        // Inline style defines a local custom property that references a global one
        let inline = "--rinch-action-icon-color: var(--rinch-color-cyan-6); background-color: var(--rinch-action-icon-color, transparent)";
        let merged = compute_merged_styles(&ss, None, Some(inline), None);

        // The local custom property should be resolved
        assert_eq!(merged.get("--rinch-action-icon-color").unwrap(), "#0c8599");
        // And it should be used to resolve other properties
        assert_eq!(merged.get("background-color").unwrap(), "#0c8599");
    }

    #[test]
    fn test_inline_custom_property_chain() {
        // Test chained local custom properties
        let css = r#"
            :root { --base-color: red; }
        "#;
        let ss = Stylesheet::parse(css);

        let inline =
            "--local-a: var(--base-color); --local-b: var(--local-a); color: var(--local-b)";
        let merged = compute_merged_styles(&ss, None, Some(inline), None);

        assert_eq!(merged.get("--local-a").unwrap(), "red");
        assert_eq!(merged.get("--local-b").unwrap(), "red");
        assert_eq!(merged.get("color").unwrap(), "red");
    }

    #[test]
    fn test_inline_custom_property_with_fallback() {
        // Test that fallbacks work with local custom properties
        let css = r#"
            :root { --rinch-color-blue: blue; }
        "#;
        let ss = Stylesheet::parse(css);

        let inline =
            "--local-color: var(--undefined, var(--rinch-color-blue)); color: var(--local-color)";
        let merged = compute_merged_styles(&ss, None, Some(inline), None);

        assert_eq!(merged.get("--local-color").unwrap(), "blue");
        assert_eq!(merged.get("color").unwrap(), "blue");
    }
}
