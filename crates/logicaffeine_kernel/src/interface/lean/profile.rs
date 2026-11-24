//! The Lean releases LOGOS knows, and the import profiles a result may be produced under.
//!
//! A [`Release`] is a fact about a Lean release, read from the release itself: the commit its
//! headers name and how its kernel reads a string literal. The tolerant reader (the Arena protocol)
//! consults releases only.
//!
//! A [`Profile`] is what a *result* is held to: one release, its toolchain, the one lean4export
//! revision that exports it (with the structural-key patch for every release after 4.14.0), the
//! SHA-256 of every exporter source after patching and, where the platform is pinned, of the built
//! exporter itself, and the assumptions a verified result may rest on. An export is admitted only
//! under the profile its header names, and only when the header names that release exactly — its
//! commit included — so a header retagged to another release is refused, never read under the
//! wrong rules ([`admit`]).
//!
//! `scripts/lean/lean-exporters.json` is the build recipe of every profile's exporter and
//! `scripts/lean/install-lean-exporters.py` builds them reproducibly into the exporter registry;
//! `lean_profile_registry` holds the recipes to this registry.

use super::intake::Header;
use super::{declined, DiagnosticCode, Result};

/// How a release's kernel reads a string literal: the constant it applies to the literal's
/// `List Char` of `Char.ofNat` code points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringLiterals {
    /// `String` is the structure `String.mk (data : List Char)`; a literal is
    /// `String.mk [Char.ofNat c, …]`.
    ListOfChars,
    /// `String` is UTF-8 bytes (`structure String where ofByteArray ::`); a literal is the
    /// definition `String.ofList [Char.ofNat c, …]`.
    Utf8Bytes,
}

impl StringLiterals {
    /// The kernel name of the constant a literal is packed with.
    pub fn packing_constant(self) -> &'static str {
        match self {
            StringLiterals::ListOfChars => "/s:\"String\"/s:\"mk\"",
            StringLiterals::Utf8Bytes => "/s:\"String\"/s:\"ofList\"",
        }
    }
    /// The kernel name of `String`'s one constructor in a release reading literals this way.
    pub fn string_constructor(self) -> &'static str {
        match self {
            StringLiterals::ListOfChars => "/s:\"String\"/s:\"mk\"",
            StringLiterals::Utf8Bytes => "/s:\"String\"/s:\"ofByteArray\"",
        }
    }
}

/// A Lean release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: &'static str,
    /// The release's full commit hash, as its exports' headers name it.
    pub githash: &'static str,
    pub string_literals: StringLiterals,
}

/// Every release LOGOS reads, oldest first. Each row is read from the release's own `Init.Prelude`
/// (`structure String`) and `lean --version`.
pub const RELEASES: [Release; 8] = [
    Release {
        version: "4.14.0",
        githash: "410fab7284703f41660ca2454218dcca9b2ec896",
        string_literals: StringLiterals::ListOfChars,
    },
    Release {
        version: "4.28.0",
        githash: "7e01a1bf5c70fc6167d49c345d3bf80596e9a79b",
        string_literals: StringLiterals::Utf8Bytes,
    },
    Release {
        version: "4.29.1",
        githash: "f72c35b3f637c8c6571d353742168ab66cc22c00",
        string_literals: StringLiterals::Utf8Bytes,
    },
    Release {
        version: "4.30.0",
        githash: "d024af099ca4bf2c86f649261ebf59565dc8c622",
        string_literals: StringLiterals::Utf8Bytes,
    },
    Release {
        version: "4.33.0",
        githash: "d8b18978322de05a8f3dba51ef03cf5461676c17",
        string_literals: StringLiterals::Utf8Bytes,
    },
    Release {
        version: "4.33.1",
        githash: "819816b2e0a3bf405af45ae5c7af2491d8f5bee6",
        string_literals: StringLiterals::Utf8Bytes,
    },
    Release {
        version: "4.34.0-rc1",
        githash: "3447a668783dbce1a8fdb97101dd067687b2b418",
        string_literals: StringLiterals::Utf8Bytes,
    },
    Release {
        version: "4.34.1",
        githash: "5045d0056413266e57c625dcd7c365b10e377c52",
        string_literals: StringLiterals::Utf8Bytes,
    },
];

