#![allow(clippy::useless_conversion)]

//! we need/want to store a value of up to 10^6 with a precision of 10^-5.
//! Fix point arithmetics would require a 14bit fractional part plus a 18bit integer part.
//!
//! Comparing f64 sqrt and few implementation of the sqrt approximation algorithm using the `fixed`
//! crate shows that there's no subtantial benefit in terms of flash size from using the fixed
//! point arithmetics.

#[cfg(all(not(feature = "std"), feature = "string-value"))]
use alloc::string::String;

use crate::{
    stream::PushBackable,
    types::ParseResult,
    utils::next_non_space,
    Error,
    DecimalRepr
};
#[cfg(all(feature = "parse-parameters", not(feature = "parse-expressions")))]
use crate::utils::skip_whitespaces;
#[cfg(any(feature = "parse-parameters", feature = "parse-expressions"))]
use crate::types::Literal;
use crate::stream::{ByteStream, UnpinTrait};
#[cfg(not(feature = "parse-expressions"))]
use crate::types::RealValue;

#[cfg(all(feature = "parse-parameters", not(feature = "parse-expressions")))]
use crate::types::expressions::{Expression, Operator};

#[cfg(feature = "future-stream")]
use futures::StreamExt;

/// Powers of ten for the decimal scales a number can have (at most 9 digits per part).
const POW10: [u32; 10] = [
    1,
    10,
    100,
    1_000,
    10_000,
    100_000,
    1_000_000,
    10_000_000,
    100_000_000,
    1_000_000_000,
];

/// Reads the digits of a number whose first byte, `b`, has already been consumed.
///
/// Returns the value, the number of digits and the first byte that is not a digit. That byte is
/// consumed: the caller decides whether to push it back.
async fn parse_digits<S, E>(input: &mut S, mut b: u8) -> Option<Result<(u32, u8, u8), E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
    E: core::convert::From<Error>,
{
    let mut n = 0u32;
    let mut order = 0u8;
    while b.is_ascii_digit() {
        // At most 9 digits, so `n` stays below 10^9 and cannot overflow.
        if order > 8 {
            return Some(Err(crate::Error::NumberOverflow.into()));
        }
        n = n * 10 + u32::from(b - b'0');
        order += 1;
        b = match input.next().await? {
            Ok(b) => b,
            Err(e) => return Some(Err(e)),
        };
    }
    Some(Ok((n, order, b)))
}

