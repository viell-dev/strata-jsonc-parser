//! The strict jsonc.org parser front: token rules in, CST out.
//!
//! [`JsoncParser::new`] is the fully strict path: comments are accepted (they are core JSONC,
//! not an option) and retained as trivia; everything else is exactly RFC 8259 as
//! `strata-json-parser` enforces it. [`JsoncParser::builder`] exposes the spec-permitted options
//! — the trailing-comma tolerance jsonc.org grants ("JSONC parsers MAY support trailing
//! commas"), plus the two RFC 8259 latitudes shared with `strata-json-parser` (`allow_bom`,
//! `max_depth`) — and `with_*` hooks to customize or replace the lower layers: the token rules
//! and the lexer configuration.
//!
//! The grammar itself is `strata-json-parser`'s shared driver
//! ([`strata_json_parser::grammar`]); this module implements the JSONC
//! [`Syntax`] over it — comments become trivia at classification time, so a comment can never
//! reach the grammar — and owns the JSONC-specific error type and options.

use std::error::Error;
use std::fmt;
use std::io::{Cursor, Read};
use strata_json_parser::cst::{self, TriviaClass};
use strata_json_parser::grammar::{self, Lexed, Punct, Syntax, ValueRole};
use strata_jsonc_rules::{JsonToken, JsoncToken, StrictRuleSet, strict_rule_set};
use strata_lexer::{LexError, LexerConfig, MatcherSet, Position, Token};

/// Classification of a retained trivia piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriviaKind {
    /// A run of insignificant whitespace (RFC 8259 §2).
    Whitespace,
    /// A tolerated leading byte-order mark (RFC 8259 §8.1, `allow_bom`).
    Bom,
    /// A `//` line comment, excluding the terminating line break (jsonc.org).
    LineComment,
    /// A `/* */` block comment, including both delimiters (jsonc.org).
    BlockComment,
}

impl TriviaKind {
    /// Returns whether this piece is a comment (line or block).
    #[must_use]
    pub const fn is_comment(self) -> bool {
        matches!(self, Self::LineComment | Self::BlockComment)
    }
}

impl TriviaClass for TriviaKind {
    #[inline]
    fn is_comment(self) -> bool {
        Self::is_comment(self)
    }

    #[inline]
    fn is_bom(self) -> bool {
        matches!(self, Self::Bom)
    }
}

/// A retained piece of grammatically insignificant JSONC source content.
pub type Trivia = cst::Trivia<TriviaKind>;
/// A significant JSONC token in the CST.
///
/// The kind is a plain [`JsonToken`]: JSONC adds no significant token kinds over RFC 8259, only
/// trivia — comments never reach the tree as tokens, they arrive in
/// [`SyntaxToken::leading`](cst::SyntaxToken::leading).
pub type SyntaxToken = cst::SyntaxToken<JsonToken, TriviaKind>;
/// A JSONC value node.
pub type Value = cst::Value<JsonToken, TriviaKind>;
/// A JSONC array node.
pub type ArrayNode = cst::ArrayNode<JsonToken, TriviaKind>;
/// One JSONC array element.
pub type Element = cst::Element<JsonToken, TriviaKind>;
/// A JSONC object node.
pub type ObjectNode = cst::ObjectNode<JsonToken, TriviaKind>;
/// One JSONC object member.
pub type Member = cst::Member<JsonToken, TriviaKind>;
/// A parsed JSONC document.
pub type Document = cst::Document<JsonToken, TriviaKind>;

/// The default nesting-depth limit, identical to `strata-json-parser`'s.
///
/// RFC 8259 §9 permits parsers to set limits on the depth of nesting; this is the maximum number
/// of nested arrays/objects. Override it with [`JsoncParserBuilder::max_depth`]. The limit also
/// bounds the parser's recursion, so raising it substantially trades stack headroom for depth.
pub const DEFAULT_MAX_DEPTH: usize = strata_json_parser::DEFAULT_MAX_DEPTH;

/// A parse failure, carrying the position at which it was observed.
#[derive(Debug)]
pub struct JsoncError {
    /// Position of the offending byte or token start.
    pub position: Position,
    /// What went wrong.
    pub kind: JsoncErrorKind,
}

