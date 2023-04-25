use crate::{
    protocol::utils::compare_no_case,
    storage::{
        AsBuffer, Chunk, Flags, Header, Htx, HtxBlock, HtxBlockConverter, OutBlock, StatusLine,
        Store,
    },
};

pub struct BlockConverter;

impl<T: AsBuffer> HtxBlockConverter<T> for BlockConverter {
    fn call(&mut self, block: HtxBlock, htx: &mut Htx<T>) {
        match block {
            HtxBlock::StatusLine => match htx.detached.status_line.pop() {
                StatusLine::Request {
                    method,
                    authority,
                    path,
                    ..
                } => {
                    htx.push_out(Store::Static(b"------------ PSEUDO HEADER\n"));
                    htx.push_out(Store::Static(b":method: "));
                    htx.push_out(method);
                    htx.push_out(Store::Static(b"\n:authority: "));
                    htx.push_out(authority);
                    htx.push_out(Store::Static(b"\n:path: "));
                    htx.push_out(path);
                    htx.push_out(Store::Static(b"\n:scheme: http\n"));
                }
                StatusLine::Response { status, .. } => {
                    htx.push_out(Store::Static(b"------------ PSEUDO HEADER\n"));
                    htx.push_out(Store::Static(b":status: "));
                    htx.push_out(status);
                    htx.push_out(Store::Static(b"\n"));
                }
                StatusLine::Unknown => unreachable!(),
            },
            HtxBlock::Cookies => {
                if htx.detached.jar.is_empty() {
                    return;
                }
                htx.push_out(Store::Static(b"------------ HEADER"));
                for cookie in htx.detached.jar.drain(..) {
                    htx.out
                        .push_back(OutBlock::Store(Store::Static(b"\nCookies: ")));
                    htx.out.push_back(OutBlock::Store(cookie));
                }
                htx.push_out(Store::Static(b"\n"));
            }
            HtxBlock::Header(Header {
                key: Store::Empty, ..
            }) => {
                // elided header
            }
            HtxBlock::Header(Header { key, val }) => {
                {
                    let key = key.data(htx.storage.buffer());
                    let val = val.data(htx.storage.buffer());
                    if compare_no_case(key, b"connection")
                        || compare_no_case(key, b"host")
                        || compare_no_case(key, b"http2-settings")
                        || compare_no_case(key, b"keep-alive")
                        || compare_no_case(key, b"proxy-connection")
                        || compare_no_case(key, b"te") && !compare_no_case(val, b"trailers")
                        || compare_no_case(key, b"trailer")
                        || compare_no_case(key, b"transfer-encoding")
                        || compare_no_case(key, b"upgrade")
                    {
                        return;
                    }
                }
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
