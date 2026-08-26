//! Policy evaluation engine for the Nexus platform.
//!
//! Evaluates RBAC permissions, ABAC constraints (MFA, managed client devices, IP ranges),
//! target device label selectors, and ADR-015 concurrent control exclusivity.

use crate::action::{Action, ActionSet};
use crate::model::{Role, SessionRestrictions, DEFAULT_MAX_DURATION_SECONDS};
use nexus_common::id::{DeviceId, SessionId, UserId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

/// Helper function to check if an IP address matches a CIDR range string or exact IP string.
///
/// Supports IPv4 and IPv6 CIDRs (e.g. `"192.168.1.0/24"`, `"2001:db8::/32"`) and single IPs.
#[must_use]
pub fn ip_matches_range(ip: &IpAddr, range_str: &str) -> bool {
    let range_str = range_str.trim();
    if let Some((cidr_ip_str, prefix_str)) = range_str.split_once('/') {
        let prefix: u8 = match prefix_str.parse() {
            Ok(p) => p,
            Err(_) => return false,
        };
        match (ip, cidr_ip_str.parse::<IpAddr>()) {
            (IpAddr::V4(client_v4), Ok(IpAddr::V4(net_v4))) => {
                if prefix > 32 {
                    return false;
                }
                if prefix == 0 {
                    return true;
                }
                let mask = !0u32 << (32 - prefix);
                let client_u32 = u32::from(*client_v4);
                let net_u32 = u32::from(net_v4);
                (client_u32 & mask) == (net_u32 & mask)
            }
            (IpAddr::V6(client_v6), Ok(IpAddr::V6(net_v6))) => {
                if prefix > 128 {
                    return false;
                }
                if prefix == 0 {
                    return true;
                }
                let mask = !0u128 << (128 - prefix);
                let client_u128 = u128::from(*client_v6);
                let net_u128 = u128::from(net_v6);
                (client_u128 & mask) == (net_u128 & mask)
            }
            _ => false,
        }
    } else {
        match range_str.parse::<IpAddr>() {
            Ok(exact_ip) => *ip == exact_ip,
            Err(_) => false,
        }
    }
}

/// Subject context containing user identity, assigned roles, and authentication/device claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectContext {
    /// Unique identifier of the authenticated user.
    pub user_id: UserId,

    /// List of role names assigned to the subject.
    pub roles: Vec<String>,

    /// Whether the subject completed multi-factor authentication.
    pub mfa_authenticated: bool,

    /// Whether the client connecting device is an enterprise-managed device.
    pub client_device_managed: bool,

    /// Originating IP address of the client connection, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<IpAddr>,
}

impl SubjectContext {
    /// Creates a new `SubjectContext` with default unauthenticated/unmanaged claims.
    pub fn new(user_id: impl Into<UserId>) -> Self {
        Self {
            user_id: user_id.into(),
            roles: Vec::new(),
            mfa_authenticated: false,
            client_device_managed: false,
            client_ip: None,
        }
    }

    /// Adds a role to the subject's assigned roles.
    #[must_use]
    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.roles.push(role.into());
        self
    }

    /// Appends multiple roles to the subject's assigned roles.
    #[must_use]
    pub fn with_roles<I: IntoIterator<Item = impl Into<String>>>(mut self, roles: I) -> Self {
        self.roles.extend(roles.into_iter().map(Into::into));
        self
    }

    /// Sets whether the subject is MFA authenticated.
    #[must_use]
    pub const fn with_mfa(mut self, mfa_authenticated: bool) -> Self {
        self.mfa_authenticated = mfa_authenticated;
        self
    }

    /// Sets whether the subject's client device is managed.
    #[must_use]
    pub const fn with_client_device_managed(mut self, client_device_managed: bool) -> Self {
        self.client_device_managed = client_device_managed;
        self
    }

    /// Sets the subject's client IP address.
    #[must_use]
    pub const fn with_client_ip(mut self, client_ip: Option<IpAddr>) -> Self {
        self.client_ip = client_ip;
        self
    }
}

/// Summary of an existing session on a target device used for concurrency checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveSessionSummary {
    /// Unique session identifier.
    pub session_id: SessionId,

    /// Target device identifier.
    pub target_device_id: DeviceId,

    /// Set of actions granted to this session.
    pub granted_actions: ActionSet,

    /// Whether the session is currently active (connected or in reconnect grace window).
    pub is_active: bool,
}

