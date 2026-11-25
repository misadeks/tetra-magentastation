use tetra_ms_core::{llc::*, types::*};

fn core() -> LlcCore {
    LlcCore::new(LlcConfig { own_issi: 1 }).unwrap()
}

fn bits(text: &str) -> PackedBits {
    let mut result = PackedBits::new();
    for byte in text.bytes() {
        result.push(1, u32::from(byte - b'0')).unwrap();
    }
    result
}

fn send(core: &mut LlcCore, command: u32, address: u32, service: Service) -> Result<(), LlcError> {
    core.handle(
        TdmaTime::default(),
        &LlcEvent::Send(SendRequest {
            command,
            address: TetraAddress::issi(address),
            service,
            fcs: false,
            sdu: bits("00010").view(),
        }),
    )
}

fn tick(core: &mut LlcCore, now: TdmaTime) {
    core.handle(now, &LlcEvent::Tick).unwrap();
}

fn receipt(action: LlcAction) -> Receipt {
    let LlcAction::Transmit {
        receipt: Some(receipt),
        pdu,
        ..
    } = action
    else {
        panic!("transmit");
    };
    assert_eq!(pdu, bits("0001000010"));
    receipt
}

fn feedback(core: &mut LlcCore, now: TdmaTime, receipt: Receipt, feedback: TxFeedback) -> Result<(), LlcError> {
    core.handle(now, &LlcEvent::Transmission { receipt, feedback })
}

#[test]
fn golden_wire_encoding_and_early_ack_unblocks_stop_and_wait() {
    let mut core = core();
    assert_eq!(core.status(), LlcStatus::default());
    send(&mut core, 1, 1, Service::Acknowledged).unwrap();
    send(&mut core, 2, 1, Service::Acknowledged).unwrap();
    tick(&mut core, TdmaTime::default());
    receipt(core.poll_action().unwrap());
    assert!(core.poll_action().is_none());
    core.handle(
        TdmaTime::default(),
        &LlcEvent::Receive {
            tag: 3,
            address: TetraAddress::issi(1),
            pdu: bits("00110").view(),
        },
    )
    .unwrap();
    assert_eq!(
        core.poll_action(),
        Some(LlcAction::Completed {
            command: 1,
            result: Completion::Acknowledged
        })
    );
    tick(&mut core, TdmaTime::default());
    let LlcAction::Transmit { pdu, command, .. } = core.poll_action().unwrap() else {
        panic!("transmit");
    };
    assert_eq!(command, 2);
    assert_eq!(pdu, bits("0001100010"));
}

#[test]
fn retry_waits_for_receipt_bounds_retries_and_rejects_stale_generations_across_wrap() {
    let mut core = core();
    let start = TdmaTime::from_int(-8);
    send(&mut core, 1, 1, Service::Acknowledged).unwrap();
    tick(&mut core, start);
    let mut current = receipt(core.poll_action().unwrap());
    tick(&mut core, start.add_timeslots(100));
    assert!(core.poll_action().is_none());
    feedback(&mut core, start.add_timeslots(100), current, TxFeedback::Deferred).unwrap();
    tick(&mut core, start.add_timeslots(200));
    assert!(core.poll_action().is_none());
    let mut now = start.add_timeslots(200);
    for attempt in 0..=N252_RETRIES {
        feedback(
            &mut core,
            now,
            current,
            if attempt % 2 == 0 {
                TxFeedback::Transmitted
            } else {
                TxFeedback::Rejected
            },
        )
        .unwrap();
        assert_eq!(
            feedback(&mut core, now, current, TxFeedback::Transmitted),
            Err(LlcError::InvalidTransition)
        );
        tick(&mut core, now.add_timeslots(T251_SLOTS - 1));
        assert!(core.poll_action().is_none());
        now = now.add_timeslots(T251_SLOTS);
        tick(&mut core, now);
        if attempt < N252_RETRIES {
            let next = receipt(core.poll_action().unwrap());
            assert_ne!(next, current);
            assert_eq!(
                feedback(&mut core, now, current, TxFeedback::Transmitted),
                Err(LlcError::StaleReceipt)
            );
            current = next;
        }
    }
    assert_eq!(
        core.poll_action(),
        Some(LlcAction::Completed {
            command: 1,
            result: Completion::RetryExhausted
        })
    );
    assert_eq!(core.status().pending, 0);
    assert_eq!(
        feedback(&mut core, now, current, TxFeedback::Transmitted),
        Err(LlcError::StaleReceipt)
    );
}

