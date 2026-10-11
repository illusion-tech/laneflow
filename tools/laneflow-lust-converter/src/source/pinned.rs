//! §2.2 pinned LuST Scenario v2.0 source revision and file digests.

/// Upstream repository URL (human reference only).
pub const LUST_REPOSITORY: &str = "https://github.com/lcodeca/LuSTScenario";

/// Upstream tag (human reference only; commit is authority).
pub const LUST_TAG: &str = "v2.0";

/// Exact source revision authority.
pub const LUST_COMMIT: &str = "c4bd5bd3751d426d42a9a1749c815e47ea188549";

/// One pinned input file under the LuST checkout root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinnedSourceFile {
    /// Path relative to the LuST repository root.
    pub relative_path: &'static str,
    /// Exact byte length.
    pub bytes: u64,
    /// Lowercase hex SHA-256 of the file contents (no `sha256:` prefix).
    pub sha256_hex: &'static str,
}

/// Fixed consumption set from `docs/design/real-road-workloads.md` §2.2.
pub const PINNED_SOURCE_FILES: &[PinnedSourceFile] = &[
    PinnedSourceFile {
        relative_path: "scenario/lust.net.xml",
        bytes: 10_940_662,
        sha256_hex: "6f5d76223cf14b797ae6267f13b23eb6c872d76adec1fb22a8569a806dc09341",
    },
    PinnedSourceFile {
        relative_path: "scenario/lust.poly.xml",
        bytes: 2_743_451,
        sha256_hex: "abb519b3e12e0392111e9d0c3517e8f54ec424dedbb54dd9ceaf31a9eb3fcc8e",
    },
    PinnedSourceFile {
        relative_path: "scenario/tll.static.xml",
        bytes: 83_530,
        sha256_hex: "893f4e0e9ffb8e8eed67caeafada8fec958fb92f437aa41f69783c059c474102",
    },
    PinnedSourceFile {
        relative_path: "scenario/vtypes.add.xml",
        bytes: 1_707,
        sha256_hex: "8bfa1f4f2c51f4f15066e5d799027572f3482aaaa67fb37ee10a75cecf42ee92",
    },
    PinnedSourceFile {
        relative_path: "scenario/DUERoutes/local.static.0.rou.xml",
        bytes: 41_634_764,
        sha256_hex: "08a76518941bcb56ee42a50e34a360efade103a21a6ea3d7d19f5a66765e6503",
    },
    PinnedSourceFile {
        relative_path: "scenario/DUERoutes/local.static.1.rou.xml",
        bytes: 42_862_343,
        sha256_hex: "1e657f82da83d43869d9bf91d7f20c997bc9b2d031cd4f08ccdc3ccd84836ccf",
    },
    PinnedSourceFile {
        relative_path: "scenario/DUERoutes/local.static.2.rou.xml",
        bytes: 44_934_267,
        sha256_hex: "bdd93df275c624f44db4d3bbdc659cd91f237a5951b1d09fa0be105264ecdd35",
    },
    PinnedSourceFile {
        relative_path: "LICENSE.md",
        bytes: 1_134,
        sha256_hex: "9a16c8681095f730e72ad6efe2b56c30a924552d4b6b6326868309749d64837b",
    },
];