impl ActiveSessionSummary {
    /// Creates a new `ActiveSessionSummary`.
    #[must_use]
    pub fn new(
        session_id: SessionId,
        target_device_id: DeviceId,
        granted_actions: impl Into<ActionSet>,
        is_active: bool,
    ) -> Self {
        Self {
            session_id,
            target_device_id,
            granted_actions: granted_actions.into(),
            is_active,
        }
    }
}

/// Resource context describing the target device, its labels, and current active sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceContext {
    /// Unique device identifier of the target machine.
    pub target_device_id: DeviceId,

    /// Key-value labels assigned to the target device.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub device_labels: HashMap<String, String>,

    /// Active sessions currently connected to or held on the target device.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_sessions_on_device: Vec<ActiveSessionSummary>,
}

impl ResourceContext {
    /// Creates a new `ResourceContext` for the given target device.
    #[must_use]
    pub fn new(target_device_id: DeviceId) -> Self {
        Self {
            target_device_id,
            device_labels: HashMap::new(),
            active_sessions_on_device: Vec::new(),
        }
    }

    /// Adds a key-value label to the resource context.
    #[must_use]
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.device_labels.insert(key.into(), value.into());
        self
    }

    /// Sets multiple labels on the resource context.
    #[must_use]
    pub fn with_labels<K: Into<String>, V: Into<String>, I: IntoIterator<Item = (K, V)>>(
        mut self,
        labels: I,
    ) -> Self {
        for (k, v) in labels {
            self.device_labels.insert(k.into(), v.into());
        }
        self
    }

    /// Adds an active session summary to the resource context.
    #[must_use]
    pub fn with_active_session(mut self, session: ActiveSessionSummary) -> Self {
        self.active_sessions_on_device.push(session);
        self
    }

    /// Appends multiple active session summaries to the resource context.
    #[must_use]
    pub fn with_active_sessions<I: IntoIterator<Item = ActiveSessionSummary>>(
        mut self,
        sessions: I,
    ) -> Self {
        self.active_sessions_on_device.extend(sessions);
        self
    }
}

/// Reason why an authorization or session request was denied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum DenialReason {
    /// No matching role was found for the subject among assigned roles.
    #[error("no matching role found for subject")]
    NoMatchingRole,

    /// The requested action is not allowed by any of the subject's assigned roles.
    #[error("action is not allowed by assigned roles")]
    ActionNotAllowed,

    /// The target device labels did not match the role's device selectors.
    #[error("target device does not match role label selectors")]
    DeviceNotMatched,

    /// Multi-factor authentication is required by the matching role policy.
    #[error("multi-factor authentication is required")]
    MfaRequired,

    /// An enterprise-managed client device is required by the matching role policy.
    #[error("managed client device is required")]
    ManagedClientDeviceRequired,

    /// The client IP address is not within the permitted IP ranges.
    #[error("client IP address is not allowed")]
    IpAddressNotAllowed,

    /// ADR-015 conflict: target device already has an active exclusive desktop control session.
    #[error("concurrent control conflict with active session {active_session_id}")]
    ConcurrentControlConflict {
        /// Session ID of the conflicting active session holding desktop.control.
        active_session_id: SessionId,
    },
}

/// Result of evaluating a policy authorization request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvaluationDecision {
    /// Access is permitted with effective granted actions and session restrictions.
    Allowed {
        /// Effective set of granted actions for the session.
        granted_actions: ActionSet,

        /// Derived session constraints and security restrictions.
        restrictions: SessionRestrictions,

        /// Name of the role (or roles) that authorized this request.
        matched_role: String,
    },

    /// Access is denied with an explicit reason.
    Denied {
        /// The specific reason access was denied.
        reason: DenialReason,
    },
}

impl EvaluationDecision {
    /// Returns `true` if the evaluation resulted in [`EvaluationDecision::Allowed`].
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    /// Returns `true` if the evaluation resulted in [`EvaluationDecision::Denied`].
    #[must_use]
    pub const fn is_denied(&self) -> bool {
        matches!(self, Self::Denied { .. })
    }

    /// Returns the denial reason if denied, or `None` if allowed.
    #[must_use]
    pub const fn denial_reason(&self) -> Option<&DenialReason> {
        match self {
            Self::Denied { reason } => Some(reason),
            Self::Allowed { .. } => None,
        }
    }

