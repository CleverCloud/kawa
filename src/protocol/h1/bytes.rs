use nom::{
    bytes::streaming::{tag, take, take_while},
    character::{
        is_alphanumeric, is_space,
        streaming::{char, hex_digit1, one_of},
    },
    combinator::map_res,
    error::{make_error, ErrorKind as NomErrorKind, ParseError},
    sequence::tuple,
    Err as NomError, IResult,
};

use crate::htx::{Header, StatusLine, Store, Version};

fn error_position<I, E: ParseError<I>>(input: I, kind: NomErrorKind) -> NomError<E> {
    NomError::Error(make_error(input, kind))
}

pub fn compare_no_case(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.iter().zip(right).all(|(a, b)| match (*a, *b) {
        (0..=64, 0..=64) | (91..=96, 91..=96) | (123..=255, 123..=255) => a == b,
        (65..=90, 65..=90) | (97..=122, 97..=122) | (65..=90, 97..=122) | (97..=122, 65..=90) => {
            *a | 0b00_10_00_00 == *b | 0b00_10_00_00
        }
        _ => false,
    })
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
    tag("\r\n")(i)
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
            scheme: Store::Static(b"HTTP"),
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

//not a space nor a comma
//
// allows ISO-8859-1 characters in header values
// this is allowed in RFC 2616 but not in rfc7230
// cf https://github.com/sozu-proxy/sozu/issues/479
#[cfg(feature = "tolerant-http1-parser")]
fn is_single_header_value_char(i: u8) -> bool {
    (i > 33 && i <= 126 && i != 44) || i >= 160
}

pub fn chunk_size(input: &[u8]) -> IResult<&[u8], usize> {
    let (i, s) = map_res(hex_digit1, std::str::from_utf8)(input)?;
    match usize::from_str_radix(s, 16) {
        Ok(sz) => Ok((i, sz)),
        Err(_) => Err(error_position(input, NomErrorKind::MapRes)),
    }
}

pub fn parse_chunk_header(i: &[u8]) -> IResult<&[u8], usize> {
    let (i, (_, size, _)) = tuple((crlf, chunk_size, crlf))(i)?;
    Ok((i, size))
}
