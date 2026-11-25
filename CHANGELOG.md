# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- `Parser::into_inner`, which returns the input stream (e.g. to close a file once it is parsed).

### Changed
- Published on crates.io as `printhor-async-gcode` 0.4.0 (`async-gcode` belongs to the upstream
  project). The library is still `async_gcode`: depend on it as
  `async-gcode = { package = "printhor-async-gcode", version = "0.4.0" }`.
- `Error::BadChecksum` reports the checksum received besides the computed one:
  `BadChecksum { computed, received }` (it was `BadChecksum(computed)`).
- Minor fix to the documentation.
- Faster parsing of numbers and words: the first byte of a value is passed on instead of being
  pushed back and read again at every nesting level (about 2x on a slicer G-code file).

### Fixed
- A number whose scaled value does not fit in the `i32` mantissa of `DecimalRepr` (for instance
  `X21474.83648` or `X99999.99999`) is now reported as `Error::NumberOverflow`. It used to wrap
  around silently, or panic with overflow checks enabled.
- A `\r\n` end of line is a single end of line: one `GCode::Execute` and one line in
  `get_current_line`. It used to be two (an empty line after each line). `\r` and `\n` alone still
  end a line, and `GCode::Execute` is still emitted as soon as the `\r` is received. With
  `parse-checksum`, the `\n` is not part of the sum of the next line.
- The text mode (`switch_to_text`) ends at a `\r` too. With a `\r\n` end of line, the text used to
  end with `\r`; with a `\r` alone, it used to run into the next lines.
- With `parse-comments` and `parse-trailing-comment`, a line with a trailing comment counted as two
  lines in `get_current_line`.
- Every combination of features compiles, without warnings; most of them did not. Among others:
  `future-stream` with `parse-trailing-comment` or `string-value` (missing `Unpin` bounds),
  `parse-expressions` without `future-stream`, `float-f64` without `std`, `string-value` without
  `parse-trailing-comment`, and `string-value` or `parse-parameters` without `optional-value`.
  Without `parse-trailing-comment`, a `;` is part of the text in text mode.
- The test suite builds and passes with every combination of features, including those without
  `future-stream`, which use the `ByteStream` trait. The tests no longer depend on `f64`
  conversions.
- `either` is only a dependency with `parse-parameters` or `parse-expressions`, and the `cli` example
  requires `std` and `future-stream`.
- No input makes the parser panic (clippy denies overflowing arithmetic, indexing, `unwrap` and
  `panic!` in the crate), at no cost: same RAM, same code for the numbers, and about the same code
  size with `opt-level = "z"`.
  - String values (`string-value`) and comments (`parse-comments`) are bounded by
    `MAX_TEXT_LENGTH`, like the text mode: longer ones are read up to their end, then reported as
    `Error::TextTooLong`. An unterminated string used to grow without limit.
  - Expressions and parameter chains (`##1`) are bounded by `MAX_EXPRESSION_ITEMS`: longer ones are
    an `Error::InvalidExpression`. Nested brackets used to grow without limit.
  - The allocations of strings, comments and expressions are fallible: a full heap is an
    `Error::OutOfMemory` instead of a panic of the allocation error handler.
  - With `future-stream`, the input is not polled again once it has ended (polling a finished
    `Stream` may panic): the parser keeps on returning `None`.

## [0.2.0] – 2020-08-16
### Added
- `Readme.md`, `License` and this `Changelog.md`.
- Async-await no-std friendly implementation.
- Add parsing test suite.
- Command line interface demo.
- GCode example file.
- Expressions and parameters support.
- Add Github actions.
- Documentation of non-obvious, publicly exposed types & methods.

### Changed

- —

<!-- footnote/links -->
[Unreleased]: https://github.com/ithinuel/async-gcode/compare/v0.2.0..HEAD
[0.2.0]: https://github.com/ithinuel/async-gcode/releases/tag/v0.2.0
