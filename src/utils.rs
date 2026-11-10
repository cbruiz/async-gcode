use crate::stream::{ByteStream, PushBackable, UnpinTrait};

#[cfg(feature = "future-stream")]
use futures::StreamExt;

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
