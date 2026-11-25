//! Portable byte-chunk path versus bit-at-a-time reference. Not an MCU cycle benchmark.
use std::{hint::black_box, time::Instant};
use tetra_ms_types::BitView;

fn main() -> Result<(), tetra_ms_types::BitError> {
    let bytes = black_box([0xa5; 336]);
    let view = BitView::new(&bytes, 2688)?;
    let iterations = 1_000_000;
    for width in [1, 8, 16, 24, 32] {
        let start = Instant::now();
        let mut reference = 0;
        for i in 0..iterations {
            reference ^= black_box(view).read_reference((i % 2600) as u16, black_box(width))?;
        }
        let reference_ns = start.elapsed().as_nanos();
        let start = Instant::now();
        let mut chunked = 0;
        for i in 0..iterations {
            chunked ^= black_box(view).read((i % 2600) as u16, black_box(width))?;
        }
        let chunked_ns = start.elapsed().as_nanos();
        assert_eq!(black_box(reference), black_box(chunked));
        println!("width={width} iterations={iterations} reference_ns={reference_ns} byte_chunks_ns={chunked_ns}");
    }
    Ok(())
}
