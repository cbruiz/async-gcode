//! The ebnf representatio following [https://bottlecaps.de/rr/ui]'s syntax
//! ```ebnf
//! line       ::= '/'? ( [Nn] [0-9]+ )?
//!                ( [a-zA-Z] real_value? | '#' real_value '=' real_value? | '(' [^)] ')' )*
//!                ( '*' [0-9]+ /* 0 to 255 */ )?
//!                ( ';' [^\n]* )? '\n'
//! real_value ::= '#'* ( real_number
//!                     | '"' [^"] '"'
//!                     | 'atan' expression '/' expression
//!                     | ( 'abs' | 'acos' | 'asin' | 'cos' | 'exp' | 'fix' | 'fup' | 'ln' | 'round' | 'sin' | 'sqrt' | 'tan' ) expression )
//! expression ::= '[' real_value ( ( '**' | '/' | 'mod' | '*' | 'and' | 'xor' | '-' | 'or' | '+' ) real_value )* ']'
//! real_number ::= ( '+' | '-' )? ( [0-9]+ ( '.' [0-9]* )? | '.' [0-9]+ )
//! ```
//!
#[cfg(all(not(feature = "std"), feature = "parse-comments"))]
use alloc::string::String;
#[cfg(all(not(feature = "std"), feature = "parse-comments"))]
use alloc::vec::Vec;

mod values;

#[cfg(feature = "parse-expressions")]
mod expressions;

#[cfg(test)]
mod test;

use crate::{
    stream::{MyTryStreamExt, PushBackable},
    types::{Comment, ParseResult},
    Error, GCode, Literal, RealValue
};

#[cfg(any(feature = "string-value", feature = "parse-checksum", feature = "parse-parameters"))]
use crate::utils::skip_whitespaces;

#[cfg(feature = "future-stream")]
use futures::StreamExt;

#[cfg(feature = "parse-checksum")]
use values::parse_number;

#[cfg(not(feature = "parse-expressions"))]
use values::parse_word_value;

#[cfg(feature = "string-value")]
use crate::parser::values::parse_text;

#[cfg(feature = "parse-expressions")]
use expressions::parse_word_value;
use crate::stream::{ByteStream, UnpinTrait};

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum AsyncParserState {
    Start(bool),
    LineNumberOrSegment,
    #[cfg(feature = "string-value")]
    TextMode,
    Segment,
    ErrorRecovery,
    #[cfg(all(
        feature = "parse-trailing-comment",
        any(feature = "parse-checksum", feature = "string-value")
    ))]
    EoLOrTrailingComment,
    #[cfg(any(
        feature = "parse-trailing-comment",
        feature = "parse-checksum",
        feature = "string-value"
    ))]
    EndOfLine,
}

#[cfg(all(feature = "parse-trailing-comment", not(feature = "parse-comments")))]
async fn parse_eol_comment<S, E>(input: &mut S) -> Option<Result<Comment, E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
{
    loop {
        let b = match input.next().await? {
            Ok(o) => o,
            Err(e) => return Some(Err(e)),
        };
        match b {
            b'\r' | b'\n' => {
                input.push_back(b);
                break Some(Ok(()));
            }
            _ => {}
        }
    }
}

#[cfg(all(feature = "parse-trailing-comment", feature = "parse-comments"))]
async fn parse_eol_comment<S, E>(input: &mut S) -> Option<ParseResult<Comment, E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
{
    let mut v = Vec::new();
    loop {
        let b = try_result!(input.next());
        match b {
            b'\r' | b'\n' => {
                input.push_back(b);
                break Some(match String::from_utf8(v) {
                    Ok(s) => ParseResult::Ok(s),
                    Err(_) => Error::InvalidUTF8String.into(),
                });
            }
            b => v.push(b),
        }
    }
}

#[cfg(not(feature = "parse-comments"))]
async fn parse_inline_comment<S, E>(input: &mut S) -> Option<ParseResult<Comment, E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
{
    loop {
        match try_result!(input.next()) {
            b'\\' => {
                try_result!(input.next());
            }
            b'(' => break Some(Error::UnexpectedByte(b'(').into()),
            b')' => break Some(ParseResult::Ok(())),
            _ => {}
        }
    }
}

