use std::{cmp::min, io::Write};

use parser::{compare_no_case, crlf, parse_chunk_header, parse_header, parse_request_line};

mod htx;
mod parser;

use htx::{Chunk, Header, HtxBodySize, HtxKind, HtxParsingPhase, StatusLine, Store, Version, HTX};
use nom::{Err as NomError, Offset, ParseTo};

use crate::{
    htx::{debug_htx, HtxBlock},
    parser::parse_response_line,
};

fn handle_error<E>(htx: &mut HTX, error: NomError<E>) {
    match error {
        NomError::Error(_) | NomError::Failure(_) => {
            htx.parsing_phase = HtxParsingPhase::Error;
        }
        NomError::Incomplete(_) => {}
    }
}

fn process_headers(htx: &mut HTX, buf: &mut [u8]) {
    let mut host = Store::Empty;
    for block in &mut htx.blocks {
        match block {
            HtxBlock::Header(header) => {
                let key = header.key.data(buf).expect("Header key missing");
                if compare_no_case(key, b"connection") {
                    header.val.modify(buf, b"close")
                } else if compare_no_case(key, b"host") {
                    host = match &header.val {
                        Store::Slice(slice) => Store::Deported(slice.clone()),
                        _ => unreachable!(),
                    };
                } else if compare_no_case(key, b"content-length") {
                    match htx.body_size {
                        HtxBodySize::Empty => {}
                        HtxBodySize::Chunked | HtxBodySize::Length(_) => todo!(),
                    }
                    match header.val.data(buf).and_then(|length| length.parse_to()) {
                        Some(length) => htx.body_size = HtxBodySize::Length(length),
                        None => todo!(),
                    }
                } else if compare_no_case(key, b"transfer-encoding") {
                    let val = header.val.data(buf).expect("Header value missing");
                    if compare_no_case(val, b"chunked") {
                        match htx.body_size {
                            HtxBodySize::Empty => {}
                            HtxBodySize::Chunked | HtxBodySize::Length(_) => todo!(),
                        }
                        htx.body_size = HtxBodySize::Chunked;
                    }
                }
            }
            _ => {}
        }
    }
    match htx.blocks.get_mut(0) {
        Some(HtxBlock::StatusLine(StatusLine::Request { authority, .. })) => {
            *authority = host;
        }
        _ => {}
    };
}

