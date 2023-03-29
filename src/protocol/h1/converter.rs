use crate::storage::{
    AsBuffer, Chunk, ChunkHeader, Flags, Header, Htx, HtxBlock, HtxBlockConverter, OutBlock,
    StatusLine, Store, Version,
};

pub struct BlockConverter;

impl Version {
    fn as_store(&self) -> Store {
        match self {
            Version::V10 => Store::Static(b"HTTP/1.0"),
            Version::V11 | Version::V20 => Store::Static(b"HTTP/1.1"),
            Version::Unknown => unreachable!(),
        }
    }
}

impl<T: AsBuffer> HtxBlockConverter<T> for BlockConverter {
    fn call(&mut self, block: HtxBlock, htx: &mut Htx<T>) {
        match block {
            HtxBlock::StatusLine => match htx.detached.status_line.clone() {
                StatusLine::Request {
                    version,
                    method,
                    uri,
                    authority,
                    ..
                } => {
                    htx.push_out(method);
                    htx.push_out(Store::Static(b" "));
                    htx.push_out(uri);
                    htx.push_out(Store::Static(b" "));
                    htx.push_out(version.as_store());
                    htx.push_out(Store::Static(b"\r\nHost: "));
                    htx.push_out(authority);
                    htx.push_out(Store::Static(b"\r\n"));
                }
                StatusLine::Response {
                    version,
                    status,
                    reason,
                    ..
                } => {
                    htx.push_out(version.as_store());
                    htx.push_out(Store::Static(b" "));
                    htx.push_out(status);
                    htx.push_out(Store::Static(b" "));
                    htx.push_out(reason);
                    htx.push_out(Store::Static(b"\r\n"));
                }
            },
            HtxBlock::Cookies => {
                if htx.detached.jar.is_empty() {
                    return;
                }
                htx.push_out(Store::Static(b"Cookies: "));
                for cookie in htx.detached.jar.drain(..) {
                    htx.out.push_back(OutBlock::Store(cookie));
                    htx.out.push_back(OutBlock::Store(Store::Static(b"; ")));
                }
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
                end_body,
                end_chunk,
                end_header,
                ..
            }) => {
                if htx.is_streaming() && end_body {
                    htx.push_out(Store::Static(b"0\r\n"));
                }
                if end_header || end_chunk {
                    htx.push_out(Store::Static(b"\r\n"));
                }
            }
        }
    }
}
