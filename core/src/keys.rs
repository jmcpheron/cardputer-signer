//! The Cardputer v1.1's keyboard. A 74HC138 picks one of 8 lines, and 7 inputs read that line's
//! keys (docs/BUILDLOG.md). Lines 0-3 read the odd columns of rows 3 to 0, lines 4-7 the even
//! columns, as M5Stack's keyboard driver maps them.

/// The 4 rows of 14 keys, as printed. 0 is a key with no character (esc, del, tab, fn, shift, ok,
/// ctrl, opt, alt).
const ROWS: [&[u8; 14]; 4] = [
    b"`1234567890-=\0",
    b"\0qwertyuiop[]\\",
    b"\0\0asdfghjkl;'\0",
    b"\0\0\0zxcvbnm,./ ",
];

/// (row from the top, column from the left) of the key at (line, input).
pub fn position(line: u8, input: u8) -> (u8, u8) {
    let col = if line > 3 { 2 * input } else { 2 * input + 1 };
    (3 - line % 4, col)
}

/// The character the key at (line, input) types, lowercase, if it types one.
pub fn char_at(line: u8, input: u8) -> Option<char> {
    if line > 7 || input > 6 {
        return None;
    }
    let (row, col) = position(line, input);
    match ROWS[row as usize][col as usize] {
        0 => None,
        c => Some(c as char),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The keys the signer reads, from the bench (firmware/src/lib.rs), and the hex digits.
    #[test]
    fn the_keys() {
        assert_eq!(char_at(1, 5), Some(';'));
        assert_eq!(char_at(0, 5), Some('.'));
        assert_eq!(char_at(4, 5), Some(','));
        assert_eq!(char_at(4, 6), Some('/'));
        assert_eq!(position(1, 6), (2, 13)); // ok (Enter)
        assert_eq!(position(3, 6), (0, 13)); // del
        let mut typed = String::new();
        for line in 0..8 {
            for input in 0..7 {
                if let Some(c) = char_at(line, input) {
                    assert!(!typed.contains(c), "{c} twice");
                    typed.push(c);
                }
            }
        }
        for c in "0123456789abcdef".chars() {
            assert!(typed.contains(c), "{c}");
        }
        assert_eq!(typed.len(), 14 * 4 - 8);
    }
}
