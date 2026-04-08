//! Splash Screen + Sentinel Bot Animation

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
// Sentinel — The watching eye of Sovereign
// ────────────────────────────────────────────────────────

/// Sentinel expression: face + message
pub struct SentinelFrame {
    pub face: &'static str,
    pub message: &'static str,
}

const SENTINEL_IDLE: &[SentinelFrame] = &[
    SentinelFrame { face: "  ( o.o )  ", message: "Ready" },
    SentinelFrame { face: "  ( -.- )  ", message: "..." },
    SentinelFrame { face: "  ( o.o )  ", message: "Listening" },
];

const SENTINEL_ROUTING: SentinelFrame = SentinelFrame {
    face: "  ( o.o )> ", message: "Routing...",
};

const SENTINEL_THINKING: &[SentinelFrame] = &[
    SentinelFrame { face: "  ( >.< )  ", message: "Thinking..." },
    SentinelFrame { face: "  ( *_* )  ", message: "Processing..." },
    SentinelFrame { face: "  ( -.- )  ", message: "Analyzing..." },
    SentinelFrame { face: "  ( o.o )  ", message: "Reasoning..." },
];

const SENTINEL_GENERATING: &[SentinelFrame] = &[
    SentinelFrame { face: "  ( ^.^ )  ", message: "Writing..." },
    SentinelFrame { face: "  ( *_* )  ", message: "Crafting..." },
    SentinelFrame { face: "  ( o.o )> ", message: "Flowing..." },
    SentinelFrame { face: "  ( ^.^ )  ", message: "Almost..." },
    SentinelFrame { face: "  ( >.> )  ", message: "Checking..." },
    SentinelFrame { face: "  ( ^_^ )  ", message: "Polishing..." },
];

const SENTINEL_ERROR: SentinelFrame = SentinelFrame {
    face: "  ( ;_; )  ", message: "Error...",
};

const SENTINEL_DONE: SentinelFrame = SentinelFrame {
    face: "  ( ^_^ )  ", message: "Done!",
};

const SENTINEL_INDEXING: SentinelFrame = SentinelFrame {
    face: "  ( *_* )  ", message: "Indexing...",
};

/// Sentinel display state
pub enum SentinelMood {
    Idle,
    Routing,
    Thinking,
    Generating,
    Error,
    Done,
    Indexing,
}

/// Get the Sentinel face and message for current mood/tick
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

/// Render Sentinel as 2 lines: face and message
pub fn sentinel_lines(mood: &SentinelMood, tick: u64) -> [String; 2] {
    let (face, message) = sentinel_frame(mood, tick);
    [
        face.to_string(),
        format!("    {message}"),
    ]
}
