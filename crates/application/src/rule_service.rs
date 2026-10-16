use std::{error::Error, fmt};

use domain::{
    CleanupCandidate, FileRecord, Rule, RuleError, RuleMatchContext, RuleOperation,
    generate_candidates,
};

use crate::{AppError, RulePort};

impl RulePort for RuleService {
    fn rules(&self) -> Result<Vec<Rule>, AppError> {
        Ok(self.rules().to_vec())
    }
}

pub struct RuleService {
    rules: Vec<Rule>,
}

#[derive(Debug)]
pub struct RuleServiceError {
    rule_id: &'static str,
    source: RuleError,
}

impl RuleServiceError {
    pub fn rule_id(&self) -> &str {
        self.rule_id
    }
}

impl fmt::Display for RuleServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to load {}: {}",
            self.rule_id, self.source
        )
    }
}

impl Error for RuleServiceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

impl RuleService {
    pub fn load_builtin() -> Result<Self, RuleServiceError> {
        let sources = [
            (
                "system_temp",
                include_str!("../../../rules/builtin/system_temp.toml"),
            ),
            (
                "system_logs",
                include_str!("../../../rules/builtin/system_logs.toml"),
            ),
            (
                "recycle_bin",
                include_str!("../../../rules/builtin/recycle_bin.toml"),
            ),
            (
                "edge_cache",
                include_str!("../../../rules/builtin/edge_cache.toml"),
            ),
            (
                "chrome_cache",
                include_str!("../../../rules/builtin/chrome_cache.toml"),
            ),
            (
                "firefox_cache",
                include_str!("../../../rules/builtin/firefox_cache.toml"),
            ),
            (
                "app_cache",
                include_str!("../../../rules/builtin/app_cache.toml"),
            ),
            (
                "wechat_cache",
                include_str!("../../../rules/builtin/wechat_cache.toml"),
            ),
        ];
        let rules = sources
            .into_iter()
            .map(|(rule_id, source)| {
                Rule::from_toml(source).map_err(|source| RuleServiceError { rule_id, source })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { rules })
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn candidates(
        &self,
        files: &[FileRecord],
        context: &RuleMatchContext,
    ) -> Vec<CleanupCandidate> {
        generate_candidates(&self.rules, files, context)
    }

    pub fn shell_recycle_bin_rule(&self) -> Option<&Rule> {
        self.rules
            .iter()
            .find(|rule| rule.operation() == RuleOperation::ShellRecycleBinMetadata)
    }
}
