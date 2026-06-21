//! Brand assets for terminal surfaces — an ASCII rendering of the brim
//! fill-gauge mark (a tank filling bottom-to-top, the "brimming" metaphor).
//!
//! SVG versions of the same mark live in `assets/`.

/// Three-line mark: a small tank filling up. Pair with [`WORDMARK`] beside it.
/// The fill characters (`░▒▓█`) are the accent-colored part.
pub const MARK: [&str; 3] = [
    "╭──╮",
    "│▒▓│",
    "│▓█│",
];

/// One-line brand glyph used inline (headers, prompts).
pub const GLYPH: &str = "▸";

/// Product name.
pub const NAME: &str = "brim";

/// Tagline shown under the wordmark.
pub const TAGLINE: &str = "AI quota tracker";
