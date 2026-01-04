use std::collections::BTreeSet;
use std::io::Write;

pub trait Dictionary {
    fn add(&mut self, value: &[u8]);
}

#[derive(Default)]
pub struct FileDictionary {
    words: BTreeSet<Vec<u8>>,
}

impl FileDictionary {
    pub fn new() -> Self {
        Self {
            words: BTreeSet::new(),
        }
    }

    pub fn write<W: Write>(&self, writer: &mut W) {
        for word in self.words.iter() {
            writer.write_all(self.format_line(word).as_bytes()).unwrap();
        }
    }

    fn format_line(&self, value: &[u8]) -> String {
        format!("\"{}\"\n", value.escape_ascii())
    }
}

impl Dictionary for FileDictionary {
    fn add(&mut self, value: &[u8]) {
        self.words.insert(value.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_add_and_write() {
        let mut dictionary = FileDictionary::new();
        let word1 = b"hello";
        let word2 = b"world\x00\x01\x02";

        dictionary.add(word1);
        dictionary.add(word2);

        let mut output = Cursor::new(Vec::new());
        dictionary.write(&mut output);

        let result = String::from_utf8(output.into_inner()).unwrap();
        assert_eq!(result, "\"hello\"\n\"world\\x00\\x01\\x02\"\n");
    }

    #[test]
    fn test_add_duplicate() {
        let mut dictionary = FileDictionary::new();
        let word = b"duplicate";

        dictionary.add(word);
        dictionary.add(word);

        assert_eq!(dictionary.words.len(), 1);
    }
}
