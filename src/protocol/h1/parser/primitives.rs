use std::str::from_utf8_unchecked;

use nom::{
    bytes::{
        complete::{take_while, take_while1},
        streaming::{tag, take},
    },
    character::{
        is_alphanumeric, is_space,
        streaming::{char, hex_digit1, one_of},
    },
    combinator::opt,
    error::{make_error, ErrorKind as NomErrorKind, ParseError},
    sequence::tuple,
    Err as NomError, IResult,
};

use crate::{
    protocol::utils::compare_no_case,
    storage::{Header, StatusLine, Store, Version},
};

fn error_position<I, E: ParseError<I>>(i: I, kind: NomErrorKind) -> NomError<E> {
    NomError::Error(make_error(i, kind))
}

// Primitives
fn is_token_char(i: u8) -> bool {
    is_alphanumeric(i) || b"!#$%&'*+-.^_`|~".contains(&i)
}

fn token(i: &[u8]) -> IResult<&[u8], &[u8]> {
    take_while(is_token_char)(i)
}

fn is_status_token_char(i: u8) -> bool {
    i >= 32 && i != 127
}

fn status_token(i: &[u8]) -> IResult<&[u8], &[u8]> {
    take_while(is_status_token_char)(i)
}

fn space(i: &[u8]) -> IResult<&[u8], char> {
    char(' ')(i)
}

pub fn crlf(i: &[u8]) -> IResult<&[u8], &[u8]> {
    tag(b"\r\n")(i)
}

fn is_vchar(i: u8) -> bool {
    i > 32 && i <= 126
}

// allows ISO-8859-1 characters in header values
// this is allowed in RFC 2616 but not in rfc7230
// cf https://github.com/sozu-proxy/sozu/issues/479
#[cfg(feature = "tolerant-http1-parser")]
fn is_header_value_char(i: u8) -> bool {
    i == 9 || (i >= 32 && i <= 126) || i >= 160
}

#[cfg(not(feature = "tolerant-http1-parser"))]
fn is_header_value_char(i: u8) -> bool {
    i == 9 || (32..=126).contains(&i)
}

fn vchar_1(i: &[u8]) -> IResult<&[u8], &[u8]> {
    take_while(is_vchar)(i)
}

fn http_version(i: &[u8]) -> IResult<&[u8], Version> {
    let (i, _) = tag("HTTP/1.")(i)?;
    let (i, minor) = one_of("01")(i)?;

    Ok((
        i,
        if minor == '0' {
            Version::V10
        } else {
            Version::V11
        },
    ))
}

fn http_status(i: &[u8]) -> IResult<&[u8], (&[u8], u16)> {
    let (i, status) = take(3usize)(i)?;
    let code = std::str::from_utf8(status)
        .ok()
        .and_then(|status| status.parse::<u16>().ok());
    match code {
        Some(code) => Ok((i, (status, code))),
        None => Err(error_position(i, NomErrorKind::MapRes)),
    }
}

/// parse first line of HTTP request into RawStatusLine, including terminating CRLF
///
/// example: `GET www.clever.cloud.com HTTP/1.1\r\n`
pub fn parse_request_line<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], StatusLine> {
    let (i, method) = token(i)?;
    let (i, _) = space(i)?;
    let (i, uri) = vchar_1(i)?; // ToDo proper URI parsing?
    let (i, _) = space(i)?;
    let (i, version) = http_version(i)?;
    let (i, _) = crlf(i)?;

    Ok((
        i,
        StatusLine::Request {
            version,
            method: Store::new_slice(buffer, method),
            uri: Store::new_slice(buffer, uri),
            authority: Store::Empty,
            path: Store::Empty,
        },
    ))
}

/// parse first line of HTTP response into RawStatusLine, including terminating CRLF
///
/// example: `HTTP/1.1 200 OK\r\n`
pub fn parse_response_line<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], StatusLine> {
    let (i, (version, _, (status, code), _, reason, _)) =
        tuple((http_version, space, http_status, space, status_token, crlf))(i)?;

    Ok((
        i,
        StatusLine::Response {
            version,
            code,
            status: Store::new_slice(buffer, status),
            reason: Store::new_slice(buffer, reason),
        },
    ))
}

/// parse a HTTP header, including terminating CRLF
///
/// example: `Content-Length: 42\r\n`
pub fn parse_header<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], Header> {
    // TODO handle folding?
    let (i, (key, _, _, val, _)) = tuple((
        token,
        tag(":"),
        take_while(is_space),
        take_while(is_header_value_char),
        crlf,
    ))(i)?;

    Ok((
        i,
        Header {
            key: Store::new_slice(buffer, key),
            val: Store::new_slice(buffer, val),
        },
    ))
}

fn is_single_crumb_char(i: u8) -> bool {
    i != 59
}

