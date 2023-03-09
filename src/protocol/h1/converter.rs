use crate::htx::{
    Chunk, ChunkHeader, Flags, Header, Htx, HtxBlock, HtxBlockConverter, StatusLine, Store, Version,
};

pub struct BlockConverter;

impl Version {
    fn as_store(&self) -> Store {
        match self {
            Version::V10 => Store::Static(b"HTTP/1.0"),
            Version::V11 | Version::V20 => Store::Static(b"HTTP/1.1"),
        }
    }
}

impl HtxBlockConverter for BlockConverter {
    fn call(&mut self, block: HtxBlock, htx: &mut Htx) {
        match block {
            HtxBlock::StatusLine(StatusLine::Request {
                version,
                method,
                uri,
                authority,
                ..
            }) => {
                htx.push_out(method);
                htx.push_out(Store::Static(b" "));
                htx.push_out(uri);
                htx.push_out(Store::Static(b" "));
                htx.push_out(version.as_store());
                htx.push_out(Store::Static(b"\r\nHost: "));
                htx.push_out(authority);
                htx.push_out(Store::Static(b"\r\n"));
            }
            HtxBlock::StatusLine(StatusLine::Response {
                version,
                status,
                reason,
                ..
            }) => {
                htx.push_out(version.as_store());
                htx.push_out(Store::Static(b" "));
                htx.push_out(status);
                htx.push_out(Store::Static(b" "));
                htx.push_out(reason);
                htx.push_out(Store::Static(b"\r\n"));
            }
            HtxBlock::Header(Header {
                key: Store::Empty, ..
            }) => {
                // elided header
            }
            HtxBlock::Header(Header { key, val }) => {
                htx.push_out(key);
                htx.push_out(Store::Static(b": "));
                htx.push_out(val);
                htx.push_out(Store::Static(b"\r\n"));
            }
            HtxBlock::ChunkHeader(ChunkHeader { length }) => {
                htx.push_out(length);
                htx.push_out(Store::Static(b"\r\n"));
            }
            HtxBlock::Chunk(Chunk { data }) => {
                htx.push_out(data);
            }
            HtxBlock::Flags(Flags {
                end_header,
                end_chunk,
                ..
            }) => {
                if end_header || end_chunk {
                    htx.push_out(Store::Static(b"\r\n"));
                }
            }
        }
    }
}
