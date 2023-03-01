use std::cmp::min;

use crate::protocol::h1::bytes::{
    compare_no_case, crlf, parse_chunk_header, parse_header, parse_request_line,
    parse_response_line,
};

use crate::htx::{Chunk, HtxBlock, HtxBodySize, HtxKind, HtxParsingPhase, StatusLine, Store, HTX};

use nom::{Err as NomError, Offset, ParseTo};

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

pub fn parse(htx: &mut HTX, buf: &mut [u8]) {
    let mut need_processing = false;
    loop {
        let unparsed_buf = &buf[htx.index..];
        if unparsed_buf.is_empty() {
            break;
        }
        let i = match htx.parsing_phase {
            HtxParsingPhase::StatusLine => {
                let status_line = match htx.kind {
                    HtxKind::Request => parse_request_line(buf, unparsed_buf),
                    HtxKind::Response => parse_response_line(buf, unparsed_buf),
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