fn parse(htx: &mut HTX, buf: &mut [u8]) {
    let mut need_processing = false;
    loop {
        let unparsed_buf = &buf[htx.index..];
        if unparsed_buf.is_empty() {
            break;
        }
        let i = match htx.parsing_phase {
            HtxParsingPhase::StatusLine => {
                let status_line = match htx.kind {
                    htx::HtxKind::Request => parse_request_line(buf, unparsed_buf),
                    htx::HtxKind::Response => parse_response_line(buf, unparsed_buf),
                };
                let (i, status_line) = match status_line {
                    Ok(ok) => ok,
                    Err(error) => {
                        handle_error(htx, error);
                        break;
                    }
                };
                println!("{status_line:?}");
                htx.blocks.push(HtxBlock::StatusLine(status_line));
                htx.parsing_phase = HtxParsingPhase::Headers;
                i
            }
            HtxParsingPhase::Headers => match parse_header(buf, unparsed_buf) {
                Ok((i, header)) => {
                    println!("{header:?}");
                    htx.blocks.push(HtxBlock::Header(header));
                    i
                }
                Err(NomError::Incomplete(_)) => {
                    break;
                }
                Err(_) => match crlf(unparsed_buf) {
                    Ok((i, _)) => {
                        need_processing = true;
                        i
                    }
                    Err(error) => {
                        handle_error(htx, error);
                        break;
                    }
                },
            },
            HtxParsingPhase::Body => {
                let length = unparsed_buf.len();
                let taken = min(length, htx.expects);
                htx.expects -= taken;
                htx.blocks.push(HtxBlock::Chunk(Chunk {
                    data: Store::new_slice(buf, &unparsed_buf[..taken]),
                }));
                if htx.expects == 0 {
                    htx.parsing_phase = HtxParsingPhase::Terminated;
                }
                &unparsed_buf[taken..]
            }
            HtxParsingPhase::Chunks => {
                if htx.expects == 0 {
                    let (i, chunk_size) = match parse_chunk_header(unparsed_buf) {
                        Ok(ok) => ok,
                        Err(error) => {
                            handle_error(htx, error);
                            break;
                        }
                    };
                    let header_size = unparsed_buf.offset(i);
                    htx.expects = chunk_size;
                    htx.blocks.push(HtxBlock::Chunk(Chunk {
                        data: Store::new_slice(buf, &unparsed_buf[..header_size]),
                    }));
                    if chunk_size == 0 {
                        htx.parsing_phase = HtxParsingPhase::Trailers;
                    }
                    i
                } else {
                    let length = unparsed_buf.len();
                    let taken = min(length, htx.expects);
                    htx.expects -= taken;
                    htx.blocks.push(HtxBlock::Chunk(Chunk {
                        data: Store::new_slice(buf, &unparsed_buf[..taken]),
                    }));
                    &unparsed_buf[taken..]
                }
            }
            HtxParsingPhase::Trailers => match parse_header(buf, unparsed_buf) {
                Ok((i, header)) => {
                    println!("{header:?}");
                    htx.blocks.push(HtxBlock::Header(header));
                    i
                }
                Err(NomError::Incomplete(_)) => {
                    break;
                }
                Err(_) => match crlf(unparsed_buf) {
                    Ok((i, _)) => {
                        htx.parsing_phase = HtxParsingPhase::Terminated;
                        i
                    }
                    Err(error) => {
                        handle_error(htx, error);
                        break;
                    }
                },
            },
            HtxParsingPhase::Terminated | HtxParsingPhase::Error => break,
        };
        htx.index = buf.offset(i);
        if need_processing {
            process_headers(htx, buf);
            need_processing = false;
            htx.parsing_phase = match htx.body_size {
                HtxBodySize::Empty => HtxParsingPhase::Terminated,
                HtxBodySize::Chunked => {
                    htx.index -= 2;
                    HtxParsingPhase::Chunks
                }
                HtxBodySize::Length(length) => {
                    htx.expects = length;
                    HtxParsingPhase::Body
                }
            }
        }
    }
}

fn h1_block_converter(block: HtxBlock, out: &mut Vec<Store>) {
    match block {
        HtxBlock::StatusLine(StatusLine::Request {
            version,
            method,
            uri,
            ..
        }) => {
            let version = match version {
                Version::V10 => b"HTTP/1.0",
                Version::V11 | Version::V20 => b"HTTP/1.1",
            };
            out.push(method);
            out.push(Store::Static(b" "));
            out.push(uri);
            out.push(Store::Static(b" "));
            out.push(Store::Static(version));
            out.push(Store::Static(b"\r\n"));
        }
        HtxBlock::StatusLine(StatusLine::Response {
            version,
            status,
            reason,
            ..
        }) => {
            let version = match version {
                Version::V10 => b"HTTP/1.0",
                Version::V11 | Version::V20 => b"HTTP/1.1",
            };
            out.push(Store::Static(version));
            out.push(Store::Static(b" "));
            out.push(status);
            out.push(Store::Static(b" "));
            out.push(reason);
            out.push(Store::Static(b"\r\n"));
        }
        HtxBlock::Header(Header { key, val }) => {
            out.push(key);
            out.push(Store::Static(b": "));
            out.push(val);
            out.push(Store::Static(b"\r\n"));
        }
        HtxBlock::Chunk(Chunk { data }) => {
            out.push(data);
        }
    }
}

fn test(htx_type: HtxKind, buf: &[u8]) {
    let buf = &mut buf.to_vec();
    let mut htx = HTX::new(htx_type);
    debug_htx(&htx, buf);

    parse(&mut htx, buf);
    debug_htx(&htx, buf);

    htx.prepare(h1_block_converter);
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

        parse(&mut htx, &mut buf);
        debug_htx(&htx, &buf);

        htx.prepare(h1_block_converter);
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

        parse(&mut htx, &mut buf);
        debug_htx(&htx, &buf);

        buf.drain(..p1);
        htx.push_left(p1 as u32);
        debug_htx(&htx, &buf);

        htx.prepare(h1_block_converter);
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
