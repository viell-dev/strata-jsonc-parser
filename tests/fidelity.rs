//! Source fidelity: comment trivia retention, span correctness, and byte-for-byte
//! reconstruction of the input from the CST.

use strata_jsonc_parser::{Document, JsoncError, JsoncParser, TriviaKind};
use strata_lexer::Span;

fn parse(input: &str) -> Result<Document, JsoncError> {
    JsoncParser::new().parse_str(input)
}

/// Collects every retained span — leading trivia and lexemes, in source order — for the whole
/// document.
fn all_spans(document: &Document) -> Vec<Span> {
    let mut spans = Vec::new();

    document.root().for_each_token(&mut |token| {
        for trivia in token.leading() {
            spans.push(trivia.span());
        }

        spans.push(token.span());
    });

    for trivia in document.trailing_trivia() {
        spans.push(trivia.span());
    }

    spans
}

/// Rebuilds the source text from the retained spans.
fn reconstruct(document: &Document) -> Option<Vec<u8>> {
    let mut out = Vec::new();

    for span in all_spans(document) {
        out.extend_from_slice(document.bytes(span)?);
    }

    Some(out)
}

fn assert_reconstructs(input: &str, document: &Document) {
    assert_eq!(
        reconstruct(document).as_deref(),
        Some(input.as_bytes()),
        "reconstruction mismatch for {input:?}"
    );

    // The retained spans are contiguous and gap-free from byte 0 to the end of input.
    let mut cursor = 0;
    for span in all_spans(document) {
        assert_eq!(span.start(), cursor, "gap or overlap at byte {cursor}");
        cursor = span.end();
    }
    assert_eq!(cursor, u64::try_from(input.len()).unwrap_or(u64::MAX));
}

#[test]
fn comment_heavy_document_reconstructs_byte_for_byte() -> Result<(), JsoncError> {
    let input = "// VSCode-style settings\n{\n  // section one\n\n  \"editor.fontSize\": 14, \
                 /* inline */ \"editor.wordWrap\": \"on\",\n\n  /*\n   * section two\n   */\n  \
                 \"files.exclude\": [\"target\", \"node_modules\"]\n} // done\n";
    let document = parse(input)?;
    assert_reconstructs(input, &document);

    Ok(())
}

#[test]
fn trailing_comma_document_reconstructs_byte_for_byte() -> Result<(), JsoncError> {
    let input = "{\n  \"a\": [1, 2,], // list\n}";
    let document = JsoncParser::builder()
        .allow_trailing_commas(true)
        .build()
        .parse_str(input)?;
    assert_reconstructs(input, &document);

    Ok(())
}

#[test]
fn bom_document_reconstructs_byte_for_byte() -> Result<(), JsoncError> {
    let input = "\u{FEFF}// note\nnull";
    let document = JsoncParser::builder()
        .allow_bom(true)
        .build()
        .parse_str(input)?;
    assert_reconstructs(input, &document);

    Ok(())
}

#[test]
fn comment_spans_and_positions_are_exact() -> Result<(), JsoncError> {
    let input = "/* one */\n// two\n1";
    let document = parse(input)?;
    let leading = document.root().first_token().leading();

    let kinds: Vec<(TriviaKind, u64, u64)> = leading
        .iter()
        .map(|trivia| (trivia.kind(), trivia.span().start(), trivia.span().end()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (TriviaKind::BlockComment, 0, 9),
            (TriviaKind::Whitespace, 9, 10),
            (TriviaKind::LineComment, 10, 16),
            (TriviaKind::Whitespace, 16, 17),
        ]
    );

    let Some(line_comment) = leading.get(2) else {
        panic!("expected four trivia pieces");
    };
    assert_eq!(line_comment.start().line(), 2);
    assert_eq!(line_comment.start().column(), 1);
    assert_eq!(line_comment.end().line(), 2);
    assert_eq!(line_comment.end().column(), 7);

    Ok(())
}

#[test]
fn blank_line_grouping_survives_around_comments() -> Result<(), JsoncError> {
    // The whitespace run between the value and the comment carries the blank line — the human
    // grouping signal the ecosystem preserves.
    let document = parse("{\"a\": 1,\n\n// group two\n\"b\": 2}")?;
    let strata_jsonc_parser::Value::Object(object) = document.root() else {
        panic!("expected an object");
    };
    let Some(second) = object.members().get(1) else {
        panic!("expected two members");
    };

    let Some(whitespace) = second.name().leading().first() else {
        panic!("expected whitespace before the comment");
    };
    assert_eq!(whitespace.kind(), TriviaKind::Whitespace);
    assert_eq!(whitespace.blank_lines(), 1);
    assert_eq!(second.name().leading_comments().count(), 1);

    Ok(())
}

#[test]
fn comment_bytes_are_zero_copy_addressable() -> Result<(), JsoncError> {
    let document = parse("[/* pi, roughly */ 3.14]")?;
    let strata_jsonc_parser::Value::Array(array) = document.root() else {
        panic!("expected an array");
    };
    let Some(element) = array.elements().first() else {
        panic!("expected one element");
    };
    let Some(comment) = element.value().first_token().leading_comments().next() else {
        panic!("expected a comment");
    };

    assert_eq!(
        document.trivia_bytes(comment),
        Some(&b"/* pi, roughly */"[..])
    );

    Ok(())
}
