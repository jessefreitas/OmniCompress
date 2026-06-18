use super::{Compressor, Outcome};

#[derive(Default)]
pub struct LogTextCompressor;

impl Compressor for LogTextCompressor {
    fn name(&self) -> &'static str {
        "log_text"
    }

    fn compress(&self, content: &str) -> Outcome {
        let lines: Vec<&str> = content.lines().collect();

        // Not worth the overhead for tiny inputs.
        if lines.len() < 10 {
            return Outcome::untouched(content);
        }

        // Collapse runs of consecutive identical lines into "line ×N".
        let mut out = String::with_capacity(content.len() / 2);
        let mut i = 0;
        while i < lines.len() {
            let mut j = i + 1;
            while j < lines.len() && lines[j] == lines[i] {
                j += 1;
            }
            let run = j - i;
            if run > 1 {
                out.push_str(&format!("{} ×{}\n", lines[i], run));
            } else {
                out.push_str(lines[i]);
                out.push('\n');
            }
            i = j;
        }

        // Only replace when it genuinely shrinks the payload.
        if out.len() >= content.len() {
            return Outcome::untouched(content);
        }

        Outcome {
            compressed: out,
            original: Some(content.to_string()),
            detail: "log_collapse".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor::Compressor;

    #[test]
    fn collapses_repeated_lines() {
        let log = std::iter::repeat("conexão recusada na porta 5432\n")
            .take(40)
            .collect::<String>()
            + "evento único final\n";
        let out = LogTextCompressor::default().compress(&log);
        assert!(out.original.is_some());
        assert!(
            out.compressed.contains("×40") || out.compressed.contains("x40"),
            "deve colapsar repetição: {}",
            out.compressed
        );
        assert!(out.compressed.contains("evento único final"));
        assert!(out.compressed.len() * 2 < log.len());
    }
}
