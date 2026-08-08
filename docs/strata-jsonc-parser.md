# strata-jsonc-parser

`strata-jsonc-parser` is the strict [jsonc.org](https://jsonc.org) "JSON with Comments" parser
for the Strata parser ecosystem. JSONC extends RFC 8259 only with `//` line comments and
non-nesting `/* */` block comments, allowed anywhere insignificant whitespace is allowed;
comments are trivia — retained in the CST with spans, never affecting the parsed data — and
everything else stays exactly as strict as `strata-json-parser`: no `#` comments, no `NaN`, no
single quotes, full string/number shape validation. String and number content stays raw in the
tree; decoding runs at the resolver layer, through the re-exported `decode` module.

The crate sits at the parser stage of the per-language stack and is the consumer side of
`strata-json-parser`'s generic layer: it does not mirror or wrap the JSON parser, it
instantiates it —

```text
strata-jsonc-rules -> strata-jsonc-parser (this crate) -> strata-jsonc (resolver)
                          ^ a Syntax impl + type aliases over
                            strata-json-parser's generic cst + grammar
```

- **Edition / MSRV:** Rust 2024, MSRV `1.87.0`.
- **License:** MPL-2.0.
- **Dependencies:** `strata-json-parser`, `strata-jsonc-rules`, `strata-lexer`.

## 1. Module and public-API map

| Module  | Public items |
|---------|--------------|
| `parse` | `JsoncParser<M>`, `JsoncParserBuilder<M>`, `JsoncError`, `JsoncErrorKind`, `DEFAULT_MAX_DEPTH`, `TriviaKind`, CST aliases (`Document`, `Value`, `ArrayNode`, `Element`, `ObjectNode`, `Member`, `SyntaxToken`, `Trivia`) |
| root    | everything above, plus `strata_json_parser::decode` and the `strata-jsonc-rules` surface re-exported |

## 2. Instantiating the generic layer

`strata-json-parser` §2–3 parameterizes its CST over `(K, T)` — significant token kind and
trivia kind — and captures all grammar variance in the `Syntax` trait. This crate supplies the
three JSONC answers and nothing else:

- **`K = JsonToken`.** JSONC adds no significant token kinds over RFC 8259, so the aliases
  instantiate the generic tree with the plain JSON kind: `SyntaxToken` is
  `cst::SyntaxToken<JsonToken, TriviaKind>`, and likewise for `Value`, `ArrayNode`, `Element`,
  `ObjectNode`, `Member`, `Trivia`, and `Document`. That "JSONC's data language is exactly JSON"
  is visible in the types: a token in a structural slot is a `JsonToken`, full stop.
- **`T = TriviaKind`.** The crate's own trivia enum adds `LineComment` and `BlockComment` to
  `Whitespace` and `Bom`, plus `is_comment()`. It implements the upstream `TriviaClass` hook, so
  the kind-aware conveniences of the generic tree (`Document::bom()`,
  `SyntaxToken::leading_comments()`, `Document::trivia_bytes()`) work unchanged.
- **A private `JsoncSyntax` implementing `Syntax`** over lex kind `JsoncToken`. Its `classify`
  is the whole superset: both comment kinds classify as trivia — the point where jsonc.org's
  "Removing all comments MUST yield the same data representation of the JSON values" is enforced
  structurally, since a comment can never reach the grammar — the BOM policy lives there too
  (trivia under `allow_bom`, `UnexpectedBom` otherwise), and every other token unwraps to its
  `JsonToken` kind. `value_role`, `punct`, and `is_member_name` are identical to JSON's;
  `allow_trailing_commas` gates the driver's tolerance; the error-constructor methods build
  `JsoncError`, so diagnostics keep a JSONC-precise enum.

Everything else is inherited from `parse_document`: trivia folding (each piece attaches to the
next significant token, or to the document end), the nesting-depth limit, the whole-input
requirement, source parking into the document's `SourceStore`, and byte-for-byte reconstruction
(`tests/fidelity.rs` proves it, comments, BOM, and tolerated trailing commas included).

## 3. The JSONC front (`parse`)

`JsoncParser::new()` / `default()` is the fully strict path: comments accepted and retained
(they are core JSONC, not an option), trailing commas rejected, no BOM tolerance,
`DEFAULT_MAX_DEPTH` (= `strata_json_parser::DEFAULT_MAX_DEPTH`, 128) nesting. `parse(impl
Read)`, `parse_str`, and `parse_bytes` each run one document to EOF; trailing content other than
whitespace and comments is an error. The parser clones its rule set into a fresh scanner per
parse, so one parser serves many inputs.

`JsoncParser::builder()` exposes, per the ecosystem's builders-per-layer shape:

- `allow_trailing_commas(bool)` — off by default. The spec's single optionality clause: the
  jsonc.org grammar notes "trailing commas are NOT supported", Appendix A grants "JSONC parsers
  MAY support trailing commas", and the reference `jsonc-parser` leaves `allowTrailingComma` off
  unless configured. When on, a comma directly before `]`/`}` is tolerated and retained on the
  last `Element`/`Member` (fidelity); a lone comma (`[,]`, `{,}`) or repeated commas (`[1,,2]`)
  remain errors either way.
- `allow_bom(bool)` — off by default, identical to `strata-json-parser`'s option (RFC 8259
  §8.1 permission). Retained as `TriviaKind::Bom` when on.
