use nom::{
    bytes::{
        complete::{
            tag as tag_complete, take_while as take_while_complete,
            take_while1 as take_while1_complete,
        },
        streaming::{tag, take, take_while},
    },
    character::{
        complete::char as char_complete,
        is_space,
        streaming::{char, hex_digit1, one_of},
    },
    combinator::opt,
    error::{make_error, ErrorKind as NomErrorKind, ParseError},
    sequence::tuple,
    Err as NomError, IResult,
};

use crate::{
    protocol::utils::compare_no_case,
    storage::{Pair, StatusLine, Store, Version},
};

fn error_position<I, E: ParseError<I>>(i: I, kind: NomErrorKind) -> NomError<E> {
    NomError::Error(make_error(i, kind))
}

macro_rules! make_bool_table {
    ($($v:expr,)*) => ([
        $($v != 0,)*
    ])
}

//////////////////////////////////////////////////
// STREAMING PARSERS
//////////////////////////////////////////////////

#[rustfmt::skip]
const TCHAR_MAP: [bool; 256] = make_bool_table![
    // Control characters
// \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
//                                  \e
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    // Visible characters
// SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    0, 1, 0, 1, 1, 1, 1, 1, 0, 0, 1, 1, 0, 1, 1, 0,
//  0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0,
//  @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 1, 1,
//  `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 0,

    // Non ascii characters
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

#[rustfmt::skip]
#[allow(dead_code)]
const VCHAR_MAP: [bool; 256] = make_bool_table![
    // Control characters
// \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0,
//                                  \e
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    // Visible characters
// SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,

    // Non ascii characters
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

#[rustfmt::skip]
const CCHAR_MAP: [bool; 256] = make_bool_table![
    // Control characters
// \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
//                                  \e
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    // Visible characters
// SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 1,
//  @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,

    // Non ascii characters
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

#[rustfmt::skip]
const ACHAR_MAP: [bool; 256] = make_bool_table![
    // Control characters
// \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0,
//                                  \e
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    // Visible characters
// SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,

    // Non ascii characters
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

// Primitives
fn is_tchar(i: u8) -> bool {
    // is_alphanumeric(i) || b"!#$%&'*+-.^_`|~".contains(&i)
    // unsafe { *TCHAR_MAP.get_unchecked(i as usize) }
    TCHAR_MAP[i as usize]
}
fn token(i: &[u8]) -> IResult<&[u8], &[u8]> {
    take_while(is_tchar)(i)
}

fn is_vchar(i: u8) -> bool {
    i > 32 && i < 127
    // VCHAR_MAP[i as usize]
}
fn value_token(i: &[u8]) -> IResult<&[u8], &[u8]> {
    take_while(is_vchar)(i)
}

fn is_reason_char(i: u8) -> bool {
    // i == 9 || (32..=126).contains(&i)
    ACHAR_MAP[i as usize]
}
fn reason_token(i: &[u8]) -> IResult<&[u8], &[u8]> {
    take_while(is_reason_char)(i)
}

fn space(i: &[u8]) -> IResult<&[u8], char> {
    char(' ')(i)
}

pub fn crlf(i: &[u8]) -> IResult<&[u8], &[u8]> {
    tag(b"\r\n")(i)
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
    // i == 9 || (32..=126).contains(&i)
    ACHAR_MAP[i as usize]
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
    let (i, (method, _, uri, _, version, _)) =
        tuple((token, space, value_token, space, http_version, crlf))(i)?;

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
        tuple((http_version, space, http_status, space, reason_token, crlf))(i)?;

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
pub fn parse_header<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], Pair> {
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
        Pair {
            key: Store::new_slice(buffer, key),
            val: Store::new_slice(buffer, val),
        },
    ))
}

//////////////////////////////////////////////////
// COMPLETE PARSERS
//////////////////////////////////////////////////

#[rustfmt::skip]
const SCHEME_CHAR_MAP: [bool; 256] = make_bool_table![
    // Control characters
// \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
//                                  \e
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    // Visible characters
// SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 0,
//  0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0,
//  @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0,
//  `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0,

    // Non ascii characters
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

#[rustfmt::skip]
const AUTHORITY_CHAR_MAP: [bool; 256] = make_bool_table![
    // Control characters
// \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
//                                  \e
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    // Visible characters
// SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    0, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,
//  0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,
//  @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,

    // Non ascii characters
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

#[rustfmt::skip]
const USERINFO_CHAR_MAP: [bool; 256] = make_bool_table![
    // Control characters
// \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
//                                  \e
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    // Visible characters
// SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    0, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,
//  0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,
//  @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1,
//  `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
//  p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,

    // Non ascii characters
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// not ";" nor "="
fn is_single_crumb_key_char(i: u8) -> bool {
    // i != 59 && i != 61
    i != 61 && CCHAR_MAP[i as usize]
}

/// not ";"
fn is_single_crumb_val_char(i: u8) -> bool {
    // i != 59
    CCHAR_MAP[i as usize]
}

/// parse a single crumb from a Cookie header
///
/// examples:
/// ```txt
/// crumb=0          -> ("crumb", "0")
/// crumb=1; crumb=2 -> ("crumb", "1")
/// ```
pub fn parse_single_crumb<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], Pair> {
    let (i, (key, val)) = tuple((
        take_while_complete(is_single_crumb_key_char),
        opt(tuple((
            tag_complete(b"="),
            take_while_complete(is_single_crumb_val_char),
        ))),
    ))(i)?;

    let crumb = match val {
        Some((_, val)) => Pair {
            key: Store::new_detached(buffer, key),
            val: Store::new_detached(buffer, val),
        },
        None => Pair {
            key: Store::Static(b""),
            val: Store::new_detached(buffer, key),
        },
    };
    if i.is_empty() {
        return Ok((i, crumb));
    }
    let (i, _) = tag_complete(b"; ")(i)?;
    Ok((i, crumb))
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
    // is_alphanumeric(i) || b"+-.".contains(&i)
    SCHEME_CHAR_MAP[i as usize]
}
fn is_authority_char(i: u8) -> bool {
    // !b"/?#".contains(&i)
    AUTHORITY_CHAR_MAP[i as usize]
}
fn is_userinfo_char(i: u8) -> bool {
    // !b"/?#\\@".contains(&i)
    USERINFO_CHAR_MAP[i as usize]
}
fn userinfo(i: &[u8]) -> IResult<&[u8], &[u8]> {
    let (i, (userinfo, _)) =
        tuple((take_while1_complete(is_userinfo_char), char_complete('@')))(i)?;
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
        take_while1_complete(is_scheme_char),
        tag_complete(b"://"),
        opt(userinfo),
        take_while1_complete(is_authority_char),
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
