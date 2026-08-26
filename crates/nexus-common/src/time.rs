//! Time and clock primitives for the Nexus platform.
//!
//! Provides OS-independent timestamps and clock traits with mock capabilities
//! for deterministic testing without sleeping.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH};

/// Represents a point in time measured as seconds since the Unix epoch (1970-01-01 00:00:00 UTC).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct UnixTimestamp(pub u64);

impl UnixTimestamp {
    /// The Unix Epoch: 1970-01-01 00:00:00 UTC (0 seconds).
    pub const EPOCH: Self = Self(0);

    /// The minimum representable timestamp (0 seconds).
    pub const MIN: Self = Self(0);

    /// The maximum representable timestamp (`u64::MAX` seconds).
    pub const MAX: Self = Self(u64::MAX);

    /// Creates a new `UnixTimestamp` from seconds since the Unix epoch.
    #[inline]
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs)
    }

    /// Creates a new `UnixTimestamp` from milliseconds since the Unix epoch (floored to seconds).
    #[inline]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis / 1_000)
    }

    /// Creates a new `UnixTimestamp` from microseconds since the Unix epoch (floored to seconds).
    #[inline]
    pub const fn from_micros(micros: u64) -> Self {
        Self(micros / 1_000_000)
    }

    /// Returns the number of seconds since the Unix epoch.
    #[inline]
    pub const fn as_secs(&self) -> u64 {
        self.0
    }

    /// Returns the timestamp as milliseconds since the Unix epoch.
    ///
    /// Saturates at `u64::MAX` on arithmetic overflow.
    #[inline]
    pub const fn as_millis(&self) -> u64 {
        self.0.saturating_mul(1_000)
    }

    /// Returns the timestamp as microseconds since the Unix epoch.
    ///
    /// Uses `u128` to prevent overflow for any valid `u64` seconds.
    #[inline]
    pub const fn as_micros(&self) -> u128 {
        (self.0 as u128).saturating_mul(1_000_000)
    }

    /// Returns the timestamp as nanoseconds since the Unix epoch.
    ///
    /// Uses `u128` to prevent overflow for any valid `u64` seconds.
    #[inline]
    pub const fn as_nanos(&self) -> u128 {
        (self.0 as u128).saturating_mul(1_000_000_000)
    }

    /// Consumes the timestamp and returns the raw `u64` epoch seconds.
    #[inline]
    pub const fn into_inner(self) -> u64 {
        self.0
    }

    /// Saturating addition with a [`Duration`].
    ///
    /// Adds the duration's whole seconds to the timestamp, saturating at `u64::MAX`.
    #[inline]
    pub const fn saturating_add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration.as_secs()))
    }

    /// Saturating addition with seconds.
    ///
    /// Adds seconds to the timestamp, saturating at `u64::MAX`.
    #[inline]
    pub const fn saturating_add_secs(self, secs: u64) -> Self {
        Self(self.0.saturating_add(secs))
    }

    /// Saturating subtraction with a [`Duration`].
    ///
    /// Subtracts the duration's whole seconds from the timestamp, saturating at `0`.
    #[inline]
    pub const fn saturating_sub(self, duration: Duration) -> Self {
        Self(self.0.saturating_sub(duration.as_secs()))
    }

    /// Saturating subtraction with seconds.
    ///
    /// Subtracts seconds from the timestamp, saturating at `0`.
    #[inline]
    pub const fn saturating_sub_secs(self, secs: u64) -> Self {
        Self(self.0.saturating_sub(secs))
    }

    /// Computes the duration elapsed since an earlier timestamp.
    ///
    /// If `self < earlier`, returns `Duration::ZERO`.
    #[inline]
    pub const fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration::from_secs(self.0.saturating_sub(earlier.0))
    }

    /// Computes the duration elapsed since an earlier timestamp, returning `None` if `self < earlier`.
    #[inline]
    pub const fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        if self.0 >= earlier.0 {
            Some(Duration::from_secs(self.0 - earlier.0))
        } else {
            None
        }
    }

    /// Checked timestamp addition with a [`Duration`].
    ///
    /// Returns `None` on overflow.
    #[inline]
    pub const fn checked_add(self, duration: Duration) -> Option<Self> {
        match self.0.checked_add(duration.as_secs()) {
            Some(secs) => Some(Self(secs)),
            None => None,
        }
    }

    /// Checked timestamp subtraction with a [`Duration`].
    ///
    /// Returns `None` on underflow.
    #[inline]
    pub const fn checked_sub(self, duration: Duration) -> Option<Self> {
        match self.0.checked_sub(duration.as_secs()) {
            Some(secs) => Some(Self(secs)),
            None => None,
        }
    }
}