    /// Returns the granted actions if allowed, or `None` if denied.
    #[must_use]
    pub const fn granted_actions(&self) -> Option<&ActionSet> {
        match self {
            Self::Allowed {
                granted_actions, ..
            } => Some(granted_actions),
            Self::Denied { .. } => None,
        }
    }

    /// Returns the restrictions if allowed, or `None` if denied.
    #[must_use]
    pub const fn restrictions(&self) -> Option<&SessionRestrictions> {
        match self {
            Self::Allowed { restrictions, .. } => Some(restrictions),
            Self::Denied { .. } => None,
        }
    }

    /// Returns the matched role if allowed, or `None` if denied.
    #[must_use]
    pub fn matched_role(&self) -> Option<&str> {
        match self {
            Self::Allowed { matched_role, .. } => Some(matched_role),
            Self::Denied { .. } => None,
        }
    }
}

/// Policy evaluation engine implementing RBAC, ABAC, device label matching, and ADR-015 exclusivity.
#[derive(Debug, Clone, Default)]
pub struct PolicyEngine {
    roles: HashMap<String, Role>,
}

impl PolicyEngine {
    /// Creates a new policy engine pre-populated with the provided roles.
    #[must_use]
    pub fn new(roles: Vec<Role>) -> Self {
        let mut map = HashMap::with_capacity(roles.len());
        for role in roles {
            map.insert(role.name.clone(), role);
        }
        Self { roles: map }
    }

    /// Adds or updates a role in the policy engine.
    pub fn add_role(&mut self, role: Role) {
        self.roles.insert(role.name.clone(), role);
    }

    /// Builder method to add a role and return `self`.
    #[must_use]
    pub fn with_role(mut self, role: Role) -> Self {
        self.add_role(role);
        self
    }

    /// Returns a reference to a role by name, if it exists.
    #[must_use]
    pub fn get_role(&self, name: &str) -> Option<&Role> {
        self.roles.get(name)
    }

    /// Returns the number of roles configured in the engine.
    #[must_use]
    pub fn role_count(&self) -> usize {
        self.roles.len()
    }

    /// Helper to evaluate ABAC conditions for a role against a subject.
    fn evaluate_role_abac(role: &Role, subject: &SubjectContext) -> Result<(), DenialReason> {
        if role.conditions.require_mfa && !subject.mfa_authenticated {
            return Err(DenialReason::MfaRequired);
        }
        if role.conditions.managed_client_device && !subject.client_device_managed {
            return Err(DenialReason::ManagedClientDeviceRequired);
        }
        if !role.conditions.allowed_ip_ranges.is_empty() {
            match subject.client_ip {
                Some(ip) => {
                    let matches_any = role
                        .conditions
                        .allowed_ip_ranges
                        .iter()
                        .any(|range| ip_matches_range(&ip, range));
                    if !matches_any {
                        return Err(DenialReason::IpAddressNotAllowed);
                    }
                }
                None => {
                    return Err(DenialReason::IpAddressNotAllowed);
                }
            }
        }
        Ok(())
    }

