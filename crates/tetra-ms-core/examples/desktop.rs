use tetra_ms_core::{
    llc::{LlcConfig, LlcCore, LlcError, LlcEvent, MEMORY_FOOTPRINT, SendRequest, Service},
    types::{BitView, TdmaTime, TetraAddress},
};

fn main() -> Result<(), LlcError> {
    println!("{MEMORY_FOOTPRINT:?}");
    println!(
        "random_access={} access_parameters={} reassembly={} fragment_bits={}",
        core::mem::size_of::<tetra_ms_core::random_access::MsRandomAccess>(),
        core::mem::size_of::<tetra_ms_core::random_access::AccessParamStore>(),
        core::mem::size_of::<tetra_ms_core::reassembly::MsReassembly>(),
        core::mem::size_of::<tetra_ms_core::types::FragmentBits>(),
    );
    println!(
        "mm={} mle={} mac={} cmce={} sds={}",
        core::mem::size_of::<tetra_ms_core::mm::MmCore>(),
        core::mem::size_of::<tetra_ms_core::mle::MleCore>(),
        core::mem::size_of::<tetra_ms_core::mac::MsMac>(),
        core::mem::size_of::<tetra_ms_core::cmce::CmceCore>(),
        core::mem::size_of::<tetra_ms_core::sds::SdsCore>(),
    );
    let mut core = LlcCore::new(LlcConfig { own_issi: 1_000_001 })?;
    let now = TdmaTime::default();
    core.handle(
        now,
        &LlcEvent::Send(SendRequest {
            command: 1,
            address: TetraAddress::issi(1_000_001),
            service: Service::Acknowledged,
            fcs: false,
            sdu: BitView::new(&[0x10], 5)?,
        }),
    )?;
    core.handle(now, &LlcEvent::Tick)?;
    while let Some(action) = core.poll_action() {
        println!("{action:?}");
        // A real adapter sends Transmit to MAC and feeds its receipt back.
    }
    core.reset(LlcConfig { own_issi: 1_000_001 })?;
    Ok(())
}
