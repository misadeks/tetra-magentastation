use tetra_config::magentastation::SharedConfig;
use tetra_core::tetra_entities::TetraEntity;
use tetra_core::{BitBuffer, Sap, SsiType, TdmaTime, TetraAddress, TxReporter, TxState};
use tetra_entities::llc::llc_ms::Llc;
use tetra_entities::{MessageQueue, TetraEntityTrait};
use tetra_saps::tla::TlaTlDataReqBl;
use tetra_saps::tma::TmaUnitdataInd;
use tetra_saps::{SapMsg, SapMsgInner};

#[path = "common/default_stack.rs"]
mod default_stack;

const ISSI: u32 = 1_000_001;

fn llc() -> Llc {
    let mut cfg = default_stack::default_test_config_ms();
    cfg.ms.as_mut().unwrap().issi = ISSI;
    Llc::new(SharedConfig::from_parts(cfg, None))
}

fn send(llc: &mut Llc, q: &mut MessageQueue, reporter: TxReporter, group: bool) {
    llc.rx_prim(
        q,
        SapMsg::new(
            Sap::TlaSap,
            TetraEntity::Mle,
            TetraEntity::Llc,
            SapMsgInner::TlaTlDataReqBl(TlaTlDataReqBl {
                main_address: TetraAddress::new(ISSI, if group { SsiType::Gssi } else { SsiType::Issi }),
                link_id: 2,
                endpoint_id: 7,
                tl_sdu: BitBuffer::from_bitstr("00010"),
                pdu_priority: Some(3),
                is_emergency: false,
                stealing_permission: true,
                subscriber_class: 1,
                fcs_flag: false,
                air_interface_encryption: None,
                stealing_repeats_flag: None,
                data_class_info: None,
                req_handle: 17,
                graceful_degradation: None,
                chan_alloc: None,
                tx_reporter: Some(reporter),
            }),
        ),
    );
}

fn receive(llc: &mut Llc, q: &mut MessageQueue, bits: &str, ssi: u32) {
    receive_address(llc, q, bits, TetraAddress::issi(ssi));
}

fn receive_address(llc: &mut Llc, q: &mut MessageQueue, bits: &str, address: TetraAddress) {
    llc.rx_prim(
        q,
        SapMsg::new(
            Sap::TmaSap,
            TetraEntity::Umac,
            TetraEntity::Llc,
            SapMsgInner::TmaUnitdataInd(TmaUnitdataInd {
                pdu: Some(BitBuffer::from_bitstr(bits)),
                main_address: address,
                scrambling_code: 11,
                endpoint_id: 7,
                new_endpoint_id: None,
                css_endpoint_id: None,
                air_interface_encryption: 0,
                chan_change_response_req: false,
                chan_change_handle: None,
                chan_info: None,
            }),
        ),
    );
}

fn transmitted_bits(q: &mut MessageQueue) -> String {
    let SapMsgInner::TmaUnitdataReq(req) = q.pop_front().expect("transmit action").msg else {
        panic!("expected transmit");
    };
    assert_eq!(req.req_handle, 17);
    assert_eq!(req.link_id, if req.main_address.ssi_type == SsiType::Gssi { 0 } else { 2 });
    assert!(req.stealing_permission);
    assert_eq!(req.endpoint_id, 7);
    assert_eq!(req.pdu_priority, Some(3));
    req.pdu.to_bitstr()
}

#[test]
fn automatic_ack_identity_policy_preserves_individual_and_non_individual_cases() {
    for (address, should_ack) in [
        (TetraAddress::new(ISSI, SsiType::Ssi), true),
        (TetraAddress::issi(ISSI), true),
        (TetraAddress::new(ISSI + 1, SsiType::Ssi), false),
        (TetraAddress::issi(ISSI + 1), false),
        (TetraAddress::new(42, SsiType::EventLabel), true),
        (TetraAddress::new(99, SsiType::Gssi), true),
    ] {
        let mut llc = llc();
        let mut q = MessageQueue::new();
        let now = TdmaTime::default();
        llc.tick_start(&mut q, now);
        receive_address(&mut llc, &mut q, "00011101", address);
        assert!(matches!(q.pop_front().unwrap().msg, SapMsgInner::TlaTlDataIndBl(_)));
        assert_eq!(llc.tick_end(&mut q, now), should_ack, "{address:?}");
        if should_ack {
            let SapMsgInner::TmaUnitdataReq(req) = q.pop_front().unwrap().msg else {
                panic!("automatic ACK");
            };
            assert_eq!(req.pdu.to_bitstr(), "00111");
            assert_eq!(req.main_address, address);
        }
        assert!(q.pop_front().is_none());
    }
}

