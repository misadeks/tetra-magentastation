//! Caller-driven CMCE/SDS integration without transport, runtime or audio.
use tetra_ms_core::{
    cmce::{CallState, CallTable, CmceAction, CmceCore, CmceEvent, DEFAULT_BASIC},
    sds::{SdsCore, SdsEvent},
};
use tetra_ms_types::{SsiType, TdmaTime, TetraAddress};

fn main() {
    let now = TdmaTime::default();
    let mut cmce = CmceCore::new();
    cmce.push_event(now, CmceEvent::Identity(Some(123))).unwrap();
    cmce.push_event(
        now,
        CmceEvent::Originate {
            called: TetraAddress::new(91, SsiType::Gssi),
            basic: DEFAULT_BASIC,
            duplex: false,
            request_tx: true,
            hook: false,
            external: None,
        },
    )
    .unwrap();
    while let Some(action) = cmce.pop_action() {
        if let CmceAction::Uplink { sdu, route, .. } = action {
            assert_eq!(sdu.view().read(0, 5), Ok(7));
            assert_eq!(route.main_address, TetraAddress::new(123, SsiType::Issi));
            // The caller supplies this complete CMCE SDU to MLE.
        }
    }
    let mut sds = SdsCore::new();
    sds.push_event(SdsEvent::Status {
        destination: TetraAddress::new(456, SsiType::Issi),
        status: 0x8002,
    })
    .unwrap();
    assert!(sds.pop_action().is_some());
    println!(
        "CallState: {} bytes; 16-call table: {} bytes; CMCE entity: {} bytes; SDS: {} bytes",
        core::mem::size_of::<CallState>(),
        core::mem::size_of::<CallTable<CallState, 16>>(),
        core::mem::size_of::<CmceCore>(),
        core::mem::size_of::<SdsCore>()
    );
}
