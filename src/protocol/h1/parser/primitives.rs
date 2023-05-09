use std::arch::x86_64::_mm_lddqu_si128;

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
#[repr(align(16))]
struct CharRanges([u8; 16]);
impl std::ops::Deref for CharRanges {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[repr(align(16))]
struct CharTable([bool; 256]);
impl std::ops::Deref for CharTable {
    type Target = [bool; 256];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

macro_rules! make_bool_table {
    ($($v:expr,)*) => {
        CharTable([
            $($v != 0,)*
        ])
    }
}

#[inline]
fn take_while_simd(
    min: usize,
    predicate: impl Fn(u8) -> bool,
    ranges: &'static CharRanges,
) -> impl Fn(&[u8]) -> IResult<&[u8], &[u8]> {
    move |input| {
        use std::arch::x86_64::{
            _mm_cmpestri, _mm_loadu_si128, _SIDD_CMP_RANGES, _SIDD_LEAST_SIGNIFICANT,
            _SIDD_UBYTE_OPS,
        };

        let start = input.as_ptr() as usize;
        let mut i = input.as_ptr() as usize;
        let mut left = input.len();
        let mut found = false;

        if left >= 16 {
            let ranges16 = unsafe { _mm_loadu_si128(ranges.as_ptr() as *const _) };
            let ranges_len = ranges.len() as i32;
            loop {
                let sl = unsafe { _mm_lddqu_si128(i as *const _) };

                let idx = unsafe {
                    _mm_cmpestri(
                        ranges16,
                        ranges_len,
                        sl,
                        16,
                        _SIDD_LEAST_SIGNIFICANT | _SIDD_CMP_RANGES | _SIDD_UBYTE_OPS,
                    )
                };
                // println!(
                //     "{:?}: {}",
                //     std::str::from_utf8(&input[i - start..i - start + 16]),
                //     idx
                // );

                if idx != 16 {
                    i += idx as usize;
                    found = true;
                    break;
                }

                i += 16;
                left -= 16;

                if left < 16 {
                    break;
                }
            }
        }

        let mut i = i - start;
        if !found {
            loop {
                if !predicate(input[i]) {
                    break;
                }
                i += 1;
                if i == input.len() {
                    // println!("{:?}: incomplete", from_utf8(input));
                    return Err(NomError::Incomplete(nom::Needed::Unknown));
                }
            }
        }

        if i < min {
            // println!("{:?}: takewhile1", from_utf8(input));
            Err(error_position(input, NomErrorKind::TakeWhile1))
        } else if i == input.len() {
            // println!("{:?}: incomplete", from_utf8(input));
            return Err(NomError::Incomplete(nom::Needed::Unknown));
        } else {
            let (prefix, suffix) = input.split_at(i);
            // println!("{:?}: {:?}", from_utf8(input), from_utf8(prefix));
            Ok((suffix, prefix))
        }
    }
}

#[inline]
fn take_while_complete_simd(
    predicate: impl Fn(u8) -> bool,
    ranges: &'static CharRanges,
) -> impl Fn(&[u8]) -> IResult<&[u8], &[u8]> {
    move |input| {
        use std::arch::x86_64::{
            _mm_cmpestri, _mm_load_si128, _mm_loadu_si128, _SIDD_CMP_RANGES,
            _SIDD_LEAST_SIGNIFICANT, _SIDD_UBYTE_OPS,
        };

        let start = input.as_ptr() as usize;
        let mut i = input.as_ptr() as usize;
        let mut left = input.len();
        let mut found = false;

        if left >= 16 {
            let ranges16 = unsafe { _mm_load_si128(ranges.as_ptr() as *const _) };
            let ranges_len = ranges.len() as i32;
            loop {
                let sl = unsafe { _mm_loadu_si128(i as *const _) };

                let idx = unsafe {
                    _mm_cmpestri(
                        ranges16,
                        ranges_len,
                        sl,
                        16,
                        _SIDD_LEAST_SIGNIFICANT | _SIDD_CMP_RANGES | _SIDD_UBYTE_OPS,
                    )
                };
                // println!(
                //     "{:?}: {}",
                //     std::str::from_utf8(&input[i - start..i - start + 16]),
                //     idx
                // );

                if idx != 16 {
                    i += idx as usize;
                    found = true;
                    break;
                }

                i += 16;
                left -= 16;

                if left < 16 {
                    break;
                }
            }
        }

        let mut i = i - start;
        if !found {
            loop {
                if !predicate(input[i]) {
                    break;
                }
                i += 1;
                if i == input.len() {
                    break;
                }
            }
        }

        let (prefix, suffix) = input.split_at(i);
        // println!("{:?}: {:?}", from_utf8(input), from_utf8(prefix));
        Ok((suffix, prefix))
    }
}

//////////////////////////////////////////////////
// STREAMING PARSERS
//////////////////////////////////////////////////

#[rustfmt::skip]
const TCHAR_MAP: CharTable = make_bool_table![
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
const TCHAR_RANGES: CharRanges = CharRanges([
    0x00, 0x20, 0x3A, 0x40, 0x7F, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]);

#[rustfmt::skip]
#[allow(dead_code)]
const VCHAR_MAP: CharTable = make_bool_table![
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
const VCHAR_RANGES: CharRanges =
    CharRanges([0x00, 0x20, 0x7F, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

#[rustfmt::skip]
const CCHAR_MAP: CharTable = make_bool_table![
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
const CK_CHAR_RANGES: CharRanges = CharRanges([
    0x00, 0x20, 0x3B, 0x3B, 0x3D, 0x3D, 0x7F, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0,
]);
const CV_CHAR_RANGES: CharRanges = CharRanges([
    0x00, 0x20, 0x3B, 0x3B, 0x7F, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]);

#[rustfmt::skip]
const ACHAR_MAP: CharTable = make_bool_table![
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
const ACHAR_RANGES: CharRanges = CharRanges([
    0x00, 0x08, 0x0A, 0x1F, 0x7F, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]);

// Primitives
#[inline]
fn is_tchar(i: u8) -> bool {
    // is_alphanumeric(i) || b"!#$%&'*+-.^_`|~".contains(&i)
    // unsafe { *TCHAR_MAP.get_unchecked(i as usize) }
    TCHAR_MAP[i as usize]
}

#[inline]
fn is_vchar(i: u8) -> bool {
    i > 32 && i < 127
    // VCHAR_MAP[i as usize]
}

#[inline]
fn is_reason_char(i: u8) -> bool {
    // i == 9 || (32..=126).contains(&i)
    ACHAR_MAP[i as usize]
}

#[inline]
fn space(i: &[u8]) -> IResult<&[u8], char> {
    char(' ')(i)
}

#[inline]
pub fn crlf(i: &[u8]) -> IResult<&[u8], &[u8]> {
    tag(b"\r\n")(i)
}

// allows ISO-8859-1 characters in header values
// this is allowed in RFC 2616 but not in rfc7230
// cf https://github.com/sozu-proxy/sozu/issues/479
// #[cfg(feature = "tolerant-http1-parser")]
// fn is_header_value_char(i: u8) -> bool {
//     i == 9 || (i >= 32 && i <= 126) || i >= 160
// }

// #[cfg(not(feature = "tolerant-http1-parser"))]
fn is_header_value_char(i: u8) -> bool {
    // i == 9 || (32..=126).contains(&i)
    ACHAR_MAP[i as usize]
}

#[inline]
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
pub fn parse_request_line<'a>(buffer: &[u8], i: &'a [u8]) -> IResult<&'a [u8], StatusLine> {
    let (i, (method, _, uri, _, version, _)) = tuple((
        take_while(is_tchar),
        space,
        // take_while(is_vchar),
        take_while_simd(0, is_vchar, &VCHAR_RANGES),
        space,
        http_version,
        crlf,
    ))(i)?;

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
    let (i, (version, _, (status, code), _, reason, _)) = tuple((
        http_version,
        space,
        http_status,
        space,
        take_while(is_reason_char),
        crlf,
    ))(i)?;

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
pub fn parse_header<'a>(i: &'a [u8]) -> IResult<&'a [u8], (&'a [u8], &'a [u8])> {
    // TODO handle folding?
    let (i, (key, _, _, val, _)) = tuple((
        // take_while(is_tchar),
        // take_while_unrolled(0, is_tchar),
        take_while_simd(1, is_tchar, &TCHAR_RANGES),
        tag(":"),
        take_while(is_space),
        take_while_simd(1, is_header_value_char, &ACHAR_RANGES),
        // take_while_unrolled(1, is_header_value_char),
        // take_while(is_header_value_char),
        crlf,
    ))(i)?;

    Ok((
        i,
        (key, val),
        // Pair {
        //     key: Store::new_slice(buffer, key),
        //     val: Store::new_slice(buffer, val),
        // },
    ))
}

//////////////////////////////////////////////////
// COMPLETE PARSERS
//////////////////////////////////////////////////

#[rustfmt::skip]
const SCHEME_CHAR_MAP: CharTable = make_bool_table![
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
const AUTHORITY_CHAR_MAP: CharTable = make_bool_table![
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
const USERINFO_CHAR_MAP: CharTable = make_bool_table![
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
        // take_while_complete(is_single_crumb_key_char),
        take_while_complete_simd(is_single_crumb_key_char, &CK_CHAR_RANGES),
        opt(tuple((
            tag_complete(b"="),
            // take_while_complete(is_single_crumb_val_char),
            take_while_complete_simd(is_single_crumb_val_char, &CV_CHAR_RANGES),
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
