use super::*;
use crate::OK;
use std::boxed::Box;

#[repr(C, align(4))]
struct Storage([u8; STORAGE_SIZE as usize]);
struct Context(Box<MaybeUninit<Storage>>);

fn config() -> Config {
    let mut config: Config = unsafe { MaybeUninit::zeroed().assume_init() };
    config.size = size_of::<Config>() as u32;
    config.abi_version = ABI_VERSION;
    config.own_issi = 1_000_001;
    config.flags = C_FILTERED | C_INDIVIDUAL | C_SCRAMBLING | C_CARRIER;
    config.scrambling_code = 0x1234_5678;
    config.serving_carrier = 1593;
    config
}

fn event(kind: u32) -> Event {
    let mut event: Event = unsafe { MaybeUninit::zeroed().assume_init() };
    event.size = size_of::<Event>() as u32;
    event.abi_version = ABI_VERSION;
    event.kind = kind;
    event
}

impl Context {
    fn new() -> Self {
        let mut context = Self(Box::<Storage>::new_uninit());
        assert_eq!(unsafe { tetra_ms_mac_init(context.ptr(), STORAGE_SIZE, &config()) }, OK);
        context
    }
    fn ptr(&mut self) -> *mut c_void {
        self.0.as_mut_ptr().cast()
    }
    fn push(&mut self, event: &Event) -> i32 {
        unsafe { tetra_ms_mac_push_event(self.ptr(), event) }
    }
    fn action(&mut self) -> Result<Action, i32> {
        let mut out = MaybeUninit::<Action>::uninit();
        unsafe {
            ptr::addr_of_mut!((*out.as_mut_ptr()).size).write(size_of::<Action>() as u32);
            ptr::addr_of_mut!((*out.as_mut_ptr()).abi_version).write(ABI_VERSION);
            let result = tetra_ms_mac_poll_action(self.ptr(), out.as_mut_ptr());
            if result == OK { Ok(out.assume_init()) } else { Err(result) }
        }
    }
    fn status(&mut self) -> Status {
        let mut out = MaybeUninit::<Status>::uninit();
        unsafe {
            ptr::addr_of_mut!((*out.as_mut_ptr()).size).write(size_of::<Status>() as u32);
            ptr::addr_of_mut!((*out.as_mut_ptr()).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_ms_mac_get_status(self.ptr(), out.as_mut_ptr()), OK);
            out.assume_init()
        }
    }
    fn entropy(&mut self, event: &Event) -> Entropy {
        let mut out = MaybeUninit::<Entropy>::uninit();
        unsafe {
            ptr::addr_of_mut!((*out.as_mut_ptr()).size).write(size_of::<Entropy>() as u32);
            ptr::addr_of_mut!((*out.as_mut_ptr()).abi_version).write(ABI_VERSION);
            assert_eq!(tetra_ms_mac_entropy_needed(self.ptr(), event, out.as_mut_ptr()), OK);
            out.assume_init()
        }
    }
    fn define(&mut self, code: u32, imm: u32, priority: u32) {
        let mut input = event(SET_PARAMS);
        input.source = 1;
        input.code = code;
        input.params = Params {
            imm,
            wt: 1,
            nu: 2,
            min_priority: priority,
            ..Params::default()
        };
        assert_eq!(self.push(&input), OK);
        assert_eq!(self.action().unwrap().kind, ACCESS_PARAMS);
    }
    fn submit(&mut self, token: u32, bits: u32) {
        let mut input = event(SUBMIT);
        input.flags = E_RECEIPT | E_PRIORITY;
        input.receipt = token;
        input.priority = 3;
        input.pdu_bits = bits;
        input.pdu.fill(0xa5);
        assert_eq!(self.push(&input), OK);
        assert_eq!(self.action().unwrap().kind, SUBMITTED);
    }
}

#[cfg(feature = "c-smoke")]
#[test]
fn linked_c_mac_caller() {
    unsafe extern "C" {
        fn tetra_ms_mac_c_smoke() -> i32;
    }
    assert_eq!(unsafe { tetra_ms_mac_c_smoke() }, 0);
}

