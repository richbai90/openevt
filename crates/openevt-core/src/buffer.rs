//! Recyclable buffers shared by event-stream implementations.

use crossbeam::channel::Sender;
use std::ops::Deref;

/// A wrapper that returns its inner vector to a recycling channel when dropped.
pub struct PooledBuffer<T> {
    /// The underlying vector. Wrapped in an Option so it can be taken out during Drop.
    pub buffer: Option<Vec<T>>,
    /// The channel used to return the vector to the pool for reuse.
    pub return_channel: Sender<Vec<T>>,
}

/// Implements `Deref` to allow `PooledBuffer` to be treated transparently as a `&Vec<T>`.
impl<T> Deref for PooledBuffer<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        self.buffer
            .as_ref()
            .expect("Buffer is always Some until Drop")
    }
}

/// Automatically returns the cleared vector to the recycling channel when dropped.
impl<T> Drop for PooledBuffer<T> {
    fn drop(&mut self) {
        // Take ownership of the buffer out of the Option
        if let Some(mut buf) = self.buffer.take() {
            buf.clear(); // Reset length to 0, but retain capacity
            // Try to return to the pool. If the pool is full or dropped, let it deallocate.
            let _ = self.return_channel.try_send(buf);
        }
    }
}