#[cfg(feature = "parse-checksum")]
pub(crate) async fn parse_number<S, E>(input: &mut S) -> Option<Result<(u32, u8), E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
    E: core::convert::From<Error>,
{
    let b = match input.next().await? {
        Ok(b) => b,
        Err(e) => return Some(Err(e)),
    };
    let (n, order, b) = match parse_digits(input, b).await? {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    input.push_back(b);
    Some(Ok((n, order)))
}

/// Parses a real number whose first byte (a sign, a decimal point or a digit) has already been
/// consumed. The byte that ends the number is pushed back.
async fn parse_real_literal<S, E>(input: &mut S, first: u8) -> Option<ParseResult<DecimalRepr, E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
    E: core::convert::From<Error>,
{
    // extract sign: default to positiv
    let mut b = first;
    let negativ = b == b'-';
    if negativ || b == b'+' {
        // skip spaces after the sign
        b = try_result!(next_non_space(input));
    }

    // parse integer part
    let int = if b != b'.' {
        // if not a decimal point, there must be an integer
        let (v, _, next) = try_result!(parse_digits(input, b));
        // skip spaces after integer part
        b = if next == b' ' {
            try_result!(next_non_space(input))
        } else {
            next
        };
        Some(v)
    } else {
        None
    };

    // parse decimal part: mandatory if integer part is abscent
    let dec = if b == b'.' {
        // skip spaces after decimal point
        let first = try_result!(next_non_space(input));
        let (n, ord, next) = try_result!(parse_digits(input, first));
        b = next;
        Some((n, ord))
    } else {
        None
    };
    input.push_back(b);

    let res = if int.is_none() && dec.is_none() {
        ParseResult::Parsing(Error::BadNumberFormat.into())
    } else {
        let int = int.unwrap_or(0);
        let (n, ord) = dec.unwrap_or((0u32, 0u8));
        // The scaled value must fit in the i32 mantissa of `DecimalRepr`.
        let magnitude = int
            .checked_mul(POW10[usize::from(ord)])
            .and_then(|v| v.checked_add(n))
            .and_then(|v| i32::try_from(v).ok());
        match magnitude {
            Some(m) => ParseResult::Ok(DecimalRepr::new(if negativ { -m } else { m }, ord)),
            None => ParseResult::Parsing(Error::NumberOverflow),
        }
    };
    Some(res)
}

#[cfg(feature = "string-value")]
async fn parse_string_literal<S, E>(input: &mut S) -> Option<ParseResult<String, E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
{
    #[cfg(not(feature = "std"))]
    use alloc::vec::Vec;

    // we cannot use take_until(…).collect() because we need to distinguish input's end of stream
    // from take_until(…) end of stream

    let mut array = Vec::new();
    loop {
        match try_result!(input.next()) {
            b'"' => break,
            b'\\' => {
                array.push(try_result!(input.next()));
            }
            b => array.push(b),
        }
    }

    match String::from_utf8(array) {
        Ok(string) => Some(ParseResult::Ok(string)),
        Err(_) => Some(Error::InvalidUTF8String.into()),
    }
}

#[cfg(any(feature = "parse-parameters", feature = "parse-expressions"))]
pub(crate) async fn parse_literal<S, E>(input: &mut S) -> Option<ParseResult<Literal, E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
    E: core::convert::From<Error>,
{
    let b = try_result!(input.next());
    Some(match b {
        b'+' | b'-' | b'.' | b'0'..=b'9' => {
            ParseResult::Ok(Literal::from(try_parse!(parse_real_literal(input, b))))
        }
        #[cfg(feature = "string-value")]
        b'"' => ParseResult::Ok(Literal::from(try_parse!(parse_string_literal(input)))),
        _ => Error::UnexpectedByte(b).into(),
    })
}

/// Reads the text of the line: until the end of the line, which is pushed back, or the end of the
/// input. With `parse-trailing-comment`, a `;` also ends the text and is pushed back.
#[cfg(feature = "string-value")]
pub(crate) async fn parse_text<S, E>(input: &mut S) -> Option<ParseResult<String, E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
    E: core::convert::From<Error>,
{
    #[cfg(not(feature = "std"))]
    use alloc::vec::Vec;

    let mut array = Vec::new();
    loop {
        match input.next().await {
            #[cfg(feature = "parse-trailing-comment")]
            Some(Ok(b';')) => {
                input.push_back(b';');
                break;
            }
            Some(Ok(b @ (b'\r' | b'\n'))) => {
                input.push_back(b);
                break;
            }
            Some(Ok(b)) => {
                array.push(b)
            }
            Some(Err(e)) => {
                return Some(Err(e).into())
            }
            None => {
                break;
            }
        }
    }
    match String::from_utf8(array) {
        Ok(string) => Some(ParseResult::Ok(string)),
        Err(_) => Some(Error::InvalidUTF8String.into()),
    }
}

/// Skips spaces and parses a real value (the value of a word, a line number or a parameter).
#[cfg(not(feature = "parse-expressions"))]
pub(crate) async fn parse_word_value<S, E>(input: &mut S) -> Option<ParseResult<RealValue, E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
    E: core::convert::From<Error>,
{
    let b = try_result!(next_non_space(input));
    // println!("real value: {:?}", b as char);

    let res = match b {
        b'+' | b'-' | b'.' | b'0'..=b'9' => {
            ParseResult::Ok(RealValue::from(try_parse!(parse_real_literal(input, b))))
        }
        #[cfg(feature = "string-value")]
        b'"' => ParseResult::Ok(RealValue::from(try_parse!(parse_string_literal(input)))),
        #[cfg(feature = "parse-parameters")]
        b'#' => {
            #[cfg(not(feature = "std"))]
            use alloc::vec::Vec;

            let mut n = 1;
            let literal = loop {
                try_result!(skip_whitespaces(input));
                let b = try_result!(input.next());
                if b != b'#' {
                    input.push_back(b);

                    break try_parse!(parse_literal(input));
                }
                n += 1;
            };

            let vec: Vec<_> = core::iter::once(literal.into())
                .chain(core::iter::repeat_n(Operator::GetParameter.into(), n))
                .collect();
            ParseResult::Ok(Expression(vec).into())
        }
        #[cfg(feature = "optional-value")]
        b => {
            input.push_back(b);
            ParseResult::Ok(RealValue::None)
        }
        #[cfg(not(feature = "optional-value"))]
        b => Error::UnexpectedByte(b).into(),
    };
    Some(res)
}