#[test]
fn caller_owned_in_place_init_reset_and_header_only_outputs() {
    let mut context = Context::new();
    let initial = context.status();
    assert_eq!(initial.flags, S_FILTERED | S_INDIVIDUAL | S_SCRAMBLING | S_CARRIER);
    assert_eq!(initial.actions, 0);
    assert!(initial.marker_mask.iter().all(|&value| value == 0));
    let mut input = event(ENCODE);
    input.value = 1;
    input.ssi = 1_000_001;
    input.pdu_bits = 20;
    input.pdu[0] = 0xab;
    input.pdu[1] = 0xcd;
    input.pdu[2] = 0xe0;
    let expected = wire::access(input.ssi, BitView::new(&input.pdu, 20).unwrap()).unwrap();
    assert_eq!(context.push(&input), OK);
    input.pdu.fill(0xff);
    let mut output = MaybeUninit::<Action>::uninit();
    unsafe {
        ptr::addr_of_mut!((*output.as_mut_ptr()).size).write(0);
        ptr::addr_of_mut!((*output.as_mut_ptr()).abi_version).write(ABI_VERSION);
        assert_eq!(tetra_ms_mac_poll_action(context.ptr(), output.as_mut_ptr()), ABI_MISMATCH);
    }
    assert_eq!(context.status().actions, 1);
    let out = context.action().unwrap();
    assert_eq!(out.kind, ENCODED);
    assert_eq!(out.pdu_bits, 92);
    assert_eq!(&out.pdu[..expected.as_bytes().len()], expected.as_bytes());
    assert!(out.pdu[expected.as_bytes().len()..].iter().all(|&value| value == 0));
    assert!(out.pdu2.iter().all(|&value| value == 0));
    let mut empty: Action = unsafe { MaybeUninit::zeroed().assume_init() };
    empty.size = size_of::<Action>() as u32;
    empty.abi_version = ABI_VERSION;
    empty.kind = 0xfeed_beef;
    assert_eq!(unsafe { tetra_ms_mac_poll_action(context.ptr(), &mut empty) }, EMPTY);
    assert_eq!(empty.kind, 0xfeed_beef);
    context.submit(9, 20);
    let mut scrambling = event(SCRAMBLING);
    scrambling.source = 1;
    scrambling.scrambling_code = 0xabcdef;
    assert_eq!(context.push(&scrambling), OK);
    assert_eq!(context.action().unwrap().kind, CONFIGURE);
    assert_eq!(context.status().pending_scrambling_code, 0x1234_5678);
    assert_eq!(context.status().scrambling_code, 0xabcdef);
    let before = context.status();
    let mut invalid = config();
    invalid.group_count = 257;
    assert_eq!(unsafe { tetra_ms_mac_reset(context.ptr(), &invalid) }, CAPACITY);
    assert_eq!(before, context.status());
    assert_eq!(unsafe { tetra_ms_mac_reset(context.ptr(), &config()) }, OK);
    assert_eq!(initial, context.status());
}

#[test]
fn validation_alias_checks_and_queue_full_are_non_consuming() {
    let mut context = Context::new();
    let mut input = event(ALLOCATE);
    input.allocation = Allocation {
        carrier: 1593,
        flags: 4,
        kind: 0,
        slot_bitmap: 8,
        owner: 1_000_001,
    };
    for _ in 0..ACTION_CAPACITY {
        assert_eq!(context.push(&input), OK);
    }
    let before = context.status();
    let mut submit = event(SUBMIT);
    submit.pdu_bits = 20;
    submit.flags = E_RECEIPT;
    submit.receipt = 99;
    assert_eq!(context.push(&submit), QUEUE_FULL);
    assert_eq!(before, context.status());
    input.flags = 0x8000_0000;
    assert_eq!(context.push(&input), ARGUMENT);
    assert_eq!(before, context.status());
    let mut uninitialized = MaybeUninit::<Event>::uninit();
    unsafe {
        ptr::addr_of_mut!((*uninitialized.as_mut_ptr()).size).write(0);
        ptr::addr_of_mut!((*uninitialized.as_mut_ptr()).abi_version).write(ABI_VERSION);
        assert_eq!(tetra_ms_mac_push_event(context.ptr(), uninitialized.as_ptr()), ABI_MISMATCH);
        let overlap = context
            .ptr()
            .cast::<u8>()
            .add(STORAGE_SIZE as usize - size_of::<Action>())
            .cast::<Action>();
        assert_eq!(tetra_ms_mac_poll_action(context.ptr(), overlap), ARGUMENT);
        assert_eq!(
            tetra_ms_mac_entropy_needed(context.ptr(), &input, (&mut input as *mut Event).cast()),
            ARGUMENT
        );
        let overflow = (usize::MAX & !3) as *mut c_void;
        assert_eq!(tetra_ms_mac_init(overflow, STORAGE_SIZE, &config()), ARGUMENT);
    }
    assert_eq!(before, context.status());
    for _ in 0..ACTION_CAPACITY {
        assert_eq!(context.action().unwrap().kind, ALLOCATION);
    }
    assert_eq!(context.action().err(), Some(EMPTY));
}