/// The release `version` names, when LOGOS knows it.
pub fn release(version: &str) -> Option<&'static Release> {
    RELEASES.iter().find(|r| r.version == version)
}

/// How a profile's exporter is run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExporterForm {
    /// `lean --run Main.lean` from the built checkout, its library on `LEAN_PATH`.
    Interpreted,
    /// The `lean4export` executable `lake build` writes to `.lake/build/bin/`.
    Executable,
}

impl ExporterForm {
    pub fn as_str(self) -> &'static str {
        match self {
            ExporterForm::Interpreted => "interpreted",
            ExporterForm::Executable => "executable",
        }
    }
}

/// The one exporter a profile's results are produced with, identified by content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exporter {
    /// The name and version the exporter writes in its header.
    pub name: &'static str,
    pub version: &'static str,
    pub repository: &'static str,
    /// The upstream commit the exporter is built from.
    pub revision: &'static str,
    pub upstream_tag: Option<&'static str>,
    /// The SHA-256 of the upstream archive at `revision`.
    pub archive_sha256: &'static str,
    /// The SHA-256 of `scripts/lean/lean4export-structural.json`, when the structural-key patch is
    /// applied.
    pub structural_patch_sha256: Option<&'static str>,
    pub form: ExporterForm,
    /// `(path, SHA-256)` of the exporter's sources as built, the patch applied and `lean-toolchain`
    /// naming the profile's toolchain.
    pub sources: &'static [(&'static str, &'static str)],
    /// `(platform, SHA-256)` of the built executable on each platform its build is pinned for.
    pub binaries: &'static [(&'static str, &'static str)],
    /// The flags every result is exported with: metadata and unsafe declarations are exported so
    /// that the reader, not the exporter, decides what they mean.
    pub flags: &'static [&'static str],
}

impl Exporter {
    /// The pinned SHA-256 of the executable built for `platform`.
    pub fn binary_for(&self, platform: &str) -> Option<&'static str> {
        self.binaries
            .iter()
            .find(|(p, _)| *p == platform)
            .map(|(_, digest)| *digest)
    }
}

/// The toolchain and assumption profile a result is produced under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub lean_version: &'static str,
    /// The release's full commit hash.
    pub lean_githash: &'static str,
    /// The elan toolchain name.
    pub toolchain: &'static str,
    pub format_version: &'static str,
    pub exporter: Exporter,
    /// The assumptions a verified result may depend on, by Lean name.
    pub allowed_axioms: &'static [&'static str],
}

impl Profile {
    /// The release this profile produces results of.
    pub fn release(&self) -> &'static Release {
        release(self.lean_version).expect("every profile is a known release")
    }
    /// The exporter's identity as a result records it: name, revision, patch and, when pinned for
    /// `platform`, the executable's digest.
    pub fn exporter_identity(&self, platform: &str) -> String {
        let mut identity = format!("{} {}", self.exporter.name, self.exporter.revision);
        if let Some(patch) = self.exporter.structural_patch_sha256 {
            identity.push_str(&format!(" structural-patch:{patch}"));
        }
        if let Some(binary) = self.exporter.binary_for(platform) {
            identity.push_str(&format!(" sha256:{binary}"));
        }
        identity
    }
}

