use tetra_core::{BitBuffer, TdmaTime, TetraAddress, Todo};
use tetra_ms_core::reassembly::{FragmentError, MsReassembly};

use crate::umac::subcomp::defrag::{DefragBuffer, DefragBufferState};

/// Application allocation/logging boundary for the portable MS reassembler.
pub struct MsDefrag {
    core: MsReassembly,
}

impl Default for MsDefrag {
    fn default() -> Self {
        Self::new()
    }
}

impl MsDefrag {
    pub fn new() -> Self {
        Self { core: MsReassembly::new() }
    }

    pub fn reset(&mut self) {
        self.core.reset();
    }

    pub fn age_buffers(&mut self, time: TdmaTime) {
        match self.core.age(time) {
            Ok(0) => {}
            Ok(expired) => tracing::warn!(expired, "MS fragment chains timed out"),
            Err(error) => tracing::warn!(?error, "MS fragment aging failed"),
        }
    }

    pub fn insert_first(&mut self, buffer: &mut BitBuffer, time: TdmaTime, address: TetraAddress, aie_info: Option<Todo>) {
        let result = remaining(buffer).and_then(|bits| self.core.first(time, address, aie_info, bits));
        match result {
            Ok(replaced) => {
                if replaced {
                    tracing::warn!("MS first fragment replaced an existing chain");
                }
                buffer.set_raw_pos(buffer.get_raw_end());
            }
            Err(error) => tracing::warn!(?error, "MS first fragment rejected"),
        }
    }

    fn append(&mut self, buffer: &mut BitBuffer, time: TdmaTime, last: bool) {
        match remaining(buffer).and_then(|bits| self.core.next(time, bits, last)) {
            Ok(()) => buffer.set_raw_pos(buffer.get_raw_end()),
            Err(error) => tracing::warn!(?error, "MS continuation fragment rejected"),
        }
    }

    pub fn insert_next(&mut self, buffer: &mut BitBuffer, time: TdmaTime) {
        self.append(buffer, time, false);
    }

    pub fn insert_last(&mut self, buffer: &mut BitBuffer, time: TdmaTime) {
        self.append(buffer, time, true);
    }

    pub fn get_aie_info(&self, time: TdmaTime) -> Option<&Todo> {
        match self.core.metadata(time) {
            Ok(metadata) => metadata.and_then(|value| value.aie_info.as_ref()),
            Err(error) => {
                tracing::warn!(?error, "MS fragment metadata rejected");
                None
            }
        }
    }

    pub fn take_defragged_buf(&mut self, time: TdmaTime) -> Option<DefragBuffer> {
        let (metadata, data) = match self.core.completed(time) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(?error, "MS fragment chain not complete");
                return None;
            }
        };
        let mut buffer = BitBuffer::new(usize::from(data.len()));
        for offset in 0..data.len() {
            // The checked view guarantees this bit is in range.
            match data.read(offset, 1) {
                Ok(bit) => buffer.write_bit(bit as u8),
                Err(error) => {
                    tracing::error!(?error, "MS completed fragment copy failed");
                    return None;
                }
            }
        }
        buffer.seek(0);
        let output = DefragBuffer {
            state: DefragBufferState::Complete,
            addr: metadata.address,
            t_first: metadata.first,
            t_last: metadata.last,
            num_frags: usize::from(metadata.fragments),
            aie_info: metadata.aie_info,
            buffer,
        };
        if let Err(error) = self.core.release(time) {
            tracing::error!(?error, "MS fragment release failed");
            return None;
        }
        Some(output)
    }
}

fn remaining(buffer: &BitBuffer) -> Result<tetra_ms_types::BitView<'_>, FragmentError> {
    let offset = u16::try_from(buffer.get_pos()).map_err(|_| FragmentError::CapacityExceeded)?;
    let length = u16::try_from(buffer.get_len_remaining()).map_err(|_| FragmentError::CapacityExceeded)?;
    buffer
        .as_bit_view()
        .and_then(|bits| bits.window(offset, length))
        .map_err(|_| FragmentError::CapacityExceeded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_3_chunks() {
        let mut buf1 = BitBuffer::from_bitstr("000");
        let t1 = TdmaTime::default().add_timeslots(2);
        let mut buf2 = BitBuffer::from_bitstr("111");
        let t2 = t1.add_timeslots(4);
        let mut buf3 = BitBuffer::from_bitstr("0011");
        let t3 = t2.add_timeslots(4);
        let mut defragger = MsDefrag::new();
        defragger.insert_first(&mut buf1, t1, TetraAddress::issi(1234), None);
        defragger.insert_next(&mut buf2, t2);
        defragger.insert_last(&mut buf3, t3);
        let out = defragger.take_defragged_buf(t3).unwrap();
        assert_eq!(out.buffer.to_bitstr(), "0001110011");
    }
}
