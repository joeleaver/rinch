//! Inline styles from a document someone else wrote, filtered.
//!
//! Each declaration is parsed with cssparser, so a property is judged by the
//! name the browser reads (`posit\69 on` is `position`, a comment inside a
//! name is no name), a value's strings keep their `;` and `)`, and a picture
//! is every `url()`, quoted or not, escaped or not. What is kept is written
//! back token by token from what was parsed, never from the source text.
//!
//! Left out:
//! - what lifts content out of its own box onto the app: `position` other
//!   than `static`/`relative`, `z-index`;
//! - what runs code or changes how the app is used: `behavior`,
//!   `-moz-binding`, `expression()`, a string naming `javascript:`, `cursor`,
//!   `pointer-events`;
//! - a picture the [`PictureRule`] refuses, and every way to load one other
//!   than `url()` (`image-set()`, `image()`, `cross-fade()`, `element()`,
//!   `paint()`, `src()`), which the rule would never see;
//! - a number over [`MAX_LENGTH`] anywhere in a value (in a `calc()` too),
//!   and a `font-size` or `line-height` over [`MAX_FONT_SIZE`] px in any unit
//!   (`pt`, `em`, `%`, a bare number, counted at 16px): no document needs one,
//!   and laying it out costs the app.

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, ToCss, Token,
};

use crate::PictureRule;

/// Properties left out whatever their value.
const DROPPED: &[&str] = &[
    "behavior",
    "-moz-binding",
    "cursor",
    "pointer-events",
    "z-index",
];

/// Functions that load a picture without `url()`, or run code.
const BLOCKED_FUNCTIONS: &[&str] = &[
    "expression",
    "image-set",
    "-webkit-image-set",
    "image",
    "cross-fade",
    "-webkit-cross-fade",
    "element",
    "-moz-element",
    "paint",
    "src",
];

/// The largest length kept, in any unit's number.
pub const MAX_LENGTH: f32 = 100_000.0;
/// The largest `font-size` kept, in px.
pub const MAX_FONT_SIZE: f32 = 1_000.0;

/// `style` (an inline `style` attribute) with only what is safe kept; see
/// the module docs.
pub(crate) fn clean_style(style: &str, picture: &PictureRule) -> String {
    let mut parser = Parser::new(style);
    let mut declarations = Declarations { picture };
    let kept: Vec<String> = RuleBodyParser::new(&mut parser, &mut declarations)
        .filter_map(|d| d.ok().flatten())
        .collect();
    kept.join("; ")
}

struct Declarations<'a> {
    picture: &'a PictureRule,
}

type Kept = Option<String>;

impl<'i> DeclarationParser<'i> for Declarations<'_> {
    type Declaration = Kept;
    type Error = ();

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<Kept, ParseError<()>> {
        let name = name.to_ascii_lowercase();
        let mut value = String::new();
        let kept = walk(input, self.picture, &name, &mut value).is_ok();
        // Whatever the walk left unread is part of this declaration.
        while input.next_including_whitespace_and_comments().is_ok() {}
        if !kept || !plain_name(&name) || DROPPED.contains(&name.as_str()) {
            return Ok(None);
        }
        let value = value.trim();
        if value.is_empty() {
            return Ok(None);
        }
        if name == "position" {
            let bare = value
                .trim_end_matches("!important")
                .trim()
                .to_ascii_lowercase();
            if !matches!(bare.as_str(), "static" | "relative") {
                return Ok(None);
            }
        }
        Ok(Some(format!("{name}: {value}")))
    }
}

impl<'i> AtRuleParser<'i> for Declarations<'_> {
    type Prelude = ();
    type AtRule = Kept;
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for Declarations<'_> {
    type Prelude = ();
    type QualifiedRule = Kept;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, Kept, ()> for Declarations<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

