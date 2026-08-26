//! ADR-017 Dynamic Policy Narrowing Validator.
//!
//! Validates that dynamic policy-update pushes to an active session only narrow
//! capabilities and restrictions, never escalating permissions or widening restrictions.

use crate::action::ActionSet;
use crate::model::SessionRestrictions;
use serde::{Deserialize, Serialize};

/// Error returned when a proposed dynamic policy update attempts to escalate permissions or widen restrictions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum NarrowingError {
    /// Attempted to grant actions not present in the original capability.
    #[error("action escalation attempted with unauthorized actions: {unauthorized_actions:?}")]
    ActionEscalationAttempted {
        /// The set of unauthorized actions that were not in the original action set.
        unauthorized_actions: ActionSet,
    },

    /// Attempted to widen a session restriction that was previously constrained.
    #[error("session restriction widened: {0}")]
    RestrictionWidened(String),
}

impl NarrowingError {
    /// Constructs an `ActionEscalationAttempted` error.
    #[must_use]
    pub fn action_escalation(unauthorized_actions: impl Into<ActionSet>) -> Self {
        Self::ActionEscalationAttempted {
            unauthorized_actions: unauthorized_actions.into(),
        }
    }

    /// Constructs a `RestrictionWidened` error.
    #[must_use]
    pub fn restriction_widened(reason: impl Into<String>) -> Self {
        Self::RestrictionWidened(reason.into())
    }
}

