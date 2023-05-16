use std::cmp::min;
use std::mem;

use nom::{Err as NomError, Offset, ParseTo};

/// Primitives used to parse http using nom and simd optimization when applicable
mod primitives;

use crate::{
    protocol::{
        h1::parser::primitives::{
            crlf, parse_chunk_header, parse_header, parse_header_or_cookie, parse_request_line,
            parse_response_line, parse_single_crumb, parse_url,
        },
        utils::compare_no_case,
    },
    storage::{
        AsBuffer, Block, BodySize, Chunk, ChunkHeader, Flags, Kawa, Kind, Pair, ParsingPhase,
        StatusLine, Store,
    },
};

#[inline]
fn handle_error<T: AsBuffer, E>(kawa: &Kawa<T>, error: NomError<E>) -> ParsingPhase {
    match error {
        NomError::Error(_) | NomError::Failure(_) => ParsingPhase::Error,
        NomError::Incomplete(_) => kawa.parsing_phase,
    }
}

fn process_headers<T: AsBuffer>(kawa: &mut Kawa<T>) {
    let buf = kawa.storage.mut_buffer();

    let (mut authority, path) = match &kawa.detached.status_line {
        StatusLine::Request {
            uri: Store::Slice(uri),
            method: Store::Slice(method),
            ..
        } => {
            let uri = uri.data(buf);
            let method = method.data(buf);
            match parse_url(buf, method, uri) {
                Some((authority, path)) => (authority, path),
                _ => {
                    kawa.parsing_phase = ParsingPhase::Error;
                    return;
                }
            }
        }
        _ => (Store::Empty, Store::Empty),
    };

    for block in &mut kawa.blocks {
        if let Block::Header(header) = block {
            let Store::Slice(key) = &header.key else { unreachable!() };
            let key = key.data(buf);
            if compare_no_case(key, b"host") {
                // request line has higher priority than Host header
                if let Store::Empty = authority {
                    mem::swap(&mut authority, &mut header.val);
                }
                header.elide(); // Host header is elided
            } else if compare_no_case(key, b"content-length") {
                match kawa.body_size {
                    BodySize::Empty => {}
                    BodySize::Chunked | BodySize::Length(_) => todo!(),
                }
                match header.val.data(buf).parse_to() {
                    Some(length) => kawa.body_size = BodySize::Length(length),
                    None => todo!(),
                }
            } else if compare_no_case(key, b"transfer-encoding") {
                let val = header.val.data(buf);
                if compare_no_case(val, b"chunked") {
                    match kawa.body_size {
                        BodySize::Empty => {}
                        BodySize::Chunked | BodySize::Length(_) => todo!(),
                    }
                    kawa.body_size = BodySize::Chunked;
                }
            }
        }
    }
    match &mut kawa.detached.status_line {
        StatusLine::Request {
            authority: old_authority,
            path: old_path,
            ..
        } => {
            *old_authority = authority;
            *old_path = path;
        }
        StatusLine::Response { code: 100, .. } => {
            kawa.body_size = BodySize::Length(0);
        }
        StatusLine::Response { code: 101, .. } => {
            kawa.body_size = BodySize::Length(0);
        }
        _ => {}
    };
}

pub trait ParserCallbacks<T: AsBuffer> {
    fn on_headers(&mut self, _kawa: &mut Kawa<T>) {}
}

pub struct NoCallbacks;
impl<T: AsBuffer> ParserCallbacks<T> for NoCallbacks {}

