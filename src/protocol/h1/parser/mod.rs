use std::cmp::min;
use std::mem;

use nom::{error::Error as NomError, Err as NomErr, Offset, ParseTo};

/// Primitives used to parse http using nom and simd optimization when applicable
pub mod primitives;

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
fn handle_error<T: AsBuffer>(kawa: &Kawa<T>, error: NomErr<NomError<&[u8]>>) -> ParsingPhase {
    match error {
        NomErr::Error(error) | NomErr::Failure(error) => {
            let index = kawa.storage.buffer().offset(error.input) as u32;
            ParsingPhase::Error {
                marker: kawa.parsing_phase.marker(),
                kind: index.into(),
            }
        }
        NomErr::Incomplete(_) => kawa.parsing_phase,
    }
}

#[inline]
fn handle_recovery_error<T: AsBuffer>(
    kawa: &Kawa<T>,
    primary_error: NomError<&[u8]>,
    recovery_error: NomErr<NomError<&[u8]>>,
) -> ParsingPhase {
    match recovery_error {
        NomErr::Error(_) | NomErr::Failure(_) => {
            let index = kawa.storage.buffer().offset(primary_error.input) as u32;
            ParsingPhase::Error {
                marker: kawa.parsing_phase.marker(),
                kind: index.into(),
            }
        }
        NomErr::Incomplete(_) => kawa.parsing_phase,
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
                    kawa.parsing_phase.error("Invalid URI".into());
                    return;
                }
            }
        }
        _ => (Store::Empty, Store::Empty),
    };

    for block in &mut kawa.blocks {
        if let Block::Header(header) = block {
            let Store::Slice(key) = &header.key else {
                unreachable!()
            };
            let key = key.data(buf);
            if compare_no_case(key, b"host") {
                // request line has higher priority than Host header
                if let Store::Empty = authority {
                    mem::swap(&mut authority, &mut header.val);
                }
                header.elide(); // Host header is elided
            } else if compare_no_case(key, b"content-length") {
                let length = match header.val.data(buf).parse_to() {
                    Some(length) => length,
                    None => {
                        kawa.parsing_phase
                            .error("Invalid Content-Length field value".into());
                        return;
                    }
                };
                match kawa.body_size {
                    BodySize::Empty => {}
                    BodySize::Chunked => {
                        println!("WARNING: Found both a Transfer-Encoding and a Content-Length, ignoring the latter");
                        header.elide();
                        continue;
                    }
                    BodySize::Length(previous_length) => {
                        if previous_length != length {
                            kawa.parsing_phase
                                .error("Inconsistent Content-Length information".into());
                            return;
                        } else {
                            header.elide();
                        }
                    }
                }
                kawa.body_size = BodySize::Length(length);
            } else if compare_no_case(key, b"transfer-encoding") {
                let val = header.val.data(buf);
                const CHUNKED: &[u8] = b"chunked";
                if val.len() >= CHUNKED.len()
                    && compare_no_case(&val[val.len() - CHUNKED.len()..], CHUNKED)
                {
                    match kawa.body_size {
                        BodySize::Empty => {}
                        BodySize::Chunked => {
                            println!("WARNING: Found multiple Transfer-Encoding");
                        }
                        BodySize::Length(_) => {
                            println!("WARNING: Found both a Content-Length and a Transfer-Encoding, ignoring the former");
                        }
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
        // RFC 2616, 10.2.5:
        // The 204 response MUST NOT include a message-body, and thus is always
        // terminated by the first empty line after the header fields.
        // RFC 2616, 10.3.5:
        // The 304 response MUST NOT contain a message-body, and thus is always
        // terminated by the first empty line after the header fields.
        // RFC 2616, 10.1:
        // This class of status code indicates a provisional response,
        // consisting only of the Status-Line and optional headers, and is
        // terminated by an empty line.
        StatusLine::Response { code, .. }
            if *code == 204 || *code == 304 || (*code >= 100 && *code < 200) =>
        {
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
        while !unparsed_buf.is_empty() {
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
                        kawa.blocks.push_back(Block::Cookies);
                        kawa.parsing_phase = ParsingPhase::Cookies { first: true };
                        unparsed_buf = i;
                    }
                    Err(NomErr::Incomplete(_)) => {
                        break;
                    }
                    Err(NomErr::Error(error)) | Err(NomErr::Failure(error)) => {
                        match crlf(unparsed_buf) {
                            Ok((i, _)) => {
                                need_processing = true;
                                unparsed_buf = i;
                                break;
                            }
                            Err(recovery_error) => {
                                kawa.parsing_phase =
                                    handle_recovery_error(kawa, error, recovery_error);
                                break;
                            }
                        }
                    }
                },
                ParsingPhase::Cookies { ref mut first } => {
                    match parse_single_crumb(unparsed_buf, *first) {
                        Ok((i, (key, val))) => {
                            *first = false;
                            kawa.detached.jar.push_back(Pair {
                                key: Store::new_slice(buf, key),
                                val: Store::new_slice(buf, val),
                            });
                            unparsed_buf = i;
                        }
                        Err(NomErr::Incomplete(_)) => {
                            break;
                        }
                        Err(NomErr::Error(error)) | Err(NomErr::Failure(error)) => {
                            match crlf(unparsed_buf) {
                                Ok((i, _)) => {
                                    kawa.parsing_phase = ParsingPhase::Headers;
                                    unparsed_buf = i;
                                }
                                Err(recovery_error) => {
                                    kawa.parsing_phase =
                                        handle_recovery_error(kawa, error, recovery_error);
                                    break;
                                }
                            }
                        }
                    }
                }
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
                ParsingPhase::Chunks { ref mut first } => {
                    if kawa.expects == 0 {
                        let (i, (size_hexa, size)) = match parse_chunk_header(*first, unparsed_buf)
                        {
                            Ok(ok) => {
                                *first = false;
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
                    Err(NomErr::Incomplete(_)) => {
                        break;
                    }
                    Err(NomErr::Error(error)) | Err(NomErr::Failure(error)) => {
                        match crlf(unparsed_buf) {
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
                            Err(recovery_error) => {
                                kawa.parsing_phase =
                                    handle_recovery_error(kawa, error, recovery_error);
                                break;
                            }
                        }
                    }
                },
                ParsingPhase::Terminated | ParsingPhase::Error { .. } => break,
            };
        }
        // it is absolutely essential that this line is called at the end of a parsing phase
        // do not for any reason short circuit this line
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
