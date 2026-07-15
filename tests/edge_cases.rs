use std::{io::Write, str::from_utf8};

use kawa::{h1, Block, BodySize, Buffer, Kawa, Kind, SliceBuffer};

#[test]
fn compressed_chunked() {
    const REQUEST: &[u8] = b"\
GET /image.jpg HTTP/1.1\r\n\
Host: www.compressed.com\r\n\
Transfer-Encoding: gzip,chunked\r\n\r\n0\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write_all(REQUEST).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    kawa::debug_kawa(&req);
    assert!(req.is_streaming());
    assert!(req.is_terminated());
    assert!(req.storage.unparsed_data().is_empty());
}

#[test]
fn multiple_content_length() {
    const REQUEST_VALID: &[u8] = b"\
GET /image.jpg HTTP/1.1\r\n\
Host: www.compressed.com\r\n\
Content-Length: 3\r\n\
Content-Length: 3\r\n\r\nABC";
    const REQUEST_INVALID: &[u8] = b"\
GET /image.jpg HTTP/1.1\r\n\
Host: www.compressed.com\r\n\
Content-Length: 3\r\n\
Content-Length: 4\r\n\r\nABCD";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write_all(REQUEST_VALID).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    kawa::debug_kawa(&req);
    assert!(req.body_size == BodySize::Length(3));
    assert!(req.is_terminated());
    assert!(req.storage.unparsed_data().is_empty());

    req.clear();
    req.storage.write_all(REQUEST_INVALID).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    kawa::debug_kawa(&req);
    assert!(req.is_error());
}

#[test]
fn multiple_length_information() {
    const REQUEST: &[u8] = b"\
GET /image.jpg HTTP/1.1\r\n\
Host: www.compressed.com\r\n\
Content-Length: 3\r\n\
Content-Length: 3\r\n\
Transfer-Encoding: chunked\r\n\
Transfer-Encoding: chunked\r\n\
Content-Length: 4\r\n\r\n0\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write_all(REQUEST).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    kawa::debug_kawa(&req);
    assert!(req.is_terminated());
    assert!(req.is_streaming());
    assert!(req.storage.unparsed_data().is_empty());
    for block in &req.blocks {
        if let Block::Header(header) = block {
            if let Some(key) = header.key.data_opt(&buffer) {
                assert_ne!(key, b"Content-Length");
            }
        }
    }
}

#[test]
fn transfer_encoding_ows_and_final_coding() {
    // RFC 9112 §6.1 / RFC 9110 §5.6.3: chunked framing is selected only
    // when "chunked" is the FINAL transfer-coding, after trimming OWS
    // (SP/HTAB) from the header value and from each comma-separated
    // token. This is a regression test for sozu-proxy/sozu#726: the
    // pre-fix code did a suffix-only, untrimmed compare, so a value like
    // "chunked\t" (trailing tab) neither matched nor was rejected, and
    // Content-Length framing silently stayed active while the malformed
    // Transfer-Encoding header was left un-elided -- both framing headers
    // reached the backend.
    let mut buffer = vec![0; 4096];

    // 1. plain chunked
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Transfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert_eq!(req.body_size, BodySize::Chunked);
        assert!(!req.is_error());
    }

    // 2. trailing tab -- the #726 regression case
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Transfer-Encoding: chunked\t\r\n\r\n0\r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert_eq!(req.body_size, BodySize::Chunked);
        assert!(!req.is_error());
    }

    // 3. surrounding OWS
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Transfer-Encoding:  chunked \r\n\r\n0\r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert_eq!(req.body_size, BodySize::Chunked);
        assert!(!req.is_error());
    }

    // 4. "gzip, chunked" -- chunked is the final coding
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Transfer-Encoding: gzip, chunked\r\n\r\n0\r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert_eq!(req.body_size, BodySize::Chunked);
        assert!(!req.is_error());
    }

    // 5. "chunked, gzip" -- chunked is NOT the final coding -> reject
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Transfer-Encoding: chunked, gzip\r\n\r\nabc";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert!(req.is_error());
    }

    // 6. "identity" -- not chunked at all -> reject
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Transfer-Encoding: identity\r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert!(req.is_error());
    }

    // 7. "xchunked" -- false positive under the old suffix-only check -> reject
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Transfer-Encoding: xchunked\r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert!(req.is_error());
    }
}

