use std::io::Write;

use kawa::{debug_kawa, h1, h2, AsBuffer, BlockConverter, Buffer, Kawa, Kind, SliceBuffer};

fn test_with_converter<T: AsBuffer, C: BlockConverter<T>>(
    kind: Kind,
    storage: Buffer<T>,
    fragment: &[u8],
    converter: &mut C,
) -> T {
    println!("////////////////////////////////////////");
    let mut kawa = Kawa::new(kind, storage);
    let _ = kawa.storage.write(fragment).expect("WRITE");
    debug_kawa(&kawa);

    h1::parse(&mut kawa, &mut h1::NoCallbacks);
    debug_kawa(&kawa);

    kawa.prepare(converter);
    debug_kawa(&kawa);

    let out = kawa.as_io_slice();
    println!("{out:?}");
    let mut writer = std::io::BufWriter::new(Vec::new());
    let amount = writer.write_vectored(&out).expect("WRITE");
    let result = unsafe { std::str::from_utf8_unchecked(writer.buffer()) };
    println!("===============================\n{result}\n===============================");

    let buffer = unsafe { std::str::from_utf8_unchecked(kawa.storage.used()) };
    println!("===============================\n{buffer}\n===============================");

    kawa.consume(amount);
    println!("{amount}");
    debug_kawa(&kawa);
    kawa.storage.buffer
}
fn test<T: AsBuffer>(kind: Kind, storage: T, fragment: &[u8]) -> T {
    let buffer = Buffer::new(storage);
    let storage = test_with_converter(kind, buffer, fragment, &mut h1::BlockConverter);
    let buffer = Buffer::new(storage);
    let storage = test_with_converter(kind, buffer, fragment, &mut h2::BlockConverter);
    storage
}

fn test_partial_with_converter<T: AsBuffer, C: BlockConverter<T>>(
    kind: Kind,
    storage: Buffer<T>,
    mut fragments: Vec<&[u8]>,
    converter: &mut C,
) -> T {
    let mut writer = std::io::BufWriter::new(Vec::new());
    let mut kawa = Kawa::new(kind, storage);

    while !fragments.is_empty() {
        let fragment = fragments.remove(0);
        let _ = kawa.storage.write(fragment).expect("WRITE");

        let buffer = unsafe { std::str::from_utf8_unchecked(kawa.storage.used()) };
        println!("===============================\n{buffer}\n===============================");
        debug_kawa(&kawa);

        h1::parse(&mut kawa, &mut h1::NoCallbacks);
        debug_kawa(&kawa);

        kawa.prepare(converter);
        debug_kawa(&kawa);

        let out = kawa.as_io_slice();
        println!("{out:?}");
        let amount = writer.write_vectored(&out).expect("WRITE");
        println!("{amount:?}");
        kawa.consume(amount);

        let result = unsafe { std::str::from_utf8_unchecked(writer.buffer()) };
        println!("===============================\n{result}\n===============================");
    }
    debug_kawa(&kawa);
    kawa.storage.buffer
}
fn test_partial<T: AsBuffer>(kind: Kind, storage: T, fragments: Vec<&[u8]>) -> T {
    let storage = test_partial_with_converter(
        kind,
        Buffer::new(storage),
        fragments.clone(),
        &mut h1::BlockConverter,
    );
    let storage = test_partial_with_converter(
        kind,
        Buffer::new(storage),
        fragments,
        &mut h2::BlockConverter,
    );
    storage
}

#[test]
fn tests() {
    let mut buffer = vec![0; 512];
    test(
        Kind::Request,
        SliceBuffer(&mut buffer[..]),
        b"CONNECT www.example.com:80 HTTP/1.1\r\nTE: lol\r\nTE: trailers\r\n\r\n",
    );

    test(
        Kind::Request,
        SliceBuffer(&mut buffer[..]),
        b"POST /cgi-bin/process.cgi HTTP/1.1\r
User-Agent: Mozilla/4.0 (compatible; MSIE5.01; Windows NT)\r
Host: www.tutorialspoint.com\r
Content-Type: application/x-www-form-urlencoded\r
Content-Length: 49\r
Cookie: crumb=1\r
Accept-Language: en-us\r
Accept-Encoding: gzip, deflate\r
Connection: Keep-Alive\r
Cookie: crumb=2; crumb=3\r
\r
licenseID=string&content=string&/paramsXML=string",
    );

    test(
        Kind::Response,
        SliceBuffer(&mut buffer[..128]),
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
        SliceBuffer(&mut buffer[..128]),
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
