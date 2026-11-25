//! MS MAC reassembly: one bounded chain per downlink timeslot.
use tetra_ms_types::{BitView, FragmentBits, TdmaTime, TetraAddress};

pub const MAX_FRAGMENT_BITS: u16 = 4096;
pub const TIMEOUT_SLOTS: i32 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentError {
    InvalidTime,
    InvalidAddress,
    NotActive,
    NotComplete,
    CapacityExceeded,
    OverflowAborted,
    CounterExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentMetadata {
    pub address: TetraAddress,
    pub first: TdmaTime,
    pub last: TdmaTime,
    pub fragments: u16,
    pub aie_info: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentPhase {
    Inactive,
    Active,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentStatus {
    pub phase: FragmentPhase,
    pub bits: u16,
    pub fragments: u16,
}

#[derive(Default)]
struct Slot {
    data: FragmentBits,
    metadata: Option<FragmentMetadata>,
    complete: bool,
}

#[derive(Default)]
pub struct MsReassembly {
    slots: [Slot; 4],
}

impl MsReassembly {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init_in_place(storage: &mut core::mem::MaybeUninit<Self>) -> &mut Self {
        let out = storage.as_mut_ptr();
        // SAFETY: The caller lends exclusive aligned storage. Initialize every
        // field of each of the four contiguous slots before exposing Self.
        // None has no initialized metadata payload and these types have no Drop.
        unsafe {
            let slots = core::ptr::addr_of_mut!((*out).slots).cast::<Slot>();
            for index in 0..4 {
                let slot = slots.add(index);
                core::ptr::addr_of_mut!((*slot).data).write(FragmentBits::new());
                core::ptr::addr_of_mut!((*slot).metadata).write(None);
                core::ptr::addr_of_mut!((*slot).complete).write(false);
            }
            storage.assume_init_mut()
        }
    }

    pub fn status(&self) -> [FragmentStatus; 4] {
        core::array::from_fn(|index| {
            let slot = &self.slots[index];
            FragmentStatus {
                phase: if slot.metadata.is_none() {
                    FragmentPhase::Inactive
                } else if slot.complete {
                    FragmentPhase::Complete
                } else {
                    FragmentPhase::Active
                },
                bits: slot.data.len(),
                fragments: slot.metadata.map_or(0, |metadata| metadata.fragments),
            }
        })
    }

    fn index(now: TdmaTime) -> Result<usize, FragmentError> {
        if !now.is_valid() {
            return Err(FragmentError::InvalidTime);
        }
        Ok(usize::from(now.t - 1))
    }

    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.data.clear();
            slot.metadata = None;
            slot.complete = false;
        }
    }

    /// Returns a bitmask of expired slots. The original boundary is strictly >40.
    pub fn age(&mut self, now: TdmaTime) -> Result<u8, FragmentError> {
        Self::index(now)?;
        let mut expired = 0;
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.metadata.is_some_and(|metadata| now.diff(metadata.last) > TIMEOUT_SLOTS) {
                slot.metadata = None;
                slot.complete = false;
                slot.data.clear();
                expired |= 1 << index;
            }
        }
        Ok(expired)
    }

    /// Starts a chain, explicitly reporting replacement of an existing chain.
    pub fn first(&mut self, now: TdmaTime, address: TetraAddress, aie_info: Option<i32>, bits: BitView<'_>) -> Result<bool, FragmentError> {
        let index = Self::index(now)?;
        if address.ssi > 0xff_ffff {
            return Err(FragmentError::InvalidAddress);
        }
        if bits.len() > MAX_FRAGMENT_BITS {
            return Err(FragmentError::CapacityExceeded);
        }
        let slot = &mut self.slots[index];
        let replaced = slot.metadata.is_some();
        slot.data.clear();
        slot.data.append(bits).map_err(|_| FragmentError::CapacityExceeded)?;
        slot.metadata = Some(FragmentMetadata {
            address,
            aie_info,
            first: now,
            last: now,
            fragments: 1,
        });
        slot.complete = false;
        Ok(replaced)
    }

    /// Overflow aborts the chain, matching the desktop's historical behavior.
    pub fn next(&mut self, now: TdmaTime, bits: BitView<'_>, last: bool) -> Result<(), FragmentError> {
        let index = Self::index(now)?;
        let slot = &mut self.slots[index];
        let Some(mut metadata) = slot.metadata.filter(|_| !slot.complete) else {
            return Err(FragmentError::NotActive);
        };
        if bits.len() > MAX_FRAGMENT_BITS - slot.data.len() {
            slot.metadata = None;
            slot.complete = false;
            slot.data.clear();
            return Err(FragmentError::OverflowAborted);
        }
        metadata.fragments = metadata.fragments.checked_add(1).ok_or(FragmentError::CounterExhausted)?;
        slot.data.append(bits).map_err(|_| FragmentError::CapacityExceeded)?;
        metadata.last = now;
        slot.metadata = Some(metadata);
        slot.complete = last;
        Ok(())
    }

    pub fn metadata(&self, now: TdmaTime) -> Result<Option<&FragmentMetadata>, FragmentError> {
        Ok(self.slots[Self::index(now)?].metadata.as_ref())
    }

    /// Borrow completed data until the caller has copied/consumed it.
    pub fn completed(&self, now: TdmaTime) -> Result<(FragmentMetadata, BitView<'_>), FragmentError> {
        let slot = &self.slots[Self::index(now)?];
        if !slot.complete {
            return Err(FragmentError::NotComplete);
        }
        let metadata = slot.metadata.ok_or(FragmentError::NotComplete)?;
        Ok((metadata, slot.data.view()))
    }

    pub fn release(&mut self, now: TdmaTime) -> Result<(), FragmentError> {
        self.completed(now)?;
        let slot = &mut self.slots[Self::index(now)?];
        slot.metadata = None;
        slot.complete = false;
        slot.data.clear();
        Ok(())
    }
}