#[test]
fn golden_stop_and_wait_early_ack_and_sequence_alternation() {
    let mut llc = llc();
    let mut q = MessageQueue::new();
    let now = TdmaTime::default();
    llc.tick_start(&mut q, now);
    send(&mut llc, &mut q, TxReporter::new(), false);
    send(&mut llc, &mut q, TxReporter::new(), false);
    assert!(llc.tick_end(&mut q, now));
    assert_eq!(transmitted_bits(&mut q), "0001000010");
    assert!(q.pop_front().is_none(), "second request waits for the first ACK");
    receive(&mut llc, &mut q, "00110", ISSI);
    assert!(llc.tick_end(&mut q, now));
    assert_eq!(transmitted_bits(&mut q), "0001100010");
}

#[test]
fn same_tick_ack_observes_completed_mac_but_does_not_invent_transmission() {
    for completed in [true, false] {
        let mut llc = llc();
        let mut q = MessageQueue::new();
        let now = TdmaTime::default();
        let caller = TxReporter::new();
        llc.tick_start(&mut q, now);
        send(&mut llc, &mut q, caller.clone(), false);
        llc.tick_end(&mut q, now);
        let SapMsgInner::TmaUnitdataReq(transmit) = q.pop_front().unwrap().msg else {
            panic!("MAC request");
        };
        if completed {
            transmit.tx_reporter.unwrap().mark_transmitted();
        }
        receive(&mut llc, &mut q, "00110", ISSI);
        let expected = if completed { TxState::Acknowledged } else { TxState::Pending };
        assert_eq!(caller.get_state(), expected, "MAC completed: {completed}");
        llc.tick_end(&mut q, now);
        assert_eq!(caller.get_state(), expected);
        send(&mut llc, &mut q, TxReporter::new(), false);
        llc.tick_end(&mut q, now);
        assert_eq!(transmitted_bits(&mut q), "0001100010");
    }
}

#[test]
fn golden_group_udata_and_ack_piggyback() {
    let mut llc = llc();
    let mut q = MessageQueue::new();
    let now = TdmaTime::default();
    llc.tick_start(&mut q, now);
    receive(&mut llc, &mut q, "00011101", ISSI);
    let SapMsgInner::TlaTlDataIndBl(ind) = q.pop_front().unwrap().msg else {
        panic!("data");
    };
    assert_eq!(ind.tl_sdu.unwrap().to_bitstr(), "101");
    send(&mut llc, &mut q, TxReporter::new(), false);
    llc.tick_end(&mut q, now);
    assert_eq!(transmitted_bits(&mut q), "00001000010");
    send(&mut llc, &mut q, TxReporter::new_unacked(), true);
    llc.tick_end(&mut q, now);
    assert_eq!(transmitted_bits(&mut q), "001000010");
}

#[test]
fn golden_retry_starts_at_mac_receipt_and_reuses_encoded_bits() {
    let mut llc = llc();
    let mut q = MessageQueue::new();
    let now = TdmaTime::default();
    let reporter = TxReporter::new();
    llc.tick_start(&mut q, now);
    send(&mut llc, &mut q, reporter.clone(), false);
    llc.tick_end(&mut q, now);
    let SapMsgInner::TmaUnitdataReq(transmit) = q.pop_front().unwrap().msg else {
        panic!("transmit");
    };
    let golden = transmit.pdu.to_bitstr();
    let mac_reporter = transmit.tx_reporter.unwrap();
    let later = now.add_timeslots(100);
    llc.tick_start(&mut q, later);
    assert!(!llc.tick_end(&mut q, later), "no retry while MAC is pending");
    mac_reporter.mark_transmitted();
    llc.tick_end(&mut q, later);
    let before = later.add_timeslots(15);
    llc.tick_start(&mut q, before);
    assert!(!llc.tick_end(&mut q, before));
    let deadline = later.add_timeslots(16);
    llc.tick_start(&mut q, deadline);
    assert!(llc.tick_end(&mut q, deadline));
    assert_eq!(transmitted_bits(&mut q), golden);
}

#[test]
fn duplicate_data_is_currently_delivered_twice_but_overheard_data_is_not_acked() {
    let mut llc = llc();
    let mut q = MessageQueue::new();
    let now = TdmaTime::default();
    llc.tick_start(&mut q, now);
    receive(&mut llc, &mut q, "00010101", ISSI + 1);
    receive(&mut llc, &mut q, "00010101", ISSI + 1);
    assert_eq!(q.iter().count(), 2);
    q.pop_front();
    q.pop_front();
    assert!(!llc.tick_end(&mut q, now));
    assert!(q.pop_front().is_none());
}
