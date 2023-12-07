use std::{io::Write, str::from_utf8};

use kawa::{h1, Buffer, Kawa, Kind, SliceBuffer};

#[test]
fn malformed_cookies_separator() {
    const REQ_LONG: &'static [u8] = b"\
GET /cookies HTTP/1.1\r\n\
Host: www.bad.com\r\n\
Cookie: a=1; b=2;c=3; foo; ==bar=\r\n\r\n0\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write(REQ_LONG).expect("write");
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
        let key = from_utf8(crumb.key.data(REQ_LONG));
        let val = from_utf8(crumb.val.data(REQ_LONG));
        assert_eq!(Ok(k), key);
        assert_eq!(Ok(v), val);
    }
}

#[test]
fn spaces_in_cookie() {
    const REQ_LONG: &'static [u8] = b"\
GET /cookies HTTP/1.1\r\n\
Host: www.bad.com\r\n\
Cookie: a=b;  c d e  = fg h ;i=j;  k   l=  mn  \r\n\r\n0\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write(REQ_LONG).expect("write");
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
        let key = from_utf8(crumb.key.data(REQ_LONG));
        let val = from_utf8(crumb.val.data(REQ_LONG));
        assert_eq!(Ok(k), key);
        assert_eq!(Ok(v), val);
    }
}
