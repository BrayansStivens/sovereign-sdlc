//! Async Loading States + Telemetry
//!
//! Spinner, token throughput, and post-generation summary.
//! Pure ASCII — no emojis.

/// Braille spinner frames
const SPINNER: &[char] = &['-', '\\', '|', '/'];

/// Dots animation
const DOTS: &[&str] = &["", ".", "..", "..."];

/// Generation telemetry (populated from Ollama response metadata)
#[derive(Debug, Clone, Default)]
pub struct GenTelemetry {
    pub total_tokens: u64,
    pub elapsed_ms: u64,
    pub prompt_tokens: u64,
    pub eval_tokens: u64,
}

impl GenTelemetry {
    pub fn tokens_per_sec(&self) -> f64 {
        if self.elapsed_ms == 0 { return 0.0; }
        self.eval_tokens as f64 / (self.elapsed_ms as f64 / 1000.0)
    }

    /// Post-generation summary line (ASCII, Claude-style)
    /// Format: [+] Worked for 3.2s . 847 tokens consumed
    pub fn summary_line(&self) -> String {
        let secs = self.elapsed_ms as f64 / 1000.0;
        let tok_s = self.tokens_per_sec();
        format!(
            "[+] Worked for {secs:.1}s . {total} tokens . >_ {tok_s:.1} tok/s",
            total = self.total_tokens,
        )
    }
}

/// Current loading state
#[derive(Debug, Clone, PartialEq)]
pub enum LoadingState {
    Idle,
    Routing,
    Thinking,
    Generating { elapsed_secs: u64 },
    Indexing { files_done: usize, files_total: usize },
    Scanning,
}

/// Manages loading animation state
pub struct LoadingAnimation {
    pub state: LoadingState,
    tick: u64,
    /// Latest telemetry from generation
    pub telemetry: Option<GenTelemetry>,
}

impl LoadingAnimation {
    pub fn new() -> Self {
        Self {
            state: LoadingState::Idle,
            tick: 0,
            telemetry: None,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn set(&mut self, state: LoadingState) {
        if self.state != state {
            self.state = state;
            self.tick = 0;
        }
    }

    pub fn spinner_char(&self) -> char {
        SPINNER[(self.tick as usize) % SPINNER.len()]
    }

    fn dots(&self) -> &str {
        DOTS[(self.tick as usize / 3) % DOTS.len()]
    }

    /// Cursor blink for "generating" state
    fn cursor(&self) -> &str {
        if self.tick % 4 < 2 { ">_" } else { "  " }
    }

    /// Format the status line — pure ASCII telemetry
    pub fn status_text(&self) -> String {
        match &self.state {
            LoadingState::Idle => String::new(),
            LoadingState::Routing => {
                format!("{} Routing...", self.spinner_char())
            }
            LoadingState::Thinking => {
                format!("{} Thinking{}  (Esc to cancel)", self.spinner_char(), self.dots())
            }
            LoadingState::Generating { elapsed_secs } => {
                format!(
                    "* Generating{} [ {} -- tok/s | ~ {}s ]  (Esc to cancel)",
                    self.dots(), self.cursor(), elapsed_secs,
                )
            }
            LoadingState::Indexing { files_done, files_total } => {
                format!(
                    "{} Indexing {}/{}{}", self.spinner_char(),
                    files_done, files_total, self.dots(),
                )
            }
            LoadingState::Scanning => {
                format!("{} Scanning{}", self.spinner_char(), self.dots())
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.state != LoadingState::Idle
    }

    /// Record telemetry after generation completes
    pub fn finish_generation(&mut self, elapsed_ms: u64, response_len: usize) {
        // Rough token estimate: 1 token ~= 4 chars
        let eval_tokens = (response_len / 4) as u64;
        self.telemetry = Some(GenTelemetry {
            total_tokens: eval_tokens,
            elapsed_ms,
            prompt_tokens: 0,
            eval_tokens,
        });
    }

    /// Get the last telemetry summary (if any)
    pub fn last_summary(&self) -> Option<String> {
        self.telemetry.as_ref().map(|t| t.summary_line())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_cycles() {
        let mut anim = LoadingAnimation::new();
        anim.set(LoadingState::Thinking);
        let c1 = anim.spinner_char();
        anim.tick();
        let c2 = anim.spinner_char();
        assert_ne!(c1, c2);
    }

    #[test]
    fn test_idle_no_text() {
        let anim = LoadingAnimation::new();
        assert!(anim.status_text().is_empty());
        assert!(!anim.is_active());
    }

    #[test]
    fn test_generating_shows_time() {
        let mut anim = LoadingAnimation::new();
        anim.set(LoadingState::Generating { elapsed_secs: 5 });
        let text = anim.status_text();
        assert!(text.contains("5s"));
        assert!(text.contains("Generating"));
    }

    #[test]
    fn test_telemetry_tok_per_sec() {
        let t = GenTelemetry {
            total_tokens: 100,
            elapsed_ms: 2000,
            prompt_tokens: 20,
            eval_tokens: 80,
        };
        assert!((t.tokens_per_sec() - 40.0).abs() < 0.1);
    }

    #[test]
    fn test_summary_line() {
        let t = GenTelemetry {
            total_tokens: 500,
            elapsed_ms: 3200,
            prompt_tokens: 100,
            eval_tokens: 400,
        };
        let summary = t.summary_line();
        assert!(summary.contains("[+]"));
        assert!(summary.contains("3.2s"));
        assert!(summary.contains("500 tokens"));
        assert!(summary.contains("tok/s"));
    }

    #[test]
    fn test_finish_generation() {
        let mut anim = LoadingAnimation::new();
        anim.finish_generation(5000, 2000); // 2000 chars ~ 500 tokens
        assert!(anim.telemetry.is_some());
        let t = anim.telemetry.unwrap();
        assert_eq!(t.eval_tokens, 500);
        assert_eq!(t.elapsed_ms, 5000);
    }

    #[test]
    fn test_indexing_progress() {
        let mut anim = LoadingAnimation::new();
        anim.set(LoadingState::Indexing { files_done: 10, files_total: 50 });
        assert!(anim.status_text().contains("10/50"));
    }
}
