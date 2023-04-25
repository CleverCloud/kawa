pub mod buffer;
pub mod debug;
pub mod repr;

pub use buffer::{AsBuffer, Buffer};
pub use debug::debug_kawa;
pub use repr::{
    Block, BodySize, Chunk, ChunkHeader, Flags, Header, Kawa, Kind, OutBlock, ParsingPhase,
    StatusLine, Store, Version,
};

pub trait BlockConverter<T: AsBuffer> {
    fn initialize(&mut self, _kawa: &mut Kawa<T>) {}
    fn call(&mut self, block: Block, kawa: &mut Kawa<T>);
    fn finalize(&mut self, _kawa: &mut Kawa<T>) {}
}
