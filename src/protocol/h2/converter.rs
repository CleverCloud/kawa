use crate::htx::{Chunk, Flags, Header, Htx, HtxBlock, HtxBlockConverter, StatusLine, Store};

pub struct BlockConverter;

impl HtxBlockConverter for BlockConverter {
    fn call(&mut self, block: HtxBlock, htx: &mut Htx) {
        let out = &mut htx.out;
        match block {
            HtxBlock::StatusLine(StatusLine::Request {
                method,
                authority,
                path,
                ..
            }) => {
                out.push_back(Store::Static(b"------------ PSEUDO HEADER\n"));
                out.push_back(Store::Static(b":method: "));
                out.push_back(method);
                out.push_back(Store::Static(b"\n:authority: "));
                out.push_back(authority);
                out.push_back(Store::Static(b"\n:path: "));
                out.push_back(path);
                out.push_back(Store::Static(b"\n:scheme: http\n"));
            }
            HtxBlock::StatusLine(StatusLine::Response { status, .. }) => {
                out.push_back(Store::Static(b"------------ PSEUDO HEADER\n"));
                out.push_back(Store::Static(b":status: "));
                out.push_back(status);
                out.push_back(Store::Static(b"\n"));
            }
            HtxBlock::Header(Header {
                key: Store::Empty, ..
            }) => {
                // elided header
            }
            HtxBlock::Header(Header { key, val }) => {
                out.push_back(Store::Static(b"------------ HEADER\n"));
                out.push_back(key);
                out.push_back(Store::Static(b": "));
                out.push_back(val);
                out.push_back(Store::Static(b"\n"));
            }
            HtxBlock::ChunkHeader(_) => {
                // this converter doesn't align H1 chunks on H2 data frames
            }
            HtxBlock::Chunk(Chunk { data }) => {
                out.push_back(Store::Static(b"------------ DATA\n"));
                out.push_back(data);
                out.push_back(Store::Static(b"\n"));
            }
            HtxBlock::Flags(Flags {
                end_header,
                end_stream,
                ..
            }) => {
                if end_header {
                    out.push_back(Store::Static(b"------------ END HEADER\n"));
                }
                if end_stream {
                    out.push_back(Store::Static(b"------------ END STREAM\n"));
                }
            }
        }
    }
}
