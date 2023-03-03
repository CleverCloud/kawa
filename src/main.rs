use std::io::Write;

mod htx;
mod protocol;

use htx::{debug_htx, Htx, HtxBuffer, HtxKind};
use protocol::h1;

fn test(htx_type: HtxKind, storage: HtxBuffer, fragment: &[u8]) {
    let mut htx = Htx::new(htx_type, storage);
    let _ = htx.storage.write(fragment).expect("WRITE");
    debug_htx(&htx);

    h1::parse(&mut htx);
    debug_htx(&htx);

    htx.prepare(h1::block_converter);
    debug_htx(&htx);

    let out = htx.as_io_slice();
    println!("{out:?}");
    let mut writer = std::io::BufWriter::new(Vec::new());
    let amount = writer.write_vectored(&out).expect("WRITE");
    let result = unsafe { std::str::from_utf8_unchecked(writer.buffer()) };
    println!("===============================\n{result}\n===============================");

    let buffer = unsafe { std::str::from_utf8_unchecked(htx.storage.used()) };
    println!("===============================\n{buffer}\n===============================");

    htx.consume(amount);
    println!("{amount}");
    debug_htx(&htx);
}

fn test_partial(htx_type: HtxKind, storage: HtxBuffer, mut fragments: Vec<&[u8]>) {
    let mut writer = std::io::BufWriter::new(Vec::new());
    let mut htx = Htx::new(htx_type, storage);

    while !fragments.is_empty() {
        let fragment = fragments.remove(0);
        let _ = htx.storage.write(fragment).expect("WRITE");

        let buffer = unsafe { std::str::from_utf8_unchecked(htx.storage.used()) };
        println!("===============================\n{buffer}\n===============================");
        debug_htx(&htx);

        h1::parse(&mut htx);
        debug_htx(&htx);

        htx.prepare(h1::block_converter);
        debug_htx(&htx);

        let out = htx.as_io_slice();
        println!("{out:?}");
        let amount = writer.write_vectored(&out).expect("WRITE");
        println!("{amount:?}");
        htx.consume(amount);

        let result = unsafe { std::str::from_utf8_unchecked(writer.buffer()) };
        println!("===============================\n{result}\n===============================");
    }
    debug_htx(&htx);
}

fn main() {
    let mut buffer = vec![0; 512];
    test(
        HtxKind::Request,
        HtxBuffer::new(&mut buffer),
        b"POST /cgi-bin/process.cgi HTTP/1.1\r
User-Agent: Mozilla/4.0 (compatible; MSIE5.01; Windows NT)\r
Host: www.tutorialspoint.com\r
Content-Type: application/x-www-form-urlencoded\r
Content-Length: 49\r
Accept-Language: en-us\r
Accept-Encoding: gzip, deflate\r
Connection: Keep-Alive\r
\r
licenseID=string&content=string&/paramsXML=string",
    );

    test(
        HtxKind::Response,
        HtxBuffer::new(&mut buffer[..128]),
        b"HTTP/1.1 200 OK\r
Transfer-Encoding: chunked\r
Connection: Keep-Alive\r
Trailer: Foo\r
\r
4\r
Wiki\r
5\r
pedia\r
0\r
Foo: bar\r
\r
",
    );

    test_partial(
        HtxKind::Response,
        HtxBuffer::new(&mut buffer[..128]),
        vec![
            b"HTTP/1.1 200 OK\r
Transfer-Encoding: chunked\r
Connection: Keep-Alive\r
Trailer: Foo\r
\r
4",
            b"\r
Wi",
            b"ki\r
5\r
pedia\r
0",
            b"\r
Foo: bar\r
\r
",
        ],
    );
}
