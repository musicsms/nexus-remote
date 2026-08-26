//! Policy and role models for the Nexus policy engine.
//!
//! Provides [`DeviceLabelSelector`], [`PolicyConditions`], [`SessionRestrictions`], and [`Role`].

use crate::action::{Action, ActionSet};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Default maximum session duration in seconds (1 hour = 3600 seconds).
pub const DEFAULT_MAX_DURATION_SECONDS: u64 = 3600;

/// Represents key-value label matchers or wildcard selectors for target devices.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceLabelSelector {
    /// Wildcard selector matching any target device regardless of labels.
    #[default]
    Wildcard,
    /// Exact match on a single key-value label pair.
    Exact { key: String, value: String },
    /// Match when all specified key-value label pairs are present and equal.
    MatchAll { labels: BTreeMap<String, String> },
}

impl DeviceLabelSelector {
    /// Creates a wildcard selector matching any device.
    #[must_use]
    pub const fn wildcard() -> Self {
        Self::Wildcard
    }

    /// Creates an exact key-value label selector.
    ///
    /// If both `key` and `value` are `"*"`, this constructs a wildcard selector.
    #[must_use]
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        let value = value.into();
        if key == "*" && value == "*" {
            Self::Wildcard
        } else {
            Self::Exact { key, value }
        }
    }

    /// Creates an exact key-value label selector.
    #[must_use]
    pub fn exact(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self::Exact {
            key: key.into(),
            value: value.into(),
        }
    }

    /// Creates a selector that matches when all specified key-value pairs are matched.
    #[must_use]
    pub fn match_all<K: Into<String>, V: Into<String>, I: IntoIterator<Item = (K, V)>>(
        labels: I,
    ) -> Self {
        Self::MatchAll {
            labels: labels
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// Returns `true` if the given device labels satisfy this selector.
    #[must_use]
    pub fn matches(&self, labels: &HashMap<String, String>) -> bool {
        match self {
            Self::Wildcard => true,
            Self::Exact { key, value } => {
                if key == "*" && value == "*" {
                    true
                } else if value == "*" {
                    labels.contains_key(key)
                } else if key == "*" {
                    labels.values().any(|v| v == value)
                } else {
                    labels.get(key) == Some(value)
                }
            }
            Self::MatchAll {
                labels: selector_labels,
            } => selector_labels.iter().all(|(k, v)| {
                if k == "*" && v == "*" {
                    true
                } else if v == "*" {
                    labels.contains_key(k)
                } else if k == "*" {
                    labels.values().any(|val| val == v)
                } else {
                    labels.get(k) == Some(v)
                }
            }),
        }
    }
}

impl From<(&str, &str)> for DeviceLabelSelector {
    fn from((k, v): (&str, &str)) -> Self {
        Self::new(k, v)
    }
}

impl From<(String, String)> for DeviceLabelSelector {
    fn from((k, v): (String, String)) -> Self {
        Self::new(k, v)
    }
}

impl From<BTreeMap<String, String>> for DeviceLabelSelector {
    fn from(labels: BTreeMap<String, String>) -> Self {
        Self::MatchAll { labels }
    }
}

impl From<HashMap<String, String>> for DeviceLabelSelector {
    fn from(labels: HashMap<String, String>) -> Self {
        Self::MatchAll {
            labels: labels.into_iter().collect(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Attribute-Based Access Control (ABAC) conditions and security constraints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyConditions {
    /// Whether multi-factor authentication is required for this policy.
    pub require_mfa: bool,

    /// Whether the client device must be an enterprise-managed device.
    pub managed_client_device: bool,

    /// Whether clipboard operations (read/write) are permitted in the session.
    #[serde(default = "default_true")]
    pub clipboard_enabled: bool,

    /// Whether file transfers (upload/download) are permitted in the session.
    #[serde(default = "default_true")]
    pub file_transfer_enabled: bool,

    /// Whether audio streaming (listen/send) is permitted in the session.
    #[serde(default = "default_true")]
    pub audio_enabled: bool,

    /// Maximum session duration in seconds, if constrained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_seconds: Option<u64>,

    /// Maximum display resolution width in pixels, if constrained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution_width: Option<u32>,

    /// Maximum display resolution height in pixels, if constrained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution_height: Option<u32>,

    /// Allowed client IP CIDR ranges or addresses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_ip_ranges: Vec<String>,
}

impl Default for PolicyConditions {
    fn default() -> Self {
        Self {
            require_mfa: false,
            managed_client_device: false,
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: None,
            max_resolution_width: None,
            max_resolution_height: None,
            allowed_ip_ranges: Vec::new(),
        }
    }
}

impl PolicyConditions {
    /// Creates a new `PolicyConditions` with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets whether multi-factor authentication is required.
    #[must_use]
    pub const fn with_require_mfa(mut self, require_mfa: bool) -> Self {
        self.require_mfa = require_mfa;
        self
    }

    /// Sets whether a managed client device is required.
    #[must_use]
    pub const fn with_managed_client_device(mut self, managed_client_device: bool) -> Self {
        self.managed_client_device = managed_client_device;
        self
    }

    /// Sets whether clipboard access is enabled.
    #[must_use]
    pub const fn with_clipboard_enabled(mut self, clipboard_enabled: bool) -> Self {
        self.clipboard_enabled = clipboard_enabled;
        self
    }

    /// Sets whether file transfer is enabled.
    #[must_use]
    pub const fn with_file_transfer_enabled(mut self, file_transfer_enabled: bool) -> Self {
        self.file_transfer_enabled = file_transfer_enabled;
        self
    }

    /// Sets whether audio streaming is enabled.
    #[must_use]
    pub const fn with_audio_enabled(mut self, audio_enabled: bool) -> Self {
        self.audio_enabled = audio_enabled;
        self
    }

    /// Sets the maximum session duration in seconds.
    #[must_use]
    pub const fn with_max_duration_seconds(mut self, max_duration_seconds: Option<u64>) -> Self {
        self.max_duration_seconds = max_duration_seconds;
        self
    }

    /// Sets the maximum display resolution constraint.
    #[must_use]
    pub const fn with_max_resolution(mut self, width: Option<u32>, height: Option<u32>) -> Self {
        self.max_resolution_width = width;
        self.max_resolution_height = height;
        self
    }

    /// Sets the allowed client IP ranges.
    #[must_use]
    pub fn with_allowed_ip_ranges<I: IntoIterator<Item = impl Into<String>>>(
        mut self,
        ranges: I,
    ) -> Self {
        self.allowed_ip_ranges = ranges.into_iter().map(Into::into).collect();
        self
    }

    /// Adds a single allowed client IP range.
    #[must_use]
    pub fn with_allowed_ip_range(mut self, range: impl Into<String>) -> Self {
        self.allowed_ip_ranges.push(range.into());
        self
    }

    /// Returns the maximum resolution tuple `(width, height)` if both dimensions are set.
    #[must_use]
    pub const fn max_resolution(&self) -> Option<(u32, u32)> {
        match (self.max_resolution_width, self.max_resolution_height) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        }
    }
}

/// Effective runtime restrictions derived for an authorized session capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRestrictions {
    /// Whether clipboard operations (read/write) are permitted in this session.
    pub clipboard_enabled: bool,

    /// Whether file transfers (upload/download) are permitted in this session.
    pub file_transfer_enabled: bool,

    /// Whether audio streaming (listen/send) is permitted in this session.
    pub audio_enabled: bool,

    /// Maximum active session duration in seconds before forced termination.
    pub max_duration_seconds: u64,

    /// Maximum video resolution constraint `(width, height)` in pixels, if constrained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution: Option<(u32, u32)>,
}

impl Default for SessionRestrictions {
    fn default() -> Self {
        Self {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: DEFAULT_MAX_DURATION_SECONDS,
            max_resolution: None,
        }
    }
}

impl SessionRestrictions {
    /// Creates a new `SessionRestrictions` instance.
    #[must_use]
    pub const fn new(
        clipboard_enabled: bool,
        file_transfer_enabled: bool,
        audio_enabled: bool,
        max_duration_seconds: u64,
        max_resolution: Option<(u32, u32)>,
    ) -> Self {
        Self {
            clipboard_enabled,
            file_transfer_enabled,
            audio_enabled,
            max_duration_seconds,
            max_resolution,
        }
    }

    /// Derives runtime `SessionRestrictions` from `PolicyConditions`, using a fallback default duration if unspecified.
    #[must_use]
    pub const fn from_conditions(
        conditions: &PolicyConditions,
        default_duration_seconds: u64,
    ) -> Self {
        let max_duration = match conditions.max_duration_seconds {
            Some(d) => d,
            None => default_duration_seconds,
        };
        Self {
            clipboard_enabled: conditions.clipboard_enabled,
            file_transfer_enabled: conditions.file_transfer_enabled,
            audio_enabled: conditions.audio_enabled,
            max_duration_seconds: max_duration,
            max_resolution: conditions.max_resolution(),
        }
    }
}

/// A role definition combining allowed actions, device label selectors, and ABAC conditions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    /// Unique name or identifier of the role.
    pub name: String,

    /// Set of actions permitted by this role.
    pub allowed_actions: ActionSet,

    /// Device label selectors specifying target devices this role applies to.
    /// If empty, matches all target devices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_label_selectors: Vec<DeviceLabelSelector>,

    /// Policy conditions and security constraints for this role.
    #[serde(default)]
    pub conditions: PolicyConditions,
}

impl Role {
    /// Creates a new `Role` with the given name and empty permissions/selectors.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            allowed_actions: ActionSet::new(),
            device_label_selectors: Vec::new(),
            conditions: PolicyConditions::default(),
        }
    }

    /// Sets the allowed actions for this role.
    #[must_use]
    pub fn with_actions(mut self, actions: impl Into<ActionSet>) -> Self {
        self.allowed_actions = actions.into();
        self
    }

    /// Adds a single action to the allowed action set.
    #[must_use]
    pub fn with_action(mut self, action: Action) -> Self {
        self.allowed_actions.insert(action);
        self
    }

    /// Adds a device label selector to this role.
    #[must_use]
    pub fn with_label_selector(mut self, selector: DeviceLabelSelector) -> Self {
        self.device_label_selectors.push(selector);
        self
    }

    /// Appends multiple device label selectors to this role.
    #[must_use]
    pub fn with_label_selectors<I: IntoIterator<Item = DeviceLabelSelector>>(
        mut self,
        selectors: I,
    ) -> Self {
        self.device_label_selectors.extend(selectors);
        self
    }

    /// Sets the policy conditions for this role.
    #[must_use]
    pub fn with_conditions(mut self, conditions: PolicyConditions) -> Self {
        self.conditions = conditions;
        self
    }

    /// Returns `true` if this role allows the specified action.
    #[must_use]
    pub fn allows_action(&self, action: Action) -> bool {
        self.allowed_actions.contains(action)
    }

    /// Returns `true` if this role applies to a target device with the given labels.
    ///
    /// If no device label selectors are configured on this role, it matches all devices.
    /// Otherwise, it returns `true` if any selector matches the device labels.
    #[must_use]
    pub fn matches_device(&self, labels: &HashMap<String, String>) -> bool {
        if self.device_label_selectors.is_empty() {
            return true;
        }
        self.device_label_selectors
            .iter()
            .any(|selector| selector.matches(labels))
    }

    /// Returns the role's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns a reference to the allowed action set.
    #[must_use]
    pub const fn allowed_actions(&self) -> &ActionSet {
        &self.allowed_actions
    }

    /// Returns a slice of device label selectors.
    #[must_use]
    pub fn device_label_selectors(&self) -> &[DeviceLabelSelector] {
        &self.device_label_selectors
    }

    /// Returns a reference to the policy conditions.
    #[must_use]
    pub const fn conditions(&self) -> &PolicyConditions {
        &self.conditions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_label_selector_exact_matching() {
        let selector = DeviceLabelSelector::exact("environment", "production");

        let mut labels = HashMap::new();
        labels.insert("environment".to_string(), "production".to_string());
        labels.insert("os".to_string(), "linux".to_string());

        // Exact match
        assert!(selector.matches(&labels));

        // Different value
        let mut dev_labels = HashMap::new();
        dev_labels.insert("environment".to_string(), "staging".to_string());
        assert!(!selector.matches(&dev_labels));

        // Missing key
        let empty_labels = HashMap::new();
        assert!(!selector.matches(&empty_labels));
    }

    #[test]
    fn test_device_label_selector_wildcard_matching() {
        let wildcard = DeviceLabelSelector::wildcard();
        let empty_labels = HashMap::new();
        assert!(wildcard.matches(&empty_labels));

        let mut labels = HashMap::new();
        labels.insert("os".to_string(), "windows".to_string());
        assert!(wildcard.matches(&labels));

        // Selector constructed with ("*", "*")
        let star_star = DeviceLabelSelector::new("*", "*");
        assert_eq!(star_star, DeviceLabelSelector::Wildcard);
        assert!(star_star.matches(&empty_labels));

        // Selector with key wildcard (any key has specified value)
        let key_star = DeviceLabelSelector::exact("*", "production");
        assert!(!key_star.matches(&empty_labels));
        let mut prod_labels = HashMap::new();
        prod_labels.insert("env".to_string(), "production".to_string());
        assert!(key_star.matches(&prod_labels));

        // Selector with value wildcard (specified key exists)
        let val_star = DeviceLabelSelector::exact("env", "*");
        assert!(!val_star.matches(&empty_labels));
        let mut staging_labels = HashMap::new();
        staging_labels.insert("env".to_string(), "staging".to_string());
        assert!(val_star.matches(&staging_labels));
    }

    #[test]
    fn test_device_label_selector_match_all_multiple_labels() {
        let mut selector_map = BTreeMap::new();
        selector_map.insert("environment".to_string(), "production".to_string());
        selector_map.insert("os".to_string(), "linux".to_string());

        let selector = DeviceLabelSelector::match_all(selector_map);

        // Matches when all required labels match
        let mut device_labels = HashMap::new();
        device_labels.insert("environment".to_string(), "production".to_string());
        device_labels.insert("os".to_string(), "linux".to_string());
        device_labels.insert("datacenter".to_string(), "us-east".to_string());
        assert!(selector.matches(&device_labels));

        // Fails when one label value is wrong
        let mut wrong_val = device_labels.clone();
        wrong_val.insert("os".to_string(), "windows".to_string());
        assert!(!selector.matches(&wrong_val));

        // Fails when one label is missing
        let mut missing_label = HashMap::new();
        missing_label.insert("environment".to_string(), "production".to_string());
        assert!(!selector.matches(&missing_label));

        // Empty MatchAll matches any device
        let empty_selector = DeviceLabelSelector::match_all(Vec::<(String, String)>::new());
        assert!(empty_selector.matches(&HashMap::new()));
    }

    #[test]
    fn test_role_device_matching() {
        // Role with no selectors matches everything
        let unconstrained_role = Role::new("admin");
        let empty_labels = HashMap::new();
        assert!(unconstrained_role.matches_device(&empty_labels));

        let mut labels = HashMap::new();
        labels.insert("environment".to_string(), "production".to_string());
        assert!(unconstrained_role.matches_device(&labels));

        // Role with single exact selector
        let prod_role = Role::new("prod-support")
            .with_label_selector(DeviceLabelSelector::exact("environment", "production"));
        assert!(prod_role.matches_device(&labels));

        let mut dev_labels = HashMap::new();
        dev_labels.insert("environment".to_string(), "development".to_string());
        assert!(!prod_role.matches_device(&dev_labels));

        // Role with multiple selectors (OR semantics)
        let multi_role = Role::new("multi-env")
            .with_label_selector(DeviceLabelSelector::exact("environment", "production"))
            .with_label_selector(DeviceLabelSelector::exact("environment", "staging"));
        assert!(multi_role.matches_device(&labels));
        let mut staging_labels = HashMap::new();
        staging_labels.insert("environment".to_string(), "staging".to_string());
        assert!(multi_role.matches_device(&staging_labels));
        assert!(!multi_role.matches_device(&dev_labels));

        // From conversions for selectors
        let conv_role = Role::new("conv").with_label_selector(("os", "windows").into());
        let mut win_labels = HashMap::new();
        win_labels.insert("os".to_string(), "windows".to_string());
        assert!(conv_role.matches_device(&win_labels));
    }

    #[test]
    fn test_role_action_permissions() {
        let role = Role::new("viewer")
            .with_action(Action::DesktopView)
            .with_action(Action::ClipboardRead);

        assert_eq!(role.name(), "viewer");
        assert!(role.allows_action(Action::DesktopView));
        assert!(role.allows_action(Action::ClipboardRead));
        assert!(!role.allows_action(Action::DesktopControl));
        assert!(!role.allows_action(Action::FileUpload));

        let full_role = Role::new("admin").with_actions(ActionSet::all());
        for action in Action::ALL {
            assert!(full_role.allows_action(action));
        }
    }

    #[test]
    fn test_policy_conditions_defaults_and_builder() {
        let default_cond = PolicyConditions::default();
        assert!(!default_cond.require_mfa);
        assert!(!default_cond.managed_client_device);
        assert!(default_cond.clipboard_enabled);
        assert!(default_cond.file_transfer_enabled);
        assert!(default_cond.audio_enabled);
        assert_eq!(default_cond.max_duration_seconds, None);
        assert_eq!(default_cond.max_resolution_width, None);
        assert_eq!(default_cond.max_resolution_height, None);
        assert!(default_cond.allowed_ip_ranges.is_empty());
        assert_eq!(default_cond.max_resolution(), None);

        let custom_cond = PolicyConditions::new()
            .with_require_mfa(true)
            .with_managed_client_device(true)
            .with_clipboard_enabled(false)
            .with_file_transfer_enabled(false)
            .with_audio_enabled(false)
            .with_max_duration_seconds(Some(1800))
            .with_max_resolution(Some(1920), Some(1080))
            .with_allowed_ip_range("10.0.0.0/8")
            .with_allowed_ip_ranges(["192.168.1.0/24"]);

        assert!(custom_cond.require_mfa);
        assert!(custom_cond.managed_client_device);
        assert!(!custom_cond.clipboard_enabled);
        assert!(!custom_cond.file_transfer_enabled);
        assert!(!custom_cond.audio_enabled);
        assert_eq!(custom_cond.max_duration_seconds, Some(1800));
        assert_eq!(custom_cond.max_resolution_width, Some(1920));
        assert_eq!(custom_cond.max_resolution_height, Some(1080));
        assert_eq!(custom_cond.max_resolution(), Some((1920, 1080)));
        assert_eq!(
            custom_cond.allowed_ip_ranges,
            vec!["192.168.1.0/24".to_string()]
        );
    }

    #[test]
    fn test_session_restrictions_creation_and_derivation() {
        let restrictions = SessionRestrictions::new(false, true, false, 7200, Some((1280, 720)));
        assert!(!restrictions.clipboard_enabled);
        assert!(restrictions.file_transfer_enabled);
        assert!(!restrictions.audio_enabled);
        assert_eq!(restrictions.max_duration_seconds, 7200);
        assert_eq!(restrictions.max_resolution, Some((1280, 720)));

        let default_restrictions = SessionRestrictions::default();
        assert!(default_restrictions.clipboard_enabled);
        assert!(default_restrictions.file_transfer_enabled);
        assert!(default_restrictions.audio_enabled);
        assert_eq!(
            default_restrictions.max_duration_seconds,
            DEFAULT_MAX_DURATION_SECONDS
        );
        assert_eq!(default_restrictions.max_resolution, None);

        // Derive from PolicyConditions without max duration
        let conditions = PolicyConditions::new()
            .with_clipboard_enabled(false)
            .with_max_resolution(Some(1920), Some(1080));
        let derived = SessionRestrictions::from_conditions(&conditions, 1800);
        assert!(!derived.clipboard_enabled);
        assert!(derived.file_transfer_enabled);
        assert_eq!(derived.max_duration_seconds, 1800);
        assert_eq!(derived.max_resolution, Some((1920, 1080)));

        // Derive from PolicyConditions with max duration
        let conditions_with_duration = PolicyConditions::new().with_max_duration_seconds(Some(300));
        let derived_with_dur =
            SessionRestrictions::from_conditions(&conditions_with_duration, 1800);
        assert_eq!(derived_with_dur.max_duration_seconds, 300);
    }

    #[test]
    fn test_serde_json_device_label_selector() {
        let wildcard = DeviceLabelSelector::Wildcard;
        let wildcard_json = serde_json::to_string(&wildcard).unwrap();
        assert_eq!(wildcard_json, "{\"type\":\"wildcard\"}");
        let de_wildcard: DeviceLabelSelector = serde_json::from_str(&wildcard_json).unwrap();
        assert_eq!(wildcard, de_wildcard);

        let exact = DeviceLabelSelector::exact("os", "windows");
        let exact_json = serde_json::to_string(&exact).unwrap();
        assert_eq!(
            exact_json,
            "{\"type\":\"exact\",\"key\":\"os\",\"value\":\"windows\"}"
        );
        let de_exact: DeviceLabelSelector = serde_json::from_str(&exact_json).unwrap();
        assert_eq!(exact, de_exact);

        let mut map = BTreeMap::new();
        map.insert("env".to_string(), "prod".to_string());
        let match_all = DeviceLabelSelector::match_all(map);
        let match_all_json = serde_json::to_string(&match_all).unwrap();
        assert_eq!(
            match_all_json,
            "{\"type\":\"match_all\",\"labels\":{\"env\":\"prod\"}}"
        );
        let de_match_all: DeviceLabelSelector = serde_json::from_str(&match_all_json).unwrap();
        assert_eq!(match_all, de_match_all);
    }

    #[test]
    fn test_serde_json_policy_conditions() {
        // Deserializing empty JSON object should yield defaults
        let empty_json = "{}";
        let de_default: PolicyConditions = serde_json::from_str(empty_json).unwrap();
        assert_eq!(de_default, PolicyConditions::default());

        let custom = PolicyConditions::new()
            .with_require_mfa(true)
            .with_managed_client_device(true)
            .with_clipboard_enabled(false)
            .with_max_duration_seconds(Some(1200))
            .with_allowed_ip_ranges(["10.0.0.0/8"]);

        let json = serde_json::to_string(&custom).unwrap();
        let de_custom: PolicyConditions = serde_json::from_str(&json).unwrap();
        assert_eq!(custom, de_custom);
    }

    #[test]
    fn test_serde_json_session_restrictions() {
        let restrictions = SessionRestrictions::new(false, false, true, 900, Some((1024, 768)));
        let json = serde_json::to_string(&restrictions).unwrap();
        let de: SessionRestrictions = serde_json::from_str(&json).unwrap();
        assert_eq!(restrictions, de);
    }

    #[test]
    fn test_serde_json_role_roundtrip() {
        let role = Role::new("prod-admin")
            .with_actions([Action::DesktopView, Action::DesktopControl])
            .with_label_selector(DeviceLabelSelector::exact("environment", "production"))
            .with_conditions(
                PolicyConditions::new()
                    .with_require_mfa(true)
                    .with_clipboard_enabled(false),
            );

        let json = serde_json::to_string(&role).unwrap();
        let de: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(role, de);

        // Deserializing minimal role JSON with defaults
        let minimal_json = "{\"name\":\"viewer\",\"allowed_actions\":[\"desktop.view\"]}";
        let de_minimal: Role = serde_json::from_str(minimal_json).unwrap();
        assert_eq!(de_minimal.name, "viewer");
        assert!(de_minimal.allows_action(Action::DesktopView));
        assert!(!de_minimal.allows_action(Action::DesktopControl));
        assert!(de_minimal.device_label_selectors.is_empty());
        assert_eq!(de_minimal.conditions, PolicyConditions::default());
    }
}
