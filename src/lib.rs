//! JSONC (jsonc.org "JSON with Comments") parser for the Strata parser ecosystem.
//!
//! `strata-jsonc-parser` is a strict [jsonc.org](https://jsonc.org) parser composed on
//! `strata-json-parser`: JSONC extends RFC 8259 only with `//` line comments and non-nesting
//! `/* */` block comments, allowed anywhere insignificant whitespace is allowed. Comments are
//! trivia — retained in the CST with spans, never affecting the parsed data — and everything
//! else stays exactly as strict as `strata-json-parser`: no `#` comments, no nested block
//! comments, no `NaN`, no single quotes, full string/number shape validation.
//!
//! The crate does not mirror the JSON parser; it instantiates it. The token rules come from
//! `strata-jsonc-rules`, the tree is `strata-json-parser`'s generic CST
//! ([`strata_json_parser::cst`]) with `K = `[`JsonToken`] — JSONC adds no significant token
//! kinds — and `T = `[`TriviaKind`], which adds [`TriviaKind::LineComment`] and
//! [`TriviaKind::BlockComment`], and the grammar is the shared driver
//! ([`strata_json_parser::grammar`]) behind a private JSONC
//! [`Syntax`](strata_json_parser::grammar::Syntax) implementation. What remains here is the
//! public front: [`JsoncParser`], strict by default via [`JsoncParser::new`], with a builder for
//! the spec-permitted options — [`allow_trailing_commas`](JsoncParserBuilder::allow_trailing_commas)
//! (jsonc.org Appendix A: parsers "MAY support trailing commas"), plus
//! [`allow_bom`](JsoncParserBuilder::allow_bom) and [`max_depth`](JsoncParserBuilder::max_depth)
//! shared with `strata-json-parser` — and the JSONC error type.
//!
//! String and number content stays raw in the tree; decoding is the resolver layer's job, served
//! by the re-exported [`decode`] module.
//!
//! # Example
//!
//! ```
//! use strata_jsonc_parser::{JsoncParser, TriviaKind, Value};
//!
//! # fn main() -> Result<(), strata_jsonc_parser::JsoncError> {
//! let document = JsoncParser::new().parse_str(
//!     "{\n  // the crate name\n  \"name\": \"strata\"\n}",
//! )?;
//!
//! let Value::Object(object) = document.root() else {
//!     panic!("expected an object");
//! };
//! let member = object.members().first().unwrap();
//!
//! // The comment is retained as trivia on the token it precedes.
//! let comment = member.name().leading_comments().next().unwrap();
//! assert_eq!(comment.kind(), TriviaKind::LineComment);
//! assert_eq!(
//!     document.trivia_bytes(comment),
//!     Some(&b"// the crate name"[..])
//! );
//! # Ok(())
//! # }
//! ```

mod parse;

pub use parse::{
    ArrayNode, DEFAULT_MAX_DEPTH, Document, Element, JsoncError, JsoncErrorKind, JsoncParser,
    JsoncParserBuilder, Member, ObjectNode, SyntaxToken, Trivia, TriviaKind, Value,
};

// The resolver-facing decode layer (string unescaping, number conversion) is shared with the
// JSON parser: JSONC strings and numbers are exactly RFC 8259 strings and numbers.
pub use strata_json_parser::decode;

// Re-exported so downstream code can name the kinds carried inside the CST and share or extend
// rule sets without depending on `strata-jsonc-rules` (or `strata-json-rules`) directly.
// `SharedRuleSet` is kind-generic upstream, so it wraps JSONC rule sets too.
pub use strata_jsonc_rules::{
    BlockCommentRule, JsonToken, JsoncToken, LineCommentRule, SharedRuleSet, StrictRuleSet,
    strict_rule_set, strict_scanner,
};
