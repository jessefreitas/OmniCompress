use super::{Compressor, Outcome};

#[derive(Default)]
pub struct ProseCompressor {
    /// When `true`, prose is left untouched (extractive elision drops the middle,
    /// which is lossy and unrecoverable without a retrieve loop).
    pub lossless: bool,
}

const HEAD: usize = 3;
const TAIL: usize = 2;

fn segments(s: &str) -> Vec<String> {
    let paragraphs: Vec<&str> = s.split("\n\n").collect();
    if paragraphs.len() > 1 {
        return paragraphs.iter().map(|p| p.to_string()).collect();
    }

    // Fallback to sentences if only one paragraph
    let mut result = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        current.push(c);
        if c == '.' || c == '!' || c == '?' || c == '\n' {
            // Check if followed by space or end of string
            if i + 1 < chars.len() && chars[i + 1] == ' ' {
                current.push(' ');
                i += 1;
            }
            result.push(current.clone());
            current.clear();
        }
        i += 1;
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

impl Compressor for ProseCompressor {
    fn name(&self) -> &'static str {
        "prose"
    }

    fn compress(&self, content: &str) -> Outcome {
        if self.lossless {
            return Outcome::untouched(content);
        }
        if content.len() < 800 {
            return Outcome::untouched(content);
        }

        let segs = segments(content);
        let total = segs.len();

        if total <= HEAD + TAIL + 1 {
            return Outcome::untouched(content);
        }

        let head_segs: Vec<String> = segs.iter().take(HEAD).cloned().collect();
        let tail_segs: Vec<String> = segs.iter().skip(total - TAIL).cloned().collect();
        let elided = total - HEAD - TAIL;

        let mut compressed = head_segs.join("\n\n");
        compressed.push_str(&format!(
            "\n[omnicompress: {} segmentos elididos — original no CCR]\n",
            elided
        ));
        compressed.push_str(&tail_segs.join("\n\n"));

        if compressed.len() < content.len() {
            Outcome {
                compressed,
                original: Some(content.to_string()),
                detail: format!("prose_extractive:{}", elided),
            }
        } else {
            Outcome::untouched(content)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::compressor::Compressor;
    use super::ProseCompressor;

    #[test]
    fn test_long_prose_paragraphs() {
        let p = "Lorem ipsum dolor sit amet, consectetur adipiscing elit sed do eiusmod.";
        let mut content = String::new();
        for i in 0..12 {
            content.push_str(&format!("Paragraph {} start {} end\n\n", i, p));
        }

        let compressor = ProseCompressor::default();
        let out = compressor.compress(&content);

        assert!(out.original.is_some(), "Original should be stored in CCR");
        assert!(out.compressed.contains("elididos"), "Should contain elision marker");
        assert!(out.compressed.len() < content.len(), "Compressed should be smaller");
        assert!(out.compressed.contains("Paragraph 0 start"), "Should keep first paragraph");
        assert!(out.compressed.contains("Paragraph 11 start"), "Should keep last paragraph");
    }

    #[test]
    fn test_short_text() {
        let content = "This is a short text. It is way too short to be compressed by prose extractor.";
        let compressor = ProseCompressor::default();
        let out = compressor.compress(content);

        assert!(out.original.is_none(), "Short text should be untouched");
    }

    #[test]
    fn test_few_segments_untouched() {
        // Dois parágrafos grandes = 2 segmentos (<= HEAD+TAIL+1) → nada a elidir.
        let para = "word ".repeat(120); // ~600 chars, sem linha em branco interna
        let content = format!("{para}\n\n{para}");
        assert!(content.len() > 800);
        let out = ProseCompressor::default().compress(&content);
        assert!(out.original.is_none(), "poucos segmentos → untouched");
    }
}
