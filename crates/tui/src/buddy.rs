//! Buddy System — RPG-style pet companion for the TUI
//!
//! Persistent per-project mascot with species, rarity, stats,
//! hardware-adaptive animation, and security-reactive mood.

use ratatui::prelude::*;
use ratatui::widgets::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ────────────────────────────────────────────────────────
// Species & Animation Frames
// ────────────────────────────────────────────────────────

/// v0.4.0 species: Gato, Buho, Dragon, Fractal (backwards-compat with v0.2 species)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Species {
    // v0.2 legacy (still loadable from old buddy.json)
    Raven,
    Golem,
    Spirit,
    // v0.4 evolved
    Dragon,
    Gato,
    Buho,
    Fractal,
}

impl Species {
    /// ASCII art frames: [idle_a, idle_b, angry]
    pub fn frames(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            // v0.2 legacy
            Species::Raven  => ("(o v o)", "(- v -)", "(O V O)"),
            Species::Golem  => ("[O_O]",   "[o_o]",   "[X_X]"),
            Species::Spirit => ("-{_}-",   "~{_}~",   "!{_}!"),
            // v0.4 species
            Species::Gato    => ("=^.^=",   "=^-^=",   "=>.<="),
            Species::Buho    => ("(O,O)",   "(-,O)",   "(X,X)"),
            Species::Dragon  => (r"(@\___", r"(O\___", r"(X\***"),
            Species::Fractal => ("{*_*}",   "{~_~}",   "{!_!}"),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Species::Raven   => "Raven",
            Species::Golem   => "Golem",
            Species::Spirit  => "Spirit",
            Species::Gato    => "Gato",
            Species::Buho    => "Buho",
            Species::Dragon  => "Dragon",
            Species::Fractal => "Fractal",
        }
    }

    /// v0.4 roll: prioritizes new species but legacy can still appear
    fn from_roll(roll: u32) -> Self {
        match roll {
            0..=200   => Species::Gato,
            201..=400 => Species::Buho,
            401..=600 => Species::Dragon,
            601..=800 => Species::Fractal,
            801..=900 => Species::Raven,
            901..=950 => Species::Spirit,
            _         => Species::Golem,
        }
    }
}

// ────────────────────────────────────────────────────────
// Rarity (Gacha)
// ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
    Sovereign,
}

impl Rarity {
    fn from_roll(roll: u32) -> Self {
        match roll {
            991..=1000 => Rarity::Sovereign,
            961..=990  => Rarity::Epic,
            901..=960  => Rarity::Rare,
            751..=900  => Rarity::Uncommon,
            _          => Rarity::Common,
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Rarity::Common    => Color::White,
            Rarity::Uncommon  => Color::Green,
            Rarity::Rare      => Color::Blue,
            Rarity::Epic      => Color::Magenta,
            Rarity::Sovereign => Color::Yellow,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Rarity::Common    => "Common",
            Rarity::Uncommon  => "Uncommon",
            Rarity::Rare      => "Rare",
            Rarity::Epic      => "Epic",
            Rarity::Sovereign => "SOVEREIGN",
        }
    }
}

// ────────────────────────────────────────────────────────
// Mood (reactive to system state)
// ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Happy,
    Idle,
    Working,
    Stressed,      // Hardware >80%
    Angry,         // Critical security finding
    Exhausted,     // Hardware >90%
    Confused,      // Council disagreement
    Remembering,   // Loading old session
}

impl Mood {
    pub fn color(&self) -> Color {
        match self {
            Mood::Happy       => Color::Green,
            Mood::Idle        => Color::White,
            Mood::Working     => Color::Cyan,
            Mood::Stressed    => Color::Yellow,
            Mood::Angry       => Color::Red,
            Mood::Exhausted   => Color::DarkGray,
            Mood::Confused    => Color::Magenta,
            Mood::Remembering => Color::Blue,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Mood::Happy       => "Happy",
            Mood::Idle        => "Idle",
            Mood::Working     => "Working",
            Mood::Stressed    => "Stressed",
            Mood::Angry       => "ANGRY!",
            Mood::Exhausted   => "Exhausted",
            Mood::Confused    => "Confused?!",
            Mood::Remembering => "Remembering...",
        }
    }
}

