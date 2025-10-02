use std::{io::Write, str::from_utf8};

use kawa::{h1, Block, BodySize, Buffer, Kawa, Kind, SliceBuffer};

#[test]
fn compressed_chunked() {
    const REQUEST: &'static [u8] = b"\
GET /image.jpg HTTP/1.1\r\n\
Host: www.compressed.com\r\n\
Transfer-Encoding: gzip,chunked\r\n\r\n0\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write(REQUEST).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    kawa::debug_kawa(&req);
    assert!(req.is_streaming());
    assert!(req.is_terminated());
    assert!(req.storage.unparsed_data().is_empty());
}

#[test]
fn multiple_content_length() {
    const REQUEST_VALID: &'static [u8] = b"\
GET /image.jpg HTTP/1.1\r\n\
Host: www.compressed.com\r\n\
Content-Length: 3\r\n\
Content-Length: 3\r\n\r\nABC";
    const REQUEST_INVALID: &'static [u8] = b"\
GET /image.jpg HTTP/1.1\r\n\
Host: www.compressed.com\r\n\
Content-Length: 3\r\n\
Content-Length: 4\r\n\r\nABCD";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write(REQUEST_VALID).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    kawa::debug_kawa(&req);
    assert!(req.body_size == BodySize::Length(3));
    assert!(req.is_terminated());
    assert!(req.storage.unparsed_data().is_empty());

    req.clear();
    req.storage.write(REQUEST_INVALID).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    kawa::debug_kawa(&req);
    assert!(req.is_error());
}

#[test]
fn multiple_length_information() {
    const REQUEST: &'static [u8] = b"\
GET /image.jpg HTTP/1.1\r\n\
Host: www.compressed.com\r\n\
Content-Length: 3\r\n\
Content-Length: 3\r\n\
Transfer-Encoding: chunked\r\n\
Transfer-Encoding: chunked\r\n\
Content-Length: 4\r\n\r\n0\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write(REQUEST).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    kawa::debug_kawa(&req);
    assert!(req.is_terminated());
    assert!(req.is_streaming());
    assert!(req.storage.unparsed_data().is_empty());
    for block in req.blocks {
        if let Block::Header(header) = block {
            if let Some(key) = header.key.data_opt(&buffer) {
                assert_ne!(key, b"Content-Length");
            }
        }
    }
}

#[test]
fn malformed_cookies_separator() {
    const REQUEST: &'static [u8] = b"\
GET /cookies HTTP/1.1\r\n\
Host: www.bad.com\r\n\
Cookie: a=1; b=2;c=3; foo; ==bar=\r\n\r\n0\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write(REQUEST).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    kawa::debug_kawa(&req);
    assert!(req.storage.unparsed_data().is_empty());
    for (i, (k, v)) in [
        ("a", "1"),
        ("b", "2"),
        ("c", "3"),
        ("", "foo"),
        ("", "=bar="),
    ]
    .into_iter()
    .enumerate()
    {
        let crumb = &req.detached.jar[i];
        let key = from_utf8(crumb.key.data(REQUEST));
        let val = from_utf8(crumb.val.data(REQUEST));
        assert_eq!(Ok(k), key);
        assert_eq!(Ok(v), val);
    }
}

#[test]
fn spaces_in_cookie() {
    const REQUEST: &'static [u8] = b"\
GET /cookies HTTP/1.1\r\n\
Host: www.bad.com\r\n\
Cookie: a=b;  c d e  = fg h ;i=j;  k   l=  mn  \r\n\r\n0\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write(REQUEST).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    kawa::debug_kawa(&req);
    assert!(req.storage.unparsed_data().is_empty());
    for (i, (k, v)) in [
        ("a", "b"),
        ("c d e  ", " fg h "),
        ("i", "j"),
        ("k   l", "  mn  "),
    ]
    .into_iter()
    .enumerate()
    {
        let crumb = &req.detached.jar[i];
        let key = from_utf8(crumb.key.data(REQUEST));
        let val = from_utf8(crumb.val.data(REQUEST));
        assert_eq!(Ok(k), key);
        assert_eq!(Ok(v), val);
    }
}
