//! Bound burst capacity at both UDP hops without adding an application queue.
use socket2::SockRef;
use tokio::net::UdpSocket;

pub(crate) fn configure(socket: &UdpSocket) -> (u64, u64) {
    let socket = SockRef::from(socket);
    // Some iOS/Darwin kernels reject requests above their per-socket limit.
    // Keep an already larger buffer; failed tuning must not break connectivity.
    let receive = socket.recv_buffer_size().unwrap_or_default();
    if receive < 512 * 1024 {
        for size in [512 * 1024, 256 * 1024, 128 * 1024, 64 * 1024] {
            if size > receive && socket.set_recv_buffer_size(size).is_ok() {
                break;
            }
        }
    }
    if socket.send_buffer_size().unwrap_or_default() < 64 * 1024 {
        let _ = socket.set_send_buffer_size(64 * 1024);
    }
    (
        socket.recv_buffer_size().unwrap_or_default() as u64,
        socket.send_buffer_size().unwrap_or_default() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tuning_preserves_binding_and_forwards_a_frame_sized_burst() {
        let receive = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let send = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = receive.local_addr().unwrap();
        SockRef::from(&receive)
            .set_recv_buffer_size(8 * 1024)
            .unwrap();
        SockRef::from(&receive)
            .set_send_buffer_size(8 * 1024)
            .unwrap();
        let before = SockRef::from(&receive).recv_buffer_size().unwrap();
        let (rx, tx) = configure(&receive);
        configure(&send);
        assert_eq!(receive.local_addr().unwrap(), address);
        assert!(rx >= before as u64);
        assert!(rx >= 64 * 1024);
        assert!(tx >= 64 * 1024);
        // A common keyframe burst exceeds Darwin's small default TX buffer.
        for sequence in 0..24_u8 {
            send.send_to(&[sequence; 1120], address).await.unwrap();
        }
        for sequence in 0..24_u8 {
            let mut packet = [0; 1200];
            let (size, _) = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                receive.recv_from(&mut packet),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(size, 1120);
            assert_eq!(packet[..size], [sequence; 1120]);
        }
    }
}