const REPOSITORY: &str = "https://github.com/leanprover/lean4export";
const FLAGS: &[&str] = &["--export-mdata", "--export-unsafe"];
const AXIOMS: &[&str] = &["propext", "Quot.sound", "Classical.choice"];
const STRUCTURAL_PATCH: &str = "ef137e3990e9205d3b172ce7f129565def6fcd4f00cd46471a91b5b08cccba5b";
const MAIN: (&str, &str) = (
    "Main.lean",
    "bd8e350903f1382b25fe767f9086dd9882000f01085f7046ea06545e7848842a",
);
const PARSE: (&str, &str) = (
    "Export/Parse.lean",
    "1f943b5cc776eece6d528937b976509c8bd57d10bc69a39b1c119bfb6d664286",
);
const LAKE_MANIFEST: (&str, &str) = (
    "lake-manifest.json",
    "f1b9d60d0ebd46f79f48e6833deefeaa835fdee12b4078241635ee14b6a0f07a",
);
const LAKEFILE: (&str, &str) = (
    "lakefile.toml",
    "54dde3aba280f32035c882dcd2f2039e738e20ed45ca538337b65cc69c02f7df",
);
/// The structurally patched `Export.lean` of upstream revisions 15f6055 (v4.33.0) and b18d673
/// (v4.34.0-rc1), whose sources agree.
const EXPORT_4_33: (&str, &str) = (
    "Export.lean",
    "e77e2ab2f85e39eed95779b92b90837edf807e14aadffeb7569ef85015e8805f",
);

/// Stage A: Lean 4.14.0, lean4export 3.1.0 at revision 20c4a64a run as `lean --run Main.lean`,
/// unpatched, export format 3.1.0, Lean core only, and Lean's three standard axioms.
pub const STAGE_A: Profile = Profile {
    lean_version: "4.14.0",
    lean_githash: "410fab7284703f41660ca2454218dcca9b2ec896",
    toolchain: "leanprover/lean4:v4.14.0",
    format_version: "3.1.0",
    exporter: Exporter {
        name: "lean4export",
        version: "3.1.0",
        repository: REPOSITORY,
        revision: "20c4a64aa1b76f7c79a6db15d089084359f8c3dc",
        upstream_tag: None,
        archive_sha256: "8c602d35c28462b924b57f4b519b817dd17d963fa75764732889af6c121d1cf3",
        structural_patch_sha256: None,
        form: ExporterForm::Interpreted,
        sources: &[
            MAIN,
            (
                "Export.lean",
                "16fa1fec370500f4ca03f042fc3e65cac27839b640b32c5a921e085206c7031b",
            ),
        ],
        binaries: &[],
        flags: FLAGS,
    },
    allowed_axioms: AXIOMS,
};

/// Lean 4.28.0, lean4export v4.28.0 (d065b00) with the structural-key patch.
pub const LEAN_4_28_0: Profile = Profile {
    lean_version: "4.28.0",
    lean_githash: "7e01a1bf5c70fc6167d49c345d3bf80596e9a79b",
    toolchain: "leanprover/lean4:v4.28.0",
    format_version: "3.1.0",
    exporter: Exporter {
        name: "lean4export",
        version: "3.1.0",
        repository: REPOSITORY,
        revision: "d065b0009aed0520e9e99752847a33b337661690",
        upstream_tag: Some("v4.28.0"),
        archive_sha256: "82f317a35664e8b90007b6e754a08cdc5f0065b5d3846dd2ec849817397a39e8",
        structural_patch_sha256: Some(STRUCTURAL_PATCH),
        form: ExporterForm::Executable,
        sources: &[
            (
                "Export.lean",
                "1dee205b41b6cad4f17c1f79c9c1921d69b2d264ab711aeee40ea56a5bb1e2ec",
            ),
            PARSE,
            MAIN,
            LAKE_MANIFEST,
            LAKEFILE,
            (
                "lean-toolchain",
                "db7bb24b756d745bbde83fe92718b51bd3625dae3701ba0f598d0eedcd3f3028",
            ),
        ],
        binaries: &[(
            "x86_64-unknown-linux-gnu",
            "1e5347101f199f2b06852fe9231b2092a231e13cfba3ca15d3457c025159f2c6",
        )],
        flags: FLAGS,
    },
    allowed_axioms: AXIOMS,
};

