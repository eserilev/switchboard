//! Parser for the tmux control-mode protocol (`tmux -C`).
//!
//! tmux sends one notification per line. The reply to each command comes
//! between `%begin` and `%end` (or `%error`). Reply lines are plain text and
//! can start with `%` too (a pane id is `%3`), so the parser keeps state.

/// One parsed message from tmux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The full reply to one command.
    Reply {
        ok: bool,
        lines: Vec<String>,
    },
    /// Bytes a pane wrote, already unescaped.
    Output {
        pane: String,
        data: Vec<u8>,
    },
    WindowAdd(String),
    WindowClose(String),
    Pause(String),
    Continue(String),
    /// The server ended the control client.
    Exit(Option<String>),
    /// A notification we do not use yet.
    Other(String),
}

#[derive(Default)]
pub struct Parser {
    block: Option<(String, Vec<String>)>,
}

impl Parser {
    /// Feeds one line, with no trailing newline. Returns a message when one is done.
    pub fn feed(&mut self, line: &[u8]) -> Option<Message> {
        let line = line.strip_suffix(b"\r").unwrap_or(line);

        if let Some((num, lines)) = &mut self.block {
            for (tag, ok) in [(&b"%end "[..], true), (&b"%error "[..], false)] {
                if let Some(rest) = line.strip_prefix(tag) {
                    if field(rest, 1) == Some(num.as_str()) {
                        let lines = std::mem::take(lines);
                        self.block = None;
                        return Some(Message::Reply { ok, lines });
                    }
                }
            }
            lines.push(String::from_utf8_lossy(line).into_owned());
            return None;
        }

        if let Some(rest) = line.strip_prefix(b"%begin ") {
            let num = field(rest, 1).unwrap_or_default().to_owned();
            self.block = Some((num, Vec::new()));
            return None;
        }
        if let Some(rest) = line.strip_prefix(b"%output ") {
            let (pane, data) = split_once(rest);
            return Some(Message::Output {
                pane: text(pane),
                data: unescape(data),
            });
        }
        if let Some(rest) = line.strip_prefix(b"%extended-output ") {
            // %extended-output %<pane> <age> ... : <data>
            let (pane, rest) = split_once(rest);
            let at = rest.windows(3).position(|w| w == b" : ")?;
            return Some(Message::Output {
                pane: text(pane),
                data: unescape(&rest[at + 3..]),
            });
        }
        let word = |p: &[u8]| line.strip_prefix(p).map(|r| text(split_once(r).0));
        if let Some(w) = word(b"%window-add ") {
            return Some(Message::WindowAdd(w));
        }
        if let Some(w) = word(b"%window-close ").or_else(|| word(b"%unlinked-window-close ")) {
            return Some(Message::WindowClose(w));
        }
        if let Some(p) = word(b"%pause ") {
            return Some(Message::Pause(p));
        }
        if let Some(p) = word(b"%continue ") {
            return Some(Message::Continue(p));
        }
        if line == b"%exit" {
            return Some(Message::Exit(None));
        }
        if let Some(reason) = line.strip_prefix(b"%exit ") {
            return Some(Message::Exit(Some(text(reason))));
        }
        Some(Message::Other(text(line)))
    }
}

/// Undoes the control-mode escapes: `\ooo` (octal) and `\\`.
pub fn unescape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'\\' {
            let oct = data
                .get(i + 1..i + 4)
                .filter(|d| d.iter().all(|b| (b'0'..=b'7').contains(b)));
            if let Some(d) = oct {
                out.push((d[0] - b'0') * 64 + (d[1] - b'0') * 8 + (d[2] - b'0'));
                i += 4;
                continue;
            }
            if data.get(i + 1) == Some(&b'\\') {
                out.push(b'\\');
                i += 2;
                continue;
            }
        }
        out.push(data[i]);
        i += 1;
    }
    out
}

fn split_once(s: &[u8]) -> (&[u8], &[u8]) {
    match s.iter().position(|&b| b == b' ') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, &[]),
    }
}

fn field(s: &[u8], n: usize) -> Option<&str> {
    std::str::from_utf8(s).ok()?.split(' ').nth(n)
}

fn text(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(lines: &[&str]) -> Vec<Message> {
        let mut p = Parser::default();
        lines.iter().filter_map(|l| p.feed(l.as_bytes())).collect()
    }

    #[test]
    fn unescapes_octal_and_backslash() {
        assert_eq!(unescape(br"a\015\012b"), b"a\r\nb");
        assert_eq!(unescape(br"\033[1m"), b"\x1b[1m");
        assert_eq!(unescape(br"x\\y"), b"x\\y");
        assert_eq!(unescape(br"\9z"), b"\\9z");
        assert_eq!(unescape(br"end\"), b"end\\");
    }

    #[test]
    fn reply_lines_can_look_like_notifications() {
        let got = feed_all(&["%begin 1 7 0", "%3", "%output %1 not-output", "%end 1 7 0"]);
        assert_eq!(
            got,
            vec![Message::Reply {
                ok: true,
                lines: vec!["%3".into(), "%output %1 not-output".into()]
            }]
        );
    }

    #[test]
    fn end_with_another_number_stays_in_the_block() {
        let got = feed_all(&["%begin 1 7 0", "%end 1 8 0", "%end 1 7 0"]);
        assert_eq!(
            got,
            vec![Message::Reply {
                ok: true,
                lines: vec!["%end 1 8 0".into()]
            }]
        );
    }

    #[test]
    fn error_reply() {
        let got = feed_all(&["%begin 1 2 0", "can't find pane: %99", "%error 1 2 0"]);
        assert_eq!(
            got,
            vec![Message::Reply {
                ok: false,
                lines: vec!["can't find pane: %99".into()]
            }]
        );
    }

    #[test]
    fn output_and_extended_output() {
        let got = feed_all(&[r"%output %4 hi\015\012", r"%extended-output %4 12 : x\134y"]);
        assert_eq!(
            got,
            vec![
                Message::Output {
                    pane: "%4".into(),
                    data: b"hi\r\n".to_vec()
                },
                Message::Output {
                    pane: "%4".into(),
                    data: b"x\\y".to_vec()
                },
            ]
        );
    }

    #[test]
    fn output_with_no_data() {
        let got = feed_all(&["%output %4 "]);
        assert_eq!(
            got,
            vec![Message::Output {
                pane: "%4".into(),
                data: vec![]
            }]
        );
    }

    #[test]
    fn window_and_flow_notifications() {
        let got = feed_all(&[
            "%window-add @2",
            "%window-close @2",
            "%unlinked-window-close @5",
            "%pause %3",
            "%continue %3",
        ]);
        assert_eq!(
            got,
            vec![
                Message::WindowAdd("@2".into()),
                Message::WindowClose("@2".into()),
                Message::WindowClose("@5".into()),
                Message::Pause("%3".into()),
                Message::Continue("%3".into()),
            ]
        );
    }

    #[test]
    fn exit_with_and_without_reason() {
        assert_eq!(feed_all(&["%exit"]), vec![Message::Exit(None)]);
        assert_eq!(
            feed_all(&["%exit server exited"]),
            vec![Message::Exit(Some("server exited".into()))]
        );
    }

    #[test]
    fn crlf_is_trimmed() {
        assert_eq!(
            feed_all(&["%window-add @1\r"]),
            vec![Message::WindowAdd("@1".into())]
        );
    }
}