const _: () = {
    assert!(core::mem::size_of::<MsReassembly>() <= 2304);
    assert!(core::mem::size_of::<FragmentBits>() == 516);
    assert!(core::mem::size_of::<[FragmentStatus; 4]>() <= 32);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_preserve_bits_and_original_timeout_boundary() {
        let mut core = MsReassembly::new();
        let now = TdmaTime::default().add_timeslots(-4);
        let address = TetraAddress::issi(1234);
        assert_eq!(core.first(now, address, None, BitView::new(&[0], 3).unwrap()), Ok(false));
        core.next(now.add_timeslots(4), BitView::new(&[0xe0], 3).unwrap(), false).unwrap();
        core.next(now.add_timeslots(8), BitView::new(&[0x30], 4).unwrap(), true).unwrap();
        let (metadata, data) = core.completed(now).unwrap();
        assert_eq!(metadata.fragments, 3);
        assert_eq!(data.read(0, 10), Ok(0b0001110011));
        assert_eq!(core.age(now.add_timeslots(48)), Ok(0));
        assert_eq!(core.age(now.add_timeslots(49)), Ok(1 << (now.t - 1)));
        assert_eq!(core.completed(now).map(|_| ()), Err(FragmentError::NotComplete));
    }

    #[test]
    fn capacity_aborts_only_affected_chain_and_invalid_input_does_not_index() {
        let mut core = MsReassembly::new();
        let now = TdmaTime::default();
        let bytes = [0xff; 513];
        core.first(now, TetraAddress::issi(1), None, BitView::new(&bytes, 4096).unwrap())
            .unwrap();
        assert_eq!(
            core.next(now, BitView::new(&bytes, 1).unwrap(), true),
            Err(FragmentError::OverflowAborted)
        );
        assert_eq!(
            core.next(now, BitView::new(&bytes, 1).unwrap(), true),
            Err(FragmentError::NotActive)
        );
        assert_eq!(
            core.first(now, TetraAddress::issi(1), None, BitView::new(&bytes, 4097).unwrap()),
            Err(FragmentError::CapacityExceeded)
        );
        assert_eq!(core.age(TdmaTime { t: 0, ..now }), Err(FragmentError::InvalidTime));
    }
}
