//! Strongly typed first-class actions and action sets for the Nexus policy engine.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

/// Strongly typed first-class actions supported by the Nexus platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Action {
    #[serde(rename = "desktop.view")]
    DesktopView,
    #[serde(rename = "desktop.control")]
    DesktopControl,
    #[serde(rename = "clipboard.read")]
    ClipboardRead,
    #[serde(rename = "clipboard.write")]
    ClipboardWrite,
    #[serde(rename = "file.upload")]
    FileUpload,
    #[serde(rename = "file.download")]
    FileDownload,
    #[serde(rename = "audio.listen")]
    AudioListen,
    #[serde(rename = "audio.send")]
    AudioSend,
    #[serde(rename = "session.record")]
    SessionRecord,
    #[serde(rename = "session.request")]
    SessionRequest,
    #[serde(rename = "session.approve")]
    SessionApprove,
}

impl Action {
    /// Array containing all supported [`Action`] variants.
    pub const ALL: [Action; 11] = [
        Self::DesktopView,
        Self::DesktopControl,
        Self::ClipboardRead,
        Self::ClipboardWrite,
        Self::FileUpload,
        Self::FileDownload,
        Self::AudioListen,
        Self::AudioSend,
        Self::SessionRecord,
        Self::SessionRequest,
        Self::SessionApprove,
    ];

    /// Returns the canonical string representation of this action.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::DesktopView => "desktop.view",
            Self::DesktopControl => "desktop.control",
            Self::ClipboardRead => "clipboard.read",
            Self::ClipboardWrite => "clipboard.write",
            Self::FileUpload => "file.upload",
            Self::FileDownload => "file.download",
            Self::AudioListen => "audio.listen",
            Self::AudioSend => "audio.send",
            Self::SessionRecord => "session.record",
            Self::SessionRequest => "session.request",
            Self::SessionApprove => "session.approve",
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an action string fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown action: {0}")]
pub struct ActionParseError(pub String);

impl FromStr for Action {
    type Err = ActionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "desktop.view" => Ok(Self::DesktopView),
            "desktop.control" => Ok(Self::DesktopControl),
            "clipboard.read" => Ok(Self::ClipboardRead),
            "clipboard.write" => Ok(Self::ClipboardWrite),
            "file.upload" => Ok(Self::FileUpload),
            "file.download" => Ok(Self::FileDownload),
            "audio.listen" => Ok(Self::AudioListen),
            "audio.send" => Ok(Self::AudioSend),
            "session.record" => Ok(Self::SessionRecord),
            "session.request" => Ok(Self::SessionRequest),
            "session.approve" => Ok(Self::SessionApprove),
            _ => Err(ActionParseError(s.to_string())),
        }
    }
}

/// A set of distinct [`Action`]s, internally backed by a [`BTreeSet`] for deterministic ordering.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionSet {
    actions: BTreeSet<Action>,
}

impl ActionSet {
    /// Creates a new, empty [`ActionSet`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            actions: BTreeSet::new(),
        }
    }

    /// Creates an [`ActionSet`] containing all supported actions.
    #[must_use]
    pub fn all() -> Self {
        Self {
            actions: Action::ALL.into_iter().collect(),
        }
    }

    /// Inserts an action into the set.
    ///
    /// Returns `true` if the action was not previously present.
    pub fn insert(&mut self, action: Action) -> bool {
        self.actions.insert(action)
    }

    /// Removes an action from the set.
    ///
    /// Returns `true` if the action was present.
    pub fn remove(&mut self, action: &Action) -> bool {
        self.actions.remove(action)
    }

    /// Returns `true` if the set contains the given action.
    #[must_use]
    pub fn contains(&self, action: Action) -> bool {
        self.actions.contains(&action)
    }

    /// Returns `true` if `self` is a subset of `other`.
    #[must_use]
    pub fn is_subset(&self, other: &ActionSet) -> bool {
        self.actions.is_subset(&other.actions)
    }

    /// Returns `true` if `self` is a superset of `other`.
    #[must_use]
    pub fn is_superset(&self, other: &ActionSet) -> bool {
        self.actions.is_superset(&other.actions)
    }

    /// Returns the number of actions in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Returns `true` if the set contains no actions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Clears the set, removing all actions.
    pub fn clear(&mut self) {
        self.actions.clear();
    }

    /// Returns an iterator over the actions in this set in deterministic order.
    pub fn iter(&self) -> impl Iterator<Item = Action> + '_ {
        self.actions.iter().copied()
    }

    /// Returns the union of `self` and `other`.
    #[must_use]
    pub fn union(&self, other: &ActionSet) -> ActionSet {
        self.actions.union(&other.actions).copied().collect()
    }

    /// Returns the intersection of `self` and `other`.
    #[must_use]
    pub fn intersection(&self, other: &ActionSet) -> ActionSet {
        self.actions.intersection(&other.actions).copied().collect()
    }

    /// Returns the difference of `self` and `other` (`self - other`).
    #[must_use]
    pub fn difference(&self, other: &ActionSet) -> ActionSet {
        self.actions.difference(&other.actions).copied().collect()
    }
}

