use log::warn;
use nom::{error::Error as NomError, Err as NomErr, Offset, ParseTo};
use std::cmp::min;
use std::mem;

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

/// Trims leading and trailing optional whitespace (OWS) as defined by
/// RFC 9110 §5.6.3: space (0x20) and horizontal tab (0x09).
#[inline]
fn trim_ows(mut data: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = data {
        data = rest;
    }
    while let [rest @ .., b' ' | b'\t'] = data {
        data = rest;
    }
    data
}

/// Returns true if the FINAL comma-separated transfer-coding token of a
/// Transfer-Encoding header value is `chunked`, case-insensitively, once
/// OWS has been trimmed from around the token (RFC 9112 §6.1).
#[inline]
fn ends_with_chunked_coding(val: &[u8]) -> bool {
    const CHUNKED: &[u8] = b"chunked";
    let last_token = val.rsplit(|&b| b == b',').next().unwrap_or(val);
    compare_no_case(trim_ows(last_token), CHUNKED)
}

fn process_headers<T: AsBuffer>(kawa: &mut Kawa<T>) {
    let buf = kawa.storage.buffer();

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

    let mut content_length = None;
    // Tracks whether the MOST RECENTLY seen Transfer-Encoding field line, on
    // its own, does NOT end in the chunked coding. RFC 9110 §5.3 makes
    // repeated Transfer-Encoding field lines equivalent to a single
    // comma-joined value in order, so the combined final transfer-coding is
    // the final coding of the LAST such line -- a split "gzip" then
    // "chunked" is the same wire semantics as one "gzip, chunked" line and
    // must not be rejected on the first line. The reject decision below is
    // therefore deferred until every header block has been walked, using
    // only the last-seen line's outcome; a later chunked-final line clears
    // this flag and applies chunked framing immediately (see the loop body).
    let mut transfer_encoding_pending_reject = false;
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
                    BodySize::Empty => {
                        content_length = Some(header);
                        kawa.body_size = BodySize::Length(length);
                    }
                    BodySize::Chunked => {
                        warn!("Found both a Transfer-Encoding and a Content-Length, ignoring the latter");
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
            } else if compare_no_case(key, b"transfer-encoding") {
                let val = header.val.data(buf);
                // RFC 9112 §6.1: "the chunked transfer coding MUST be
                // applied last" -- chunked framing is only selected when
                // "chunked" is the FINAL transfer-coding in the (possibly
                // comma-separated) value, once OWS (space/HTAB, RFC 9110
                // §5.6.3) around the value and each token is trimmed.
                // Closes sozu-proxy/sozu#726: the previous suffix-only,
                // untrimmed check let a value like "chunked\t" (trailing
                // tab) neither match nor error, leaving Content-Length
                // framing active while the malformed Transfer-Encoding
                // header stayed un-elided -- i.e. both framing headers
                // were forwarded to the backend. It also let "xchunked"
                // false-positive as chunked.
                if ends_with_chunked_coding(val) {
                    transfer_encoding_pending_reject = false;
                    match kawa.body_size {
                        BodySize::Empty => {}
                        BodySize::Chunked => {
                            warn!("Found multiple Transfer-Encoding");
                        }
                        BodySize::Length(_) => {
                            warn!("Found both a Content-Length and a Transfer-Encoding, ignoring the former");
                            if let Some(content_length) = content_length.take() {
                                content_length.elide();
                            }
                        }
                    }
                    kawa.body_size = BodySize::Chunked;
                } else {
                    // This line's own final coding is not chunked. It may
                    // still be superseded by a later Transfer-Encoding
                    // field line (the split-header case above), so defer
                    // the reject decision instead of erroring here.
                    transfer_encoding_pending_reject = true;
                }
            }
        }
    }
    if transfer_encoding_pending_reject && kawa.kind == Kind::Request {
        // RFC 9112 §6.3 mandates rejecting a REQUEST whose
        // Transfer-Encoding is present but does not end in chunked,
        // because the message body length can then not be determined
        // reliably. This rule does not apply to responses: a response
        // with a non-chunked-final Transfer-Encoding (e.g. "gzip" alone)
        // is spec-valid and its body is close-delimited (read until
        // connection close) instead of an error, so body_size is left
        // exactly as Content-Length processing above produced it
        // (Length(n) or Empty).
        kawa.parsing_phase
            .error("Transfer-Encoding present without chunked as the final coding".into());
        return;
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