/// Categories of parse failures.
///
/// `found` kinds are plain [`JsonToken`]s: comments are trivia and never reach the grammar, so
/// every significant token the grammar can trip over is a JSON kind.
#[derive(Debug)]
pub enum JsoncErrorKind {
    /// The lexer failed: I/O, an unrecognized byte (e.g. a `#` — not a JSONC comment), a
    /// token-shape violation reported at the offending byte, or an unterminated block comment
    /// reported at its opening `/*`.
    Lex(LexError),
    /// A token of an unexpected kind where the grammar required something else.
    Expected {
        /// What the grammar required here.
        expected: &'static str,
        /// The token kind actually found.
        found: JsonToken,
    },
    /// The input ended where the grammar required more.
    UnexpectedEof {
        /// What the grammar required here.
        expected: &'static str,
    },
    /// A significant token followed the complete top-level value.
    TrailingContent {
        /// The token kind found after the value.
        found: JsonToken,
    },
    /// The input starts with a byte-order mark and the parser was not configured to tolerate it.
    ///
    /// RFC 8259 §8.1 forbids adding a BOM but permits implementations to ignore one; enable
    /// [`JsoncParserBuilder::allow_bom`] to opt into that permission.
    UnexpectedBom,
    /// Nesting exceeded the configured depth limit (RFC 8259 §9 implementation limit).
    DepthLimitExceeded {
        /// The configured limit.
        limit: usize,
    },
}

impl fmt::Display for JsoncErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(error) => write!(formatter, "{error}"),
            Self::Expected { expected, found } => {
                write!(formatter, "expected {expected}, found {}", found.describe())
            }
            Self::UnexpectedEof { expected } => {
                write!(formatter, "unexpected end of input, expected {expected}")
            }
            Self::TrailingContent { found } => write!(
                formatter,
                "unexpected {} after the top-level value",
                found.describe()
            ),
            Self::UnexpectedBom => formatter.write_str(
                "leading byte-order mark (enable allow_bom to tolerate it per RFC 8259 \u{a7}8.1)",
            ),
            Self::DepthLimitExceeded { limit } => {
                write!(formatter, "nesting depth limit of {limit} exceeded")
            }
        }
    }
}

impl fmt::Display for JsoncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            // The lexer's message already carries its byte position.
            JsoncErrorKind::Lex(error) => write!(formatter, "{error}"),
            kind => write!(
                formatter,
                "{kind} at line {}, column {} (byte {})",
                self.position.line(),
                self.position.column(),
                self.position.byte()
            ),
        }
    }
}

