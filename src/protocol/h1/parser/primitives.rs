use nom::{
    bytes::{
        complete::{tag as tag_complete, take_while as take_while_complete},
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
    compile_lookup, make_char_table,
    protocol::utils::compare_no_case,
    storage::{Store, Version},
};

fn error_position<I, E: ParseError<I>>(i: I, kind: NomErrorKind) -> NomError<E> {
    NomError::Error(make_error(i, kind))
}

/// A set of rules to decide if a character is valid or not
pub struct CharLookup {
    ranges: CharRanges,
    table: CharTable,
    len: i32,
}

/// Character lookup table, it indicates for each character if it passes or not
#[repr(align(16))]
pub struct CharTable([bool; 256]);

impl std::ops::Deref for CharTable {
    type Target = [bool; 256];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Character invalid ranges, defines up to 8 ranges of invalid characters
#[repr(align(16))]
pub struct CharRanges([u8; 16]);

impl std::ops::Deref for CharRanges {
    type Target = [u8; 16];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

//////////////////////////////////////////////////
// STREAMING PARSERS
//////////////////////////////////////////////////

/*
    Creates a tchar module for parsing header keys and http methods.

    Control characters
   \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    Visible characters
   SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    0, 1, X, 1, 1, 1, 1, 1, 0, 0, 1, 1, 0, 1, 1, X,
    0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0,
    @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 1, 1,
    `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 0,

    note: _mm_cmpestri can only hold 8 ranges, " and / are invalid tchars but will slip through.
    It should be acceptable as tchars are delimited by spaces or colons and Kawa is not an HTTP
    validator, it parses the strict minimum to extract an higher representation. Nonetheless, the
    parsers are strict enough to ensure all slices are valid UTF-8, so from_utf8_uncheck can be
    used on them.
*/
compile_lookup!(tchar => [0x00..0x20, '('..')', '['..']', '{', '}', ',', ':'..'@', 0x7F..0xFF]);

/*
    Creates a vchar module for preparsing URIs.

    Control characters
   \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    Visible characters
   SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,
*/
compile_lookup!(vchar => [0x00..0x20, 0x7F..0xFF]);

/*
    Creates a ck_char and cv_char module for parsing cookie keys and values respectively.

    Control characters
   \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    Visible characters
   SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    ?, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, ?, 1, 1,
    @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,

    note: cookie values can contain equal signs and spaces, not keys. A key can't contain colons.
*/
compile_lookup!(ck_char => [0x00..0x20, ';', '=', 0x7F..0xFF]);
compile_lookup!(cv_char => [0x00..0x1F, ';', 0x7F..0xFF]);

/*
    Creates a achar module for parsing header values and http reasons.

    Control characters
   \0                   \a \b \t \n \v \f \r
    0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    Visible characters
   SP  !  "  #  $  %  &  '  (  )  *  +  ,  -  .  /
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    0  1  2  3  4  5  6  7  8  9  :  ;  <  =  >  ?
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    @  A  B  C  D  E  F  G  H  I  J  K  L  M  N  O
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    P  Q  R  S  T  U  V  W  X  Y  Z  [  \  ]  ^  _
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    `  a  b  c  d  e  f  g  h  i  j  k  l  m  n  o
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    p  q  r  s  t  u  v  w  x  y  z  {  |  }  ~
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,
*/
compile_lookup!(achar => [0x00..0x08, 0x0A..0x1F, 0x7F..0xFF]);

#[inline]
fn space(i: &[u8]) -> IResult<&[u8], char> {
    char(' ')(i)
}

#[inline]
pub fn crlf(i: &[u8]) -> IResult<&[u8], &[u8]> {
    tag(b"\r\n")(i)
}

#[inline]
fn http_version(i: &[u8]) -> IResult<&[u8], Version> {
    let (i, _) = tag(b"HTTP/1.")(i)?;
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

#[inline]
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
#[inline]
#[allow(clippy::type_complexity)]
pub fn parse_request_line(i: &[u8]) -> IResult<&[u8], (&[u8], &[u8], Version)> {
    let (i, method) = tchar::take_while_fast(i)?;
    let (i, _) = space(i)?;
    let (i, uri) = vchar::take_while_fast(i)?;
    let (i, _) = space(i)?;
    let (i, version) = http_version(i)?;
    let (i, _) = crlf(i)?;
    Ok((i, (method, uri, version)))
}

/// parse first line of HTTP response into RawStatusLine, including terminating CRLF
///
/// example: `HTTP/1.1 200 OK\r\n`
#[inline]
#[allow(clippy::type_complexity)]
pub fn parse_response_line(i: &[u8]) -> IResult<&[u8], (Version, &[u8], u16, &[u8])> {
    let (i, version) = http_version(i)?;
    let (i, _) = space(i)?;
    let (i, (status, code)) = http_status(i)?;
    let (i, _) = space(i)?;
    let (i, reason) = achar::take_while_fast(i)?;
    let (i, _) = crlf(i)?;
    Ok((i, (version, status, code, reason)))
}

/// parse a HTTP header, including terminating CRLF
/// if it is a cookie header, nothing is returned and parse_single_crumb should be called
///
/// example: `Content-Length: 42\r\n`
#[inline]
#[allow(clippy::type_complexity)]
pub fn parse_header_or_cookie(i: &[u8]) -> IResult<&[u8], Option<(&[u8], &[u8])>> {
    let (i, key) = tchar::take_while_fast(i)?;
    let (i, _) = tag(b":")(i)?;
    let (i, _) = take_while(is_space)(i)?;
    if compare_no_case(key, b"cookie") {
        return Ok((i, None));
    }
    let (i, val) = achar::take_while_fast(i)?;
    let (i, _) = crlf(i)?;
    Ok((i, Some((key, val))))
}

/// parse a HTTP header, including terminating CRLF
/// note: treat cookie headers as regular headers
///
/// example: `Content-Length: 42\r\n`
#[inline]
pub fn parse_header(i: &[u8]) -> IResult<&[u8], (&[u8], &[u8])> {
    let (i, key) = tchar::take_while_fast(i)?;
    let (i, _) = tag(b":")(i)?;
    let (i, _) = take_while(is_space)(i)?;
    let (i, val) = achar::take_while_fast(i)?;
    let (i, _) = crlf(i)?;
    Ok((i, (key, val)))
}

/// parse a single crumb from a Cookie header
///
/// examples:
/// ```txt
/// crumb=0          -> ("crumb", "0")
/// crumb=1; crumb=2 -> ("crumb", "1")
/// ```
#[inline]
#[allow(clippy::type_complexity)]
pub fn parse_single_crumb(i: &[u8], first: bool) -> IResult<&[u8], (&[u8], &[u8])> {
    let i = if !first {
        let (i, _) = tag(b"; ")(i)?;
        i
    } else {
        i
    };
    let (i, key) = ck_char::take_while_fast(i)?;
    let (i, val) = opt(tuple((tag(b"="), cv_char::take_while_fast)))(i)?;

    match val {
        Some((_, val)) => Ok((i, (key, val))),
        None => Ok((i, (&key[..0], key))),
    }
}

//////////////////////////////////////////////////
// COMPLETE PARSERS
//////////////////////////////////////////////////

#[rustfmt::skip]
const SCHEME_CHAR_MAP: CharTable = make_char_table![
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
const AUTHORITY_CHAR_MAP: CharTable = make_char_table![
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
const USERINFO_CHAR_MAP: CharTable = make_char_table![
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

#[inline]
fn is_scheme_char(i: u8) -> bool {
    SCHEME_CHAR_MAP[i as usize]
}
#[inline]
fn is_authority_char(i: u8) -> bool {
    AUTHORITY_CHAR_MAP[i as usize]
}
#[inline]
fn is_userinfo_char(i: u8) -> bool {
    USERINFO_CHAR_MAP[i as usize]
}

#[inline]
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

#[inline]
pub fn parse_chunk_header(first: bool, i: &[u8]) -> IResult<&[u8], (&[u8], usize)> {
    if first {
        let (i, size) = chunk_size(i)?;
        let (i, _) = crlf(i)?;
        Ok((i, size))
    } else {
        let (i, _) = crlf(i)?;
        let (i, size) = chunk_size(i)?;
        let (i, _) = crlf(i)?;
        Ok((i, size))
    }
}

#[inline]
fn userinfo(i: &[u8]) -> IResult<&[u8], &[u8]> {
    let (i, userinfo) = take_while_complete(is_userinfo_char)(i)?;
    let (i, _) = char_complete('@')(i)?;
    Ok((i, userinfo))
}

/// ```txt
/// server-wide:         OPTIONS * HTTP/1.1                                      -> (Empty, "*")
/// origin:              OPTIONS /index.html                                     -> (Empty, "/index.html")
/// absolute+empty path: OPTIONS http://www.example.org:8001 HTTP/1.1            -> ("www.example.org:8001", "*")
/// absolute:            OPTIONS http://www.example.org:8001/index.html HTTP/1.1 -> ("www.example.org:8001", "/index.html")
/// ```
#[inline]
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
#[inline]
fn parse_authority_form<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], (Store, Store)> {
    Ok((&[], (Store::new_slice(buffer, i), Store::Static(b"/"))))
}
/// ```txt
/// /index.html?k=v#h -> (Empty, "/index.html?k=v#h")
/// ```
#[inline]
fn parse_origin_form<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], (Store, Store)> {
    Ok((&[], (Store::Empty, Store::new_slice(buffer, i))))
}
/// ```txt
/// http://www.example.org:8001                            -> ("www.example.org:8001", "/")
/// http://www.example.org:8001?k=v#h                      -> ("www.example.org:8001", "?k=v#h")
/// http://www.example.org:8001/index.html?k=v#h           -> ("www.example.org:8001", "/index.html?k=v#h")
/// http://user:pass@www.example.org:8001/index.html?k=v#h -> ("www.example.org:8001", "/index.html?k=v#h")
/// ```
#[inline]
fn parse_absolute_form<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], (Store, Store)> {
    let (i, _scheme) = take_while_complete(is_scheme_char)(i)?;
    let (i, _) = tag_complete(b"://")(i)?;
    let (i, _userinfo) = opt(userinfo)(i)?;
    let (path, authority) = take_while_complete(is_authority_char)(i)?;

    let authority = Store::new_slice(buffer, authority);
    let path = if path.is_empty() {
        Store::Static(b"/")
    } else {
        Store::new_slice(buffer, path)
    };
    Ok((&[], (authority, path)))
}

#[inline]
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
