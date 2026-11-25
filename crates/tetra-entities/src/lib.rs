#![allow(dead_code)]

pub mod cmce;
pub mod entity_trait;
pub mod llc;
pub mod lmac;
pub mod messagerouter;
pub mod mle;
pub mod mm;
pub mod phy;
pub mod sndcp;
pub mod tnmm;
pub mod umac;

pub mod network;

pub mod management;
pub mod net_control;
pub mod net_telemetry;

// Re-export commonly used items from router
pub use entity_trait::TetraEntityTrait;
pub use messagerouter::{MessagePrio, MessageQueue, MessageRouter};

#[cfg(test)]
mod test_utils {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    pub(crate) fn capture_logs(level: tracing::Level, run: impl FnOnce()) -> String {
        struct Writer(Arc<Mutex<Vec<u8>>>);

        impl Write for Writer {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().write(bytes)
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(level)
            .with_writer(move || Writer(writer.clone()))
            .finish();
        tracing::subscriber::with_default(subscriber, run);
        String::from_utf8(output.lock().unwrap().clone()).unwrap()
    }
}