#[cfg(feature = "parse-comments")]
async fn parse_inline_comment<S, E>(input: &mut S) -> Option<ParseResult<Comment, E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
{
    let mut v = Vec::new();
    loop {
        let b = try_result!(input.next());
        match b {
            b'\\' => {
                v.push(try_result!(input.next()));
            }
            b'(' => break Some(Error::UnexpectedByte(b'(').into()),
            b')' => {
                break Some(match String::from_utf8(v) {
                    Ok(s) => ParseResult::Ok(s),
                    Err(_) => Error::InvalidUTF8String.into(),
                })
            }
            b => v.push(b),
        }
    }
}

// use a different struct to compute checksum
#[cfg(not(feature = "parse-checksum"))]
use crate::stream::pushback::PushBack;
#[cfg(feature = "parse-checksum")]
type PushBack<T> = crate::stream::xorsum_pushback::XorSumPushBack<T>;

macro_rules! try_await {
    ($input:expr) => {
        match $input.await? {
            ParseResult::Ok(ok) => ok,
            ParseResult::Parsing(err) => break Err(err.into()),
            ParseResult::Input(err) => break Err(err.into()),
        }
    };
}

// Unused when no feature needs it.
#[allow(unused_macros)]
macro_rules! try_await_result {
    ($input:expr) => {
        match $input.await? {
            Ok(ok) => ok,
            Err(err) => break Err(err.into()),
        }
    };
}

pub struct Parser<S, E>
where
    S: ByteStream<Item = Result<u8, E>>,
{
    input: PushBack<S>,
    state: AsyncParserState,
    line_count: u32,
    /// The last line ended with a `\r`: a `\n` right after it is part of the same end of line.
    after_cr: bool,
}