#[test]
fn pure_entropy_and_retry_actions_match_all_native_access_codes() {
    for raw_code in 0..4 {
        let mut context = Context::new();
        let mut definition = event(SET_PARAMS);
        definition.source = 1;
        definition.code = raw_code;
        definition.params = Params {
            imm: 0,
            wt: 1,
            nu: 2,
            fl_factor: 1,
            min_priority: 5,
            ..Params::default()
        };
        assert_eq!(context.push(&definition), OK);
        context.action().unwrap();
        let native_params = ra::RandomAccessParams {
            imm: 0,
            wt: 1,
            nu: 2,
            fl_factor: true,
            ts_ptr: 0,
            min_pdu_prio: 5,
            subscriber_class: None,
            gssi: None,
        };
        let code = [AccessCode::A, AccessCode::B, AccessCode::C, AccessCode::D][raw_code as usize];
        let emergency = raw_code == 0;
        let mut native = MsRandomAccess::new();
        native.initiate(TdmaTime::default(), code, &native_params, 6, emergency).unwrap();
        let mut initiate = event(RA_INITIATE);
        initiate.code = raw_code;
        initiate.priority = 6;
        initiate.flags = if emergency { E_EMERGENCY } else { 0 };
        assert_eq!(context.push(&initiate), OK);
        let mut emitted = 0;
        for step in 0..64 {
            let mut input = event(RA_POLL);
            input.now = step * 4;
            input.flags = E_UL_VALID;
            input.code = raw_code;
            let marker = step % 3 == 0;
            input.subslots = [SlotAccess {
                code_present: 1,
                code: raw_code,
                frame_kind: if marker { 3 } else { 2 },
                frame_length: if marker { 32 } else { 0 },
            }; 2];
            let frame_len = if marker {
                ra::BaseFrameLength::FrameMarker(32)
            } else {
                ra::BaseFrameLength::OngoingFrame
            };
            let assignment = ra::SlotAccessAssign {
                subslot1: ra::SubslotAccess {
                    access_code: Some(code),
                    frame_len,
                },
                subslot2: ra::SubslotAccess {
                    access_code: Some(code),
                    frame_len,
                },
            };
            let now = TdmaTime::from_int(input.now as i32);
            let needed = native.entropy_needed(now, &assignment, true, &native_params).unwrap();
            let before = context.status();
            for _ in 0..2 {
                let queried = context.entropy(&input);
                let expected = match needed {
                    None => (0, 0),
                    Some(ra::EntropyRequest::OneOfTwo) => (2, 2),
                    Some(ra::EntropyRequest::SubslotIndex(maximum)) => (1, u32::from(maximum)),
                };
                assert_eq!((queried.kind, queried.maximum), expected);
                assert_eq!(queried.admission_failure, 0);
                assert_eq!(before, context.status());
            }
            if let Some(ra::EntropyRequest::SubslotIndex(maximum)) = needed {
                input.choice_index = u32::from(maximum) + 1;
                assert_eq!(context.push(&input), ra_error(RaFailure::InvalidEntropy));
                assert_eq!(before, context.status());
            }
            input.choice_index = 3;
            input.choice_subslot = 2;
            let expected = native
                .poll_downlink_slot(now, &assignment, true, &native_params, &mut ra::RaChoices::new(3, ra::Subslot::Two))
                .unwrap();
            assert_eq!(context.push(&input), OK);
            match expected {
                Some(RaAction::Transmit { ul_time, subslot }) => {
                    emitted += 1;
                    let action = context.action().unwrap();
                    assert_eq!(action.kind, RA_TRANSMIT);
                    assert_eq!(action.now, ul_time.to_int() as u32);
                    assert_eq!(action.subslot, if subslot == ra::Subslot::One { 1 } else { 2 });
                    assert_eq!(action.flags & F_MAC_BLOCK, 0);
                }
                Some(RaAction::Failed(reason)) => {
                    let action = context.action().unwrap();
                    assert_eq!(action.kind, RA_FAILED);
                    assert_eq!(action.reason, ra_error(reason));
                }
                None => assert_eq!(context.action().err(), Some(EMPTY)),
                _ => panic!("unsolicited native success"),
            }
            let status = context.status();
            assert_eq!(status.ra_transmissions, u32::from(native.status().transmissions));
            assert_eq!(status.flags & S_RA_ACTIVE != 0, native.is_active());
        }
        assert_eq!(emitted, if emergency { 4 } else { 2 });
    }
}

