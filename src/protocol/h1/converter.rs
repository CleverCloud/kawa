use crate::htx::{
    Chunk, ChunkHeader, Flags, Header, HtxBlock, HtxBodySize, StatusLine, Store, Version,
};

pub fn block_converter(_: &HtxBodySize, block: HtxBlock, out: &mut Vec<Store>) {
    match block {
        HtxBlock::StatusLine(StatusLine::Request {
            version,
            method,
            uri,
            authority,
            ..
        }) => {
            let version = match version {
                Version::V10 => b"HTTP/1.0",
                Version::V11 | Version::V20 => b"HTTP/1.1",
            };
            out.push(method);
            out.push(Store::Static(b" "));
            out.push(uri);
            out.push(Store::Static(b" "));
            out.push(Store::Static(version));
            out.push(Store::Static(b"\r\nHost: "));
            out.push(authority);
            out.push(Store::Static(b"\r\n"));
        }
        HtxBlock::StatusLine(StatusLine::Response {
            version,
            status,
            reason,
            ..
        }) => {
            let version = match version {
                Version::V10 => b"HTTP/1.0",
                Version::V11 | Version::V20 => b"HTTP/1.1",
            };
            out.push(Store::Static(version));
            out.push(Store::Static(b" "));
            out.push(status);
            out.push(Store::Static(b" "));
            out.push(reason);
            out.push(Store::Static(b"\r\n"));
        }
        HtxBlock::Header(Header {
            key: Store::Empty, ..
        }) => {}
        HtxBlock::Header(Header { key, val }) => {
            out.push(key);
            out.push(Store::Static(b": "));
            out.push(val);
            out.push(Store::Static(b"\r\n"));
        }
        HtxBlock::ChunkHeader(ChunkHeader { length }) => {
            out.push(length);
            out.push(Store::Static(b"\r\n"));
        }
        HtxBlock::Chunk(Chunk { data, .. }) => {
            out.push(data);
        }
        HtxBlock::Flags(Flags {
            end_header,
            end_chunk,
            ..
        }) => {
            if end_header || end_chunk {
                out.push(Store::Static(b"\r\n"));
            }
        }
    }
}
