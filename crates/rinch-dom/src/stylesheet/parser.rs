//! CSS parser implementations using cssparser.

use std::collections::HashMap;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};

/// A single parsed declaration (property: value with optional !important).
pub(super) struct ParsedDeclaration {
    pub(super) name: String,
    pub(super) value: String,
    pub(super) important: bool,
}

/// Parser for declarations inside a rule block.
pub(super) struct RinchDeclarationParser;

impl<'i> DeclarationParser<'i> for RinchDeclarationParser {
    type Declaration = ParsedDeclaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _start: &ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        let start_pos = input.position();
        while input.next_including_whitespace().is_ok() {}
        let raw = input.slice_from(start_pos).trim();

        let (value, important) = if let Some(pos) = raw.rfind("!important") {
            let before = raw[..pos].trim_end();
            if before.is_empty() {
                return Err(input.new_custom_error(()));
            }
            (before.to_string(), true)
        } else {
            (raw.to_string(), false)
        };

        if value.is_empty() {
            return Err(input.new_custom_error(()));
        }

        Ok(ParsedDeclaration {
            name: name.to_string(),
            value,
            important,
        })
    }
}

impl<'i> AtRuleParser<'i> for RinchDeclarationParser {
    type Prelude = ();
    type AtRule = ParsedDeclaration;
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for RinchDeclarationParser {
    type Prelude = ();
    type QualifiedRule = ParsedDeclaration;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, ParsedDeclaration, ()> for RinchDeclarationParser {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

/// Intermediate rule from top-level parsing (before selector parsing).
pub(super) struct RawCssRule {
    pub(super) selector_text: String,
    pub(super) properties: HashMap<String, (String, bool)>,
}

/// Top-level rule parser for stylesheets.
pub(super) struct RinchRuleParser;

impl<'i> AtRuleParser<'i> for RinchRuleParser {
    type Prelude = ();
    type AtRule = RawCssRule;
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for RinchRuleParser {
    type Prelude = String;
    type QualifiedRule = RawCssRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        // Collect the entire prelude as a string (selector text)
        let start = input.position();
        while input.next_including_whitespace().is_ok() {}
        let selector = input.slice_from(start).trim().to_string();
        if selector.is_empty() {
            return Err(input.new_custom_error(()));
        }
        Ok(selector)
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {
        // Parse declarations inside the block
        let mut properties = HashMap::new();
        let mut decl_parser = RinchDeclarationParser;
        for decl in RuleBodyParser::new(input, &mut decl_parser).flatten() {
            properties.insert(decl.name, (decl.value, decl.important));
        }

        Ok(RawCssRule {
            selector_text: prelude,
            properties,
        })
    }
}

/// Parse CSS property declarations from a string (used for inline styles).
pub(super) fn parse_properties(body: &str) -> HashMap<String, (String, bool)> {
    let mut input = ParserInput::new(body);
    let mut parser = Parser::new(&mut input);
    let mut decl_parser = RinchDeclarationParser;
    let mut result = HashMap::new();
    for decl in RuleBodyParser::new(&mut parser, &mut decl_parser).flatten() {
        result.insert(decl.name, (decl.value, decl.important));
    }
    result
}