pub fn parse<T: AsBuffer, C: ParserCallbacks<T>>(kawa: &mut Kawa<T>, callbacks: &mut C) {
    let mut need_processing = false;
    loop {
        let buf = kawa.storage.buffer();
        let mut unparsed_buf = kawa.storage.unparsed_data();
        'main: while !unparsed_buf.is_empty() {
            match kawa.parsing_phase {
                ParsingPhase::StatusLine => {
                    match kawa.kind {
                        Kind::Request => match parse_request_line(unparsed_buf) {
                            Ok((i, (method, uri, version))) => {
                                kawa.detached.status_line = StatusLine::Request {
                                    version,
                                    method: Store::new_slice(buf, method),
                                    uri: Store::new_slice(buf, uri),
                                    authority: Store::Empty,
                                    path: Store::Empty,
                                };
                                unparsed_buf = i;
                            }
                            Err(error) => {
                                kawa.parsing_phase = handle_error(kawa, error);
                                break;
                            }
                        },
                        Kind::Response => match parse_response_line(unparsed_buf) {
                            Ok((i, (version, status, code, reason))) => {
                                kawa.detached.status_line = StatusLine::Response {
                                    version,
                                    code,
                                    status: Store::new_slice(buf, status),
                                    reason: Store::new_slice(buf, reason),
                                };
                                unparsed_buf = i;
                            }
                            Err(error) => {
                                kawa.parsing_phase = handle_error(kawa, error);
                                break;
                            }
                        },
                    };
                    kawa.blocks.push_back(Block::StatusLine);
                    kawa.parsing_phase = ParsingPhase::Headers;
                }
                ParsingPhase::Headers => match parse_header_or_cookie(unparsed_buf) {
                    Ok((i, Some((key, val)))) => {
                        kawa.blocks.push_back(Block::Header(Pair {
                            key: Store::new_slice(buf, key),
                            val: Store::new_slice(buf, val),
                        }));
                        unparsed_buf = i;
                    }
                    Ok((i, None)) => {
                        unparsed_buf = i;
                        kawa.blocks.push_back(Block::Cookies);
                        let mut first = true;
                        loop {
                            match parse_single_crumb(unparsed_buf, first) {
                                Ok((i, (key, val))) => {
                                    kawa.detached.jar.push_back(Pair {
                                        key: Store::new_slice(buf, key),
                                        val: Store::new_slice(buf, val),
                                    });
                                    unparsed_buf = i;
                                }
                                Err(NomError::Incomplete(_)) => {
                                    break 'main;
                                }
                                Err(_) => match crlf(unparsed_buf) {
                                    Ok((i, _)) => {
                                        unparsed_buf = i;
                                        break;
                                    }
                                    Err(error) => {
                                        kawa.parsing_phase = handle_error(kawa, error);
                                        break 'main;
                                    }
                                },
                            }
                            first = false;
                        }
                    }
                    Err(NomError::Incomplete(_)) => {
                        break;
                    }
                    Err(_) => match crlf(unparsed_buf) {
                        Ok((i, _)) => {
                            need_processing = true;
                            unparsed_buf = i;
                            break;
                        }
                        Err(error) => {
                            kawa.parsing_phase = handle_error(kawa, error);
                            break;
                        }
                    },
                },
                ParsingPhase::Body => {
                    let len = unparsed_buf.len();
                    let taken = if kawa.body_size == BodySize::Empty {
                        len
                    } else {
                        let taken = min(len, kawa.expects);
                        kawa.expects -= taken;
                        taken
                    };
                    kawa.blocks.push_back(Block::Chunk(Chunk {
                        data: Store::new_slice(buf, &unparsed_buf[..taken]),
                    }));
                    if kawa.expects == 0 {
                        kawa.parsing_phase = ParsingPhase::Terminated;
                        kawa.blocks.push_back(Block::Flags(Flags {
                            end_body: true,
                            end_chunk: false,
                            end_header: false,
                            end_stream: true,
                        }));
                    }
                    unparsed_buf = &unparsed_buf[taken..];
                }
                ParsingPhase::Chunks { first } => {
                    if kawa.expects == 0 {
                        let (i, (size_hexa, size)) = match parse_chunk_header(first, unparsed_buf) {
                            Ok(ok) => {
                                kawa.parsing_phase = ParsingPhase::Chunks { first: false };
                                ok
                            }
                            Err(error) => {
                                kawa.parsing_phase = handle_error(kawa, error);
                                break;
                            }
                        };
                        kawa.expects = size;
                        if size == 0 {
                            kawa.blocks.push_back(Block::Flags(Flags {
                                end_body: true,
                                end_chunk: false,
                                end_header: false,
                                end_stream: false,
                            }));
                            kawa.parsing_phase = ParsingPhase::Trailers;
                        } else {
                            kawa.blocks.push_back(Block::ChunkHeader(ChunkHeader {
                                length: Store::new_slice(buf, size_hexa),
                            }));
                        }
                        unparsed_buf = i;
                    } else {
                        let len = unparsed_buf.len();
                        let taken = min(len, kawa.expects);
                        kawa.expects -= taken;
                        kawa.blocks.push_back(Block::Chunk(Chunk {
                            data: Store::new_slice(buf, &unparsed_buf[..taken]),
                        }));
                        if kawa.expects == 0 {
                            kawa.blocks.push_back(Block::Flags(Flags {
                                end_body: false,
                                end_chunk: true,
                                end_header: false,
                                end_stream: false,
                            }));
                        }
                        unparsed_buf = &unparsed_buf[taken..];
                    }
                }
                ParsingPhase::Trailers => match parse_header(unparsed_buf) {
                    Ok((i, (key, val))) => {
                        kawa.blocks.push_back(Block::Header(Pair {
                            key: Store::new_slice(buf, key),
                            val: Store::new_slice(buf, val),
                        }));
                        unparsed_buf = i;
                    }
                    Err(NomError::Incomplete(_)) => {
                        break;
                    }
                    Err(_) => match crlf(unparsed_buf) {
                        Ok((i, _)) => {
                            kawa.parsing_phase = ParsingPhase::Terminated;
                            kawa.blocks.push_back(Block::Flags(Flags {
                                end_body: false,
                                end_chunk: false,
                                end_header: true,
                                end_stream: true,
                            }));
                            unparsed_buf = i;
                            break;
                        }
                        Err(error) => {
                            kawa.parsing_phase = handle_error(kawa, error);
                            break;
                        }
                    },
                },
                ParsingPhase::Terminated | ParsingPhase::Error => return,
            };
        }
        kawa.storage.head = buf.offset(unparsed_buf);
        if need_processing {
            process_headers(kawa);
            if kawa.is_error() {
                return;
            }
            need_processing = false;
            kawa.parsing_phase = match kawa.body_size {
                BodySize::Chunked => ParsingPhase::Chunks { first: true },
                BodySize::Length(0) => ParsingPhase::Terminated,
                BodySize::Length(length) => {
                    kawa.expects = length;
                    ParsingPhase::Body
                }
                BodySize::Empty => {
                    kawa.expects = 1;
                    ParsingPhase::Body
                }
            };
            callbacks.on_headers(kawa);
            kawa.blocks.push_back(Block::Flags(Flags {
                end_body: false,
                end_chunk: false,
                end_header: true,
                end_stream: kawa.is_terminated(),
            }));
        } else {
            return;
        }
    }
}