    /// Evaluates an authorization request for a subject, target resource, and requested action.
    ///
    /// Evaluation steps:
    /// 1. **ADR-015 Exclusivity Check**: If `requested_action == Action::DesktopControl`, verify no other
    ///    active session on the device holds `Action::DesktopControl`.
    /// 2. **Role Lookup**: Look up all assigned roles from `subject.roles`.
    /// 3. **Action Verification**: Filter roles that permit `requested_action`.
    /// 4. **Device Label Matching**: Filter roles that match `resource.device_labels`.
    /// 5. **ABAC Evaluation**: Verify MFA, managed client device, and IP range constraints.
    /// 6. **Union & Restriction Derivation**: Compute union of granted actions and effective restrictions.
    #[must_use]
    pub fn evaluate(
        &self,
        subject: &SubjectContext,
        resource: &ResourceContext,
        requested_action: Action,
    ) -> EvaluationDecision {
        // Step 1: ADR-015 Concurrent Control Exclusivity Check
        if requested_action == Action::DesktopControl {
            for session in &resource.active_sessions_on_device {
                if session.is_active && session.granted_actions.contains(Action::DesktopControl) {
                    return EvaluationDecision::Denied {
                        reason: DenialReason::ConcurrentControlConflict {
                            active_session_id: session.session_id.clone(),
                        },
                    };
                }
            }
        }

        // Step 2: Role Lookup
        let assigned_roles: Vec<&Role> = subject
            .roles
            .iter()
            .filter_map(|role_name| self.roles.get(role_name))
            .collect();

        if assigned_roles.is_empty() {
            return EvaluationDecision::Denied {
                reason: DenialReason::NoMatchingRole,
            };
        }

        // Step 3: Action Verification
        let action_roles: Vec<&Role> = assigned_roles
            .into_iter()
            .filter(|role| role.allows_action(requested_action))
            .collect();

        if action_roles.is_empty() {
            return EvaluationDecision::Denied {
                reason: DenialReason::ActionNotAllowed,
            };
        }

        // Step 4: Device Label Matching
        let device_roles: Vec<&Role> = action_roles
            .into_iter()
            .filter(|role| role.matches_device(&resource.device_labels))
            .collect();

        if device_roles.is_empty() {
            return EvaluationDecision::Denied {
                reason: DenialReason::DeviceNotMatched,
            };
        }

        // Step 5: ABAC Evaluation
        let mut matching_roles = Vec::new();
        let mut first_abac_failure = None;

        for role in device_roles {
            match Self::evaluate_role_abac(role, subject) {
                Ok(()) => matching_roles.push(role),
                Err(reason) => {
                    if first_abac_failure.is_none() {
                        first_abac_failure = Some(reason);
                    }
                }
            }
        }

        if matching_roles.is_empty() {
            return EvaluationDecision::Denied {
                reason: first_abac_failure.unwrap_or(DenialReason::NoMatchingRole),
            };
        }

        // Step 6: Single or merged matching roles calculation
        if matching_roles.len() == 1 {
            let role = matching_roles[0];
            let granted_actions = role.allowed_actions.clone();
            let restrictions = SessionRestrictions::from_conditions(
                &role.conditions,
                DEFAULT_MAX_DURATION_SECONDS,
            );
            EvaluationDecision::Allowed {
                granted_actions,
                restrictions,
                matched_role: role.name.clone(),
            }
        } else {
            // Union of granted actions
            let mut union_actions = ActionSet::new();
            for role in &matching_roles {
                union_actions = union_actions.union(&role.allowed_actions);
            }

            // Effective restrictions (most permissive / union semantics)
            let clipboard_enabled = matching_roles
                .iter()
                .any(|r| r.conditions.clipboard_enabled);
            let file_transfer_enabled = matching_roles
                .iter()
                .any(|r| r.conditions.file_transfer_enabled);
            let audio_enabled = matching_roles.iter().any(|r| r.conditions.audio_enabled);

            let max_duration_seconds = matching_roles
                .iter()
                .map(|r| {
                    r.conditions
                        .max_duration_seconds
                        .unwrap_or(DEFAULT_MAX_DURATION_SECONDS)
                })
                .max()
                .unwrap_or(DEFAULT_MAX_DURATION_SECONDS);

            let has_unrestricted_res = matching_roles
                .iter()
                .any(|r| r.conditions.max_resolution().is_none());
            let max_resolution = if has_unrestricted_res {
                None
            } else {
                let max_w = matching_roles
                    .iter()
                    .filter_map(|r| r.conditions.max_resolution_width)
                    .max();
                let max_h = matching_roles
                    .iter()
                    .filter_map(|r| r.conditions.max_resolution_height)
                    .max();
                match (max_w, max_h) {
                    (Some(w), Some(h)) => Some((w, h)),
                    _ => None,
                }
            };

            let restrictions = SessionRestrictions::new(
                clipboard_enabled,
                file_transfer_enabled,
                audio_enabled,
                max_duration_seconds,
                max_resolution,
            );

            let matched_role = matching_roles
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(",");

            EvaluationDecision::Allowed {
                granted_actions: union_actions,
                restrictions,
                matched_role,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeviceLabelSelector, PolicyConditions};
    use std::net::Ipv4Addr;

    #[test]
    fn test_happy_path_rbac_desktop_control_and_view() {
        let admin_role = Role::new("admin")
            .with_action(Action::DesktopView)
            .with_action(Action::DesktopControl)
            .with_action(Action::ClipboardRead)
            .with_action(Action::ClipboardWrite);

        let engine = PolicyEngine::new(vec![admin_role]);

        let subject = SubjectContext::new(UserId::new("usr-alice").unwrap()).with_role("admin");
        let resource = ResourceContext::new(DeviceId::new("dev-prod-1").unwrap());

        // Evaluate DesktopControl
        let decision = engine.evaluate(&subject, &resource, Action::DesktopControl);
        assert!(decision.is_allowed());
        assert!(!decision.is_denied());
        assert_eq!(decision.denial_reason(), None);
        assert_eq!(decision.matched_role(), Some("admin"));

        let granted = decision.granted_actions().unwrap();
        assert!(granted.contains(Action::DesktopControl));
        assert!(granted.contains(Action::DesktopView));
        assert!(granted.contains(Action::ClipboardRead));
        assert!(granted.contains(Action::ClipboardWrite));
        assert!(!granted.contains(Action::FileUpload));

        let restrictions = decision.restrictions().unwrap();
        assert!(restrictions.clipboard_enabled);
        assert_eq!(
            restrictions.max_duration_seconds,
            DEFAULT_MAX_DURATION_SECONDS
        );

        // Evaluate DesktopView
        let view_decision = engine.evaluate(&subject, &resource, Action::DesktopView);
        assert!(view_decision.is_allowed());
    }

    #[test]
    fn test_adr015_concurrent_control_exclusivity() {
        let admin_role = Role::new("admin")
            .with_action(Action::DesktopView)
            .with_action(Action::DesktopControl);
        let engine = PolicyEngine::new(vec![admin_role]);

        let target_dev = DeviceId::new("dev-target").unwrap();
        let existing_session_id = SessionId::new("sess-active-control").unwrap();

        let active_control_session = ActiveSessionSummary::new(
            existing_session_id.clone(),
            target_dev.clone(),
            [Action::DesktopView, Action::DesktopControl],
            true, // is_active: true
        );

        let resource =
            ResourceContext::new(target_dev.clone()).with_active_session(active_control_session);

        let subject = SubjectContext::new(UserId::new("usr-bob").unwrap()).with_role("admin");

        // 1. Request DesktopControl when another active session already holds DesktopControl -> DENIED
        let control_decision = engine.evaluate(&subject, &resource, Action::DesktopControl);
        assert!(control_decision.is_denied());
        assert!(!control_decision.is_allowed());
        assert_eq!(
            control_decision.denial_reason(),
            Some(&DenialReason::ConcurrentControlConflict {
                active_session_id: existing_session_id
            })
        );

        // 2. Request DesktopView concurrently on the same device -> ALLOWED (multi-viewer fan-out per ADR-015)
        let view_decision = engine.evaluate(&subject, &resource, Action::DesktopView);
        assert!(view_decision.is_allowed());
        assert_eq!(view_decision.matched_role(), Some("admin"));
    }

    #[test]
    fn test_inactive_ended_session_does_not_block_desktop_control() {
        let admin_role = Role::new("admin").with_action(Action::DesktopControl);
        let engine = PolicyEngine::new(vec![admin_role]);

        let target_dev = DeviceId::new("dev-target").unwrap();
        let ended_session_id = SessionId::new("sess-ended").unwrap();

        // Inactive/Ended session holding DesktopControl
        let ended_session = ActiveSessionSummary::new(
            ended_session_id,
            target_dev.clone(),
            [Action::DesktopControl],
            false, // is_active: false
        );

        let resource = ResourceContext::new(target_dev).with_active_session(ended_session);

        let subject = SubjectContext::new(UserId::new("usr-charlie").unwrap()).with_role("admin");

        // Request DesktopControl -> Should be ALLOWED because previous session is inactive
        let decision = engine.evaluate(&subject, &resource, Action::DesktopControl);
        assert!(decision.is_allowed());
    }

    #[test]
    fn test_abac_mfa_requirement() {
        let sensitive_role = Role::new("prod-ops")
            .with_action(Action::DesktopControl)
            .with_conditions(PolicyConditions::new().with_require_mfa(true));

        let engine = PolicyEngine::new(vec![sensitive_role]);
        let resource = ResourceContext::new(DeviceId::new("dev-1").unwrap());

        // Without MFA -> Denied with MfaRequired
        let subject_no_mfa = SubjectContext::new(UserId::new("usr-1").unwrap())
            .with_role("prod-ops")
            .with_mfa(false);
        let decision_no_mfa = engine.evaluate(&subject_no_mfa, &resource, Action::DesktopControl);
        assert_eq!(
            decision_no_mfa.denial_reason(),
            Some(&DenialReason::MfaRequired)
        );

        // With MFA -> Allowed
        let subject_with_mfa = SubjectContext::new(UserId::new("usr-1").unwrap())
            .with_role("prod-ops")
            .with_mfa(true);
        let decision_with_mfa =
            engine.evaluate(&subject_with_mfa, &resource, Action::DesktopControl);
        assert!(decision_with_mfa.is_allowed());
    }

    #[test]
    fn test_abac_managed_client_device_requirement() {
        let corporate_role = Role::new("corp-access")
            .with_action(Action::DesktopView)
            .with_conditions(PolicyConditions::new().with_managed_client_device(true));

        let engine = PolicyEngine::new(vec![corporate_role]);
        let resource = ResourceContext::new(DeviceId::new("dev-1").unwrap());

        // Unmanaged client device -> Denied
        let unmanaged_subject = SubjectContext::new(UserId::new("usr-1").unwrap())
            .with_role("corp-access")
            .with_client_device_managed(false);
        let decision = engine.evaluate(&unmanaged_subject, &resource, Action::DesktopView);
        assert_eq!(
            decision.denial_reason(),
            Some(&DenialReason::ManagedClientDeviceRequired)
        );

        // Managed client device -> Allowed
        let managed_subject = SubjectContext::new(UserId::new("usr-1").unwrap())
            .with_role("corp-access")
            .with_client_device_managed(true);
        let decision_ok = engine.evaluate(&managed_subject, &resource, Action::DesktopView);
        assert!(decision_ok.is_allowed());
    }

    #[test]
    fn test_abac_ip_range_restrictions() {
        let net_role = Role::new("office-only")
            .with_action(Action::DesktopView)
            .with_conditions(
                PolicyConditions::new()
                    .with_allowed_ip_range("192.168.1.0/24")
                    .with_allowed_ip_range("10.0.0.1"),
            );

        let engine = PolicyEngine::new(vec![net_role]);
        let resource = ResourceContext::new(DeviceId::new("dev-1").unwrap());

        // Matching CIDR (192.168.1.42) -> Allowed
        let subject_cidr = SubjectContext::new(UserId::new("usr-1").unwrap())
            .with_role("office-only")
            .with_client_ip(Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42))));
        assert!(engine
            .evaluate(&subject_cidr, &resource, Action::DesktopView)
            .is_allowed());

