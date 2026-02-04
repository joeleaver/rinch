//! Input rules for automatic transformations.

use crate::error::EditorError;
use crate::editor::Editor;
use regex::Regex;

/// Handler function type for input rules.
pub type InputRuleHandler = fn(&mut Editor, &regex::Captures) -> Result<(), EditorError>;

/// An input rule that triggers automatic transformations based on text patterns.
pub struct InputRule {
    /// Pattern to match
    pub pattern: Regex,
    /// Description of what this rule does
    pub description: String,
    /// Handler to execute when matched
    pub handler: InputRuleHandler,
}

impl InputRule {
    /// Create a new input rule.
    pub fn new(pattern: Regex, description: impl Into<String>, handler: InputRuleHandler) -> Self {
        Self {
            pattern,
            description: description.into(),
            handler,
        }
    }

    /// Check if this rule matches the given text, returning the captures if so.
    pub fn matches<'t>(&self, text: &'t str) -> Option<regex::Captures<'t>> {
        self.pattern.captures(text)
    }

    /// Apply the transformation if the pattern matches.
    pub fn apply(&self, editor: &mut Editor, text: &str) -> Result<bool, EditorError> {
        if let Some(captures) = self.pattern.captures(text) {
            (self.handler)(editor, &captures)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl std::fmt::Debug for InputRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputRule")
            .field("pattern", &self.pattern.as_str())
            .field("description", &self.description)
            .finish()
    }
}

/// A collection of input rules.
#[derive(Debug, Default)]
pub struct InputRuleSet {
    rules: Vec<InputRule>,
}

impl InputRuleSet {
    /// Create a new empty rule set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a rule to the set.
    pub fn add_rule(&mut self, rule: InputRule) {
        self.rules.push(rule);
    }

    /// Get the number of rules.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Check if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Check all rules against text and apply the first match.
    /// Returns Ok(true) if a rule was applied, Ok(false) if none matched.
    pub fn check_and_apply(&self, editor: &mut Editor, text: &str) -> Result<bool, EditorError> {
        for rule in &self.rules {
            if rule.apply(editor, text)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Schema;
    use crate::editor::EditorConfig;

    fn test_editor() -> Editor {
        Editor::new(Schema::starter_kit(), EditorConfig::default()).unwrap()
    }

    #[test]
    fn input_rule_matches_pattern() {
        let rule = InputRule::new(
            Regex::new(r"^# ").unwrap(),
            "Heading",
            |_editor, _caps| Ok(()),
        );
        assert!(rule.matches("# Hello").is_some());
        assert!(rule.matches("Hello").is_none());
    }

    #[test]
    fn input_rule_no_match_returns_false() {
        let rule = InputRule::new(
            Regex::new(r"^# ").unwrap(),
            "Heading",
            |_editor, _caps| Ok(()),
        );
        let mut editor = test_editor();
        assert_eq!(rule.apply(&mut editor, "Hello").unwrap(), false);
    }

    #[test]
    fn input_rule_match_returns_true() {
        let rule = InputRule::new(
            Regex::new(r"^# ").unwrap(),
            "Heading",
            |_editor, _caps| Ok(()),
        );
        let mut editor = test_editor();
        assert_eq!(rule.apply(&mut editor, "# Hello").unwrap(), true);
    }

    #[test]
    fn input_rule_set_applies_first_match() {
        let mut set = InputRuleSet::new();
        set.add_rule(InputRule::new(
            Regex::new(r"^## ").unwrap(),
            "H2",
            |_editor, _caps| Ok(()),
        ));
        set.add_rule(InputRule::new(
            Regex::new(r"^# ").unwrap(),
            "H1",
            |_editor, _caps| Ok(()),
        ));
        let mut editor = test_editor();
        // "## " matches first rule
        assert_eq!(set.check_and_apply(&mut editor, "## Hello").unwrap(), true);
    }

    #[test]
    fn input_rule_set_no_match() {
        let mut set = InputRuleSet::new();
        set.add_rule(InputRule::new(
            Regex::new(r"^# ").unwrap(),
            "Heading",
            |_editor, _caps| Ok(()),
        ));
        let mut editor = test_editor();
        assert_eq!(set.check_and_apply(&mut editor, "Hello").unwrap(), false);
    }

    #[test]
    fn input_rule_set_empty() {
        let set = InputRuleSet::new();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn input_rule_debug() {
        let rule = InputRule::new(
            Regex::new(r"^# ").unwrap(),
            "Heading rule",
            |_e, _c| Ok(()),
        );
        let dbg = format!("{:?}", rule);
        assert!(dbg.contains("Heading rule"));
    }
}
