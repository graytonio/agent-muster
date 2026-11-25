//! The default config tree, embedded in the binary for `muster init`.

/// (relative path, contents)
pub const FILES: &[(&str, &str)] = &[
    ("muster.toml", include_str!("../defaults/muster.toml")),
    (
        "workflows/feature.toml",
        include_str!("../defaults/workflows/feature.toml"),
    ),
    (
        "workflows/bugfix.toml",
        include_str!("../defaults/workflows/bugfix.toml"),
    ),
    (
        "workflows/investigate.toml",
        include_str!("../defaults/workflows/investigate.toml"),
    ),
    (
        "prompts/planner.md",
        include_str!("../defaults/prompts/planner.md"),
    ),
    (
        "prompts/builder.md",
        include_str!("../defaults/prompts/builder.md"),
    ),
    (
        "prompts/reviewer.md",
        include_str!("../defaults/prompts/reviewer.md"),
    ),
    (
        "prompts/reproducer.md",
        include_str!("../defaults/prompts/reproducer.md"),
    ),
    (
        "prompts/fixer.md",
        include_str!("../defaults/prompts/fixer.md"),
    ),
    (
        "prompts/investigator.md",
        include_str!("../defaults/prompts/investigator.md"),
    ),
];