        // Matching exact IP (10.0.0.1) -> Allowed
        let subject_exact = SubjectContext::new(UserId::new("usr-1").unwrap())
            .with_role("office-only")
            .with_client_ip(Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(engine
            .evaluate(&subject_exact, &resource, Action::DesktopView)
            .is_allowed());

        // Non-matching IP (192.168.2.1) -> Denied
        let subject_wrong_ip = SubjectContext::new(UserId::new("usr-1").unwrap())
            .with_role("office-only")
            .with_client_ip(Some(IpAddr::V4(Ipv4Addr::new(192, 168, 2, 1))));
        assert_eq!(
            engine
                .evaluate(&subject_wrong_ip, &resource, Action::DesktopView)
                .denial_reason(),
            Some(&DenialReason::IpAddressNotAllowed)
        );

        // No IP provided -> Denied
        let subject_no_ip =
            SubjectContext::new(UserId::new("usr-1").unwrap()).with_role("office-only");
        assert_eq!(
            engine
                .evaluate(&subject_no_ip, &resource, Action::DesktopView)
                .denial_reason(),
            Some(&DenialReason::IpAddressNotAllowed)
        );
    }

    #[test]
    fn test_device_label_mismatch_denies_request() {
        let prod_role = Role::new("prod-engineer")
            .with_action(Action::DesktopControl)
            .with_label_selector(DeviceLabelSelector::exact("environment", "production"));

        let engine = PolicyEngine::new(vec![prod_role]);

        let subject = SubjectContext::new(UserId::new("usr-1").unwrap()).with_role("prod-engineer");

        // Staging device -> Denied with DeviceNotMatched
        let staging_resource = ResourceContext::new(DeviceId::new("dev-staging-1").unwrap())
            .with_label("environment", "staging");
        let decision_staging = engine.evaluate(&subject, &staging_resource, Action::DesktopControl);
        assert_eq!(
            decision_staging.denial_reason(),
            Some(&DenialReason::DeviceNotMatched)
        );

        // Production device -> Allowed
        let prod_resource = ResourceContext::new(DeviceId::new("dev-prod-1").unwrap())
            .with_label("environment", "production");
        let decision_prod = engine.evaluate(&subject, &prod_resource, Action::DesktopControl);
        assert!(decision_prod.is_allowed());
    }

