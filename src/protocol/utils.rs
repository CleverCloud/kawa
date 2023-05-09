pub fn compare_no_case(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.iter()
        .zip(right)
        .all(|(a, b)| *a | 0b00_10_00_00 == *b | 0b00_10_00_00)

    // left.iter().zip(right).all(|(a, b)| match (*a, *b) {
    //     (0..=64, 0..=64) | (91..=96, 91..=96) | (123..=255, 123..=255) => a == b,
    //     (65..=90, 65..=90) | (97..=122, 97..=122) | (65..=90, 97..=122) | (97..=122, 65..=90) => {
    //         *a | 0b00_10_00_00 == *b | 0b00_10_00_00
    //     }
    //     _ => false,
    // })
}

// pub fn compare_no_case_simd(left: &[u8], right: &[u8]) -> bool {
//     use std::arch::x86_64::{
//         _mm_cmpestri, _mm_load_si128, _mm_loadu_si128, _mm_or_si128, _SIDD_CMP_EQUAL_ORDERED,
//     };
//     let la = left.len() as i32;
//     let lb = right.len() as i32;
//     if la != lb {
//         return false;
//     }

//     const MASK: &[u8; 16] = &[0b0010_0000; 16];
//     let mask = unsafe { _mm_load_si128(MASK.as_ptr() as *const _) };
//     let left = unsafe { _mm_loadu_si128(left.as_ptr() as *const _) };
//     let right = unsafe { _mm_load_si128(right.as_ptr() as *const _) };
//     let left = unsafe { _mm_or_si128(left, mask) };
//     let result = unsafe { _mm_cmpestri(left, la, right, lb, _SIDD_CMP_EQUAL_ORDERED) };
//     result == 0
// }