fn ongoing_aach() -> Event {
    let mut input = event(AACH);
    input.pdu_bits = 14;
    input.pdu[0] = 0x42;
    input.pdu[1] = 0x08;
    input.choice_subslot = 2;
    input
}

#[test]
fn admission_rejection_returns_fragment_receipt_without_consuming_entropy() {
    let mut context = Context::new();
    context.define(0, 15, 5);
    context.submit(9, 100);
    let input = ongoing_aach();
    let before = context.status();
    let needed = context.entropy(&input);
    assert_eq!(needed.kind, 0);
    assert_eq!(needed.admission_failure, ra_error(RaFailure::AccessCodeNotPermitted));
    assert_eq!(before, context.status());
    assert_eq!(context.push(&input), OK);
    assert_eq!(context.action().unwrap().kind, AACH_INFO);
    let action = context.action().unwrap();
    assert_eq!(action.kind, RA_FAILED);
    assert_eq!(action.flags & (F_RECEIPT | F_RECEIPT2), F_RECEIPT2);
    assert_eq!(action.receipt2, 9);
    assert_eq!(context.status().flags & (S_PENDING | S_FRAGMENT | S_RA_ACTIVE), 0);
}

#[test]
fn fragmented_resource_ack_emits_reserved_blocks_and_final_receipt() {
    let mut context = Context::new();
    context.define(0, 15, 0);
    context.submit(99, 456);
    assert_eq!(context.push(&ongoing_aach()), OK);
    assert_eq!(context.action().unwrap().kind, AACH_INFO);
    assert_eq!(context.action().unwrap().kind, RA_TRANSMIT);
    let mut block = wire::MacBlock::new();
    for (width, value) in [
        (6, 0),
        (1, 1),
        (6, 5),
        (3, 1),
        (24, 1_000_001),
        (1, 0),
        (1, 1),
        (4, 2),
        (4, 0),
        (1, 0),
    ] {
        block.push(width, value).unwrap();
    }
    let mut input = event(DOWNLINK);
    input.pdu_bits = u32::from(block.len());
    input.pdu[..block.as_bytes().len()].copy_from_slice(block.as_bytes());
    assert_eq!(context.push(&input), OK);
    assert_eq!(context.action().unwrap().kind, RA_SUCCEEDED);
    let first = context.action().unwrap();
    assert_eq!(first.kind, TRANSMIT);
    assert_eq!(first.flags & F_RESERVED, F_RESERVED);
    assert_eq!(first.flags & F_RECEIPT, 0);
    assert_eq!(first.now, 2);
    assert_eq!(first.pdu_bits, 268);
    let resource = context.action().unwrap();
    assert_eq!(resource.kind, RESOURCE);
    assert_eq!(resource.route, 3);
    assert_eq!((resource.header_bits, resource.payload_end, resource.next_bits), (51, 51, 40));
    let reserved = context.status();
    assert_eq!(reserved.reserved_count, 1);
    assert_eq!(reserved.reserved_receipt, 99);
    assert_ne!(reserved.flags & S_RESERVED_RECEIPT, 0);
    let mut advance = event(SET_TIME);
    advance.now = 4;
    assert_eq!(context.push(&advance), OK);
    assert_eq!(context.push(&event(POLL_RESERVED)), OK);
    let last = context.action().unwrap();
    assert_eq!(last.kind, TRANSMIT);
    assert_eq!(last.now, 6);
    assert_eq!(last.receipt, 99);
    assert_ne!(last.flags & F_RECEIPT, 0);
    assert_eq!(context.status().reserved_count, 0);
    let source = [0xa5; 57];
    let expected = wire::FragmentBlocks::build(BitView::new(&source, 456).unwrap().window(56, 400).unwrap(), 2).unwrap();
    for (output, expected) in [&first, &last].into_iter().zip(expected.as_slice()) {
        assert_eq!(&output.pdu[..expected.as_bytes().len()], expected.as_bytes());
    }
}

