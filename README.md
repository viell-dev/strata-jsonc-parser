# strata-jsonc-parser

JSONC ([jsonc.org](https://jsonc.org) "JSON with Comments") parser for the Strata parser
ecosystem, producing a trivia-retaining, zero-copy CST by reusing `strata-json-parser`'s CST and
grammar parts.

## Status

This crate is in initial development. The public API is not settled yet.

## Usage

```rust
use strata_jsonc_parser::{JsoncError, JsoncParser, TriviaKind, Value};

fn main() -> Result<(), JsoncError> {
    let document = JsoncParser::new()
        .parse_str("{\n  // editor width\n  \"columns\": 100\n}")?;

    let Value::Object(object) = document.root() else {
        panic!("expected an object root");
    };
    let [member] = object.members() else {
        panic!("expected one member");
    };

    // Nothing is decoded: the CST keeps raw spans into the retained source.
    assert_eq!(
        document.token_bytes(member.name()),
        Some(&b"\"columns\""[..])
    );

    // The comment is retained as trivia on the token it precedes.
    let Some(comment) = member.name().leading_comments().next() else {
        panic!("expected the leading comment");
    };
    assert_eq!(comment.kind(), TriviaKind::LineComment);
    assert_eq!(comment.span().start(), 4);
    assert_eq!(
        document.trivia_bytes(comment),
        Some(&b"// editor width"[..])
    );

    Ok(())
}
```

Trailing commas are a spec-permitted opt-in
(`JsoncParser::builder().allow_trailing_commas(true)`); everything else is as strict as JSON.
The crate is `strata-json-parser`'s generic CST and grammar instantiated for JSONC, and resolver
layers decode string and number lexemes through the re-exported `decode` module. See
[docs/strata-jsonc-parser.md](docs/strata-jsonc-parser.md) for the design reference.

## Minimum Supported Rust Version

The MSRV is **1.87.0**.

## License

Licensed under the Mozilla Public License, Version 2.0 (see [LICENSE](LICENSE)
or <https://www.mozilla.org/en-US/MPL/2.0/>).

MPL-2.0 is a file-level weak copyleft license: its obligations attach to this crate's files (in
source and compiled form) and to modifications of them, not to your own source files that merely
use this crate. Distributing a binary that includes this crate obliges you to tell recipients
that the crate is included and how to obtain its source; for an unmodified crate, pointing them
at the exact crates.io release is a reasonable way to do that. Example material (documentation
code examples, `examples/`, integration tests, and benchmarks) is additionally licensed under
[MIT-0](LICENSE-MIT-0), so it can be copied freely. See
[LICENSE-CLARIFICATION.md](LICENSE-CLARIFICATION.md) for the maintainers' full statement of
intent regarding normal Rust usage (linking, monomorphization, macro expansion), binary
distribution, and generated code, and [CONTRIBUTING.md](CONTRIBUTING.md) for the contribution
policy.
