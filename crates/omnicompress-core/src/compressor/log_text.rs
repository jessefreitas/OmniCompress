use super::{Compressor, Outcome};

/// Sentinel that opens every run-count marker. Chosen to be vanishingly unlikely
/// to begin a real log line, so [`restore_collapsed`] can reverse the transform
/// without ambiguity. A run of N identical lines `L` collapses to `⟨×N⟩L`.
const RUN_MARKER: &str = "⟨×";

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

        // Refuse to compress if any line already starts with our marker — that
        // would make the transform irreversible. Fail-open: pass through intact.
        if lines.iter().any(|l| l.starts_with(RUN_MARKER)) {
            return Outcome::untouched(content);
        }

        // Collapse runs of consecutive identical lines into "⟨×N⟩line".
        let mut out = String::with_capacity(content.len() / 2);
        let mut i = 0;
        while i < lines.len() {
            let mut j = i + 1;
            while j < lines.len() && lines[j] == lines[i] {
                j += 1;
            }
            let run = j - i;
            if run > 1 {
                out.push_str(&format!("{RUN_MARKER}{run}⟩{}\n", lines[i]));
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

        // The collapsed form is self-contained and fully reversible (see
        // `restore_collapsed`), so this is a lossless transform: nothing is sent
        // to the CCR (`original: None`). A passthrough proxy keeps every byte of
        // information without depending on a retrieve loop.
        Outcome {
            compressed: out,
            original: None,
            detail: "log_collapse".into(),
        }
    }
}

/// Reverse of the run-length collapse: expand every `⟨×N⟩line` marker back into N
/// copies of `line`. Lines without a marker are emitted verbatim. The trailing
/// newline convention matches `compress` (one `\n` per logical line).
pub fn restore_collapsed(compressed: &str) -> String {
    let mut out = String::with_capacity(compressed.len() * 2);
    for line in compressed.lines() {
        if let Some(rest) = line.strip_prefix(RUN_MARKER) {
            if let Some((count, text)) = rest.split_once('⟩') {
                if let Ok(n) = count.parse::<usize>() {
                    for _ in 0..n {
                        out.push_str(text);
                        out.push('\n');
                    }
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
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
        // Self-contained lossless transform: nothing goes to the CCR.
        assert!(out.original.is_none(), "colapso é lossless: nada vai pro CCR");
        assert!(
            out.compressed.contains("×40"),
            "deve colapsar repetição: {}",
            out.compressed
        );
        assert!(out.compressed.contains("evento único final"));
        assert!(out.compressed.len() * 2 < log.len());
    }

    #[test]
    fn collapse_roundtrips_exactly() {
        let log = std::iter::repeat("conexão recusada na porta 5432\n")
            .take(40)
            .collect::<String>()
            + "evento único final\n";
        let out = LogTextCompressor::default().compress(&log);
        assert_eq!(restore_collapsed(&out.compressed), log, "deve reconstruir byte-a-byte");
    }

    #[test]
    fn marker_collision_passes_through() {
        // Input that already contains the run marker must not be compressed
        // (would be irreversible) — fail-open.
        let log = std::iter::repeat("⟨×9⟩já parece um marcador\n")
            .take(12)
            .collect::<String>();
        let out = LogTextCompressor::default().compress(&log);
        assert!(out.original.is_none());
        assert_eq!(out.compressed, log, "deve passar intacto");
        assert_eq!(out.detail, "untouched");
    }
}
