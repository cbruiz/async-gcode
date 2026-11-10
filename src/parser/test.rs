use super::{Error, GCode, Parser};

use crate::test_utils::stream_of;
use crate::types::RealValue;
use crate::DecimalRepr;

#[cfg(feature = "parse-checksum")]
mod parse_checksum;
#[cfg(feature = "parse-expressions")]
mod parse_expressions;
#[cfg(feature = "parse-parameters")]
mod parse_parameters;
#[cfg(feature = "parse-trailing-comment")]
mod parse_trailing_comment;

/// Parses the whole input, whose items may be stream errors.
fn parse_all<I, E>(input: I) -> Vec<Result<GCode, E>>
where
    I: Iterator<Item = Result<u8, E>> + Unpin,
    E: From<Error>,
{
    let mut parser = Parser::new(stream_of(input));
    futures_executor::block_on(async {
        let mut items = Vec::new();
        while let Some(item) = parser.next().await {
            items.push(item);
        }
        items
    })
}

/// Parses the whole input, and the number of lines.
fn parse_all_and_count<T: Iterator<Item = u8> + Unpin>(it: T) -> (Vec<Result<GCode, Error>>, u32) {
    let mut parser = Parser::new(stream_of(it.map(Result::<_, Error>::Ok)));
    futures_executor::block_on(async {
        let mut items = Vec::new();
        while let Some(item) = parser.next().await {
            items.push(item);
        }
        (items, parser.get_current_line())
    })
}

fn block_on<T: Iterator<Item = u8> + Unpin>(it: T) -> Vec<Result<GCode, Error>> {
    parse_all(it.map(Result::<_, Error>::Ok))
}

/// The real value `integer / 10^scale`: `num(-21098, 3)` is -21.098.
fn num(integer: i32, scale: u8) -> RealValue {
    DecimalRepr::new(integer, scale).into()
}

/// The literal `integer / 10^scale`: `lit(-21098, 3)` is -21.098.
#[cfg(any(feature = "parse-parameters", feature = "parse-expressions"))]
fn lit(integer: i32, scale: u8) -> crate::Literal {
    DecimalRepr::new(integer, scale).into()
}
#[cfg(not(feature = "parse-comments"))]
fn to_gcode_comment(_msg: &str) -> [Result<GCode, Error>; 0] {
    []
}
#[cfg(feature = "parse-comments")]
fn to_gcode_comment(msg: &str) -> [Result<GCode, Error>; 1] {
    [Ok(GCode::Comment(msg.to_string()))]
}

#[test]
fn empty_lines_are_not_ignored() {
    assert_eq!(
        block_on("\n\n".bytes()),
        &[Ok(GCode::Execute), Ok(GCode::Execute)]
    );
}

#[test]
fn lines_end_with_lf_cr_or_crlf() {
    // "\r\n" is a single end of line, like "\r" or "\n" alone; so is "\n\r" followed by "\r".
    for input in [
        "G1\nG2\n\nG3\n",
        "G1\r\nG2\r\n\r\nG3\r\n",
        "G1\rG2\r\rG3\r",
        "G1\r\nG2\n\rG3\r\n",
        "G1 \r\n  G2\r\n \r\nG3  \r\n",
    ] {
        assert_eq!(
            parse_all_and_count(input.bytes()),
            (
                vec![
                    Ok(GCode::Word('g', num(1, 0))),
                    Ok(GCode::Execute),
                    Ok(GCode::Word('g', num(2, 0))),
                    Ok(GCode::Execute),
                    Ok(GCode::Execute),
                    Ok(GCode::Word('g', num(3, 0))),
                    Ok(GCode::Execute),
                ],
                4
            ),
            "input: {:?}",
            input
        );
    }
}

#[test]
fn a_cr_ends_the_line_without_waiting_for_the_next_byte() {
    // The input ends right after the '\r'.
    assert_eq!(
        parse_all_and_count("G1\r".bytes()),
        (vec![Ok(GCode::Word('g', num(1, 0))), Ok(GCode::Execute)], 1)
    );
}