- `max_depth(usize)` — RFC 8259 §9 implementation limit, also bounds recursion. Comments do not
  consume depth.
- `with_rules(set)` — any `MatcherSet<Kind = JsoncToken> + Clone`; wrap non-`Clone` sets in
  `SharedRuleSet`. A superset *language* instead implements its own `Syntax` over its own kinds.
- `with_lexer_config(config)` — reader capacities, position tracking, ...; `compact_after` is
  always overridden to `None` at parse time.

Errors are `JsoncError { position, kind }` with one-based line/column plus absolute byte.
`JsoncErrorKind`: `Lex(LexError)` (including unterminated block comments at the `/*` and `#` as
an unrecognized byte), `Expected`, `UnexpectedEof`, `TrailingContent`, `UnexpectedBom`,
`DepthLimitExceeded`. `found` kinds are plain `JsonToken`s — comments never reach the grammar,
so every significant token the grammar can trip over is a JSON kind.

## 4. Decoding (re-exported `decode`)

JSONC strings and numbers are exactly RFC 8259 strings and numbers, so the resolver-facing
decode layer is `strata-json-parser`'s `decode` module re-exported: `unescape_string` (§7
escapes, UTF-16 surrogate-pair combination, the `LoneSurrogates` policy) and `parse_number`
(integer-preserving `Number` classification). The re-export is what lets the JSONC resolver
reuse JSON decoding without resolvers depending on each other.

## 5. Design choices

- **Comments as trivia, enforced structurally.** Classification is the one place comments
  exist; the grammar driver and the tree types never see them, which *is* the spec's
  "comments are ignored during parsing" requirement.
- **A `Syntax` front, not a mirrored tree.** The first-generation POC mirrored the JSON
  parser's CST and driver (~400 duplicated lines); this crate is the instantiation that design
  was rebuilt for — one trait impl plus type aliases, with the CST, driver, trivia folding, and
  fidelity guarantees inherited rather than copied.
- **`JsonToken` in the tree.** Keeping upstream's significant-token enum makes JSONC documents
  directly legible to JSON-level tooling and avoids a parallel kind that could drift.
- **Options strict by default.** Both spec latitudes (`allow_trailing_commas`, `allow_bom`) are
  opt-in, matching the ecosystem posture and the reference implementation's defaults.

## 6. Quality bar

Same as the ecosystem: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D
warnings` under the strict lint set in `Cargo.toml`, and `cargo test` all clean. Lint exceptions
use `#[expect(lint, reason = "…")]` on the smallest existing scope.

## 7. Limitations and follow-up

- Single-buffer retention: the parser parks one consumed buffer after EOF; streaming/chunked DOM
  construction awaits the lexer's batch protocol.
- Serde glue (parser → structs directly) is a separate planned crate, not part of this one.
- The public API is not settled; this is an initial-development crate on an experimental branch.
- Package verification (`cargo package`) is blocked until the path dependencies are published.