impl AsRef<u64> for UnixTimestamp {
    #[inline]
    fn as_ref(&self) -> &u64 {
        &self.0
    }
}

impl From<u64> for UnixTimestamp {
    #[inline]
    fn from(secs: u64) -> Self {
        Self(secs)
    }
}

impl From<UnixTimestamp> for u64 {
    #[inline]
    fn from(ts: UnixTimestamp) -> Self {
        ts.0
    }
}

impl fmt::Display for UnixTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for UnixTimestamp {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let secs = s.parse::<u64>()?;
        Ok(Self::from_secs(secs))
    }
}

impl Add<Duration> for UnixTimestamp {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Duration) -> Self::Output {
        Self(self.0 + rhs.as_secs())
    }
}

impl AddAssign<Duration> for UnixTimestamp {
    #[inline]
    fn add_assign(&mut self, rhs: Duration) {
        self.0 += rhs.as_secs();
    }
}

impl Sub<Duration> for UnixTimestamp {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Duration) -> Self::Output {
        Self(self.0 - rhs.as_secs())
    }
}

impl SubAssign<Duration> for UnixTimestamp {
    #[inline]
    fn sub_assign(&mut self, rhs: Duration) {
        self.0 -= rhs.as_secs();
    }
}

impl Sub<UnixTimestamp> for UnixTimestamp {
    type Output = Duration;

    #[inline]
    fn sub(self, rhs: UnixTimestamp) -> Self::Output {
        Duration::from_secs(self.0 - rhs.0)
    }
}

impl TryFrom<SystemTime> for UnixTimestamp {
    type Error = SystemTimeError;

    fn try_from(time: SystemTime) -> Result<Self, Self::Error> {
        let duration = time.duration_since(UNIX_EPOCH)?;
        Ok(Self::from_secs(duration.as_secs()))
    }
}

impl From<UnixTimestamp> for SystemTime {
    fn from(ts: UnixTimestamp) -> Self {
        UNIX_EPOCH + Duration::from_secs(ts.as_secs())
    }
}

/// Abstract clock source trait.
///
/// Enables dependency injection of mock clocks in tests for deterministic timing
/// without sleeping or OS clock side effects.
pub trait Clock: Send + Sync {
    /// Returns the current time as a [`UnixTimestamp`].
    fn now(&self) -> UnixTimestamp;

    /// Returns the current time in milliseconds since the Unix epoch.
    fn now_millis(&self) -> u64 {
        self.now().as_millis()
    }
}

impl<T: Clock + ?Sized> Clock for &T {
    fn now(&self) -> UnixTimestamp {
        (**self).now()
    }

    fn now_millis(&self) -> u64 {
        (**self).now_millis()
    }
}

impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now(&self) -> UnixTimestamp {
        (**self).now()
    }

    fn now_millis(&self) -> u64 {
        (**self).now_millis()
    }
}

impl<T: Clock + ?Sized> Clock for Box<T> {
    fn now(&self) -> UnixTimestamp {
        (**self).now()
    }

    fn now_millis(&self) -> u64 {
        (**self).now_millis()
    }
}

/// Clock implementation backed by the host system clock ([`SystemTime`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemClock;

impl SystemClock {
    /// Creates a new `SystemClock`.
    pub const fn new() -> Self {
        Self
    }
}

impl Clock for SystemClock {
    fn now(&self) -> UnixTimestamp {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => UnixTimestamp::from_secs(duration.as_secs()),
            Err(_) => UnixTimestamp::EPOCH,
        }
    }

    fn now_millis(&self) -> u64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_millis().min(u64::MAX as u128) as u64,
            Err(_) => 0,
        }
    }
}

/// Thread-safe mock clock for deterministic time control in testing.
#[derive(Debug, Clone)]
pub struct MockClock {
    now_millis: Arc<AtomicU64>,
}

impl MockClock {
    /// Creates a new `MockClock` initialized to the given timestamp.
    pub fn new(initial: UnixTimestamp) -> Self {
        Self {
            now_millis: Arc::new(AtomicU64::new(initial.as_millis())),
        }
    }

