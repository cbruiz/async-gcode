use crate::stream::{ByteStream, PushBackable, UnpinTrait};

#[cfg(feature = "future-stream")]
use futures::StreamExt;

#[cfg(all(
    not(feature = "std"),
    any(feature = "parse-comments", feature = "string-value")
))]
use alloc::string::String;
#[cfg(all(
    not(feature = "std"),
    any(
        feature = "parse-comments",
        feature = "string-value",
        feature = "parse-expressions"
    )
))]
use alloc::vec::Vec;

/// Pushes an item of an expression to `vec`, which holds at most [`crate::MAX_EXPRESSION_ITEMS`].
///
/// A full heap is an error instead of a panic (or an abort) of the allocation error handler. The
/// fast path (room left in the allocation) costs the same as a plain `push`: its `len == capacity`
/// test is the one `push` does anyway.
#[cfg(feature = "parse-expressions")]
pub(crate) fn push_expression_item<T>(vec: &mut Vec<T>, item: T) -> Result<(), crate::Error> {
    let len = vec.len();
    if len >= crate::MAX_EXPRESSION_ITEMS {
        return Err(crate::Error::InvalidExpression);
    }
    if len == vec.capacity() && vec.try_reserve(1).is_err() {
        return Err(crate::Error::OutOfMemory);
    }
    vec.push(item);
    Ok(())
}

/// The bytes of a string, a text or a comment, read up to their end whatever their length.
///
/// At most [`crate::MAX_TEXT_LENGTH`] bytes are stored, so that an arbitrarily long (or
/// unterminated) one cannot exhaust a small heap. Once it is too long, or the heap is full, the
/// following bytes are dropped and [`TextBuffer::into_string`] reports the error: the caller keeps
/// on reading up to the end, so that the parser stays in sync with the input.
///
/// Zero-cost compared to the plain `Vec` it replaces: no more RAM in the async state (the flag
/// fits in its padding), the same allocations, and about the same code (`opt-level = "z"`). The
/// inlining attributes are measured choices for that: keep them.
#[cfg(any(feature = "parse-comments", feature = "string-value"))]
pub(crate) struct TextBuffer {
    bytes: Vec<u8>,
    /// A byte was dropped: the text is too long when it reached the limit, otherwise the heap
    /// was full (the failure to grow happens below the limit only).
    failed: bool,
}

#[cfg(any(feature = "parse-comments", feature = "string-value"))]
impl TextBuffer {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            failed: false,
        }
    }

    /// Out of line: a single copy of the (fallible) growth, whatever the number of callers.
    #[inline(never)]
    pub(crate) fn push(&mut self, b: u8) {
        // `try_reserve` only allocates when the capacity is reached: then `push` does not.
        if self.failed
            || self.bytes.len() >= crate::MAX_TEXT_LENGTH
            || self.bytes.try_reserve(1).is_err()
        {
            self.failed = true;
        } else {
            self.bytes.push(b);
        }
    }

    /// The string, without any new allocation. Inlined: the buffer is then dropped by the drop
    /// code the async state has anyway.
    #[inline]
    pub(crate) fn into_string(self) -> Result<String, crate::Error> {
        if self.failed {
            Err(if self.bytes.len() >= crate::MAX_TEXT_LENGTH {
                crate::Error::TextTooLong
            } else {
                crate::Error::OutOfMemory
            })
        } else {
            String::from_utf8(self.bytes).map_err(|_| crate::Error::InvalidUTF8String)
        }
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! try_parse {
    ($input:expr) => {
        match $input.await? {
            ParseResult::Ok(b) => b,
            ParseResult::Input(e) => return Some(ParseResult::Input(e)),
            ParseResult::Parsing(e) => return Some(ParseResult::Parsing(e)),
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! try_result {
    ($input:expr) => {
        match $input.await? {
            Ok(b) => b,
            Err(e) => return Some(ParseResult::Input(e)),
        }
    };
}

/// Consumes spaces and returns the first byte that is not a space (consumed too).
pub(crate) async fn next_non_space<S, E>(input: &mut S) -> Option<Result<u8, E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
{
    loop {
        match input.next().await? {
            Ok(b' ') => {}
            other => return Some(other),
        }
    }
}

/// Consumes spaces and pushes back the first byte that is not a space.
#[cfg(any(
    feature = "string-value",
    feature = "parse-checksum",
    feature = "parse-parameters",
    feature = "parse-expressions"
))]
pub(crate) async fn skip_whitespaces<S, E>(input: &mut S) -> Option<Result<(), E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
{
    match next_non_space(input).await? {
        Ok(b) => {
            input.push_back(b);
            Some(Ok(()))
        }
        Err(e) => Some(Err(e)),
    }
}

#[cfg(all(
    test,
    any(
        feature = "string-value",
        feature = "parse-checksum",
        feature = "parse-parameters",
        feature = "parse-expressions"
    )
))]
mod test {
    use super::skip_whitespaces;
    use crate::test_utils::{stream_of, take};

    #[test]
    fn skips_white_spaces_and_pushes_back_the_first_non_space_byte() {
        #[cfg(not(feature = "parse-checksum"))]
        use crate::stream::pushback::PushBack;
        #[cfg(feature = "parse-checksum")]
        use crate::stream::xorsum_pushback::XorSumPushBack as PushBack;

        let iter = stream_of(
            b"      d"
                .iter()
                .copied()
                .map(Result::<_, core::convert::Infallible>::Ok),
        );

        #[cfg(not(feature = "parse-checksum"))]
        let mut data = PushBack::new(iter);
        #[cfg(feature = "parse-checksum")]
        let mut data = PushBack::new(iter, 0);

        assert_eq!(
            futures_executor::block_on(skip_whitespaces(&mut data)),
            Some(Ok(()))
        );
        assert_eq!(take(&mut data, 1), [Ok(b'd')]);
    }
}