#[test]
fn displacement_and_associated_traffic_preserve_receipts_and_packed_bits() {
    let mut context = Context::new();
    context.submit(0, 20);
    for token in 1..=16 {
        context.submit(token, 20);
    }
    let mut input = event(SUBMIT);
    input.flags = E_RECEIPT;
    input.receipt = 17;
    input.pdu_bits = 20;
    assert_eq!(context.push(&input), OK);
    assert_eq!(context.action().unwrap().kind, SUBMITTED);
    let dropped = context.action().unwrap();
    assert_eq!(dropped.kind, DISCARDED_TRANSFER);
    assert_eq!(dropped.receipt, 1);
    assert_eq!(dropped.flags & F_RECEIPT, F_RECEIPT);
    assert_eq!(dropped.pdu_bits, 92);
    assert_eq!(context.push(&event(CANCEL_ACCESS)), OK);
    let cancelled = context.action().unwrap();
    assert_eq!(cancelled.receipt, 0);
    assert_eq!(cancelled.flags & F_RECEIPT, F_RECEIPT);
    assert_eq!(context.push(&event(PROMOTE)), OK);
    assert_eq!(context.status().pending_receipt, 2);
    assert_eq!(unsafe { tetra_ms_mac_reset(context.ptr(), &config()) }, OK);
    let mut allocation = event(ALLOCATE);
    allocation.allocation = Allocation {
        carrier: 1593,
        flags: 4,
        kind: 0,
        slot_bitmap: 8,
        owner: 1_000_001,
    };
    assert_eq!(context.push(&allocation), OK);
    context.action().unwrap();
    for token in 0..=16 {
        let mut input = event(SUBMIT);
        input.flags = E_RECEIPT | E_STEALING;
        input.receipt = token;
        input.pdu_bits = 20;
        assert_eq!(context.push(&input), OK);
        assert_eq!(context.action().unwrap().route, 2);
        if token == 16 {
            let dropped = context.action().unwrap();
            assert_eq!(dropped.kind, DISCARDED_STOLEN);
            assert_eq!(dropped.receipt, 0);
        }
    }
    assert_eq!(context.push(&event(POLL_TRAFFIC)), OK);
    let traffic = context.action().unwrap();
    assert_eq!(traffic.kind, TRANSMIT);
    assert_eq!(traffic.flags & (F_TRAFFIC | F_SECOND), F_TRAFFIC | F_SECOND);
    assert_eq!(
        (traffic.channel, traffic.channel2, traffic.pdu_bits, traffic.pdu2_bits),
        (3, 4, 124, 274)
    );
    assert_eq!(traffic.receipt, 1);
    assert!(traffic.pdu2.iter().all(|&value| value == 0));
    let mut audio = event(AUDIO);
    audio.pdu_bits = 274;
    for _ in 0..4 {
        assert_eq!(context.push(&audio), OK);
    }
    assert_eq!(context.push(&audio), OK);
    assert_eq!(context.action().unwrap().kind, AUDIO_DROPPED);
    assert_eq!(context.push(&event(UPLANE)), OK);
    assert_eq!(context.status().queued_audio, 0);
}

fn raw(fields: &[(u8, u32)]) -> tetra_ms_core::types::BitStorage<512> {
    let mut result = tetra_ms_core::types::BitStorage::new();
    for &(width, value) in fields {
        result.push(width, value).unwrap();
    }
    result
}

