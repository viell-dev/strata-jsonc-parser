//! Strict RFC 8259 behavior inside JSONC: everything `strata-json-parser` rejects stays
//! rejected — comments are the only lexical addition — plus the shared `allow_bom` and
//! `max_depth` options.

use strata_jsonc_parser::{Document, JsoncError, JsoncErrorKind, JsoncParser, TriviaKind};
use strata_lexer::LexErrorKind;

fn parse(input: &str) -> Result<Document, JsoncError> {
    JsoncParser::new().parse_str(input)
}

fn parse_error(input: &str) -> JsoncError {
    match parse(input) {
        Err(error) => error,
        Ok(document) => panic!("expected a parse error, got {:?}", document.root()),
    }
}

#[test]
fn non_json_literals_are_rejected() {
    // No RFC 8259 latitude, no JSONC extension: NaN/Infinity/undefined are not values.
    for input in ["NaN", "Infinity", "-Infinity", "undefined", "True"] {
        let error = parse_error(input);
        assert!(
            matches!(error.kind, JsoncErrorKind::Lex(_)),
            "input {input:?}, got {:?}",
            error.kind
        );
    }
}

#[test]
fn single_quoted_strings_are_rejected() {
    let error = parse_error("'single'");
    let JsoncErrorKind::Lex(lex) = error.kind else {
        panic!("expected a lexical error");
    };
    assert!(matches!(lex.kind, LexErrorKind::Unrecognized));
    assert_eq!(lex.position.byte(), 0);
}

#[test]
fn number_strictness_is_kept() {
    for input in ["01", "1.", ".5", "1e", "+1", "0x10"] {
        let error = parse_error(input);
        assert!(
            matches!(error.kind, JsoncErrorKind::Lex(_)),
            "input {input:?}, got {:?}",
            error.kind
        );
    }
}

#[test]
fn string_strictness_is_kept() {
    // Bad escape, unescaped control character, unterminated string.
    for input in [r#""\q""#, "\"a\tb\"", r#""open"#] {
        let error = parse_error(input);
        assert!(
            matches!(error.kind, JsoncErrorKind::Lex(_)),
            "input {input:?}, got {:?}",
            error.kind
        );
    }
}

#[test]
fn trailing_content_is_rejected_but_trailing_comments_are_not() {
    let error = parse_error("1 2");
    assert!(matches!(error.kind, JsoncErrorKind::TrailingContent { .. }));

    // Comments and whitespace after the value are trivia, not trailing content.
    assert!(parse("1 // fine\n/* also fine */").is_ok());
}

#[test]
fn bom_is_rejected_by_default_and_retained_when_allowed() -> Result<(), JsoncError> {
    let error = parse_error("\u{FEFF}// c\n1");
    assert!(matches!(error.kind, JsoncErrorKind::UnexpectedBom));
    assert_eq!(error.position.byte(), 0);

    let document = JsoncParser::builder()
        .allow_bom(true)
        .build()
        .parse_str("\u{FEFF}// c\n1")?;
    let Some(bom) = document.bom() else {
        panic!("expected the BOM to be retained as trivia");
    };
    assert_eq!(bom.kind(), TriviaKind::Bom);
    assert_eq!(bom.span().end(), 3);

    Ok(())
}

#[test]
fn depth_limit_is_enforced() {
    let parser = JsoncParser::builder().max_depth(2).build();

    assert!(parser.parse_str("[[1]]").is_ok());

    let Err(error) = parser.parse_str("[[[1]]]") else {
        panic!("expected a depth error");
    };
    assert!(matches!(
        error.kind,
        JsoncErrorKind::DepthLimitExceeded { limit: 2 }
    ));

    // Comments do not consume depth.
    assert!(parser.parse_str("[/* c */ [/* c */ 1]]").is_ok());
}

#[test]
fn error_positions_carry_line_column_and_byte() {
    // The offending `}` sits on line 3 after a comment line.
    let error = parse_error("[\n// c\n}");
    assert_eq!(error.position.line(), 3);
    assert_eq!(error.position.column(), 1);
    assert_eq!(error.position.byte(), 7);
}

#[test]
fn reusable_parser_serves_many_inputs() -> Result<(), JsoncError> {
    let parser = JsoncParser::new();

    for input in ["1", "// c\ntrue", "{\"a\": [/* b */ null]}"] {
        parser.parse_str(input)?;
    }

    Ok(())
}
