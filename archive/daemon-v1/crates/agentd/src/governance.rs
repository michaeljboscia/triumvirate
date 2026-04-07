#![allow(dead_code)]

use std::path::PathBuf;

use crate::config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GovernedAction {
    GitPush,
    FileDelete,
    DbDrop,
    FleetMerge,
}

impl GovernedAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitPush => "git_push",
            Self::FileDelete => "file_delete",
            Self::DbDrop => "db_drop",
            Self::FleetMerge => "fleet_merge",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GovernanceDecision {
    pub allowed: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct GovernanceEngine {
    policy_dir: PathBuf,
}

impl GovernanceEngine {
    pub fn default() -> Self {
        let policy_dir = config::dirs().join("policies");
        Self { policy_dir }
    }

    pub fn evaluate(&self, action: GovernedAction, human_approved: bool) -> GovernanceDecision {
        // Cedar policy engine bridge point:
        // Until Cedar integration lands, enforce conservative default:
        // destructive actions require explicit human approval.
        let destructive = matches!(
            action,
            GovernedAction::GitPush
                | GovernedAction::FileDelete
                | GovernedAction::DbDrop
                | GovernedAction::FleetMerge
        );

        if destructive && !human_approved {
            return GovernanceDecision {
                allowed: false,
                reason: format!(
                    "action '{}' requires human approval",
                    action.as_str()
                ),
            };
        }

        GovernanceDecision {
            allowed: true,
            reason: "allowed by default policy".to_string(),
        }
    }

    pub fn policy_dir(&self) -> &PathBuf {
        &self.policy_dir
    }
}

