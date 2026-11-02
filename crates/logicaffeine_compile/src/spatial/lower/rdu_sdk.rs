//! The SDK bridge kit — every plausible RDU front door behind ONE object-safe seam, so the day the vendor SDK
//! arrives is a fill-in-the-blanks job in this file and nowhere else.
//!
//! A bridge has two halves. `ingest` turns the certified neutral artifact ([`NeutralArtifact`]) into whatever
//! the SDK takes ([`VendorConfig`]) and MUST forward the proof: the config is stamped with the certificate's
//! digest, and an artifact whose certificate does not recheck against its own program is refused before a
//! byte leaves. `certify_back` takes whatever the SDK hands back ([`VendorPlacement`] — a placement of the
//! same computation) and earns it the proof-carrying certificate through `certify_external_spatial`, or
//! `None`, fail-closed.
//!
//! [`MockBridge`] is the identity (the neutral JSON bytes in, the neutral JSON parsed back out) — it exists
//! so the round trip, the refusals and the CLI dispatch are locked TONIGHT against a bridge that behaves.
//! [`SambaFlowBridge`] refuses every call by name until the vendor API exists.

use std::fmt;

use super::rdu::parse_neutral;
use super::NeutralArtifact;
use crate::spatial::certificate::{certify_external_spatial, ChoreographyCertificate};
use crate::spatial::ir::SpatialProgram;

/// The media type the mock bridge emits: the neutral artifact, byte for byte.
pub const NEUTRAL_FORMAT: &str = "logos.spatial.neutral+json";

/// Why the SambaFlow bridge refuses today — the one string the CLI, the runbook and the lock all read.
pub const SAMBAFLOW_REASON: &str = "needs the vendor SDK";

/// Every bridge `bridge_by_name` dispatches to, in the order the CLI lists them.
pub const BRIDGE_NAMES: &[&str] = &["mock", "sambaflow"];

/// What a bridge hands the vendor: the bytes in the SDK's format, stamped with the digest of the certificate
/// they were proven under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VendorConfig {
    pub format: String,
    pub bytes: Vec<u8>,
    pub certificate_digest: String,
}

/// What the vendor hands back: their placement of the computation, as a neutral envelope (`schema`,
/// `version`, `lane`, `program` — exactly what `parse_neutral` reads).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VendorPlacement {
    pub program_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeRefusal {
    /// The bridge exists as a seam but the SDK behind it does not, yet.
    NotImplemented {
        bridge: &'static str,
        reason: &'static str,
    },
    /// The artifact is not a neutral artifact.
    Malformed(String),
    /// The artifact's certificate does not recheck against the program it travels with.
    Uncertified(String),
}

impl fmt::Display for BridgeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeRefusal::NotImplemented { bridge, reason } => {
                write!(f, "rdu bridge: {bridge} is not implemented — {reason}")
            }
            BridgeRefusal::Malformed(why) => {
                write!(f, "rdu bridge: malformed neutral artifact — {why}")
            }
            BridgeRefusal::Uncertified(why) => {
                write!(f, "rdu bridge: uncertified artifact — {why}")
            }
        }
    }
}

impl std::error::Error for BridgeRefusal {}

/// The digest a config carries: the certificate's schedule digest as sixteen lowercase hex digits.
pub fn certificate_digest(cert: &ChoreographyCertificate) -> String {
    format!("{:016x}", cert.schedule_digest)
}

/// The `certificate.schedule_digest` a neutral envelope carries, in the same sixteen-hex-digit form.
pub fn envelope_digest(json: &str) -> Result<String, BridgeRefusal> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| BridgeRefusal::Malformed(e.to_string()))?;
    let digest = v
        .get("certificate")
        .and_then(|c| c.get("schedule_digest"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            BridgeRefusal::Malformed("no certificate.schedule_digest in the envelope".to_string())
        })?;
    Ok(format!("{digest:016x}"))
}

/// One RDU front door. Object-safe: the CLI holds a `Box<dyn RdaBridge>` chosen by name.
pub trait RdaBridge {
    fn name(&self) -> &str;
    /// The neutral artifact → the SDK's config bytes, or a typed refusal. A bridge never emits without
    /// forwarding the certificate: the artifact's own proof must recheck against its own program first.
    fn ingest(&self, neutral: &NeutralArtifact) -> Result<VendorConfig, BridgeRefusal>;
    /// The SDK's placement of `source`'s computation → a certificate on THEIR mapping, or `None`.
    fn certify_back(
        &self,
        placement: &VendorPlacement,
        source: &SpatialProgram,
    ) -> Option<ChoreographyCertificate>;
}

/// The identity bridge: the config IS the neutral JSON, and a placement is read back with `parse_neutral`.
pub struct MockBridge;

impl RdaBridge for MockBridge {
    fn name(&self) -> &str {
        "mock"
    }

    fn ingest(&self, neutral: &NeutralArtifact) -> Result<VendorConfig, BridgeRefusal> {
        let program = parse_neutral(&neutral.json).map_err(BridgeRefusal::Malformed)?;
        let digest = certificate_digest(&neutral.certificate);
        // The envelope's own digest must be the certificate's: the bytes forwarded and the proof they
        // travel with are bound by that one number.
        let in_envelope = envelope_digest(&neutral.json)?;
        if in_envelope != digest {
            return Err(BridgeRefusal::Uncertified(format!("the envelope names certificate digest {in_envelope} but the certificate beside it is {digest}")));
        }
        if !neutral.certificate.recheck(&program) {
            return Err(BridgeRefusal::Uncertified(format!("the certificate (digest {digest}) does not recheck against the program in the artifact")));
        }
        Ok(VendorConfig {
            format: NEUTRAL_FORMAT.to_string(),
            bytes: neutral.json.clone().into_bytes(),
            certificate_digest: digest,
        })
    }

    fn certify_back(
        &self,
        placement: &VendorPlacement,
        source: &SpatialProgram,
    ) -> Option<ChoreographyCertificate> {
        let external = parse_neutral(&placement.program_json).ok()?;
        certify_external_spatial(&external, source)
    }
}

/// The SambaFlow front door: a seam with the vendor API behind it still to come. Every call refuses by name.
pub struct SambaFlowBridge;

impl RdaBridge for SambaFlowBridge {
    fn name(&self) -> &str {
        "sambaflow"
    }

    fn ingest(&self, _neutral: &NeutralArtifact) -> Result<VendorConfig, BridgeRefusal> {
        Err(BridgeRefusal::NotImplemented {
            bridge: "sambaflow",
            reason: SAMBAFLOW_REASON,
        })
    }

    fn certify_back(
        &self,
        _placement: &VendorPlacement,
        _source: &SpatialProgram,
    ) -> Option<ChoreographyCertificate> {
        None
    }
}

/// The bridge behind a name, or `None` for a name that is not one of [`BRIDGE_NAMES`].
pub fn bridge_by_name(name: &str) -> Option<Box<dyn RdaBridge>> {
    match name {
        "mock" => Some(Box::new(MockBridge)),
        "sambaflow" => Some(Box::new(SambaFlowBridge)),
        _ => None,
    }
}