    #[test]
    fn test_no_matching_role_and_action_not_allowed() {
        let viewer_role = Role::new("viewer").with_action(Action::DesktopView);
        let engine = PolicyEngine::new(vec![viewer_role]);
        let resource = ResourceContext::new(DeviceId::new("dev-1").unwrap());

        // Unknown role -> NoMatchingRole
        let unknown_subject =
            SubjectContext::new(UserId::new("usr-1").unwrap()).with_role("nonexistent-role");
        assert_eq!(
            engine
                .evaluate(&unknown_subject, &resource, Action::DesktopView)
                .denial_reason(),
            Some(&DenialReason::NoMatchingRole)
        );

        // Empty roles -> NoMatchingRole
        let empty_subject = SubjectContext::new(UserId::new("usr-1").unwrap());
        assert_eq!(
            engine
                .evaluate(&empty_subject, &resource, Action::DesktopView)
                .denial_reason(),
            Some(&DenialReason::NoMatchingRole)
        );

        // Assigned role does not allow requested action -> ActionNotAllowed
        let viewer_subject = SubjectContext::new(UserId::new("usr-1").unwrap()).with_role("viewer");
        assert_eq!(
            engine
                .evaluate(&viewer_subject, &resource, Action::DesktopControl)
                .denial_reason(),
            Some(&DenialReason::ActionNotAllowed)
        );
    }

