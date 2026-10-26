//! Persistent-view/quota/accounting boundaries, independent of execution scheduling.

use chrono::Utc;
use rustack_stepfunctions_model::types::{HistoryEvent, HistoryEventType};

use super::*;
use crate::{ExecutionOutcome, Payload};

fn event() -> HistoryEvent {
    HistoryEvent::builder()
        .id(0)
        .r#type(HistoryEventType::PassStateEntered)
        .timestamp(Utc::now())
        .build()
}
fn journal(events: u32, bytes: usize) -> (Journal, Arc<HistoryCapacity>) {
    let limits = HistoryLimits::new(events, bytes).unwrap();
    let capacity = HistoryCapacity::new(256 * 1024 * 1024, limits).unwrap();
    let reserve = TerminalReserve::new(&capacity).unwrap();
    (Journal::new(limits, capacity.clone(), reserve), capacity)
}
#[test]
fn test_should_bound_history_configuration_and_page_arguments() {
    for (events, bytes) in [
        (1, 1024 * 1024),
        (25_001, 1024 * 1024),
        (2, 1024 * 1024 - 1),
        (2, 256 * 1024 * 1024 + 1),
    ] {
        assert!(HistoryLimits::new(events, bytes).is_err());
    }
    assert!(HistoryCapacity::new(7 * 1024 * 1024, HistoryLimits::default()).is_err());
    assert!(HistoryCapacity::new(8 * 1024 * 1024, HistoryLimits::default()).is_err());
    let history = ExecutionHistory::default();
    for (first, limit) in [(0, 1), (2, 1), (1, 0), (1, 1001)] {
        assert!(history.page(first, limit, true).is_err());
    }
}
#[test]
fn test_should_share_stable_pages_and_release_accounting_after_last_view() {
    let (mut journal, capacity) = journal(100, 1024 * 1024);
    for _ in 0..65 {
        journal.append(event(), false).unwrap();
    }
    let old = journal.view.clone();
    let charged = capacity.used();
    for _ in 0..2 {
        journal.append(event(), false).unwrap();
    }
    assert_eq!(old.len(), 65);
    assert_eq!(journal.view.len(), 67);
    let first = old.page(1, 64, true).unwrap();
    assert_eq!(first.events().len(), 64);
    assert_eq!(first.next_id(), Some(65));
    let second = old.page(first.next_id().unwrap(), 64, true).unwrap();
    assert_eq!(second.events().len(), 1);
    assert_eq!(second.events()[0].id, 65);
    assert_eq!(second.next_id(), None);
    drop(journal);
    assert_eq!(capacity.used(), charged - TERMINAL_RESERVE);
    drop(old);
    assert_eq!(capacity.used(), 0);
}
#[test]
fn test_should_keep_one_event_and_fixed_bytes_for_termination() {
    let (mut journal, capacity) = journal(2, 1024 * 1024);
    journal.append(event(), false).unwrap();
    let charged = capacity.used();
    assert!(matches!(
        journal.append(event(), false),
        Err(ExecutionError::Capacity)
    ));
    assert_eq!(capacity.used(), charged);
    let outcome = ExecutionOutcome::TimedOut;
    let terminal = events::terminal(&outcome, Utc::now()).unwrap();
    journal.append(terminal, true).unwrap();
    assert_eq!(journal.view.len(), 2);
    assert_eq!(capacity.used(), charged);
    drop(journal);
    assert_eq!(capacity.used(), 0);
}
#[test]
fn test_should_bound_complete_encoded_pages_without_truncating_individual_events() {
    let (mut journal, _) = journal(100, 32 * 1024 * 1024);
    let payload = Payload::from_value(&serde_json::json!("\0".repeat(40_000))).unwrap();
    for _ in 0..20 {
        journal
            .append(events::started(&payload, Utc::now()).unwrap(), false)
            .unwrap();
    }
    let page = journal.view.page(1, 1000, true).unwrap();
    assert!(!page.events().is_empty());
    assert!(page.events().len() < 20);
    assert!(page.next_id().is_some());
    assert!(serde_json::to_vec(page.events()).unwrap().len() < PAGE_BYTES);
    let omitted = journal.view.page(1, 1000, false).unwrap();
    assert_eq!(omitted.events().len(), 20);
    assert_eq!(omitted.next_id(), None);
    assert!(
        !serde_json::to_string(omitted.events())
            .unwrap()
            .contains("u0000")
    );
}
#[test]
fn test_should_reject_global_exhaustion_without_changing_journal_or_charges() {
    let limits = HistoryLimits::new(25_000, 1024 * 1024).unwrap();
    let capacity = HistoryCapacity::new(8 * 1024 * 1024, limits).unwrap();
    let reserve = TerminalReserve::new(&capacity).unwrap();
    let mut journal = Journal::new(limits, capacity.clone(), reserve);
    let held = capacity.reserve(capacity.limit - capacity.used()).unwrap();
    let before = capacity.used();
    assert!(matches!(
        journal.append(event(), false),
        Err(ExecutionError::Capacity)
    ));
    assert_eq!(journal.view.len(), 0);
    assert_eq!(capacity.used(), before);
    journal
        .append(
            events::terminal(&ExecutionOutcome::TimedOut, Utc::now()).unwrap(),
            true,
        )
        .unwrap();
    assert_eq!(capacity.used(), before);
    drop(held);
    drop(journal);
    assert_eq!(capacity.used(), 0);
}
#[test]
fn test_should_fill_and_drop_exact_twenty_five_thousand_events_without_recursion_overflow() {
    let (mut journal, capacity) = journal(25_000, 256 * 1024 * 1024);
    for _ in 0..24_999 {
        journal.append(event(), false).unwrap();
    }
    assert!(matches!(
        journal.append(event(), false),
        Err(ExecutionError::Capacity)
    ));
    journal
        .append(
            events::terminal(&ExecutionOutcome::TimedOut, Utc::now()).unwrap(),
            true,
        )
        .unwrap();
    assert_eq!(journal.view.len(), 25_000);
    assert_eq!(
        journal.view.page(25_000, 1, true).unwrap().events()[0].id,
        25_000
    );
    drop(journal);
    assert_eq!(capacity.used(), 0);
}