#[test]
fn a_crlf_ends_the_error_recovery_once() {
    assert_eq!(
        parse_all_and_count("N.1 G1\r\nG2\r\n".bytes()),
        (
            vec![
                Err(Error::UnexpectedByte(b'.')),
                Ok(GCode::Execute),
                Ok(GCode::Word('g', num(2, 0))),
                Ok(GCode::Execute),
            ],
            2
        )
    );
}

#[test]
fn block_delete_emited_immediately() {
    assert_eq!(block_on("/".bytes()), &[Ok(GCode::BlockDelete)]);
}

#[test]
fn spaces_are_not_allowed_before_block_delete() {
    assert_eq!(block_on(" /".bytes()), &[Err(Error::UnexpectedByte(b'/'))]);
}

#[test]
fn error_in_underlying_stream_are_passed_through_and_parser_recovers_on_execute() {
    #[derive(Debug, Copy, Clone, PartialEq)]
    enum TestError {
        SomeError,
        ParseError(Error),
    }
    impl From<Error> for TestError {
        fn from(from: Error) -> Self {
            TestError::ParseError(from)
        }
    }

    let string = b"g12 h12 b32\n";
    let input = string[0..6]
        .iter()
        .copied()
        .map(Result::Ok)
        .chain([Err(TestError::SomeError)].iter().copied())
        .chain(string[6..].iter().copied().map(Result::Ok));

    assert_eq!(
        parse_all(input),
        [
            Ok(GCode::Word('g', (12).into())),
            Err(TestError::SomeError),
            Ok(GCode::Execute)
        ]
    )
}

#[test]
fn line_number() {
    let input = "n23\n".bytes();
    assert_eq!(
        block_on(input),
        &[Ok(GCode::LineNumber(Some(23))), Ok(GCode::Execute)]
    );

    let input = "N0023\n".bytes();
    assert_eq!(
        block_on(input),
        &[Ok(GCode::LineNumber(Some(23))), Ok(GCode::Execute)]
    );

    let input = " N 0023 \n".bytes();
    assert_eq!(
        block_on(input),
        &[Ok(GCode::LineNumber(Some(23))), Ok(GCode::Execute)]
    );
}

#[test]
fn line_number_with_more_than_9_digits_are_not_ok() {
    let input = "N9999999999\n".bytes();
    assert_eq!(
        block_on(input),
        &[Err(Error::NumberOverflow), Ok(GCode::Execute)]
    )
}

#[test]
fn line_number_can_only_be_intergers() {
    let input = "N000.09\n".bytes();
    assert_eq!(
        block_on(input),
        &[
            Err(Error::UnexpectedByte(b'.')),
            Ok(GCode::Execute)
        ]
    );
    let input = "N009.\n".bytes();
    assert_eq!(
        block_on(input),
        &[
            Ok(GCode::LineNumber(Some(9))),
            Ok(GCode::Execute)
        ]
    );
    let input = "N.009\n".bytes();
    assert_eq!(
        block_on(input),
        &[Err(Error::UnexpectedByte(b'.')), Ok(GCode::Execute)]
    )
}

#[test]
#[cfg(not(feature = "optional-value"))]
fn line_number_must_have_a_value() {
    let input = "N\n".bytes();
    assert_eq!(block_on(input), &[Err(Error::UnexpectedByte(b'\n'))]);
}

#[test]
#[cfg(feature = "optional-value")]
fn line_number_may_not_have_a_value() {
    let input = "N\n".bytes();
    assert_eq!(
        block_on(input),
        &[Ok(GCode::LineNumber(None)), Ok(GCode::Execute)]
    );
}

#[test]
fn comments_may_contain_utf8_characters() {
    let msg = "accélération = δv/δt";
    let input = format!("({})", msg).into_bytes().into_iter();

    assert_eq!(block_on(input), to_gcode_comment(msg));
}

