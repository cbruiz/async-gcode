//! This crate implements a GCode (RS-274) parser.
//!
//! The default dialect is taken from NIST's [RS274/NGC interpreter version 3].
//! Some expensive part of that dialect such as parameters and expressions are gated behind feature
//! – respectively `parse-parameters` and `parse-expressions` – in order to avoid dependency on
//! dynamic allocation.
//!
//! Some extension to this dialect such as checksum, trailing comments, optional values are also
//! supported and gated behind features.
//!
//! ## 🔩 Example
//!
//! With the `future-stream` feature, the input is a `futures::stream::Stream`:
//!
//! ```
//! # #[cfg(feature = "future-stream")] {
//! use futures::stream;
//! use futures_executor::block_on;
//! use async_gcode::{Parser, Error};
//! let input = r"
//!    G21 H21. I21.098
//!    J-21 K-21. L-21.098
//!    M+21 N+21. P+21.098
//!    Q.098 R-.098 S+.098
//!    t - 21 . 33 "
//!    // mapping to `Result<u8, Error>` to render error conversion transparent.
//!    .bytes().map(Result::<_, Error>::Ok);
//!
//! block_on(async {
//!     let mut parser = Parser::new(stream::iter(input));
//!
//!     loop {
//!         if let Some(res) = parser.next().await {
//!             println!("{:?}", res);
//!         } else {
//!             break;
//!         }
//!     }
//! });
//! # }
//! ```
//!
//! Without it, the input implements this crate's [`ByteStream`] trait.
//!
//! ## Error management
//!
//! On parsing error the `Parser` can no longer trust its input and enters an error recovery state.
//! No `GCode` or `Error` will be emitted until the end of the line is received. Then a
//! `GCode::Execute` is emitted and the parser restarts in a reliable known state.
//!
//! ## End of line
//!
//! A line ends with `\n`, `\r` or `\r\n`. Each of them emits a single `GCode::Execute` and counts
//! as a single line. `GCode::Execute` is emitted as soon as the `\r` is received, without waiting
//! for the next byte.
//!
//! ## ⚙ Features
//! - `std` : Enabled by default. Allows for the use of dynamic allocation.
//! - `parse-comments` : enables the parser to return `GCode::Comment(String)`; requires an allocator.
//! - `parse-trailing-comment`: allows line to end with a `; comment`.
//! - `parse-checksum` : Enables the use of xorsum.
//! - `parse-parameters` : Enables the use of `#` parameters ; requires an allocator.
//!   If `string-value` is enabled then parameters may use string index.
//!   If `optional-value` is enabled then parameters value may be omitted but **NOT** the indices.
//! - `parse-expressions` : Enables parsing infix expressions ; requires an allocator.
//! - `optional-value` : Allows to omit in `RealValue` in word and parameter value positions.
//!   Parameter indices cannot be omitted nor can be literals in expressions.
//! - `string-value` : Allows `RealValue` to be a string. Any character preceded with `\` will be
//!   used as is (useful for `"`, `)` or new line). Also enables the text mode
//!   (`Parser::switch_to_text`), which reads the rest of the line as a `GCode::Text`. The text
//!   ends at the end of the line, or at a `;` when `parse-trailing-comment` is enabled.
//! - `future-stream` : Enabled by default. The input is a `futures::stream::Stream`. Without it,
//!   the input implements this crate's `ByteStream` trait.
//! - `float-f64` : Enabled by `std`. Conversions between `f64` and the parsed values.
//!
//! ## ⚠ Warning
//!
//! Dev-dependencies currently leak features to dependencies.
//! This crate requires rust-nightly to build with no_std until `-Z features=dev_dep` makes it to
//! stable.
//!
//! ## Note for future development
//! It might be interesting to have a look at ISO 6983 and/or ISO 14649.
//!
//! During development fixed arithmetics was considered to be made available as an alternative to
//! the floating point arithmetics especially for small target. After investigation, it does not
//! seem to be a significant gain for the precision required by the gcode standard.
//!
//! ## No panic
//!
//! The parser runs in the async tasks of firmwares, where a panic halts the machine: no input
//! makes it panic. A number too large for the internal representation is an
//! [`Error::NumberOverflow`]. The values that need the heap (strings, comments, expressions) are
//! bounded (`MAX_TEXT_LENGTH`, `MAX_EXPRESSION_ITEMS`) and their allocations are fallible: a
//! full heap is an `Error::OutOfMemory` instead of a panic of the allocation error handler.
//! Clippy enforces the absence of arithmetic overflow, indexing, `unwrap` and `panic!` in the
//! crate.
//!
//! [RS274/NGC interpreter version 3]: https://www.nist.gov/publications/nist-rs274ngc-interpreter-version-3?pub_id=823374
// The tests use the standard library whatever the features.
#![cfg_attr(not(any(feature = "std", test)), no_std)]
// See "No panic" above. The tests are free to panic.
#![cfg_attr(
    not(test),
    deny(
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented
    )
)]