#[test]
fn ack_piggyback_and_traffic_auto_ack_match_original_bits_and_slot() {
    let mut core = core();
    let now = TdmaTime::default();
    core.handle(
        now,
        &LlcEvent::Receive {
            tag: 4,
            address: TetraAddress::issi(1),
            pdu: bits("00011101").view(),
        },
    )
    .unwrap();
    assert_eq!(
        core.poll_action(),
        Some(LlcAction::Deliver {
            tag: 4,
            service: Service::Acknowledged,
            fcs: false,
            sdu: bits("101"),
        })
    );
    send(&mut core, 1, 1, Service::Acknowledged).unwrap();
    tick(&mut core, now);
    let LlcAction::Transmit { pdu, .. } = core.poll_action().unwrap() else {
        panic!("transmit");
    };
    assert_eq!(pdu, bits("00001000010"));
    core.handle(
        now,
        &LlcEvent::Receive {
            tag: 5,
            address: TetraAddress::issi(1),
            pdu: bits("00011101").view(),
        },
    )
    .unwrap();
    core.poll_action();
    tick(&mut core, now);
    let LlcAction::Transmit {
        pdu, ack_slot, receipt, ..
    } = core.poll_action().unwrap()
    else {
        panic!("ack");
    };
    assert_eq!(pdu, bits("00111"));
    assert_eq!(ack_slot, Some(3)); // Original LLC uses downlink time minus two.
    assert_eq!(receipt, None);
}

#[test]
fn queues_fail_explicitly_without_mutation_and_resume_after_draining() {
    let mut core = core();
    for command in 0..PENDING_CAPACITY {
        send(&mut core, u32::from(command), 1, Service::Acknowledged).unwrap();
    }
    let full = core.status();
    assert_eq!(send(&mut core, 20, 1, Service::Acknowledged), Err(LlcError::QueueFull));
    assert_eq!(send(&mut core, 0, 1, Service::Acknowledged), Err(LlcError::ReceiptBusy));
    assert_eq!(core.status(), full);
    for command in 100..100 + UDATA_CAPACITY {
        send(&mut core, u32::from(command), 2, Service::Unacknowledged).unwrap();
    }
    assert_eq!(send(&mut core, 200, 1, Service::Unacknowledged), Err(LlcError::QueueFull));
    for tag in 0..ACK_CAPACITY {
        core.handle(
            TdmaTime::default(),
            &LlcEvent::Receive {
                tag: u32::from(tag),
                address: TetraAddress::issi(1),
                pdu: bits("00010").view(),
            },
        )
        .unwrap();
        core.poll_action();
    }
    let full = core.status();
    assert_eq!(
        core.handle(
            TdmaTime::default(),
            &LlcEvent::Receive {
                tag: 99,
                address: TetraAddress::issi(1),
                pdu: bits("00010").view(),
            }
        ),
        Err(LlcError::QueueFull)
    );
    assert_eq!(core.status(), full);
    tick(&mut core, TdmaTime::default());
    let mut count = 0;
    while core.poll_action().is_some() {
        count += 1;
    }
    assert_eq!(count, 1 + ACK_CAPACITY + UDATA_CAPACITY);
    assert_eq!(core.status().scheduled_acks, 0);
}

#[test]
fn action_queue_saturation_is_atomic() {
    let mut core = core();
    for tag in 0..ACTION_CAPACITY {
        core.handle(
            TdmaTime::default(),
            &LlcEvent::Receive {
                tag: u32::from(tag),
                address: TetraAddress::issi(1),
                pdu: bits("00101").view(),
            },
        )
        .unwrap();
    }
    let before = core.status();
    assert_eq!(
        core.handle(
            TdmaTime::default(),
            &LlcEvent::Receive {
                tag: 99,
                address: TetraAddress::issi(1),
                pdu: bits("00101").view(),
            }
        ),
        Err(LlcError::QueueFull)
    );
    assert_eq!(core.status(), before);
    core.poll_action();
    core.handle(
        TdmaTime::default(),
        &LlcEvent::Receive {
            tag: 99,
            address: TetraAddress::issi(1),
            pdu: bits("00101").view(),
        },
    )
    .unwrap();
}

#[test]
fn invalid_input_reset_and_link_capacity() {
    assert!(matches!(LlcCore::new(LlcConfig { own_issi: 0 }), Err(LlcError::InvalidIdentity)));
    let mut core = core();
    assert_eq!(
        core.handle(
            TdmaTime {
                t: 0,
                ..TdmaTime::default()
            },
            &LlcEvent::Tick
        ),
        Err(LlcError::InvalidTime)
    );
    for (pdu, error) in [
        ("000", LlcError::TruncatedPdu),
        ("0000", LlcError::TruncatedPdu),
        ("0001", LlcError::TruncatedPdu),
        ("0011", LlcError::TruncatedPdu),
        ("1000", LlcError::UnsupportedPdu(8)),
        ("0101", LlcError::TruncatedPdu),
    ] {
        assert_eq!(
            core.handle(
                TdmaTime::default(),
                &LlcEvent::Receive {
                    tag: 0,
                    address: TetraAddress::issi(1),
                    pdu: bits(pdu).view(),
                }
            ),
            Err(error)
        );
        assert_eq!(core.status(), LlcStatus::default());
    }
    send(&mut core, 1, 1, Service::Acknowledged).unwrap();
    tick(&mut core, TdmaTime::default());
    let old = receipt(core.poll_action().unwrap());
    core.reset(LlcConfig { own_issi: 2 }).unwrap();
    assert_eq!(core.status(), LlcStatus::default());
    assert_eq!(
        feedback(&mut core, TdmaTime::default(), old, TxFeedback::Transmitted),
        Err(LlcError::StaleReceipt)
    );
    for ssi in 1..=u32::from(LINK_CAPACITY) {
        send(&mut core, ssi, ssi, Service::Acknowledged).unwrap();
        tick(&mut core, TdmaTime::default());
        core.poll_action();
        core.handle(
            TdmaTime::default(),
            &LlcEvent::Receive {
                tag: 0,
                address: TetraAddress::issi(ssi),
                pdu: bits("00110").view(),
            },
        )
        .unwrap();
        core.poll_action();
    }
    assert_eq!(send(&mut core, 20, 20, Service::Acknowledged), Err(LlcError::CapacityExceeded));
    assert_eq!(core.status().pending, 0);
}