#[test]
fn open_parenthesis_are_not_allowed_in_comments() {
    let msg = "accélération = (δv/δt";
    let input = format!("({})", msg).into_bytes().into_iter();
    assert_eq!(block_on(input), [Err(Error::UnexpectedByte(b'('))]);
}

#[test]
fn comments_may_contain_escaped_open_parenthesis() {
    let msg = "accélération = (δv/δt)";
    let input = "(accélération = \\(δv/δt\\))".bytes();
    assert_eq!(block_on(input), to_gcode_comment(msg));
}

#[test]
fn word_with_number() {
    let input = r"
        G21 H21. I21.098
        J-21 K-21. L-21.098
        M+21 N+21. P+21.098
        Q.098 R-.098 S+.098
        t - 21 . 33 "
        .bytes();

    assert_eq!(
        block_on(input),
        &[
            Ok(GCode::Execute),
            Ok(GCode::Word('g', num(21, 0))),
            Ok(GCode::Word('h', num(21, 0))),
            Ok(GCode::Word('i', num(21098, 3))),
            Ok(GCode::Execute),
            Ok(GCode::Word('j', num(-21, 0))),
            Ok(GCode::Word('k', num(-21, 0))),
            Ok(GCode::Word('l', num(-21098, 3))),
            Ok(GCode::Execute),
            Ok(GCode::Word('m', num(21, 0))),
            Ok(GCode::Word('n', num(21, 0))),
            Ok(GCode::Word('p', num(21098, 3))),
            Ok(GCode::Execute),
            Ok(GCode::Word('q', num(98, 3))),
            Ok(GCode::Word('r', num(-98, 3))),
            Ok(GCode::Word('s', num(98, 3))),
            Ok(GCode::Execute),
            Ok(GCode::Word('t', num(-2133, 2))),
        ]
    );
}

#[test]
fn word_values_must_fit_the_decimal_mantissa() {
    // The scaled value must fit in the i32 mantissa of `DecimalRepr`. Beyond that the value used
    // to wrap around (or panic with overflow checks); now it is a number overflow and the parser
    // recovers on the next line.
    let input = "X21474.83647\nX-21474.83647\nX21474.83648\nG1 Y99999.99999 Z1\nG1 Z2\n".bytes();
    assert_eq!(
        block_on(input),
        &[
            Ok(GCode::Word('x', DecimalRepr::new(i32::MAX, 5).into())),
            Ok(GCode::Execute),
            Ok(GCode::Word('x', DecimalRepr::new(-i32::MAX, 5).into())),
            Ok(GCode::Execute),
            Err(Error::NumberOverflow),
            Ok(GCode::Execute),
            Ok(GCode::Word('g', DecimalRepr::new(1, 0).into())),
            Err(Error::NumberOverflow),
            Ok(GCode::Execute),
            Ok(GCode::Word('g', DecimalRepr::new(1, 0).into())),
            Ok(GCode::Word('z', DecimalRepr::new(2, 0).into())),
            Ok(GCode::Execute),
        ]
    );
}

#[test]
#[cfg(all(feature = "optional-value", not(feature = "parse-expressions")))]
fn word_may_not_have_a_value() {
    let input = "G75 Z T48 S P.3\n".bytes();
    assert_eq!(
        block_on(input),
        &[
            Ok(GCode::Word('g', num(75, 0))),
            Ok(GCode::Word('z', RealValue::None)),
            Ok(GCode::Word('t', num(48, 0))),
            Ok(GCode::Word('s', RealValue::None)),
            Ok(GCode::Word('p', num(3, 1))),
            Ok(GCode::Execute)
        ]
    );
}

#[test]
#[cfg(all(feature = "optional-value", feature = "parse-expressions"))]
fn word_may_not_have_a_value_but_ambiguous_sequence_will_error() {
    // T following a value less Z will be read as the start of «Tan» and the expression will err
    // on the unexpected '4' that follows.
    let input = "G75 Z T48 S P.3\n".bytes();
    assert_eq!(
        block_on(input),
        &[
            Ok(GCode::Word('g', num(75, 0))),
            Ok(GCode::Word('z', RealValue::None)),
            Err(Error::UnexpectedByte(b'4')),
            Ok(GCode::Execute)
        ]
    );
}

