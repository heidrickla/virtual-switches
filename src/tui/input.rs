//! One line of text being typed in the switch list: a filter, a name, a path. The cursor moves in it
//! and the text scrolls sideways to keep the cursor in view.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Span;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    pub text: String,
    /// in characters
    pub cursor: usize,
}

impl Input {
    /// The text with the cursor after it.
    pub fn new(text: &str) -> Self {
        Input {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn byte(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(i, _)| i)
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// Pasted text at the cursor, its line breaks and other control characters left out.
    pub fn paste(&mut self, text: &str) {
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        let at = self.byte(self.cursor);
        self.text.insert_str(at, &clean);
        self.cursor += clean.chars().count();
    }

    /// Edits or moves in the text; false when the key is not one of the field's. AltGr arrives as
    /// Ctrl and Alt together, and the characters it types are text.
    pub fn key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let altgr = ctrl && k.modifiers.contains(KeyModifiers::ALT);
        let at = self.byte(self.cursor);
        match k.code {
            KeyCode::Char(c) if !ctrl || altgr => {
                self.text.insert(at, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let from = self.byte(self.cursor - 1);
                self.text.replace_range(from..at, "");
                self.cursor -= 1;
            }
            KeyCode::Delete if self.cursor < self.len() => {
                let to = self.byte(self.cursor + 1);
                self.text.replace_range(at..to, "");
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home | KeyCode::Char('a') if k.code == KeyCode::Home || ctrl => {
                self.cursor = 0
            }
            KeyCode::End | KeyCode::Char('e') if k.code == KeyCode::End || ctrl => {
                self.cursor = self.len()
            }
            KeyCode::Char('u') if ctrl => {
                self.text.replace_range(..at, "");
                self.cursor = 0;
            }
            KeyCode::Char('k') if ctrl => self.text.truncate(at),
            KeyCode::Char('w') if ctrl => {
                // the word before the cursor, and the separators after it
                let before = &self.text[..at];
                let word_end = before
                    .trim_end_matches(|c: char| !c.is_alphanumeric())
                    .len();
                let kept = before[..word_end]
                    .rfind(|c: char| !c.is_alphanumeric())
                    .map_or(0, |i| {
                        i + before[i..].chars().next().map_or(1, char::len_utf8)
                    });
                let removed = before[kept..].chars().count();
                self.text.replace_range(kept..at, "");
                self.cursor -= removed;
            }
            KeyCode::Backspace | KeyCode::Delete => {}
            _ => return false,
        }
        true
    }

    /// The part of the text that fits `width` columns with the cursor in view, and the cursor's
    /// column in it.
    pub fn window(&self, width: usize) -> (String, usize) {
        let width = width.max(1);
        let chars: Vec<char> = self.text.chars().collect();
        let cols = |cs: &[char]| Span::raw(cs.iter().collect::<String>()).width();
        let mut start = 0;
        // the cursor's own cell counts, so the text before it gets one column less
        while start < self.cursor && cols(&chars[start..self.cursor]) >= width {
            start += 1;
        }
        let mut end = self.cursor;
        while end < chars.len() && cols(&chars[start..=end]) <= width {
            end += 1;
        }
        let shown: String = chars[start..end.min(chars.len())].iter().collect();
        (shown, cols(&chars[start..self.cursor]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(i: &mut Input, code: KeyCode) {
        i.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn the_cursor_moves_and_text_goes_in_where_it_is() {
        let mut i = Input::new("core.txt");
        for _ in 0..4 {
            press(&mut i, KeyCode::Left);
        }
        press(&mut i, KeyCode::Char('1'));
        assert_eq!((i.text.as_str(), i.cursor), ("core1.txt", 5));
        press(&mut i, KeyCode::Delete);
        press(&mut i, KeyCode::Home);
        press(&mut i, KeyCode::Backspace);
        assert_eq!(i.text, "core1txt");
        press(&mut i, KeyCode::End);
        i.paste("\r\n.bak");
        assert_eq!((i.text.as_str(), i.cursor), ("core1txt.bak", 12));
        i.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(i.text, "core1txt.");
        i.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(i.is_empty());
        // AltGr types the character, as a backslash on many European layouts
        i.key(KeyEvent::new(
            KeyCode::Char('\\'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ));
        assert_eq!(i.text, "\\");
        assert!(!i.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
    }

    #[test]
    fn a_long_text_scrolls_to_keep_the_cursor_in_view() {
        let i = Input::new("C:\\Users\\lewis\\captures\\core1.txt");
        let (shown, col) = i.window(10);
        assert_eq!((shown.as_str(), col), ("core1.txt", 9));
        let mut i = i;
        press(&mut i, KeyCode::Home);
        let (shown, col) = i.window(10);
        assert_eq!((shown.as_str(), col), ("C:\\Users\\l", 0));
        assert_eq!(Input::new("ab").window(10), ("ab".to_string(), 2));
    }
}