impl<S, E> Parser<S, E>
where
    S: ByteStream<Item = Result<u8, E>> + UnpinTrait,
    E: From<Error>,
{
    pub fn new(input: S) -> Self {
        Self {
            #[cfg(feature = "parse-checksum")]
            input: input.xor_summed_push_backable(0),
            #[cfg(not(feature = "parse-checksum"))]
            input: input.push_backable(),
            state: AsyncParserState::Start(true),
            line_count: 0,
            after_cr: false,
        }
    }

    pub async fn reset(&mut self)  {
        #[cfg(feature = "parse-checksum")]
        self.input.reset_sum(0);
        #[cfg(not(feature = "future-stream"))]
        self.input.recovery_check().await;
        self.state = AsyncParserState::Start(true);
    }

    pub fn get_state(&self) -> AsyncParserState {
        self.state
    }

    #[cfg(feature = "string-value")]
    pub fn switch_to_text(&mut self) {
        self.state = AsyncParserState::TextMode;
    }

    pub fn get_current_line(&self) -> u32 {
        self.line_count
    }

    pub fn update_current_line(&mut self, line: u32) {
        self.line_count = line;
    }

    /// Ends the current line, whose end of line byte (`\r` or `\n`) has just been read.
    fn end_line(&mut self, eol: u8) -> GCode {
        self.state = AsyncParserState::Start(true);
        #[cfg(feature = "parse-checksum")]
        self.input.reset_sum(0);
        self.after_cr = eol == b'\r';
        self.line_count += 1;
        GCode::Execute
    }

    /// Skips spaces and ends the line, which must come next.
    #[cfg(any(
        feature = "parse-trailing-comment",
        feature = "parse-checksum",
        feature = "string-value"
    ))]
    async fn parse_eol(&mut self) -> Option<ParseResult<GCode, E>> {
        Some(loop {
            match try_result!(self.input.next()) {
                b @ (b'\r' | b'\n') => break ParseResult::Ok(self.end_line(b)),
                b' ' => {}
                b => break Error::UnexpectedByte(b).into(),
            }
        })
    }

    pub async fn next(&mut self) -> Option<Result<GCode, E>> {
        // A byte that a state hands over to the next state of this same call. Equivalent to
        // pushing it back to the input, without going through the input again.
        let mut handed_over: Option<u8> = None;
        if self.after_cr {
            // The line already ended at the '\r', so a '\n' that follows is skipped.
            let next = self.input.next().await?;
            self.after_cr = false;
            match next {
                Ok(b'\n') => {
                    // The '\n' went into the sum of the next line.
                    #[cfg(feature = "parse-checksum")]
                    self.input.reset_sum(0);
                }
                Ok(b) => handed_over = Some(b),
                Err(err) => return Some(Err(err)),
            }
        }
        let res = loop {
            let b = match handed_over.take() {
                Some(b) => b,
                None => match self.input.next().await? {
                    Ok(b) => b,
                    Err(err) => return Some(Err(err)),
                },
            };

            // println!("{:?}: {:?}", self.state, char::from(b));
            match self.state {
                AsyncParserState::Start(ref mut first_byte) => match b {
                    b'?' => {
                        break Ok(GCode::StatusCommand);
                    }
                    b'\r' | b'\n' => {
                        break Ok(self.end_line(b));
                    }
                    b'/' if *first_byte => {
                        self.state = AsyncParserState::LineNumberOrSegment;
                        break Ok(GCode::BlockDelete);
                    }
                    b' ' => {
                        *first_byte = false;
                    }
                    _ => {
                        handed_over = Some(b);
                        self.state = AsyncParserState::LineNumberOrSegment
                    }
                },
                AsyncParserState::LineNumberOrSegment => match b.to_ascii_lowercase() {
                    b'n' => {
                        let rv = try_await!(parse_word_value(&mut self.input));
                        match rv {
                            RealValue::Literal(Literal::RealNumber(_rn)) => {
                                if _rn.scale() > 0 {
                                    break Err(Error::UnexpectedByte(b'.').into());
                                }
                                else {
                                    let lnum = _rn.integer_part().try_into().ok();
                                    break Ok(GCode::LineNumber(lnum));
                                }
                            }
                            #[cfg(feature = "optional-value")]
                            RealValue::None => {
                                break Ok(GCode::LineNumber(None));
                            }
                            // A string or an expression, depending on the features.
                            #[allow(unreachable_patterns)]
                            _ => {
                                break Err(Error::InvalidNumberConversion.into());
                            }
                        }
                    }
                    _ => {
                        handed_over = Some(b);
                        self.state = AsyncParserState::Segment;
                    }
                },
                #[cfg(feature="string-value")]
                AsyncParserState::TextMode => {
                    self.input.push_back(b);
                    try_await_result!(skip_whitespaces(&mut self.input));
                    let rv = try_await!(parse_text(&mut self.input));
                    #[cfg(feature = "parse-trailing-comment")]
                    {
                        self.state = AsyncParserState::EoLOrTrailingComment;
                    }
                    #[cfg(not(feature = "parse-trailing-comment"))]
                    {
                        self.state = AsyncParserState::EndOfLine;
                    }
                    break Ok(GCode::Text(RealValue::Literal(Literal::String(rv))));
                },
                AsyncParserState::Segment => match b.to_ascii_lowercase() {
                    b' ' => {}
                    letter @ b'a'..=b'z' => {
                        let rv = try_await!(parse_word_value(&mut self.input));
                        // println!("word({:?}, {:?})", letter as char, rv);
                        break Ok(GCode::Word(letter.into(), rv));
                    }
                    b'\r' | b'\n' => {
                        break Ok(self.end_line(b));
                    }
                    // param support feature
                    #[cfg(feature = "parse-parameters")]
                    b'#' => {
                        #[allow(clippy::match_single_binding)]
                        let param_id = match try_await!(parse_word_value(&mut self.input)) {
                            #[cfg(feature = "optional-value")]
                            crate::RealValue::None => {
                                let b = try_await_result!(self.input.next());
                                break Err(Error::UnexpectedByte(b).into());
                            }
                            id => id,
                        };
                        // println!("param_id: {:?}", param_id);
                        try_await_result!(skip_whitespaces(&mut self.input));
                        let b = try_await_result!(self.input.next());
                        if b'=' != b {
                            break Err(Error::UnexpectedByte(b).into());
                        }

                        let value = try_await!(parse_word_value(&mut self.input));
                        // println!("param_id: {:?}", value);

                        break Ok(GCode::ParameterSet(param_id, value));
                    }
                    // checksum support feature
                    #[cfg(feature = "parse-checksum")]
                    b'*' => {
                        let sum = self.input.sum() ^ b'*';
                        try_await_result!(skip_whitespaces(&mut self.input));
                        let (n, _) = try_await_result!(parse_number(&mut self.input));
                        // println!("{} {}", sum, n);
                        if n >= 256 {
                            break Err(Error::NumberOverflow.into());
                        } else if (n as u8) != sum {
                            break Err(Error::BadChecksum(sum).into());
                        } else {
                            try_await_result!(skip_whitespaces(&mut self.input));
                            #[cfg(not(feature = "parse-trailing-comment"))]
                            {
                                self.state = AsyncParserState::EndOfLine;
                            }
                            #[cfg(feature = "parse-trailing-comment")]
                            {
                                self.state = AsyncParserState::EoLOrTrailingComment;
                            }
                        }
                    }
                    // comment support features
                    #[cfg(not(feature = "parse-comments"))]
                    b'(' => {
                        try_await!(parse_inline_comment(&mut self.input));
                    }
                    #[cfg(feature = "parse-comments")]
                    b'(' => {
                        break Ok(GCode::Comment(try_await!(parse_inline_comment(
                            &mut self.input
                        ))));
                    }
                    #[cfg(all(
                        feature = "parse-trailing-comment",
                        not(feature = "parse-comments")
                    ))]
                    b';' => {
                        try_await_result!(parse_eol_comment(&mut self.input));
                        self.state = AsyncParserState::EndOfLine;
                    }
                    #[cfg(all(feature = "parse-trailing-comment", feature = "parse-comments"))]
                    b';' => {
                        let s = try_await!(parse_eol_comment(&mut self.input));
                        self.state = AsyncParserState::EndOfLine;
                        break Ok(GCode::Comment(s));
                    }
                    _ => break Err(Error::UnexpectedByte(b).into()),
                },
                #[cfg(all(
                    feature = "parse-trailing-comment",
                    any(feature = "parse-checksum", feature = "string-value")
                ))]
                AsyncParserState::EoLOrTrailingComment => {
                    #[cfg(not(feature = "parse-comments"))]
                    {
                        match b {
                            b';' => try_await_result!(parse_eol_comment(&mut self.input)),
                            _ => {
                                self.input.push_back(b);
                                break Ok(try_await!(self.parse_eol()));
                            }
                        }
                    }
                    #[cfg(feature = "parse-comments")]
                    {
                        match b {
                            b';' => {
                                let s = try_await!(parse_eol_comment(&mut self.input));
                                self.state = AsyncParserState::EndOfLine;
                                break Ok(GCode::Comment(s));
                            }
                            _ => {
                                self.input.push_back(b);
                                break Ok(try_await!(self.parse_eol()));
                            }
                        }
                    }
                },
                #[cfg(any(
                    feature = "parse-trailing-comment",
                    feature = "parse-checksum",
                    feature = "string-value"
                ))]
                AsyncParserState::EndOfLine => {
                    self.input.push_back(b);
                    break Ok(try_await!(self.parse_eol()));
                }
                AsyncParserState::ErrorRecovery => match b {
                    b'\r' | b'\n' => {
                        break Ok(self.end_line(b));
                    }
                    _ => {}
                },
            }
        };
        // eprintln!("{}:{} {:?}", file!(), line!(), res);
        if res.is_err() {
            self.state = AsyncParserState::ErrorRecovery;
        }
        Some(res)
    }
}