fn set_pdu(input: &mut Event, bits: BitView<'_>) {
    input.pdu.fill(0);
    input.pdu_bits = copy_bits(&mut input.pdu, bits);
}

#[test]
fn raw_resource_windows_feed_the_existing_independent_reassembly_abi() {
    let mut context = Context::new();
    let mut cfg = config();
    cfg.group_count = 1;
    cfg.groups[0] = 91;
    assert_eq!(unsafe { tetra_ms_mac_reset(context.ptr(), &cfg) }, OK);
    let mut resource = raw(&[
        (2, 0),
        (1, 1),
        (1, 0),
        (2, 0),
        (1, 0),
        (6, 63),
        (3, 6),
        (24, 91),
        (6, 16),
        (1, 1),
        (4, 7),
        (1, 1),
        (4, 1),
        (4, 0),
        (1, 1),
        (2, 1),
        (4, 4),
        (2, 3),
        (1, 0),
        (1, 0),
        (12, 1593),
        (1, 0),
        (2, 0),
        (2, 2),
    ]);
    assert_eq!(resource.len(), 88);
    resource.push(13, 0x1234).unwrap();
    resource.push(1, 1).unwrap();
    resource.push(22, 0).unwrap();
    let mut input = event(DOWNLINK);
    set_pdu(&mut input, resource.view());
    assert_eq!(context.push(&input), OK);
    let first = context.action().unwrap();
    assert_eq!((first.kind, first.route, first.ssi, first.marker), (RESOURCE, 5, 91, 16));
    assert_eq!(
        (first.header_bits, first.payload_end, first.fill_bits, first.pdu_bits),
        (88, 101, 23, 13)
    );
    assert_eq!(
        (
            first.grant.kind,
            first.grant.slots,
            first.allocation.slot_bitmap,
            first.allocation_result
        ),
        (2, 1, 4, 0)
    );
    assert_eq!(
        first.flags & (F_FILL | F_ADDRESS | F_MARKER | F_GRANT | F_ALLOCATION | F_ALLOCATION_RESULT),
        F_FILL | F_ADDRESS | F_MARKER | F_GRANT | F_ALLOCATION | F_ALLOCATION_RESULT
    );
    let status = context.status();
    assert_eq!(status.assigned_slot_bitmap, 4);
    assert_eq!(status.marker_mask[0], 1 << 16);
    assert_eq!(status.marker_owners[16], 91);
    let end = raw(&[(3, 3), (1, 0), (1, 0), (6, 2), (1, 0), (1, 0), (3, 5)]);
    set_pdu(&mut input, end.view());
    assert_eq!(context.push(&input), OK);
    let last = context.action().unwrap();
    assert_eq!((last.kind, last.header_bits, last.pdu_bits), (END, 13, 3));
    let mut reassembly = Box::<[u32; crate::reassembly::STORAGE_SIZE as usize / 4]>::new_uninit();
    let storage = reassembly.as_mut_ptr().cast();
    assert_eq!(
        unsafe { crate::reassembly::tetra_ms_reassembly_init(storage, crate::reassembly::STORAGE_SIZE) },
        OK
    );
    let mut fragment = crate::reassembly::Event {
        size: size_of::<crate::reassembly::Event>() as u32,
        abi_version: ABI_VERSION,
        kind: 1,
        now: 0,
        ssi: first.ssi,
        ssi_type: first.ssi_type,
        aie_valid: 0,
        aie_info: 0,
        pdu_bits: first.pdu_bits as u16,
        reserved: 0,
        pdu: first.pdu,
    };
    assert_eq!(unsafe { crate::reassembly::tetra_ms_reassembly_push(storage, &fragment) }, OK);
    fragment.kind = 3;
    fragment.pdu_bits = last.pdu_bits as u16;
    fragment.pdu = last.pdu;
    assert_eq!(unsafe { crate::reassembly::tetra_ms_reassembly_push(storage, &fragment) }, OK);
    let mut output = MaybeUninit::<crate::reassembly::Action>::uninit();
    unsafe {
        ptr::addr_of_mut!((*output.as_mut_ptr()).size).write(size_of::<crate::reassembly::Action>() as u32);
        ptr::addr_of_mut!((*output.as_mut_ptr()).abi_version).write(ABI_VERSION);
        assert_eq!(crate::reassembly::tetra_ms_reassembly_poll(storage, 0, output.as_mut_ptr()), OK);
        let output = output.assume_init();
        assert_eq!((output.ssi, output.fragments, output.pdu_bits), (91, 2, 16));
        assert_eq!(&output.pdu[..2], &[0x91, 0xa5]);
    }
}

