use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use super::{NetworkAddress, NetworkError, NetworkMessage, NetworkTransport};

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_FRAMES_PER_POLL: usize = 64;

/// Configuration for creating a TCP transport
#[derive(Debug, Clone)]
pub struct TcpTransportConfig {
    /// Server address to connect to
    pub server_addr: NetworkAddress,
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Read timeout
    pub read_timeout: Duration,
}

impl TcpTransportConfig {
    /// Create a new TCP transport configuration with default timeouts
    pub fn new(server_addr: NetworkAddress) -> Self {
        Self {
            server_addr,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(30),
        }
    }

    /// Create a new TCP transport configuration with custom timeouts
    pub fn with_timeouts(server_addr: NetworkAddress, connect_timeout: Duration, read_timeout: Duration) -> Self {
        Self {
            server_addr,
            connect_timeout,
            read_timeout,
        }
    }
}

/// TCP-based network transport
pub struct TcpTransport {
    stream: Option<TcpStream>,
    server_addr: NetworkAddress,
    connect_timeout: Duration,
    read_timeout: Duration,
    rx_header: [u8; 4],
    rx_header_len: usize,
    rx_payload: Vec<u8>,
    rx_payload_len: usize,
}

impl TcpTransport {
    pub fn new(server_addr: NetworkAddress, connect_timeout: Duration, read_timeout: Duration) -> Self {
        Self {
            stream: None,
            server_addr,
            connect_timeout,
            read_timeout,
            rx_header: [0; 4],
            rx_header_len: 0,
            rx_payload: Vec::new(),
            rx_payload_len: 0,
        }
    }

    fn ensure_stream_exists(&mut self) -> Result<(), NetworkError> {
        if self.stream.is_none() {
            self.connect()?;
        }
        Ok(())
    }

    fn get_tcp_addr(&self) -> Result<String, NetworkError> {
        match &self.server_addr {
            NetworkAddress::Tcp { host, port } => Ok(format!("{}:{}", host, port)),
            _ => Err(NetworkError::ConnectionFailed("Invalid address type for TcpTransport".to_string())),
        }
    }

    /// Closes the current TCP connection, if any.
    fn close_connection(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.rx_header_len = 0;
        self.rx_payload.clear();
        self.rx_payload_len = 0;
    }

    fn read_frame(&mut self) -> Result<Option<NetworkMessage>, NetworkError> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| NetworkError::ReceiveFailed("No active connection".to_string()))?;
        loop {
            let buffer = if self.rx_header_len < 4 {
                &mut self.rx_header[self.rx_header_len..]
            } else {
                &mut self.rx_payload[self.rx_payload_len..]
            };
            let read = if buffer.is_empty() { Ok(0) } else { stream.read(buffer) };
            let count = match read {
                Ok(0) if !buffer.is_empty() => return Err(NetworkError::ReceiveFailed("TCP peer disconnected during frame".to_string())),
                Ok(count) => count,
                Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(None),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(NetworkError::ReceiveFailed(error.to_string())),
            };
            if self.rx_header_len < 4 {
                self.rx_header_len += count;
                if self.rx_header_len == 4 {
                    let len = u32::from_be_bytes(self.rx_header) as usize;
                    if len > MAX_FRAME_BYTES {
                        return Err(NetworkError::ReceiveFailed("TCP frame exceeds one MiB".to_string()));
                    }
                    self.rx_payload.resize(len, 0);
                    self.rx_payload_len = 0;
                }
            } else {
                self.rx_payload_len += count;
            }
            if self.rx_header_len == 4 && self.rx_payload_len == self.rx_payload.len() {
                self.rx_header_len = 0;
                self.rx_payload_len = 0;
                return Ok(Some(NetworkMessage {
                    source: self.server_addr.clone(),
                    payload: std::mem::take(&mut self.rx_payload),
                    timestamp: Instant::now(),
                }));
            }
        }
    }

    /// Internal send implementation - does the actual I/O
    fn try_send(&mut self, payload: &[u8]) -> Result<(), NetworkError> {
        if let Some(ref mut stream) = self.stream {
            // Send message length first, then payload
            let len = payload.len() as u32;
            let len_bytes = len.to_be_bytes();

            stream
                .write_all(&len_bytes)
                .map_err(|e| NetworkError::SendFailed(format!("Failed to send length: {}", e)))?;

            stream
                .write_all(payload)
                .map_err(|e| NetworkError::SendFailed(format!("Failed to send payload: {}", e)))?;

            stream
                .flush()
                .map_err(|e| NetworkError::SendFailed(format!("Failed to flush: {}", e)))?;

            Ok(())
        } else {
            Err(NetworkError::SendFailed("No active connection".to_string()))
        }
    }
}

impl NetworkTransport for TcpTransport {
    fn connect(&mut self) -> Result<(), NetworkError> {
        tracing::debug!("TcpTransport connecting to {:?}", self.server_addr);
        // Close any existing connection gracefully
        self.close_connection();
        let addr = self.get_tcp_addr()?;

        match TcpStream::connect_timeout(
            &addr
                .parse()
                .map_err(|e| NetworkError::ConnectionFailed(format!("Invalid address: {}", e)))?,
            self.connect_timeout,
        ) {
            Ok(stream) => {
                // Set read timeout
                stream
                    .set_read_timeout(Some(self.read_timeout))
                    .map_err(|e| NetworkError::ConnectionFailed(format!("Failed to set timeout: {}", e)))?;
                stream
                    .set_write_timeout(Some(self.connect_timeout))
                    .map_err(|e| NetworkError::ConnectionFailed(format!("Failed to set write timeout: {e}")))?;

                self.stream = Some(stream);
                Ok(())
            }

            Err(e) => Err(NetworkError::ConnectionFailed(format!("TCP connect failed: {}", e))),
        }
    }

