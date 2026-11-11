// tests/backtester.rs
use backend::domain::backtester::group_by_day;
use chrono::{TimeZone, Utc};

#[test]
fn groups_matches_by_utc_day_in_order() {
    let d = |day, hour| Utc.with_ymd_and_hms(2025, 8, day, hour, 0, 0).unwrap();
    let matches = vec![(1, d(1, 12)), (2, d(1, 20)), (3, d(2, 15)), (4, d(4, 9))];
    assert_eq!(group_by_day(&matches), vec![vec![1, 2], vec![3], vec![4]]);
}