    #[test]
    fn test_multiple_roles_union_and_fallback() {
        let view_role = Role::new("viewer")
            .with_action(Action::DesktopView)
            .with_action(Action::ClipboardRead);

        let transfer_role = Role::new("file-operator")
            .with_action(Action::DesktopView)
            .with_action(Action::FileUpload)
            .with_action(Action::FileDownload);

        let engine = PolicyEngine::new(vec![view_role, transfer_role]);
        let resource = ResourceContext::new(DeviceId::new("dev-1").unwrap());

        let multi_subject = SubjectContext::new(UserId::new("usr-1").unwrap())
            .with_role("viewer")
            .with_role("file-operator");

        // Evaluating DesktopView matches both roles, granting union of actions
        let decision = engine.evaluate(&multi_subject, &resource, Action::DesktopView);
        assert!(decision.is_allowed());
        let granted = decision.granted_actions().unwrap();
        assert!(granted.contains(Action::DesktopView));
        assert!(granted.contains(Action::ClipboardRead));
        assert!(granted.contains(Action::FileUpload));
        assert!(granted.contains(Action::FileDownload));

        // Evaluating FileUpload matches only file-operator role
        let upload_decision = engine.evaluate(&multi_subject, &resource, Action::FileUpload);
        assert!(upload_decision.is_allowed());
        assert_eq!(upload_decision.matched_role(), Some("file-operator"));
    }

    #[test]
    fn test_serde_json_roundtrip() {
        let subject = SubjectContext::new(UserId::new("usr-1").unwrap())
            .with_role("admin")
            .with_mfa(true)
            .with_client_device_managed(true)
            .with_client_ip(Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));

        let subject_json = serde_json::to_string(&subject).unwrap();
        let de_subject: SubjectContext = serde_json::from_str(&subject_json).unwrap();
        assert_eq!(subject, de_subject);

        let session_sum = ActiveSessionSummary::new(
            SessionId::new("sess-1").unwrap(),
            DeviceId::new("dev-1").unwrap(),
            [Action::DesktopControl],
            true,
        );
        let session_json = serde_json::to_string(&session_sum).unwrap();
        let de_session: ActiveSessionSummary = serde_json::from_str(&session_json).unwrap();
        assert_eq!(session_sum, de_session);

        let resource = ResourceContext::new(DeviceId::new("dev-1").unwrap())
            .with_label("os", "linux")
            .with_active_session(session_sum);
        let res_json = serde_json::to_string(&resource).unwrap();
        let de_resource: ResourceContext = serde_json::from_str(&res_json).unwrap();
        assert_eq!(resource, de_resource);

        let denial = DenialReason::ConcurrentControlConflict {
            active_session_id: SessionId::new("sess-1").unwrap(),
        };
        let denial_json = serde_json::to_string(&denial).unwrap();
        let de_denial: DenialReason = serde_json::from_str(&denial_json).unwrap();
        assert_eq!(denial, de_denial);
    }
}