/// Validates that `proposed_actions` and `proposed_restrictions` represent a valid narrowing
/// (or equal subset) of `original_actions` and `original_restrictions`.
///
/// Under ADR-017:
/// 1. `proposed_actions` must be a subset of `original_actions`.
/// 2. Restrictions can only become more restrictive, never less:
///    - Cannot enable clipboard if originally disabled.
///    - Cannot enable file transfer if originally disabled.
///    - Cannot enable audio if originally disabled.
///    - Cannot extend `max_duration_seconds`.
///    - Cannot increase either width or height of `max_resolution` if a max resolution was set,
///      nor remove an existing `max_resolution` constraint.
///
/// Returns `Ok(proposed_restrictions.clone())` if valid, or a [`NarrowingError`] if invalid.
pub fn validate_policy_narrowing(
    original_actions: &ActionSet,
    proposed_actions: &ActionSet,
    original_restrictions: &SessionRestrictions,
    proposed_restrictions: &SessionRestrictions,
) -> Result<SessionRestrictions, NarrowingError> {
    if !proposed_actions.is_subset(original_actions) {
        let unauthorized_actions = proposed_actions.difference(original_actions);
        return Err(NarrowingError::ActionEscalationAttempted {
            unauthorized_actions,
        });
    }

    if !original_restrictions.clipboard_enabled && proposed_restrictions.clipboard_enabled {
        return Err(NarrowingError::RestrictionWidened(
            "cannot enable clipboard if originally disabled".to_string(),
        ));
    }

    if !original_restrictions.file_transfer_enabled && proposed_restrictions.file_transfer_enabled {
        return Err(NarrowingError::RestrictionWidened(
            "cannot enable file transfer if originally disabled".to_string(),
        ));
    }

    if !original_restrictions.audio_enabled && proposed_restrictions.audio_enabled {
        return Err(NarrowingError::RestrictionWidened(
            "cannot enable audio if originally disabled".to_string(),
        ));
    }

    if proposed_restrictions.max_duration_seconds > original_restrictions.max_duration_seconds {
        return Err(NarrowingError::RestrictionWidened(
            "cannot extend max session duration".to_string(),
        ));
    }

    if let Some((orig_w, orig_h)) = original_restrictions.max_resolution {
        match proposed_restrictions.max_resolution {
            None => {
                return Err(NarrowingError::RestrictionWidened(
                    "cannot remove max resolution constraint".to_string(),
                ));
            }
            Some((prop_w, prop_h)) => {
                if prop_w > orig_w || prop_h > orig_h {
                    return Err(NarrowingError::RestrictionWidened(
                        "cannot increase max resolution width or height".to_string(),
                    ));
                }
            }
        }
    }

    Ok(proposed_restrictions.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;

    #[test]
    fn test_allowed_narrowing_drop_actions() {
        let original_actions: ActionSet = [
            Action::DesktopView,
            Action::DesktopControl,
            Action::ClipboardRead,
            Action::ClipboardWrite,
        ]
        .into();

        let proposed_actions: ActionSet = [Action::DesktopView, Action::ClipboardRead].into();

        let original_restrictions = SessionRestrictions::default();
        let proposed_restrictions = SessionRestrictions::default();

        let result = validate_policy_narrowing(
            &original_actions,
            &proposed_actions,
            &original_restrictions,
            &proposed_restrictions,
        );

        assert_eq!(result, Ok(proposed_restrictions));
    }

    #[test]
    fn test_allowed_narrowing_identity() {
        let actions: ActionSet = [Action::DesktopView, Action::DesktopControl].into();
        let restrictions = SessionRestrictions::default();

        let result = validate_policy_narrowing(&actions, &actions, &restrictions, &restrictions);
        assert_eq!(result, Ok(restrictions));
    }

    #[test]
    fn test_allowed_narrowing_tighten_restrictions() {
        let actions: ActionSet = [Action::DesktopView].into();
        let original_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: None,
        };

        let proposed_restrictions = SessionRestrictions {
            clipboard_enabled: false,
            file_transfer_enabled: false,
            audio_enabled: false,
            max_duration_seconds: 1800,
            max_resolution: Some((1920, 1080)),
        };

        let result = validate_policy_narrowing(
            &actions,
            &actions,
            &original_restrictions,
            &proposed_restrictions,
        );

        assert_eq!(result, Ok(proposed_restrictions));
    }

    #[test]
    fn test_allowed_narrowing_lower_resolution() {
        let actions: ActionSet = [Action::DesktopView].into();
        let original_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: Some((1920, 1080)),
        };

        let proposed_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: Some((1280, 720)),
        };

        let result = validate_policy_narrowing(
            &actions,
            &actions,
            &original_restrictions,
            &proposed_restrictions,
        );

        assert_eq!(result, Ok(proposed_restrictions));
    }

    #[test]
    fn test_reject_action_escalation() {
        let original_actions: ActionSet = [Action::DesktopView].into();
        let proposed_actions: ActionSet = [Action::DesktopView, Action::DesktopControl].into();

        let restrictions = SessionRestrictions::default();

        let result = validate_policy_narrowing(
            &original_actions,
            &proposed_actions,
            &restrictions,
            &restrictions,
        );

        let mut expected_unauthorized = ActionSet::new();
        expected_unauthorized.insert(Action::DesktopControl);

        assert_eq!(
            result,
            Err(NarrowingError::ActionEscalationAttempted {
                unauthorized_actions: expected_unauthorized,
            })
        );
    }

    #[test]
    fn test_reject_multiple_action_escalations() {
        let original_actions: ActionSet = [Action::DesktopView].into();
        let proposed_actions: ActionSet = [
            Action::DesktopView,
            Action::DesktopControl,
            Action::FileUpload,
        ]
        .into();

        let restrictions = SessionRestrictions::default();

        let result = validate_policy_narrowing(
            &original_actions,
            &proposed_actions,
            &restrictions,
            &restrictions,
        );

        let mut expected_unauthorized = ActionSet::new();
        expected_unauthorized.insert(Action::DesktopControl);
        expected_unauthorized.insert(Action::FileUpload);

        assert_eq!(
            result,
            Err(NarrowingError::ActionEscalationAttempted {
                unauthorized_actions: expected_unauthorized,
            })
        );
    }

    #[test]
    fn test_reject_widen_clipboard() {
        let actions: ActionSet = [Action::DesktopView].into();
        let original_restrictions = SessionRestrictions {
            clipboard_enabled: false,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: None,
        };

        let proposed_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: None,
        };

        let result = validate_policy_narrowing(
            &actions,
            &actions,
            &original_restrictions,
            &proposed_restrictions,
        );

        assert_eq!(
            result,
            Err(NarrowingError::RestrictionWidened(
                "cannot enable clipboard if originally disabled".to_string()
            ))
        );
    }

    #[test]
    fn test_reject_widen_file_transfer() {
        let actions: ActionSet = [Action::DesktopView].into();
        let original_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: false,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: None,
        };

        let proposed_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: None,
        };

        let result = validate_policy_narrowing(
            &actions,
            &actions,
            &original_restrictions,
            &proposed_restrictions,
        );

        assert_eq!(
            result,
            Err(NarrowingError::RestrictionWidened(
                "cannot enable file transfer if originally disabled".to_string()
            ))
        );
    }

    #[test]
    fn test_reject_widen_audio() {
        let actions: ActionSet = [Action::DesktopView].into();
        let original_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: false,
            max_duration_seconds: 3600,
            max_resolution: None,
        };

        let proposed_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: None,
        };

        let result = validate_policy_narrowing(
            &actions,
            &actions,
            &original_restrictions,
            &proposed_restrictions,
        );

        assert_eq!(
            result,
            Err(NarrowingError::RestrictionWidened(
                "cannot enable audio if originally disabled".to_string()
            ))
        );
    }

    #[test]
    fn test_reject_extend_max_duration() {
        let actions: ActionSet = [Action::DesktopView].into();
        let original_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 1800,
            max_resolution: None,
        };

        let proposed_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: None,
        };

        let result = validate_policy_narrowing(
            &actions,
            &actions,
            &original_restrictions,
            &proposed_restrictions,
        );

        assert_eq!(
            result,
            Err(NarrowingError::RestrictionWidened(
                "cannot extend max session duration".to_string()
            ))
        );
    }

    #[test]
    fn test_reject_increase_max_resolution_width() {
        let actions: ActionSet = [Action::DesktopView].into();
        let original_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: Some((1280, 720)),
        };

        let proposed_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: Some((1920, 720)),
        };

        let result = validate_policy_narrowing(
            &actions,
            &actions,
            &original_restrictions,
            &proposed_restrictions,
        );

        assert_eq!(
            result,
            Err(NarrowingError::RestrictionWidened(
                "cannot increase max resolution width or height".to_string()
            ))
        );
    }

    #[test]
    fn test_reject_increase_max_resolution_height() {
        let actions: ActionSet = [Action::DesktopView].into();
        let original_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: Some((1280, 720)),
        };

        let proposed_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: Some((1280, 1080)),
        };

        let result = validate_policy_narrowing(
            &actions,
            &actions,
            &original_restrictions,
            &proposed_restrictions,
        );

        assert_eq!(
            result,
            Err(NarrowingError::RestrictionWidened(
                "cannot increase max resolution width or height".to_string()
            ))
        );
    }

    #[test]
    fn test_reject_remove_max_resolution_constraint() {
        let actions: ActionSet = [Action::DesktopView].into();
        let original_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: Some((1280, 720)),
        };

        let proposed_restrictions = SessionRestrictions {
            clipboard_enabled: true,
            file_transfer_enabled: true,
            audio_enabled: true,
            max_duration_seconds: 3600,
            max_resolution: None,
        };

        let result = validate_policy_narrowing(
            &actions,
            &actions,
            &original_restrictions,
            &proposed_restrictions,
        );

        assert_eq!(
            result,
            Err(NarrowingError::RestrictionWidened(
                "cannot remove max resolution constraint".to_string()
            ))
        );
    }

    #[test]
    fn test_serde_json_narrowing_error() {
        let err1 = NarrowingError::ActionEscalationAttempted {
            unauthorized_actions: [Action::DesktopControl].into(),
        };
        let json1 = serde_json::to_string(&err1).expect("serialize error");
        let de1: NarrowingError = serde_json::from_str(&json1).expect("deserialize error");
        assert_eq!(err1, de1);

        let err2 = NarrowingError::RestrictionWidened("cannot enable clipboard".to_string());
        let json2 = serde_json::to_string(&err2).expect("serialize error");
        let de2: NarrowingError = serde_json::from_str(&json2).expect("deserialize error");
        assert_eq!(err2, de2);
    }
}