impl Error for JsoncError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            JsoncErrorKind::Lex(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LexError> for JsoncError {
    fn from(error: LexError) -> Self {
        Self {
            position: error.position,
            kind: JsoncErrorKind::Lex(error),
        }
    }
}

/// The JSONC [`Syntax`]: one parse's classification policy over [`JsoncToken`].
///
/// This is the whole superset: comments classify as trivia — the point where jsonc.org's
/// "Removing all comments MUST yield the same data representation of the JSON values" is
/// enforced structurally — and every significant token unwraps to its [`JsonToken`] kind, so the
/// shared grammar driver and generic CST are reused unchanged.
#[derive(Debug, Clone, Copy)]
struct JsoncSyntax {
    allow_bom: bool,
    allow_trailing_commas: bool,
}

impl Syntax for JsoncSyntax {
    type Lex = JsoncToken;
    type Kind = JsonToken;
    type Trivia = TriviaKind;
    type Error = JsoncError;

    fn eof_lex(&self) -> JsoncToken {
        JsoncToken::Eof
    }

    fn classify(
        &self,
        token: &Token<JsoncToken>,
    ) -> Result<Lexed<JsonToken, TriviaKind>, JsoncError> {
        match token.kind {
            JsoncToken::LineComment => Ok(Lexed::Trivia(TriviaKind::LineComment)),
            JsoncToken::BlockComment => Ok(Lexed::Trivia(TriviaKind::BlockComment)),
            JsoncToken::Json(JsonToken::Whitespace) => Ok(Lexed::Trivia(TriviaKind::Whitespace)),
            JsoncToken::Json(JsonToken::Bom) => {
                if self.allow_bom {
                    Ok(Lexed::Trivia(TriviaKind::Bom))
                } else {
                    Err(JsoncError {
                        position: token.start(),
                        kind: JsoncErrorKind::UnexpectedBom,
                    })
                }
            }
            JsoncToken::Json(kind) => Ok(Lexed::Significant(kind)),
            JsoncToken::Eof => Ok(Lexed::Eof),
        }
    }

    fn value_role(&self, kind: JsonToken) -> Option<ValueRole> {
        match kind {
            JsonToken::LeftBrace => Some(ValueRole::ObjectOpen),
            JsonToken::LeftBracket => Some(ValueRole::ArrayOpen),
            JsonToken::String => Some(ValueRole::String),
            JsonToken::Number => Some(ValueRole::Number),
            JsonToken::True => Some(ValueRole::True),
            JsonToken::False => Some(ValueRole::False),
            JsonToken::Null => Some(ValueRole::Null),
            _ => None,
        }
    }

    fn punct(&self, kind: JsonToken) -> Option<Punct> {
        match kind {
            JsonToken::Colon => Some(Punct::Colon),
            JsonToken::Comma => Some(Punct::Comma),
            JsonToken::RightBrace => Some(Punct::ObjectClose),
            JsonToken::RightBracket => Some(Punct::ArrayClose),
            _ => None,
        }
    }

    fn is_member_name(&self, kind: JsonToken) -> bool {
        kind == JsonToken::String
    }

    fn value_expectation(&self) -> &'static str {
        "a JSON value"
    }

    fn member_name_expectation(&self) -> &'static str {
        "an object member name (a string)"
    }

    fn allow_trailing_commas(&self) -> bool {
        self.allow_trailing_commas
    }

    fn expected(&self, position: Position, expected: &'static str, found: JsonToken) -> JsoncError {
        JsoncError {
            position,
            kind: JsoncErrorKind::Expected { expected, found },
        }
    }

    fn unexpected_eof(&self, position: Position, expected: &'static str) -> JsoncError {
        JsoncError {
            position,
            kind: JsoncErrorKind::UnexpectedEof { expected },
        }
    }

    fn trailing_content(&self, position: Position, found: JsonToken) -> JsoncError {
        JsoncError {
            position,
            kind: JsoncErrorKind::TrailingContent { found },
        }
    }

    fn depth_exceeded(&self, position: Position, limit: usize) -> JsoncError {
        JsoncError {
            position,
            kind: JsoncErrorKind::DepthLimitExceeded { limit },
        }
    }
}

/// A reusable, strict jsonc.org parser.
///
/// `M` is the token rule set handed to the lexer for every parse; it defaults to the strict
/// jsonc.org rules. The parser clones the rules into a fresh scanner per [`JsoncParser::parse`]
/// call, so one configured parser can serve many inputs.
#[derive(Debug, Clone)]
pub struct JsoncParser<M = StrictRuleSet> {
    rules: M,
    allow_bom: bool,
    allow_trailing_commas: bool,
    max_depth: usize,
    lexer_config: LexerConfig,
}

impl JsoncParser<StrictRuleSet> {
    /// Creates a fully strict parser: comments accepted and retained (core JSONC), trailing
    /// commas rejected, no BOM tolerance, [`DEFAULT_MAX_DEPTH`] nesting.
    pub fn new() -> Self {
        Self {
            rules: strict_rule_set(),
            allow_bom: false,
            allow_trailing_commas: false,
            max_depth: DEFAULT_MAX_DEPTH,
            lexer_config: LexerConfig::new(),
        }
    }

    /// Starts a builder for customizing spec-permitted options or replacing lower layers.
    pub fn builder() -> JsoncParserBuilder<StrictRuleSet> {
        JsoncParserBuilder::new()
    }
}

