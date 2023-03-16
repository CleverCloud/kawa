pub mod buffer;
pub mod debug;
pub mod repr;

pub use buffer::{AsBuffer, HtxBuffer};
pub use debug::debug_htx;
pub use repr::{
    BodySize, Chunk, ChunkHeader, Flags, Header, Htx, HtxBlock, Kind, ParsingPhase, StatusLine,
    Store, Version,
};

pub trait HtxBlockConverter<T: AsBuffer> {
    fn initialize(&mut self, _htx: &mut Htx<T>) {}
    fn call(&mut self, block: HtxBlock, htx: &mut Htx<T>);
    fn finalize(&mut self, _htx: &mut Htx<T>) {}
}