impl FromIterator<Action> for ActionSet {
    fn from_iter<T: IntoIterator<Item = Action>>(iter: T) -> Self {
        Self {
            actions: iter.into_iter().collect(),
        }
    }
}

impl Extend<Action> for ActionSet {
    fn extend<T: IntoIterator<Item = Action>>(&mut self, iter: T) {
        self.actions.extend(iter);
    }
}

impl IntoIterator for ActionSet {
    type Item = Action;
    type IntoIter = std::collections::btree_set::IntoIter<Action>;

    fn into_iter(self) -> Self::IntoIter {
        self.actions.into_iter()
    }
}

impl<'a> IntoIterator for &'a ActionSet {
    type Item = Action;
    type IntoIter = std::iter::Copied<std::collections::btree_set::Iter<'a, Action>>;

    fn into_iter(self) -> Self::IntoIter {
        self.actions.iter().copied()
    }
}

impl<const N: usize> From<[Action; N]> for ActionSet {
    fn from(arr: [Action; N]) -> Self {
        arr.into_iter().collect()
    }
}

impl From<&[Action]> for ActionSet {
    fn from(slice: &[Action]) -> Self {
        slice.iter().copied().collect()
    }
}

impl From<Vec<Action>> for ActionSet {
    fn from(vec: Vec<Action>) -> Self {
        vec.into_iter().collect()
    }
}

impl From<BTreeSet<Action>> for ActionSet {
    fn from(actions: BTreeSet<Action>) -> Self {
        Self { actions }
    }
}

