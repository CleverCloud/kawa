pub mod buffer;
pub mod debug;
pub mod repr;
pub mod vecdeque;

pub use buffer::{AsBuffer, Buffer};
pub use debug::debug_kawa;
pub use repr::{
    Block, BodySize, Chunk, ChunkHeader, Flags, Kawa, Kind, OutBlock, Pair, ParsingErrorKind,
    ParsingPhase, ParsingPhaseMarker, StatusLine, Store, Version,
};
pub use vecdeque::VecDeque;

pub trait BlockConverter<T: AsBuffer> {
    fn initialize(&mut self, _kawa: &mut Kawa<T>) {}
    fn call(&mut self, block: Block, kawa: &mut Kawa<T>) -> bool;
    fn finalize(&mut self, _kawa: &mut Kawa<T>) {}
}
