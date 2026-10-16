use std::{collections::HashMap, error::Error, fmt};

use serde::Deserialize;

use crate::{
    CleanupCandidate, CleanupMode, Confidence, FileRecord, RecoveryMode, RiskLevel,
    can_enter_quick_clean, is_forbidden_path,
};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct RuleMatchPaths {
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuleMatchContext {
    pub variables: Vec<(String, crate::NormalizedPath)>,
    pub running_processes: Option<Vec<String>>,
    pub admin_available: Option<bool>,
}

impl RuleMatchContext {
    pub fn with_variable(mut self, name: impl Into<String>, path: crate::NormalizedPath) -> Self {
        self.variables.push((name.into(), path));
        self
    }

    pub fn with_running_processes<I, S>(mut self, processes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.running_processes = Some(processes.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_admin_available(mut self, available: bool) -> Self {
        self.admin_available = Some(available);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
pub enum RuleOperation {
    #[default]
    #[serde(rename = "scan_candidates")]
    ScanCandidates,
    #[serde(rename = "shell_recycle_bin_metadata")]
    ShellRecycleBinMetadata,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct RuleDocument {
    id: Option<String>,
    name: Option<String>,
    category: Option<String>,
    group: Option<String>,
    display_name: Option<String>,
    #[serde(default)]
    display_order: u32,
    #[serde(default = "default_group_order")]
    group_order: u32,
    item_from_path_after: Option<String>,
    #[serde(default)]
    display_items: Vec<RuleDisplayItem>,
    #[serde(default)]
    priority: u32,
    mode: Option<CleanupMode>,
    risk: Option<RiskLevel>,
    confidence: Option<Confidence>,
    recovery: Option<RecoveryMode>,
    #[serde(default)]
    processes: Vec<String>,
    #[serde(default)]
    requires_admin: bool,
    warning: Option<String>,
    version: Option<String>,
    #[serde(default)]
    minimum_os: Option<String>,
    #[serde(rename = "match", default)]
    matching: RuleMatchPaths,
    #[serde(rename = "exclude", default)]
    excluding: RuleMatchPaths,
    #[serde(default)]
    operation: RuleOperation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct RuleDisplayItem {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub order: u32,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rule {
    id: String,
    name: String,
    category: String,
    group: String,
    display_name: String,
    display_order: u32,
    group_order: u32,
    item_from_path_after: Option<String>,
    display_items: Vec<RuleDisplayItem>,
    priority: u32,
    mode: CleanupMode,
    risk: RiskLevel,
    confidence: Confidence,
    recovery: RecoveryMode,
    processes: Vec<String>,
    requires_admin: bool,
    warning: String,
    version: String,
    minimum_os: Option<String>,
    matching: RuleMatchPaths,
    excluding: RuleMatchPaths,
    operation: RuleOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleError {
    InvalidToml(String),
    MissingField(&'static str),
    EmptyField(&'static str),
}

impl fmt::Display for RuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidToml(message) => write!(formatter, "invalid rule TOML: {message}"),
            Self::MissingField(field) => write!(formatter, "missing rule field: {field}"),
            Self::EmptyField(field) => write!(formatter, "empty rule field: {field}"),
        }
    }
}

impl Error for RuleError {}

impl Rule {
    pub fn from_toml(input: &str) -> Result<Self, RuleError> {
        let document: RuleDocument =
            toml::from_str(input).map_err(|error| RuleError::InvalidToml(error.to_string()))?;
        Self::try_from(document)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn group(&self) -> &str {
        &self.group
    }

    pub fn display_order(&self) -> u32 {
        self.display_order
    }

    pub fn group_order(&self) -> u32 {
        self.group_order
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn display_items(&self) -> &[RuleDisplayItem] {
        &self.display_items
    }

    pub fn display_item_for_path(&self, path: &crate::NormalizedPath) -> Option<&RuleDisplayItem> {
        self.display_items.iter().find(|item| {
            item.paths.iter().any(|pattern| {
                display_pattern_may_match(pattern, path)
                    && path_matches(&pattern.replace('/', "\\"), path.display())
            })
        })
    }

    pub fn display_name_for_path(&self, path: &crate::NormalizedPath) -> String {
        self.path_item(path)
            .unwrap_or_else(|| self.display_name.clone())
    }

    fn path_item(&self, path: &crate::NormalizedPath) -> Option<String> {
        let marker = self.item_from_path_after.as_ref()?;
        let marker = marker.split(['/', '\\']).collect::<Vec<_>>();
        let components = path.display().split('\\').collect::<Vec<_>>();
        components
            .windows(marker.len() + 1)
            .find(|window| {
                window[..marker.len()]
                    .iter()
                    .zip(&marker)
                    .all(|(left, right)| left.eq_ignore_ascii_case(right))
            })
            .map(|window| window[marker.len()].to_owned())
            .filter(|item| !item.is_empty())
    }

    pub fn priority(&self) -> u32 {
        self.priority
    }

    pub fn mode(&self) -> CleanupMode {
        self.mode
    }

    pub fn risk(&self) -> RiskLevel {
        self.risk
    }

    pub fn confidence(&self) -> Confidence {
        self.confidence
    }

    pub fn recovery(&self) -> RecoveryMode {
        self.recovery
    }

    pub fn processes(&self) -> &[String] {
        &self.processes
    }

    pub fn requires_admin(&self) -> bool {
        self.requires_admin
    }

    pub fn warning(&self) -> &str {
        &self.warning
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn minimum_os(&self) -> Option<&str> {
        self.minimum_os.as_deref()
    }

    pub fn operation(&self) -> RuleOperation {
        self.operation
    }

    pub fn matches_path(&self, path: &crate::NormalizedPath) -> bool {
        self.matches_path_with_variables(path, &[])
    }

    pub fn matches_path_with_variables(
        &self,
        path: &crate::NormalizedPath,
        variables: &[(&str, crate::NormalizedPath)],
    ) -> bool {
        let variables = variables
            .iter()
            .map(|(name, path)| ((*name).to_owned(), path.clone()))
            .collect::<Vec<_>>();
        self.matches_path_with_context(
            path,
            &RuleMatchContext {
                variables,
                ..RuleMatchContext::default()
            },
        )
    }

    pub fn matches_path_with_context(
        &self,
        path: &crate::NormalizedPath,
        context: &RuleMatchContext,
    ) -> bool {
        if self.operation != RuleOperation::ScanCandidates || is_forbidden_path(path) {
            return false;
        }
        let matches = |pattern: &str| {
            path_matches(
                &expand_variables(pattern, &context.variables),
                path.display(),
            )
        };
        self.matching
            .paths
            .iter()
            .any(|pattern| self.may_match_path_pattern(pattern, path, context) && matches(pattern))
            && !self.excluding.paths.iter().any(|pattern| matches(pattern))
    }

    pub fn may_match_path(&self, path: &crate::NormalizedPath) -> bool {
        self.matching
            .paths
            .iter()
            .any(|pattern| self.may_match_path_pattern(pattern, path, &RuleMatchContext::default()))
    }

    fn may_match_path_pattern(
        &self,
        pattern: &str,
        path: &crate::NormalizedPath,
        context: &RuleMatchContext,
    ) -> bool {
        let expanded = expand_variables(pattern, &context.variables);
        let prefix = expanded
            .split(['*', '?'])
            .next()
            .unwrap_or_default()
            .trim_end_matches(['\\', '/']);
        prefix.is_empty() || path_prefix_matches(prefix, path.display())
    }

    pub fn candidate_for(&self, file: &FileRecord) -> Option<CleanupCandidate> {
        self.candidate_for_with_context(file, &RuleMatchContext::default())
    }

    pub fn candidate_for_with_context(
        &self,
        file: &FileRecord,
        context: &RuleMatchContext,
    ) -> Option<CleanupCandidate> {
        if file.facts().reparse_kind.is_some()
            || !self.matches_path_with_context(file.path(), context)
        {
            return None;
        }

        let mut block_reasons = Vec::new();
        if !self.processes.is_empty() {
            match &context.running_processes {
                None => block_reasons.push("related process state is unknown".to_owned()),
                Some(running) => {
                    if let Some(process) = self.processes.iter().find(|process| {
                        running
                            .iter()
                            .any(|item| item.eq_ignore_ascii_case(process))
                    }) {
                        block_reasons.push(format!("related process is running: {process}"));
                    }
                }
            }
        }
        if self.requires_admin && !matches!(context.admin_available, Some(true)) {
            block_reasons.push("administrator availability is unknown or unavailable".to_owned());
        }

        let quick_clean_block_reason =
            (!block_reasons.is_empty()).then(|| block_reasons.join("; "));
        let reason = match &quick_clean_block_reason {
            Some(block_reason) => format!("{}; quick clean blocked: {block_reason}", self.warning),
            None => self.warning.clone(),
        };
        let mut candidate = CleanupCandidate {
            path: file.path().clone(),
            file_identity: Some(file.identity()),
            rule_id: Some(self.id.clone()),
            category: Some(
                self.display_item_for_path(file.path())
                    .map(|item| item.key.clone())
                    .or_else(|| {
                        self.path_item(file.path())
                            .map(|item| format!("{}/{item}", self.category))
                    })
                    .unwrap_or_else(|| self.category.clone()),
            ),
            reason: Some(reason),
            estimated_size: Some(file.logical_size().max(file.allocated_size())),
            quick_clean_eligible: false,
            quick_clean_block_reason,
            mode: Some(self.mode),
            risk: Some(self.risk),
            confidence: Some(self.confidence),
            recovery: Some(self.recovery),
            locked: Some(file.locked()),
            path_allowed: None,
            file_identity_unchanged: None,
            last_write_time: Some(file.facts().last_write_time),
        };
        candidate.quick_clean_eligible =
            candidate.quick_clean_block_reason.is_none() && can_enter_quick_clean(&candidate);
        Some(candidate)
    }
}

impl TryFrom<RuleDocument> for Rule {
    type Error = RuleError;

    fn try_from(document: RuleDocument) -> Result<Self, Self::Error> {
        let required = |value: Option<String>, field: &'static str| {
            value
                .ok_or(RuleError::MissingField(field))
                .and_then(|value| {
                    if value.trim().is_empty() {
                        Err(RuleError::EmptyField(field))
                    } else {
                        Ok(value)
                    }
                })
        };
        let name = required(document.name, "name")?;
        Ok(Self {
            id: required(document.id, "id")?,
            name: name.clone(),
            category: required(document.category, "category")?,
            group: document.group.unwrap_or_else(|| "其他清理".into()),
            display_name: document.display_name.unwrap_or(name),
            display_order: document.display_order,
            group_order: document.group_order,
            item_from_path_after: document.item_from_path_after,
            display_items: document.display_items,
            priority: document.priority,
            mode: document.mode.ok_or(RuleError::MissingField("mode"))?,
            risk: document.risk.ok_or(RuleError::MissingField("risk"))?,
            confidence: document
                .confidence
                .ok_or(RuleError::MissingField("confidence"))?,
            recovery: document
                .recovery
                .ok_or(RuleError::MissingField("recovery"))?,
            processes: document.processes,
            requires_admin: document.requires_admin,
            warning: required(document.warning, "warning")?,
            version: required(document.version, "version")?,
            minimum_os: document.minimum_os,
            matching: document.matching,
            excluding: document.excluding,
            operation: document.operation,
        })
    }
}

fn default_group_order() -> u32 {
    100
}

pub fn generate_candidates(
    rules: &[Rule],
    files: &[FileRecord],
    context: &RuleMatchContext,
) -> Vec<CleanupCandidate> {
    let mut selected: HashMap<crate::FileIdentity, (usize, &Rule, CleanupCandidate)> =
        HashMap::new();
    for (rule_order, rule) in rules.iter().enumerate() {
        for file in files {
            let Some(candidate) = rule.candidate_for_with_context(file, context) else {
                continue;
            };
            let key = file.identity();
            let replace = selected.get(&key).is_none_or(|(old_order, old_rule, _)| {
                rule.priority > old_rule.priority
                    || (rule.priority == old_rule.priority && rule_order < *old_order)
            });
            if replace {
                selected.insert(key, (rule_order, rule, candidate));
            }
        }
    }
    let mut candidates = selected
        .into_values()
        .map(|(_, _, candidate)| candidate)
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.path.display().cmp(right.path.display()));
    candidates
}

fn expand_variables(pattern: &str, variables: &[(String, crate::NormalizedPath)]) -> String {
    let mut expanded = pattern.replace('/', "\\");
    for (name, path) in variables {
        expanded = expanded.replace(&format!("%{name}%"), path.display());
    }
    expanded
}

fn path_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_end_matches('\\');
    let path = path.trim_end_matches('\\');
    if !pattern.contains('*') && !pattern.contains('?') {
        return path_prefix_matches(pattern, path);
    }
    glob_matches(pattern.as_bytes(), path.as_bytes())
}

fn display_pattern_may_match(pattern: &str, path: &crate::NormalizedPath) -> bool {
    let normalized = pattern.replace('/', "\\");
    let prefix = normalized
        .split(['*', '?'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(['\\', '/']);
    prefix.is_empty() || path_prefix_matches(prefix, path.display())
}

fn path_prefix_matches(pattern: &str, path: &str) -> bool {
    let mut path_chars = path.chars();
    for pattern_char in pattern.chars() {
        let Some(path_char) = path_chars.next() else {
            return false;
        };
        if !path_char.eq_ignore_ascii_case(&pattern_char) {
            return false;
        }
    }
    matches!(path_chars.next(), None | Some('\\'))
}

fn glob_matches(pattern: &[u8], path: &[u8]) -> bool {
    let mut pattern_index = 0;
    let mut path_index = 0;
    let mut star_index = None;
    let mut star_path_index = 0;

    while path_index < path.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' && path[path_index] != b'\\'
                || pattern[pattern_index] != b'*'
                    && pattern[pattern_index].eq_ignore_ascii_case(&path[path_index]))
        {
            pattern_index += 1;
            path_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            star_path_index = path_index;
            pattern_index += 1;
        } else if let Some(star) = star_index
            && (path[star_path_index] != b'\\' || star + 1 == pattern.len())
        {
            star_path_index += 1;
            path_index = star_path_index;
            pattern_index = star + 1;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}
