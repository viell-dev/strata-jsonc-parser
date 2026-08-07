//! The single jsonc.org optionality clause: "JSONC parsers MAY support trailing commas"
//! (Appendix A). Off by default; when enabled, only a comma directly before `]`/`}` is
//! tolerated, and it is retained in the CST.

use strata_jsonc_parser::{Document, JsonToken, JsoncError, JsoncErrorKind, JsoncParser, Value};

fn strict(input: &str) -> Result<Document, JsoncError> {
    JsoncParser::new().parse_str(input)
}

fn tolerant(input: &str) -> Result<Document, JsoncError> {
    JsoncParser::builder()
        .allow_trailing_commas(true)
        .build()
        .parse_str(input)
}

fn error_of(result: Result<Document, JsoncError>) -> JsoncError {
    match result {
        Err(error) => error,
        Ok(document) => panic!("expected a parse error, got {:?}", document.root()),
    }
}

#[test]
fn trailing_commas_are_rejected_by_default() {
    let error = error_of(strict("[1,]"));
    assert!(matches!(
        error.kind,
        JsoncErrorKind::Expected {
            expected: "a JSON value",
            found: JsonToken::RightBracket,
        }
    ));

    let error = error_of(strict("{\"a\": 1,}"));
    assert!(matches!(
        error.kind,
        JsoncErrorKind::Expected {
            expected: "an object member name (a string)",
            found: JsonToken::RightBrace,
        }
    ));
}

#[test]
fn trailing_array_comma_is_tolerated_and_retained() -> Result<(), JsoncError> {
    let document = tolerant("[1, 2,]")?;
    let Value::Array(array) = document.root() else {
        panic!("expected an array");
    };

    assert_eq!(array.elements().len(), 2);
    let Some(last) = array.elements().last() else {
        panic!("expected two elements");
    };
    let Some(comma) = last.comma() else {
        panic!("expected the trailing comma to be retained");
    };
    assert_eq!(comma.kind, JsonToken::Comma);
    assert_eq!(document.token_bytes(comma), Some(&b","[..]));

    Ok(())
}

#[test]
fn trailing_object_comma_is_tolerated_and_retained() -> Result<(), JsoncError> {
    let document = tolerant("{\"a\": 1,}")?;
    let Value::Object(object) = document.root() else {
        panic!("expected an object");
    };

    assert_eq!(object.members().len(), 1);
    let Some(member) = object.members().last() else {
        panic!("expected one member");
    };
    assert!(member.comma().is_some(), "the trailing comma is retained");

    Ok(())
}

#[test]
fn comments_may_sit_between_the_trailing_comma_and_the_close() -> Result<(), JsoncError> {
    let document = tolerant("[1, // last\n]")?;
    let Value::Array(array) = document.root() else {
        panic!("expected an array");
    };

    assert!(
        array
            .elements()
            .last()
            .is_some_and(|last| last.comma().is_some())
    );
    assert_eq!(array.close().leading_comments().count(), 1);

    Ok(())
}

#[test]
fn lone_commas_stay_errors_even_when_tolerated() {
    for input in ["[,]", "{,}"] {
        let error = error_of(tolerant(input));
        assert!(
            matches!(error.kind, JsoncErrorKind::Expected { .. }),
            "input {input:?}"
        );
        assert_eq!(error.position.byte(), 1, "input {input:?}");
    }
}

#[test]
fn repeated_commas_stay_errors_even_when_tolerated() {
    for input in ["[1,,2]", "[1,,]", "{\"a\": 1,,}"] {
        let error = error_of(tolerant(input));
        assert!(
            matches!(error.kind, JsoncErrorKind::Expected { .. }),
            "input {input:?}"
        );
    }
}

#[test]
fn empty_containers_take_no_comma_either_way() -> Result<(), JsoncError> {
    // `[]`/`{}` stay fine; a comma in an empty container is not "trailing", it is lone.
    let document = tolerant("[]")?;
    let Value::Array(array) = document.root() else {
        panic!("expected an array");
    };
    assert!(array.elements().is_empty());

    let document = tolerant("{}")?;
    let Value::Object(object) = document.root() else {
        panic!("expected an object");
    };
    assert!(object.members().is_empty());

    Ok(())
}

#[test]
fn strict_behavior_is_unchanged_when_the_option_is_off() -> Result<(), JsoncError> {
    // Sanity: the option only widens; normal documents parse identically either way.
    let strict_doc = strict("[1, 2]")?;
    let tolerant_doc = tolerant("[1, 2]")?;
    assert_eq!(strict_doc.root(), tolerant_doc.root());

    Ok(())
}