impl Default for JsoncParser<StrictRuleSet> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> JsoncParser<M>
where
    M: MatcherSet<Kind = JsoncToken> + Clone,
{
    /// Parses a complete JSONC text from `reader` into a [`Document`].
    ///
    /// Any RFC 8259 value is a valid top-level document. The entire input must be consumed:
    /// trailing content other than whitespace and comments is an error. On success the consumed
    /// source bytes are retained in the document's source store, keeping the CST zero-copy.
    pub fn parse<R: Read>(&self, reader: R) -> Result<Document, JsoncError> {
        let syntax = JsoncSyntax {
            allow_bom: self.allow_bom,
            allow_trailing_commas: self.allow_trailing_commas,
        };

        grammar::parse_document(
            reader,
            self.rules.clone(),
            &syntax,
            self.max_depth,
            self.lexer_config,
        )
    }

    /// Parses a complete JSONC text from a string slice.
    pub fn parse_str(&self, input: &str) -> Result<Document, JsoncError> {
        self.parse(Cursor::new(input.as_bytes()))
    }

    /// Parses a complete JSONC text from a byte slice. The bytes must be UTF-8 (RFC 8259 §8.1);
    /// violations inside strings or comments are lexical errors, and any non-ASCII byte outside
    /// them matches no rule.
    pub fn parse_bytes(&self, input: &[u8]) -> Result<Document, JsoncError> {
        self.parse(Cursor::new(input))
    }

    /// The token rule set handed to the lexer at each parse.
    pub const fn rules(&self) -> &M {
        &self.rules
    }

    /// Whether a leading byte-order mark is tolerated (RFC 8259 §8.1).
    pub const fn allow_bom(&self) -> bool {
        self.allow_bom
    }

    /// Whether a trailing comma before `]`/`}` is tolerated (jsonc.org Appendix A).
    pub const fn allow_trailing_commas(&self) -> bool {
        self.allow_trailing_commas
    }

    /// The configured nesting-depth limit (RFC 8259 §9).
    pub const fn max_depth(&self) -> usize {
        self.max_depth
    }
}

/// Builder for [`JsoncParser`], strict by default.
#[derive(Debug, Clone)]
pub struct JsoncParserBuilder<M = StrictRuleSet> {
    rules: M,
    allow_bom: bool,
    allow_trailing_commas: bool,
    max_depth: usize,
    lexer_config: LexerConfig,
}

impl JsoncParserBuilder<StrictRuleSet> {
    /// Starts from the fully strict defaults.
    pub fn new() -> Self {
        Self {
            rules: strict_rule_set(),
            allow_bom: false,
            allow_trailing_commas: false,
            max_depth: DEFAULT_MAX_DEPTH,
            lexer_config: LexerConfig::new(),
        }
    }
}

impl Default for JsoncParserBuilder<StrictRuleSet> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> JsoncParserBuilder<M> {
    /// Replaces the token rule set handed to the lexer.
    ///
    /// This is the rules-layer composition point: tests can mock the rules, and adjusted rule
    /// sets can be swapped in without touching the grammar driver. The set must still produce
    /// [`JsoncToken`] kinds; a superset *language* instead implements its own
    /// [`Syntax`](strata_json_parser::grammar::Syntax) over its own kinds, reusing the rule
    /// structs from `strata-jsonc-rules` via
    /// [`MatcherExt::map_kind`](strata_lexer::MatcherExt::map_kind).
    pub fn with_rules<N>(self, rules: N) -> JsoncParserBuilder<N>
    where
        N: MatcherSet<Kind = JsoncToken> + Clone,
    {
        JsoncParserBuilder {
            rules,
            allow_bom: self.allow_bom,
            allow_trailing_commas: self.allow_trailing_commas,
            max_depth: self.max_depth,
            lexer_config: self.lexer_config,
        }
    }

    /// Replaces the lexer configuration (reader capacities, position tracking, ...).
    ///
    /// The parser always disables buffer compaction (`compact_after = None`) at parse time: the
    /// consumed bytes must survive until they are parked into the document's source store.
    #[must_use]
    pub const fn with_lexer_config(mut self, config: LexerConfig) -> Self {
        self.lexer_config = config;
        self
    }

    /// Tolerates one leading byte-order mark, retaining it as trivia.
    ///
    /// Off by default; identical to `strata-json-parser`'s option. RFC 8259 §8.1: a JSON text
    /// must not begin with a BOM, but implementations "MAY ignore the presence of a byte order
    /// mark rather than treating it as an error". With the option off, a leading BOM is
    /// [`JsoncErrorKind::UnexpectedBom`]; a BOM anywhere else in the input is always an error.
    #[must_use]
    pub const fn allow_bom(mut self, allow: bool) -> Self {
        self.allow_bom = allow;
        self
    }

