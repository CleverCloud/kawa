use std::io::Write;

use htx::HtxKind;

use crate::htx::{debug_htx, HTX};

mod htx;
mod protocol;

use protocol::h1;

fn test(htx_type: HtxKind, buf: &[u8]) {
    let buf = &mut buf.to_vec();
    let mut htx = HTX::new(htx_type);
    debug_htx(&htx, buf);

    h1::parse(&mut htx, buf);
    debug_htx(&htx, buf);

    htx.prepare(h1::block_converter);
    debug_htx(&htx, buf);

    let out = htx.as_io_slice(buf);
    println!("{out:?}");
    let mut writer = std::io::BufWriter::new(Vec::new());
    let result = writer.write_vectored(&out);
    println!("{result:?}");
    let push_left = htx.consume(result.unwrap());
    println!("{push_left:?}");

    let result = unsafe { std::str::from_utf8_unchecked(writer.buffer()) };
    println!("===============================\n{result}\n===============================");

    let request = unsafe { std::str::from_utf8_unchecked(buf) };
    println!("===============================\n{request}\n===============================");

    debug_htx(&htx, buf);
}

fn test_partial(htx_type: HtxKind, mut fragments: Vec<&[u8]>) {
    let mut buf = Vec::new();
    let mut writer = std::io::BufWriter::new(Vec::new());
    let mut htx = HTX::new(htx_type);

    while !fragments.is_empty() {
        let fragment = fragments.remove(0);
        buf.extend_from_slice(fragment);
        let request = unsafe { std::str::from_utf8_unchecked(&buf) };
        println!("===============================\n{request}\n===============================");
        debug_htx(&htx, &buf);

        h1::parse(&mut htx, &mut buf);
        debug_htx(&htx, &buf);

        htx.prepare(h1::block_converter);
        debug_htx(&htx, &buf);

        let out = htx.as_io_slice(&buf);
        println!("{out:?}");
        let result = writer.write_vectored(&out);
        println!("{result:?}");
        let push_left = htx.consume(result.unwrap());
        println!("{push_left:?}");

        buf.drain(..push_left);
        htx.push_left(push_left as u32);

        let result = unsafe { std::str::from_utf8_unchecked(writer.buffer()) };
        println!("===============================\n{result}\n===============================");
    }
}

fn test_partial_with_push(htx_type: HtxKind, mut fragments: Vec<&[u8]>) {
    let mut buf = Vec::new();
    let mut writer = std::io::BufWriter::new(Vec::new());
    let mut htx = HTX::new(htx_type);

    while !fragments.is_empty() {
        let fragment = fragments.remove(0);
        buf.extend_from_slice(fragment);

        let out = htx.as_io_slice(&buf);
        println!("{out:?}");
        let result = writer.write_vectored(&out);
        println!("{result:?}");
        let push_left = htx.consume(result.unwrap());
        let leftmost = htx.leftmost_ref();

        let result = unsafe { std::str::from_utf8_unchecked(writer.buffer()) };
        println!("===============================\n{result}\n===============================");

        println!("{push_left} {leftmost}");

        let request = unsafe { std::str::from_utf8_unchecked(&buf) };
        println!("===============================\n{request}\n===============================");

        let p1 = push_left / 2;
        let p2 = push_left - p1;

        debug_htx(&htx, &buf);

        h1::parse(&mut htx, &mut buf);
        debug_htx(&htx, &buf);

        buf.drain(..p1);
        htx.push_left(p1 as u32);
        debug_htx(&htx, &buf);

        htx.prepare(h1::block_converter);
        debug_htx(&htx, &buf);

        buf.drain(..p2);
        htx.push_left(p2 as u32);
        debug_htx(&htx, &buf);
    }
}

fn main() {
    test(
        HtxKind::Request,
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

    test_partial_with_push(
        HtxKind::Response,
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