/// Returns the value of the first non-elided header matching `name`.
fn header_value<'a>(req: &'a Kawa<SliceBuffer<'a>>, name: &[u8]) -> Option<&'a [u8]> {
    let buf = req.storage.buffer();
    req.blocks.iter().find_map(|block| match block {
        Block::Header(header) => {
            let key = header.key.data_opt(buf)?;
            if key.eq_ignore_ascii_case(name) {
                header.val.data_opt(buf)
            } else {
                None
            }
        }
        _ => None,
    })
}

#[test]
fn header_value_excludes_surrounding_ows() {
    // RFC 9112 §5: "A field value does not include leading or trailing
    // whitespace." Leading OWS was already stripped (the `take_while` after
    // the colon), but `achar` accepts SP/HTAB, so trailing OWS was swallowed
    // into the stored value. Two distinct defects followed, both fixed by
    // trimming the value at parse time:
    //
    //   * `Transfer-Encoding: chunked\t` selects chunked framing (the value
    //     is OWS-trimmed to decide the final coding) yet was FORWARDED
    //     verbatim. A peer that does not itself trim then sees a coding it
    //     does not recognise and -- the Content-Length having been elided
    //     per RFC 9110 §6.3 -- no length either, so it reads the chunked
    //     body as a pipelined message: a TE.TE desync. Deciding framing on a
    //     normalized reading while forwarding the un-normalized bytes is the
    //     gap; forward the coding we actually framed on.
    //
    //   * `Content-Length: 5 ` is a legal field value, but `"5 ".parse()`
    //     returned None and the request was rejected outright.
    let mut buffer = vec![0; 4096];

    // 1. the forwarded Transfer-Encoding is the coding we framed on
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Transfer-Encoding: chunked\t\r\n\r\n0\r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert_eq!(req.body_size, BodySize::Chunked);
        assert!(!req.is_error());
        assert_eq!(
            header_value(&req, b"transfer-encoding"),
            Some(&b"chunked"[..]),
            "the Transfer-Encoding we framed on must be forwarded canonically, \
             not as the obfuscated spelling we received"
        );
    }

    // 2. trailing OWS no longer rejects a legal Content-Length
    {
        const REQUEST: &[u8] = b"\
POST / HTTP/1.1\r\n\
Host: example.com\r\n\
Content-Length: 5 \r\n\r\nHello";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert!(!req.is_error());
        assert_eq!(req.body_size, BodySize::Length(5));
    }

    // 3. the rule is general, not Transfer-Encoding specific: leading and
    //    trailing OWS around any field value are not part of the value.
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
X-Custom: \tvalue \t\r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert!(!req.is_error());
        assert_eq!(header_value(&req, b"x-custom"), Some(&b"value"[..]));
    }

    // 4. an all-OWS field value trims to empty rather than to whitespace
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
X-Empty: \t \r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert!(!req.is_error());
        assert_eq!(header_value(&req, b"x-empty"), Some(&b""[..]));
    }
}

#[test]
fn transfer_encoding_ows_elides_content_length() {
    // "Transfer-Encoding: chunked\t" (trailing tab) alongside a
    // Content-Length must still select chunked framing AND elide the
    // Content-Length, so only one framing header reaches the backend.
    const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Content-Length: 3\r\n\
Transfer-Encoding: chunked\t\r\n\r\n0\r\n\r\n";
    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write_all(REQUEST).expect("write");
    h1::parse(&mut req, &mut h1::NoCallbacks);
    assert_eq!(req.body_size, BodySize::Chunked);
    assert!(!req.is_error());
    for block in &req.blocks {
        if let Block::Header(header) = block {
            if let Some(key) = header.key.data_opt(&buffer) {
                assert_ne!(key, b"Content-Length");
            }
        }
    }
}

