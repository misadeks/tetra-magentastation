use tetra_ms_core::{
    llc::{Completion, LlcAction, LlcConfig, LlcCore, LlcEvent, SendRequest, Service, TxFeedback},
    mac::wire,
    mle::{MleAction, MleConfig, MleCore, MleEvent, Network, Protocol, UplinkSource},
    mm::{self, CellId, MmCore, RegState, ServiceStatus},
    types::{PackedBits, TdmaTime, TetraAddress},
};

fn fields(values: &[(u8, u32)]) -> PackedBits {
    let mut bits = PackedBits::new();
    for &(width, value) in values {
        bits.push(width, value).unwrap();
    }
    bits
}

#[test]
fn serving_cell_to_registration_and_network_accept_use_only_portable_components() {
    let network = Network { mcc: 901, mnc: 9999 };
    let config = MleConfig {
        home_network: network,
        allowed_networks: &[],
        subscriber_class: Some(1),
    };
    let address = TetraAddress::issi(1_000_001);
    let mut mle = MleCore::new();
    let mut mm = MmCore::new(
        mm::Config {
            issi: address.ssi,
            mcc: network.mcc,
            mnc: network.mnc,
            initial_la: 9,
        },
        &[],
    )
    .unwrap();
    let mut llc = LlcCore::new(LlcConfig { own_issi: address.ssi }).unwrap();
    let now = TdmaTime::default();

    let sync = fields(&[(10, u32::from(network.mcc)), (14, u32::from(network.mnc)), (2, 0), (2, 0), (1, 0)]);
    mle.handle(config, MleEvent::Sync(sync.view())).unwrap();
    assert_eq!(mle.pop_action(), Some(MleAction::ConfigureNetwork(network)));
    let info = fields(&[(14, 9), (16, 1), (12, 0x840)]);
    mle.handle(config, MleEvent::Sysinfo(info.view())).unwrap();
    let Some(MleAction::Activate {
        network,
        location_area,
        registration_required,
        system_wide_services,
        ..
    }) = mle.pop_action()
    else {
        panic!("serving cell activation")
    };
    mm.handle(mm::Event::Activate {
        cell: CellId {
            mcc: network.mcc,
            mnc: network.mnc,
            la: location_area,
        },
        registration_required,
        system_wide_services,
        force_register: false,
    })
    .unwrap();
    let Some(mm::Action::Send {
        sdu,
        priority,
        detach_receipt,
    }) = mm.poll_action()
    else {
        panic!("registration demand");
    };
    assert!(!detach_receipt);
    assert_eq!(sdu.len(), 10);
    assert_eq!(sdu.view().read(0, 10), Ok(0b0010011000));
    assert_eq!(mm.service_status(), ServiceStatus::WaitingForRegistration);
    while mm.poll_action().is_some() {}
    mle.handle(
        config,
        MleEvent::Uplink {
            source: UplinkSource::Mm {
                priority: Some(priority),
                emergency: false,
            },
            sdu: sdu.view(),
            endpoint_id: 0,
            link_id: 0,
        },
    )
    .unwrap();
    let Some(MleAction::Transmit { pdu, parameters }) = mle.pop_action() else {
        panic!("MLE envelope");
    };
    assert_eq!(parameters.priority, Some(6));
    llc.handle(
        now,
        &LlcEvent::Send(SendRequest {
            command: 1,
            address,
            service: Service::Acknowledged,
            fcs: parameters.fcs,
            sdu: pdu.view(),
        }),
    )
    .unwrap();
    llc.handle(now, &LlcEvent::Tick).unwrap();
    let Some(LlcAction::Transmit {
        pdu,
        receipt: Some(receipt),
        ..
    }) = llc.poll_action()
    else {
        panic!("registration transmission");
    };
    assert_eq!(pdu.len(), 18);
    assert_eq!(pdu.view().read(5, 3), Ok(1)); // MLE MM discriminator.
    assert_eq!(pdu.view().read(8, 10), Ok(0b0010011000));
    let block = wire::access(address.ssi, pdu.view()).unwrap();
    assert_eq!(block.len(), 92);
    assert_eq!(block.view().read(30, 18), pdu.view().read(0, 18));
    llc.handle(
        now,
        &LlcEvent::Transmission {
            receipt,
            feedback: TxFeedback::Transmitted,
        },
    )
    .unwrap();

    // BL-ADATA acknowledges N(S)=0 and carries MLE/MM D-LOCATION-UPDATE-ACCEPT.
    let accept = fields(&[(4, 0), (1, 0), (1, 0), (3, 1), (8, 0b01010110)]);
    llc.handle(
        now.add_timeslots(4),
        &LlcEvent::Receive {
            tag: 7,
            address,
            pdu: accept.view(),
        },
    )
    .unwrap();
    assert_eq!(
        llc.poll_action(),
        Some(LlcAction::Completed {
            command: 1,
            result: Completion::Acknowledged
        })
    );
    let Some(LlcAction::Deliver { sdu, .. }) = llc.poll_action() else {
        panic!("LLC delivery")
    };
    mle.handle(config, MleEvent::Downlink(sdu.view())).unwrap();
    let Some(MleAction::Deliver {
        protocol: Protocol::Mm,
        sdu,
        ..
    }) = mle.pop_action()
    else {
        panic!("MM routing");
    };
    mm.handle(mm::Event::Downlink(sdu.view())).unwrap();
    assert_eq!(mm.state().reg_state, RegState::Registered);
    assert_eq!(mm.service_status(), ServiceStatus::InService);
}
