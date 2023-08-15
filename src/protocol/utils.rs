pub fn compare_no_case(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.iter()
        .zip(right)
        .all(|(a, b)| *a | 0b00_10_00_00 == *b | 0b00_10_00_00)
}

#[macro_export]
macro_rules! make_char_table {
    ($($v:expr,)*) => {
        CharTable([
            $($v != 0,)*
        ])
    }
}

#[macro_export]
macro_rules! make_char_map {
    ($predicate:ident, $($flag:expr,)*) => ([
        $($predicate($flag),)*
    ])
}

#[macro_export]
macro_rules! make_char_ranges {
    ($array:ident[$i:ident]) => {};
    ($array:ident[$i:ident] $lb:tt..$hb:tt, $($tail:tt)*) => {
        $array[$i] = $lb as u8;
        $i += 1;
        $array[$i] = $hb as u8;
        $i += 1;
        $crate::make_char_ranges!($array[$i] $($tail)*)
    };
    ($array:ident[$i:ident] $r:tt, $($tail:tt)*) => {
        $array[$i] = $r as u8;
        $i += 1;
        $array[$i] = $r as u8;
        $i += 1;
        $crate::make_char_ranges!($array[$i] $($tail)*)
    };
}

#[macro_export]
macro_rules! make_char_predicate {
    ($result:ident $c:ident) => {};
    ($result:ident $c:ident $lb:tt..$hb:tt, $($tail:tt)*) => {
        $result = $result && ($c < ($lb as u8) || $c > ($hb as u8));
        $crate::make_char_predicate!($result $c $($tail)*)
    };
    ($result:ident $c:ident $r:tt, $($tail:tt)*) => {
        $result = $result && $c != ($r as u8);
        $crate::make_char_predicate!($result $c $($tail)*)
    };
}

#[macro_export]
macro_rules! make_char_lookup {
    ($($t:tt)*) => {
        {
            let mut ranges = [0u8; 16];
            let mut length = 0;
            $crate::make_char_ranges!(ranges[length] $($t)*,);
            #[allow(unused_comparisons)]
            const fn predicate(c: u8) -> bool {
                let mut result = true;
                $crate::make_char_predicate!(result c $($t)*,);
                result
            }
            let map = $crate::make_char_map!(predicate,
                0x00,0x01,0x02,0x03,0x04,0x05,0x06,0x07,0x08,0x09,0x0A,0x0B,0x0C,0x0D,0x0E,0x0F,
                0x10,0x11,0x12,0x13,0x14,0x15,0x16,0x17,0x18,0x19,0x1A,0x1B,0x1C,0x1D,0x1E,0x1F,
                0x20,0x21,0x22,0x23,0x24,0x25,0x26,0x27,0x28,0x29,0x2A,0x2B,0x2C,0x2D,0x2E,0x2F,
                0x30,0x31,0x32,0x33,0x34,0x35,0x36,0x37,0x38,0x39,0x3A,0x3B,0x3C,0x3D,0x3E,0x3F,
                0x40,0x41,0x42,0x43,0x44,0x45,0x46,0x47,0x48,0x49,0x4A,0x4B,0x4C,0x4D,0x4E,0x4F,
                0x50,0x51,0x52,0x53,0x54,0x55,0x56,0x57,0x58,0x59,0x5A,0x5B,0x5C,0x5D,0x5E,0x5F,
                0x60,0x61,0x62,0x63,0x64,0x65,0x66,0x67,0x68,0x69,0x6A,0x6B,0x6C,0x6D,0x6E,0x6F,
                0x70,0x71,0x72,0x73,0x74,0x75,0x76,0x77,0x78,0x79,0x7A,0x7B,0x7C,0x7D,0x7E,0x7F,
                0x80,0x81,0x82,0x83,0x84,0x85,0x86,0x87,0x88,0x89,0x8A,0x8B,0x8C,0x8D,0x8E,0x8F,
                0x90,0x91,0x92,0x93,0x94,0x95,0x96,0x97,0x98,0x99,0x9A,0x9B,0x9C,0x9D,0x9E,0x9F,
                0xA0,0xA1,0xA2,0xA3,0xA4,0xA5,0xA6,0xA7,0xA8,0xA9,0xAA,0xAB,0xAC,0xAD,0xAE,0xAF,
                0xB0,0xB1,0xB2,0xB3,0xB4,0xB5,0xB6,0xB7,0xB8,0xB9,0xBA,0xBB,0xBC,0xBD,0xBE,0xBF,
                0xC0,0xC1,0xC2,0xC3,0xC4,0xC5,0xC6,0xC7,0xC8,0xC9,0xCA,0xCB,0xCC,0xCD,0xCE,0xCF,
                0xD0,0xD1,0xD2,0xD3,0xD4,0xD5,0xD6,0xD7,0xD8,0xD9,0xDA,0xDB,0xDC,0xDD,0xDE,0xDF,
                0xE0,0xE1,0xE2,0xE3,0xE4,0xE5,0xE6,0xE7,0xE8,0xE9,0xEA,0xEB,0xEC,0xED,0xEE,0xEF,
                0xF0,0xF1,0xF2,0xF3,0xF4,0xF5,0xF6,0xF7,0xF8,0xF9,0xFA,0xFB,0xFC,0xFD,0xFE,0xFF,
            );
            CharLookup {
                ranges: CharRanges(ranges),
                table: CharTable(map),
                len: length as i32,
            }
        }
    };
}

