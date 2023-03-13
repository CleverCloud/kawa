use crate::htx::{Chunk, Flags, Header, Htx, HtxBlock, HtxBlockConverter, StatusLine, Store};

pub struct BlockConverter;

impl HtxBlockConverter for BlockConverter {
    fn call(&mut self, block: HtxBlock, htx: &mut Htx) {
        match block {
            HtxBlock::StatusLine(StatusLine::Request {
                method,
                authority,
                path,
                ..
            }) => {
                htx.push_out(Store::Static(b"------------ PSEUDO HEADER\n"));
                htx.push_out(Store::Static(b":method: "));
                htx.push_out(method);
                htx.push_out(Store::Static(b"\n:authority: "));
                htx.push_out(authority);
                htx.push_out(Store::Static(b"\n:path: "));
                htx.push_out(path);
                htx.push_out(Store::Static(b"\n:scheme: http\n"));
            }
            HtxBlock::StatusLine(StatusLine::Response { status, .. }) => {
                htx.push_out(Store::Static(b"------------ PSEUDO HEADER\n"));
                htx.push_out(Store::Static(b":status: "));
                htx.push_out(status);
                htx.push_out(Store::Static(b"\n"));
            }
            HtxBlock::Header(Header {
                key: Store::Empty, ..
            }) => {
                // elided header
            }
            HtxBlock::Header(Header { key, val }) => {
                htx.push_out(Store::Static(b"------------ HEADER\n"));
                htx.push_out(key);
                htx.push_out(Store::Static(b": "));
                htx.push_out(val);
                htx.push_out(Store::Static(b"\n"));
            }
            HtxBlock::ChunkHeader(_) => {
                // this converter doesn't align H1 chunks on H2 data frames
            }
            HtxBlock::Chunk(Chunk { data }) => {
                htx.push_out(Store::Static(b"------------ DATA\n"));
                htx.push_out(data);
                htx.push_out(Store::Static(b"\n"));
                htx.push_delimiter()
            }
            HtxBlock::Flags(Flags {
                end_header,
                end_stream,
                ..
            }) => {
                if end_header {
                    htx.push_out(Store::Static(b"------------ END HEADER\n"));
                }
                if end_stream {
                    htx.push_out(Store::Static(b"------------ END STREAM\n"));
                }
                if end_header || end_stream {
                    htx.push_delimiter()
                }
            }
        }
    }
}