#[cfg(all(
    not(feature = "std"),
    any(
        feature = "parse-expressions",
        feature = "parse-parameters",
        feature = "parse-comments",
        feature = "string-value"
    )
))]
extern crate alloc;

#[cfg(all(not(feature = "std"), feature = "parse-comments"))]
use alloc::string::String;

#[macro_use]
mod utils;

mod stream;
mod types;

#[cfg(test)]
mod test_utils;

mod parser;

pub use stream::{ByteStream, TryByteStream};
pub use parser::AsyncParserState;
pub use parser::Parser;
pub use types::Literal;
pub use types::RealValue;

pub use types::DecimalRepr;

#[cfg(any(feature = "parse-expressions", feature = "parse-parameters"))]
pub use types::expressions::Expression;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Error {
    /// Error no the gcode syntax
    UnexpectedByte(u8),

    /// The parsed number exceeded the expected range.
    NumberOverflow,

    /// Incompatible number conversions.
    InvalidNumberConversion,

    /// Format error during number parsing. Typically a dot without digits (at least one is
    /// required).
    BadNumberFormat,

    #[cfg(any(feature = "parse-comments", feature = "string-value"))]
    /// The string or comment received contained an invalid UTF-8 character sequence.
    InvalidUTF8String,

    #[cfg(any(feature = "parse-comments", feature = "string-value"))]
    /// The string or comment received is longer than [MAX_TEXT_LENGTH] bytes.
    TextTooLong,

    #[cfg(feature = "parse-checksum")]
    /// Checksum verification failed. The error contains the computed value.
    BadChecksum(u8),

    #[cfg(any(feature = "parse-expressions", feature = "parse-parameters"))]
    /// The expressions received was invalid, longer than [MAX_EXPRESSION_ITEMS], or could not be
    /// evaluated (`Expression::evaluate` is not implemented yet).
    InvalidExpression,

    #[cfg(any(
        feature = "parse-comments",
        feature = "string-value",
        feature = "parse-parameters",
        feature = "parse-expressions"
    ))]
    /// The heap could not hold the string, comment or expression received.
    OutOfMemory,
}

/// Max length (in bytes) of a string value, a text or a comment. Longer ones are consumed up to
/// their end, then rejected with [Error::TextTooLong].
#[cfg(any(feature = "parse-comments", feature = "string-value"))]
pub const MAX_TEXT_LENGTH: usize = 128;

/// Max number of items (literals, operators and open brackets) of an expression or a parameter
/// chain (`##1`). Longer ones are rejected with [Error::InvalidExpression].
#[cfg(any(feature = "parse-expressions", feature = "parse-parameters"))]
pub const MAX_EXPRESSION_ITEMS: usize = 64;

#[derive(Debug, PartialEq, Clone)]
pub enum GCode {
    StatusCommand,
    BlockDelete,
    LineNumber(Option<u32>),
    #[cfg(feature = "parse-comments")]
    Comment(String),
    Word(char, RealValue),
    #[cfg(feature = "string-value")]
    Text(RealValue),
    #[cfg(feature = "parse-parameters")]
    /// When `optional-value` is enabled, the index cannot be `RealValue::None`.
    ParameterSet(RealValue, RealValue),
    Execute,
}