#[test]
fn broadcast_precedence_sync_and_identity_configuration_preserve_native_metadata() {
    let mut context = Context::new();
    let definition = raw(&[
        (4, 9),
        (1, 0),
        (2, 0),
        (4, 15),
        (4, 4),
        (4, 3),
        (1, 1),
        (4, 4),
        (3, 3),
        (2, 1),
        (16, 0xbeef),
        (3, 4),
    ]);
    let mut input = event(DOWNLINK);
    set_pdu(&mut input, definition.view());
    assert_eq!(context.push(&input), OK);
    let action = context.action().unwrap();
    assert_eq!(action.kind, ACCESS_PARAMS);
    assert_eq!(action.params.subscriber_class, 0xbeef);
    assert_eq!(action.flags, F_APPLIED);
    let info = raw(&[
        (4, 8),
        (12, 1593),
        (4, 4),
        (2, 0),
        (3, 7),
        (1, 0),
        (17, 0),
        (1, 0),
        (16, 7),
        (2, 2),
        (4, 1),
        (4, 1),
        (4, 1),
        (1, 0),
        (4, 0),
        (3, 0),
        (4, 10),
    ]);
    set_pdu(&mut input, info.view());
    assert_eq!(context.push(&input), OK);
    let correction = context.action().unwrap();
    assert_eq!((correction.kind, correction.hyperframe), (CONFIGURE, 7));
    let broadcast = context.action().unwrap();
    assert_eq!(
        (
            broadcast.kind,
            broadcast.carrier,
            broadcast.band,
            broadcast.duplex,
            broadcast.hyperframe
        ),
        (SYSINFO, 1593, 4, 7, 7)
    );
    assert_eq!((broadcast.header_bits, broadcast.pdu_bits, broadcast.pdu[0]), (82, 4, 0xa0));
    assert_eq!(broadcast.params.imm, 1);
    let status = context.status();
    assert_eq!(
        (
            status.defined_mask,
            status.sysinfo_present,
            status.params[0].imm,
            status.params[0].subscriber_class
        ),
        (1, 1, 15, 0xbeef)
    );
    let sync = raw(&[(4, 1), (6, 1), (2, 3), (5, 11), (6, 9), (8, 0), (10, 420), (14, 555)]);
    input.kind = SYNC;
    set_pdu(&mut input, sync.view());
    assert_eq!(context.push(&input), OK);
    let synchronized = context.action().unwrap();
    let expected_time = TdmaTime { t: 4, f: 11, m: 9, h: 0 }.to_int() as u32;
    assert_eq!(
        (
            synchronized.kind,
            synchronized.now,
            synchronized.colour,
            synchronized.header_bits,
            synchronized.pdu_bits
        ),
        (SYNCED, expected_time, 1, 31, 24)
    );
    let mut identities = event(IDENTITIES);
    identities.mcc = 420;
    identities.mnc = 555;
    identities.flags = E_INDIVIDUAL | E_GROUPS;
    identities.ssi = 1_000_001;
    identities.group_count = 2;
    identities.groups[0] = 91;
    identities.groups[1] = 220;
    assert_eq!(context.push(&identities), OK);
    let configured = context.action().unwrap();
    assert_eq!(configured.flags & F_SCRAMBLING, F_SCRAMBLING);
    assert_eq!(configured.scrambling_code, 1_761_749_767);
    let status = context.status();
    assert_eq!(
        (status.now, status.mcc, status.mnc, status.group_count),
        (expected_time, 420, 555, 2)
    );
    assert_eq!(&status.groups[..2], &[91, 220]);
}

