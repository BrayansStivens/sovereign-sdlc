//! Splash Screen + Houston Bot Animation
//!
//! Block-letter ASCII banner + expressive robot companion.

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
// Houston — Expressive bot for generation feedback
// ────────────────────────────────────────────────────────

/// Houston frame: face line + message
pub struct HoustonFrame {
    pub face: &'static str,
    pub message: &'static str,
}

/// All Houston expressions
const HOUSTON_ROUTING: HoustonFrame = HoustonFrame {
    face: " ● ◡ ● ",
    message: "Routing...",
};

const HOUSTON_THINKING: &[HoustonFrame] = &[
    HoustonFrame { face: " ◠ ◡ ◠ ", message: "Thinking..." },
    HoustonFrame { face: " ✦ ◡ ✦ ", message: "Processing..." },
    HoustonFrame { face: " - ᴥ - ", message: "Working..." },
    HoustonFrame { face: " ◠ ◡ ◠ ", message: "Analyzing..." },
];

const HOUSTON_GENERATING: &[HoustonFrame] = &[
    HoustonFrame { face: " ● ◡ ● ", message: "Generating..." },
    HoustonFrame { face: " ✦ ◡ ✦ ", message: "Writing code..." },
    HoustonFrame { face: " ◠ ◡ ◠ ", message: "Almost there..." },
    HoustonFrame { face: " ^ ᴥ ^ ", message: "Coming along!" },
    HoustonFrame { face: " ● ◡ ● ", message: "Keep going..." },
    HoustonFrame { face: " ✦ ◡ ✦ ", message: "Crafting..." },
];

const HOUSTON_ERROR: HoustonFrame = HoustonFrame {
    face: " ; ᴥ ; ",
    message: "Ups, an error...",
};

const HOUSTON_DONE: HoustonFrame = HoustonFrame {
    face: " ^ ᴥ ^ ",
    message: "Done!",
};

const HOUSTON_IDLE: &[HoustonFrame] = &[
    HoustonFrame { face: " ● ◡ ● ", message: "Ready" },
    HoustonFrame { face: " - ᴥ - ", message: "..." },
    HoustonFrame { face: " ● ◡ ● ", message: "Waiting" },
];

const HOUSTON_INDEXING: HoustonFrame = HoustonFrame {
    face: " ✦ ◡ ✦ ",
    message: "Indexing project...",
};

/// Houston display state
pub enum HoustonMood {
    Idle,
    Routing,
    Thinking,
    Generating,
    Error,
    Done,
    Indexing,
}

/// Get the Houston frame for the current mood and tick
pub fn houston_frame(mood: &HoustonMood, tick: u64) -> (&'static str, &'static str) {
    match mood {
        HoustonMood::Idle => {
            let f = &HOUSTON_IDLE[(tick as usize / 8) % HOUSTON_IDLE.len()];
            (f.face, f.message)
        }
        HoustonMood::Routing => (HOUSTON_ROUTING.face, HOUSTON_ROUTING.message),
        HoustonMood::Thinking => {
            let f = &HOUSTON_THINKING[(tick as usize / 4) % HOUSTON_THINKING.len()];
            (f.face, f.message)
        }
        HoustonMood::Generating => {
            let f = &HOUSTON_GENERATING[(tick as usize / 5) % HOUSTON_GENERATING.len()];
            (f.face, f.message)
        }
        HoustonMood::Error => (HOUSTON_ERROR.face, HOUSTON_ERROR.message),
        HoustonMood::Done => (HOUSTON_DONE.face, HOUSTON_DONE.message),
        HoustonMood::Indexing => (HOUSTON_INDEXING.face, HOUSTON_INDEXING.message),
    }
}

/// Render Houston as 3 lines of text for the TUI
pub fn houston_lines(mood: &HoustonMood, tick: u64) -> [String; 3] {
    let (face, message) = houston_frame(mood, tick);
    [
        format!("   ╭─────╮"),
        format!("   │{}│  {}", face, message),
        format!("   ╰─────╯"),
    ]
}