    /// Creates a new `MockClock` initialized with epoch seconds.
    pub fn from_secs(secs: u64) -> Self {
        Self::new(UnixTimestamp::from_secs(secs))
    }

    /// Creates a new `MockClock` initialized with epoch milliseconds.
    pub fn from_millis(millis: u64) -> Self {
        Self {
            now_millis: Arc::new(AtomicU64::new(millis)),
        }
    }

    /// Advances the mock clock by the specified [`Duration`].
    pub fn advance(&self, duration: Duration) {
        let delta = duration.as_millis().min(u64::MAX as u128) as u64;
        self.now_millis
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |cur| {
                Some(cur.saturating_add(delta))
            })
            .ok();
    }

    /// Advances the mock clock by the specified seconds.
    pub fn advance_secs(&self, secs: u64) {
        self.advance(Duration::from_secs(secs));
    }

    /// Advances the mock clock by the specified milliseconds.
    pub fn advance_millis(&self, millis: u64) {
        self.advance(Duration::from_millis(millis));
    }

    /// Sets the mock clock to the specified timestamp.
    pub fn set(&self, timestamp: UnixTimestamp) {
        self.now_millis
            .store(timestamp.as_millis(), Ordering::SeqCst);
    }

    /// Sets the mock clock to the specified epoch seconds.
    pub fn set_secs(&self, secs: u64) {
        self.set(UnixTimestamp::from_secs(secs));
    }

    /// Sets the mock clock to the specified epoch milliseconds.
    pub fn set_millis(&self, millis: u64) {
        self.now_millis.store(millis, Ordering::SeqCst);
    }
}

impl Default for MockClock {
    fn default() -> Self {
        Self::new(UnixTimestamp::EPOCH)
    }
}

impl Clock for MockClock {
    fn now(&self) -> UnixTimestamp {
        let millis = self.now_millis.load(Ordering::SeqCst);
        UnixTimestamp::from_millis(millis)
    }

    fn now_millis(&self) -> u64 {
        self.now_millis.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashSet};
    use std::thread;

    #[test]
    fn test_unix_timestamp_creation_and_accessors() {
        let ts = UnixTimestamp::from_secs(1_700_000_000);
        assert_eq!(ts.as_secs(), 1_700_000_000);
        assert_eq!(ts.as_millis(), 1_700_000_000_000);
        assert_eq!(ts.as_micros(), 1_700_000_000_000_000);
        assert_eq!(ts.as_nanos(), 1_700_000_000_000_000_000);
        assert_eq!(ts.into_inner(), 1_700_000_000);
        assert_eq!(*ts.as_ref(), 1_700_000_000);

        let from_ms = UnixTimestamp::from_millis(1_700_000_000_500);
        assert_eq!(from_ms.as_secs(), 1_700_000_000);

        let from_us = UnixTimestamp::from_micros(1_700_000_000_500_000);
        assert_eq!(from_us.as_secs(), 1_700_000_000);

        assert_eq!(UnixTimestamp::EPOCH.as_secs(), 0);
        assert_eq!(UnixTimestamp::MIN.as_secs(), 0);
        assert_eq!(UnixTimestamp::MAX.as_secs(), u64::MAX);
    }

    #[test]
    fn test_unix_timestamp_math() {
        let base = UnixTimestamp::from_secs(1_000);

        // Saturating add
        assert_eq!(
            base.saturating_add(Duration::from_secs(50)),
            UnixTimestamp::from_secs(1_050)
        );
        assert_eq!(
            base.saturating_add_secs(50),
            UnixTimestamp::from_secs(1_050)
        );
        assert_eq!(
            UnixTimestamp::MAX.saturating_add(Duration::from_secs(100)),
            UnixTimestamp::MAX
        );
        assert_eq!(
            UnixTimestamp::MAX.saturating_add_secs(100),
            UnixTimestamp::MAX
        );

        // Saturating sub
        assert_eq!(
            base.saturating_sub(Duration::from_secs(50)),
            UnixTimestamp::from_secs(950)
        );
        assert_eq!(base.saturating_sub_secs(50), UnixTimestamp::from_secs(950));
        assert_eq!(
            base.saturating_sub(Duration::from_secs(2_000)),
            UnixTimestamp::EPOCH
        );
        assert_eq!(base.saturating_sub_secs(2_000), UnixTimestamp::EPOCH);

        // Duration since
        let t1 = UnixTimestamp::from_secs(100);
        let t2 = UnixTimestamp::from_secs(150);
        assert_eq!(t2.saturating_duration_since(t1), Duration::from_secs(50));
        assert_eq!(t1.saturating_duration_since(t2), Duration::ZERO);

        assert_eq!(t2.checked_duration_since(t1), Some(Duration::from_secs(50)));
        assert_eq!(t1.checked_duration_since(t2), None);

        // Checked add and sub
        assert_eq!(
            base.checked_add(Duration::from_secs(50)),
            Some(UnixTimestamp::from_secs(1_050))
        );
        assert_eq!(UnixTimestamp::MAX.checked_add(Duration::from_secs(1)), None);

        assert_eq!(
            base.checked_sub(Duration::from_secs(50)),
            Some(UnixTimestamp::from_secs(950))
        );
        assert_eq!(base.checked_sub(Duration::from_secs(1_500)), None);
    }

