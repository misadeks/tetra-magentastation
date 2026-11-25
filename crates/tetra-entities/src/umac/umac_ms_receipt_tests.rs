use super::tests::{ms_umac, ongoing_a_assign, tma_stealing_req, tma_uplink_req, umac_cfg, umac_with_queued_priority};
use super::*;

#[test]
fn rejected_access_reports_both_first_and_tail_sidecar_receipts() {
    let mut umac = umac_with_queued_priority(7, Some(0), false);
    let first = TxReporter::new();
    let tail = TxReporter::new();
    let first_handle = umac.track_reporter(Some(first.clone())).unwrap();
    let tail_handle = umac.track_reporter(Some(tail.clone())).unwrap();
    umac.pending_uplink.as_mut().unwrap().tx_reporter = first_handle;
    umac.pending_fragment = Some(UplinkFragment {
        remainder: pack(&BitBuffer::from_bitstr("101")).unwrap(),
        scrambling_code: 123,
        end_kind: FragEndKind::MacEndHu,
        tx_reporter: tail_handle,
    });
    umac.drive_random_access(&mut MessageQueue::new(), &ongoing_a_assign());
    assert!(first.is_discarded() && tail.is_discarded());
    assert!(umac.pending_uplink.is_none() && umac.pending_fragment.is_none());
    assert!(umac.tx_reporters.is_empty());
}

#[test]
fn one_fragment_reporter_stays_in_sidecar_until_completion_or_missed_slot() {
    for missed in [false, true] {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(123);
        let reporter = TxReporter::new();
        let mut queue = MessageQueue::new();
        umac.rx_tma_prim(&mut queue, tma_uplink_req(&reporter, &"1".repeat(456)));
        assert!(umac.pending_uplink.as_ref().unwrap().tx_reporter.is_none());
        let handle = umac.pending_fragment.as_ref().unwrap().tx_reporter.unwrap();
        assert_eq!(umac.tx_reporters.len(), 1);
        umac.pending_uplink = None; // MAC-ACCESS acknowledged; no receipt lives on the first fragment.
        umac.emit_fragment_end(
            &mut queue,
            Some(&BasicSlotgrant {
                capacity_allocation: BasicSlotgrantCapAlloc::Grant2Slots,
                granting_delay: BasicSlotgrantGrantingDelay::CapAllocAtNextOpportunity,
            }),
        );
        assert_eq!(umac.reserved_tx.as_ref().unwrap().tx_reporter, Some(handle));
        assert_eq!(umac.tx_reporters.len(), 1);
        assert!(!reporter.is_transmitted() && !reporter.is_discarded());
        umac.dltime = umac.dltime.add_timeslots(if missed { 5 } else { 4 });
        umac.drive_reserved_tx(&mut queue);
        assert!(umac.tx_reporters.is_empty());
        assert!(umac.reserved_tx.is_none());
        assert_eq!(reporter.is_discarded(), missed);
        assert_eq!(reporter.is_transmitted(), !missed);
    }
}

#[test]
fn combined_queues_bound_sidecar_and_discard_evicted_or_dropped_receipts() {
    let mut umac = ms_umac();
    umac.scrambling_code = Some(123);
    umac.assigned_traffic_slots = [true, false, false, false];
    let mut queue = MessageQueue::new();
    let active = TxReporter::new();
    let queued = TxReporter::new();
    let stolen = TxReporter::new();
    let mut retained = vec![active.clone()];
    umac.rx_tma_prim(&mut queue, tma_uplink_req(&active, "101"));
    umac.rx_tma_prim(&mut queue, tma_uplink_req(&queued, "101"));
    for _ in 1..mac::MAX_UPLINK_QUEUE {
        let reporter = TxReporter::new();
        umac.rx_tma_prim(&mut queue, tma_uplink_req(&reporter, "101"));
        retained.push(reporter);
    }
    umac.rx_tma_prim(&mut queue, tma_stealing_req(true, 1, Some(stolen.clone())));
    for _ in 1..mac::MAX_STOLEN_SIGNALLING {
        let reporter = TxReporter::new();
        umac.rx_tma_prim(&mut queue, tma_stealing_req(true, 1, Some(reporter.clone())));
        retained.push(reporter);
    }
    assert_eq!(umac.tx_reporters.len(), MAX_TX_REPORTERS - 1);
    let next = TxReporter::new();
    umac.rx_tma_prim(&mut queue, tma_uplink_req(&next, "101"));
    assert!(queued.is_discarded());
    assert!(!active.is_discarded() && !next.is_discarded());
    retained.push(next);
    let next = TxReporter::new();
    umac.rx_tma_prim(&mut queue, tma_stealing_req(true, 1, Some(next.clone())));
    assert!(stolen.is_discarded());
    retained.push(next);
    assert_eq!(umac.tx_reporters.len(), MAX_TX_REPORTERS - 1);
    drop(umac);
    assert!(retained.iter().all(TxReporter::is_discarded));
}

