use std::cmp::min;
use std::mem;

use nom::{Err as NomError, Offset, ParseTo};

mod primitives;

use crate::protocol::{
    h1::parser::primitives::{
        crlf, parse_chunk_header, parse_header, parse_request_line, parse_response_line, parse_url,
    },
    utils::compare_no_case,
};
use crate::storage::{
    BodySize, Chunk, ChunkHeader, Flags, Header, Htx, HtxBlock, Kind, ParsingPhase, StatusLine,
    Store,
};

fn handle_error<E>(htx: &Htx, error: NomError<E>) -> ParsingPhase {
    match error {
        NomError::Error(_) | NomError::Failure(_) => ParsingPhase::Error,
        NomError::Incomplete(_) => htx.parsing_phase,
    }
}

fn process_headers(htx: &mut Htx) {
    let buf = &mut htx.storage.buffer;

    let (mut authority, path) = match htx.blocks.get_mut(0) {
        Some(HtxBlock::StatusLine(StatusLine::Request { uri, method, .. })) => {
            let uri = uri.data(buf);
            let method = method.data(buf);
            match parse_url(buf, method, uri) {
                Some((authority, path)) => (authority, path),
                _ => {
                    htx.parsing_phase = ParsingPhase::Error;
                    return;
                }
            }
        }
        Some(HtxBlock::StatusLine(StatusLine::Response { .. })) => (Store::Empty, Store::Empty),
        _ => unreachable!(),
    };

    for block in &mut htx.blocks {
        #[allow(clippy::single_match)]
        match block {
            HtxBlock::Header(header) => {
                let key = header.key.data(buf);
                if compare_no_case(key, b"connection") {
                    header.val.modify(buf, b"close")
                } else if compare_no_case(key, b"host") {
                    // request line has higher priority than Host header
                    if let Store::Empty = authority {
                        mem::swap(&mut authority, &mut header.val);
                    }
                    header.key = Store::Empty; // Host header is elided
                } else if compare_no_case(key, b"content-length") {
                    match htx.body_size {
                        BodySize::Empty => {}
                        BodySize::Chunked | BodySize::Length(_) => todo!(),
                    }
                    match header.val.data(buf).parse_to() {
                        Some(length) => htx.body_size = BodySize::Length(length),
                        None => todo!(),
                    }
                } else if compare_no_case(key, b"transfer-encoding") {
                    let val = header.val.data(buf);
                    if compare_no_case(val, b"chunked") {
                        match htx.body_size {
                            BodySize::Empty => {}
                            BodySize::Chunked | BodySize::Length(_) => todo!(),
                        }
                        htx.body_size = BodySize::Chunked;
                    }
                }
            }
            _ => {}
        }
    }
    match htx.blocks.get_mut(0) {
        Some(HtxBlock::StatusLine(StatusLine::Request {
            authority: old_authority,
            path: old_path,
            ..
        })) => {
            *old_authority = authority;
            *old_path = path;
        }
        Some(HtxBlock::StatusLine(StatusLine::Response { .. })) => {}
        _ => unreachable!(),
    };
    htx.blocks.push_back(HtxBlock::Header(Header {
        key: Store::Static(b"Sozu-id"),
        val: Store::new_vec(format!("SOZUBALANCEID-{}", htx.storage.head).as_bytes()),
    }));
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
            ParsingPhase::StatusLine => {
                let status_line = match htx.kind {
                    Kind::Request => parse_request_line(buf, unparsed_buf),
                    Kind::Response => parse_response_line(buf, unparsed_buf),
                };
                let (i, status_line) = match status_line {
                    Ok(ok) => ok,
                    Err(error) => {
                        htx.parsing_phase = handle_error(htx, error);
                        break;
                    }
                };
                println!("{status_line:?}");
                htx.blocks.push_back(HtxBlock::StatusLine(status_line));
                htx.parsing_phase = ParsingPhase::Headers;
                i
            }
            ParsingPhase::Headers => match parse_header(buf, unparsed_buf) {
                Ok((i, header)) => {
                    println!("{header:?}");
                    htx.blocks.push_back(HtxBlock::Header(header));
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
            ParsingPhase::Body => {
                let len = unparsed_buf.len();
                let taken = min(len, htx.expects);
                htx.expects -= taken;
                htx.blocks.push_back(HtxBlock::Chunk(Chunk {
                    data: Store::new_slice(buf, &unparsed_buf[..taken]),
                }));
                if htx.expects == 0 {
                    htx.parsing_phase = ParsingPhase::Terminated;
                    htx.blocks.push_back(HtxBlock::Flags(Flags {
                        end_body: true,
                        end_chunk: false,
                        end_header: false,
                        end_stream: true,
                    }));
                }
                &unparsed_buf[taken..]
            }
            ParsingPhase::Chunks { first } => {
                if htx.expects == 0 {
                    let (i, (size_hexa, size)) = match parse_chunk_header(first, unparsed_buf) {
                        Ok(ok) => {
                            htx.parsing_phase = ParsingPhase::Chunks { first: false };
                            ok
                        }
                        Err(error) => {
                            htx.parsing_phase = handle_error(htx, error);
                            break;
                        }
                    };
                    htx.expects = size;
                    if size == 0 {
                        htx.blocks.push_back(HtxBlock::Flags(Flags {
                            end_body: true,
                            end_chunk: false,
                            end_header: false,
                            end_stream: false,
                        }));
                        htx.parsing_phase = ParsingPhase::Trailers;
                    } else {
                        htx.blocks.push_back(HtxBlock::ChunkHeader(ChunkHeader {
                            length: Store::new_slice(buf, size_hexa),
                        }));
                    }
                    i
                } else {
                    let len = unparsed_buf.len();
                    let taken = min(len, htx.expects);
                    htx.expects -= taken;
                    htx.blocks.push_back(HtxBlock::Chunk(Chunk {
                        data: Store::new_slice(buf, &unparsed_buf[..taken]),
                    }));
                    if htx.expects == 0 {
                        htx.blocks.push_back(HtxBlock::Flags(Flags {
                            end_body: false,
                            end_chunk: true,
                            end_header: false,
                            end_stream: false,
                        }));
                    }
                    &unparsed_buf[taken..]
                }
            }
            ParsingPhase::Trailers => match parse_header(buf, unparsed_buf) {
                Ok((i, header)) => {
                    println!("{header:?}");
                    htx.blocks.push_back(HtxBlock::Header(header));
                    i
                }
                Err(NomError::Incomplete(_)) => {
                    break;
                }
                Err(_) => match crlf(unparsed_buf) {
                    Ok((i, _)) => {
                        htx.parsing_phase = ParsingPhase::Terminated;
                        htx.blocks.push_back(HtxBlock::Flags(Flags {
                            end_body: false,
                            end_chunk: false,
                            end_header: true,
                            end_stream: true,
                        }));
                        i
                    }
                    Err(error) => {
                        htx.parsing_phase = handle_error(htx, error);
                        break;
                    }
                },
            },
            ParsingPhase::Terminated | ParsingPhase::Error => break,
        };
        htx.storage.head = htx.storage.buffer.offset(i);
        if need_processing {
            process_headers(htx);
            if htx.is_error() {
                return;
            }
            need_processing = false;
            htx.parsing_phase = match htx.body_size {
                BodySize::Empty | BodySize::Length(0) => ParsingPhase::Terminated,
                BodySize::Chunked => ParsingPhase::Chunks { first: true },
                BodySize::Length(length) => {
                    htx.expects = length;
                    ParsingPhase::Body
                }
            };
            htx.blocks.push_back(HtxBlock::Flags(Flags {
                end_body: false,
                end_chunk: false,
                end_header: true,
                end_stream: htx.is_terminated(),
            }));
        }
    }
}