#[test]
fn oversized_input_does_not_consume_a_sequence_or_pending_ack() {
    let mut core = core();
    core.handle(
        TdmaTime::default(),
        &LlcEvent::Receive {
            tag: 0,
            address: TetraAddress::issi(1),
            pdu: bits("00010").view(),
        },
    )
    .unwrap();
    core.poll_action();
    let before = core.status();
    let mut large = PackedBits::new();
    for _ in 0..MAX_PDU_BITS / 32 {
        large.push(32, 0).unwrap();
    }
    assert_eq!(
        core.handle(
            TdmaTime::default(),
            &LlcEvent::Send(SendRequest {
                command: 1,
                address: TetraAddress::issi(1),
                service: Service::Acknowledged,
                fcs: false,
                sdu: large.view(),
            })
        ),
        Err(LlcError::CapacityExceeded)
    );
    assert_eq!(core.status(), before);
    send(&mut core, 1, 1, Service::Acknowledged).unwrap();
    tick(&mut core, TdmaTime::default());
    let Some(LlcAction::Transmit { pdu, receipt: Some(_), .. }) = core.poll_action() else {
        panic!("transmit")
    };
    assert_eq!(pdu, bits("00000000010"));
}

#[test]
fn receive_capacity_counts_the_header_as_well_as_the_sdu() {
    let mut core = core();
    let bytes = [0x20; MAX_PDU_BITS as usize / 8 + 1]; // BL-UDATA, no FCS.
    let before = core.status();
    assert_eq!(
        core.handle(
            TdmaTime::default(),
            &LlcEvent::Receive {
                tag: 0,
                address: TetraAddress::issi(1),
                pdu: BitView::new(&bytes, MAX_PDU_BITS + 1).unwrap(),
            }
        ),
        Err(LlcError::CapacityExceeded)
    );
    assert_eq!(core.status(), before);
    core.handle(
        TdmaTime::default(),
        &LlcEvent::Receive {
            tag: 0,
            address: TetraAddress::issi(1),
            pdu: BitView::new(&bytes, MAX_PDU_BITS).unwrap(),
        },
    )
    .unwrap();
    let Some(LlcAction::Deliver { sdu, .. }) = core.poll_action() else {
        panic!("deliver")
    };
    assert_eq!(sdu.len(), MAX_PDU_BITS - 4);
}

#[test]
fn identical_sequences_are_deterministic() {
    let mut first = core();
    let mut second = core();
    for command in 0..PENDING_CAPACITY {
        send(&mut first, u32::from(command), 1, Service::Acknowledged).unwrap();
        send(&mut second, u32::from(command), 1, Service::Acknowledged).unwrap();
    }

    for slots in 0..100 {
        let now = TdmaTime::from_int(slots);
        tick(&mut first, now);
        tick(&mut second, now);
        assert_eq!(first.status(), second.status());
        while let Some(action) = first.poll_action() {
            assert_eq!(second.poll_action(), Some(action));
        }
        assert!(second.poll_action().is_none());
    }
}

#[test]
fn caller_owned_initialization_and_reset_do_not_need_a_second_core() {
    let mut storage = core::mem::MaybeUninit::<LlcCore>::uninit();
    assert!(matches!(
        LlcCore::init_in_place(&mut storage, LlcConfig { own_issi: 0 }),
        Err(LlcError::InvalidIdentity)
    ));
    let core = LlcCore::init_in_place(&mut storage, LlcConfig { own_issi: 1 }).unwrap();
    assert_eq!(core.status(), LlcStatus::default());
    send(core, 1, 1, Service::Acknowledged).unwrap();
    tick(core, TdmaTime::default());
    receipt(core.poll_action().unwrap());
    let address = core as *const LlcCore;
    core.reset(LlcConfig { own_issi: 1 }).unwrap();
    assert_eq!(core as *const LlcCore, address);
    assert_eq!(core.status(), LlcStatus::default());
}