/// Lean 4.33.1, lean4export v4.33.0 (15f6055) with the structural-key patch.
pub const LEAN_4_33_1: Profile = Profile {
    lean_version: "4.33.1",
    lean_githash: "819816b2e0a3bf405af45ae5c7af2491d8f5bee6",
    toolchain: "leanprover/lean4:v4.33.1",
    format_version: "3.1.0",
    exporter: Exporter {
        name: "lean4export",
        version: "3.1.0",
        repository: REPOSITORY,
        revision: "15f6055e299ad5b89345e533cc2192f4cc00f659",
        upstream_tag: Some("v4.33.0"),
        archive_sha256: "266271a9c062e31535518475a20b69e92f8516d74832d3866f00298e7192eade",
        structural_patch_sha256: Some(STRUCTURAL_PATCH),
        form: ExporterForm::Executable,
        sources: &[
            EXPORT_4_33,
            PARSE,
            MAIN,
            LAKE_MANIFEST,
            LAKEFILE,
            (
                "lean-toolchain",
                "3aac669c7a910ec2389f4e4f921b605adf6ebf2d1e0c9b9cd0be4d33f3f5db71",
            ),
        ],
        binaries: &[(
            "x86_64-unknown-linux-gnu",
            "a38ef42f19a4e5cc33591a37ba9c478472b1528bd4995e9890c5681474baad70",
        )],
        flags: FLAGS,
    },
    allowed_axioms: AXIOMS,
};

/// Lean 4.34.0-rc1, lean4export v4.34.0-rc1 (b18d673) with the structural-key patch.
pub const LEAN_4_34_0_RC1: Profile = Profile {
    lean_version: "4.34.0-rc1",
    lean_githash: "3447a668783dbce1a8fdb97101dd067687b2b418",
    toolchain: "leanprover/lean4:v4.34.0-rc1",
    format_version: "3.1.0",
    exporter: Exporter {
        name: "lean4export",
        version: "3.1.0",
        repository: REPOSITORY,
        revision: "b18d673bd29b476466a51a3be1012df2ed322b10",
        upstream_tag: Some("v4.34.0-rc1"),
        archive_sha256: "e1a22bb11b1fff7bcf36fd1546c11d36ca0a72601d81b08e65673469d54e0003",
        structural_patch_sha256: Some(STRUCTURAL_PATCH),
        form: ExporterForm::Executable,
        sources: &[
            EXPORT_4_33,
            PARSE,
            MAIN,
            LAKE_MANIFEST,
            LAKEFILE,
            (
                "lean-toolchain",
                "cdbc6c372a2b37ad94430a6cec69cedfa4d36f255bfd968773fd91bf7a1746bf",
            ),
        ],
        binaries: &[(
            "x86_64-unknown-linux-gnu",
            "3bf07d7048ae1b731537dd0324e9eee316ab202fc8ba1c2b5f832186fa7f841d",
        )],
        flags: FLAGS,
    },
    allowed_axioms: AXIOMS,
};

/// Lean 4.34.1, lean4export v4.34.0 (076e8e5) with the structural-key patch.
pub const LEAN_4_34_1: Profile = Profile {
    lean_version: "4.34.1",
    lean_githash: "5045d0056413266e57c625dcd7c365b10e377c52",
    toolchain: "leanprover/lean4:v4.34.1",
    format_version: "3.1.0",
    exporter: Exporter {
        name: "lean4export",
        version: "3.1.0",
        repository: REPOSITORY,
        revision: "076e8e57707e813375e8f9da8bf989799ace9680",
        upstream_tag: Some("v4.34.0"),
        archive_sha256: "6841a09bbb920432388c6f260e3c43db3b9f73589b20658e0341c3b65104eb29",
        structural_patch_sha256: Some(STRUCTURAL_PATCH),
        form: ExporterForm::Executable,
        sources: &[
            (
                "Export.lean",
                "82938dc1c3654f52dbbb92786ae93e4b23822798951a2279b46661f7bfeb176b",
            ),
            PARSE,
            MAIN,
            LAKE_MANIFEST,
            LAKEFILE,
            (
                "lean-toolchain",
                "d5edba4e4b8faad9c1baeadb265716d20d03be4d1a2647dc5e35b0c0325bea7b",
            ),
        ],
        binaries: &[(
            "x86_64-unknown-linux-gnu",
            "34c93120387a834450c6c8c8cfaab19ecb4d7660be66a585f2940e47c6534ea1",
        )],
        flags: FLAGS,
    },
    allowed_axioms: AXIOMS,
};

