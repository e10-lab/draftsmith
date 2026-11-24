//! 도면 문자 정리: MTEXT 서식 코드와 `%%` 특수 문자를 걷어 내 보이는 글자만 남긴다.
//!
//! 글꼴·크기 바꿈 같은 서식은 아직 버린다 (줄바꿈만 살림). 화면 표시는 M0 ⑥ 에서 다듬는다.

/// TEXT 의 `%%` 코드: `%%c` 지름 ⌀, `%%d` 도 °, `%%p` ±, `%%%` %, `%%u`·`%%o` 밑줄·윗줄(버림)
pub fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '%' && it.peek() == Some(&'%') {
            it.next();
            match it.next() {
                Some('c' | 'C') => out.push('⌀'),
                Some('d' | 'D') => out.push('°'),
                Some('p' | 'P') => out.push('±'),
                Some('%') => out.push('%'),
                Some('u' | 'U' | 'o' | 'O' | 'k' | 'K') => {}
                Some(d) if d.is_ascii_digit() => {
                    // %%nnn: 문자 번호
                    let mut num = String::from(d);
                    while num.len() < 3 && it.peek().is_some_and(|c| c.is_ascii_digit()) {
                        num.push(it.next().unwrap());
                    }
                    if let Some(ch) = num.parse::<u32>().ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                Some(other) => {
                    out.push_str("%%");
                    out.push(other);
                }
                None => out.push_str("%%"),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// MTEXT 서식 코드를 걷어 낸다. 줄바꿈(`\P`)은 `\n` 으로.
pub fn plain_mtext(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' | '}' => {}
            '\\' => match it.next() {
                Some('P') => out.push('\n'),
                Some('~') => out.push('\u{a0}'),
                Some('\\') => out.push('\\'),
                Some('{') => out.push('{'),
                Some('}') => out.push('}'),
                // 켜고 끄는 서식 (밑줄·윗줄·취소선·이어쓰기)
                Some('L' | 'l' | 'O' | 'o' | 'K' | 'k' | 'N') => {}
                // 쌓기 \S위^아래; → 위/아래
                Some('S') => {
                    for c in it.by_ref() {
                        if c == ';' {
                            break;
                        }
                        out.push(if c == '^' || c == '#' { '/' } else { c });
                    }
                }
                // 유니코드 \U+AC00
                Some('U') if it.peek() == Some(&'+') => {
                    it.next();
                    let hex: String = (0..4).filter_map(|_| it.next()).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                // 값을 받는 서식: \f글꼴|…; \H2.5x; \C1; \W0.8; \Q15; \T1.2; \A1; \p…;
                Some(
                    'f' | 'F' | 'H' | 'h' | 'C' | 'c' | 'W' | 'w' | 'Q' | 'q' | 'T' | 't' | 'A'
                    | 'a' | 'p',
                ) => {
                    for c in it.by_ref() {
                        if c == ';' {
                            break;
                        }
                    }
                }
                Some(other) => out.push(other),
                None => {}
            },
            _ => out.push(c),
        }
    }
    plain_text(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtext_codes() {
        assert_eq!(
            plain_mtext(r"{\fMalgun Gothic|b0|i0|c129;거실}\P{\H0.7x;12.3㎡}"),
            "거실\n12.3㎡"
        );
        assert_eq!(plain_mtext(r"\A1;2\S1^2;층"), "21/2층");
        assert_eq!(plain_mtext(r"\U+AC70\U+C2E4"), "거실");
        assert_eq!(plain_text("%%c100 %%p5 90%%d"), "⌀100 ±5 90°");
    }
}