/// This macro an entire module which contains parsing utilities for a particular rule
#[macro_export]
macro_rules! compile_lookup {
    ($name:ident => [$($t:tt)*]) => {
        mod $name {
            use $crate::h1::parser::primitives::{CharLookup, CharRanges, CharTable};
            pub const LOOKUP: CharLookup = $crate::make_char_lookup!($($t)*);
            pub const TABLE: CharTable = LOOKUP.table;
            #[allow(dead_code)]
            pub const RANGES: CharRanges = LOOKUP.ranges;
            #[allow(dead_code)]
            pub const LENGTH: i32 = LOOKUP.len;

            #[inline]
            #[allow(dead_code)]
            /// Fast character lookup to decide if it fits the rule
            pub fn predicate(i: u8) -> bool {
                unsafe { *TABLE.get_unchecked(i as usize) }
            }

            #[inline]
            #[cfg(feature="simd")]
            /// Returns the longest string that fits the rule (simd optimized)
            ///
            /// *Streaming version* will return a Err::Incomplete(Needed::Unknown) if the pattern reaches the end of the input.
            fn take_while_simd(input: &[u8]) -> nom::IResult<&[u8], &[u8]> {
                use std::arch::x86_64::{
                    _mm_cmpestri, _mm_lddqu_si128, _mm_loadu_si128, _SIDD_CMP_RANGES,
                    _SIDD_LEAST_SIGNIFICANT, _SIDD_UBYTE_OPS,
                };

                let start = input.as_ptr() as usize;
                let mut i = input.as_ptr() as usize;
                let limit = input.as_ptr() as usize + input.len() - 16;
                let mut found = false;

                while i < limit {
                    let ranges_128 = unsafe { _mm_loadu_si128(RANGES.as_ptr() as *const _) };
                    let input_128 = unsafe { _mm_lddqu_si128(i as *const _) };
                    let index = unsafe {
                        _mm_cmpestri(
                            ranges_128,
                            LENGTH,
                            input_128,
                            16,
                            _SIDD_LEAST_SIGNIFICANT | _SIDD_CMP_RANGES | _SIDD_UBYTE_OPS,
                        )
                    };
                    if index != 16 {
                        i += index as usize;
                        found = true;
                        break;
                    }
                    i += 16;
                }

                let mut i = i - start;
                if !found {
                    while i < input.len() {
                        if unsafe { !TABLE.get_unchecked(*input.get_unchecked(i) as usize) } {
                            break;
                        }
                        i += 1;
                    }
                }

                if i == input.len() {
                    return Err(nom::Err::Incomplete(nom::Needed::Unknown));
                } else {
                    unsafe {
                        Ok((
                            input.get_unchecked(i..),
                            input.get_unchecked(..i),
                        ))
                    }
                }
            }

            #[inline]
            #[cfg(feature="simd")]
            /// Returns the longest string that fits the rule (simd optimized)
            fn take_while_complete_simd(input: &[u8]) -> nom::IResult<&[u8], &[u8]> {
                use std::arch::x86_64::{
                    _mm_cmpestri, _mm_lddqu_si128, _mm_loadu_si128, _SIDD_CMP_RANGES,
                    _SIDD_LEAST_SIGNIFICANT, _SIDD_UBYTE_OPS,
                };

                let start = input.as_ptr() as usize;
                let mut i = input.as_ptr() as usize;
                let limit = input.as_ptr() as usize + input.len() - 16;
                let mut found = false;

                while i < limit {
                    let ranges_128 = unsafe { _mm_loadu_si128(RANGES.as_ptr() as *const _) };
                    let input_128 = unsafe { _mm_lddqu_si128(i as *const _) };
                    let index = unsafe {
                        _mm_cmpestri(
                            ranges_128,
                            LENGTH,
                            input_128,
                            16,
                            _SIDD_LEAST_SIGNIFICANT | _SIDD_CMP_RANGES | _SIDD_UBYTE_OPS,
                        )
                    };
                    if index != 16 {
                        i += index as usize;
                        found = true;
                        break;
                    }
                    i += 16;
                }

                let mut i = i - start;
                if !found {
                    while i < input.len() {
                        if unsafe { !TABLE.get_unchecked(*input.get_unchecked(i) as usize) } {
                            break;
                        }
                        i += 1;
                    }
                }

                unsafe {
                    Ok((
                        input.get_unchecked(i..),
                        input.get_unchecked(..i),
                    ))
                }
            }

            #[inline]
            #[allow(dead_code)]
            /// Returns the longest string that fits the rule (not simd optimized)
            ///
            /// *Streaming version* will return a Err::Incomplete(Needed::Unknown) if the pattern reaches the end of the input.
            pub fn take_while(input: &[u8]) -> nom::IResult<&[u8], &[u8]> {
                let mut i = 0;
                while i < input.len() {
                    if unsafe { !TABLE.get_unchecked(*input.get_unchecked(i) as usize) } {
                        break;
                    }
                    i += 1;
                }
                if i == input.len() {
                    return Err(nom::Err::Incomplete(nom::Needed::Unknown));
                } else {
                    unsafe {
                        Ok((
                            input.get_unchecked(i..),
                            input.get_unchecked(..i),
                        ))
                    }
                }
            }

            #[inline]
            #[allow(dead_code)]
            /// Returns the longest string that fits the rule (not simd optimized)
            pub fn take_while_complete(input: &[u8]) -> nom::IResult<&[u8], &[u8]> {
                let mut i = 0;
                while i < input.len() {
                    if unsafe { !TABLE.get_unchecked(*input.get_unchecked(i) as usize) } {
                        break;
                    }
                    i += 1;
                }
                unsafe {
                    Ok((
                        input.get_unchecked(i..),
                        input.get_unchecked(..i),
                    ))
                }
            }

            #[inline]
            #[allow(dead_code)]
            /// Returns the longest string that fits the rule (using simd if enabled)
            ///
            /// *Streaming version* will return a Err::Incomplete(Needed::Unknown) if the pattern reaches the end of the input.
            pub fn take_while_fast(input: &[u8]) -> nom::IResult<&[u8], &[u8]> {
                #[cfg(feature="simd")]
                let result = take_while_simd(input);
                #[cfg(not(feature="simd"))]
                let result = take_while(input);
                result
            }

            #[inline]
            #[allow(dead_code)]
            /// Returns the longest string that fits the rule (using simd if enabled)
            pub fn take_while_complete_fast(input: &[u8]) -> nom::IResult<&[u8], &[u8]> {
                #[cfg(feature="simd")]
                let result = take_while_complete_simd(input);
                #[cfg(not(feature="simd"))]
                let result = take_while_complete(input);
                result
            }
        }
    }
}