#[test]
fn all_native_wire_encoders_and_multi_output_capacity_are_exposed() {
    let mut context = Context::new();
    let source = [0xa5; 512];
    for (selector, length) in [(1, 62), (2, 83), (3, 264), (4, 81), (5, 254), (6, 456)] {
        let mut input = event(ENCODE);
        input.ssi = 1_000_001;
        input.value = selector;
        input.pdu_bits = length;
        input.pdu = source;
        assert_eq!(context.push(&input), OK);
        let out = context.action().unwrap();
        let bits = BitView::new(&source, length as u16).unwrap();
        let expected = match selector {
            1 => wire::access(input.ssi, bits).unwrap(),
            2 => wire::stolen_data(input.ssi, bits).unwrap(),
            3 => wire::fragment(bits).unwrap(),
            4 => wire::end_hu(bits).unwrap(),
            5 => wire::end_ul(bits).unwrap(),
            _ => {
                let (block, tail, kind) = wire::access_fragment(input.ssi, bits).unwrap();
                assert_eq!((out.fragment_kind, out.fragment_slots), fragment_kind(kind));
                assert_eq!(out.pdu2_bits, u32::from(tail.len()));
                assert_eq!(&out.pdu2[..tail.as_bytes().len()], tail.as_bytes());
                block
            }
        };
        assert_eq!(out.pdu_bits, u32::from(expected.len()));
        assert_eq!(&out.pdu[..expected.as_bytes().len()], expected.as_bytes());
    }
    let mut allocate = event(ALLOCATE);
    allocate.allocation = Allocation {
        carrier: 1593,
        flags: 4,
        kind: 0,
        slot_bitmap: 8,
        owner: 1_000_001,
    };
    for _ in 0..3 {
        assert_eq!(context.push(&allocate), OK);
    }
    let mut sequence = event(ENCODE);
    sequence.value = 7;
    sequence.source = 6;
    sequence.pdu_bits = 1500;
    sequence.pdu = source;
    let before = context.status();
    assert_eq!(context.push(&sequence), QUEUE_FULL);
    assert_eq!(before, context.status());
    for _ in 0..3 {
        context.action().unwrap();
    }
    assert_eq!(context.push(&sequence), OK);
    let expected = wire::FragmentBlocks::build(BitView::new(&source, 1500).unwrap(), 6).unwrap();
    for (index, block) in expected.as_slice().iter().enumerate() {
        let action = context.action().unwrap();
        assert_eq!((action.kind, action.code, action.fragment_slots), (ENCODED, index as u32, 6));
        assert_eq!(&action.pdu[..block.as_bytes().len()], block.as_bytes());
    }
    assert_eq!(context.action().err(), Some(EMPTY));
}

#[test]
fn associated_pdus_continue_without_c_side_bit_parsing_and_reject_invalid_progress() {
    let mut context = Context::new();
    let input = raw(&[
        (6, 0),
        (1, 0),
        (6, 7),
        (3, 1),
        (24, 1_000_001),
        (3, 0),
        (13, 0x1234),
        (2, 0),
        (1, 0),
        (1, 0),
        (2, 0),
        (1, 0),
        (6, 2),
        (3, 0),
        (8, 0),
    ]);
    assert_eq!(input.len(), 80);
    let mut downlink = event(DOWNLINK);
    set_pdu(&mut downlink, input.view());
    assert_eq!(context.push(&downlink), OK);
    let first = context.action().unwrap();
    assert_eq!((first.route, first.pdu_bits, first.next_bits), (4, 13, 56));
    let mut continuation = event(CONTINUE_BLOCK);
    set_pdu(&mut continuation, input.view());
    continuation.value = 81;
    let before = context.status();
    assert_eq!(context.push(&continuation), TRUNCATED);
    assert_eq!(before, context.status());
    continuation.value = first.next_bits;
    assert_eq!(context.push(&continuation), OK);
    let next = context.action().unwrap();
    assert_eq!((next.kind, next.pdu_bits), (NEXT_PDU, 24));
    downlink.pdu = next.pdu;
    downlink.pdu_bits = next.pdu_bits;
    assert_eq!(context.push(&downlink), OK);
    let null = context.action().unwrap();
    assert_eq!((null.kind, null.route, null.header_bits), (RESOURCE, 0, 16));
    continuation.value = 80;
    assert_eq!(context.push(&continuation), OK);
    assert_eq!(context.action().err(), Some(EMPTY));
}