/// parse a single crumb from a Cookie header
///
/// examples:
/// ```txt
/// crumb          -> "crumb"
/// crumb1; crumb2 -> "crumb1"
/// ```
pub fn parse_single_crumb<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], Store> {
    let (i, value) = take_while1(is_single_crumb_char)(i)?;
    println!("{}", unsafe { from_utf8_unchecked(value) });
    let value = Store::new_slice(buffer, value);
    if i.is_empty() {
        return Ok((i, value));
    }
    let (i, _) = tag(b"; ")(i)?;
    Ok((i, value))
}

pub fn chunk_size(i: &[u8]) -> IResult<&[u8], (&[u8], usize)> {
    let (i, size_hexa) = hex_digit1(i)?;
    let size = std::str::from_utf8(size_hexa)
        .ok()
        .and_then(|chunk_size| usize::from_str_radix(chunk_size, 16).ok());
    match size {
        Some(size) => Ok((i, (size_hexa, size))),
        None => Err(error_position(i, NomErrorKind::MapRes)),
    }
}

pub fn parse_chunk_header(first: bool, i: &[u8]) -> IResult<&[u8], (&[u8], usize)> {
    if first {
        let (i, (size, _)) = tuple((chunk_size, crlf))(i)?;
        Ok((i, size))
    } else {
        let (i, (_, size, _)) = tuple((crlf, chunk_size, crlf))(i)?;
        Ok((i, size))
    }
}

fn is_scheme_char(i: u8) -> bool {
    is_alphanumeric(i) || b"+-.".contains(&i)
}
fn is_authority_char(i: u8) -> bool {
    !b"/?#".contains(&i)
}
fn is_userinfo_char(i: u8) -> bool {
    !b"/?#\\@".contains(&i)
}
fn userinfo(i: &[u8]) -> IResult<&[u8], &[u8]> {
    let (i, (userinfo, _)) = tuple((take_while(is_userinfo_char), tag(b"@")))(i)?;
    Ok((i, userinfo))
}

/// ```txt
/// server-wide:         OPTIONS * HTTP/1.1                                      -> (Empty, "*")
/// origin:              OPTIONS /index.html                                     -> (Empty, "/index.html")
/// absolute+empty path: OPTIONS http://www.example.org:8001 HTTP/1.1            -> ("www.example.org:8001", "*")
/// absolute:            OPTIONS http://www.example.org:8001/index.html HTTP/1.1 -> ("www.example.org:8001", "/index.html")
/// ```
fn parse_asterisk_form<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], (Store, Store)> {
    if i == b"*" {
        Ok((i, (Store::Static(b"*"), Store::Empty)))
    } else if i[0] == b'/' {
        parse_origin_form(buffer, i)
    } else {
        parse_absolute_form(buffer, i)
    }
}
/// ```txt
/// www.example.org:8001 -> ("www.example.org:8001", "/")
/// ```
fn parse_authority_form<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], (Store, Store)> {
    Ok((&[], (Store::new_slice(buffer, i), Store::Static(b"/"))))
}
/// ```txt
/// /index.html?k=v#h -> (Empty, "/index.html?k=v#h")
/// ```
fn parse_origin_form<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], (Store, Store)> {
    Ok((&[], (Store::Empty, Store::new_slice(buffer, i))))
}
/// ```txt
/// http://www.example.org:8001                            -> ("www.example.org:8001", "/")
/// http://www.example.org:8001?k=v#h                      -> ("www.example.org:8001", "?k=v#h")
/// http://www.example.org:8001/index.html?k=v#h           -> ("www.example.org:8001", "/index.html?k=v#h")
/// http://user:pass@www.example.org:8001/index.html?k=v#h -> ("www.example.org:8001", "/index.html?k=v#h")
/// ```
fn parse_absolute_form<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], (Store, Store)> {
    let (path, (_scheme, _, _userinfo, authority)) = tuple((
        take_while(is_scheme_char),
        tag(b"://"),
        opt(userinfo),
        take_while(is_authority_char),
    ))(i)?;
    let authority = Store::new_slice(buffer, authority);
    let path = if path.is_empty() {
        Store::Static(b"/")
    } else {
        Store::new_slice(buffer, path)
    };
    Ok((&[], (authority, path)))
}

pub fn parse_url(method: &[u8], buffer: &[u8], i: &[u8]) -> Option<(Store, Store)> {
    if i.is_empty() {
        return Some((Store::Empty, Store::Static(b"/")));
    }
    let url = if compare_no_case(method, b"OPTIONS") {
        parse_asterisk_form(buffer, i)
    } else if compare_no_case(method, b"CONNECT") {
        parse_authority_form(buffer, i)
    } else if i[0] == b'/' {
        parse_origin_form(buffer, i)
    } else {
        parse_authority_form(buffer, i)
    };
    match url {
        Ok((_, url)) => Some(url),
        _ => None,
    }
}
