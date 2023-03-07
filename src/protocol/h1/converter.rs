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
        let out = &mut htx.out;
        match block {
            HtxBlock::StatusLine(StatusLine::Request {
                version,
                method,
                uri,
                authority,
                ..
            }) => {
                out.push_back(method);
                out.push_back(Store::Static(b" "));
                out.push_back(uri);
                out.push_back(Store::Static(b" "));
                out.push_back(version.as_store());
                out.push_back(Store::Static(b"\r\nHost: "));
                out.push_back(authority);
                out.push_back(Store::Static(b"\r\n"));
            }
            HtxBlock::StatusLine(StatusLine::Response {
                version,
                status,
                reason,
                ..
            }) => {
                out.push_back(version.as_store());
                out.push_back(Store::Static(b" "));
                out.push_back(status);
                out.push_back(Store::Static(b" "));
                out.push_back(reason);
                out.push_back(Store::Static(b"\r\n"));
            }
            HtxBlock::Header(Header {
                key: Store::Empty, ..
            }) => {
                // elided header
            }
            HtxBlock::Header(Header { key, val }) => {
                out.push_back(key);
                out.push_back(Store::Static(b": "));
                out.push_back(val);
                out.push_back(Store::Static(b"\r\n"));
            }
            HtxBlock::ChunkHeader(ChunkHeader { length }) => {
                out.push_back(length);
                out.push_back(Store::Static(b"\r\n"));
            }
            HtxBlock::Chunk(Chunk { data }) => {
                out.push_back(data);
            }
            HtxBlock::Flags(Flags {
                end_header,
                end_chunk,
                ..
            }) => {
                if end_header || end_chunk {
                    out.push_back(Store::Static(b"\r\n"));
                }
            }
        }
    }
}