    #[test]
    fn test_unix_timestamp_ops() {
        let mut t = UnixTimestamp::from_secs(500);

        let t2 = t + Duration::from_secs(100);
        assert_eq!(t2, UnixTimestamp::from_secs(600));

        let t3 = t2 - Duration::from_secs(200);
        assert_eq!(t3, UnixTimestamp::from_secs(400));

        t += Duration::from_secs(50);
        assert_eq!(t, UnixTimestamp::from_secs(550));

        t -= Duration::from_secs(100);
        assert_eq!(t, UnixTimestamp::from_secs(450));

        let diff = t2 - t3;
        assert_eq!(diff, Duration::from_secs(200));
    }

    #[test]
    fn test_unix_timestamp_ordering_and_equality() {
        let t1 = UnixTimestamp::from_secs(10);
        let t2 = UnixTimestamp::from_secs(20);
        let t3 = UnixTimestamp::from_secs(30);

        assert!(t1 < t2);
        assert!(t2 > t1);
        assert_eq!(t1, UnixTimestamp::from_secs(10));
        assert_ne!(t1, t2);

        let mut set = HashSet::new();
        set.insert(t1);
        assert!(set.contains(&UnixTimestamp::from_secs(10)));
        assert!(!set.contains(&t2));

        let mut btree = BTreeSet::new();
        btree.insert(t2);
        btree.insert(t1);
        btree.insert(t3);

        let ordered: Vec<UnixTimestamp> = btree.into_iter().collect();
        assert_eq!(ordered, vec![t1, t2, t3]);
    }

    #[test]
    fn test_unix_timestamp_display_and_from_str() {
        let ts = UnixTimestamp::from_secs(1_700_000_000);
        assert_eq!(ts.to_string(), "1700000000");

        let parsed: UnixTimestamp = "1700000000".parse().unwrap();
        assert_eq!(parsed, ts);

        let parse_err = "not-a-number".parse::<UnixTimestamp>();
        assert!(parse_err.is_err());
    }

    #[test]
    fn test_unix_timestamp_from_and_into_u64() {
        let raw = 123456789u64;
        let ts = UnixTimestamp::from(raw);
        assert_eq!(ts.as_secs(), raw);

        let back: u64 = ts.into();
        assert_eq!(back, raw);
    }

    #[test]
    fn test_unix_timestamp_system_time_conversion() {
        let sys_time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let ts = UnixTimestamp::try_from(sys_time).expect("valid unix time");
        assert_eq!(ts.as_secs(), 1_700_000_000);

        let back: SystemTime = ts.into();
        assert_eq!(back, sys_time);

        // Before epoch error handling
        let before_epoch = UNIX_EPOCH - Duration::from_secs(100);
        let err = UnixTimestamp::try_from(before_epoch);
        assert!(err.is_err());
    }

    #[test]
    fn test_unix_timestamp_serde_json() {
        let ts = UnixTimestamp::from_secs(1_700_000_000);
        let json_str = serde_json::to_string(&ts).unwrap();
        assert_eq!(json_str, "1700000000");

        let deserialized: UnixTimestamp = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized, ts);

        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Container {
            created_at: UnixTimestamp,
        }

        let container = Container { created_at: ts };
        let c_json = serde_json::to_string(&container).unwrap();
        assert_eq!(c_json, "{\"created_at\":1700000000}");

