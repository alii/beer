use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{self, Duration};

use crate::Result;

const MAX_DATAGRAM_SIZE: usize = 1472; // Standard MTU minus IP and UDP headers
const AUDIO_HEADER_SIZE: usize = 8; // 4 bytes for sequence number, 4 bytes for timestamp
const DISCOVERY_PORT: u16 = 50000;
const DEFAULT_STREAM_PORT: u16 = 50001;
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(1);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AudioSender {
    socket: Arc<UdpSocket>,
    discovery_socket: Arc<UdpSocket>,
    clients: Arc<Mutex<HashSet<SocketAddr>>>,
    stream_port: u16,
}

pub struct AudioReceiver {
    socket: Arc<UdpSocket>,
    discovery_socket: Arc<UdpSocket>,
    server_addr: Arc<Mutex<Option<SocketAddr>>>,
}

impl AudioSender {
    pub async fn new(bind_addr: Option<&str>) -> Result<Self> {
        let bind_addr = bind_addr
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| format!("0.0.0.0:{}", DEFAULT_STREAM_PORT));

        // Create and configure UDP socket
        let socket = UdpSocket::bind(&bind_addr).await?;

        #[cfg(target_os = "macos")]
        {
            use std::os::unix::io::AsRawFd;
            let fd = socket.as_raw_fd();
            unsafe {
                let optval: libc::c_int = 1;
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_TIMESTAMP,
                    &optval as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        let socket = Arc::new(socket);
        let stream_port = socket.local_addr()?.port();

        // Set up discovery socket
        let discovery_socket = UdpSocket::bind(format!("0.0.0.0:{}", DISCOVERY_PORT)).await?;
        discovery_socket.set_broadcast(true)?;
        let discovery_socket = Arc::new(discovery_socket);

        let clients = Arc::new(Mutex::new(HashSet::new()));

        let sender = Self {
            socket,
            discovery_socket,
            clients,
            stream_port,
        };

        sender.start_discovery_service().await?;
        Ok(sender)
    }

    async fn start_discovery_service(&self) -> Result<()> {
        let discovery_socket = self.discovery_socket.clone();
        let clients = self.clients.clone();
        let stream_port = self.stream_port;

        // Handle incoming discovery requests
        let discovery_socket_clone = discovery_socket.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 64];
            loop {
                match discovery_socket_clone.recv_from(&mut buf).await {
                    Ok((_, client_addr)) => {
                        let response = format!("SERVER:{}", stream_port);
                        if let Err(e) = discovery_socket_clone
                            .send_to(response.as_bytes(), client_addr)
                            .await
                        {
                            log::error!("Failed to send discovery response: {}", e);
                            continue;
                        }
                        clients
                            .lock()
                            .await
                            .insert(SocketAddr::new(client_addr.ip(), stream_port));
                    }
                    Err(e) => log::error!("Discovery receive error: {}", e),
                }
            }
        });

        // Broadcast server presence periodically
        let broadcast_addr = SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)),
            DISCOVERY_PORT,
        );

        tokio::spawn(async move {
            let mut interval = time::interval(DISCOVERY_INTERVAL);
            loop {
                interval.tick().await;
                let announcement = format!("SERVER:{}", stream_port);
                if let Err(e) = discovery_socket
                    .send_to(announcement.as_bytes(), broadcast_addr)
                    .await
                {
                    log::error!("Failed to broadcast server presence: {}", e);
                }
            }
        });

        Ok(())
    }

    pub async fn start_sending(&self, mut rx: mpsc::Receiver<Vec<f32>>) -> Result<()> {
        log::info!("Starting audio sender on port {}", self.stream_port);

        while let Some(samples) = rx.recv().await {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u32;

            // Convert samples to bytes efficiently
            let mut packet = Vec::with_capacity(AUDIO_HEADER_SIZE + samples.len() * 4);
            packet.extend_from_slice(&[0u8; 4]); // Unused sequence number
            packet.extend_from_slice(&timestamp.to_le_bytes());

            // Add samples directly to packet
            for sample in samples {
                packet.extend_from_slice(&sample.to_le_bytes());
            }

            // Send to all clients
            let clients = self.clients.lock().await.clone();
            for client in clients {
                if let Err(e) = self.socket.send_to(&packet, client).await {
                    log::error!("Failed to send to client {}: {}", client, e);
                }
            }
        }
        Ok(())
    }
}

impl AudioReceiver {
    pub async fn new(bind_addr: Option<&str>) -> Result<Self> {
        let bind_addr = bind_addr
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| format!("0.0.0.0:{}", DEFAULT_STREAM_PORT));

        // Create and configure UDP socket
        let socket = UdpSocket::bind(&bind_addr).await?;

        #[cfg(target_os = "macos")]
        {
            use std::os::unix::io::AsRawFd;
            let fd = socket.as_raw_fd();
            unsafe {
                let optval: libc::c_int = 1;
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_TIMESTAMP,
                    &optval as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        let socket = Arc::new(socket);

        // Set up discovery socket
        let discovery_socket = UdpSocket::bind("0.0.0.0:0").await?;
        discovery_socket.set_broadcast(true)?;
        let discovery_socket = Arc::new(discovery_socket);

        Ok(Self {
            socket,
            discovery_socket,
            server_addr: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn start_receiving(&self, tx: mpsc::Sender<Vec<f32>>) -> Result<()> {
        let mut buf = vec![0u8; MAX_DATAGRAM_SIZE];
        log::info!("Starting audio receiver on {:?}", self.socket.local_addr()?);

        loop {
            let (len, _) = self.socket.recv_from(&mut buf).await?;

            if len < AUDIO_HEADER_SIZE {
                continue;
            }

            // Convert audio data to samples immediately
            let samples: Vec<f32> = buf[AUDIO_HEADER_SIZE..len]
                .chunks_exact(4)
                .map(|chunk| {
                    let mut bytes = [0u8; 4];
                    bytes.copy_from_slice(chunk);
                    f32::from_le_bytes(bytes)
                })
                .collect();

            // Send samples immediately
            if let Err(e) = tx.send(samples).await {
                log::error!("Failed to send samples to player: {}", e);
                break;
            }
        }

        Ok(())
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    pub async fn server_addr(&self) -> Result<SocketAddr> {
        self.server_addr
            .lock()
            .await
            .ok_or_else(|| crate::AudioStreamerError::NetworkError("No server found".into()))
    }

    pub async fn discover_server(&self) -> Result<()> {
        let broadcast_addr = SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)),
            DISCOVERY_PORT,
        );

        // Send discovery request
        let request = "DISCOVER";
        self.discovery_socket
            .send_to(request.as_bytes(), broadcast_addr)
            .await?;

        // Wait for server response
        let mut buf = [0u8; 64];
        let timeout = time::sleep(DISCOVERY_TIMEOUT);
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                result = self.discovery_socket.recv_from(&mut buf) => {
                    match result {
                        Ok((len, addr)) => {
                            let response = String::from_utf8_lossy(&buf[..len]);
                            if let Some(port_str) = response.strip_prefix("SERVER:") {
                                if let Ok(port) = port_str.trim().parse::<u16>() {
                                    let server_addr = SocketAddr::new(addr.ip(), port);
                                    *self.server_addr.lock().await = Some(server_addr);
                                    break;
                                }
                            }
                        }
                        Err(e) => log::error!("Discovery receive error: {}", e),
                    }
                }
                _ = &mut timeout => {
                    return Err(crate::AudioStreamerError::NetworkError(
                        "Server discovery timeout".into()
                    ));
                }
            }
        }

        Ok(())
    }
}
