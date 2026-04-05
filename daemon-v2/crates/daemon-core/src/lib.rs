//! Daemon runtime core boundary.
//!
//! This crate is the extraction target for daemon-only orchestration logic.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonInfo {
    pub service: &'static str,
}

impl Default for DaemonInfo {
    fn default() -> Self {
        Self {
            service: "triumvirate-daemon-v2",
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_service_name_is_stable() {
        let info = super::DaemonInfo::default();
        assert_eq!(info.service, "triumvirate-daemon-v2");
    }
}
