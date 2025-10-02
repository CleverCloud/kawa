mod protocol;
mod storage;

pub use protocol::{h1, h2};
pub use storage::*;

pub struct SliceBuffer<'a>(pub &'a mut [u8]);

impl crate::AsBuffer for SliceBuffer<'_> {
    fn as_buffer(&self) -> &[u8] {
        self.0
    }
    fn as_mut_buffer(&mut self) -> &mut [u8] {
        self.0
    }
}
