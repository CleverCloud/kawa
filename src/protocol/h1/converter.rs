use crate::storage::{
    AsBuffer, Block, BlockConverter, Chunk, ChunkHeader, Flags, Header, Kawa, OutBlock, StatusLine,
    Store, Version,
};

pub struct H1BlockConverter;

impl Version {
    fn as_store(&self) -> Store {
        match self {
            Version::V10 => Store::Static(b"HTTP/1.0"),
            Version::V11 | Version::V20 => Store::Static(b"HTTP/1.1"),
            Version::Unknown => unreachable!(),
        }
    }
}

impl<T: AsBuffer> BlockConverter<T> for H1BlockConverter {
    fn call(&mut self, block: Block, kawa: &mut Kawa<T>) {
        match block {
            Block::StatusLine => match kawa.detached.status_line.pop() {
                StatusLine::Request {
                    version,
                    method,
                    uri,
                    authority,
                    ..
                } => {
                    kawa.push_out(method);
                    kawa.push_out(Store::Static(b" "));
                    kawa.push_out(uri);
                    kawa.push_out(Store::Static(b" "));
                    kawa.push_out(version.as_store());
                    kawa.push_out(Store::Static(b"\r\nHost: "));
                    kawa.push_out(authority);
                    kawa.push_out(Store::Static(b"\r\n"));
                }
                StatusLine::Response {
                    version,
                    status,
                    reason,
                    ..
                } => {
                    kawa.push_out(version.as_store());
                    kawa.push_out(Store::Static(b" "));
                    kawa.push_out(status);
                    kawa.push_out(Store::Static(b" "));
                    kawa.push_out(reason);
                    kawa.push_out(Store::Static(b"\r\n"));
                }
                StatusLine::Unknown => unreachable!(),
            },
            Block::Cookies => {
                if kawa.detached.jar.is_empty() {
                    return;
                }
                kawa.push_out(Store::Static(b"Cookies: "));
                for cookie in kawa.detached.jar.drain(..) {
                    kawa.out.push_back(OutBlock::Store(cookie));
                    kawa.out.push_back(OutBlock::Store(Store::Static(b"; ")));
                }
                kawa.push_out(Store::Static(b"\r\n"));
            }
            Block::Header(Header {
                key: Store::Empty, ..
            }) => {
                // elided header
            }
            Block::Header(Header { key, val }) => {
                kawa.push_out(key);
                kawa.push_out(Store::Static(b": "));
                kawa.push_out(val);
                kawa.push_out(Store::Static(b"\r\n"));
            }
            Block::ChunkHeader(ChunkHeader { length }) => {
                kawa.push_out(length);
                kawa.push_out(Store::Static(b"\r\n"));
            }
            Block::Chunk(Chunk { data }) => {
                kawa.push_out(data);
            }
            Block::Flags(Flags {
                end_body,
                end_chunk,
                end_header,
                ..
            }) => {
                if kawa.is_streaming() && end_body {
                    kawa.push_out(Store::Static(b"0\r\n"));
                }
                if end_header || end_chunk {
                    kawa.push_out(Store::Static(b"\r\n"));
                }
            }
        }
    }
}
