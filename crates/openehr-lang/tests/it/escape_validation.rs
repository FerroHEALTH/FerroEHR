// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Escape-sequence validation in the ODIN and BEL lexers
//! (`AM/docs/ADL2/master03-file_encoding.adoc` §Special Character Sequences):
//! only `\r \n \t \\ \" \'` are legal quoted forms (strings additionally take
//! the §File Encoding `\uHHHH`/`\uHHHHHHHH` ASCII-encoded-unicode forms) —
//! "Any other character combination starting with a backslash is illegal."

/// The ODIN reader rejects an illegal escape in a character value and keeps
/// accepting the six legal forms plus plain/unicode characters.
#[test]
fn odin_character_escapes() {
    for ok in [
        r"x = <'\n'>",
        r"x = <'\t'>",
        r"x = <'\r'>",
        r"x = <'\\'>",
        r#"x = <'\"'>"#,
        r"x = <'\''>",
        "x = <'ü'>",
    ] {
        assert!(
            openehr_lang::v1_1::odin::parse(ok).is_ok(),
            "legal character must parse: {ok}"
        );
    }
    for bad in [r"x = <'\q'>", r"x = <'\d'>"] {
        assert!(
            openehr_lang::v1_1::odin::parse(bad).is_err(),
            "illegal escape must be rejected: {bad}"
        );
    }
}

/// The BEL lexer rejects illegal escapes in strings and characters, and keeps
/// accepting the legal forms including `\u`-encoded unicode in strings.
#[test]
fn bel_string_and_character_escapes() {
    let parses = |expr: &str| openehr_lang::v1_1::bel::parse_statements(expr).is_ok();
    for ok in [
        r#"/a[at0001]/b = "a\n\t\\\"z""#,
        r#"/a[at0001]/b = "grüße""#,
        "/a[at0001]/b = \"grüße 中文\"",
        r"/a[at0001]/b = '\''",
        r"/a[at0001]/b = 'x'",
    ] {
        assert!(parses(ok), "legal form must parse: {ok}");
    }
    for bad in [
        r#"/a[at0001]/b = "a\qz""#,
        r#"/a[at0001]/b = "a\u12z""#,
        r"/a[at0001]/b = '\q'",
    ] {
        assert!(!parses(bad), "illegal escape must be rejected: {bad}");
    }
}