/// A property name as written back: letters, digits and `-` only (an
/// escaped name could hold a `;` or a `:`).
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Write `input`'s tokens to `out`, refusing (with `Err`) what the module
/// docs list.
fn walk<'i>(
    input: &mut Parser<'i>,
    picture: &PictureRule,
    property: &str,
    out: &mut String,
) -> Result<(), ParseError<()>> {
    // A run of numbers joined by `*` (in a `calc()`), multiplied so far.
    let mut product: Option<f32> = None;
    let mut times = false;
    // The px per unit of the run's length, when it has one.
    let mut scale: Option<f32> = None;
    // Past a `/`: in `font`, the line height.
    let mut after_slash = false;
    loop {
        let token = match input.next_including_whitespace() {
            Ok(t) => t.clone(),
            Err(_) => return Ok(()),
        };
        let refuse = || Err(ParseError::custom(()));
        // Each number counts as large as the product it is part of.
        let magnitude = match &token {
            Token::Number { value, .. } | Token::Dimension { value, .. } => Some(value.abs()),
            Token::Percentage { unit_value, .. } => Some(unit_value.abs()),
            _ => None,
        };
        match (&token, magnitude) {
            (_, Some(m)) => {
                let unit = match &token {
                    Token::Dimension { unit, .. } => Some(unit_scale(unit)),
                    _ => None,
                };
                let multiplied = times;
                let m = if times { product.unwrap_or(1.0) * m } else { m };
                scale = if times { scale.or(unit) } else { unit };
                if m > MAX_LENGTH {
                    return refuse();
                }
                // A product sizing text counts in px (a bare one at 16px).
                if multiplied && sizes_text(property) && m * scale.unwrap_or(16.0) > MAX_FONT_SIZE {
                    return refuse();
                }
                product = Some(m);
                times = false;
            }
            (Token::Delim('*'), _) => times = true,
            (Token::Delim('/'), _) => after_slash = true,
            (Token::WhiteSpace(_), _) => {}
            _ => {
                product = None;
                times = false;
            }
        }
        match &token {
            Token::UnquotedUrl(src) => {
                let Some(url) = picture_url(src, picture) else {
                    return refuse();
                };
                out.push_str(&url);
            }
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                if BLOCKED_FUNCTIONS.contains(&name.as_str()) {
                    return refuse();
                }
                if name == "url" {
                    let src = input.parse_nested_block(|i| {
                        let src = i.expect_string()?.as_ref().to_owned();
                        i.expect_exhausted()?;
                        Ok(src)
                    })?;
                    let Some(url) = picture_url(&src, picture) else {
                        return refuse();
                    };
                    out.push_str(&url);
                    continue;
                }
                out.push_str(&token.to_css_string());
                input.parse_nested_block(|i| walk(i, picture, property, out))?;
                out.push(')');
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let close = match token {
                    Token::ParenthesisBlock => ')',
                    Token::SquareBracketBlock => ']',
                    _ => '}',
                };
                out.push_str(&token.to_css_string());
                input.parse_nested_block(|i| walk(i, picture, property, out))?;
                out.push(close);
            }
            Token::QuotedString(s) if s.to_ascii_lowercase().contains("javascript:") => {
                return refuse();
            }
            Token::BadUrl(_)
            | Token::BadString(_)
            | Token::CloseParenthesis
            | Token::CloseSquareBracket
            | Token::CloseCurlyBracket => return refuse(),
            Token::Dimension { value, unit, .. } => {
                let text_size = value * unit_scale(unit);
                if value.abs() > MAX_LENGTH || (sizes_text(property) && text_size > MAX_FONT_SIZE) {
                    return refuse();
                }
                out.push_str(&token.to_css_string());
            }
            // A bare number (a `line-height` of 1.4) and a percentage
            // (`unit_value`: 1.0 is 100%) both scale the 16px text size.
            // In `font`, a bare number before the `/` is a weight (700),
            // not a size.
            Token::Number { value, .. }
            | Token::Percentage {
                unit_value: value, ..
            } => {
                let text_size = value * 16.0;
                let bare_weight =
                    property == "font" && matches!(token, Token::Number { .. }) && !after_slash;
                if value.abs() > MAX_LENGTH
                    || (sizes_text(property) && !bare_weight && text_size > MAX_FONT_SIZE)
                {
                    return refuse();
                }
                out.push_str(&token.to_css_string());
            }
            _ => out.push_str(&token.to_css_string()),
        }
    }
}

/// CSS px per `unit` (an `em` counted at 16px); 0 for a unit that is not a
/// length.
fn unit_scale(unit: &str) -> f32 {
    match unit.to_ascii_lowercase().as_str() {
        "px" => 1.0,
        "pt" => 4.0 / 3.0,
        "pc" | "em" | "rem" | "ex" | "ch" | "lh" => 16.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        _ => 0.0,
    }
}

/// Whether `property` sizes text, so its value counts against
/// [`MAX_FONT_SIZE`].
fn sizes_text(property: &str) -> bool {
    matches!(property, "font-size" | "line-height" | "font")
}

/// `url("…")` for what the picture rule makes of `src`, or `None` if it
/// refuses it.
fn picture_url(src: &str, picture: &PictureRule) -> Option<String> {
    let kept = picture(src.trim())?;
    let mut out = String::from("url(");
    cssparser::serialize_string(&kept, &mut out).ok()?;
    out.push(')');
    Some(out)
}
