use tetra_ms_core::{
    llc::{LlcAction, LlcConfig, LlcCore, LlcEvent, Service},
    reassembly::MsReassembly,
    types::{BitView, TdmaTime, TetraAddress},
};

#[test]
fn fragmented_downlink_delivers_exact_llc_sdu_and_ack_without_desktop_services() {
    let address = TetraAddress::issi(1234);
    let mut fragments = MsReassembly::new();
    let mut llc = LlcCore::new(LlcConfig { own_issi: address.ssi }).unwrap();
    let first = TdmaTime::default();
    let last = first.add_timeslots(8);

    // The existing BL-DATA + detach golden, 0001000010, split across MAC fragments.
    fragments.first(first, address, None, BitView::new(&[0x00], 3).unwrap()).unwrap();
    fragments
        .next(first.add_timeslots(4), BitView::new(&[0x80], 3).unwrap(), false)
        .unwrap();
    fragments.next(last, BitView::new(&[0x20], 4).unwrap(), true).unwrap();
    let (metadata, pdu) = fragments.completed(last).unwrap();
    llc.handle(
        last,
        &LlcEvent::Receive {
            tag: 9,
            address: metadata.address,
            pdu,
        },
    )
    .unwrap();
    fragments.release(last).unwrap();

    let Some(LlcAction::Deliver { tag, service, sdu, .. }) = llc.poll_action() else {
        panic!("SDU delivery");
    };
    assert_eq!(tag, 9);
    assert_eq!(service, Service::Acknowledged);
    assert_eq!(sdu.len(), 5);
    assert_eq!(sdu.view().read(0, 5), Ok(0b00010));
    llc.handle(last, &LlcEvent::Tick).unwrap();
    let Some(LlcAction::Transmit {
        receipt, ack_slot, pdu, ..
    }) = llc.poll_action()
    else {
        panic!("automatic ACK");
    };
    assert_eq!(receipt, None);
    assert_eq!(ack_slot, Some(last.add_timeslots(-2).t));
    assert_eq!(pdu.len(), 5);
    assert_eq!(pdu.view().read(0, 5), Ok(0b00110));
    assert!(llc.poll_action().is_none());
}
