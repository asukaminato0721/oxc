/// Split `input` on a separator that is not nested in parentheses, brackets, braces, or quotes.
pub fn segment(input: &str, separator: u8) -> Vec<&str> {
    let bytes = input.as_bytes();
    let mut closing = Vec::with_capacity(8);
    let mut parts = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if closing.is_empty() && byte == separator {
            parts.push(&input[start..index]);
            start = index + 1;
            index += 1;
            continue;
        }
        match byte {
            b'\\' => index += 2,
            b'\'' | b'"' => {
                let quote = byte;
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => index += 2,
                        current if current == quote => {
                            index += 1;
                            break;
                        }
                        _ => index += 1,
                    }
                }
            }
            b'(' => {
                closing.push(b')');
                index += 1;
            }
            b'[' => {
                closing.push(b']');
                index += 1;
            }
            b'{' => {
                closing.push(b'}');
                index += 1;
            }
            b')' | b']' | b'}' => {
                if closing.last() == Some(&byte) {
                    closing.pop();
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    parts.push(&input[start..]);
    parts
}

pub fn is_valid_arbitrary(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut closing = Vec::with_capacity(8);
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            quote @ (b'\'' | b'"') => {
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => index += 2,
                        current if current == quote => {
                            index += 1;
                            break;
                        }
                        _ => index += 1,
                    }
                }
            }
            b'(' => {
                closing.push(b')');
                index += 1;
            }
            b'[' => {
                closing.push(b']');
                index += 1;
            }
            byte @ (b')' | b']' | b'}') => {
                if closing.pop() != Some(byte) {
                    return false;
                }
                index += 1;
            }
            b';' if closing.is_empty() => return false,
            _ => index += 1,
        }
    }
    closing.is_empty()
}

/// Decode Tailwind's underscore spelling for arbitrary values. Underscores in the first argument
/// of `var()` are identifiers and therefore remain underscores.
pub fn decode_arbitrary(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    let mut var_depth = 0_u32;
    let mut var_first_argument = false;
    while index < bytes.len() {
        if bytes[index] == b'\\' && bytes.get(index + 1) == Some(&b'_') {
            output.push('_');
            index += 2;
            continue;
        }
        if input[index..].starts_with("var(") {
            output.push_str("var(");
            index += 4;
            var_depth += 1;
            var_first_argument = true;
            continue;
        }
        match bytes[index] {
            b'_' if !var_first_argument => output.push(' '),
            b'_' => output.push('_'),
            b',' if var_depth > 0 => {
                output.push(',');
                var_first_argument = false;
            }
            b')' if var_depth > 0 => {
                output.push(')');
                var_depth -= 1;
                var_first_argument = false;
            }
            _ => {
                let character = input[index..].chars().next().expect("index is in bounds");
                output.push(character);
                index += character.len_utf8();
                continue;
            }
        }
        index += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{decode_arbitrary, is_valid_arbitrary, segment};

    #[test]
    fn splits_only_at_top_level() {
        assert_eq!(segment("hover:[&:focus]:block", b':'), ["hover", "[&:focus]", "block"]);
        assert_eq!(segment("bg-[url('a/b')]/50", b'/'), ["bg-[url('a/b')]", "50"]);
    }

    #[test]
    fn validates_arbitrary_values() {
        assert!(is_valid_arbitrary("color-mix(in_oklab,red_50%,blue)"));
        assert!(!is_valid_arbitrary("red;display:block"));
        assert!(!is_valid_arbitrary("calc(1+2"));
    }

    #[test]
    fn decodes_underscores() {
        assert_eq!(decode_arbitrary("hello_world"), "hello world");
        assert_eq!(decode_arbitrary(r"hello\_world"), "hello_world");
        assert_eq!(decode_arbitrary("var(--my_value,_fallback)"), "var(--my_value, fallback)");
    }
}