impl From<ActionSet> for BTreeSet<Action> {
    fn from(set: ActionSet) -> Self {
        set.actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_as_str_and_display() {
        assert_eq!(Action::DesktopView.as_str(), "desktop.view");
        assert_eq!(Action::DesktopControl.as_str(), "desktop.control");
        assert_eq!(Action::ClipboardRead.as_str(), "clipboard.read");
        assert_eq!(Action::ClipboardWrite.as_str(), "clipboard.write");
        assert_eq!(Action::FileUpload.as_str(), "file.upload");
        assert_eq!(Action::FileDownload.as_str(), "file.download");
        assert_eq!(Action::AudioListen.as_str(), "audio.listen");
        assert_eq!(Action::AudioSend.as_str(), "audio.send");
        assert_eq!(Action::SessionRecord.as_str(), "session.record");
        assert_eq!(Action::SessionRequest.as_str(), "session.request");
        assert_eq!(Action::SessionApprove.as_str(), "session.approve");

        assert_eq!(format!("{}", Action::DesktopView), "desktop.view");
        assert_eq!(format!("{}", Action::DesktopControl), "desktop.control");
        assert_eq!(format!("{}", Action::ClipboardRead), "clipboard.read");
        assert_eq!(format!("{}", Action::ClipboardWrite), "clipboard.write");
        assert_eq!(format!("{}", Action::FileUpload), "file.upload");
        assert_eq!(format!("{}", Action::FileDownload), "file.download");
        assert_eq!(format!("{}", Action::AudioListen), "audio.listen");
        assert_eq!(format!("{}", Action::AudioSend), "audio.send");
        assert_eq!(format!("{}", Action::SessionRecord), "session.record");
        assert_eq!(format!("{}", Action::SessionRequest), "session.request");
        assert_eq!(format!("{}", Action::SessionApprove), "session.approve");
    }

    #[test]
    fn test_action_all_constant() {
        assert_eq!(Action::ALL.len(), 11);
        for action in Action::ALL {
            assert_eq!(action.as_str().parse::<Action>().unwrap(), action);
        }
    }

    #[test]
    fn test_action_from_str_and_parse_error() {
        assert_eq!(
            "desktop.view".parse::<Action>().unwrap(),
            Action::DesktopView
        );
        assert_eq!(
            "desktop.control".parse::<Action>().unwrap(),
            Action::DesktopControl
        );
        assert_eq!(
            "clipboard.read".parse::<Action>().unwrap(),
            Action::ClipboardRead
        );
        assert_eq!(
            "clipboard.write".parse::<Action>().unwrap(),
            Action::ClipboardWrite
        );
        assert_eq!("file.upload".parse::<Action>().unwrap(), Action::FileUpload);
        assert_eq!(
            "file.download".parse::<Action>().unwrap(),
            Action::FileDownload
        );
        assert_eq!(
            "audio.listen".parse::<Action>().unwrap(),
            Action::AudioListen
        );
        assert_eq!("audio.send".parse::<Action>().unwrap(), Action::AudioSend);
        assert_eq!(
            "session.record".parse::<Action>().unwrap(),
            Action::SessionRecord
        );
        assert_eq!(
            "session.request".parse::<Action>().unwrap(),
            Action::SessionRequest
        );
        assert_eq!(
            "session.approve".parse::<Action>().unwrap(),
            Action::SessionApprove
        );

        let err = "unknown.action".parse::<Action>().unwrap_err();
        assert_eq!(err, ActionParseError("unknown.action".to_string()));
        assert_eq!(format!("{err}"), "unknown action: unknown.action");
    }

    #[test]
    fn test_action_set_operations() {
        let mut set = ActionSet::new();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);

        assert!(set.insert(Action::DesktopView));
        assert!(!set.insert(Action::DesktopView));
        assert_eq!(set.len(), 1);
        assert!(!set.is_empty());
        assert!(set.contains(Action::DesktopView));
        assert!(!set.contains(Action::DesktopControl));

        set.insert(Action::DesktopControl);
        assert_eq!(set.len(), 2);
        assert!(set.contains(Action::DesktopControl));

        assert!(set.remove(&Action::DesktopView));
        assert!(!set.remove(&Action::DesktopView));
        assert_eq!(set.len(), 1);
        assert!(!set.contains(Action::DesktopView));

        set.clear();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn test_action_set_all() {
        let all = ActionSet::all();
        assert_eq!(all.len(), 11);
        for action in Action::ALL {
            assert!(all.contains(action));
        }
    }

    #[test]
    fn test_action_set_subset_and_superset() {
        let mut subset = ActionSet::new();
        subset.insert(Action::DesktopView);

        let mut superset = ActionSet::new();
        superset.insert(Action::DesktopView);
        superset.insert(Action::DesktopControl);

        assert!(subset.is_subset(&superset));
        assert!(!superset.is_subset(&subset));
        assert!(superset.is_superset(&subset));
        assert!(!subset.is_superset(&superset));
        assert!(subset.is_subset(&subset));
    }

    #[test]
    fn test_action_set_set_algebra() {
        let a: ActionSet = [Action::DesktopView, Action::DesktopControl].into();
        let b: ActionSet = [Action::DesktopControl, Action::ClipboardRead].into();

        let union = a.union(&b);
        assert_eq!(union.len(), 3);
        assert!(union.contains(Action::DesktopView));
        assert!(union.contains(Action::DesktopControl));
        assert!(union.contains(Action::ClipboardRead));

        let intersection = a.intersection(&b);
        assert_eq!(intersection.len(), 1);
        assert!(intersection.contains(Action::DesktopControl));

        let difference = a.difference(&b);
        assert_eq!(difference.len(), 1);
        assert!(difference.contains(Action::DesktopView));
    }

    #[test]
    fn test_action_set_iteration_and_collection() {
        let actions = vec![
            Action::DesktopView,
            Action::ClipboardRead,
            Action::FileUpload,
        ];
        let set: ActionSet = actions.clone().into_iter().collect();
        assert_eq!(set.len(), 3);
        assert!(set.contains(Action::DesktopView));
        assert!(set.contains(Action::ClipboardRead));
        assert!(set.contains(Action::FileUpload));

        let collected: Vec<Action> = set.iter().collect();
        assert_eq!(collected.len(), 3);

        let collected_ref: Vec<Action> = (&set).into_iter().collect();
        assert_eq!(collected_ref.len(), 3);

        let mut extended = ActionSet::new();
        extended.extend([Action::AudioListen, Action::AudioSend]);
        assert_eq!(extended.len(), 2);
        assert!(extended.contains(Action::AudioListen));
        assert!(extended.contains(Action::AudioSend));

        let from_slice: ActionSet = [Action::SessionRecord, Action::SessionApprove][..].into();
        assert_eq!(from_slice.len(), 2);

        let btree: BTreeSet<Action> = set.into();
        assert_eq!(btree.len(), 3);
        let from_btree: ActionSet = btree.into();
        assert_eq!(from_btree.len(), 3);
    }

    #[test]
    fn test_action_serde_serialization() {
        let action = Action::DesktopControl;
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, "\"desktop.control\"");

        let deserialized: Action = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, Action::DesktopControl);

        let mut set = ActionSet::new();
        set.insert(Action::DesktopView);
        set.insert(Action::ClipboardRead);

        let set_json = serde_json::to_string(&set).unwrap();
        assert_eq!(set_json, "[\"desktop.view\",\"clipboard.read\"]");
        let deserialized_set: ActionSet = serde_json::from_str(&set_json).unwrap();
        assert_eq!(set, deserialized_set);

        // Invalid JSON action
        let invalid = serde_json::from_str::<Action>("\"invalid.action\"");
        assert!(invalid.is_err());
    }
}
