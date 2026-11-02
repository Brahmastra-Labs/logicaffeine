//! `verify` — attach the choreography certificate, or refuse.

use super::SpatialReject;
use crate::spatial::certificate::{certify_choreography, CertifyFault};
use crate::spatial::ir::*;
use crate::spatial::target::SpatialTarget;

pub fn verify(
    p: &SpatialProgram,
    _target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    if p.certificate.is_some() {
        return Ok(p.clone());
    }
    if p.schedule.is_none() {
        return Err(SpatialReject::WrongPhase {
            pass: "verify",
            expected: "Scheduled",
            found: p.phase(),
        });
    }
    p.validate()?;
    match certify_choreography(p) {
        Ok(cert) => {
            let mut q = p.clone();
            q.certificate = Some(cert);
            Ok(q)
        }
        Err(CertifyFault::Invalid(v)) => Err(SpatialReject::Invalid(v)),
        Err(CertifyFault::Eval(f)) => Err(SpatialReject::Unverified(f)),
        Err(other) => Err(SpatialReject::Unverified(
            crate::spatial::eval::ChoreoFault::Unsupported(format!("{other:?}")),
        )),
    }
}
