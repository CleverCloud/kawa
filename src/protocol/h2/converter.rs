use crate::{
    protocol::utils::compare_no_case,
    storage::{
        AsBuffer, Block, BlockConverter, Chunk, Flags, Pair, Kawa, OutBlock, StatusLine, Store,
    },
};

pub struct H2BlockConverter;

impl<T: AsBuffer> BlockConverter<T> for H2BlockConverter {
    fn call(&mut self, block: Block, kawa: &mut Kawa<T>) {
        match block {
            Block::StatusLine => match kawa.detached.status_line.pop() {
                StatusLine::Request {
                    method,
                    authority,
                    path,
                    ..
                } => {
                    kawa.push_out(Store::Static(b"------------ PSEUDO HEADER\n"));
                    kawa.push_out(Store::Static(b":method: "));
                    kawa.push_out(method);
                    kawa.push_out(Store::Static(b"\n:authority: "));
                    kawa.push_out(authority);
                    kawa.push_out(Store::Static(b"\n:path: "));
                    kawa.push_out(path);
                    kawa.push_out(Store::Static(b"\n:scheme: http\n"));
                }
                StatusLine::Response { status, .. } => {
                    kawa.push_out(Store::Static(b"------------ PSEUDO HEADER\n"));
                    kawa.push_out(Store::Static(b":status: "));
                    kawa.push_out(status);
                    kawa.push_out(Store::Static(b"\n"));
                }
                StatusLine::Unknown => unreachable!(),
            },
            Block::Cookies => {
                if kawa.detached.jar.is_empty() {
                    return;
                }
                kawa.push_out(Store::Static(b"------------ HEADER"));
                for cookie in kawa.detached.jar.drain(..) {
                    kawa.out
                        .push_back(OutBlock::Store(Store::Static(b"\nCookies: ")));
                    kawa.out.push_back(OutBlock::Store(cookie.key));
                    kawa.out.push_back(OutBlock::Store(Store::Static(b"=")));
                    kawa.out.push_back(OutBlock::Store(cookie.val));
                }
                kawa.push_out(Store::Static(b"\n"));
            }
            Block::Header(Pair {
                key: Store::Empty, ..
            }) => {
                // elided header
            }
            Block::Header(Pair { key, val }) => {
                {
                    let key = key.data(kawa.storage.buffer());
                    let val = val.data(kawa.storage.buffer());
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
                kawa.push_out(Store::Static(b"------------ HEADER\n"));
                kawa.push_out(key);
                kawa.push_out(Store::Static(b": "));
                kawa.push_out(val);
                kawa.push_out(Store::Static(b"\n"));
            }
            Block::ChunkHeader(_) => {
                // this converter doesn't align H1 chunks on H2 data frames
            }
            Block::Chunk(Chunk { data }) => {
                kawa.push_out(Store::Static(b"------------ DATA\n"));
                kawa.push_out(data);
                kawa.push_out(Store::Static(b"\n"));
                kawa.push_delimiter()
            }
            Block::Flags(Flags {
                end_header,
                end_stream,
                ..
            }) => {
                if end_header {
                    kawa.push_out(Store::Static(b"------------ END HEADER\n"));
                }
                if end_stream {
                    kawa.push_out(Store::Static(b"------------ END STREAM\n"));
                }
                if end_header || end_stream {
                    kawa.push_delimiter()
                }
            }
        }
    }
}