        let c_de: Container = serde_json::from_str(&c_json).unwrap();
        assert_eq!(c_de, container);
    }

    #[test]
    fn test_system_clock() {
        let clock = SystemClock::new();
        let default_clock = SystemClock;
        assert_eq!(clock, default_clock);

        let now = clock.now();
        // Since year 2024 is > 1.7 billion epoch seconds
        assert!(now.as_secs() > 1_700_000_000);

        let millis = clock.now_millis();
        assert!(millis >= now.as_millis());
    }

    #[test]
    fn test_mock_clock_initial_and_set() {
        let clock = MockClock::new(UnixTimestamp::from_secs(100));
        assert_eq!(clock.now(), UnixTimestamp::from_secs(100));
        assert_eq!(clock.now_millis(), 100_000);

        clock.set(UnixTimestamp::from_secs(200));
        assert_eq!(clock.now(), UnixTimestamp::from_secs(200));
        assert_eq!(clock.now_millis(), 200_000);

        clock.set_secs(300);
        assert_eq!(clock.now(), UnixTimestamp::from_secs(300));
        assert_eq!(clock.now_millis(), 300_000);

        clock.set_millis(350_500);
        assert_eq!(clock.now(), UnixTimestamp::from_secs(350));
        assert_eq!(clock.now_millis(), 350_500);

        let default_clock = MockClock::default();
        assert_eq!(default_clock.now(), UnixTimestamp::EPOCH);
        assert_eq!(default_clock.now_millis(), 0);

        let from_secs = MockClock::from_secs(50);
        assert_eq!(from_secs.now(), UnixTimestamp::from_secs(50));

        let from_millis = MockClock::from_millis(50_250);
        assert_eq!(from_millis.now(), UnixTimestamp::from_secs(50));
        assert_eq!(from_millis.now_millis(), 50_250);
    }

    #[test]
    fn test_mock_clock_advance() {
        let clock = MockClock::from_secs(100);

        // Advance seconds via Duration
        clock.advance(Duration::from_secs(10));
        assert_eq!(clock.now(), UnixTimestamp::from_secs(110));
        assert_eq!(clock.now_millis(), 110_000);

        // Advance secs helper
        clock.advance_secs(5);
        assert_eq!(clock.now(), UnixTimestamp::from_secs(115));
        assert_eq!(clock.now_millis(), 115_000);

        // Advance millis (subsecond)
        clock.advance_millis(500);
        assert_eq!(clock.now(), UnixTimestamp::from_secs(115));
        assert_eq!(clock.now_millis(), 115_500);

        // Advance another 500ms to cross full second boundary
        clock.advance(Duration::from_millis(500));
        assert_eq!(clock.now(), UnixTimestamp::from_secs(116));
        assert_eq!(clock.now_millis(), 116_000);
    }

    #[test]
    fn test_mock_clock_multithreaded_advance_and_clone() {
        let clock = MockClock::from_secs(0);
        let num_threads = 8;
        let advances_per_thread = 100;
        let delta_ms = 10;

        let mut handles = Vec::new();
        for _ in 0..num_threads {
            let clock_clone = clock.clone();
            let handle = thread::spawn(move || {
                for _ in 0..advances_per_thread {
                    clock_clone.advance_millis(delta_ms);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let expected_millis = (num_threads as u64) * (advances_per_thread as u64) * delta_ms;
        assert_eq!(clock.now_millis(), expected_millis);
        assert_eq!(clock.now(), UnixTimestamp::from_millis(expected_millis));
    }

    #[test]
    fn test_clock_trait_objects_and_references() {
        fn read_now(clock: &impl Clock) -> UnixTimestamp {
            clock.now()
        }

        fn read_now_dyn(clock: &dyn Clock) -> UnixTimestamp {
            clock.now()
        }

        let mock = MockClock::from_secs(1234);
        assert_eq!(read_now(&mock), UnixTimestamp::from_secs(1234));
        assert_eq!(read_now_dyn(&mock), UnixTimestamp::from_secs(1234));

        let arc_clock: Arc<dyn Clock> = Arc::new(MockClock::from_secs(5678));
        assert_eq!(read_now(&arc_clock), UnixTimestamp::from_secs(5678));
        assert_eq!(arc_clock.now_millis(), 5_678_000);

        let box_clock: Box<dyn Clock> = Box::new(MockClock::from_secs(9999));
        assert_eq!(read_now(&box_clock), UnixTimestamp::from_secs(9999));
        assert_eq!(box_clock.now_millis(), 9_999_000);
    }
}