#[test]
#[cfg(feature = "string-value")]
fn line_numbers_cannot_be_strings() {
    let input = "N\"12\" G1\nG2\n".bytes();
    assert_eq!(
        block_on(input),
        &[
            Err(Error::InvalidNumberConversion),
            Ok(GCode::Execute),
            Ok(GCode::Word('g', num(2, 0))),
            Ok(GCode::Execute),
        ]
    );
}

/// Parses the input switching to text mode after `M117`, as printhor does.
#[cfg(feature = "string-value")]
fn parse_with_text(input: &str) -> Vec<Result<GCode, Error>> {
    let mut parser = Parser::new(stream_of(input.bytes().map(Result::<_, Error>::Ok)));
    futures_executor::block_on(async {
        let mut items = Vec::new();
        while let Some(item) = parser.next().await {
            if item == Ok(GCode::Word('m', num(117, 0))) {
                parser.switch_to_text();
            }
            items.push(item);
        }
        items
    })
}

#[cfg(feature = "string-value")]
fn text(s: &str) -> Result<GCode, Error> {
    Ok(GCode::Text(s.to_string().into()))
}

#[test]
#[cfg(feature = "string-value")]
fn text_ends_at_the_end_of_the_line() {
    for input in ["M117 Hello world\nG1\n", "M117 Hello world\r\nG1\r\n", "M117 Hello world\rG1\r"] {
        assert_eq!(
            parse_with_text(input),
            &[
                Ok(GCode::Word('m', num(117, 0))),
                text("Hello world"),
                Ok(GCode::Execute),
                Ok(GCode::Word('g', num(1, 0))),
                Ok(GCode::Execute),
            ],
            "input: {:?}",
            input
        );
    }
    assert_eq!(
        parse_with_text("M117\r\n"),
        &[Ok(GCode::Word('m', num(117, 0))), text(""), Ok(GCode::Execute)]
    );
}

#[test]
#[cfg(all(feature = "string-value", feature = "parse-trailing-comment"))]
fn text_ends_at_a_trailing_comment() {
    let mut expected = vec![Ok(GCode::Word('m', num(117, 0))), text("Hello ")];
    expected.extend_from_slice(&to_gcode_comment(" world"));
    expected.push(Ok(GCode::Execute));
    assert_eq!(parse_with_text("M117 Hello ; world\r\n"), expected);
}

#[test]
#[cfg(all(feature = "string-value", not(feature = "parse-trailing-comment")))]
fn text_may_contain_semicolons_without_trailing_comments() {
    assert_eq!(
        parse_with_text("M117 Hello ; world\r\n"),
        &[
            Ok(GCode::Word('m', num(117, 0))),
            text("Hello ; world"),
            Ok(GCode::Execute),
        ]
    );
}

#[test]
#[cfg(feature = "string-value")]
fn value_may_be_strings() {
    let input = r#"G "Hello\"World\"" "#.bytes();
    assert_eq!(
        block_on(input),
        &[Ok(GCode::Word('g', "Hello\"World\"".to_string().into()))]
    )
}

#[test]
fn into_inner_returns_the_rest_of_the_input() {
    use crate::test_utils::collect;
    let mut parser = Parser::new(stream_of("G1\nG2\n".bytes().map(Result::<_, Error>::Ok)));
    let first = futures_executor::block_on(parser.next());
    assert_eq!(first, Some(Ok(GCode::Word('g', num(1, 0)))));
    // The end of the first line was pushed back by the parser: only that byte is dropped
    let mut stream = parser.into_inner();
    assert_eq!(
        collect(&mut stream),
        "G2\n".bytes().map(Result::<_, Error>::Ok).collect::<Vec<_>>()
    );
}