    fn send_reliable(&mut self, payload: &[u8]) -> Result<(), NetworkError> {
        if payload.len() > MAX_FRAME_BYTES {
            return Err(NetworkError::SendFailed("TCP frame exceeds one MiB".to_string()));
        }
        self.ensure_stream_exists()?;

        // Try to send, and retry once after reconnect if the connection was stale
        match self.try_send(payload) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Connection may have been closed by server (idle timeout, etc.)
                tracing::trace!("Send failed, attempting reconnect: {}", e);
                self.connect()?;
                self.try_send(payload)
            }
        }
    }

    fn send_unreliable(&mut self, _payload: &[u8]) -> Result<(), NetworkError> {
        Err(NetworkError::SendFailed("TCP has no unreliable channel".to_string()))
    }

    fn receive_reliable(&mut self) -> Vec<NetworkMessage> {
        let mut messages = Vec::new();

        if let Some(stream) = &self.stream {
            if let Err(e) = stream.set_nonblocking(true) {
                tracing::error!("Failed to set non-blocking mode: {}", e);
                self.close_connection();
                return messages;
            }
        } else {
            return messages;
        }
        for _ in 0..MAX_FRAMES_PER_POLL {
            match self.read_frame() {
                Ok(Some(message)) => messages.push(message),
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!("TCP receive failed: {error}");
                    self.close_connection();
                    break;
                }
            }
        }
        if let Some(stream) = &self.stream
            && let Err(error) = stream.set_nonblocking(false)
        {
            tracing::error!("TCP could not restore blocking mode: {error}");
            self.close_connection();
        }

        messages
    }

    fn receive_unreliable(&mut self) -> Vec<NetworkMessage> {
        tracing::warn!("TCP has no unreliable channel");
        Vec::new()
    }

    /// Wait for a single response message with blocking read and timeout
    /// Used for request-response patterns where we expect a reply
    fn wait_for_response_reliable(&mut self) -> Result<NetworkMessage, NetworkError> {
        if let Some(stream) = &self.stream {
            // Ensure blocking mode (should already be set from reconnect)
            stream
                .set_nonblocking(false)
                .map_err(|e| NetworkError::ReceiveFailed(format!("Failed to set blocking mode: {}", e)))?;

            match self.read_frame() {
                Ok(Some(message)) => Ok(message),
                Ok(None) => Err(NetworkError::Timeout),
                Err(error) => {
                    self.close_connection();
                    Err(error)
                }
            }
        } else {
            Err(NetworkError::ReceiveFailed("No active connection".to_string()))
        }
    }

    fn disconnect(&mut self) {
        self.close_connection();
    }

    fn is_connected(&self) -> bool {
        self.stream.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn pair() -> (TcpTransport, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (peer, _) = listener.accept().unwrap();
        let mut transport = TcpTransport::new(
            NetworkAddress::Tcp {
                host: "127.0.0.1".to_string(),
                port: listener.local_addr().unwrap().port(),
            },
            Duration::from_secs(1),
            Duration::from_secs(1),
        );
        transport.stream = Some(stream);
        (transport, peer)
    }

    fn poll_until(transport: &mut TcpTransport, expected: usize) -> Vec<NetworkMessage> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut messages = Vec::new();
        while messages.len() < expected {
            messages.extend(transport.receive_reliable());
            assert!(Instant::now() < deadline, "TCP framing timed out");
            std::thread::yield_now();
        }
        messages
    }

    #[test]
    fn split_prefix_body_multiple_frames_and_empty_payload() {
        let (mut transport, mut peer) = pair();
        let header = 3u32.to_be_bytes();
        for byte in &header[..3] {
            peer.write_all(&[*byte]).unwrap();
            assert!(transport.receive_reliable().is_empty());
        }
        peer.write_all(&[header[3], b'a']).unwrap();
        assert!(transport.receive_reliable().is_empty());
        peer.write_all(b"bc").unwrap();
        let messages = poll_until(&mut transport, 1);
        assert_eq!(messages[0].payload, b"abc");
        peer.write_all(&[0, 0, 0, 0, 0, 0, 0, 1, b'z']).unwrap();
        let messages = poll_until(&mut transport, 2);
        assert!(messages[0].payload.is_empty());
        assert_eq!(messages[1].payload, b"z");
    }

    #[test]
    fn oversized_prefix_and_partial_disconnect_clear_state() {
        for prefix in [(MAX_FRAME_BYTES as u32 + 1).to_be_bytes().to_vec(), vec![0, 0]] {
            let (mut transport, mut peer) = pair();
            peer.write_all(&prefix).unwrap();
            peer.shutdown(std::net::Shutdown::Both).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while transport.is_connected() {
                assert!(transport.receive_reliable().is_empty());
                assert!(Instant::now() < deadline);
            }
            assert_eq!(transport.rx_header_len, 0);
            assert!(transport.rx_payload.is_empty());
        }
    }
}