#[test]
fn transfer_encoding_split_header_lines() {
    // RFC 9110 §5.3: repeated Transfer-Encoding field lines are
    // equivalent to a single comma-joined value, in the order the lines
    // appear on the wire. Regression test: rejecting as soon as one
    // Transfer-Encoding header BLOCK doesn't end in chunked (instead of
    // waiting for the combined/last value) broke split Transfer-Encoding
    // whose final line is chunked.
    let mut buffer = vec![0; 4096];

    // "gzip" then "chunked" on separate lines == "gzip, chunked" as a
    // single value -> chunked is the final coding -> valid chunked
    // framing, not an error.
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Transfer-Encoding: gzip\r\n\
Transfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert_eq!(req.body_size, BodySize::Chunked);
        assert!(!req.is_error());
        assert!(req.is_terminated());
    }

    // "chunked" then "identity" on separate lines == "chunked, identity"
    // as a single value -> chunked is NOT the final coding -> reject
    // (RFC 9112 §6.3), even though the FIRST line alone ends in chunked.
    {
        const REQUEST: &[u8] = b"\
GET / HTTP/1.1\r\n\
Host: example.com\r\n\
Transfer-Encoding: chunked\r\n\
Transfer-Encoding: identity\r\n\r\n";
        let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
        req.storage.write_all(REQUEST).expect("write");
        h1::parse(&mut req, &mut h1::NoCallbacks);
        assert!(req.is_error());
    }
}

#[test]
fn transfer_encoding_response_non_chunked_final_is_not_an_error() {
    // RFC 9112 §6.3's "reject a message whose Transfer-Encoding does not
    // end in chunked" is a REQUEST-only rule: the server cannot ask the
    // client to retry with a different framing, so an unreliable request
    // body length must be rejected outright. A RESPONSE with a
    // Transfer-Encoding that does not end in chunked (e.g. a lone
    // "gzip") is spec-valid instead: the body is close-delimited (read
    // until the connection closes) rather than an error.
    let mut buffer = vec![0; 4096];

    {
        const RESPONSE: &[u8] = b"\
HTTP/1.1 200 OK\r\n\
Transfer-Encoding: gzip\r\n\r\n";
        let mut resp = Kawa::new(Kind::Response, Buffer::new(SliceBuffer(&mut buffer[..])));
        resp.storage.write_all(RESPONSE).expect("write");
        h1::parse(&mut resp, &mut h1::NoCallbacks);
        assert!(!resp.is_error());
        // No Content-Length was present, so the pre-fix reference value
        // is BodySize::Empty (close-delimited: read until connection
        // close), not merely "anything other than Chunked".
        assert_eq!(resp.body_size, BodySize::Empty);
    }

    // A response whose Transfer-Encoding DOES end in chunked still gets
    // chunked framing -- the request-only reject rule above must not
    // suppress genuinely valid chunked responses.
    {
        const RESPONSE: &[u8] = b"\
HTTP/1.1 200 OK\r\n\
Transfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        let mut resp = Kawa::new(Kind::Response, Buffer::new(SliceBuffer(&mut buffer[..])));
        resp.storage.write_all(RESPONSE).expect("write");
        h1::parse(&mut resp, &mut h1::NoCallbacks);
        assert_eq!(resp.body_size, BodySize::Chunked);
        assert!(!resp.is_error());
        assert!(resp.is_terminated());
    }

    // Regression (the finding): a response whose split Transfer-Encoding lines
    // are "chunked" then "identity" combines to "chunked, identity" -- chunked
    // is NOT the final coding, so the body is close-delimited, NOT chunked. The
    // earlier "chunked" line must not latch chunked framing (the prior fix left
    // body_size stuck at Chunked here).
    {
        const RESPONSE: &[u8] = b"\
HTTP/1.1 200 OK\r\n\
Transfer-Encoding: chunked\r\n\
Transfer-Encoding: identity\r\n\r\n";
        let mut resp = Kawa::new(Kind::Response, Buffer::new(SliceBuffer(&mut buffer[..])));
        resp.storage.write_all(RESPONSE).expect("write");
        h1::parse(&mut resp, &mut h1::NoCallbacks);
        assert!(!resp.is_error());
        assert_eq!(resp.body_size, BodySize::Empty);
    }
}

#[test]
fn malformed_cookies_separator() {
    const REQUEST: &[u8] = b"\
GET /cookies HTTP/1.1\r\n\
Host: www.bad.com\r\n\
Cookie: a=1; b=2;c=3; foo; ==bar=\r\n\r\n0\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write_all(REQUEST).expect("write");
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
    const REQUEST: &[u8] = b"\
GET /cookies HTTP/1.1\r\n\
Host: www.bad.com\r\n\
Cookie: a=b;  c d e  = fg h ;i=j;  k   l=  mn  \r\n\r\n0\r\n\r\n";

    let mut buffer = vec![0; 4096];
    let mut req = Kawa::new(Kind::Request, Buffer::new(SliceBuffer(&mut buffer[..])));
    req.storage.write_all(REQUEST).expect("write");
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