#[test]
fn test_should_reverse_fixed_history_without_changing_causal_ids_or_splitting_events() {
    let (mut journal, _) = journal(100, 32 * 1024 * 1024);
    let payload = Payload::from_value(&serde_json::json!("\0".repeat(40_000))).unwrap();
    for _ in 0..20 {
        journal
            .append(events::started(&payload, Utc::now()).unwrap(), false)
            .unwrap();
    }
    let old = journal.view.clone();
    journal.append(event(), false).unwrap();
    let first = old.page_reverse(20, 1000, true).unwrap();
    assert!(first.events().len() < 20);
    assert!(serde_json::to_vec(first.events()).unwrap().len() < PAGE_BYTES);
    assert_eq!(first.events().first().unwrap().id, 20);
    let next = old
        .page_reverse(first.next_id().unwrap(), 1000, true)
        .unwrap();
    assert_eq!(first.events().len() + next.events().len(), 20);
    assert_eq!(next.events().last().unwrap().id, 1);
    assert_eq!(next.next_id(), None);
    let omitted = old.page_reverse(20, 1000, false).unwrap();
    assert_eq!(omitted.events().len(), 20);
    assert!(
        !serde_json::to_string(omitted.events())
            .unwrap()
            .contains("u0000")
    );
    assert!(old.page_reverse(0, 1, true).is_err());
    assert!(old.page_reverse(21, 1, true).is_err());
    assert!(old.page_reverse(20, 0, true).is_err());
    assert!(
        ExecutionHistory::default()
            .page_reverse(0, 1, false)
            .unwrap()
            .events()
            .is_empty()
    );
}
