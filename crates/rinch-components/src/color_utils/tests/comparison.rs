use super::*;

#[test]
fn denotes_same_folds_notation_the_format_erases() {
    // Short hex expands to the long form.
    assert!(denotes_same("#336", "#333366", ColorFormat::Hex));
    // Digit case is not a colour difference.
    assert!(denotes_same("#FF0000", "#ff0000", ColorFormat::Hex));
    // An alpha pair is dropped by a non-alpha format...
    assert!(denotes_same("#3333666c", "#333366", ColorFormat::Hex));
    // ...but is a real difference under one that keeps it.
    assert!(!denotes_same("#3333666c", "#333366", ColorFormat::Hexa));
    // Spelling across colour syntaxes folds too.
    assert!(denotes_same("rgb(255, 0, 0)", "#ff0000", ColorFormat::Hex));
}

#[test]
fn denotes_same_separates_actual_colours() {
    assert!(!denotes_same("#336", "#337", ColorFormat::Hex));
    assert!(!denotes_same("rgb(255, 0, 0)", "#ff0001", ColorFormat::Hex));
}

#[test]
fn denotes_same_compares_unparseable_text_literally() {
    // Two copies of the same unfinished prefix match, whitespace aside.
    assert!(denotes_same("#33", "#33", ColorFormat::Hex));
    assert!(denotes_same(" #33 ", "#33", ColorFormat::Hex));
    // Different unfinished text does not.
    assert!(!denotes_same("#33", "#34", ColorFormat::Hex));
    // A prefix too short to parse never equals a parseable colour, even
    // the one it is on its way to.
    assert!(!denotes_same("#33", "#333333", ColorFormat::Hex));
}

/// `display_spelling` is the display-format view of a colour string:
/// a parseable value in any notation re-spells in `format`, and text
/// that parses as no colour is returned unchanged (GH #237).
#[test]
fn display_spelling_respells_a_colour_and_keeps_unparseable_text() {
    assert_eq!(display_spelling("red", ColorFormat::Hex), "#ff0000");
    assert_eq!(
        display_spelling("hsl(200, 3%, 49%)", ColorFormat::Hex),
        "#797e81"
    );
    assert_eq!(
        display_spelling("#336", ColorFormat::Rgb),
        "rgb(51, 51, 102)"
    );
    assert_eq!(display_spelling("#0000ff80", ColorFormat::Hex), "#0000ff");
    assert_eq!(display_spelling("#33", ColorFormat::Hex), "#33");
    assert_eq!(display_spelling("", ColorFormat::Hex), "");
    // `denotes_same` is equality of the two spellings on the trimmed pair.
    assert!(denotes_same(" red ", "#ff0000", ColorFormat::Hex));
    assert!(!denotes_same("#33", "#333333", ColorFormat::Hex));
}

#[test]
fn text_denotes_compares_the_full_colour() {
    let navy = parse_color("#333366").unwrap();
    assert!(text_denotes("#336", navy));
    assert!(text_denotes("rgb(51, 51, 102)", navy));
    assert!(!text_denotes("#333367", navy));

    // Alpha is part of the colour even when a display format drops it:
    // the typed pair agrees only while the colour really holds it.
    let translucent = parse_color("#3333666c").unwrap();
    assert!(text_denotes("#3333666c", translucent));
    assert!(!text_denotes("#3333666c", navy));
    assert!(!text_denotes("#333366", translucent));

    // Unparseable text denotes nothing.
    assert!(!text_denotes("#33", navy));
    assert!(!text_denotes("", navy));
}