#[test]
fn reset_and_identity_changes_cancel_sidecars_without_reusing_handles() {
    let mut umac = ms_umac();
    umac.scrambling_code = Some(123);
    umac.assigned_traffic_slots = [true, false, false, false];
    let mut queue = MessageQueue::new();
    let active = TxReporter::new();
    let queued = TxReporter::new();
    let stolen = TxReporter::new();
    umac.rx_tma_prim(&mut queue, tma_uplink_req(&active, &"1".repeat(100)));
    umac.rx_tma_prim(&mut queue, tma_uplink_req(&queued, "101"));
    umac.rx_tma_prim(&mut queue, tma_stealing_req(true, 1, Some(stolen.clone())));
    let old = umac.pending_fragment.as_ref().unwrap().tx_reporter.unwrap();
    let last = umac.next_tx_reporter;
    umac.reset().unwrap();
    assert!(active.is_discarded() && queued.is_discarded() && stolen.is_discarded());
    assert!(umac.tx_reporters.is_empty());
    assert!(umac.pending_uplink.is_none() && umac.pending_fragment.is_none() && umac.reserved_tx.is_none());
    assert!(umac.uplink_queue.is_empty() && umac.pending_stolen_signalling.is_empty());
    umac.scrambling_code = Some(123);
    let next = TxReporter::new();
    umac.rx_tma_prim(&mut queue, tma_uplink_req(&next, "101"));
    assert!(umac.pending_uplink.as_ref().unwrap().tx_reporter.unwrap() > last);
    umac.finish_receipt(Some(old), true);
    assert!(!next.is_transmitted() && !next.is_discarded());
    umac.set_config(umac.config.clone());
    assert!(!next.is_discarded());
    let mobile = umac_cfg("Ms", 2_000_001, &[]);
    umac.config.apply_config_live(mobile.config.config().as_ref().clone());
    umac.tick_start(&mut queue, TdmaTime::default());
    assert!(next.is_discarded() && umac.tx_reporters.is_empty());
    umac.set_config(mobile.config.clone());
    umac.scrambling_code = Some(123);
    let next = TxReporter::new();
    umac.rx_tma_prim(&mut queue, tma_uplink_req(&next, "101"));
    let other_identity = umac_cfg("Ms", 3_000_001, &[]);
    umac.set_config(other_identity.config.clone());
    assert!(next.is_discarded() && umac.tx_reporters.is_empty());
    assert_eq!(umac.valid_individual_ssi, Some(3_000_001));
}

#[test]
fn exhausted_handles_do_not_wrap_and_taken_receipts_leave_protocol_ownership() {
    let mut umac = ms_umac();
    umac.scrambling_code = Some(123);
    umac.next_tx_reporter = u32::MAX - 1;
    let retained = TxReporter::new();
    let rejected = TxReporter::new();
    let mut queue = MessageQueue::new();
    umac.rx_tma_prim(&mut queue, tma_uplink_req(&retained, "101"));
    assert_eq!(umac.pending_uplink.as_ref().unwrap().tx_reporter, Some(u32::MAX));
    umac.rx_tma_prim(&mut queue, tma_uplink_req(&rejected, "101"));
    assert!(rejected.is_discarded());
    assert!(!retained.is_discarded());
    assert_eq!(umac.tx_reporters.len(), 1);
    let pending = umac.take_pending_uplink().unwrap();
    assert!(umac.tx_reporters.is_empty());
    drop(umac);
    assert!(!retained.is_discarded() && !retained.is_transmitted());
    pending.tx_reporter.unwrap().try_mark_transmitted();
    assert!(retained.is_transmitted());
}