// ────────────────────────────────────────────────────────
// Name Generation
// ────────────────────────────────────────────────────────

const PREFIXES: &[&str] = &[
    "Byte", "Kernel", "Shadow", "Cipher", "Flux", "Nano",
    "Hexa", "Pixel", "Rust", "Volt", "Nova", "Onyx",
    "Rune", "Synth", "Glitch", "Arc", "Zero", "Nex",
];

const SUFFIXES: &[&str] = &[
    "", "is", "on", "ix", "us", "ra", "os", "ax", "el", "or",
];

fn generate_name(seed: u64) -> String {
    let prefix = PREFIXES[(seed as usize) % PREFIXES.len()];
    let suffix = SUFFIXES[((seed / 7) as usize) % SUFFIXES.len()];
    format!("{prefix}{suffix}")
}

// ────────────────────────────────────────────────────────
// Buddy State (persisted to .sovereign/buddy.json)
// ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyData {
    pub name: String,
    pub species: Species,
    pub rarity: Rarity,
    pub level: u32,
    pub xp: u64,
    pub lines_audited: u64,
    pub vulns_caught: u64,
    pub created_at: String,
    /// v0.4: Code Quality Radar — clippy warning count from last scan
    #[serde(default)]
    pub clippy_warnings: u32,
    /// v0.4: Technical debt score (0=clean, 100=heavy debt)
    #[serde(default)]
    pub tech_debt_score: u32,
    /// v0.4: Total auto-fixes applied
    #[serde(default)]
    pub auto_fixes: u64,
}

impl BuddyData {
    pub fn xp_for_next_level(&self) -> u64 {
        (self.level as u64 + 1) * 100
    }

    pub fn add_xp(&mut self, amount: u64) {
        self.xp += amount;
        while self.xp >= self.xp_for_next_level() {
            self.xp -= self.xp_for_next_level();
            self.level += 1;
        }
    }
}

/// Runtime buddy state (non-persisted animation/mood)
pub struct Buddy {
    pub data: BuddyData,
    pub mood: Mood,
    frame_tick: u64,
    file_path: PathBuf,
}

impl Buddy {
    /// Load from project or generate new buddy
    pub fn load_or_create(project_root: &Path) -> Self {
        let file_path = project_root.join(".sovereign").join("buddy.json");

        let data = if file_path.exists() {
            match std::fs::read_to_string(&file_path) {
                Ok(json) => serde_json::from_str(&json).unwrap_or_else(|_| Self::roll_new()),
                Err(_) => Self::roll_new(),
            }
        } else {
            Self::roll_new()
        };

        let buddy = Self {
            data,
            mood: Mood::Idle,
            frame_tick: 0,
            file_path,
        };
        buddy.save();
        buddy
    }

    /// Roll a new buddy (Gacha!)
    fn roll_new() -> BuddyData {
        // Simple seed from system time
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42);

        let rarity_roll = ((seed % 1000) + 1) as u32;
        let species_roll = (((seed / 1000) % 1000) + 1) as u32;

        let species = Species::from_roll(species_roll);
        let rarity = Rarity::from_roll(rarity_roll);
        let name = generate_name(seed);

        let created_at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

