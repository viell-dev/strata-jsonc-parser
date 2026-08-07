//! Comment behavior: every jsonc.org comment form, comment placement, and the shapes the spec
//! explicitly excludes.

use strata_jsonc_parser::{
    Document, JsoncError, JsoncErrorKind, JsoncParser, Trivia, TriviaKind, Value,
};
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

/// Asserts the error is a lexical error and returns it.
fn lex_error(input: &str) -> strata_lexer::LexError {
    match parse_error(input).kind {
        JsoncErrorKind::Lex(error) => error,
        kind => panic!("expected a lexical error, got {kind:?}"),
    }
}

#[test]
fn line_comment_before_the_value() -> Result<(), JsoncError> {
    let document = parse("// note\n1")?;
    let leading = document.root().first_token().leading();

    assert_eq!(leading.len(), 2);
    assert_eq!(
        leading.first().map(Trivia::kind),
        Some(TriviaKind::LineComment)
    );
    assert_eq!(
        leading.get(1).map(Trivia::kind),
        Some(TriviaKind::Whitespace)
    );

    Ok(())
}

#[test]
fn line_comment_after_the_value_is_trailing_trivia() -> Result<(), JsoncError> {
    let document = parse("1 // note")?;
    let kinds: Vec<TriviaKind> = document
        .trailing_trivia()
        .iter()
        .map(Trivia::kind)
        .collect();

    assert_eq!(kinds, vec![TriviaKind::Whitespace, TriviaKind::LineComment]);

    Ok(())
}

#[test]
fn block_comment_between_tokens() -> Result<(), JsoncError> {
    let document = parse("[1, /* two */ 2]")?;
    let Value::Array(array) = document.root() else {
        panic!("expected an array");
    };
    let Some(second) = array.elements().get(1) else {
        panic!("expected two elements");
    };
    let Some(comment) = second.value().first_token().leading_comments().next() else {
        panic!("expected a comment before the second element");
    };

    assert_eq!(comment.kind(), TriviaKind::BlockComment);
    assert_eq!(document.trivia_bytes(comment), Some(&b"/* two */"[..]));

    Ok(())
}

#[test]
fn multiline_block_comment_parses() -> Result<(), JsoncError> {
    let document = parse("/*\n * banner\n */\nnull")?;
    let Some(comment) = document.root().first_token().leading_comments().next() else {
        panic!("expected a leading comment");
    };

    assert_eq!(comment.kind(), TriviaKind::BlockComment);
    assert_eq!(comment.line_breaks(), 2);

    Ok(())
}

#[test]
fn comments_fit_every_whitespace_position() -> Result<(), JsoncError> {
    // A comment anywhere insignificant whitespace is allowed: around the document, around
    // structural tokens, between member name, colon, value, and comma.
    let input = "/*a*/ { /*b*/ \"k\" /*c*/ : /*d*/ [ /*e*/ 1 /*f*/ , /*g*/ 2 /*h*/ ] /*i*/ , \
                 /*j*/ \"l\" : null /*k*/ } /*l*/";
    let document = parse(input)?;

    let mut comments = 0usize;
    document.root().for_each_token(&mut |token| {
        comments = comments.saturating_add(token.leading_comments().count());
    });
    comments = comments.saturating_add(
        document
            .trailing_trivia()
            .iter()
            .filter(|trivia| trivia.kind().is_comment())
            .count(),
    );

    assert_eq!(comments, 12);

    Ok(())
}

#[test]
fn empty_block_comment_parses() -> Result<(), JsoncError> {
    let document = parse("/**/1/**/")?;
    let leading = document.root().first_token().leading();

    assert_eq!(
        leading.first().map(Trivia::kind),
        Some(TriviaKind::BlockComment)
    );
    assert_eq!(
        document.trailing_trivia().first().map(Trivia::kind),
        Some(TriviaKind::BlockComment)
    );

    Ok(())
}

#[test]
fn block_comments_do_not_nest() -> Result<(), JsoncError> {
    // "The closing of the nested comment will be interpreted as the end of the outer comment."
    // The comment ends at the first `*/`; the second `*/` is then plain (unrecognized) input.
    let document = parse("/* /* */ 1")?;
    let Some(comment) = document.root().first_token().leading_comments().next() else {
        panic!("expected a leading comment");
    };
    assert_eq!(document.trivia_bytes(comment), Some(&b"/* /* */"[..]));

    let error = lex_error("/* /* */ */ 1");
    assert!(matches!(error.kind, LexErrorKind::Unrecognized));
    assert_eq!(error.position.byte(), 9);

    Ok(())
}

#[test]
fn unterminated_block_comment_is_an_error_at_the_opener() {
    let error = lex_error("[1] /* open");
    assert!(matches!(error.kind, LexErrorKind::UnterminatedDelimiter));
    assert_eq!(error.position.byte(), 4);
    assert_eq!(error.position.line(), 1);
    assert_eq!(error.position.column(), 5);
}

#[test]
fn hash_comments_are_rejected() {
    // "Single-line comments that start with `#` are not supported." A `#` is simply not a token.
    let error = lex_error("# note\n1");
    assert!(matches!(error.kind, LexErrorKind::Unrecognized));
    assert_eq!(error.position.byte(), 0);

    let error = lex_error("1 # note");
    assert!(matches!(error.kind, LexErrorKind::Unrecognized));
    assert_eq!(error.position.byte(), 2);
}

#[test]
fn comment_lookalikes_inside_strings_are_content() -> Result<(), JsoncError> {
    let document = parse(r#""// not a comment /* either */""#)?;
    let Value::String(token) = document.root() else {
        panic!("expected a string");
    };
    assert_eq!(
        document.token_bytes(token),
        Some(&br#""// not a comment /* either */""#[..])
    );

    Ok(())
}

#[test]
fn comments_do_not_join_tokens() {
    // A comment is not a value separator: `[1 /*x*/ 2]` is still missing its comma.
    let error = parse_error("[1 /*x*/ 2]");
    assert!(matches!(
        error.kind,
        JsoncErrorKind::Expected {
            expected: "',' or ']'",
            ..
        }
    ));
}

#[test]
fn crlf_terminates_a_line_comment() -> Result<(), JsoncError> {
    let document = parse("// note\r\n\"v\"")?;
    let Some(comment) = document.root().first_token().leading_comments().next() else {
        panic!("expected a leading comment");
    };

    assert_eq!(document.trivia_bytes(comment), Some(&b"// note"[..]));

    Ok(())
}
