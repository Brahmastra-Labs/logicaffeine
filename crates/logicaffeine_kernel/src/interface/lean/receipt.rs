//! The stages a Lean → LOGOS result passes through, named separately so that no result is ever one
//! unexplained "verified" Boolean (competition rule R1). The names are the contract's wire names.

/// Every stage, in the order a result passes through them.
pub const STAGES: [&str; 7] = [
    "lean_accepted",
    "translated",
    "primary_check",
    "independent_recheck",
    "assumption_policy",
    "emission_reparsed",
    "fresh_replay",
];

/// The stages a replay establishes from a package's bytes alone. `lean_accepted` is not among them:
/// a replay never runs Lean, and says so rather than inheriting the producing run's word.
pub const REPLAY_STAGES: [&str; 6] = [
    "translated",
    "primary_check",
    "independent_recheck",
    "assumption_policy",
    "emission_reparsed",
    "fresh_replay",
];

/// What happened in one stage. `NotRun` and `Unsupported` are never a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageStatus {
    Passed,
    Failed,
    NotRun,
    Unsupported,
}

impl StageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StageStatus::Passed => "passed",
            StageStatus::Failed => "failed",
            StageStatus::NotRun => "not_run",
            StageStatus::Unsupported => "unsupported",
        }
    }

    /// The status a wire name denotes.
    pub fn parse(name: &str) -> Option<StageStatus> {
        match name {
            "passed" => Some(StageStatus::Passed),
            "failed" => Some(StageStatus::Failed),
            "not_run" => Some(StageStatus::NotRun),
            "unsupported" => Some(StageStatus::Unsupported),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage {
    pub name: &'static str,
    pub status: StageStatus,
    pub detail: String,
    /// The machine-readable reason a stage failed or was unsupported, when it did.
    pub code: Option<String>,
}

/// A receipt: one entry per stage in [`STAGES`], in order. Verified only when every stage passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub stages: Vec<Stage>,
}

impl Receipt {
    /// A receipt in which nothing has run yet.
    pub fn unstarted() -> Self {
        Receipt {
            stages: STAGES
                .iter()
                .map(|name| Stage {
                    name,
                    status: StageStatus::NotRun,
                    detail: String::new(),
                    code: None,
                })
                .collect(),
        }
    }
    pub fn stage(&self, name: &str) -> Option<&Stage> {
        self.stages.iter().find(|s| s.name == name)
    }
    pub fn set(&mut self, name: &str, status: StageStatus, detail: impl Into<String>) {
        let stage = self
            .stages
            .iter_mut()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("{name} is not a receipt stage"));
        stage.status = status;
        stage.detail = detail.into();
        stage.code = None;
    }
    /// [`Receipt::set`] with the machine-readable reason.
    pub fn set_with_code(
        &mut self,
        name: &str,
        status: StageStatus,
        code: &str,
        detail: impl Into<String>,
    ) {
        self.set(name, status, detail);
        self.stages
            .iter_mut()
            .find(|s| s.name == name)
            .expect("set above")
            .code = Some(code.to_string());
    }
    pub fn status(&self, name: &str) -> StageStatus {
        self.stage(name).map_or(StageStatus::NotRun, |s| s.status)
    }
    pub fn verified(&self) -> bool {
        self.stages.len() == STAGES.len()
            && self.stages.iter().all(|s| s.status == StageStatus::Passed)
    }
    /// Every stage a replay establishes passed. Not the same as [`Receipt::verified`]: a replay
    /// establishes everything except that the pinned Lean accepted the source, which only the
    /// producing run observes.
    pub fn replayed(&self) -> bool {
        self.stages.len() == STAGES.len()
            && REPLAY_STAGES
                .iter()
                .all(|name| self.status(name) == StageStatus::Passed)
    }
    /// The first stage that failed, in stage order.
    pub fn first_failure(&self) -> Option<&Stage> {
        self.stages.iter().find(|s| s.status == StageStatus::Failed)
    }
    /// The receipt as the contract's `stages` object, with a detail and code for each stage.
    pub fn to_json(&self) -> serde_json::Value {
        let mut stages = serde_json::Map::new();
        for stage in &self.stages {
            stages.insert(
                stage.name.to_string(),
                serde_json::json!({ "status": stage.status.as_str(), "code": stage.code, "detail": stage.detail }),
            );
        }
        serde_json::Value::Object(stages)
    }
}