    /// Tolerates a trailing comma before `]` and `}`, retaining it in the CST.
    ///
    /// Off by default. jsonc.org grammar: "trailing commas are NOT supported"; Appendix A:
    /// "JSONC parsers MAY support trailing commas" — the reference `jsonc-parser` likewise
    /// leaves `allowTrailingComma` off unless configured. When enabled, the tolerated comma is
    /// kept on the last [`Element`]/[`Member`] for source fidelity. A comma with no preceding
    /// entry (`[,]`, `{,}`) or a repeated comma (`[1,,2]`) remains an error either way.
    #[must_use]
    pub const fn allow_trailing_commas(mut self, allow: bool) -> Self {
        self.allow_trailing_commas = allow;
        self
    }

    /// Sets the nesting-depth limit.
    ///
    /// Identical to `strata-json-parser`'s option. RFC 8259 §9: "An implementation may set
    /// limits on the maximum depth of nesting." The limit counts nested arrays/objects — with a
    /// limit of 1, `[1]` parses and `[[1]]` does not. It also bounds the parser's recursion
    /// depth; see [`DEFAULT_MAX_DEPTH`].
    #[must_use]
    pub const fn max_depth(mut self, limit: usize) -> Self {
        self.max_depth = limit;
        self
    }

    /// Finishes the builder.
    pub fn build(self) -> JsoncParser<M>
    where
        M: MatcherSet<Kind = JsoncToken> + Clone,
    {
        JsoncParser {
            rules: self.rules,
            allow_bom: self.allow_bom,
            allow_trailing_commas: self.allow_trailing_commas,
            max_depth: self.max_depth,
            lexer_config: self.lexer_config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_fully_strict() {
        let parser = JsoncParser::new();
        assert!(!parser.allow_bom());
        assert!(!parser.allow_trailing_commas());
        assert_eq!(parser.max_depth(), DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn builder_defaults_match_new() {
        let parser = JsoncParser::builder().build();
        assert!(!parser.allow_bom());
        assert!(!parser.allow_trailing_commas());
        assert_eq!(parser.max_depth(), DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn builder_sets_options() {
        let parser = JsoncParser::builder()
            .allow_bom(true)
            .allow_trailing_commas(true)
            .max_depth(4)
            .build();
        assert!(parser.allow_bom());
        assert!(parser.allow_trailing_commas());
        assert_eq!(parser.max_depth(), 4);
    }

    #[test]
    fn default_depth_matches_strata_json_parser() {
        assert_eq!(DEFAULT_MAX_DEPTH, strata_json_parser::DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn comment_kinds_classify_as_comments() {
        assert!(TriviaKind::LineComment.is_comment());
        assert!(TriviaKind::BlockComment.is_comment());
        assert!(!TriviaKind::Whitespace.is_comment());
        assert!(!TriviaKind::Bom.is_comment());

        assert!(TriviaClass::is_bom(TriviaKind::Bom));
        assert!(!TriviaClass::is_bom(TriviaKind::BlockComment));
    }

    #[test]
    fn error_display_includes_position() {
        let Err(error) = JsoncParser::new().parse_str("[1 2]") else {
            panic!("expected a parse error");
        };
        let message = error.to_string();
        assert!(message.contains("expected ',' or ']'"), "{message}");
        assert!(message.contains("line 1"), "{message}");
        assert!(message.contains("byte 3"), "{message}");
    }

    #[test]
    fn bom_error_display_mentions_the_option() {
        let Err(error) = JsoncParser::new().parse_str("\u{FEFF}1") else {
            panic!("expected a parse error");
        };
        assert!(matches!(error.kind, JsoncErrorKind::UnexpectedBom));
        assert!(error.to_string().contains("allow_bom"), "{error}");
    }

    #[test]
    fn comment_positions_survive_into_errors() {
        // The error after a multiline comment must account for the comment's line breaks.
        let Err(error) = JsoncParser::new().parse_str("/* a\nb */ [1 2]") else {
            panic!("expected a parse error");
        };
        assert_eq!(error.position.line(), 2);
    }
}
