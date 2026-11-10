//! Helpers for the tests, so that they run with either stream interface (`future-stream` or
//! this crate's `ByteStream` trait).

use crate::stream::{ByteStream, UnpinTrait};

#[cfg(feature = "future-stream")]
use futures::StreamExt;

/// A stream that yields the items of an iterator.
#[cfg(feature = "future-stream")]
pub(crate) fn stream_of<I: Iterator>(items: I) -> futures::stream::Iter<I> {
    futures::stream::iter(items)
}

/// A stream that yields the items of an iterator.
#[cfg(not(feature = "future-stream"))]
pub(crate) fn stream_of<I: Iterator>(items: I) -> IterStream<I> {
    IterStream(items)
}

#[cfg(not(feature = "future-stream"))]
pub(crate) struct IterStream<I>(I);

#[cfg(not(feature = "future-stream"))]
impl<I: Iterator> ByteStream for IterStream<I> {
    type Item = I::Item;

    async fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

    async fn recovery_check(&mut self) {}
}

/// Takes up to `n` items from the stream.
pub(crate) fn take<S: ByteStream + UnpinTrait>(stream: &mut S, n: usize) -> Vec<S::Item> {
    futures_executor::block_on(async {
        let mut items = Vec::new();
        while items.len() < n {
            match stream.next().await {
                Some(item) => items.push(item),
                None => break,
            }
        }
        items
    })
}

/// Takes all the items of the stream.
pub(crate) fn collect<S: ByteStream + UnpinTrait>(stream: &mut S) -> Vec<S::Item> {
    take(stream, usize::MAX)
}
