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
