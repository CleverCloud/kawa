use std::io::Write;

mod protocol;
mod storage;

use protocol::{h1, h2};
use storage::{debug_htx, AsBuffer, Htx, HtxBlockConverter, HtxBuffer, Kind};

impl AsBuffer for &mut [u8] {
    fn as_buffer(&self) -> &[u8] {
        self
    }
    fn as_mut_buffer(&mut self) -> &mut [u8] {
        self
    }
}

fn test_with_converter<T: AsBuffer, C: HtxBlockConverter<T>>(
    htx_kind: Kind,
    storage: HtxBuffer<T>,
    fragment: &[u8],
    converter: &mut C,
) -> T {
    let mut htx = Htx::new(htx_kind, storage);
    let _ = htx.storage.write(fragment).expect("WRITE");
    debug_htx(&htx);

    h1::parse(&mut htx, &mut h1::NoCallbacks);
    debug_htx(&htx);

    htx.prepare(converter);
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
    htx.storage.buffer
}
fn test<T: AsBuffer>(htx_kind: Kind, storage: T, fragment: &[u8]) -> T {
    let htx_buffer = HtxBuffer::new(storage);
    let storage = test_with_converter(htx_kind, htx_buffer, fragment, &mut h1::BlockConverter);
    let htx_buffer = HtxBuffer::new(storage);
    let storage = test_with_converter(htx_kind, htx_buffer, fragment, &mut h2::BlockConverter);
    storage
}

fn test_partial_with_converter<T: AsBuffer, C: HtxBlockConverter<T>>(
    htx_kind: Kind,
    storage: HtxBuffer<T>,
    mut fragments: Vec<&[u8]>,
    converter: &mut C,
) -> T {
    let mut writer = std::io::BufWriter::new(Vec::new());
    let mut htx = Htx::new(htx_kind, storage);

    while !fragments.is_empty() {
        let fragment = fragments.remove(0);
        let _ = htx.storage.write(fragment).expect("WRITE");

        let buffer = unsafe { std::str::from_utf8_unchecked(htx.storage.used()) };
        println!("===============================\n{buffer}\n===============================");
        debug_htx(&htx);

        h1::parse(&mut htx, &mut h1::NoCallbacks);
        debug_htx(&htx);

        htx.prepare(converter);
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
    htx.storage.buffer
}
fn test_partial<T: AsBuffer>(htx_kind: Kind, storage: T, fragments: Vec<&[u8]>) -> T {
    let storage = test_partial_with_converter(
        htx_kind,
        HtxBuffer::new(storage),
        fragments.clone(),
        &mut h1::BlockConverter,
    );
    let storage = test_partial_with_converter(
        htx_kind,
        HtxBuffer::new(storage),
        fragments,
        &mut h2::BlockConverter,
    );
    storage
}

fn main() {
    let mut buffer = vec![0; 512];
    test(
        Kind::Request,
        &mut buffer[..],
        b"CONNECT www.example.com:80 HTTP/1.1\r\nTE: lol\r\nTE: trailers\r\n\r\n",
    );

    test(
        Kind::Request,
        &mut buffer[..],
        b"POST /cgi-bin/process.cgi HTTP/1.1\r
User-Agent: Mozilla/4.0 (compatible; MSIE5.01; Windows NT)\r
Host: www.tutorialspoint.com\r
Content-Type: application/x-www-form-urlencoded\r
Content-Length: 49\r
Cookies: crumb1\r
Accept-Language: en-us\r
Accept-Encoding: gzip, deflate\r
Connection: Keep-Alive\r
Cookies: crumb2; crumb3\r
\r
licenseID=string&content=string&/paramsXML=string",
    );

    test(
        Kind::Response,
        &mut buffer[..128],
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
        Kind::Response,
        &mut buffer[..128],
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
