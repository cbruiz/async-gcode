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

pub(crate) async fn skip_whitespaces<S, E>(input: &mut S) -> Option<Result<(), E>>
where
    S: ByteStream<Item = Result<u8, E>> + PushBackable<Item = u8> + UnpinTrait,
{
    loop {
        let b = match input.next().await? {
            Ok(b) => b,
            Err(e) => return Some(Err(e)),
        };
        if b != b' ' {
            input.push_back(b);
            break;
        }
    }
    Some(Ok(()))
}

#[cfg(test)]
mod test {
    use super::{skip_whitespaces};
    use futures::stream;
    use futures::StreamExt;

    #[test]
    fn skips_white_spaces_and_pushes_back_the_first_non_space_byte() {

        #[cfg(not(feature = "parse-checksum"))]
        use crate::stream::pushback::PushBack as PushBack;
        #[cfg(feature = "parse-checksum")]
        use crate::stream::xorsum_pushback::XorSumPushBack as PushBack;

        let iter = stream::iter(
            b"      d"
                .iter()
                .copied()
                .map(Result::<_, core::convert::Infallible>::Ok),
        );

        #[cfg(not(feature = "parse-checksum"))]
        let mut data = PushBack::new(iter);
        #[cfg(feature = "parse-checksum")]
        let mut data = PushBack::new(iter, 0);

        futures_executor::block_on(async move {
            skip_whitespaces(&mut data).await;
            assert_eq!(Some(Ok(b'd')), data.next().await);
        });
    }
}