        BuddyData {
            name,
            species,
            rarity,
            level: 1,
            xp: 0,
            lines_audited: 0,
            vulns_caught: 0,
            created_at,
            clippy_warnings: 0,
            tech_debt_score: 0,
            auto_fixes: 0,
        }
    }

    /// Persist to .sovereign/buddy.json
    pub fn save(&self) {
        if let Some(parent) = self.file_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.data) {
            let _ = std::fs::write(&self.file_path, json);
        }
    }

    /// Advance animation frame
    pub fn tick(&mut self) {
        self.frame_tick = self.frame_tick.wrapping_add(1);
    }

    /// Update mood based on hardware + security state
    pub fn update_mood(&mut self, cpu_pct: u16, ram_pct: u16, critical_findings: usize) {
        self.mood = if critical_findings > 0 {
            Mood::Angry
        } else if ram_pct > 90 || cpu_pct > 90 {
            Mood::Exhausted
        } else if ram_pct > 80 || cpu_pct > 80 {
            Mood::Stressed
        } else if cpu_pct > 40 {
            Mood::Working
        } else if self.data.level > 5 {
            Mood::Happy
        } else {
            Mood::Idle
        };
    }

    /// Set mood directly (for Council/session events)
    pub fn set_mood(&mut self, mood: Mood) {
        self.mood = mood;
    }

    /// v0.4: Code Quality Radar — update from clippy scan results
    pub fn update_code_quality(&mut self, clippy_warnings: u32, total_findings: usize) {
        self.data.clippy_warnings = clippy_warnings;
        // Tech debt score: 0-100 based on warnings + findings
        self.data.tech_debt_score = ((clippy_warnings as f32 * 2.0 + total_findings as f32 * 5.0)
            .min(100.0)) as u32;
        // XP bonus for clean scans
        if clippy_warnings == 0 && total_findings == 0 {
            self.data.add_xp(50); // Clean project bonus!
        }
    }

    /// v0.4: Record an auto-fix application
    pub fn on_auto_fix(&mut self) {
        self.data.auto_fixes += 1;
        self.data.add_xp(30);
    }

    /// v0.4: Code quality label for TUI display
    pub fn quality_label(&self) -> (&'static str, ratatui::prelude::Color) {
        match self.data.tech_debt_score {
            0..=10  => ("Pristine",  ratatui::prelude::Color::Green),
            11..=30 => ("Clean",     ratatui::prelude::Color::Cyan),
            31..=60 => ("Tech Debt", ratatui::prelude::Color::Yellow),
            _       => ("Critical",  ratatui::prelude::Color::Red),
        }
    }

    /// Greeting after absence
    pub fn return_greeting(&self, days_away: i64) -> String {
        if days_away <= 0 {
            format!("{}: Welcome back!", self.data.name)
        } else if days_away == 1 {
            format!("{}: I guarded your secrets for a day. Shall we continue the audit?", self.data.name)
        } else {
            format!(
                "{}: I've guarded your secrets for {} days. Ready to resume the audit?",
                self.data.name, days_away
            )
        }
    }

    /// Grant XP for actions
    pub fn on_code_audited(&mut self, lines: u64) {
        self.data.lines_audited += lines;
        self.data.add_xp(lines / 10);
    }

    pub fn on_vuln_caught(&mut self) {
        self.data.vulns_caught += 1;
        self.data.add_xp(25);
    }

    /// Get the current animation frame
    fn current_frame(&self) -> &'static str {
        let (a, b, angry) = self.data.species.frames();
        if self.mood == Mood::Angry {
            // Vibrate: alternate between angry frame and offset
            if self.frame_tick % 4 < 2 { angry } else { a }
        } else if self.mood == Mood::Exhausted {
            // Golem special: show exhausted frame
            if self.data.species == Species::Golem {
                angry // [X_X] for exhausted golem
            } else {
                b
            }
        } else {
            // Normal idle animation: alternate A/B
            if self.frame_tick % 6 < 3 { a } else { b }
        }
    }

    /// Render the buddy panel into a ratatui Frame
    pub fn render(&self, frame: &mut Frame, area: Rect, ram_free_pct: u16) {
        let rarity_color = self.data.rarity.color();
        let mood_color = self.mood.color();
        let sprite = self.current_frame();

        // Vibration offset for ANGRY mood
        let name_display = if self.mood == Mood::Angry && self.frame_tick % 4 < 2 {
            format!(" {} ", self.data.name)
        } else {
            self.data.name.clone()
        };

        // XP progress bar
        let xp_needed = self.data.xp_for_next_level();
        let xp_pct = if xp_needed > 0 {
            (self.data.xp as f32 / xp_needed as f32 * 10.0) as u16
        } else {
            0
        };
        let xp_bar = format!(
            "[{}{}]",
            "#".repeat(xp_pct as usize),
            ".".repeat(10_usize.saturating_sub(xp_pct as usize)),
        );

        // HP = uptime proxy (100 - cpu_pct, capped)
        // MP = RAM free %
        let hp = 100u16.saturating_sub(ram_free_pct.min(100));
        let mp = ram_free_pct.min(100);

        let hp_bar = stat_bar(hp, 8);
        let mp_bar = stat_bar(mp, 8);

        let lines = vec![
            // Sprite centered with mood color
            Line::from(Span::styled(
                format!("    {sprite}"),
                Style::default().fg(mood_color).bold(),
            )),
            Line::from(""),
            // Name + Rarity
            Line::from(vec![
                Span::styled(
                    format!("  {name_display}"),
                    Style::default().fg(rarity_color).bold(),
                ),
                Span::styled(
                    format!(" [{}]", self.data.rarity.label()),
                    Style::default().fg(rarity_color),
                ),
            ]),
            // Species + Mood
            Line::from(vec![
                Span::styled(
                    format!("  {} ", self.data.species.display_name()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("({})", self.mood.label()),
                    Style::default().fg(mood_color),
                ),
            ]),
            Line::from(""),
            // Level + XP
            Line::from(vec![
                Span::styled(
                    format!("  LVL {:>2} ", self.data.level),
                    Style::default().fg(Color::White).bold(),
                ),
                Span::styled(xp_bar, Style::default().fg(Color::Cyan)),
            ]),
            // Stats
            Line::from(vec![
                Span::styled("  HP  ", Style::default().fg(Color::Red)),
                Span::styled(hp_bar, Style::default().fg(Color::Red)),
            ]),
            Line::from(vec![
                Span::styled("  MP  ", Style::default().fg(Color::Blue)),
                Span::styled(mp_bar, Style::default().fg(Color::Blue)),
            ]),
            Line::from(""),
            // v0.4: Code Quality Radar
            {
                let (ql, qc) = self.quality_label();
                Line::from(vec![
                    Span::styled("  QA  ", Style::default().fg(qc).bold()),
                    Span::styled(ql, Style::default().fg(qc)),
                    Span::styled(
                        format!(" ({}w/{}d)", self.data.clippy_warnings, self.data.tech_debt_score),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            },
            // Lifetime stats
            Line::from(Span::styled(
                format!("  {} audited | {} fixes", self.data.lines_audited, self.data.auto_fixes),
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                format!("  {} vulns caught", self.data.vulns_caught),
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let border_color = if self.mood == Mood::Angry {
            Color::Red
        } else {
            rarity_color
        };

        let title = format!(
            " {} — {} ",
            self.data.species.display_name(),
            self.data.rarity.label()
        );

        let widget = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(title)
                .title_style(Style::default().fg(rarity_color).bold()),
        );

        frame.render_widget(widget, area);
    }
}

fn stat_bar(pct: u16, width: u16) -> String {
    let filled = (pct as f32 / 100.0 * width as f32) as u16;
    let empty = width.saturating_sub(filled);
    format!(
        "[{}{}] {:>3}%",
        "|".repeat(filled as usize),
        ".".repeat(empty as usize),
        pct
    )
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_species_from_roll_v04() {
        // v0.4 new species
        assert_eq!(Species::from_roll(100), Species::Gato);
        assert_eq!(Species::from_roll(300), Species::Buho);
        assert_eq!(Species::from_roll(500), Species::Dragon);
        assert_eq!(Species::from_roll(700), Species::Fractal);
        // Legacy species still accessible
        assert_eq!(Species::from_roll(850), Species::Raven);
        assert_eq!(Species::from_roll(930), Species::Spirit);
        assert_eq!(Species::from_roll(980), Species::Golem);
    }

    #[test]
    fn test_rarity_from_roll() {
        assert_eq!(Rarity::from_roll(1), Rarity::Common);
        assert_eq!(Rarity::from_roll(750), Rarity::Common);
        assert_eq!(Rarity::from_roll(800), Rarity::Uncommon);
        assert_eq!(Rarity::from_roll(920), Rarity::Rare);
        assert_eq!(Rarity::from_roll(970), Rarity::Epic);
        assert_eq!(Rarity::from_roll(995), Rarity::Sovereign);
    }

    #[test]
    fn test_name_generation() {
        let n1 = generate_name(42);
        let n2 = generate_name(9999);
        assert!(!n1.is_empty());
        assert!(!n2.is_empty());
        assert_ne!(n1, n2);
    }

    #[test]
    fn test_buddy_xp_leveling() {
        let mut data = BuddyData {
            name: "Test".into(),
            species: Species::Raven,
            rarity: Rarity::Common,
            level: 1,
            xp: 0,
            lines_audited: 0,
            vulns_caught: 0,
            created_at: "2026-01-01".into(),
            clippy_warnings: 0,
            tech_debt_score: 0,
            auto_fixes: 0,
        };

        // Level 1 needs 200 XP to level up (level+1)*100
        data.add_xp(199);
        assert_eq!(data.level, 1);

        data.add_xp(1);
        assert_eq!(data.level, 2);
        assert_eq!(data.xp, 0);
    }

    #[test]
    fn test_mood_priority() {
        let mut buddy = Buddy {
            data: BuddyData {
                name: "Test".into(),
                species: Species::Golem,
                rarity: Rarity::Common,
                level: 1,
                xp: 0,
                lines_audited: 0,
                vulns_caught: 0,
                created_at: "2026-01-01".into(),
                clippy_warnings: 0,
                tech_debt_score: 0,
                auto_fixes: 0,
            },
            mood: Mood::Idle,
            frame_tick: 0,
            file_path: PathBuf::from("/tmp/test-buddy.json"),
        };

        // Critical findings override everything
        buddy.update_mood(10, 10, 1);
        assert_eq!(buddy.mood, Mood::Angry);

        // High load without criticals
        buddy.update_mood(95, 95, 0);
        assert_eq!(buddy.mood, Mood::Exhausted);

        // Medium load
        buddy.update_mood(85, 50, 0);
        assert_eq!(buddy.mood, Mood::Stressed);

        // Working
        buddy.update_mood(50, 50, 0);
        assert_eq!(buddy.mood, Mood::Working);

        // Idle
        buddy.update_mood(10, 10, 0);
        assert_eq!(buddy.mood, Mood::Idle);
    }

    #[test]
    fn test_animation_frames() {
        let (a, b, angry) = Species::Raven.frames();
        assert_eq!(a, "(o v o)");
        assert_eq!(b, "(- v -)");
        assert_eq!(angry, "(O V O)");
    }

    #[test]
    fn test_persistence() {
        let tmp = std::env::temp_dir().join("sovereign-buddy-test");
        let _ = std::fs::create_dir_all(&tmp);

        let buddy = Buddy::load_or_create(&tmp);
        let name = buddy.data.name.clone();
        let species = buddy.data.species;
        buddy.save();

        // Reload — should get same buddy
        let buddy2 = Buddy::load_or_create(&tmp);
        assert_eq!(buddy2.data.name, name);
        assert_eq!(buddy2.data.species, species);

        let _ = std::fs::remove_dir_all(tmp.join(".sovereign"));
    }

    #[test]
    fn test_stat_bar() {
        let bar = stat_bar(50, 10);
        assert!(bar.contains("|||||"));
        assert!(bar.contains("....."));
        assert!(bar.contains("50%"));
    }

    #[test]
    fn test_dragon_fire_frame() {
        let (_, _, angry) = Species::Dragon.frames();
        assert!(angry.contains("***")); // fire breath
    }

    #[test]
    fn test_v04_species_frames() {
        let (a, b, _) = Species::Gato.frames();
        assert!(a.contains("^.^"));
        assert!(b.contains("^-^"));

        let (a, _, _) = Species::Buho.frames();
        assert!(a.contains("O,O"));

        let (a, _, _) = Species::Fractal.frames();
        assert!(a.contains("*_*"));
    }

    #[test]
    fn test_code_quality_radar() {
        let mut buddy = Buddy::load_or_create(&std::env::temp_dir().join("cqr-test"));
        buddy.update_code_quality(0, 0);
        assert_eq!(buddy.data.tech_debt_score, 0);
        let (label, _) = buddy.quality_label();
        assert_eq!(label, "Pristine");

        buddy.update_code_quality(10, 5);
        assert!(buddy.data.tech_debt_score > 0);

        buddy.update_code_quality(50, 20);
        assert!(buddy.data.tech_debt_score >= 60);
    }

    #[test]
    fn test_auto_fix_xp() {
        let mut buddy = Buddy::load_or_create(&std::env::temp_dir().join("fix-test"));
        let initial_xp = buddy.data.xp;
        buddy.on_auto_fix();
        assert_eq!(buddy.data.auto_fixes, 1);
        assert!(buddy.data.xp > initial_xp);
    }
}
