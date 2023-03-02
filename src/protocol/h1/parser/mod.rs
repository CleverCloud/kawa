use std::cmp::min;

use nom::{Err as NomError, Offset, ParseTo};

mod primitives;

use crate::htx::{Chunk, Htx, HtxBlock, HtxBodySize, HtxKind, HtxParsingPhase, StatusLine, Store};
use crate::protocol::h1::parser::primitives::{
    compare_no_case, crlf, parse_chunk_header, parse_header, parse_request_line,
    parse_response_line,
};

fn handle_error<E>(htx: &Htx, error: NomError<E>) -> HtxParsingPhase {
    match error {
        NomError::Error(_) | NomError::Failure(_) => HtxParsingPhase::Error,
        NomError::Incomplete(_) => htx.parsing_phase,
    }
}

fn process_headers(htx: &mut Htx) {
    let buf = &mut htx.storage.buffer;
    let mut host = Store::Empty;
    for block in &mut htx.blocks {
        #[allow(clippy::single_match)]
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
        Some(HtxBlock::StatusLine(StatusLine::Response { .. })) => {}
        _ => unreachable!(),
    };
}

pub fn parse(htx: &mut Htx) {
    let mut need_processing = false;
    loop {
        let unparsed_buf = htx.storage.unparsed_data();
        let buf = htx.storage.buffer();
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
                        htx.parsing_phase = handle_error(htx, error);
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
                        htx.parsing_phase = handle_error(htx, error);
                        break;
                    }
                },
            },
            HtxParsingPhase::Body => {
                let len = unparsed_buf.len();
                let taken = min(len, htx.expects);
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
                            htx.parsing_phase = handle_error(htx, error);
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
                    let len = unparsed_buf.len();
                    let taken = min(len, htx.expects);
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
                        htx.parsing_phase = handle_error(htx, error);
                        break;
                    }
                },
            },
            HtxParsingPhase::Terminated | HtxParsingPhase::Error => break,
        };
        htx.storage.head = htx.storage.buffer.offset(i);
        if need_processing {
            process_headers(htx);
            need_processing = false;
            htx.parsing_phase = match htx.body_size {
                HtxBodySize::Empty => HtxParsingPhase::Terminated,
                HtxBodySize::Chunked => {
                    htx.storage.head -= 2; // FIXME: THIS IS WRONG AND DANGEROUS!
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