/// Every registered profile, oldest release first.
pub const PROFILES: [Profile; 5] = [
    STAGE_A,
    LEAN_4_28_0,
    LEAN_4_33_1,
    LEAN_4_34_0_RC1,
    LEAN_4_34_1,
];

static REGISTRY: [Profile; 5] = PROFILES;

/// Every registered profile, oldest release first.
pub fn profiles() -> &'static [Profile] {
    &REGISTRY
}

/// The profile of Lean `version`, when one is registered.
pub fn by_version(version: &str) -> Option<&'static Profile> {
    REGISTRY.iter().find(|p| p.lean_version == version)
}

fn registered() -> String {
    REGISTRY
        .iter()
        .map(|p| p.lean_version)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The profile a project's `lean-toolchain` pins: `leanprover/lean4:v<version>` (or without the
/// `v`), surrounding whitespace ignored. Any other pin — another release, a nightly, another
/// repository, more than one line — is [`UnsupportedToolchain`](DiagnosticCode::UnsupportedToolchain).
pub fn for_toolchain(pin: &str) -> Result<&'static Profile> {
    let trimmed = pin.trim();
    let refused = || {
        declined(
            DiagnosticCode::UnsupportedToolchain,
            format!(
                "the project pins {trimmed:?}, which names no registered Lean profile (registered: {})",
                registered()
            ),
        )
    };
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return Err(refused());
    }
    let version = trimmed
        .strip_prefix("leanprover/lean4:")
        .map(|v| v.strip_prefix('v').unwrap_or(v))
        .ok_or_else(refused)?;
    by_version(version).ok_or_else(refused)
}

/// The profile an export's header is admitted under: its producer must be the profile's exporter,
/// its Lean release a registered profile's, and its commit that release's own. A header naming one
/// release with another's commit is a retagged header, and is refused.
pub fn admit(header: &Header) -> Result<&'static Profile> {
    let profile = by_version(&header.lean_version).ok_or_else(|| {
        declined(
            DiagnosticCode::UnsupportedToolchain,
            format!(
                "the export was produced by Lean {}, which has no registered import profile (registered: {})",
                header.lean_version,
                registered()
            ),
        )
    })?;
    hold(header, profile)?;
    Ok(profile)
}

/// Hold `header` to `profile`: the format, the exporter and the release with its commit.
pub fn hold(header: &Header, profile: &Profile) -> Result<()> {
    if header.format_version != profile.format_version {
        return Err(declined(
            DiagnosticCode::UnsupportedExportVersion,
            format!(
                "export format {} is not the supported format {}",
                header.format_version, profile.format_version
            ),
        ));
    }
    if header.lean_version != profile.lean_version
        || header.exporter_name != profile.exporter.name
        || header.exporter_version != profile.exporter.version
    {
        return Err(declined(
            DiagnosticCode::UnsupportedToolchain,
            format!(
                "the export was produced by Lean {} with {} {}; the supported profile is Lean {} with {} {}",
                header.lean_version,
                header.exporter_name,
                header.exporter_version,
                profile.lean_version,
                profile.exporter.name,
                profile.exporter.version
            ),
        ));
    }
    if header.lean_githash != profile.lean_githash {
        let named = RELEASES
            .iter()
            .find(|r| r.githash == header.lean_githash)
            .map(|r| format!(", which is Lean {}'s", r.version))
            .unwrap_or_default();
        return Err(declined(
            DiagnosticCode::UnsupportedToolchain,
            format!(
                "the export header names Lean {} but commit {:?}{named}; Lean {} is commit {}: a header retagged to another release is refused",
                header.lean_version, header.lean_githash, profile.lean_version, profile.lean_githash
            ),
        ));
    }
    Ok(())
}
