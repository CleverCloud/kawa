pub mod debug;
pub mod repr;
pub mod storage;

pub use debug::debug_htx;
pub use repr::{
    Chunk, ChunkHeader, Flags, Header, Htx, HtxBlock, HtxBodySize, HtxKind, HtxParsingPhase,
    StatusLine, Store, Version,
};
pub use storage::HtxBuffer;

pub trait HtxBlockConverter {
    fn initialize(&mut self, _htx: &mut Htx) {}
    fn call(&mut self, block: HtxBlock, htx: &mut Htx);
    fn finalize(&mut self, _htx: &mut Htx) {}
}
