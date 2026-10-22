//! JSON with comments, as people write in request bodies: `//` line comments and `/* */` block comments. They are
//! removed before the body is sent, as Postman does, so a line can be switched off by commenting it out.

/// `text` without its comments. Comments inside strings (such as `"http://host"`) are text and stay. Each comment is
/// replaced by nothing but the newlines it contained, so line numbers in a server's error still match the editor.
pub fn strip(text: &str) -> String {
    if !text.contains("//") && !text.contains("/*") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        out.push(escaped);
                    }
                }
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                // To the end of the line, keeping the newline.
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = '\0';
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                    }
                    if last == '*' && next == '/' {
                        break;
                    }
                    last = next;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip;

    #[test]
    fn removes_comments_but_not_strings_that_look_like_them() {
        let text = "{\n  // the user\n  \"name\": \"Ada\", // inline\n  /* \"age\": 36,\n  */ \"url\": \"http://x/*y*/\",\n  \"q\": \"a\\\"//b\"\n}";
        let stripped = strip(text);
        let value: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(value["name"], "Ada");
        assert_eq!(value["url"], "http://x/*y*/");
        assert_eq!(value["q"], "a\"//b");
        assert!(value.get("age").is_none());
        // Same number of lines.
        assert_eq!(stripped.lines().count(), text.lines().count());
    }

    #[test]
    fn leaves_plain_json_alone() {
        assert_eq!(strip("{\"a\": 1}"), "{\"a\": 1}");
    }
}
