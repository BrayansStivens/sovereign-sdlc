//! Splash Screen + Sentinel Bot Animation
//!
//! Houston-style expressive bot with the name Sentinel.

/// Block-letter startup banner
pub const SPLASH: &[&str] = &[
    "",
    " ███████╗ ██████╗ ██╗   ██╗███████╗██████╗ ███████╗██╗ ██████╗ ███╗   ██╗",
    " ██╔════╝██╔═══██╗██║   ██║██╔════╝██╔══██╗██╔════╝██║██╔════╝ ████╗  ██║",
    " ███████╗██║   ██║██║   ██║█████╗  ██████╔╝█████╗  ██║██║  ███╗██╔██╗ ██║",
    " ╚════██║██║   ██║╚██╗ ██╔╝██╔══╝  ██╔══██╗██╔══╝  ██║██║   ██║██║╚██╗██║",
    " ███████║╚██████╔╝ ╚████╔╝ ███████╗██║  ██║███████╗██║╚██████╔╝██║ ╚████║",
    " ╚══════╝ ╚═════╝   ╚═══╝  ╚══════╝╚═╝  ╚═╝╚══════╝╚═╝ ╚═════╝ ╚═╝  ╚═══╝",
    "                        S  D  L  C    v 0 . 4",
    "",
];

// ────────────────────────────────────────────────────────
// Sentinel — Houston-style expressive bot
// ────────────────────────────────────────────────────────

pub struct SentinelFrame {
    pub face: &'static str,
    pub message: &'static str,
}

const SENTINEL_IDLE: &[SentinelFrame] = &[
    SentinelFrame { face: " ● ◡ ● ", message: "Ready" },
    SentinelFrame { face: " - ᴥ - ", message: "..." },
    SentinelFrame { face: " ● ◡ ● ", message: "Listening" },
];

const SENTINEL_ROUTING: SentinelFrame = SentinelFrame {
    face: " ● ◡ ● ", message: "Routing...",
};

const SENTINEL_THINKING: &[SentinelFrame] = &[
    SentinelFrame { face: " ◠ ◡ ◠ ", message: "Thinking..." },
    SentinelFrame { face: " ✦ ◡ ✦ ", message: "Processing..." },
    SentinelFrame { face: " - ᴥ - ", message: "Analyzing..." },
    SentinelFrame { face: " ◠ ◡ ◠ ", message: "Reasoning..." },
];

const SENTINEL_GENERATING: &[SentinelFrame] = &[
    SentinelFrame { face: " ● ◡ ● ", message: "Writing..." },
    SentinelFrame { face: " ✦ ◡ ✦ ", message: "Crafting..." },
    SentinelFrame { face: " ◠ ◡ ◠ ", message: "Almost there..." },
    SentinelFrame { face: " ^ ᴥ ^ ", message: "Coming along!" },
    SentinelFrame { face: " ● ◡ ● ", message: "Keep going..." },
    SentinelFrame { face: " ✦ ◡ ✦ ", message: "Polishing..." },
];

const SENTINEL_ERROR: SentinelFrame = SentinelFrame {
    face: " ; ᴥ ; ", message: "Ups, an error...",
};

const SENTINEL_DONE: SentinelFrame = SentinelFrame {
    face: " ^ ᴥ ^ ", message: "Done!",
};

const SENTINEL_INDEXING: SentinelFrame = SentinelFrame {
    face: " ✦ ◡ ✦ ", message: "Indexing project...",
};

pub enum SentinelMood {
    Idle,
    Routing,
    Thinking,
    Generating,
    Error,
    Done,
    Indexing,
}

pub fn sentinel_frame(mood: &SentinelMood, tick: u64) -> (&'static str, &'static str) {
    match mood {
        SentinelMood::Idle => {
            let f = &SENTINEL_IDLE[(tick as usize / 10) % SENTINEL_IDLE.len()];
            (f.face, f.message)
        }
        SentinelMood::Routing => (SENTINEL_ROUTING.face, SENTINEL_ROUTING.message),
        SentinelMood::Thinking => {
            let f = &SENTINEL_THINKING[(tick as usize / 4) % SENTINEL_THINKING.len()];
            (f.face, f.message)
        }
        SentinelMood::Generating => {
            let f = &SENTINEL_GENERATING[(tick as usize / 5) % SENTINEL_GENERATING.len()];
            (f.face, f.message)
        }
        SentinelMood::Error => (SENTINEL_ERROR.face, SENTINEL_ERROR.message),
        SentinelMood::Done => (SENTINEL_DONE.face, SENTINEL_DONE.message),
        SentinelMood::Indexing => (SENTINEL_INDEXING.face, SENTINEL_INDEXING.message),
    }
}

/// Render Sentinel as 3 lines: boxed face with message (Houston style)
pub fn sentinel_lines(mood: &SentinelMood, tick: u64) -> [String; 3] {
    let (face, message) = sentinel_frame(mood, tick);
    [
        "   ╭─────╮".to_string(),
        format!("   │{}│  {}", face, message),
        "   ╰─────╯".to_string(),
    ]
}
