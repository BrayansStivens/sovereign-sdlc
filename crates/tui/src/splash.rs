//! Splash Screen + Generation Animation
//!
//! ASCII art startup + animated robot during generation.

/// Startup ASCII art — shown once at launch
pub const SPLASH: &[&str] = &[
    "",
    r"    ___  _____  _   _ _____ _____ _____ _____ _____ _   _ ",
    r"   / __||  _  || | | |  ___|  _  |  ___|_   _|  ___| \ | |",
    r"   \__ \| | | || | | | |__ | |_| | |__   | | | | __|  \| |",
    r"   |___/|_| |_| \_/ |____|_| |_|____|  |_| |_____|_|\__|",
    r"                   S  D  L  C    v0.4",
    "",
];

/// Robot animation frames for generation (8 frames)
pub const ROBOT_WORKING: &[&[&str]] = &[
    // Frame 0: idle
    &[
        "  [::] ",
        " /|==|\\",
        "  /  \\ ",
        "  ----  ",
    ],
    // Frame 1: right arm up
    &[
        "  [::] ",
        " /|==|/",
        "  /  \\ ",
        "  ----  ",
    ],
    // Frame 2: both arms up
    &[
        "  [::] ",
        " \\|==|/",
        "  /  \\ ",
        "  ----  ",
    ],
    // Frame 3: typing
    &[
        "  [..] ",
        " /|==|\\",
        "  |  | ",
        "  ====  ",
    ],
    // Frame 4: thinking
    &[
        "  [??] ",
        " /|==|\\",
        "  /  \\ ",
        "  o  o  ",
    ],
    // Frame 5: working
    &[
        "  [><] ",
        " \\|==|\\",
        "  /  \\ ",
        "  ----  ",
    ],
    // Frame 6: sparks
    &[
        " *[::]*",
        " /|==|\\",
        "  /  \\ ",
        "  ----  ",
    ],
    // Frame 7: done flash
    &[
        "  [OK] ",
        " \\|==|/",
        "  /  \\ ",
        "  ====  ",
    ],
];

/// Get the current robot frame based on tick
pub fn robot_frame(tick: u64) -> &'static [&'static str] {
    let idx = (tick as usize / 2) % (ROBOT_WORKING.len() - 1); // Skip "done" frame
    ROBOT_WORKING[idx]
}

/// Get the "done" frame
pub fn robot_done() -> &'static [&'static str] {
    ROBOT_WORKING[ROBOT_WORKING.len() - 1]
}

/// Progress dots animation (longer cycle)
pub fn progress_dots(tick: u64) -> &'static str {
    match (tick / 3) % 4 {
        0 => "    ",
        1 => ".   ",
        2 => "..  ",
        _ => "... ",
    }
}
