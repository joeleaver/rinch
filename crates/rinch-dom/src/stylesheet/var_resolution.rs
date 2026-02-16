//! CSS variable resolution and style merging.

use std::collections::HashMap;

use super::parser::parse_properties;
use super::Stylesheet;

/// Resolve var() references checking local custom properties first, then global stylesheet variables.
pub(super) fn resolve_var_with_locals(
    value: &str,
    stylesheet: &Stylesheet,
    local_vars: &HashMap<String, String>,
) -> String {
    if !value.contains("var(") {
        return value.to_string();
    }

    let mut result = String::with_capacity(value.len());
    let mut remaining = value;

    while let Some(start) = remaining.find("var(") {
        result.push_str(&remaining[..start]);
        let after_var = &remaining[start + 4..];

        // Find matching closing paren (handling nested parens)
        let mut depth = 1;
        let mut end = 0;
        for (i, ch) in after_var.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }

        if depth != 0 {
            // Unbalanced parens, keep as-is
            result.push_str(&remaining[start..]);
            remaining = "";
            break;
        }

        let inner = &after_var[..end];
        remaining = &after_var[end + 1..];

        // Split on first comma for fallback
        let (var_name, fallback) = if let Some(comma_pos) = inner.find(',') {
            (
                inner[..comma_pos].trim(),
                Some(inner[comma_pos + 1..].trim()),
            )
        } else {
            (inner.trim(), None)
        };

        // Check local vars first, then global
        if let Some(resolved) = local_vars.get(var_name) {
            result.push_str(resolved);
        } else if let Some(resolved) = stylesheet.variables.get(var_name) {
            // Recursively resolve the global var value
            let resolved = resolve_var_with_locals(resolved, stylesheet, local_vars);
            result.push_str(&resolved);
        } else if let Some(fb) = fallback {
            let resolved = resolve_var_with_locals(fb, stylesheet, local_vars);
            result.push_str(&resolved);
        } else {
            // Unresolved, keep original
            result.push_str("var(");
            result.push_str(inner);
            result.push(')');
        }
    }

    result.push_str(remaining);
    result
}

/// Compute merged style properties: class-based styles + inline overrides.
/// Resolves var() and rem in the final result.
/// Respects !important: inline styles won't override class-based !important values unless the inline style is also !important.
pub fn compute_merged_styles(
    stylesheet: &Stylesheet,
    class_attr: Option<&str>,
    inline_style: Option<&str>,
    tag: Option<&str>,
) -> HashMap<String, String> {
    compute_merged_styles_with_state(stylesheet, class_attr, inline_style, None, &[], tag, None)
}

/// Compute merged styles with full element state for pseudo-class and descendant matching.
/// The `inherited_custom_props` parameter allows CSS custom properties (--xxx) to be
/// inherited from parent elements, enabling proper CSS variable inheritance.
pub fn compute_merged_styles_with_state(
    stylesheet: &Stylesheet,
    class_attr: Option<&str>,
    inline_style: Option<&str>,
    element_state: Option<&super::ElementState>,
    ancestors: &[super::ElementState],
    tag: Option<&str>,
    inherited_custom_props: Option<&HashMap<String, String>>,
) -> HashMap<String, String> {
    // Get class-based styles with importance tracking
    let class_props = match (element_state, class_attr) {
        (Some(state), _) => stylesheet.match_element(state, ancestors),
        (None, Some(cls)) => stylesheet.match_classes_with_tag(cls, tag),
        (None, None) if tag.is_some() => stylesheet.match_classes_with_tag("", tag),
        _ => HashMap::new(),
    };

    // Parse inline styles
    let inline_props = if let Some(style_str) = inline_style {
        parse_properties(style_str)
    } else {
        HashMap::new()
    };

    // Merge: inline wins UNLESS class value is !important and inline is not
    let mut merged: HashMap<String, String> = HashMap::new();

    // Start with class-based
    for (k, (v, _)) in &class_props {
        merged.insert(k.clone(), v.clone());
    }

    // Build importance map from class props
    let important_keys: std::collections::HashSet<String> = class_props
        .iter()
        .filter(|(_, (_, imp))| *imp)
        .map(|(k, _)| k.clone())
        .collect();

    // Overlay inline styles, but don't override !important class values
    for (k, (v, imp)) in &inline_props {
        if important_keys.contains(k) && !imp {
            // Class has !important, inline doesn't — class wins
            continue;
        }
        merged.insert(k.clone(), v.clone());
    }

    // Resolve var() references, considering inherited, global :root, and local custom properties
    // Step 1: Start with inherited custom properties from parent (CSS custom property inheritance)
    let mut local_vars: HashMap<String, String> = HashMap::new();
    if let Some(inherited) = inherited_custom_props {
        for (key, value) in inherited {
            if key.starts_with("--") {
                local_vars.insert(key.clone(), value.clone());
            }
        }
    }

    // Step 2: Override/add local custom properties (--xxx) from this element
    for (key, value) in &merged {
        if key.starts_with("--") {
            // Resolve the value using inherited vars + global vars
            local_vars.insert(
                key.clone(),
                resolve_var_with_locals(&stylesheet.resolve_value(value), stylesheet, &local_vars),
            );
        }
    }

    // Step 3: Re-resolve local vars in case they reference each other
    for _ in 0..5 {
        let mut changed = false;
        let vars_snapshot = local_vars.clone();
        for (key, value) in local_vars.iter_mut() {
            if key.starts_with("--") && value.contains("var(") {
                let resolved = resolve_var_with_locals(value, stylesheet, &vars_snapshot);
                if &resolved != value {
                    *value = resolved;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Step 4: Add resolved custom properties back to merged (for inheritance to children)
    for (key, value) in &local_vars {
        merged.insert(key.clone(), value.clone());
    }

    // Step 5: Resolve all properties using a combined lookup (local vars first, then global)
    for value in merged.values_mut() {
        // First resolve using local custom properties
        let mut resolved = value.clone();
        // Keep resolving until no more var() references change
        for _ in 0..10 {
            let prev = resolved.clone();
            resolved = resolve_var_with_locals(&resolved, stylesheet, &local_vars);
            if resolved == prev {
                break;
            }
        }
        // Then resolve rem units
        resolved = Stylesheet::resolve_unit(&resolved);
        *value = resolved;
    }

    merged
}
